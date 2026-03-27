use std::path::Path;
use std::sync::Arc;
use std::io::Read;
use serde::Serialize;
use tauri::State;
use base64::Engine;
use zeroize::Zeroize;

use cfs_io::block_device::{CFSBlockDevice, FileBlockDevice, RawPartitionBlockDevice};
use cfs_io::cli::parse_size;
use cfs_io::crypto::{self, EncryptedBlockDevice, KdfAlgorithm, KdfParams};
use cfs_io::volume::{CFSVolume, FormatOptions, ErrorBehavior, DEFAULT_BLOCK_SIZE, INODE_FILE, INODE_DIR};
use cfs_io::fuse;

use crate::state::{AppState, OpenVolume};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct DetectResult {
    pub exists: bool,
    pub is_encrypted: bool,
    pub size_bytes: u64,
}

#[derive(Serialize, Clone)]
pub struct VolumeInfoDto {
    pub path: String,
    pub is_encrypted: bool,
    pub block_size: u32,
    pub total_blocks: u64,
    pub free_blocks: u64,
    pub inode_count: u32,
    pub total_size: u64,
    pub free_size: u64,
    // v3 fields
    pub version: u32,
    pub inode_size: u32,
    pub feature_flags: u32,
    pub block_groups: u32,
    pub journal_blocks: u64,
    pub volume_label: String,
    pub error_behavior: String,
    pub default_permissions: u32,
}

#[derive(Serialize, Clone)]
pub struct AppStatusDto {
    pub volume_loaded: bool,
    pub volume_path: Option<String>,
    pub is_encrypted: bool,
    pub is_mounted: bool,
    pub drive_letter: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct DirEntryDto {
    pub name: String,
    pub inode_index: u32,
    pub file_type: String,
    pub size: u64,
    pub modified: u64,
    pub created: u64,
}

#[derive(Serialize, Clone)]
pub struct InodeDto {
    pub file_type: String,
    pub size: u64,
    pub block_count: u32,
    pub nlinks: u16,
    pub created: u64,
    pub modified: u64,
    pub direct_blocks: Vec<u64>,
    pub has_indirect: bool,
    pub has_double_indirect: bool,
}

#[derive(Serialize, Clone)]
pub struct RawPartitionInfo {
    pub device_path: String,
    pub drive_letter: String,
    pub size_bytes: u64,
    pub is_cfs: bool,
    pub is_encrypted: bool,
}

#[derive(Serialize, Clone)]
pub struct FilePreviewDto {
    pub data_base64: String,
    pub is_text: bool,
    pub total_size: u64,
    pub truncated: bool,
}

#[derive(Serialize, Clone)]
pub struct MountInfoDto {
    pub drive_letter: String,
    pub mounted: bool,
}

#[derive(Serialize, Clone)]
pub struct VolumeFileDto {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub is_encrypted: bool,
}

/// DTO for format options passed from the frontend.
/// All fields are `Option` â€” unset fields use defaults.
/// If `preset` is set, the preset is applied first, then individual overrides.
#[derive(serde::Deserialize, Clone, Debug)]
pub struct FormatOptionsDto {
    pub block_size: Option<u32>,
    pub inode_size: Option<u32>,
    pub inode_ratio: Option<u32>,
    pub journal_percent: Option<f32>,
    pub volume_label: Option<String>,
    pub secure_delete: Option<bool>,
    pub default_permissions: Option<u32>,
    pub error_behavior: Option<String>,
    pub blocks_per_group: Option<u32>,
    pub preset: Option<String>,
}

impl FormatOptionsDto {
    /// Convert this DTO into a `FormatOptions`, applying preset first then overrides.
    pub fn to_format_options(&self) -> Result<FormatOptions, String> {
        let mut opts = match self.preset.as_deref() {
            Some("large-files") => FormatOptions::large_files(),
            Some("small-files") => FormatOptions::small_files(),
            Some("max-security") => FormatOptions::max_security(),
            Some("minimal") => FormatOptions::minimal_legacy(),
            Some("general") | None => FormatOptions::default(),
            Some(other) => return Err(format!("unknown preset: '{other}'")),
        };

        if let Some(bs) = self.block_size {
            opts.block_size = bs;
            // Auto-adjust blocks_per_group unless explicitly set
            if self.blocks_per_group.is_none() {
                opts.blocks_per_group = bs * 8;
            }
        }
        if let Some(is) = self.inode_size { opts.inode_size = is; }
        if let Some(ir) = self.inode_ratio { opts.inode_ratio = ir; }
        if let Some(j) = self.journal_percent { opts.journal_percent = j; }
        if let Some(ref l) = self.volume_label { opts.volume_label = l.clone(); }
        if let Some(sd) = self.secure_delete { opts.secure_delete = sd; }
        if let Some(dp) = self.default_permissions { opts.default_permissions = dp; }
        if let Some(ref eb) = self.error_behavior {
            opts.error_behavior = match eb.as_str() {
                "continue" => ErrorBehavior::Continue,
                "read-only" | "readonly" => ErrorBehavior::ReadOnly,
                other => return Err(format!("unknown error behavior: '{other}'")),
            };
        }
        if let Some(bpg) = self.blocks_per_group { opts.blocks_per_group = bpg; }

        opts.validate().map_err(|e| e.to_string())?;
        Ok(opts)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Default directory where CFS volume images are stored.
fn default_volumes_dir() -> String {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| "C:\\".to_string());
    format!("{}\\CFS Volumes", base)
}

/// Quick magic-byte check: returns Some(true)=encrypted, Some(false)=plain CFS, None=not CFS.
fn detect_cfs_magic(path: &Path) -> Option<bool> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).ok()?;
    match &magic {
        b"CFS1" => Some(false),
        b"CFSE" => Some(true),
        _ => None,
    }
}

