use crate::volume::inode::Inode;
use crate::volume::{INODE_DIR, INODE_FILE, INODE_SYMLINK};
use winfsp::filesystem::FileInfo;
use winfsp::U16CStr;

/// Windows reparse tag for symbolic links.
pub const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000000C;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

// ---------------------------------------------------------------------------
// Time conversion
// ---------------------------------------------------------------------------

/// 100-ns ticks between 1601-01-01 and 1970-01-01.
const UNIX_EPOCH_AS_FILETIME: u64 = 116_444_736_000_000_000;
const FILETIME_TICKS_PER_SEC: u64 = 10_000_000;

/// Convert Unix timestamp (seconds since epoch) to Windows FILETIME.
pub fn unix_to_filetime(unix_secs: u64) -> u64 {
    UNIX_EPOCH_AS_FILETIME + unix_secs * FILETIME_TICKS_PER_SEC
}

/// Convert Windows FILETIME to Unix timestamp (seconds). Clamps to 0 if before epoch.
pub fn filetime_to_unix(ft: u64) -> u64 {
    ft.saturating_sub(UNIX_EPOCH_AS_FILETIME) / FILETIME_TICKS_PER_SEC
}

/// Convert nanoseconds since Unix epoch to Windows FILETIME (100-ns intervals since 1601).
pub fn unix_ns_to_filetime(ns: u64) -> u64 {
    let ticks_since_unix = ns / 100;
    ticks_since_unix + UNIX_EPOCH_AS_FILETIME
}

/// Convert Windows FILETIME to nanoseconds since Unix epoch.
pub fn filetime_to_unix_ns(filetime: u64) -> u64 {
    if filetime < UNIX_EPOCH_AS_FILETIME {
        return 0;
    }
    let ticks_since_unix = filetime - UNIX_EPOCH_AS_FILETIME;
    ticks_since_unix.saturating_mul(100)
}

// ---------------------------------------------------------------------------
// Path conversion
// ---------------------------------------------------------------------------

/// Convert a WinFSP U16CStr path (`\`-separated) to a CFS path (`/`-separated).
/// WinFSP sends paths like `\foo\bar`; CFS uses `/foo/bar`.
/// Root `\` becomes `/`.
pub fn winfsp_path_to_cfs(path: &U16CStr) -> String {
    let s = path.to_string_lossy();
    let converted = s.replace('\\', "/");
    if converted.is_empty() {
        "/".to_string()
    } else {
        converted
    }
}

// ---------------------------------------------------------------------------
// Attribute mapping
// ---------------------------------------------------------------------------

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

/// Map CFS inode mode to Windows FILE_ATTRIBUTE_XXX flags.
pub fn inode_mode_to_file_attributes(mode: u16) -> u32 {
    match mode {
        x if x == INODE_DIR => FILE_ATTRIBUTE_DIRECTORY,
        x if x == INODE_SYMLINK => FILE_ATTRIBUTE_REPARSE_POINT,
        _ => FILE_ATTRIBUTE_NORMAL,
    }
}

// ---------------------------------------------------------------------------
// FileInfo population
// ---------------------------------------------------------------------------

/// Populate a WinFSP FileInfo from a CFS Inode.
/// Uses nanosecond timestamps (v3) with fallback to seconds (v2).
pub fn fill_file_info(info: &mut FileInfo, inode: &Inode, inode_idx: u32, block_size: u32) {
    info.file_attributes = inode_mode_to_file_attributes(inode.mode);
    info.file_size = inode.size;
    info.allocation_size = inode.block_count_u64() * block_size as u64;
    info.creation_time = unix_ns_to_filetime(inode.created);
    info.last_access_time = unix_ns_to_filetime(inode.accessed_ns);
    info.last_write_time = unix_ns_to_filetime(inode.modified);
    info.change_time = unix_ns_to_filetime(inode.changed_ns);
    info.index_number = inode_idx as u64;
    info.hard_links = inode.nlinks as u32;
    info.reparse_tag = if inode.mode == INODE_SYMLINK {
        IO_REPARSE_TAG_SYMLINK
    } else {
        0
    };
    info.ea_size = 0;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_to_filetime_roundtrip() {
        let unix = 1_700_000_000u64; // approx 2023-11-14
        let ft = unix_to_filetime(unix);
        let back = filetime_to_unix(ft);
        assert_eq!(back, unix);
    }

    #[test]
    fn test_filetime_to_unix_before_epoch() {
        // FILETIME value before Unix epoch should clamp to 0
        let ft = 100u64;
        assert_eq!(filetime_to_unix(ft), 0);
    }

    #[test]
    fn test_winfsp_path_root() {
        let path = winfsp::U16CString::from_str("\\").unwrap();
        assert_eq!(winfsp_path_to_cfs(&path), "/");
    }

    #[test]
    fn test_winfsp_path_nested() {
        let path = winfsp::U16CString::from_str("\\foo\\bar\\baz.txt").unwrap();
        assert_eq!(winfsp_path_to_cfs(&path), "/foo/bar/baz.txt");
    }

    #[test]
    fn test_inode_mode_to_attributes_dir() {
        assert_eq!(inode_mode_to_file_attributes(INODE_DIR), FILE_ATTRIBUTE_DIRECTORY);
    }

    #[test]
    fn test_inode_mode_to_attributes_file() {
        assert_eq!(inode_mode_to_file_attributes(INODE_FILE), FILE_ATTRIBUTE_NORMAL);
    }

    #[test]
    fn test_fill_file_info_basic() {
        // Use nanosecond-scale timestamps (v3 format)
        let created_ns: u64 = 1_700_000_000 * 1_000_000_000; // 2023-11-14 in ns
        let modified_ns: u64 = 1_700_001_000 * 1_000_000_000;
        let inode = Inode {
            mode: INODE_FILE,
            nlinks: 1,
            block_count: 2,
            size: 5000,
            created: created_ns,
            modified: modified_ns,
            direct_blocks: [0; 10],
            indirect_block: 0,
            double_indirect: 0,
            accessed_ns: created_ns,
            changed_ns: modified_ns,
            owner_id: 0,
            group_id: 0,
            permissions: 0o644,
            flags: 0,
            xattr_block: 0,
            xattr_inline_size: 0,
            checksum: 0,
            block_count_hi: 0,
            inline_area: [0u8; 76],
        };
        let mut info = FileInfo::default();
        fill_file_info(&mut info, &inode, 42, 4096);
        assert_eq!(info.file_size, 5000);
        assert_eq!(info.allocation_size, 2 * 4096);
        assert_eq!(info.file_attributes, FILE_ATTRIBUTE_NORMAL);
        assert_eq!(info.index_number, 42);
        assert_eq!(info.creation_time, unix_ns_to_filetime(created_ns));
        assert_eq!(info.last_write_time, unix_ns_to_filetime(modified_ns));
        assert_eq!(info.last_access_time, unix_ns_to_filetime(created_ns));
        assert_eq!(info.change_time, unix_ns_to_filetime(modified_ns));
    }
}
