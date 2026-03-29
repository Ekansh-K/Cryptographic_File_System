use anyhow::{bail, Result};

use crate::block_device::CFSBlockDevice;
use super::alloc::BlockAlloc;
use super::inode::{Inode, INODE_FLAG_INLINE_DATA, INODE_FLAG_HAS_XATTR};
use super::superblock::Superblock;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const XATTR_ENTRY_HEADER_SIZE: usize = 3; // key_len(1) + value_len(2)
pub const XATTR_MAX_KEY_LEN: usize = 255;
pub const XATTR_MAX_VALUE_LEN: usize = 65535;
pub const MAX_INLINE_XATTR_SIZE: usize = 76; // inline_area capacity
pub const XATTR_BLOCK_MAGIC: u32 = 0xEA02_0CFF;
pub const XATTR_BLOCK_HEADER_SIZE: usize = 16;

/// Valid xattr namespace prefixes.
const XATTR_NAMESPACES: &[&str] = &["user.", "security.", "system."];

// ---------------------------------------------------------------------------
// XattrEntry — variable-length packed entry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XattrEntry {
    pub key: String,
    pub value: Vec<u8>,
}

impl XattrEntry {
    pub fn serialized_size(&self) -> usize {
        XATTR_ENTRY_HEADER_SIZE + self.key.len() + self.value.len()
    }

    pub fn serialize(&self) -> Vec<u8> {
        let total = self.serialized_size();
        let mut buf = Vec::with_capacity(total);
        buf.push(self.key.len() as u8);
        buf.extend_from_slice(&(self.value.len() as u16).to_le_bytes());
        buf.extend_from_slice(self.key.as_bytes());
        buf.extend_from_slice(&self.value);
        buf
    }

    pub fn deserialize(buf: &[u8]) -> Result<(Self, usize)> {
        if buf.len() < XATTR_ENTRY_HEADER_SIZE {
            bail!("xattr entry too short");
        }
        let key_len = buf[0] as usize;
        let value_len = u16::from_le_bytes(buf[1..3].try_into().unwrap()) as usize;

        if key_len == 0 {
            bail!("xattr key length cannot be zero");
        }
        let total = XATTR_ENTRY_HEADER_SIZE + key_len + value_len;
        if buf.len() < total {
            bail!("xattr entry data truncated");
        }

        let key = String::from_utf8(buf[3..3 + key_len].to_vec())
            .map_err(|_| anyhow::anyhow!("xattr key is not valid UTF-8"))?;
        let value = buf[3 + key_len..3 + key_len + value_len].to_vec();

        Ok((XattrEntry { key, value }, total))
    }
}

// ---------------------------------------------------------------------------
// XattrBlockHeader — on-disk header for external xattr block
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct XattrBlockHeader {
    magic: u32,
    checksum: u32,
    num_entries: u32,
    total_size: u32,
}

impl XattrBlockHeader {
    fn new() -> Self {
        Self {
            magic: XATTR_BLOCK_MAGIC,
            checksum: 0,
            num_entries: 0,
            total_size: 0,
        }
    }

    fn serialize(&self) -> [u8; XATTR_BLOCK_HEADER_SIZE] {
        let mut buf = [0u8; XATTR_BLOCK_HEADER_SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&self.checksum.to_le_bytes());
        buf[8..12].copy_from_slice(&self.num_entries.to_le_bytes());
        buf[12..16].copy_from_slice(&self.total_size.to_le_bytes());
        buf
    }