fn volume_info_from(vol: &CFSVolume, path: &str, encrypted: bool) -> VolumeInfoDto {
    let sb = vol.superblock();
    let label_len = sb.volume_label.iter().position(|&b| b == 0).unwrap_or(sb.volume_label.len());
    let label = String::from_utf8_lossy(&sb.volume_label[..label_len]).to_string();
    let err_behavior = match sb.error_behavior {
        1 => "read-only".to_string(),
        _ => "continue".to_string(),
    };
    VolumeInfoDto {
        path: path.to_string(),
        is_encrypted: encrypted,
        block_size: sb.block_size,
        total_blocks: sb.total_blocks,
        free_blocks: sb.free_blocks,
        inode_count: sb.inode_count,
        total_size: sb.total_blocks * sb.block_size as u64,
        free_size: sb.free_blocks * sb.block_size as u64,
        version: sb.version,
        inode_size: sb.inode_size,
        feature_flags: sb.features_flags,
        block_groups: sb.group_count,
        journal_blocks: sb.journal_blocks,
        volume_label: label,
        error_behavior: err_behavior,
        default_permissions: sb.default_permissions,
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn detect_volume(path: &str) -> Result<DetectResult, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(DetectResult {
            exists: false,
            is_encrypted: false,
            size_bytes: 0,
        });
    }

    let meta = std::fs::metadata(p).map_err(|e| format!("Cannot read file: {e}"))?;
    let size_bytes = meta.len();

    // Read first 4 bytes to check magic
    let mut dev = FileBlockDevice::open(p, None)
        .map_err(|e| format!("Cannot open file: {e}"))?;

    let is_encrypted = crypto::is_encrypted_device(&mut dev)
        .unwrap_or(false);

    Ok(DetectResult {
        exists: true,
        is_encrypted,
        size_bytes,
    })
}

