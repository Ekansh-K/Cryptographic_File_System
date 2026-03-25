pub mod helpers;

use std::ffi::c_void;
use std::sync::Arc;

use winfsp::filesystem::{
    DirBuffer, DirInfo, DirMarker, FileSecurity, FileSystemContext,
    ModificationDescriptor, OpenFileInfo, WideNameInfo,
};
use winfsp::U16CStr;

use crate::volume::CFSVolume;
use crate::volume::{INODE_DIR, INODE_SYMLINK};

use helpers::*;

// Re-export for use in cli and cfs-ui
pub use self::mount::{cmd_mount, mount_background, is_winfsp_available, MountHandle, find_free_drive_letters, find_free_drive_letter};
mod mount;

// ---------------------------------------------------------------------------
// Windows constants
// ---------------------------------------------------------------------------

const FILE_DIRECTORY_FILE: u32 = 0x00000001;

// FspCleanupFlags
const FSP_CLEANUP_DELETE: u32 = 0x01;
const FSP_CLEANUP_SET_LAST_WRITE_TIME: u32 = 0x20;

// ---------------------------------------------------------------------------
// Error mapping: anyhow â†’ NTSTATUS â†’ FspError
// ---------------------------------------------------------------------------

fn ntstatus(code: u32) -> winfsp::FspError {
    // Construct NTSTATUS from the windows crate re-exported through winfsp
    winfsp::FspError::NTSTATUS(code as i32)
}

fn cfs_err_to_fsp(e: &anyhow::Error) -> winfsp::FspError {
    let msg = e.to_string();
    let code = if msg.contains("not found") {
        0xC0000034u32 // STATUS_OBJECT_NAME_NOT_FOUND
    } else if msg.contains("already exists") {
        0xC0000035 // STATUS_OBJECT_NAME_COLLISION
    } else if msg.contains("not a file") {
        0xC00000BA // STATUS_FILE_IS_A_DIRECTORY
    } else if msg.contains("not a directory") {
        0xC0350003 // STATUS_NOT_A_DIRECTORY
    } else if msg.contains("not empty") {
        0xC0000101 // STATUS_DIRECTORY_NOT_EMPTY
    } else if msg.contains("no free inodes") || msg.contains("disk full") || msg.contains("No free blocks") {
        0xC000007F // STATUS_DISK_FULL
    } else {
        0xC0000185 // STATUS_IO_DEVICE_ERROR
    };
    ntstatus(code)
}

fn cfs_to_fsp<T>(result: anyhow::Result<T>) -> winfsp::Result<T> {
    result.map_err(|e| cfs_err_to_fsp(&e))
}

// ---------------------------------------------------------------------------
// CfsFs â€” the main filesystem context
// ---------------------------------------------------------------------------

pub struct CfsFs {
    vol: Arc<CFSVolume>,
}

impl CfsFs {
    pub fn new(vol: CFSVolume) -> Self {
        CfsFs {
            vol: Arc::new(vol),
        }
    }

    /// Create from a shared `Arc<CFSVolume>` â€” used when the volume
    /// needs to be accessed concurrently (e.g., Tauri UI + WinFSP).
    pub fn from_shared(vol: Arc<CFSVolume>) -> Self {
        CfsFs { vol }
    }
}

// ---------------------------------------------------------------------------
// CfsFileContext â€” per-open-file state
// ---------------------------------------------------------------------------

pub struct CfsFileContext {
    pub inode_idx: u32,
    pub is_dir: bool,
    pub path: String,
    pub dir_buffer: DirBuffer,
}

// ---------------------------------------------------------------------------
// Build a minimal self-relative SECURITY_DESCRIPTOR from CFS permission bits.
// This gives Windows a valid SD so tools (icacls, Explorer) see some permissions.
// Layout: SECURITY_DESCRIPTOR_RELATIVE header (20 bytes) + Owner SID + Group SID.
// We map owner to LocalSystem and group to Users.
// ---------------------------------------------------------------------------