    fn deserialize(buf: &[u8]) -> Result<Self> {
        if buf.len() < XATTR_BLOCK_HEADER_SIZE {
            bail!("xattr block header too short");
        }
        let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        if magic != XATTR_BLOCK_MAGIC {
            bail!("invalid xattr block magic: 0x{:08X}", magic);
        }
        Ok(Self {
            magic,
            checksum: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
            num_entries: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
            total_size: u32::from_le_bytes(buf[12..16].try_into().unwrap()),
        })
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate that an xattr key has a recognized namespace prefix.
pub fn validate_xattr_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > XATTR_MAX_KEY_LEN {
        bail!("xattr key length must be 1..{}", XATTR_MAX_KEY_LEN);
    }

    if !XATTR_NAMESPACES.iter().any(|ns| key.starts_with(ns)) {
        bail!(
            "xattr key '{}' must start with one of: {}",
            key,
            XATTR_NAMESPACES.join(", ")
        );
    }

    if key.bytes().any(|b| b == 0) {
        bail!("xattr key cannot contain NUL bytes");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse packed xattr entries from a byte slice.
fn parse_xattr_entries(data: &[u8]) -> Result<Vec<XattrEntry>> {
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        // Check if remaining bytes are all zeros (end of entries)
        if data[offset..].iter().all(|&b| b == 0) {
            break;
        }
        let (entry, consumed) = XattrEntry::deserialize(&data[offset..])?;
        entries.push(entry);
        offset += consumed;
    }
    Ok(entries)
}

/// Read and parse xattr entries from an external xattr block on disk.
fn read_external_xattr_block(
    dev: &mut dyn CFSBlockDevice,
    block_addr: u64,
    block_size: u32,
) -> Result<Vec<XattrEntry>> {
    let mut buf = vec![0u8; block_size as usize];
    dev.read(block_addr * block_size as u64, &mut buf)?;

    let header = XattrBlockHeader::deserialize(&buf)?;

    // Verify checksum
    let stored_checksum = header.checksum;
    let mut check_buf = buf.clone();
    check_buf[4..8].copy_from_slice(&0u32.to_le_bytes());
    let computed = crc32fast::hash(&check_buf);
    if stored_checksum != computed {
        bail!("xattr block checksum mismatch");
    }

    let end = XATTR_BLOCK_HEADER_SIZE + header.total_size as usize;
    if end > block_size as usize {
        bail!("xattr block total_size exceeds block");
    }
    parse_xattr_entries(&buf[XATTR_BLOCK_HEADER_SIZE..end])
}

/// Write xattr entries to an external xattr block on disk.
fn write_external_xattr_block(
    dev: &mut dyn CFSBlockDevice,
    block_addr: u64,
    block_size: u32,
    entries: &[&XattrEntry],
) -> Result<()> {
    let mut buf = vec![0u8; block_size as usize];

    // Serialize entries
    let mut offset = XATTR_BLOCK_HEADER_SIZE;
    for entry in entries {
        let data = entry.serialize();
        buf[offset..offset + data.len()].copy_from_slice(&data);
        offset += data.len();
    }

    // Write header
    let mut header = XattrBlockHeader::new();
    header.num_entries = entries.len() as u32;
    header.total_size = (offset - XATTR_BLOCK_HEADER_SIZE) as u32;
    let header_bytes = header.serialize();
    buf[..XATTR_BLOCK_HEADER_SIZE].copy_from_slice(&header_bytes);

    // Compute checksum (with checksum field zeroed)
    buf[4..8].copy_from_slice(&0u32.to_le_bytes());
    let checksum = crc32fast::hash(&buf);
    buf[4..8].copy_from_slice(&checksum.to_le_bytes());

    dev.write(block_addr * block_size as u64, &buf)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Collect all xattrs from an inode
// ---------------------------------------------------------------------------

fn collect_all_xattrs(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    sb: &Superblock,
) -> Result<Vec<XattrEntry>> {
    let mut entries = Vec::new();

    // Inline xattrs
    if inode.xattr_inline_size > 0 && inode.flags & INODE_FLAG_INLINE_DATA == 0 {
        if inode.xattr_inline_size as usize > MAX_INLINE_XATTR_SIZE {
            bail!(
                "xattr_inline_size {} exceeds MAX_INLINE_XATTR_SIZE {}",
                inode.xattr_inline_size, MAX_INLINE_XATTR_SIZE
            );
        }
        let inline = parse_xattr_entries(
            &inode.inline_area[..inode.xattr_inline_size as usize],
        )?;
        entries.extend(inline);
    }

    // External xattr block
    if inode.xattr_block != 0 {
        let external = read_external_xattr_block(dev, inode.xattr_block, sb.block_size)?;
        entries.extend(external);
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// Write xattrs back (inline + external)
// ---------------------------------------------------------------------------

fn write_xattrs_back(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    sb: &mut Superblock,
    alloc: &mut BlockAlloc<'_>,
    entries: &[XattrEntry],
) -> Result<()> {
    if entries.is_empty() {
        // Clear all xattr state
        inode.xattr_inline_size = 0;
        inode.inline_area = [0u8; MAX_INLINE_XATTR_SIZE];
        inode.flags &= !INODE_FLAG_HAS_XATTR;

        // Free external block if present
        if inode.xattr_block != 0 {
            alloc.free(dev, sb, &[inode.xattr_block])?;
            inode.xattr_block = 0;
        }

        inode.touch_ctime();
        return Ok(());
    }

    // Can we use inline area?
    let can_use_inline = inode.flags & INODE_FLAG_INLINE_DATA == 0;

    let mut inline_entries: Vec<&XattrEntry> = Vec::new();
    let mut external_entries: Vec<&XattrEntry> = Vec::new();
    let mut inline_used = 0usize;

    if can_use_inline {
        for entry in entries {
            let size = entry.serialized_size();
            if inline_used + size <= MAX_INLINE_XATTR_SIZE {
                inline_entries.push(entry);
                inline_used += size;
            } else {
                external_entries.push(entry);
            }
        }
    } else {
        // All entries go external (inline_area occupied by inline data)
        external_entries.extend(entries.iter());
    }

    // Write inline xattrs
    inode.inline_area = [0u8; MAX_INLINE_XATTR_SIZE];
    inode.xattr_inline_size = 0;
    let mut offset = 0usize;
    for entry in &inline_entries {
        let data = entry.serialize();
        inode.inline_area[offset..offset + data.len()].copy_from_slice(&data);
        offset += data.len();
    }
    inode.xattr_inline_size = offset as u32;

    // Write external xattr block
    if !external_entries.is_empty() {
        // Check total external size fits in one block
        let total_external: usize = external_entries.iter()
            .map(|e| e.serialized_size())
            .sum();
        let max_external = sb.block_size as usize - XATTR_BLOCK_HEADER_SIZE;
        if total_external > max_external {
            bail!(
                "total xattr size ({} bytes) exceeds block capacity ({} bytes)",
                total_external + inline_used,
                MAX_INLINE_XATTR_SIZE + max_external
            );
        }

        // Allocate external block if not already present
        if inode.xattr_block == 0 {
            let blocks = alloc.alloc(dev, sb, 1)?;
            inode.xattr_block = blocks[0];
        }

        write_external_xattr_block(dev, inode.xattr_block, sb.block_size, &external_entries)?;
    } else if inode.xattr_block != 0 {
        // No external entries needed — free the block
        alloc.free(dev, sb, &[inode.xattr_block])?;
        inode.xattr_block = 0;
    }

    // Update flags
    if !inline_entries.is_empty() || !external_entries.is_empty() {
        inode.flags |= INODE_FLAG_HAS_XATTR;
    } else {
        inode.flags &= !INODE_FLAG_HAS_XATTR;
    }

    inode.touch_ctime();
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API (called from CFSVolume)
// ---------------------------------------------------------------------------

/// Get the value of an extended attribute.
pub fn get_xattr(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    sb: &Superblock,
    key: &str,
) -> Result<Option<Vec<u8>>> {
    let entries = collect_all_xattrs(dev, inode, sb)?;
    Ok(entries.into_iter().find(|e| e.key == key).map(|e| e.value))
}

/// Set an extended attribute (creates or updates).
pub fn set_xattr(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    sb: &mut Superblock,
    alloc: &mut BlockAlloc<'_>,
    key: &str,
    value: &[u8],
) -> Result<()> {
    let mut all_entries = collect_all_xattrs(dev, inode, sb)?;

    // Remove old entry with same key
    all_entries.retain(|e| e.key != key);

    // Add new entry
    all_entries.push(XattrEntry {
        key: key.to_string(),
        value: value.to_vec(),
    });

    write_xattrs_back(dev, inode, sb, alloc, &all_entries)
}

/// List all extended attribute keys.
pub fn list_xattr(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    sb: &Superblock,
) -> Result<Vec<String>> {
    let entries = collect_all_xattrs(dev, inode, sb)?;
    Ok(entries.into_iter().map(|e| e.key).collect())
}

/// Remove an extended attribute.
pub fn remove_xattr(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    sb: &mut Superblock,
    alloc: &mut BlockAlloc<'_>,
    key: &str,
) -> Result<()> {
    let mut all_entries = collect_all_xattrs(dev, inode, sb)?;
    let len_before = all_entries.len();
    all_entries.retain(|e| e.key != key);

    if all_entries.len() == len_before {
        bail!("xattr '{}' not found", key);
    }

    write_xattrs_back(dev, inode, sb, alloc, &all_entries)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xattr_entry_serialize_roundtrip() {
        let entry = XattrEntry {
            key: "user.tag".to_string(),
            value: b"hello".to_vec(),
        };
        let data = entry.serialize();
        let (parsed, consumed) = XattrEntry::deserialize(&data).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(consumed, data.len());
    }

    #[test]
    fn test_xattr_entry_binary_value() {
        let entry = XattrEntry {
            key: "user.bin".to_string(),
            value: vec![0x00, 0xFF, 0x80, 0x01],
        };
        let data = entry.serialize();
        let (parsed, _) = XattrEntry::deserialize(&data).unwrap();
        assert_eq!(parsed.value, vec![0x00, 0xFF, 0x80, 0x01]);
    }

    #[test]
    fn test_xattr_validate_key() {
        assert!(validate_xattr_key("user.tag").is_ok());
        assert!(validate_xattr_key("security.cap").is_ok());
        assert!(validate_xattr_key("system.acl").is_ok());
        assert!(validate_xattr_key("invalid.key").is_err());
        assert!(validate_xattr_key("").is_err());
        let long_key = format!("user.{}", "x".repeat(300));
        assert!(validate_xattr_key(&long_key).is_err());
    }

    #[test]
    fn test_xattr_parse_multiple_entries() {
        let e1 = XattrEntry { key: "user.a".to_string(), value: b"1".to_vec() };
        let e2 = XattrEntry { key: "user.b".to_string(), value: b"22".to_vec() };
        let mut buf = Vec::new();
        buf.extend(e1.serialize());
        buf.extend(e2.serialize());
        // Pad with zeros
        buf.resize(76, 0);
        let entries = parse_xattr_entries(&buf).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "user.a");
        assert_eq!(entries[1].key, "user.b");
    }

    #[test]
    fn test_xattr_entry_zero_key_rejected() {
        let buf = [0u8; 10];
        assert!(XattrEntry::deserialize(&buf).is_err());
    }
}