#[tauri::command]
pub fn create_volume(
    state: State<'_, AppState>,
    path: &str,
    size: &str,
    password: &str,
    block_size: Option<u32>,
    kdf: Option<String>,
    pbkdf2_iterations: Option<u32>,
    argon2_memory_mib: Option<u32>,
    argon2_time: Option<u32>,
    argon2_parallelism: Option<u32>,
    format_options: Option<FormatOptionsDto>,
) -> Result<VolumeInfoDto, String> {
    // Build format options from DTO, applying preset + overrides
    let dto_blocks_per_group = format_options.as_ref().and_then(|d| d.blocks_per_group);
    let mut format_opts = match format_options {
        Some(dto) => dto.to_format_options()?,
        None => FormatOptions::default(),
    };
    // CLI-level block_size param takes precedence for backward compat
    if let Some(bs) = block_size {
        format_opts.block_size = bs;
        // Auto-adjust blocks_per_group if not explicitly set via DTO
        if dto_blocks_per_group.is_none() {
            format_opts.blocks_per_group = bs * 8;
        }
    }
    format_opts.validate().map_err(|e| e.to_string())?;
    let bs = format_opts.block_size;

    // Validate password length
    if password.len() < 8 {
        return Err("Password must be at least 8 characters".into());
    }

    // Parse size
    let size_bytes = parse_size(size).map_err(|e| format!("Invalid size: {e}"))?;

    // Check file doesn't already exist
    if Path::new(path).exists() {
        return Err("File already exists".into());
    }

    // Build KDF params
    let kdf_params = match kdf.as_deref().unwrap_or("argon2id") {
        "pbkdf2" => KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: pbkdf2_iterations.unwrap_or(600_000),
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        },
        _ => KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: argon2_memory_mib.unwrap_or(32) * 1024,
            argon2_time_cost: argon2_time.unwrap_or(2),
            argon2_parallelism: argon2_parallelism.unwrap_or(1),
        },
    };

    // Copy password bytes so we can zeroize after use
    let mut pw_bytes = password.as_bytes().to_vec();

    // Create backing file
    let dev = FileBlockDevice::open(Path::new(path), Some(size_bytes))
        .map_err(|e| format!("Cannot create file: {e}"))?;

    // Always create encrypted (UI constraint)
    let enc_result = EncryptedBlockDevice::format_encrypted(
        Box::new(dev),
        &pw_bytes,
        &kdf_params,
        bs,
    );
    pw_bytes.zeroize();
    let enc = enc_result.map_err(|e| format!("Encryption failed: {e}"))?;

    let vol = CFSVolume::format_v3(Box::new(enc), &format_opts)
        .map_err(|e| format!("Format failed: {e}"))?;

    vol.sync().map_err(|e| format!("Sync failed: {e}"))?;

    let info = volume_info_from(&vol, path, true);

    let mut guard = state.volume.lock().map_err(|e| e.to_string())?;
    *guard = Some(OpenVolume {
        vol: Arc::new(vol),
        path: path.to_string(),
        is_encrypted: true,
        mount_handle: None,
        drive_letter: None,
    });

    Ok(info)
}