fn build_security_descriptor(permissions: u32, _is_dir: bool) -> Vec<u8> {
    // Owner SID: S-1-5-18 (LocalSystem) — 12 bytes
    let owner_sid: [u8; 12] = [
        1, 1, 0, 0, 0, 0, 0, 5,   // revision=1, sub_count=1, NT Authority (5)
        18, 0, 0, 0,               // sub-authority = 18 (LocalSystem)
    ];
    // Group SID: S-1-5-32-545 (Users) — 16 bytes
    let group_sid: [u8; 16] = [
        1, 2, 0, 0, 0, 0, 0, 5,   // revision=1, sub_count=2, NT Authority (5)
        32, 0, 0, 0,               // sub-authority[0] = 32 (BUILTIN)
        33, 2, 0, 0,               // sub-authority[1] = 545 (Users)
    ];

    let header_len: u32 = 20; // SECURITY_DESCRIPTOR_RELATIVE fixed header
    let owner_offset: u32 = header_len;
    let group_offset: u32 = owner_offset + owner_sid.len() as u32;
    let total_len = group_offset as usize + group_sid.len();

    let mut sd = vec![0u8; total_len];
    // Revision = 1
    sd[0] = 1;
    // Sbz1 = 0
    sd[1] = 0;
    // Control (LE u16): SE_SELF_RELATIVE (0x8000) | SE_OWNER_DEFAULTED (0) | SE_DACL_PRESENT (0x0004)
    let control: u16 = 0x8000 | 0x0004;
    sd[2..4].copy_from_slice(&control.to_le_bytes());
    // OffsetOwner (LE u32)
    sd[4..8].copy_from_slice(&owner_offset.to_le_bytes());
    // OffsetGroup (LE u32)
    sd[8..12].copy_from_slice(&group_offset.to_le_bytes());
    // OffsetSacl = 0 (no SACL)
    // OffsetDacl = 0 (no DACL — effectively allows all access)
    // (bytes 12..20 stay zero)

    sd[owner_offset as usize..group_offset as usize].copy_from_slice(&owner_sid);
    sd[group_offset as usize..total_len].copy_from_slice(&group_sid);

    let _ = permissions; // future: build a DACL from rwx bits

    sd
}

// ---------------------------------------------------------------------------
// FileSystemContext implementation
// ---------------------------------------------------------------------------

impl FileSystemContext for CfsFs {
    type FileContext = CfsFileContext;

