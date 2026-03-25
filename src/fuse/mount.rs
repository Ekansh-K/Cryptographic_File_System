use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::Result;
use winfsp::host::{FileSystemHost, VolumeParams};

use crate::volume::CFSVolume;
use super::CfsFs;

// ---------------------------------------------------------------------------
// Drive letter helpers
// ---------------------------------------------------------------------------

/// Return a list of drive letters (A–Z) that are currently free (not mapped).
pub fn find_free_drive_letters() -> Vec<char> {
    // GetLogicalDrives() returns a bitmask: bit 0 = A, bit 1 = B, ...
    let used_mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
    let mut free = Vec::new();
    for i in 0u8..26 {
        if used_mask & (1 << i) == 0 {
            free.push((b'A' + i) as char);
        }
    }
    free
}

/// Pick one free drive letter, preferring letters near the end of the
/// alphabet (Z, Y, X…) to avoid collisions with common drives.
pub fn find_free_drive_letter() -> Option<char> {
    let mut free = find_free_drive_letters();
    free.reverse(); // prefer Z, Y, X…
    // Skip A and B (legacy floppy)
    free.into_iter().find(|&c| c >= 'C')
}

// ---------------------------------------------------------------------------
// Drive icon helpers
// ---------------------------------------------------------------------------

/// Encode a &str as a null-terminated UTF-16 Vec for Win32 APIs.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Returns the icon path to use for a mounted CFS drive.
/// Searches (in order):
///  1. Next to the running executable
///  2. In an `icons/` subdirectory next to the exe (Tauri bundled resources)
///  3. Walking up the directory tree (dev mode: target/debug → project root)
///  4. Fallback: built-in Windows HDD icon
fn resolve_cfs_icon() -> String {
    const NAMES: &[&str] = &["CFS_Drive.ico", "cfs_drive.ico", "cfs.ico"];

    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(std::path::Path::new("."));

        // 1. Directly next to executable
        for name in NAMES {
            let p = dir.join(name);
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }

        // 2. icons/ subdirectory (Tauri bundled resources on Windows)
        for name in NAMES {
            let p = dir.join("icons").join(name);
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }

        // 3. Walk up the directory tree (handles dev mode layout)
        let mut current = dir.to_path_buf();
        for _ in 0..6 {
            match current.parent() {
                Some(parent) => {
                    current = parent.to_path_buf();
                    let candidates = [
                        current.join("src-tauri").join("icons").join("CFS_Drive.ico"),
                        current.join("icons").join("CFS_Drive.ico"),
                        current.join("cfs-ui").join("src-tauri").join("icons").join("CFS_Drive.ico"),
                    ];
                    for c in &candidates {
                        if c.exists() {
                            return c.to_string_lossy().into_owned();
                        }
                    }
                }
                None => break,
            }
        }
    }

    // Fallback: built-in Windows removable/generic drive icon
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    format!(r"{}\System32\imageres.dll,30", root)
}

/// Write HKCU\...\DriveIcons\{X}\DefaultIcon so Explorer shows a custom icon.
/// Non-critical — silently ignores registry errors.
fn set_drive_icon(drive_letter: char) {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW,
        HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    let icon = resolve_cfs_icon();
    let key_path = format!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\DriveIcons\{}\DefaultIcon",
        drive_letter.to_ascii_uppercase()
    );
    let kw = wide(&key_path);
    let iw = wide(&icon);
    let name_w = wide("");
    let mut hkey = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            kw.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        )
    };
    if rc == 0 {
        unsafe {
            RegSetValueExW(
                hkey,
                name_w.as_ptr(),
                0,
                REG_SZ,
                iw.as_ptr().cast(),
                (iw.len() as u32) * 2,
            );
            RegCloseKey(hkey);
        }
        refresh_shell();
    }
}

/// Remove the DriveIcons registry entry on unmount.
fn clear_drive_icon(drive_letter: char) {
    use windows_sys::Win32::System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER};
    let key_path = format!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\DriveIcons\{}",
        drive_letter.to_ascii_uppercase()
    );
    let kw = wide(&key_path);
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, kw.as_ptr()); }
    refresh_shell();
}