#[tauri::command]
pub fn unlock_volume(
    state: State<'_, AppState>,
    path: &str,
    password: &str,
    block_size: Option<u32>,
) -> Result<VolumeInfoDto, String> {
    let bs = block_size.unwrap_or(DEFAULT_BLOCK_SIZE);

    // Determine if this is a raw device path or a file
    let is_device = path.starts_with("\\\\.\\") || path.starts_with("//./");

    let mut dev: Box<dyn cfs_io::block_device::CFSBlockDevice> = if is_device {
        Box::new(
            RawPartitionBlockDevice::open(path)
                .map_err(|e| format!("Cannot open device: {e}"))?,
        )
    } else {
        let p = Path::new(path);
        if !p.exists() {
            return Err("File not found".into());
        }
        Box::new(
            FileBlockDevice::open(p, None)
                .map_err(|e| format!("Cannot open file: {e}"))?,
        )
    };
    let is_encrypted = crypto::is_encrypted_device(&mut *dev)
        .map_err(|e| format!("Cannot read volume: {e}"))?;

    // Copy password bytes so we can zeroize after use
    let mut pw_bytes = password.as_bytes().to_vec();

    let vol = if is_encrypted {
        let enc_result = EncryptedBlockDevice::open_encrypted(dev, &pw_bytes);
        pw_bytes.zeroize();
        let enc = enc_result.map_err(|e| format!("Wrong password or corrupted header: {e}"))?;
        CFSVolume::mount(Box::new(enc), bs)
            .map_err(|e| format!("Mount failed: {e}"))?
    } else {
        // Plain volume â€” password is ignored; still zeroize the copy
        pw_bytes.zeroize();
        CFSVolume::mount(dev, bs)
            .map_err(|e| format!("Mount failed: {e}"))?
    };

    let info = volume_info_from(&vol, path, is_encrypted);

    let mut guard = state.volume.lock().map_err(|e| e.to_string())?;
    *guard = Some(OpenVolume {
        vol: Arc::new(vol),
        path: path.to_string(),
        is_encrypted,
        mount_handle: None,
        drive_letter: None,
    });

    Ok(info)
}

#[tauri::command]
pub fn lock_volume(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.volume.lock().map_err(|e| e.to_string())?;
    match guard.take() {
        Some(mut ov) => {
            // Stop mount if active
            if let Some(ref mut mh) = ov.mount_handle {
                mh.stop();
            }
            // Sync and drop
            let _ = ov.vol.sync();
            Ok(())
        }
        None => Err("No volume loaded".into()),
    }
}

