use anyhow::{bail, Result};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::block_device::CFSBlockDevice;
use super::alloc::BlockAlloc;
use super::file_io::{get_block_ptr, set_block_ptr};
use super::inode::Inode;
use super::superblock::Superblock;
use super::INODE_DIR;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const DIR_ENTRY_SIZE: usize = 128;
pub const MAX_NAME_LEN: usize = 122;

/// Directory entry file_type constants.
pub const DIR_ENTRY_FILE: u8 = 1;
pub const DIR_ENTRY_DIR: u8 = 2;
pub const DIR_ENTRY_SYMLINK: u8 = 3;

/// Magic value identifying a directory checksum record in the last slot of a v3 block.
pub const DIR_CHECKSUM_MAGIC: u32 = 0xDE01C5A0;

// ---------------------------------------------------------------------------
// DirEntry — exactly 128 bytes on disk
// ---------------------------------------------------------------------------

/// On-disk directory entry (128 bytes).
///
/// ```text
/// Offset  Size   Field
/// 0       4      inode_index   u32 — inode this entry points to. 0 = unused/deleted.
/// 4       1      file_type     u8  — 1=FILE, 2=DIR
/// 5       1      name_len      u8  — actual byte length of name (1..=122)
/// 6       122    name          UTF-8, zero-padded to 122 bytes
/// ```
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub inode_index: u32,
    pub file_type: u8,
    pub name_len: u8,
    pub name: [u8; MAX_NAME_LEN],
}

impl DirEntry {
    /// Create a new directory entry. Validates name constraints.
    pub fn new(inode_index: u32, file_type: u8, name: &str) -> Result<Self> {
        validate_name(name)?;
        let name_bytes = name.as_bytes();
        let mut name_buf = [0u8; MAX_NAME_LEN];
        name_buf[..name_bytes.len()].copy_from_slice(name_bytes);
        Ok(Self {
            inode_index,
            file_type,
            name_len: name_bytes.len() as u8,
            name: name_buf,
        })
    }

    /// Serialize to exactly 128 bytes, little-endian.
    pub fn serialize(&self) -> [u8; DIR_ENTRY_SIZE] {
        let mut buf = [0u8; DIR_ENTRY_SIZE];
        buf[0..4].copy_from_slice(&self.inode_index.to_le_bytes());
        buf[4] = self.file_type;
        buf[5] = self.name_len;
        buf[6..6 + MAX_NAME_LEN].copy_from_slice(&self.name);
        buf
    }

    /// Deserialize from exactly 128 bytes.
    pub fn deserialize(buf: &[u8; DIR_ENTRY_SIZE]) -> Self {
        let inode_index = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let file_type = buf[4];
        let name_len = buf[5];
        let mut name = [0u8; MAX_NAME_LEN];
        name.copy_from_slice(&buf[6..6 + MAX_NAME_LEN]);
        Self {
            inode_index,
            file_type,
            name_len,
            name,
        }
    }

    /// Return the name as a UTF-8 string slice.
    pub fn name_str(&self) -> &str {
        std::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("<invalid>")
    }

    /// Whether this entry is unused (deleted or uninitialized).
    /// An entry is unused when inode_index==0 AND name_len==0.
    /// Root inode entries ("."/"..") have inode_index=0 but name_len>0.
    pub fn is_unused(&self) -> bool {
        self.inode_index == 0 && self.name_len == 0
    }
}