/// Tells Explorer to re-read file-type/drive associations immediately.
/// Uses SHCNF_FLUSH (0x1000) so the notification is processed synchronously
/// before we return, ensuring the icon is applied before Explorer renders it.
fn refresh_shell() {
    // SHCNE_ASSOCCHANGED = 0x08000000
    // SHCNF_IDLIST | SHCNF_FLUSH = 0x0000 | 0x1000
    unsafe {
        windows_sys::Win32::UI::Shell::SHChangeNotify(
            0x08000000i32, 0x1000u32,
            std::ptr::null(), std::ptr::null(),
        );
    }
}

// ---------------------------------------------------------------------------
// MountHandle — controls a background WinFSP mount
// ---------------------------------------------------------------------------

/// Handle to a background-mounted WinFSP filesystem. Dropping it stops the
/// dispatcher and unmounts the drive.
pub struct MountHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    drive_letter: String,
}

impl MountHandle {
    pub fn drive_letter(&self) -> &str {
        &self.drive_letter
    }

    pub fn stop(&mut self) {
        // Clear the custom drive icon before stopping the mount
        if let Some(c) = self.drive_letter.chars().next() {
            clear_drive_icon(c);
        }
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for MountHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// WinFSP availability detection
// ---------------------------------------------------------------------------

/// Returns `true` if WinFSP appears to be installed (DLL is loadable).
pub fn is_winfsp_available() -> bool {
    let paths = [
        r"C:\Program Files (x86)\WinFsp\bin\winfsp-x64.dll",
        r"C:\Program Files\WinFsp\bin\winfsp-x64.dll",
    ];
    paths.iter().any(|p| std::path::Path::new(p).exists())
}

// ---------------------------------------------------------------------------
// Background mount (for Tauri / GUI)
// ---------------------------------------------------------------------------

fn build_volume_params(block_size: u32) -> VolumeParams {
    let mut vp = VolumeParams::new();
    vp.sector_size(block_size as u16)
        .sectors_per_allocation_unit(1)
        .max_component_length(122)
        .filesystem_name("CFS")
        .case_sensitive_search(true)
        .case_preserved_names(true)
        .unicode_on_disk(true)
        .persistent_acls(true)
        .reparse_points(true)
        .named_streams(false)
        .extended_attributes(true)
        .post_cleanup_when_modified_only(true)
        .file_info_timeout(1000);
    vp
}

/// Mount a CFS volume in the background. The volume is shared via
/// `Arc<CFSVolume>` so the caller can keep browsing while mounted.
///
/// Returns a `MountHandle` that must be kept alive; dropping it unmounts.
pub fn mount_background(
    vol: Arc<CFSVolume>,
    block_size: u32,
    drive_letter: Option<String>,
) -> Result<MountHandle> {
    // Pre-check WinFSP availability
    if !is_winfsp_available() {
        anyhow::bail!("WinFSP is not installed");
    }

    // Resolve mount point: user-supplied or auto-pick a free letter
    let mount_point = match drive_letter {
        Some(dl) if !dl.is_empty() => normalize_drive_letter(&dl),
        _ => {
            let letter = find_free_drive_letter()
                .ok_or_else(|| anyhow::anyhow!("No free drive letters available"))?;
            format!("{}:", letter)
        }
    };
    let mp_clone = mount_point.clone();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();

    // Channel to signal success/failure from the spawned thread
    let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();

    let thread = std::thread::spawn(move || {
        let result = (|| -> Result<()> {
            let _init = winfsp::winfsp_init_or_die();
            let cfs = CfsFs::from_shared(vol);
            let vp = build_volume_params(block_size);
            let mut host = FileSystemHost::new(vp, cfs)?;

            // Set custom drive icon BEFORE mounting so that Explorer picks
            // it up the moment the drive letter first appears in the shell.
            // Setting it after host.mount() is a race — Explorer often
            // queries and caches the default icon before we can write ours.
            if let Some(c) = mp_clone.chars().next() {
                set_drive_icon(c);
            }

            host.mount(&mp_clone)?;
            host.start()?;

            // Signal success to the caller
            let _ = tx.send(Ok(()));

            // Wait for stop signal
            while !stop_clone.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            host.stop();
            Ok(())
        })();

        if let Err(e) = result {
            let _ = tx.send(Err(e));
        }
    });

    // Wait for the mount thread to report success or failure
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| anyhow::anyhow!("Mount timed out"))?;
    result?;

    Ok(MountHandle {
        stop,
        thread: Some(thread),
        drive_letter: mount_point,
    })
}

// ---------------------------------------------------------------------------
// CLI mount (blocking, existing behavior)
// ---------------------------------------------------------------------------

/// Normalize user input like "X", "x:", "X:\\" into "X:"
fn normalize_drive_letter(input: &str) -> String {
    let s = input.trim().trim_end_matches('\\');
    if s.len() == 1 {
        format!("{}:", s.to_ascii_uppercase())
    } else if s.len() == 2 && s.ends_with(':') {
        s.to_ascii_uppercase()
    } else {
        // Looks like a directory mount point — pass through
        s.to_string()
    }
}

/// Mount a CFS image or raw partition as a Windows drive via WinFSP.
pub fn cmd_mount(
    image: &str,
    block_size: u32,
    mount_point: Option<&str>,
    num_threads: u32,
) -> Result<()> {
    // 1. Initialize WinFSP
    let _init = winfsp::winfsp_init_or_die();

    // 2. Open block device — auto-detects encrypted volumes and prompts for password
    let vol = crate::cli::auto_open_volume(image, block_size)?;

    // 3. Resolve mount point
    let resolved_mp = match mount_point {
        Some(mp) => normalize_drive_letter(mp),
        None => {
            // Show available drive letters and let user pick
            let free = find_free_drive_letters();
            let available: Vec<char> = free.into_iter().filter(|&c| c >= 'C').collect();
            if available.is_empty() {
                anyhow::bail!("No free drive letters available");
            }
            println!("Available drive letters:");
            for chunk in available.chunks(13) {
                let line: Vec<String> = chunk.iter().map(|c| format!("{}:", c)).collect();
                println!("  {}", line.join("  "));
            }
            println!();

            // Default to last available (near end of alphabet)
            let default = *available.last().unwrap();
            eprint!("Drive letter [{}]: ", default);
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let input = input.trim();
            if input.is_empty() {
                format!("{}:", default)
            } else {
                let chosen = input.chars().next().unwrap().to_ascii_uppercase();
                if !available.contains(&chosen) {
                    anyhow::bail!("Drive {} is not available", chosen);
                }
                format!("{}:", chosen)
            }
        }
    };

    // 4. Build CfsFs context
    let cfs = CfsFs::new(vol);

    // 5. Configure VolumeParams
    let vp = build_volume_params(block_size);

    // 6. Create FileSystemHost
    let mut host = FileSystemHost::new(vp, cfs)?;

    // 7. Set icon BEFORE mounting (same race-condition fix as mount_background)
    if let Some(c) = resolved_mp.chars().next() {
        set_drive_icon(c);
    }

    // 8. Mount
    host.mount(&resolved_mp)?;

    // 9. Start dispatcher
    host.start_with_threads(num_threads)?;

    println!("CFS mounted at {}", resolved_mp);
    println!("Press Ctrl+C to unmount...");

    // 9. Wait for Ctrl+C
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).expect("Error setting Ctrl+C handler");

    while running.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // 10. Stop and unmount
    println!("\nUnmounting CFS...");

    // Flush all dirty cached data to disk before unmounting (5J)
    // cfs is moved into the host, so we rely on close() flushing.
    // The host.stop() call will close all outstanding handles which triggers close().

    if let Some(c) = resolved_mp.chars().next() {
        clear_drive_icon(c);
    }
    host.stop();

    println!("CFS unmounted.");
    Ok(())
}