#[tauri::command]
pub fn get_volume_info(state: State<'_, AppState>) -> Result<VolumeInfoDto, String> {
    let guard = state.volume.lock().map_err(|e| e.to_string())?;
    match guard.as_ref() {
        Some(ov) => {
                    Ok(volume_info_from(&ov.vol, &ov.path, ov.is_encrypted))
        }
        None => Err("No volume loaded".into()),
    }
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Result<AppStatusDto, String> {
    let guard = state.volume.lock().map_err(|e| e.to_string())?;
    match guard.as_ref() {
        Some(ov) => Ok(AppStatusDto {
            volume_loaded: true,
            volume_path: Some(ov.path.clone()),
            is_encrypted: ov.is_encrypted,
            is_mounted: ov.mount_handle.is_some(),
            drive_letter: ov.drive_letter.clone(),
        }),
        None => Ok(AppStatusDto {
            volume_loaded: false,
            volume_path: None,
            is_encrypted: false,
            is_mounted: false,
            drive_letter: None,
        }),
    }
}

// ---------------------------------------------------------------------------
// Phase F2 â€” Directory browsing
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_dir(state: State<'_, AppState>, path: &str) -> Result<Vec<DirEntryDto>, String> {
    let guard = state.volume.lock().map_err(|e| e.to_string())?;
    let ov = guard.as_ref().ok_or("No volume loaded")?;

    let entries = ov.vol.list_dir(path).map_err(|e| format!("{e}"))?;

    let mut result = Vec::new();
    for entry in &entries {
        let name = entry.name_str().to_string();
        // Skip . and ..
        if name == "." || name == ".." {
            continue;
        }
        let ft = if entry.file_type == INODE_DIR as u8 {
            "directory"
        } else {
            "file"
        };
        // Read inode for size/timestamps
        let (size, modified, created) = match ov.vol.read_inode(entry.inode_index) {
            Ok(inode) => (inode.size, inode.modified, inode.created),
            Err(_) => (0, 0, 0),
        };
        result.push(DirEntryDto {
            name,
            inode_index: entry.inode_index,
            file_type: ft.to_string(),
            size,
            modified,
            created,
        });
    }
    Ok(result)
}

#[tauri::command]
pub fn stat_entry(state: State<'_, AppState>, path: &str) -> Result<InodeDto, String> {
    let guard = state.volume.lock().map_err(|e| e.to_string())?;
    let ov = guard.as_ref().ok_or("No volume loaded")?;

    let inode = ov.vol.stat(path).map_err(|e| format!("{e}"))?;
    let ft = match inode.mode {
        m if m == INODE_FILE => "file",
        m if m == INODE_DIR => "directory",
        _ => "unused",
    };
    Ok(InodeDto {
        file_type: ft.to_string(),
        size: inode.size,
        block_count: inode.block_count,
        nlinks: inode.nlinks,
        created: inode.created,
        modified: inode.modified,
        direct_blocks: inode.direct_blocks.to_vec(),
        has_indirect: inode.indirect_block != 0,
        has_double_indirect: inode.double_indirect != 0,
    })
}

// ---------------------------------------------------------------------------
// Phase F3 â€” File preview
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn read_file_preview(
    state: State<'_, AppState>,
    path: &str,
    max_bytes: Option<u64>,
) -> Result<FilePreviewDto, String> {
    let max = max_bytes.unwrap_or(65536); // 64KB default
    let guard = state.volume.lock().map_err(|e| e.to_string())?;
    let ov = guard.as_ref().ok_or("No volume loaded")?;

    let inode = ov.vol.stat(path).map_err(|e| format!("{e}"))?;
    if inode.mode != INODE_FILE {
        return Err("Not a file".into());
    }

    let total_size = inode.size;
    let read_len = std::cmp::min(max, total_size);
    let data = ov.vol.read_file(path, 0, read_len).map_err(|e| format!("{e}"))?;

    // Heuristic: scan first 8KB for NUL bytes to determine if text
    let check_len = std::cmp::min(data.len(), 8192);
    let is_text = !data[..check_len].contains(&0u8);

    let data_base64 = base64::engine::general_purpose::STANDARD.encode(&data);

    Ok(FilePreviewDto {
        data_base64,
        is_text,
        total_size,
        truncated: total_size > max,
    })
}

// ---------------------------------------------------------------------------
// Raw partition discovery
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_raw_partitions() -> Result<Vec<RawPartitionInfo>, String> {
    let mut partitions = Vec::new();

    // Scan drive letters A-Z for RAW/unformatted partitions
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        let device_path = format!("\\\\.\\{}:", letter as char);

        // Check if the drive exists
        let drive_wide: Vec<u16> = drive.encode_utf16().chain(std::iter::once(0)).collect();
        let drive_type = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(drive_wide.as_ptr()) };

        // DRIVE_FIXED = 3, DRIVE_UNKNOWN = 0, DRIVE_NO_ROOT_DIR = 1
        // We look for fixed drives and unknown types that might be RAW
        if drive_type == 0 || drive_type == 1 {
            continue; // No root directory - drive doesn't exist
        }

        // Try to check if it's a RAW partition by checking the filesystem type
        let mut fs_name = [0u16; 256];
        let mut vol_name = [0u16; 256];
        let mut serial = 0u32;
        let mut max_component = 0u32;
        let mut flags = 0u32;
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW(
                drive_wide.as_ptr(),
                vol_name.as_mut_ptr(),
                vol_name.len() as u32,
                &mut serial,
                &mut max_component,
                &mut flags,
                fs_name.as_mut_ptr(),
                fs_name.len() as u32,
            )
        };

        if ok == 0 {
            // GetVolumeInformation failed â€” likely RAW or inaccessible partition
            // Try to open it and check for CFS magic
            let mut is_cfs = false;
            let mut is_encrypted = false;
            let mut size_bytes = 0u64;

            if let Ok(mut dev) = RawPartitionBlockDevice::open(&device_path) {
                size_bytes = dev.size();
                // Try to read magic bytes
                let mut buf = vec![0u8; 512];
                if dev.read(0, &mut buf).is_ok() {
                    if &buf[0..4] == b"CFS1" {
                        is_cfs = true;
                        is_encrypted = false;
                    } else if &buf[0..4] == b"CFSE" {
                        is_cfs = true;
                        is_encrypted = true;
                    }
                }
            }

            if size_bytes > 0 {
                partitions.push(RawPartitionInfo {
                    device_path,
                    drive_letter: format!("{}:", letter as char),
                    size_bytes,
                    is_cfs,
                    is_encrypted,
                });
            }
        }
    }

    Ok(partitions)
}