    // --- Required: get_security_by_name ---
    fn get_security_by_name(
        &self,
        file_name: &U16CStr,
        security_descriptor: Option<&mut [c_void]>,
        _reparse_point_resolver: impl FnOnce(&U16CStr) -> Option<FileSecurity>,
    ) -> winfsp::Result<FileSecurity> {
        let cfs_path = winfsp_path_to_cfs(file_name);

        let inode = match self.vol.stat(&cfs_path) {
            Ok(i) => i,
            Err(e) => return Err(cfs_err_to_fsp(&e)),
        };

        let attributes = inode_mode_to_file_attributes(inode.mode);
        let is_symlink = inode.mode == INODE_SYMLINK;
        let is_dir = inode.mode == INODE_DIR;

        let sd = build_security_descriptor(inode.permissions, is_dir);
        let sd_len = sd.len();

        if let Some(buf) = security_descriptor {
            let copy_len = std::cmp::min(buf.len(), sd_len);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    sd.as_ptr(),
                    buf.as_mut_ptr().cast::<u8>(),
                    copy_len,
                );
            }
        }

        Ok(FileSecurity {
            attributes,
            reparse: is_symlink,
            sz_security_descriptor: sd_len as u64,
        })
    }

    // --- Required: open ---
    fn open(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        _granted_access: u32,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let cfs_path = winfsp_path_to_cfs(file_name);

        let inode_idx = cfs_to_fsp(self.vol.resolve_path(&cfs_path))?;
        let inode = cfs_to_fsp(self.vol.read_inode(inode_idx))?;
        let bs = self.vol.block_size;

        fill_file_info(file_info.as_mut(), &inode, inode_idx, bs);

        Ok(CfsFileContext {
            inode_idx,
            is_dir: inode.mode == INODE_DIR,
            path: cfs_path,
            dir_buffer: DirBuffer::new(),
        })
    }

    // --- Required: close ---
    fn close(&self, _context: Self::FileContext) {
        // Flush dirty cached inodes/blocks to disk so data persists
        // even if the process is subsequently killed (no graceful shutdown).
        let _ = self.vol.sync();
    }

    // --- get_volume_info ---
    fn get_volume_info(&self, out: &mut winfsp::filesystem::VolumeInfo) -> winfsp::Result<()> {
        let sb = &self.vol.superblock();
        out.total_size = sb.total_blocks * sb.block_size as u64;
        out.free_size = sb.free_blocks * sb.block_size as u64;
        out.set_volume_label("CFS");
        Ok(())
    }

    // --- get_file_info ---
    fn get_file_info(
        &self,
        context: &Self::FileContext,
        file_info: &mut winfsp::filesystem::FileInfo,
    ) -> winfsp::Result<()> {
        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;
        let bs = self.vol.block_size;
        fill_file_info(file_info, &inode, context.inode_idx, bs);
        Ok(())
    }

    // --- read ---
    fn read(
        &self,
        context: &Self::FileContext,
        buffer: &mut [u8],
        offset: u64,
    ) -> winfsp::Result<u32> {
        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;

        if offset >= inode.size {
            return Err(ntstatus(0xC0000011)); // STATUS_END_OF_FILE
        }

        let available = inode.size - offset;
        let to_read = std::cmp::min(buffer.len() as u64, available);

        let data = cfs_to_fsp(self.vol.read_file_by_inode(context.inode_idx, offset, to_read))?;
        let bytes_read = data.len();
        buffer[..bytes_read].copy_from_slice(&data);
        Ok(bytes_read as u32)
    }

    // --- write ---
    fn write(
        &self,
        context: &Self::FileContext,
        buffer: &[u8],
        offset: u64,
        write_to_eof: bool,
        constrained_io: bool,
        file_info: &mut winfsp::filesystem::FileInfo,
    ) -> winfsp::Result<u32> {
        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;

        let actual_offset = if write_to_eof { inode.size } else { offset };
        let mut buf = buffer;

        if constrained_io {
            if actual_offset >= inode.size {
                return Ok(0);
            }
            let max_write = (inode.size - actual_offset) as usize;
            if buf.len() > max_write {
                buf = &buf[..max_write];
            }
        }

        cfs_to_fsp(self.vol.write_file_by_inode(context.inode_idx, actual_offset, buf))?;

        // Re-read inode to get updated info
        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;
        let bs = self.vol.block_size;
        fill_file_info(file_info, &inode, context.inode_idx, bs);

        Ok(buf.len() as u32)
    }

    // --- flush ---
    fn flush(
        &self,
        context: Option<&Self::FileContext>,
        file_info: &mut winfsp::filesystem::FileInfo,
    ) -> winfsp::Result<()> {
        cfs_to_fsp(self.vol.sync())?;

        if let Some(ctx) = context {
            let inode = cfs_to_fsp(self.vol.read_inode(ctx.inode_idx))?;
            let bs = self.vol.block_size;
            fill_file_info(file_info, &inode, ctx.inode_idx, bs);
        }
        Ok(())
    }

    // --- create ---
    fn create(
        &self,
        file_name: &U16CStr,
        create_options: u32,
        _granted_access: u32,
        _file_attributes: u32,
        _security_descriptor: Option<&[c_void]>,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        _extra_buffer_is_reparse_point: bool,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let cfs_path = winfsp_path_to_cfs(file_name);

        let is_dir = (create_options & FILE_DIRECTORY_FILE) != 0;

        let inode_idx = if is_dir {
            cfs_to_fsp(self.vol.mkdir(&cfs_path))?
        } else {
            cfs_to_fsp(self.vol.create_file(&cfs_path))?
        };

        cfs_to_fsp(self.vol.sync())?;

        let inode = cfs_to_fsp(self.vol.read_inode(inode_idx))?;
        let bs = self.vol.block_size;
        fill_file_info(file_info.as_mut(), &inode, inode_idx, bs);

        Ok(CfsFileContext {
            inode_idx,
            is_dir,
            path: cfs_path,
            dir_buffer: DirBuffer::new(),
        })
    }

    // --- overwrite ---
    fn overwrite(
        &self,
        context: &Self::FileContext,
        _file_attributes: u32,
        _replace_file_attributes: bool,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        file_info: &mut winfsp::filesystem::FileInfo,
    ) -> winfsp::Result<()> {
        cfs_to_fsp(self.vol.truncate_by_inode(context.inode_idx, 0))?;
        cfs_to_fsp(self.vol.sync())?;

        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;
        let bs = self.vol.block_size;
        fill_file_info(file_info, &inode, context.inode_idx, bs);
        Ok(())
    }

    // --- cleanup ---
    fn cleanup(
        &self,
        context: &Self::FileContext,
        _file_name: Option<&U16CStr>,
        flags: u32,
    ) {
        if flags & FSP_CLEANUP_DELETE != 0 {
                let result = if context.is_dir {
                self.vol.rmdir(&context.path)
            } else {
                self.vol.delete_file(&context.path)
            };
            if result.is_ok() {
                let _ = self.vol.sync();
            }
        }

        if flags & FSP_CLEANUP_SET_LAST_WRITE_TIME != 0 {
                if let Ok(mut inode) = self.vol.read_inode(context.inode_idx) {
                inode.modified = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                let _ = self.vol.write_inode(context.inode_idx, &inode);
                let _ = self.vol.sync();
            }
        }
    }

    // --- set_delete ---
    fn set_delete(
        &self,
        context: &Self::FileContext,
        _file_name: &U16CStr,
        delete_file: bool,
    ) -> winfsp::Result<()> {
        if delete_file && context.is_dir {
                let entries = cfs_to_fsp(self.vol.list_dir(&context.path))?;
            let real = entries.iter()
                .filter(|e| e.name_str() != "." && e.name_str() != "..")
                .count();
            if real > 0 {
                return Err(ntstatus(0xC0000101)); // STATUS_DIRECTORY_NOT_EMPTY
            }
        }
        // WinFSP handles the actual delete_on_close via cleanup flags
        Ok(())
    }

    // --- rename ---
    fn rename(
        &self,
        _context: &Self::FileContext,
        file_name: &U16CStr,
        new_file_name: &U16CStr,
        replace_if_exists: bool,
    ) -> winfsp::Result<()> {
        let old_path = winfsp_path_to_cfs(file_name);
        let new_path = winfsp_path_to_cfs(new_file_name);

        if !replace_if_exists {
            if let Ok(true) = self.vol.exists(&new_path) {
                return Err(ntstatus(0xC0000035)); // STATUS_OBJECT_NAME_COLLISION
            }
        }

        cfs_to_fsp(self.vol.rename(&old_path, &new_path))?;
        cfs_to_fsp(self.vol.sync())?;
        Ok(())
    }

    // --- set_basic_info ---
    fn set_basic_info(
        &self,
        context: &Self::FileContext,
        _file_attributes: u32,
        creation_time: u64,
        _last_access_time: u64,
        last_write_time: u64,
        _last_change_time: u64,
        file_info: &mut winfsp::filesystem::FileInfo,
    ) -> winfsp::Result<()> {
        let mut inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;

        if creation_time != 0 {
            inode.created = filetime_to_unix_ns(creation_time);
        }
        if last_write_time != 0 {
            inode.modified = filetime_to_unix_ns(last_write_time);
        }

        cfs_to_fsp(self.vol.write_inode(context.inode_idx, &inode))?;

        let bs = self.vol.block_size;
        fill_file_info(file_info, &inode, context.inode_idx, bs);
        Ok(())
    }

    // --- set_file_size ---
    fn set_file_size(
        &self,
        context: &Self::FileContext,
        new_size: u64,
        set_allocation_size: bool,
        file_info: &mut winfsp::filesystem::FileInfo,
    ) -> winfsp::Result<()> {

        if !set_allocation_size {
            let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;
            if new_size < inode.size {
                // Shrink
                cfs_to_fsp(self.vol.truncate_by_inode(context.inode_idx, new_size))?;
            } else if new_size > inode.size {
                // Extend by writing zeros
                let zeros = vec![0u8; (new_size - inode.size) as usize];
                cfs_to_fsp(self.vol.write_file_by_inode(context.inode_idx, inode.size, &zeros))?;
            }
        }
        // set_allocation_size is a no-op for CFS

        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;
        let bs = self.vol.block_size;
        fill_file_info(file_info, &inode, context.inode_idx, bs);
        Ok(())
    }

    // --- read_directory ---
    fn read_directory(
        &self,
        context: &Self::FileContext,
        _pattern: Option<&U16CStr>,
        marker: DirMarker,
        buffer: &mut [u8],
    ) -> winfsp::Result<u32> {
        if let Ok(dbl) = context.dir_buffer.acquire(marker.is_none(), None) {
                let entries = cfs_to_fsp(self.vol.list_dir(&context.path))?;
            let bs = self.vol.block_size;

            for entry in &entries {
                let inode = match self.vol.read_inode(entry.inode_index) {
                    Ok(i) => i,
                    Err(_) => continue,
                };

                let name = entry.name_str();
                let wide_name: Vec<u16> = name.encode_utf16().collect();

                let mut dir_info = DirInfo::<255>::new();
                dir_info.set_name_raw(wide_name.as_slice()).ok();
                // We need to set FileInfo on the DirInfo. Since file_info is private,
                // we populate it after creation via the reset method
                {
                    let fi = dir_info.file_info_mut();
                    fill_file_info(fi, &inode, entry.inode_index, bs);
                }

                if dbl.write(&mut dir_info).is_err() {
                    break;
                }
            }
            drop(dbl);
        }

        Ok(context.dir_buffer.read(marker, buffer))
    }

    // --- set_volume_label ---
    fn set_volume_label(
        &self,
        _volume_label: &U16CStr,
        volume_info: &mut winfsp::filesystem::VolumeInfo,
    ) -> winfsp::Result<()> {
        // We ignore volume label changes for now — CFS always reports "CFS"
        self.get_volume_info(volume_info)
    }

    // --- get_security (10D.5: map CFS permissions → Windows SD) ---
    fn get_security(
        &self,
        context: &Self::FileContext,
        security_descriptor: Option<&mut [c_void]>,
    ) -> winfsp::Result<u64> {
        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;
        let sd = build_security_descriptor(inode.permissions, context.is_dir);
        let needed = sd.len() as u64;
        if let Some(buf) = security_descriptor {
            let copy_len = std::cmp::min(buf.len(), sd.len());
            // SAFETY: c_void and u8 have compatible layout; we copy raw
            // bytes of the serialized SECURITY_DESCRIPTOR.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    sd.as_ptr(),
                    buf.as_mut_ptr().cast::<u8>(),
                    copy_len,
                );
            }
        }
        Ok(needed)
    }

    // --- set_security (10D.5: accept but largely ignore for now) ---
    fn set_security(
        &self,
        _context: &Self::FileContext,
        _security_information: u32,
        _modification_descriptor: ModificationDescriptor,
    ) -> winfsp::Result<()> {
        // CFS tracks permissions via its own inode permission bits.
        // For now, accept the request without persisting the Windows SD.
        // Full DACL → CFS permission mapping is possible but complex.
        Ok(())
    }

    // --- get_reparse_point (10G.2: symlink target) ---
    fn get_reparse_point(
        &self,
        context: &Self::FileContext,
        _file_name: &U16CStr,
        buffer: &mut [u8],
    ) -> winfsp::Result<u64> {
        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;
        if inode.mode != INODE_SYMLINK {
            return Err(ntstatus(0xC0000275)); // STATUS_NOT_A_REPARSE_POINT
        }

        let target = cfs_to_fsp(self.vol.readlink(&context.path))?;
        // Convert CFS path separators to Windows
        let win_target = target.replace('/', "\\");
        let wide_target: Vec<u16> = win_target.encode_utf16().collect();
        let target_byte_len = wide_target.len() * 2;

        // Build a REPARSE_DATA_BUFFER for IO_REPARSE_TAG_SYMLINK
        // Header (8 bytes) + SymbolicLinkReparseBuffer (12 bytes header + path data)
        let substitute_offset: u16 = 0;
        let substitute_len = target_byte_len as u16;
        let print_offset = substitute_len;
        let print_len = substitute_len;
        // Flags: 0 = absolute, 1 = relative
        let flags: u32 = 1; // CFS symlinks are always relative

        let data_len: u16 = 12 + (substitute_len + print_len) as u16;
        let total_len = 8 + data_len as usize;

        if buffer.len() < total_len {
            return Err(ntstatus(0xC0000023)); // STATUS_BUFFER_TOO_SMALL
        }

        // Write REPARSE_DATA_BUFFER
        let buf = &mut buffer[..total_len];
        // ReparseTag (u32)
        buf[0..4].copy_from_slice(&IO_REPARSE_TAG_SYMLINK.to_le_bytes());
        // ReparseDataLength (u16)
        buf[4..6].copy_from_slice(&data_len.to_le_bytes());
        // Reserved (u16)
        buf[6..8].copy_from_slice(&0u16.to_le_bytes());
        // SubstituteNameOffset (u16)
        buf[8..10].copy_from_slice(&substitute_offset.to_le_bytes());
        // SubstituteNameLength (u16)
        buf[10..12].copy_from_slice(&substitute_len.to_le_bytes());
        // PrintNameOffset (u16)
        buf[12..14].copy_from_slice(&print_offset.to_le_bytes());
        // PrintNameLength (u16)
        buf[14..16].copy_from_slice(&print_len.to_le_bytes());
        // Flags (u32)
        buf[16..20].copy_from_slice(&flags.to_le_bytes());
        // PathBuffer: substitute name + print name
        let path_start = 20;
        for (i, &w) in wide_target.iter().enumerate() {
            let off = path_start + i * 2;
            buf[off..off + 2].copy_from_slice(&w.to_le_bytes());
        }
        let print_start = path_start + substitute_len as usize;
        for (i, &w) in wide_target.iter().enumerate() {
            let off = print_start + i * 2;
            buf[off..off + 2].copy_from_slice(&w.to_le_bytes());
        }

        Ok(total_len as u64)
    }

    // --- get_reparse_point_by_name ---
    fn get_reparse_point_by_name(
        &self,
        file_name: &U16CStr,
        _is_directory: bool,
        buffer: &mut [u8],
    ) -> winfsp::Result<u64> {
        let cfs_path = winfsp_path_to_cfs(file_name);
        let inode_idx = cfs_to_fsp(self.vol.resolve_path(&cfs_path))?;
        let inode = cfs_to_fsp(self.vol.read_inode(inode_idx))?;

        if inode.mode != INODE_SYMLINK {
            return Err(ntstatus(0xC0000275)); // STATUS_NOT_A_REPARSE_POINT
        }

        let target = cfs_to_fsp(self.vol.readlink(&cfs_path))?;
        let win_target = target.replace('/', "\\");
        let wide_target: Vec<u16> = win_target.encode_utf16().collect();
        let target_byte_len = wide_target.len() * 2;

        let substitute_offset: u16 = 0;
        let substitute_len = target_byte_len as u16;
        let print_offset = substitute_len;
        let print_len = substitute_len;
        let flags: u32 = 1;

        let data_len: u16 = 12 + (substitute_len + print_len) as u16;
        let total_len = 8 + data_len as usize;

        if buffer.len() < total_len {
            return Err(ntstatus(0xC0000023));
        }

        let buf = &mut buffer[..total_len];
        buf[0..4].copy_from_slice(&IO_REPARSE_TAG_SYMLINK.to_le_bytes());
        buf[4..6].copy_from_slice(&data_len.to_le_bytes());
        buf[6..8].copy_from_slice(&0u16.to_le_bytes());
        buf[8..10].copy_from_slice(&substitute_offset.to_le_bytes());
        buf[10..12].copy_from_slice(&substitute_len.to_le_bytes());
        buf[12..14].copy_from_slice(&print_offset.to_le_bytes());
        buf[14..16].copy_from_slice(&print_len.to_le_bytes());
        buf[16..20].copy_from_slice(&flags.to_le_bytes());
        let path_start = 20;
        for (i, &w) in wide_target.iter().enumerate() {
            let off = path_start + i * 2;
            buf[off..off + 2].copy_from_slice(&w.to_le_bytes());
        }
        let print_start = path_start + substitute_len as usize;
        for (i, &w) in wide_target.iter().enumerate() {
            let off = print_start + i * 2;
            buf[off..off + 2].copy_from_slice(&w.to_le_bytes());
        }

        Ok(total_len as u64)
    }

    // --- get_dir_info_by_name (optimized single-entry lookup) ---
    fn get_dir_info_by_name(
        &self,
        context: &Self::FileContext,
        file_name: &U16CStr,
        out: &mut DirInfo<255>,
    ) -> winfsp::Result<()> {
        let name = file_name.to_string_lossy();
        let child_path = if context.path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", context.path, name)
        };

        let inode_idx = cfs_to_fsp(self.vol.resolve_path(&child_path))?;
        let inode = cfs_to_fsp(self.vol.read_inode(inode_idx))?;
        let bs = self.vol.block_size;

        let wide_name: Vec<u16> = name.encode_utf16().collect();
        out.set_name_raw(&*wide_name).ok();
        fill_file_info(out.file_info_mut(), &inode, inode_idx, bs);

        Ok(())
    }

    // --- get_extended_attributes (10G.4: xattr → Windows EA) ---
    fn get_extended_attributes(
        &self,
        context: &Self::FileContext,
        buffer: &mut [u8],
    ) -> winfsp::Result<u32> {
        let keys = cfs_to_fsp(self.vol.list_xattr(&context.path))?;
        if keys.is_empty() {
            return Ok(0);
        }

        let mut offset = 0usize;
        let num_keys = keys.len();
        for (i, key) in keys.iter().enumerate() {
            let value = cfs_to_fsp(self.vol.get_xattr(&context.path, key))?
                .unwrap_or_default();
            let name_bytes = key.as_bytes();
            let name_len = name_bytes.len();
            let val_len = value.len();

            // FILE_FULL_EA_INFORMATION entry size:
            // NextEntryOffset(4) + Flags(1) + EaNameLength(1) + EaValueLength(2) + Name + NUL + Value
            let entry_size = 4 + 1 + 1 + 2 + name_len + 1 + val_len;
            // Align to 4 bytes (except last entry)
            let padded_size = if i + 1 < num_keys {
                (entry_size + 3) & !3
            } else {
                entry_size
            };

            if offset + padded_size > buffer.len() {
                return Err(ntstatus(0xC0000023)); // STATUS_BUFFER_TOO_SMALL
            }

            let next_offset = if i + 1 < num_keys { padded_size as u32 } else { 0 };
            buffer[offset..offset + 4].copy_from_slice(&next_offset.to_le_bytes());
            buffer[offset + 4] = 0; // Flags
            buffer[offset + 5] = name_len as u8;
            buffer[offset + 6..offset + 8].copy_from_slice(&(val_len as u16).to_le_bytes());
            buffer[offset + 8..offset + 8 + name_len].copy_from_slice(name_bytes);
            buffer[offset + 8 + name_len] = 0; // NUL terminator
            if val_len > 0 {
                buffer[offset + 8 + name_len + 1..offset + 8 + name_len + 1 + val_len]
                    .copy_from_slice(&value);
            }
            // Zero padding bytes
            for j in entry_size..padded_size {
                buffer[offset + j] = 0;
            }
            offset += padded_size;
        }

        Ok(offset as u32)
    }

    // --- set_extended_attributes (10G.4: Windows EA → CFS xattr) ---
    fn set_extended_attributes(
        &self,
        context: &Self::FileContext,
        buffer: &[u8],
        file_info: &mut winfsp::filesystem::FileInfo,
    ) -> winfsp::Result<()> {
        // Parse FILE_FULL_EA_INFORMATION entries from buffer
        let mut pos = 0usize;
        loop {
            if pos + 8 > buffer.len() {
                break;
            }
            let next_offset = u32::from_le_bytes([
                buffer[pos], buffer[pos + 1], buffer[pos + 2], buffer[pos + 3],
            ]);
            let name_len = buffer[pos + 5] as usize;
            let val_len = u16::from_le_bytes([buffer[pos + 6], buffer[pos + 7]]) as usize;

            if pos + 8 + name_len + 1 + val_len > buffer.len() {
                break;
            }

            let name = &buffer[pos + 8..pos + 8 + name_len];
            let value = &buffer[pos + 8 + name_len + 1..pos + 8 + name_len + 1 + val_len];

            if let Ok(key) = std::str::from_utf8(name) {
                if val_len == 0 {
                    // Empty value → remove the xattr
                    let _ = self.vol.remove_xattr(&context.path, key);
                } else {
                    cfs_to_fsp(self.vol.set_xattr(&context.path, key, value))?;
                }
            }

            if next_offset == 0 {
                break;
            }
            pos += next_offset as usize;
        }

        // Refresh file info after xattr update
        let inode = cfs_to_fsp(self.vol.read_inode(context.inode_idx))?;
        let bs = self.vol.block_size;
        fill_file_info(file_info, &inode, context.inode_idx, bs);

        Ok(())
    }

    // --- dispatcher_stopped: sync on unmount (5J) ---
    fn dispatcher_stopped(&self, _normally: bool) {
        let _ = self.vol.sync();
    }
}