/// Validate a name for use in a directory entry.
/// Windows-compatible filename validation.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("name cannot be empty");
    }
    if name.len() > MAX_NAME_LEN {
        bail!("name too long ({} > {})", name.len(), MAX_NAME_LEN);
    }

    // Allow "." and ".." as special directory entries
    if name == "." || name == ".." {
        return Ok(());
    }

    // Reject null byte and path separators
    if name.contains('\0') {
        bail!("name cannot contain null bytes");
    }
    if name.contains('/') || name.contains('\\') {
        bail!("name cannot contain path separators");
    }

    // Reject characters forbidden by Windows: < > : " | ? *
    // Also reject control characters (0x01–0x1F)
    for ch in name.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
            bail!("name contains forbidden character: {:?}", ch);
        }
        if (ch as u32) < 0x20 {
            bail!("name contains control character: U+{:04X}", ch as u32);
        }
    }

    // Reject Windows reserved device names (case-insensitive),
    // with or without an extension: CON, PRN, AUX, NUL,
    // COM0–COM9, LPT0–LPT9
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL",
        "COM0", "COM1", "COM2", "COM3", "COM4",
        "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT0", "LPT1", "LPT2", "LPT3", "LPT4",
        "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem.as_str()) {
        bail!("name is a reserved Windows device name: {}", stem);
    }

    // Reject names that end with a trailing space or dot
    let last = name.chars().last().unwrap();
    if last == ' ' || last == '.' {
        bail!("name cannot end with a space or dot");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Directory Block Checksums (10D.3)
// ---------------------------------------------------------------------------

/// Entries per directory block (v2, no checksum — full block).
pub fn entries_per_block_v2(block_size: u32) -> u32 {
    block_size / DIR_ENTRY_SIZE as u32
}

/// Entries per directory block (v3 — last slot reserved for checksum record).
pub fn entries_per_block_v3(block_size: u32) -> u32 {
    (block_size / DIR_ENTRY_SIZE as u32) - 1
}

/// Check whether a directory block has a v3 checksum record in its last slot.
pub fn block_has_checksum(buf: &[u8], block_size: u32) -> bool {
    let last_slot = (entries_per_block_v2(block_size) - 1) as usize * DIR_ENTRY_SIZE;
    if buf.len() < last_slot + 8 {
        return false;
    }
    let magic = u32::from_le_bytes(buf[last_slot..last_slot + 4].try_into().unwrap());
    magic == DIR_CHECKSUM_MAGIC
}

/// Compute CRC32 for the entry area of a v3 directory block (everything
/// except the last checksum-record slot).
fn compute_dir_block_checksum(buf: &[u8], block_size: u32) -> u32 {
    let data_len = entries_per_block_v3(block_size) as usize * DIR_ENTRY_SIZE;
    crc32fast::hash(&buf[..data_len])
}

/// Stamp a CRC32 checksum record into the last slot of `buf`.
pub fn stamp_checksum(buf: &mut [u8], block_size: u32) {
    let cksum = compute_dir_block_checksum(buf, block_size);
    let last_slot = entries_per_block_v3(block_size) as usize * DIR_ENTRY_SIZE;
    // Zero the slot first
    buf[last_slot..last_slot + DIR_ENTRY_SIZE].fill(0);
    buf[last_slot..last_slot + 4].copy_from_slice(&DIR_CHECKSUM_MAGIC.to_le_bytes());
    buf[last_slot + 4..last_slot + 8].copy_from_slice(&cksum.to_le_bytes());
}

/// Verify the checksum in a v3 directory block. Returns Ok(()) or error.
fn verify_dir_block_checksum(buf: &[u8], block_size: u32, block_addr: u64) -> Result<()> {
    let last_slot = entries_per_block_v3(block_size) as usize * DIR_ENTRY_SIZE;
    if buf.len() < last_slot + 8 {
        bail!("dir block {} truncated: need {} bytes, got {}", block_addr, last_slot + 8, buf.len());
    }
    let stored = u32::from_le_bytes(buf[last_slot + 4..last_slot + 8].try_into().unwrap());
    let computed = compute_dir_block_checksum(buf, block_size);
    if stored != computed {
        bail!(
            "dir block {} checksum mismatch: stored=0x{:08x}, computed=0x{:08x}",
            block_addr, stored, computed
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Directory Operations (3E)
// ---------------------------------------------------------------------------

fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Read all valid (non-unused) entries from a directory inode.
pub fn read_dir_entries(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &Inode,
    block_size: u32,
) -> Result<Vec<DirEntry>> {
    if dir_inode.mode != INODE_DIR {
        bail!("inode is not a directory");
    }

    let bs = block_size as u64;
    let total_entries = dir_inode.size as usize / DIR_ENTRY_SIZE;

    let mut result = Vec::new();
    let mut entries_checked = 0usize;
    let mut logical_block = 0u64;

    while entries_checked < total_entries {
        let physical = get_block_ptr(dev, dir_inode, logical_block, block_size)?;
        if physical == 0 {
            break;
        }

        let mut buf = vec![0u8; bs as usize];
        dev.read(physical * bs, &mut buf)?;

        let is_v3 = block_has_checksum(&buf, block_size);
        if is_v3 {
            verify_dir_block_checksum(&buf, block_size, physical)?;
        }
        let n_entries = if is_v3 {
            entries_per_block_v3(block_size) as usize
        } else {
            bs as usize / DIR_ENTRY_SIZE
        };

        for i in 0..n_entries {
            if entries_checked >= total_entries {
                break;
            }
            let offset = i * DIR_ENTRY_SIZE;
            let entry_buf: &[u8; DIR_ENTRY_SIZE] =
                buf[offset..offset + DIR_ENTRY_SIZE].try_into().unwrap();
            let entry = DirEntry::deserialize(entry_buf);
            if !entry.is_unused() {
                result.push(entry);
            }
            entries_checked += 1;
        }

        logical_block += 1;
    }

    Ok(result)
}

/// Find a directory entry by name. Returns None if not found.
pub fn lookup(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &Inode,
    block_size: u32,
    name: &str,
) -> Result<Option<DirEntry>> {
    if dir_inode.mode != INODE_DIR {
        bail!("inode is not a directory");
    }

    let bs = block_size as u64;
    let total_entries = dir_inode.size as usize / DIR_ENTRY_SIZE;

    let mut entries_checked = 0usize;
    let mut logical_block = 0u64;

    while entries_checked < total_entries {
        let physical = get_block_ptr(dev, dir_inode, logical_block, block_size)?;
        if physical == 0 {
            break;
        }

        let mut buf = vec![0u8; bs as usize];
        dev.read(physical * bs, &mut buf)?;

        let is_v3 = block_has_checksum(&buf, block_size);
        if is_v3 {
            verify_dir_block_checksum(&buf, block_size, physical)?;
        }
        let n_entries = if is_v3 {
            entries_per_block_v3(block_size) as usize
        } else {
            bs as usize / DIR_ENTRY_SIZE
        };

        for i in 0..n_entries {
            if entries_checked >= total_entries {
                break;
            }
            let offset = i * DIR_ENTRY_SIZE;
            let entry_buf: &[u8; DIR_ENTRY_SIZE] =
                buf[offset..offset + DIR_ENTRY_SIZE].try_into().unwrap();
            let entry = DirEntry::deserialize(entry_buf);
            if !entry.is_unused() && entry.name_str() == name {
                return Ok(Some(entry));
            }
            entries_checked += 1;
        }

        logical_block += 1;
    }

    Ok(None)
}

/// Add an entry to a directory. Finds first unused slot or allocates new block.
/// The caller is responsible for reading and writing the directory inode.
pub fn add_dir_entry(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    entry: &DirEntry,
) -> Result<()> {
    let bs = sb.block_size as u64;
    let is_v3 = sb.version >= 3;
    let total_entries = dir_inode.size as usize / DIR_ENTRY_SIZE;

    // Scan existing blocks for an unused slot
    let mut entries_scanned = 0usize;
    let mut logical_block = 0u64;

    while entries_scanned < total_entries {
        let physical = get_block_ptr(dev, dir_inode, logical_block, sb.block_size)?;
        if physical == 0 {
            break;
        }

        let mut buf = vec![0u8; bs as usize];
        dev.read(physical * bs, &mut buf)?;

        let n_entries = if is_v3 || block_has_checksum(&buf, sb.block_size) {
            entries_per_block_v3(sb.block_size) as usize
        } else {
            bs as usize / DIR_ENTRY_SIZE
        };

        for i in 0..n_entries {
            if entries_scanned >= total_entries {
                break;
            }
            let offset = i * DIR_ENTRY_SIZE;
            let existing_buf: &[u8; DIR_ENTRY_SIZE] =
                buf[offset..offset + DIR_ENTRY_SIZE].try_into().unwrap();
            let existing = DirEntry::deserialize(existing_buf);
            if existing.is_unused() {
                // Found a free slot
                buf[offset..offset + DIR_ENTRY_SIZE].copy_from_slice(&entry.serialize());
                if is_v3 {
                    stamp_checksum(&mut buf, sb.block_size);
                }
                dev.write(physical * bs, &buf)?;
                dir_inode.modified = now_timestamp();
                return Ok(());
            }
            entries_scanned += 1;
        }

        logical_block += 1;
    }

    // No free slot — allocate a new block (returns physical address)
    let new_blocks = alloc.alloc(dev, sb, 1)?;
    let new_physical = new_blocks[0];
    set_block_ptr(dev, dir_inode, logical_block, new_physical, sb.block_size, alloc, sb)?;
    dir_inode.block_count += 1;

    // Zero the block, write entry at offset 0
    let mut buf = vec![0u8; bs as usize];
    buf[0..DIR_ENTRY_SIZE].copy_from_slice(&entry.serialize());
    if is_v3 {
        stamp_checksum(&mut buf, sb.block_size);
    }
    dev.write(new_physical * bs, &buf)?;

    // Directory size = total capacity (full blocks of slots)
    dir_inode.size += bs;
    dir_inode.modified = now_timestamp();
    Ok(())
}

/// Remove a directory entry by name (sets inode_index to 0). Does not shrink dir.
pub fn remove_dir_entry(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &Inode,
    block_size: u32,
    name: &str,
) -> Result<()> {
    if dir_inode.mode != INODE_DIR {
        bail!("inode is not a directory");
    }

    let bs = block_size as u64;
    let total_entries = dir_inode.size as usize / DIR_ENTRY_SIZE;

    let mut entries_checked = 0usize;
    let mut logical_block = 0u64;

    while entries_checked < total_entries {
        let physical = get_block_ptr(dev, &dir_inode, logical_block, block_size)?;
        if physical == 0 {
            break;
        }

        let mut buf = vec![0u8; bs as usize];
        dev.read(physical * bs, &mut buf)?;

        let is_v3 = block_has_checksum(&buf, block_size);
        let n_entries = if is_v3 {
            entries_per_block_v3(block_size) as usize
        } else {
            bs as usize / DIR_ENTRY_SIZE
        };

        for i in 0..n_entries {
            if entries_checked >= total_entries {
                break;
            }
            let offset = i * DIR_ENTRY_SIZE;
            let entry_buf: &[u8; DIR_ENTRY_SIZE] =
                buf[offset..offset + DIR_ENTRY_SIZE].try_into().unwrap();
            let entry = DirEntry::deserialize(entry_buf);
            if !entry.is_unused() && entry.name_str() == name {
                // Zero out the entire entry slot (marks as unused)
                buf[offset..offset + DIR_ENTRY_SIZE].fill(0);
                if is_v3 {
                    stamp_checksum(&mut buf, block_size);
                }
                dev.write(physical * bs, &buf)?;
                return Ok(());
            }
            entries_checked += 1;
        }

        logical_block += 1;
    }

    bail!("entry '{}' not found in directory", name)
}

/// Initialize a new directory's first data block with "." and ".." entries.
/// The caller is responsible for reading and writing the directory inode.
pub fn init_dir_block(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    dir_inode_idx: u32,
    parent_inode_idx: u32,
) -> Result<()> {
    let bs = sb.block_size as u64;

    // Allocate first data block (returns physical address)
    let blocks = alloc.alloc(dev, sb, 1)?;
    let physical = blocks[0];
    set_block_ptr(dev, dir_inode, 0, physical, bs as u32, alloc, sb)?;
    dir_inode.block_count = 1;

    // Build "." and ".." entries
    let dot = DirEntry::new(dir_inode_idx, INODE_DIR as u8, ".")?;
    let dotdot = DirEntry::new(parent_inode_idx, INODE_DIR as u8, "..")?;

    let mut buf = vec![0u8; bs as usize];
    buf[0..DIR_ENTRY_SIZE].copy_from_slice(&dot.serialize());
    buf[DIR_ENTRY_SIZE..2 * DIR_ENTRY_SIZE].copy_from_slice(&dotdot.serialize());
    if sb.version >= 3 {
        stamp_checksum(&mut buf, sb.block_size);
    }
    dev.write(physical * bs, &buf)?;

    // Directory size = one full block of slots
    dir_inode.size = bs;
    dir_inode.modified = now_timestamp();
    Ok(())
}

// ---------------------------------------------------------------------------
// HTree-aware dispatch wrappers
// ---------------------------------------------------------------------------

use super::htree::{HTree, should_convert_to_htree};
use super::inode::INODE_FLAG_HTREE;

/// HTree-aware lookup dispatch. If the directory has INODE_FLAG_HTREE, uses
/// the hash-indexed tree; otherwise falls back to linear scan.
pub fn lookup_dispatch(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &Inode,
    dir_inode_idx: u32,
    sb: &Superblock,
    name: &str,
) -> Result<Option<DirEntry>> {
    if dir_inode.flags & INODE_FLAG_HTREE != 0 {
        let htree = HTree::load(dev, dir_inode, dir_inode_idx, sb)?;
        htree.lookup(dev, dir_inode, name)
    } else {
        lookup(dev, dir_inode, sb.block_size, name)
    }
}

/// HTree-aware add_dir_entry dispatch. If the directory is already an HTree,
/// inserts via the hash index. Otherwise, does a linear add and auto-converts
/// to HTree if the directory just grew past 1 block.
pub fn add_dir_entry_dispatch(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &mut Inode,
    dir_inode_idx: u32,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    entry: &DirEntry,
) -> Result<()> {
    if dir_inode.flags & INODE_FLAG_HTREE != 0 {
        let mut htree = HTree::load(dev, dir_inode, dir_inode_idx, sb)?;
        htree.insert(dev, dir_inode, alloc, sb, entry.name_str(), entry.inode_index, entry.file_type)
    } else {
        let old_bc = dir_inode.block_count;
        add_dir_entry(dev, dir_inode, alloc, sb, entry)?;
        // Auto-convert to HTree when the directory just grew past 1 block
        if dir_inode.block_count > old_bc && should_convert_to_htree(dir_inode, sb) {
            HTree::convert_from_linear(dev, dir_inode, dir_inode_idx, alloc, sb)?;
        }
        Ok(())
    }
}

/// HTree-aware remove_dir_entry dispatch.
pub fn remove_dir_entry_dispatch(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &Inode,
    dir_inode_idx: u32,
    sb: &Superblock,
    name: &str,
) -> Result<()> {
    if dir_inode.flags & INODE_FLAG_HTREE != 0 {
        let mut htree = HTree::load(dev, dir_inode, dir_inode_idx, sb)?;
        htree.remove(dev, dir_inode, name)
    } else {
        remove_dir_entry(dev, dir_inode, sb.block_size, name)
    }
}

/// HTree-aware read_dir_entries dispatch.
pub fn read_dir_entries_dispatch(
    dev: &mut dyn CFSBlockDevice,
    dir_inode: &Inode,
    dir_inode_idx: u32,
    sb: &Superblock,
) -> Result<Vec<DirEntry>> {
    if dir_inode.flags & INODE_FLAG_HTREE != 0 {
        let htree = HTree::load(dev, dir_inode, dir_inode_idx, sb)?;
        htree.readdir(dev, dir_inode)
    } else {
        read_dir_entries(dev, dir_inode, sb.block_size)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use crate::volume::{CFSVolume, DEFAULT_BLOCK_SIZE, INODE_FILE, ROOT_INODE};
    use crate::volume::alloc::BlockAlloc;
    use tempfile::NamedTempFile;

    fn make_vol() -> (NamedTempFile, CFSVolume) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();
        let vol = CFSVolume::format(Box::new(dev), DEFAULT_BLOCK_SIZE).unwrap();
        (tmp, vol)
    }

    #[test]
    fn test_dir_entry_serialize_roundtrip() {
        let entry = DirEntry::new(5, INODE_FILE as u8, "hello.txt").unwrap();
        let buf = entry.serialize();
        let entry2 = DirEntry::deserialize(&buf);
        assert_eq!(entry2.inode_index, 5);
        assert_eq!(entry2.file_type, INODE_FILE as u8);
        assert_eq!(entry2.name_str(), "hello.txt");
        assert_eq!(entry2.name_len, 9);
    }

    #[test]
    fn test_dir_entry_max_name() {
        let long_name = "a".repeat(MAX_NAME_LEN);
        let entry = DirEntry::new(1, INODE_FILE as u8, &long_name).unwrap();
        let buf = entry.serialize();
        let entry2 = DirEntry::deserialize(&buf);
        assert_eq!(entry2.name_str(), long_name);
        assert_eq!(entry2.name_len as usize, MAX_NAME_LEN);
    }

    #[test]
    fn test_dir_entry_name_too_long() {
        let too_long = "a".repeat(MAX_NAME_LEN + 1);
        assert!(DirEntry::new(1, INODE_FILE as u8, &too_long).is_err());
    }

    #[test]
    fn test_dir_entry_unused() {
        let entry = DirEntry {
            inode_index: 0,
            file_type: 0,
            name_len: 0,
            name: [0u8; MAX_NAME_LEN],
        };
        assert!(entry.is_unused());

        // Root "." entry has inode_index 0 but name_len > 0 → NOT unused
        let dot = DirEntry::new(0, 2, ".").unwrap();
        assert!(!dot.is_unused());

        let real = DirEntry::new(3, INODE_FILE as u8, "test").unwrap();
        assert!(!real.is_unused());
    }

    #[test]
    fn test_dir_entry_name_validation() {
        // Null byte
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad\0name").is_err());
        // Slash in regular name
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad/name").is_err());
        // Backslash
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad\\name").is_err());
        // Empty name
        assert!(DirEntry::new(1, INODE_FILE as u8, "").is_err());
        // Forbidden Windows characters
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad:name").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad|name").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad?name").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad*name").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad<name").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad>name").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad\"name").is_err());
        // Control characters
        assert!(DirEntry::new(1, INODE_FILE as u8, "bad\x01name").is_err());
        // Trailing space or dot
        assert!(DirEntry::new(1, INODE_FILE as u8, "trailing ").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "trailing.").is_err());
        // Reserved Windows device names
        assert!(DirEntry::new(1, INODE_FILE as u8, "CON").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "NUL").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "COM1").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "LPT9").is_err());
        assert!(DirEntry::new(1, INODE_FILE as u8, "nul.txt").is_err());
        // Valid names
        assert!(DirEntry::new(1, INODE_FILE as u8, "valid.txt").is_ok());
        assert!(DirEntry::new(1, INODE_FILE as u8, "CONSOLE.txt").is_ok()); // not "CON"
        // "." and ".." are allowed
        assert!(DirEntry::new(0, 2, ".").is_ok());
        assert!(DirEntry::new(0, 2, "..").is_ok());
    }

    // -----------------------------------------------------------------------
    // 3E — Directory Operations Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_and_read_entries() {
        let (_tmp, vol) = make_vol();

        // Add 3 file entries
        for name in ["foo.txt", "bar.rs", "baz.md"] {
            let entry = DirEntry::new(1, INODE_FILE as u8, name).unwrap();
            let mut sg = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let mut root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            let ds = vol.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            add_dir_entry(
                &mut **dg, &mut root, &mut alloc,
                &mut *sg, &entry,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, ROOT_INODE, &root).unwrap();
        }

        let mut dg = vol.dev();
        let root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
        let entries = read_dir_entries(
            &mut **dg, &root, vol.block_size,
        ).unwrap();

        // 5 entries: "." + ".." + 3 files
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn test_lookup_found() {
        let (_tmp, vol) = make_vol();

        let entry = DirEntry::new(42, INODE_FILE as u8, "foo.txt").unwrap();
        {
            let mut sg = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let mut root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            let ds = vol.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            add_dir_entry(
                &mut **dg, &mut root, &mut alloc,
                &mut *sg, &entry,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, ROOT_INODE, &root).unwrap();
        }

        let mut dg = vol.dev();
        let root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
        let found = lookup(
            &mut **dg, &root,
            vol.block_size, "foo.txt",
        ).unwrap();

        assert!(found.is_some());
        assert_eq!(found.unwrap().inode_index, 42);
    }

    #[test]
    fn test_lookup_not_found() {
        let (_tmp, vol) = make_vol();

        let mut dg = vol.dev();
        let root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
        let found = lookup(
            &mut **dg, &root,
            vol.block_size, "nonexistent",
        ).unwrap();

        assert!(found.is_none());
    }

    #[test]
    fn test_remove_entry() {
        let (_tmp, vol) = make_vol();

        for (idx, name) in [(1, "a.txt"), (2, "b.txt"), (3, "c.txt")] {
            let entry = DirEntry::new(idx, INODE_FILE as u8, name).unwrap();
            let mut sg = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let mut root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            let ds = vol.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            add_dir_entry(
                &mut **dg, &mut root, &mut alloc,
                &mut *sg, &entry,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, ROOT_INODE, &root).unwrap();
        }

        {
            let mut dg = vol.dev();
            let root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            remove_dir_entry(
                &mut **dg, &root,
                vol.block_size, "b.txt",
            ).unwrap();
        }

        let mut dg = vol.dev();
        let root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
        let entries = read_dir_entries(
            &mut **dg, &root, vol.block_size,
        ).unwrap();

        // 4: "." + ".." + "a.txt" + "c.txt"
        assert_eq!(entries.len(), 4);
        assert!(entries.iter().all(|e| e.name_str() != "b.txt"));
    }

    #[test]
    fn test_dir_entry_slot_reuse() {
        let (_tmp, vol) = make_vol();

        {
            let entry = DirEntry::new(1, INODE_FILE as u8, "temp.txt").unwrap();
            let mut sg = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let mut root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            let ds = vol.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            add_dir_entry(
                &mut **dg, &mut root, &mut alloc,
                &mut *sg, &entry,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, ROOT_INODE, &root).unwrap();
        }

        let blocks_before;
        {
            let mut dg = vol.dev();
            let dir_before = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            blocks_before = dir_before.block_count;

            remove_dir_entry(
                &mut **dg, &dir_before,
                vol.block_size, "temp.txt",
            ).unwrap();
        }

        // Add new entry — should reuse slot, not grow blocks
        {
            let entry2 = DirEntry::new(2, INODE_FILE as u8, "reused.txt").unwrap();
            let mut sg = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let mut root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            let ds = vol.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            add_dir_entry(
                &mut **dg, &mut root, &mut alloc,
                &mut *sg, &entry2,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, ROOT_INODE, &root).unwrap();
        }

        let mut dg = vol.dev();
        let dir_after = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
        assert_eq!(dir_after.block_count, blocks_before);
    }

    #[test]
    fn test_dir_grows_past_one_block() {
        let (_tmp, vol) = make_vol();

        let entries_per_block = vol.block_size as usize / DIR_ENTRY_SIZE;
        // "." and ".." take 2 slots. Fill remaining slots in block 1.
        for i in 0..(entries_per_block - 2) {
            let name = format!("file{:04}", i);
            let entry = DirEntry::new((i + 1) as u32, INODE_FILE as u8, &name).unwrap();
            let mut sg = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let mut root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            let ds = vol.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            add_dir_entry(
                &mut **dg, &mut root, &mut alloc,
                &mut *sg, &entry,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, ROOT_INODE, &root).unwrap();
        }

        {
            let mut dg = vol.dev();
            let dir = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            assert_eq!(dir.block_count, 1);
        }

        // Add one more — should allocate a second block
        {
            let extra = DirEntry::new(99, INODE_FILE as u8, "overflow.txt").unwrap();
            let mut sg = vol.sb_write();
            let mut bm = vol.bitmap_lock();
            let mut dg = vol.dev();
            let mut root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
            let ds = vol.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
            add_dir_entry(
                &mut **dg, &mut root, &mut alloc,
                &mut *sg, &extra,
            ).unwrap();
            vol.inode_table.write_inode(&mut **dg, ROOT_INODE, &root).unwrap();
        }

        let mut dg = vol.dev();
        let dir = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
        assert_eq!(dir.block_count, 2);
    }

    #[test]
    fn test_dot_dotdot_created() {
        let (_tmp, vol) = make_vol();

        let mut dg = vol.dev();
        let root = vol.inode_table.read_inode(&mut **dg, ROOT_INODE).unwrap();
        let entries = read_dir_entries(
            &mut **dg, &root, vol.block_size,
        ).unwrap();

        let dot = entries.iter().find(|e| e.name_str() == ".").unwrap();
        assert_eq!(dot.inode_index, ROOT_INODE);
        assert_eq!(dot.file_type, INODE_DIR as u8);

        let dotdot = entries.iter().find(|e| e.name_str() == "..").unwrap();
        assert_eq!(dotdot.inode_index, ROOT_INODE);
        assert_eq!(dotdot.file_type, INODE_DIR as u8);
    }
}