// ---------------------------------------------------------------------------
// Phase F4 â€” Mount integration
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn check_winfsp() -> Result<bool, String> {
    Ok(fuse::is_winfsp_available())
}

#[tauri::command]
pub fn mount_drive(
    state: State<'_, AppState>,
    drive_letter: Option<String>,
) -> Result<MountInfoDto, String> {
    let mut guard = state.volume.lock().map_err(|e| e.to_string())?;
    let ov = guard.as_mut().ok_or("No volume loaded")?;

    if ov.mount_handle.is_some() {
        return Err("Already mounted".into());
    }

    if !fuse::is_winfsp_available() {
        return Err("WinFSP is not installed".into());
    }

    let vol_arc = ov.vol.clone();
    let block_size = vol_arc.superblock().block_size;

    let mount_point = drive_letter.clone();
    let handle = fuse::mount_background(vol_arc, block_size, mount_point)
        .map_err(|e| format!("Mount failed: {e}"))?;

    let dl = handle.drive_letter().to_string();
    ov.mount_handle = Some(handle);
    ov.drive_letter = Some(dl.clone());

    Ok(MountInfoDto {
        drive_letter: dl,
        mounted: true,
    })
}

#[tauri::command]
pub fn unmount_drive(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.volume.lock().map_err(|e| e.to_string())?;
    let ov = guard.as_mut().ok_or("No volume loaded")?;

    match ov.mount_handle.take() {
        Some(mut mh) => {
            mh.stop();
            ov.drive_letter = None;
            Ok(())
        }
        None => Err("Not mounted".into()),
    }
}

// ---------------------------------------------------------------------------
// Default volumes directory & scanning
// ---------------------------------------------------------------------------

/// Returns the default directory for CFS volume images, creating it if needed.
#[tauri::command]
pub fn get_default_volumes_dir() -> Result<String, String> {
    let dir = default_volumes_dir();
    if !Path::new(&dir).exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create directory: {e}"))?;
    }
    Ok(dir)
}

/// Lists all `.img` files in a directory (defaults to the CFS Volumes dir).
/// Each file is inspected for CFS/CFSE magic bytes.
#[tauri::command]
pub fn list_volume_files(dir: Option<String>) -> Result<Vec<VolumeFileDto>, String> {
    let dir = dir.unwrap_or_else(default_volumes_dir);
    let path = Path::new(&dir);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(path).map_err(|e| format!("Cannot read directory: {e}"))?;
    let mut result = Vec::new();

    for entry in entries.flatten() {
        let file_path = entry.path();
        if file_path.extension().and_then(|e| e.to_str()) != Some("img") {
            continue;
        }
        let meta = match std::fs::metadata(&file_path) {
            Ok(m) if m.is_file() => m,
            _ => continue,
        };

        let is_encrypted = detect_cfs_magic(&file_path).unwrap_or(false);

        result.push(VolumeFileDto {
            path: file_path.to_string_lossy().to_string(),
            name: file_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            size_bytes: meta.len(),
            is_encrypted,
        });
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Free drive letters & disk space
// ---------------------------------------------------------------------------

/// Returns available (unused) drive letters, C through Z.
#[tauri::command]
pub fn list_free_drive_letters() -> Result<Vec<String>, String> {
    let free = fuse::find_free_drive_letters();
    Ok(free
        .into_iter()
        .filter(|&c| c >= 'C')
        .map(|c| format!("{}:", c))
        .collect())
}

/// Returns free bytes on the disk containing the given path (or the default dir).
#[tauri::command]
pub fn get_disk_free_space(path: Option<String>) -> Result<u64, String> {
    let target = path.unwrap_or_else(default_volumes_dir);
    let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let mut free_bytes: u64 = 0;
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes as *mut u64 as *mut _,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok != 0 {
        Ok(free_bytes)
    } else {
        Err("Cannot query disk space".into())
    }
}

// ---------------------------------------------------------------------------
// KDF Benchmark
// ---------------------------------------------------------------------------

/// Benchmark a KDF configuration and return estimated unlock time in milliseconds.
#[tauri::command]
pub fn benchmark_kdf(
    kdf: &str,
    pbkdf2_iterations: Option<u32>,
    argon2_memory_mib: Option<u32>,
    argon2_time: Option<u32>,
    argon2_parallelism: Option<u32>,
) -> Result<u64, String> {
    let params = match kdf.to_ascii_lowercase().as_str() {
        "pbkdf2" => KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: pbkdf2_iterations.unwrap_or(600_000),
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        },
        _ => KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: argon2_memory_mib.unwrap_or(32) * 1024,
            argon2_time_cost: argon2_time.unwrap_or(2),
            argon2_parallelism: argon2_parallelism.unwrap_or(1),
        },
    };
    let duration = cfs_io::crypto::benchmark_kdf(&params)
        .map_err(|e| format!("Benchmark failed: {e}"))?;
    Ok(duration.as_millis() as u64)
}

// ---------------------------------------------------------------------------
// I/O Benchmark
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct IoBenchmarkResult {
    pub size_label: String,
    pub size_bytes: u64,
    pub write_speed_mbps: f64,
    pub read_speed_mbps: f64,
    pub write_time_ms: f64,
    pub read_time_ms: f64,
    pub sync_time_ms: f64,
}

// ---------------------------------------------------------------------------
// Standalone Format I/O Benchmark (no open volume required)
// ---------------------------------------------------------------------------

/// Cancel a running I/O benchmark.
#[tauri::command]
pub fn cancel_benchmark(state: State<'_, AppState>) {
    state.bench_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Benchmark I/O with a *temporary* volume formatted using the given options.
/// Creates a temp backing file, formats it, runs `runs` iterations of
/// write/read (reusing the same volume), averages the results, then cleans up.
///
/// Fixes:
///   #1/#7 — volume is created once per call (not N times for N runs).
///   #2    — sync() is timed separately and excluded from write throughput.
///   #8    — checks `bench_cancel` between runs so the user can abort.
#[tauri::command]
pub fn benchmark_format_io(
    state: State<'_, AppState>,
    format_options: FormatOptionsDto,
    size_bytes: u64,
    label: String,
    runs: u32,
) -> Result<IoBenchmarkResult, String> {
    if size_bytes == 0 {
        return Err("Benchmark size must be greater than 0".into());
    }
    let runs = runs.max(1);

    // Reset the cancellation flag at the start.
    state.bench_cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let opts = format_options.to_format_options()?;

    // We need a temp volume large enough to hold:
    //   metadata (superblock, inodes, bitmap, journal) + the benchmark data.
    // Add 64 MiB for metadata overhead (superblock, inode table, journals, bitmap).
    // The data is written once then read back in-place — no doubling needed.
    let min_volume_bytes: u64 = (size_bytes + 64 * 1024 * 1024).max(16 * 1024 * 1024);

    // Create a temporary file
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!("__cfs_bench_{}.img", std::process::id()));
    let tmp_path_str = tmp_path.to_string_lossy().to_string();

    // Ensure cleanup even on error
    struct TmpGuard(std::path::PathBuf);
    impl Drop for TmpGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _guard = TmpGuard(tmp_path.clone());

    // Create the backing file
    {
        let f = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("Cannot create temp file: {e}"))?;
        f.set_len(min_volume_bytes)
            .map_err(|e| format!("Cannot allocate temp file: {e}"))?;
    }

    // Open as block device and format
    let dev = FileBlockDevice::open(Path::new(&tmp_path_str), None)
        .map_err(|e| format!("Cannot open temp block device: {e}"))?;
    let vol = CFSVolume::format_v3(Box::new(dev), &opts)
        .map_err(|e| format!("Cannot format temp volume: {e}"))?;

    // Check free space — scope the superblock guard
    let free_bytes = {
        let sb = vol.superblock();
        sb.free_blocks * sb.block_size as u64
    };
    if size_bytes > free_bytes {
        return Err(format!(
            "Temp volume too small for benchmark: need {} bytes but only {} free. Try a smaller test size.",
            size_bytes, free_bytes
        ));
    }

    let bench_path = "/__cfs_format_bench_tmp";

    const MAX_CHUNK: usize = 4 * 1024 * 1024; // 4 MiB
    let chunk_size = (size_bytes as usize).min(MAX_CHUNK);
    let chunk = vec![0xAAu8; chunk_size];

    let mut total_write_us: u64 = 0;
    let mut total_read_us: u64 = 0;
    let mut total_sync_us: u64 = 0;
    let mut completed_runs: u32 = 0;

    for run_idx in 0..runs {
        // ── Cancellation check ──
        if state.bench_cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Benchmark cancelled".into());
        }

        // Clean up any leftover from prior run, then create a fresh file.
        if run_idx > 0 {
            let _ = vol.delete_file(bench_path);
        }
        vol.create_file(bench_path)
            .map_err(|e| format!("Cannot create benchmark file (run {run_idx}): {e}"))?;

        // ── Write benchmark (sync is timed separately) ──
        let write_start = std::time::Instant::now();
        let mut offset: u64 = 0;
        while offset < size_bytes {
            let remaining = (size_bytes - offset) as usize;
            let to_write = remaining.min(chunk_size);
            vol.write_file(bench_path, offset, &chunk[..to_write])
                .map_err(|e| format!("Write failed at offset {offset}: {e}"))?;
            offset += to_write as u64;
        }
        let write_elapsed = write_start.elapsed();

        // ── Sync (measured separately) ──
        let sync_start = std::time::Instant::now();
        vol.sync().map_err(|e| format!("Sync failed: {e}"))?;
        let sync_elapsed = sync_start.elapsed();

        // ── Read benchmark ──
        let read_start = std::time::Instant::now();
        let mut offset: u64 = 0;
        while offset < size_bytes {
            let remaining = (size_bytes - offset) as usize;
            let to_read = remaining.min(chunk_size) as u64;
            let _data = vol.read_file(bench_path, offset, to_read)
                .map_err(|e| format!("Read failed at offset {offset}: {e}"))?;
            offset += to_read;
        }
        let read_elapsed = read_start.elapsed();

        total_write_us += write_elapsed.as_micros() as u64;
        total_read_us += read_elapsed.as_micros() as u64;
        total_sync_us += sync_elapsed.as_micros() as u64;
        completed_runs += 1;
    }

    // Cleanup: delete benchmark file, drop volume.
    // The TmpGuard will delete the backing file on drop.
    let _ = vol.delete_file(bench_path);

    let n = completed_runs as f64;
    let avg_write_us = total_write_us as f64 / n;
    let avg_read_us = total_read_us as f64 / n;
    let avg_sync_us = total_sync_us as f64 / n;
    let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
    let write_secs = avg_write_us / 1_000_000.0;
    let read_secs = avg_read_us / 1_000_000.0;

    Ok(IoBenchmarkResult {
        size_label: label,
        size_bytes,
        write_speed_mbps: if write_secs > 0.0 { size_mb / write_secs } else { 0.0 },
        read_speed_mbps: if read_secs > 0.0 { size_mb / read_secs } else { 0.0 },
        write_time_ms: avg_write_us / 1000.0,
        read_time_ms: avg_read_us / 1000.0,
        sync_time_ms: avg_sync_us / 1000.0,
    })
}
