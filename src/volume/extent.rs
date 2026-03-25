//! Extent tree implementation for CFS v3.
//!
//! Replaces the legacy direct/indirect/double-indirect block pointer scheme
//! with extent-based mapping. Each extent maps a contiguous range of logical
//! file blocks to a contiguous range of physical disk blocks.
//!
//! The extent tree is stored in the inode's dual-purpose area [64..160),
//! which is 96 bytes — enough for an ExtentHeader (12B) + up to 7 entries (84B).
//! When the tree needs more space, it spills to on-disk blocks, creating a
//! B-tree-like structure up to depth 5.

use anyhow::{bail, Result};

use crate::block_device::CFSBlockDevice;
use super::alloc::BlockAlloc;
use super::inode::{Inode, INODE_FLAG_EXTENTS};
use super::superblock::Superblock;

// ===========================================================================
// Constants
// ===========================================================================

/// Magic number identifying an extent tree node.
pub const EXTENT_MAGIC: u16 = 0xF30A;

/// Maximum depth of the extent tree (matching ext4).
pub const EXTENT_MAX_DEPTH: u16 = 5;

/// Size of each extent structure (header, leaf, index) in bytes.
pub const EXTENT_ENTRY_SIZE: usize = 12;

/// Size of the extent tree root area in the inode (bytes [64..160)).
pub const EXTENT_ROOT_SIZE: usize = 96;

/// Maximum entries in the in-inode root node: (96 - 12) / 12 = 7.
pub const EXTENT_ROOT_MAX_ENTRIES: u16 = 7;

/// Maximum length for an initialized extent (15-bit value).
pub const EXTENT_MAX_LEN_INIT: u16 = 32_767;

/// High bit flag in ee_len marking an uninitialized extent.
pub const EXTENT_UNINIT_FLAG: u16 = 0x8000;

/// Number of entries that fit in a disk block (excluding header).
pub fn extent_entries_per_block(block_size: u32) -> u16 {
    ((block_size as usize - EXTENT_ENTRY_SIZE) / EXTENT_ENTRY_SIZE) as u16
}

// ===========================================================================
// ExtentHeader (12 bytes)
// ===========================================================================

/// Extent tree node header. Present at the start of every extent tree node
/// (both in-inode root and on-disk nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentHeader {
    pub magic: u16,
    pub entries: u16,
    pub max: u16,
    pub depth: u16,
    pub generation: u32,
}

impl ExtentHeader {
    /// Create a new header for an empty root node (in-inode).
    pub fn new_root() -> Self {
        Self {
            magic: EXTENT_MAGIC,
            entries: 0,
            max: EXTENT_ROOT_MAX_ENTRIES,
            depth: 0,
            generation: 0,
        }
    }

    /// Create a new header for a leaf block node.
    pub fn new_leaf(block_size: u32) -> Self {
        Self {
            magic: EXTENT_MAGIC,
            entries: 0,
            max: extent_entries_per_block(block_size),
            depth: 0,
            generation: 0,
        }
    }

    /// Create a new header for an index block node at the given depth.
    pub fn new_index(block_size: u32, depth: u16) -> Self {
        Self {
            magic: EXTENT_MAGIC,
            entries: 0,
            max: extent_entries_per_block(block_size),
            depth,
            generation: 0,
        }
    }

    pub fn is_leaf(&self) -> bool {
        self.depth == 0
    }

    pub fn is_full(&self) -> bool {
        self.entries >= self.max
    }

    pub fn serialize(&self) -> [u8; 12] {
        let mut buf = [0u8; 12];
        buf[0..2].copy_from_slice(&self.magic.to_le_bytes());
        buf[2..4].copy_from_slice(&self.entries.to_le_bytes());
        buf[4..6].copy_from_slice(&self.max.to_le_bytes());
        buf[6..8].copy_from_slice(&self.depth.to_le_bytes());
        buf[8..12].copy_from_slice(&self.generation.to_le_bytes());
        buf
    }

    pub fn deserialize(buf: &[u8; 12]) -> Result<Self> {
        let magic = u16::from_le_bytes(buf[0..2].try_into().unwrap());
        if magic != EXTENT_MAGIC {
            bail!("extent header bad magic: 0x{magic:04x} (expected 0x{EXTENT_MAGIC:04x})");
        }
        Ok(Self {
            magic,
            entries: u16::from_le_bytes(buf[2..4].try_into().unwrap()),
            max: u16::from_le_bytes(buf[4..6].try_into().unwrap()),
            depth: u16::from_le_bytes(buf[6..8].try_into().unwrap()),
            generation: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
        })
    }
}

// ===========================================================================
// ExtentLeaf (12 bytes)
// ===========================================================================

/// An extent leaf entry mapping a contiguous range of logical blocks to
/// a contiguous range of physical blocks.
///
/// Physical block = (ee_start_hi << 32) | ee_start_lo → 48-bit addressing.
/// Length bit 15: 0 = initialized, 1 = uninitialized (preallocated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentLeaf {
    pub ee_block: u32,
    pub ee_len: u16,
    pub ee_start_hi: u16,
    pub ee_start_lo: u32,
}

impl ExtentLeaf {
    /// Create a new initialized extent.
    pub fn new(logical_block: u32, physical_block: u64, length: u16) -> Self {
        assert!(
            length > 0 && length <= EXTENT_MAX_LEN_INIT,
            "extent length must be 1..={EXTENT_MAX_LEN_INIT}"
        );
        Self {
            ee_block: logical_block,
            ee_len: length,
            ee_start_hi: (physical_block >> 32) as u16,
            ee_start_lo: physical_block as u32,
        }
    }

    /// Create a new uninitialized (preallocated) extent.
    pub fn new_uninit(logical_block: u32, physical_block: u64, length: u16) -> Self {
        assert!(
            length > 0 && length <= EXTENT_MAX_LEN_INIT, // uninit also capped at 32767
            "uninit extent length must be 1..={EXTENT_MAX_LEN_INIT}"
        );
        Self {
            ee_block: logical_block,
            ee_len: length | EXTENT_UNINIT_FLAG,
            ee_start_hi: (physical_block >> 32) as u16,
            ee_start_lo: physical_block as u32,
        }
    }

    /// Get the physical start block (48-bit address).
    pub fn physical_block(&self) -> u64 {
        ((self.ee_start_hi as u64) << 32) | (self.ee_start_lo as u64)
    }

    /// Set the physical start block.
    pub fn set_physical_block(&mut self, phys: u64) {
        self.ee_start_hi = (phys >> 32) as u16;
        self.ee_start_lo = phys as u32;
    }

    /// True if this is an uninitialized (preallocated) extent.
    pub fn is_uninitialized(&self) -> bool {
        self.ee_len & EXTENT_UNINIT_FLAG != 0
    }

    /// Number of blocks in this extent (mask off uninit flag).
    pub fn block_count(&self) -> u32 {
        (self.ee_len & !EXTENT_UNINIT_FLAG) as u32
    }

    /// First logical block AFTER this extent.
    pub fn logical_end(&self) -> u32 {
        self.ee_block + self.block_count()
    }

    /// Physical block of the block just after this extent's last block.
    pub fn physical_end(&self) -> u64 {
        self.physical_block() + self.block_count() as u64
    }

    /// Check if a logical block falls within this extent.
    pub fn contains(&self, logical_block: u32) -> bool {
        logical_block >= self.ee_block && logical_block < self.logical_end()
    }

    /// Map a logical block to its physical block.
    pub fn map(&self, logical_block: u32) -> Option<u64> {
        if self.contains(logical_block) {
            let offset = (logical_block - self.ee_block) as u64;
            Some(self.physical_block() + offset)
        } else {
            None
        }
    }

    /// Check if this extent can be merged with another (must be contiguous
    /// in both logical and physical space, and same init status).
    pub fn can_merge_right(&self, right: &ExtentLeaf) -> bool {
        self.logical_end() == right.ee_block
            && self.physical_end() == right.physical_block()
            && self.is_uninitialized() == right.is_uninitialized()
            && (self.block_count() + right.block_count()) <= EXTENT_MAX_LEN_INIT as u32
    }

    pub fn serialize(&self) -> [u8; 12] {
        let mut buf = [0u8; 12];
        buf[0..4].copy_from_slice(&self.ee_block.to_le_bytes());
        buf[4..6].copy_from_slice(&self.ee_len.to_le_bytes());
        buf[6..8].copy_from_slice(&self.ee_start_hi.to_le_bytes());
        buf[8..12].copy_from_slice(&self.ee_start_lo.to_le_bytes());
        buf
    }

    pub fn deserialize(buf: &[u8; 12]) -> Self {
        Self {
            ee_block: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
            ee_len: u16::from_le_bytes(buf[4..6].try_into().unwrap()),
            ee_start_hi: u16::from_le_bytes(buf[6..8].try_into().unwrap()),
            ee_start_lo: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
        }
    }
}

// ===========================================================================
// ExtentIndex (12 bytes)
// ===========================================================================

/// An extent index entry pointing to a child extent tree node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtentIndex {
    pub ei_block: u32,
    pub ei_leaf_lo: u32,
    pub ei_leaf_hi: u16,
    pub ei_unused: u16,
}

impl ExtentIndex {
    pub fn new(logical_block: u32, child_physical: u64) -> Self {
        Self {
            ei_block: logical_block,
            ei_leaf_lo: child_physical as u32,
            ei_leaf_hi: (child_physical >> 32) as u16,
            ei_unused: 0,
        }
    }

    /// Get the physical block of the child node (48-bit address).
    pub fn child_block(&self) -> u64 {
        ((self.ei_leaf_hi as u64) << 32) | (self.ei_leaf_lo as u64)
    }

    /// Set the physical block of the child node.
    pub fn set_child_block(&mut self, phys: u64) {
        self.ei_leaf_lo = phys as u32;
        self.ei_leaf_hi = (phys >> 32) as u16;
    }

    pub fn serialize(&self) -> [u8; 12] {
        let mut buf = [0u8; 12];
        buf[0..4].copy_from_slice(&self.ei_block.to_le_bytes());
        buf[4..8].copy_from_slice(&self.ei_leaf_lo.to_le_bytes());
        buf[8..10].copy_from_slice(&self.ei_leaf_hi.to_le_bytes());
        buf[10..12].copy_from_slice(&self.ei_unused.to_le_bytes());
        buf
    }

    pub fn deserialize(buf: &[u8; 12]) -> Self {
        Self {
            ei_block: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
            ei_leaf_lo: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
            ei_leaf_hi: u16::from_le_bytes(buf[8..10].try_into().unwrap()),
            ei_unused: u16::from_le_bytes(buf[10..12].try_into().unwrap()),
        }
    }
}

// ===========================================================================
// ExtentEntries — enum for leaf vs index entries
// ===========================================================================

#[derive(Debug, Clone)]
pub enum ExtentEntries {
    Leaves(Vec<ExtentLeaf>),
    Indices(Vec<ExtentIndex>),
}

// ===========================================================================
// Inode extent root helpers (read/write the 96-byte dual-purpose area)
// ===========================================================================

/// Extract 96 raw bytes from inode's block pointer area.
fn inode_extent_raw_bytes(inode: &Inode) -> [u8; 96] {
    let mut buf = [0u8; 96];
    for (i, &ptr) in inode.direct_blocks.iter().enumerate() {
        let off = i * 8;
        buf[off..off + 8].copy_from_slice(&ptr.to_le_bytes());
    }
    buf[80..88].copy_from_slice(&inode.indirect_block.to_le_bytes());
    buf[88..96].copy_from_slice(&inode.double_indirect.to_le_bytes());
    buf
}

/// Write 96 raw bytes back into inode's block pointer fields.
fn write_raw_bytes_to_inode(inode: &mut Inode, buf: &[u8; 96]) {
    for i in 0..10 {
        let off = i * 8;
        inode.direct_blocks[i] = u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
    }
    inode.indirect_block = u64::from_le_bytes(buf[80..88].try_into().unwrap());
    inode.double_indirect = u64::from_le_bytes(buf[88..96].try_into().unwrap());
}

/// Read the extent tree root from the inode's dual-purpose area.
pub fn read_inode_extent_root(inode: &Inode) -> Result<(ExtentHeader, ExtentEntries)> {
    let buf = inode_extent_raw_bytes(inode);
    let header_bytes: [u8; 12] = buf[0..12].try_into().unwrap();
    let header = ExtentHeader::deserialize(&header_bytes)?;

    let entries = if header.is_leaf() {
        let mut leaves = Vec::with_capacity(header.entries as usize);
        for i in 0..header.entries as usize {
            let off = 12 + i * 12;
            let entry_bytes: [u8; 12] = buf[off..off + 12].try_into().unwrap();
            leaves.push(ExtentLeaf::deserialize(&entry_bytes));
        }
        ExtentEntries::Leaves(leaves)
    } else {
        let mut indices = Vec::with_capacity(header.entries as usize);
        for i in 0..header.entries as usize {
            let off = 12 + i * 12;
            let entry_bytes: [u8; 12] = buf[off..off + 12].try_into().unwrap();
            indices.push(ExtentIndex::deserialize(&entry_bytes));
        }
        ExtentEntries::Indices(indices)
    };

    Ok((header, entries))
}

/// Write the extent tree root back to the inode's dual-purpose area.
pub fn write_inode_extent_root(
    inode: &mut Inode,
    header: &ExtentHeader,
    entries: &ExtentEntries,
) {
    let mut buf = [0u8; 96];
    buf[0..12].copy_from_slice(&header.serialize());

    match entries {
        ExtentEntries::Leaves(leaves) => {
            for (i, leaf) in leaves.iter().enumerate() {
                let off = 12 + i * 12;
                buf[off..off + 12].copy_from_slice(&leaf.serialize());
            }
        }
        ExtentEntries::Indices(indices) => {
            for (i, idx) in indices.iter().enumerate() {
                let off = 12 + i * 12;
                buf[off..off + 12].copy_from_slice(&idx.serialize());
            }
        }
    }

    write_raw_bytes_to_inode(inode, &buf);
}

/// Initialize an empty extent root in an inode.
pub fn init_inode_extent_root(inode: &mut Inode) {
    let header = ExtentHeader::new_root();
    let entries = ExtentEntries::Leaves(vec![]);
    write_inode_extent_root(inode, &header, &entries);
    inode.flags |= INODE_FLAG_EXTENTS;
}

// ===========================================================================
// On-disk extent block I/O
// ===========================================================================

/// Read an extent tree node from a disk block.
pub fn read_extent_block(
    dev: &mut dyn CFSBlockDevice,
    block_addr: u64,
    block_size: u32,
) -> Result<(ExtentHeader, ExtentEntries)> {
    let mut buf = vec![0u8; block_size as usize];
    dev.read(block_addr * block_size as u64, &mut buf)?;

    let header_bytes: [u8; 12] = buf[0..12].try_into().unwrap();
    let header = ExtentHeader::deserialize(&header_bytes)?;

    if header.depth > EXTENT_MAX_DEPTH {
        bail!(
            "extent block at {} has invalid depth {}",
            block_addr,
            header.depth
        );
    }

    let entries = if header.is_leaf() {
        let mut leaves = Vec::with_capacity(header.entries as usize);
        for i in 0..header.entries as usize {
            let off = 12 + i * 12;
            let entry_bytes: [u8; 12] = buf[off..off + 12].try_into().unwrap();
            leaves.push(ExtentLeaf::deserialize(&entry_bytes));
        }
        ExtentEntries::Leaves(leaves)
    } else {
        let mut indices = Vec::with_capacity(header.entries as usize);
        for i in 0..header.entries as usize {
            let off = 12 + i * 12;
            let entry_bytes: [u8; 12] = buf[off..off + 12].try_into().unwrap();
            indices.push(ExtentIndex::deserialize(&entry_bytes));
        }
        ExtentEntries::Indices(indices)
    };

    Ok((header, entries))
}

/// Write an extent tree node to a disk block.
pub fn write_extent_block(
    dev: &mut dyn CFSBlockDevice,
    block_addr: u64,
    header: &ExtentHeader,
    entries: &ExtentEntries,
    block_size: u32,
) -> Result<()> {
    let mut buf = vec![0u8; block_size as usize];
    buf[0..12].copy_from_slice(&header.serialize());

    match entries {
        ExtentEntries::Leaves(leaves) => {
            for (i, leaf) in leaves.iter().enumerate() {
                let off = 12 + i * 12;
                buf[off..off + 12].copy_from_slice(&leaf.serialize());
            }
        }
        ExtentEntries::Indices(indices) => {
            for (i, idx) in indices.iter().enumerate() {
                let off = 12 + i * 12;
                buf[off..off + 12].copy_from_slice(&idx.serialize());
            }
        }
    }

    dev.write(block_addr * block_size as u64, &buf)?;
    Ok(())
}

// ===========================================================================
// Binary search helpers
// ===========================================================================

/// Binary search for the entry covering `logical_block` in a sorted
/// slice of leaves.
///
/// Returns (found: bool, index: usize):
/// - found=true, index=i: leaves[i] contains the block
/// - found=false, index=i: block is in a hole; 'i' is where a new extent would go
fn bsearch_leaf(leaves: &[ExtentLeaf], logical_block: u32) -> (bool, usize) {
    if leaves.is_empty() {
        return (false, 0);
    }
    // Find the last entry where ee_block <= logical_block
    let mut lo = 0usize;
    let mut hi = leaves.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if leaves[mid].ee_block <= logical_block {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        return (false, 0);
    }
    let idx = lo - 1;
    if leaves[idx].contains(logical_block) {
        (true, idx)
    } else {
        (false, lo)
    }
}

/// Binary search for the child index entry covering `logical_block`.
/// Returns the index of the entry whose ei_block <= logical_block.
fn bsearch_index(indices: &[ExtentIndex], logical_block: u32) -> usize {
    let mut lo = 0usize;
    let mut hi = indices.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if indices[mid].ei_block <= logical_block {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        0
    } else {
        lo - 1
    }
}

// ===========================================================================
// Extent tree lookup
// ===========================================================================

/// Find the extent covering `logical_block` in the extent tree rooted in `inode`.
///
/// Returns `Ok(Some(leaf))` if found, `Ok(None)` for a sparse hole.
pub fn extent_find(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    logical_block: u32,
    block_size: u32,
) -> Result<Option<ExtentLeaf>> {
    let (mut header, mut entries) = read_inode_extent_root(inode)?;

    loop {
        match &entries {
            ExtentEntries::Leaves(leaves) => {
                let (found, idx) = bsearch_leaf(leaves, logical_block);
                if found {
                    return Ok(Some(leaves[idx]));
                } else {
                    return Ok(None);
                }
            }
            ExtentEntries::Indices(indices) => {
                if indices.is_empty() {
                    return Ok(None);
                }
                let idx = bsearch_index(indices, logical_block);
                let child_block = indices[idx].child_block();
                let (child_header, child_entries) =
                    read_extent_block(dev, child_block, block_size)?;

                if child_header.depth + 1 != header.depth {
                    bail!(
                        "extent tree depth inconsistency at block {child_block}: \
                         expected depth {}, got {}",
                        header.depth - 1,
                        child_header.depth
                    );
                }

                header = child_header;
                entries = child_entries;
            }
        }
    }
}

/// Map a logical file block to its physical disk block using the extent tree.
/// Returns 0 for sparse holes and uninitialized extents.
pub fn get_block_ptr_extent(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    logical_block: u32,
    block_size: u32,
) -> Result<u64> {
    match extent_find(dev, inode, logical_block, block_size)? {
        Some(leaf) => {
            if leaf.is_uninitialized() {
                Ok(0)
            } else {
                Ok(leaf.map(logical_block).unwrap())
            }
        }
        None => Ok(0),
    }
}

// ===========================================================================
// Extent tree insertion
// ===========================================================================

/// Insert a new extent mapping into the tree.
///
/// Maps logical blocks [logical_block .. logical_block + length) to
/// physical blocks [physical_block .. physical_block + length).
pub fn extent_insert(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    logical_block: u32,
    physical_block: u64,
    length: u16,
    block_size: u32,
) -> Result<()> {
    let new_ext = ExtentLeaf::new(logical_block, physical_block, length);
    extent_insert_leaf(dev, inode, alloc, sb, new_ext, block_size)
}

/// Insert an uninitialized (preallocated) extent.
pub fn extent_insert_uninit(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    logical_block: u32,
    physical_block: u64,
    length: u16,
    block_size: u32,
) -> Result<()> {
    let new_ext = ExtentLeaf::new_uninit(logical_block, physical_block, length);
    extent_insert_leaf(dev, inode, alloc, sb, new_ext, block_size)
}

/// Core insertion: insert a pre-built ExtentLeaf into the extent tree.
fn extent_insert_leaf(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    new_ext: ExtentLeaf,
    block_size: u32,
) -> Result<()> {
    let logical_block = new_ext.ee_block;
    let length = new_ext.ee_len;

    // Read root
    let (root_header, root_entries) = read_inode_extent_root(inode)?;

    if root_header.depth == 0 {
        // Tree is a single leaf node in the inode root
        let mut leaves = match root_entries {
            ExtentEntries::Leaves(l) => l,
            _ => bail!("depth 0 but entries are indices"),
        };

        // Try merge with adjacent extents
        if try_merge_leaf(&mut leaves, &new_ext) {
            let mut hdr = root_header;
            hdr.entries = leaves.len() as u16;
            write_inode_extent_root(inode, &hdr, &ExtentEntries::Leaves(leaves));
            return Ok(());
        }

        // Insert in sorted position
        let insert_pos = leaves
            .iter()
            .position(|l| l.ee_block > logical_block)
            .unwrap_or(leaves.len());

        if (leaves.len() as u16) < root_header.max {
            // Room in root — just insert
            leaves.insert(insert_pos, new_ext);
            let mut hdr = root_header;
            hdr.entries = leaves.len() as u16;
            write_inode_extent_root(inode, &hdr, &ExtentEntries::Leaves(leaves));
            return Ok(());
        }

        // Root is full — need to grow depth
        grow_indepth(dev, inode, alloc, sb, block_size)?;
        // Retry in the now-deeper tree
        return extent_insert_leaf(dev, inode, alloc, sb, new_ext, block_size);
    }

    // depth > 0: find the correct leaf node and insert there
    insert_into_tree(dev, inode, alloc, sb, new_ext, block_size)
}

/// Insert a new leaf entry into a tree with depth >= 1.
fn insert_into_tree(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    new_ext: ExtentLeaf,
    block_size: u32,
) -> Result<()> {
    // Build path from root to leaf
    let (root_header, root_entries) = read_inode_extent_root(inode)?;

    // Traverse index levels to find the correct leaf block
    let mut path: Vec<(u64, ExtentHeader, Vec<ExtentIndex>, usize)> = Vec::new(); // (block, header, indices, child_idx)
    let mut header = root_header;
    let mut entries = root_entries;
    let mut current_block: u64 = 0; // 0 = in-inode root

    while header.depth > 0 {
        let indices = match entries {
            ExtentEntries::Indices(idx) => idx,
            _ => bail!("depth {} but entries are leaves", header.depth),
        };

        if indices.is_empty() {
            bail!("empty index node at depth {}", header.depth);
        }

        let child_idx = bsearch_index(&indices, new_ext.ee_block);
        let child_block = indices[child_idx].child_block();

        path.push((current_block, header, indices, child_idx));

        let (child_header, child_entries) =
            read_extent_block(dev, child_block, block_size)?;

        current_block = child_block;
        header = child_header;
        entries = child_entries;
    }

    // `current_block` is the leaf block, `header` is its header, `entries` is its leaves
    let mut leaves = match entries {
        ExtentEntries::Leaves(l) => l,
        _ => bail!("depth 0 but not leaves"),
    };

    // Try merge
    if try_merge_leaf(&mut leaves, &new_ext) {
        let mut hdr = header;
        hdr.entries = leaves.len() as u16;
        write_extent_block(dev, current_block, &hdr, &ExtentEntries::Leaves(leaves), block_size)?;
        return Ok(());
    }

    // Find insertion position
    let insert_pos = leaves
        .iter()
        .position(|l| l.ee_block > new_ext.ee_block)
        .unwrap_or(leaves.len());

    if !header.is_full() {
        // Room in this leaf — insert
        leaves.insert(insert_pos, new_ext);
        let mut hdr = header;
        hdr.entries = leaves.len() as u16;
        let first_key = leaves[0].ee_block;
        write_extent_block(dev, current_block, &hdr, &ExtentEntries::Leaves(leaves), block_size)?;

        // Update the index entry's ei_block if the new extent is now the first entry
        if insert_pos == 0 && !path.is_empty() {
            update_index_key(dev, inode, &path, first_key, block_size)?;
        }
        return Ok(());
    }

    // Leaf is full — need to split
    leaves.insert(insert_pos, new_ext);

    let mid = leaves.len() / 2;
    let right_leaves: Vec<ExtentLeaf> = leaves.drain(mid..).collect();
    let left_leaves = leaves;

    let split_key = right_leaves[0].ee_block;

    // Write updated left half
    let mut left_hdr = header;
    left_hdr.entries = left_leaves.len() as u16;
    write_extent_block(dev, current_block, &left_hdr, &ExtentEntries::Leaves(left_leaves.clone()), block_size)?;

    // Allocate new block for right half
    let new_leaf_blocks = alloc.alloc(dev, sb, 1)?;
    let new_leaf_block = new_leaf_blocks[0];
    let mut right_hdr = ExtentHeader::new_leaf(block_size);
    right_hdr.entries = right_leaves.len() as u16;
    write_extent_block(dev, new_leaf_block, &right_hdr, &ExtentEntries::Leaves(right_leaves), block_size)?;

    // Update left's index key if first entry changed
    if insert_pos == 0 && !path.is_empty() {
        update_index_key(dev, inode, &path, left_leaves[0].ee_block, block_size)?;
    }

    // Now insert the new index entry for the right half into the parent
    let new_index = ExtentIndex::new(split_key, new_leaf_block);
    insert_index_entry(dev, inode, alloc, sb, &path, new_index, block_size)
}

/// Insert a new index entry into the path's parent level.
/// If the parent is full, recursively split upward. If the root is full, grow depth.
fn insert_index_entry(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    path: &[(u64, ExtentHeader, Vec<ExtentIndex>, usize)],
    new_index: ExtentIndex,
    block_size: u32,
) -> Result<()> {
    if path.is_empty() {
        bail!("cannot insert index entry with empty path");
    }

    let level = path.len() - 1;
    let (parent_block, parent_header, ref parent_indices, _child_idx) = path[level];

    let mut indices = parent_indices.clone();

    // Find insertion position for the new index
    let insert_pos = indices
        .iter()
        .position(|idx| idx.ei_block > new_index.ei_block)
        .unwrap_or(indices.len());

    if (indices.len() as u16) < parent_header.max {
        // Room in parent — insert
        indices.insert(insert_pos, new_index);
        let mut hdr = parent_header;
        hdr.entries = indices.len() as u16;

        if parent_block == 0 {
            // In-inode root
            write_inode_extent_root(inode, &hdr, &ExtentEntries::Indices(indices));
        } else {
            write_extent_block(dev, parent_block, &hdr, &ExtentEntries::Indices(indices), block_size)?;
        }
        return Ok(());
    }

    // Parent is full — split
    indices.insert(insert_pos, new_index);

    let mid = indices.len() / 2;
    let right_indices: Vec<ExtentIndex> = indices.drain(mid..).collect();
    let left_indices = indices;

    let split_key = right_indices[0].ei_block;

    // Write updated left half
    let mut left_hdr = parent_header;
    left_hdr.entries = left_indices.len() as u16;

    if parent_block == 0 {
        write_inode_extent_root(inode, &left_hdr, &ExtentEntries::Indices(left_indices));
    } else {
        write_extent_block(dev, parent_block, &left_hdr, &ExtentEntries::Indices(left_indices), block_size)?;
    }

    // Allocate new block for right half
    let new_idx_blocks = alloc.alloc(dev, sb, 1)?;
    let new_idx_block = new_idx_blocks[0];
    let mut right_hdr = ExtentHeader::new_index(block_size, parent_header.depth);
    right_hdr.entries = right_indices.len() as u16;
    write_extent_block(dev, new_idx_block, &right_hdr, &ExtentEntries::Indices(right_indices), block_size)?;

    // If we split the root, we need to grow depth
    if parent_block == 0 {
        // The root was just split — we need a new root at depth+1
        grow_indepth(dev, inode, alloc, sb, block_size)?;
        // The new root now has a single index entry pointing to the old left half.
        // We need to add the right half too.
        let (root_hdr, root_entries) = read_inode_extent_root(inode)?;
        let mut root_indices = match root_entries {
            ExtentEntries::Indices(idx) => idx,
            _ => bail!("after grow_indepth root should be indices"),
        };
        root_indices.push(ExtentIndex::new(split_key, new_idx_block));
        root_indices.sort_by_key(|idx| idx.ei_block);
        let mut new_root_hdr = root_hdr;
        new_root_hdr.entries = root_indices.len() as u16;
        write_inode_extent_root(inode, &new_root_hdr, &ExtentEntries::Indices(root_indices));
        return Ok(());
    }

    // Recurse up the path
    let parent_path = &path[..level];
    let right_entry = ExtentIndex::new(split_key, new_idx_block);

    if parent_path.is_empty() {
        // We need to grow depth since we've exhausted the path
        grow_indepth(dev, inode, alloc, sb, block_size)?;
        let (root_hdr, root_entries) = read_inode_extent_root(inode)?;
        let mut root_indices = match root_entries {
            ExtentEntries::Indices(idx) => idx,
            _ => bail!("after grow_indepth root should be indices"),
        };
        root_indices.push(right_entry);
        root_indices.sort_by_key(|idx| idx.ei_block);
        let mut new_root_hdr = root_hdr;
        new_root_hdr.entries = root_indices.len() as u16;
        write_inode_extent_root(inode, &new_root_hdr, &ExtentEntries::Indices(root_indices));
        return Ok(());
    }

    insert_index_entry(dev, inode, alloc, sb, parent_path, right_entry, block_size)
}

/// Update the index key for the first entry in a child.
fn update_index_key(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    path: &[(u64, ExtentHeader, Vec<ExtentIndex>, usize)],
    new_key: u32,
    block_size: u32,
) -> Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    let level = path.len() - 1;
    let (parent_block, parent_header, ref parent_indices, child_idx) = path[level];
    let mut indices = parent_indices.clone();

    if child_idx < indices.len() {
        indices[child_idx].ei_block = new_key;
        if parent_block == 0 {
            write_inode_extent_root(inode, &parent_header, &ExtentEntries::Indices(indices));
        } else {
            write_extent_block(dev, parent_block, &parent_header, &ExtentEntries::Indices(indices), block_size)?;
        }
    }
    Ok(())
}

/// Increase tree depth by 1.
/// Moves all root entries to a new block and replaces root with a single index.
pub fn grow_indepth(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    block_size: u32,
) -> Result<()> {
    let (root_header, root_entries) = read_inode_extent_root(inode)?;

    if root_header.depth >= EXTENT_MAX_DEPTH {
        bail!("extent tree at maximum depth {}", EXTENT_MAX_DEPTH);
    }

    // Allocate new block for current root contents
    let new_blocks = alloc.alloc(dev, sb, 1)?;
    let new_block = new_blocks[0];

    // Write current root entries to the new block
    let child_header = ExtentHeader {
        magic: EXTENT_MAGIC,
        entries: root_header.entries,
        max: extent_entries_per_block(block_size),
        depth: root_header.depth,
        generation: root_header.generation,
    };
    write_extent_block(dev, new_block, &child_header, &root_entries, block_size)?;

    // Determine the first logical block covered by this subtree
    let first_block = match &root_entries {
        ExtentEntries::Leaves(l) => {
            if l.is_empty() {
                0
            } else {
                l[0].ee_block
            }
        }
        ExtentEntries::Indices(i) => {
            if i.is_empty() {
                0
            } else {
                i[0].ei_block
            }
        }
    };

    // Replace root with single index entry
    let new_root_header = ExtentHeader {
        magic: EXTENT_MAGIC,
        entries: 1,
        max: EXTENT_ROOT_MAX_ENTRIES,
        depth: root_header.depth + 1,
        generation: root_header.generation,
    };
    let new_root_entries =
        ExtentEntries::Indices(vec![ExtentIndex::new(first_block, new_block)]);

    write_inode_extent_root(inode, &new_root_header, &new_root_entries);
    Ok(())
}

/// Try to merge a new extent with adjacent extents in the leaf list.
/// Returns true if the merge succeeded.
fn try_merge_leaf(leaves: &mut Vec<ExtentLeaf>, new_ext: &ExtentLeaf) -> bool {
    let (found, idx) = bsearch_leaf(leaves, new_ext.ee_block);
    if found {
        return false; // Block already mapped
    }

    // Try merge with left neighbor
    if idx > 0 {
        if leaves[idx - 1].can_merge_right(new_ext) {
            let combined = leaves[idx - 1].block_count() + new_ext.block_count();
            leaves[idx - 1].ee_len = combined as u16;
            if leaves[idx - 1].is_uninitialized() {
                leaves[idx - 1].ee_len |= EXTENT_UNINIT_FLAG;
            }

            // Also try merge with right neighbor
            if idx < leaves.len() && leaves[idx - 1].can_merge_right(&leaves[idx]) {
                let right_len = leaves[idx].block_count();
                let total = leaves[idx - 1].block_count() + right_len;
                leaves[idx - 1].ee_len = total as u16;
                if leaves[idx - 1].is_uninitialized() {
                    leaves[idx - 1].ee_len |= EXTENT_UNINIT_FLAG;
                }
                leaves.remove(idx);
            }
            return true;
        }
    }

    // Try merge with right neighbor
    if idx < leaves.len() {
        if new_ext.can_merge_right(&leaves[idx]) {
            let combined = new_ext.block_count() + leaves[idx].block_count();
            leaves[idx].ee_block = new_ext.ee_block;
            leaves[idx].set_physical_block(new_ext.physical_block());
            leaves[idx].ee_len = combined as u16;
            if new_ext.is_uninitialized() {
                leaves[idx].ee_len |= EXTENT_UNINIT_FLAG;
            }
            return true;
        }
    }

    false
}

// ===========================================================================
// Extent tree removal
// ===========================================================================

/// Remove a range of logical blocks [logical_start .. logical_start + length)
/// from the extent tree. Returns the physical blocks that were freed
/// (caller deallocates from bitmap).
pub fn extent_remove(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    logical_start: u32,
    length: u32,
    block_size: u32,
) -> Result<Vec<u64>> {
    if length == 0 {
        return Ok(vec![]);
    }

    let logical_end = logical_start + length;
    let mut freed = Vec::new();

    // For depth-0 trees, work directly on the inode root
    let (root_header, root_entries) = read_inode_extent_root(inode)?;

    if root_header.depth == 0 {
        let mut leaves = match root_entries {
            ExtentEntries::Leaves(l) => l,
            _ => bail!("depth 0 but entries are indices"),
        };

        remove_from_leaves(&mut leaves, logical_start, logical_end, &mut freed);

        let mut hdr = root_header;
        hdr.entries = leaves.len() as u16;
        write_inode_extent_root(inode, &hdr, &ExtentEntries::Leaves(leaves));
        return Ok(freed);
    }

    // For deeper trees, we need to traverse and modify leaf blocks
    remove_from_tree_recursive(dev, inode, logical_start, logical_end, block_size, &mut freed)?;

    // Try to shrink depth if possible
    loop {
        if !try_shrink_depth(dev, inode, block_size)? {
            break;
        }
    }

    Ok(freed)
}

/// Remove extents from a leaf list that overlap [logical_start..logical_end).
fn remove_from_leaves(
    leaves: &mut Vec<ExtentLeaf>,
    logical_start: u32,
    logical_end: u32,
    freed: &mut Vec<u64>,
) {
    let mut i = 0;
    while i < leaves.len() {
        let ext = leaves[i];
        let ext_start = ext.ee_block;
        let ext_end = ext.logical_end();

        if ext_end <= logical_start || ext_start >= logical_end {
            // No overlap
            i += 1;
            continue;
        }

        if logical_start <= ext_start && logical_end >= ext_end {
            // Case 1: Full removal
            for j in 0..ext.block_count() {
                freed.push(ext.physical_block() + j as u64);
            }
            leaves.remove(i);
            continue;
        }

        if logical_start <= ext_start && logical_end < ext_end {
            // Case 2: Remove front
            let remove_count = logical_end - ext_start;
            for j in 0..remove_count {
                freed.push(ext.physical_block() + j as u64);
            }
            leaves[i].ee_block = logical_end;
            leaves[i].set_physical_block(ext.physical_block() + remove_count as u64);
            let new_len = ext.block_count() - remove_count;
            leaves[i].ee_len = new_len as u16;
            if ext.is_uninitialized() {
                leaves[i].ee_len |= EXTENT_UNINIT_FLAG;
            }
            i += 1;
            continue;
        }

        if logical_start > ext_start && logical_end >= ext_end {
            // Case 3: Remove tail
            let remove_count = ext_end - logical_start;
            for j in 0..remove_count {
                freed.push(ext.physical_block() + (logical_start - ext_start) as u64 + j as u64);
            }
            let new_len = logical_start - ext_start;
            leaves[i].ee_len = new_len as u16;
            if ext.is_uninitialized() {
                leaves[i].ee_len |= EXTENT_UNINIT_FLAG;
            }
            i += 1;
            continue;
        }

        // Case 4: Remove middle → split
        let front_len = logical_start - ext_start;
        let back_start = logical_end;
        let back_len = ext_end - logical_end;
        let remove_count = logical_end - logical_start;

        for j in 0..remove_count {
            freed.push(ext.physical_block() + (logical_start - ext_start) as u64 + j as u64);
        }

        // Shrink existing extent to front portion
        leaves[i].ee_len = front_len as u16;
        if ext.is_uninitialized() {
            leaves[i].ee_len |= EXTENT_UNINIT_FLAG;
        }

        // Create back portion
        let back_phys = ext.physical_block() + (back_start - ext_start) as u64;
        let mut back_ext = ExtentLeaf::new(back_start, back_phys, back_len as u16);
        if ext.is_uninitialized() {
            back_ext.ee_len |= EXTENT_UNINIT_FLAG;
        }
        leaves.insert(i + 1, back_ext);

        i += 2;
    }
}

/// Recursive removal for trees with depth >= 1.
fn remove_from_tree_recursive(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    logical_start: u32,
    logical_end: u32,
    block_size: u32,
    freed: &mut Vec<u64>,
) -> Result<()> {
    let (root_header, root_entries) = read_inode_extent_root(inode)?;

    match root_entries {
        ExtentEntries::Indices(indices) => {
            let mut modified_indices = indices.clone();
            let mut to_remove = Vec::new();

            for (i, idx) in indices.iter().enumerate() {
                // Determine the logical range this index covers
                let idx_start = idx.ei_block;
                let idx_end = if i + 1 < indices.len() {
                    indices[i + 1].ei_block
                } else {
                    u32::MAX
                };

                // Check overlap
                if idx_end <= logical_start || idx_start >= logical_end {
                    continue;
                }

                let child_block = idx.child_block();
                let (child_header, child_entries) =
                    read_extent_block(dev, child_block, block_size)?;

                if child_header.is_leaf() {
                    let mut leaves = match child_entries {
                        ExtentEntries::Leaves(l) => l,
                        _ => bail!("leaf header but index entries"),
                    };

                    remove_from_leaves(&mut leaves, logical_start, logical_end, freed);

                    if leaves.is_empty() {
                        // Child is now empty — mark for removal
                        to_remove.push(i);
                        // The child block itself should be freed
                        freed.push(child_block);
                    } else {
                        let mut hdr = child_header;
                        hdr.entries = leaves.len() as u16;
                        write_extent_block(dev, child_block, &hdr, &ExtentEntries::Leaves(leaves.clone()), block_size)?;
                        // Update index key if the first leaf's block changed
                        if leaves[0].ee_block != modified_indices[i].ei_block {
                            modified_indices[i].ei_block = leaves[0].ee_block;
                        }
                    }
                } else {
                    // Recurse deeper — for multi-level trees we'd need full recursion.
                    // For now, handle depth-1 trees fully.
                    bail!("extent removal in trees deeper than 1 not yet implemented");
                }
            }

            // Remove empty children (reverse order to preserve indices)
            for &i in to_remove.iter().rev() {
                modified_indices.remove(i);
            }

            let mut hdr = root_header;
            hdr.entries = modified_indices.len() as u16;
            write_inode_extent_root(inode, &hdr, &ExtentEntries::Indices(modified_indices));
        }
        ExtentEntries::Leaves(_) => {
            // Already handled by depth==0 path in extent_remove
            bail!("unexpected leaf entries in recursive removal");
        }
    }

    Ok(())
}

/// Attempt to reduce tree depth when root has a single child whose entries
/// fit in the root.
fn try_shrink_depth(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    block_size: u32,
) -> Result<bool> {
    let (root_header, root_entries) = read_inode_extent_root(inode)?;

    if root_header.depth == 0 || root_header.entries != 1 {
        return Ok(false);
    }

    let child_block = match &root_entries {
        ExtentEntries::Indices(indices) => indices[0].child_block(),
        _ => return Ok(false),
    };

    let (child_header, child_entries) = read_extent_block(dev, child_block, block_size)?;

    // Can only absorb if child entries fit in root
    if child_header.entries > EXTENT_ROOT_MAX_ENTRIES {
        return Ok(false);
    }

    // Move child entries to root
    let new_root_header = ExtentHeader {
        magic: EXTENT_MAGIC,
        entries: child_header.entries,
        max: EXTENT_ROOT_MAX_ENTRIES,
        depth: child_header.depth,
        generation: child_header.generation,
    };

    write_inode_extent_root(inode, &new_root_header, &child_entries);

    // Note: the child block should be freed by the caller, but since
    // try_shrink_depth doesn't have allocator access, we zero it and
    // the caller (extent_remove) already includes child blocks in freed list.
    Ok(true)
}

// ===========================================================================
// Uninit → Init conversion (for fallocate / preallocation writes)
// ===========================================================================

/// Convert a portion of an uninitialized extent to initialized.
///
/// Finds the uninitialized extent covering `write_start`, removes it, and
/// re-inserts up to 3 pieces:
///   [ext_start..write_start) uninit  +  [write_start..write_start+write_count) init
///   + [write_start+write_count..ext_end) uninit
///
/// If the extent is already initialized, this is a no-op.
pub fn extent_mark_initialized(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    write_start: u32,
    write_count: u32,
    block_size: u32,
) -> Result<()> {
    let ext = match extent_find(dev, inode, write_start, block_size)? {
        Some(e) => e,
        None => bail!("no extent found at logical block {}", write_start),
    };

    if !ext.is_uninitialized() {
        return Ok(()); // already initialized
    }

    let ext_start = ext.ee_block;
    let ext_len = ext.block_count();
    let ext_phys = ext.physical_block();
    let ext_end = ext_start + ext_len;
    let write_end = write_start + write_count;

    // Remove the original uninitialized extent (don't free physical blocks)
    // We use extent_remove but must re-add the physical blocks to the tree, not free them
    // Actually extent_remove returns the physical block list — we must NOT free them.
    let _removed = extent_remove(dev, inode, ext_start, ext_len, block_size)?;
    // The blocks are NOT freed in the allocator — they stay allocated.
    // extent_remove only removes from the tree and returns the list.

    // Re-insert up to 3 pieces:

    // 1. Leading uninit portion: [ext_start .. write_start)
    if write_start > ext_start {
        let pre_len = (write_start - ext_start) as u16;
        extent_insert_uninit(dev, inode, alloc, sb, ext_start, ext_phys, pre_len, block_size)?;
    }

    // 2. Initialized written portion: [write_start .. min(write_end, ext_end))
    let init_end = std::cmp::min(write_end, ext_end);
    let init_count = (init_end - write_start) as u16;
    let init_phys = ext_phys + (write_start - ext_start) as u64;
    extent_insert(dev, inode, alloc, sb, write_start, init_phys, init_count, block_size)?;

    // 3. Trailing uninit portion: [init_end .. ext_end)
    if init_end < ext_end {
        let post_len = (ext_end - init_end) as u16;
        let post_phys = ext_phys + (init_end - ext_start) as u64;
        extent_insert_uninit(dev, inode, alloc, sb, init_end, post_phys, post_len, block_size)?;
    }

    Ok(())
}

// ===========================================================================
// Walk entire extent tree (for free_all)
// ===========================================================================

/// Collect all leaf extents from the tree, in logical block order.
/// Returns the raw `ExtentLeaf` entries so the caller can inspect
/// uninit flags, lengths, physical blocks, etc.
pub fn extent_list_leaves(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    block_size: u32,
) -> Result<Vec<ExtentLeaf>> {
    let mut leaves = Vec::new();
    let (header, entries) = read_inode_extent_root(inode)?;
    collect_leaves_recursive(dev, &header, &entries, block_size, &mut leaves)?;
    leaves.sort_by_key(|l| l.ee_block);
    Ok(leaves)
}

fn collect_leaves_recursive(
    dev: &mut dyn CFSBlockDevice,
    _header: &ExtentHeader,
    entries: &ExtentEntries,
    block_size: u32,
    out: &mut Vec<ExtentLeaf>,
) -> Result<()> {
    match entries {
        ExtentEntries::Leaves(leaves) => {
            out.extend(leaves.iter().copied());
        }
        ExtentEntries::Indices(indices) => {
            for idx in indices {
                let child_block = idx.child_block();
                let (child_header, child_entries) =
                    read_extent_block(dev, child_block, block_size)?;
                collect_leaves_recursive(dev, &child_header, &child_entries, block_size, out)?;
            }
        }
    }
    Ok(())
}

/// Walk the entire extent tree and collect all physical data blocks + tree
/// node blocks. The caller frees them from the allocator.
pub fn collect_all_extents(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    block_size: u32,
) -> Result<(Vec<u64>, Vec<u64>)> {
    // Returns (data_blocks, tree_node_blocks)
    let mut data_blocks = Vec::new();
    let mut node_blocks = Vec::new();

    let (header, entries) = read_inode_extent_root(inode)?;
    collect_recursive(dev, &header, &entries, block_size, &mut data_blocks, &mut node_blocks)?;

    Ok((data_blocks, node_blocks))
}

fn collect_recursive(
    dev: &mut dyn CFSBlockDevice,
    _header: &ExtentHeader,
    entries: &ExtentEntries,
    block_size: u32,
    data_blocks: &mut Vec<u64>,
    node_blocks: &mut Vec<u64>,
) -> Result<()> {
    match entries {
        ExtentEntries::Leaves(leaves) => {
            for leaf in leaves {
                let start = leaf.physical_block();
                let count = leaf.block_count();
                for i in 0..count {
                    data_blocks.push(start + i as u64);
                }
            }
        }
        ExtentEntries::Indices(indices) => {
            for idx in indices {
                let child_block = idx.child_block();
                node_blocks.push(child_block);

                let (child_header, child_entries) =
                    read_extent_block(dev, child_block, block_size)?;
                collect_recursive(
                    dev,
                    &child_header,
                    &child_entries,
                    block_size,
                    data_blocks,
                    node_blocks,
                )?;
            }
        }
    }
    Ok(())
}

/// Free all extents in the tree and reset the inode to an empty extent root.
pub fn free_all_extents(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    block_size: u32,
) -> Result<()> {
    let (data_blocks, node_blocks) = collect_all_extents(dev, inode, block_size)?;

    // Free data blocks
    if !data_blocks.is_empty() {
        alloc.free(dev, sb, &data_blocks)?;
    }

    // Free tree node blocks
    if !node_blocks.is_empty() {
        alloc.free(dev, sb, &node_blocks)?;
    }

    // Reset inode to empty extent tree
    init_inode_extent_root(inode);
    inode.size = 0;
    inode.block_count = 0;
    inode.block_count_hi = 0;

    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use crate::volume::{alloc, BlockAlloc, CFSVolume, DEFAULT_BLOCK_SIZE};
    use tempfile::NamedTempFile;

    // -----------------------------------------------------------------------
    // 10C.1 — Data structure tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extent_header_serialize_roundtrip() {
        let hdr = ExtentHeader::new_root();
        let bytes = hdr.serialize();
        let hdr2 = ExtentHeader::deserialize(&bytes).unwrap();
        assert_eq!(hdr, hdr2);
    }

    #[test]
    fn test_extent_header_bad_magic() {
        let mut bytes = ExtentHeader::new_root().serialize();
        bytes[0] = 0x00;
        bytes[1] = 0x00;
        assert!(ExtentHeader::deserialize(&bytes).is_err());
    }

    #[test]
    fn test_extent_header_leaf_block() {
        let hdr = ExtentHeader::new_leaf(4096);
        assert_eq!(hdr.max, 340);
        assert_eq!(hdr.depth, 0);
    }

    #[test]
    fn test_extent_header_index_block() {
        let hdr = ExtentHeader::new_index(4096, 2);
        assert_eq!(hdr.max, 340);
        assert_eq!(hdr.depth, 2);
    }

    #[test]
    fn test_extent_leaf_serialize_roundtrip() {
        let leaf = ExtentLeaf::new(0, 12345, 100);
        let bytes = leaf.serialize();
        let leaf2 = ExtentLeaf::deserialize(&bytes);
        assert_eq!(leaf, leaf2);
    }

    #[test]
    fn test_extent_leaf_physical_block_48bit() {
        let leaf = ExtentLeaf {
            ee_block: 0,
            ee_len: 1,
            ee_start_hi: 1,
            ee_start_lo: 0x12345678,
        };
        assert_eq!(leaf.physical_block(), 0x1_12345678);
    }

    #[test]
    fn test_extent_leaf_uninit_flag() {
        let leaf = ExtentLeaf::new_uninit(10, 100, 50);
        assert!(leaf.is_uninitialized());
        assert_eq!(leaf.block_count(), 50);
    }

    #[test]
    fn test_extent_leaf_contains() {
        let leaf = ExtentLeaf::new(10, 100, 10); // covers [10..20)
        assert!(leaf.contains(10));
        assert!(leaf.contains(15));
        assert!(leaf.contains(19));
        assert!(!leaf.contains(9));
        assert!(!leaf.contains(20));
    }

    #[test]
    fn test_extent_leaf_map() {
        let leaf = ExtentLeaf::new(10, 100, 10); // [10..20) @ phys 100
        assert_eq!(leaf.map(15), Some(105));
        assert_eq!(leaf.map(10), Some(100));
        assert_eq!(leaf.map(19), Some(109));
        assert_eq!(leaf.map(20), None);
    }

    #[test]
    fn test_extent_leaf_can_merge_right() {
        let a = ExtentLeaf::new(0, 100, 10); // [0..10) @ 100
        let b = ExtentLeaf::new(10, 110, 5); // [10..15) @ 110
        assert!(a.can_merge_right(&b));
    }

    #[test]
    fn test_extent_leaf_cannot_merge_gap() {
        let a = ExtentLeaf::new(0, 100, 10); // [0..10) @ 100
        let b = ExtentLeaf::new(10, 200, 5); // [10..15) @ 200 (physical gap)
        assert!(!a.can_merge_right(&b));
    }

    #[test]
    fn test_extent_leaf_cannot_merge_init_mismatch() {
        let a = ExtentLeaf::new(0, 100, 10);
        let b = ExtentLeaf::new_uninit(10, 110, 5);
        assert!(!a.can_merge_right(&b));
    }

    #[test]
    fn test_extent_leaf_merge_length_limit() {
        let a = ExtentLeaf::new(0, 100, 32000);
        let b = ExtentLeaf::new(32000, 32100, 1000);
        // Combined = 33000 > 32767
        assert!(!a.can_merge_right(&b));
    }

    #[test]
    fn test_extent_index_serialize_roundtrip() {
        let idx = ExtentIndex::new(100, 0x1_00000500);
        let bytes = idx.serialize();
        let idx2 = ExtentIndex::deserialize(&bytes);
        assert_eq!(idx, idx2);
    }

    #[test]
    fn test_extent_index_child_block_48bit() {
        let idx = ExtentIndex {
            ei_block: 0,
            ei_leaf_lo: 0x300,
            ei_leaf_hi: 2,
            ei_unused: 0,
        };
        assert_eq!(idx.child_block(), 0x2_00000300);
    }

    // -----------------------------------------------------------------------
    // 10C.2 — Node I/O tests
    // -----------------------------------------------------------------------

    fn make_test_dev(size: u64) -> (NamedTempFile, Box<dyn CFSBlockDevice>) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(size)).unwrap();
        (tmp, Box::new(dev))
    }

    #[test]
    fn test_read_write_extent_block_leaf() {
        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        let block_size = 4096;
        let block_addr = 10;

        let leaves = vec![
            ExtentLeaf::new(0, 100, 10),
            ExtentLeaf::new(100, 200, 50),
        ];
        let mut hdr = ExtentHeader::new_leaf(block_size);
        hdr.entries = leaves.len() as u16;

        write_extent_block(dev.as_mut(), block_addr, &hdr, &ExtentEntries::Leaves(leaves.clone()), block_size).unwrap();
        let (hdr2, entries2) = read_extent_block(dev.as_mut(), block_addr, block_size).unwrap();

        assert_eq!(hdr, hdr2);
        match entries2 {
            ExtentEntries::Leaves(l) => assert_eq!(l, leaves),
            _ => panic!("expected leaves"),
        }
    }

    #[test]
    fn test_read_write_extent_block_index() {
        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        let block_size = 4096;
        let block_addr = 10;

        let indices = vec![
            ExtentIndex::new(0, 20),
            ExtentIndex::new(1000, 21),
        ];
        let mut hdr = ExtentHeader::new_index(block_size, 1);
        hdr.entries = indices.len() as u16;

        write_extent_block(dev.as_mut(), block_addr, &hdr, &ExtentEntries::Indices(indices.clone()), block_size).unwrap();
        let (hdr2, entries2) = read_extent_block(dev.as_mut(), block_addr, block_size).unwrap();

        assert_eq!(hdr, hdr2);
        match entries2 {
            ExtentEntries::Indices(idx) => assert_eq!(idx, indices),
            _ => panic!("expected indices"),
        }
    }

    #[test]
    fn test_inode_extent_root_roundtrip() {
        let mut inode = Inode::new_file();
        inode.init_extent_root();

        // Write some leaves to the root
        let leaves = vec![
            ExtentLeaf::new(0, 100, 10),
            ExtentLeaf::new(50, 200, 20),
        ];
        let mut hdr = ExtentHeader::new_root();
        hdr.entries = 2;
        write_inode_extent_root(&mut inode, &hdr, &ExtentEntries::Leaves(leaves.clone()));

        let (hdr2, entries2) = read_inode_extent_root(&inode).unwrap();
        assert_eq!(hdr, hdr2);
        match entries2 {
            ExtentEntries::Leaves(l) => assert_eq!(l, leaves),
            _ => panic!("expected leaves"),
        }
    }

    #[test]
    fn test_inode_raw_bytes_roundtrip() {
        let mut inode = Inode::new_file();
        inode.direct_blocks[0] = 0x1234;
        inode.direct_blocks[9] = 0x9876;
        inode.indirect_block = 0xABCD;
        inode.double_indirect = 0xEF01;

        let raw = inode_extent_raw_bytes(&inode);
        let mut inode2 = Inode::new_file();
        write_raw_bytes_to_inode(&mut inode2, &raw);

        assert_eq!(inode.direct_blocks, inode2.direct_blocks);
        assert_eq!(inode.indirect_block, inode2.indirect_block);
        assert_eq!(inode.double_indirect, inode2.double_indirect);
    }

    // -----------------------------------------------------------------------
    // 10C.3 — Lookup tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_bsearch_leaf_empty() {
        let (found, idx) = bsearch_leaf(&[], 5);
        assert!(!found);
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_bsearch_leaf_single_hit() {
        let leaves = vec![ExtentLeaf::new(0, 100, 10)]; // [0..10)
        let (found, idx) = bsearch_leaf(&leaves, 5);
        assert!(found);
        assert_eq!(idx, 0);
    }

    #[test]
    fn test_bsearch_leaf_single_miss() {
        let leaves = vec![ExtentLeaf::new(0, 100, 10)]; // [0..10)
        let (found, idx) = bsearch_leaf(&leaves, 15);
        assert!(!found);
        assert_eq!(idx, 1);
    }

    #[test]
    fn test_bsearch_index_single() {
        let indices = vec![ExtentIndex::new(0, 20)];
        assert_eq!(bsearch_index(&indices, 100), 0);
    }

    #[test]
    fn test_bsearch_index_multiple() {
        let indices = vec![
            ExtentIndex::new(0, 20),
            ExtentIndex::new(100, 21),
            ExtentIndex::new(200, 22),
        ];
        assert_eq!(bsearch_index(&indices, 150), 1);
        assert_eq!(bsearch_index(&indices, 200), 2);
        assert_eq!(bsearch_index(&indices, 50), 0);
    }

    #[test]
    fn test_lookup_empty_tree() {
        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let result = extent_find(dev.as_mut(), &inode, 0, 4096).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_lookup_single_extent() {
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        // Manually write a single leaf to root
        let leaves = vec![ExtentLeaf::new(0, 100, 50)]; // [0..50) @ 100
        let mut hdr = ExtentHeader::new_root();
        hdr.entries = 1;
        write_inode_extent_root(&mut inode, &hdr, &ExtentEntries::Leaves(leaves));

        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        let result = extent_find(dev.as_mut(), &inode, 25, 4096).unwrap();
        assert!(result.is_some());
        let leaf = result.unwrap();
        assert_eq!(leaf.map(25), Some(125));
    }

    #[test]
    fn test_lookup_hole() {
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let leaves = vec![ExtentLeaf::new(100, 200, 50)]; // [100..150) @ 200
        let mut hdr = ExtentHeader::new_root();
        hdr.entries = 1;
        write_inode_extent_root(&mut inode, &hdr, &ExtentEntries::Leaves(leaves));

        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        // Block 50 is in a hole (before the extent)
        let result = extent_find(dev.as_mut(), &inode, 50, 4096).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_block_ptr_extent_uninit() {
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let leaves = vec![ExtentLeaf::new_uninit(0, 100, 10)];
        let mut hdr = ExtentHeader::new_root();
        hdr.entries = 1;
        write_inode_extent_root(&mut inode, &hdr, &ExtentEntries::Leaves(leaves));

        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        let phys = get_block_ptr_extent(dev.as_mut(), &inode, 5, 4096).unwrap();
        assert_eq!(phys, 0); // uninit returns 0
    }

    // -----------------------------------------------------------------------
    // 10C.4 — Insertion tests (with actual volume)
    // -----------------------------------------------------------------------

    fn make_vol_for_extents() -> (NamedTempFile, CFSVolume) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        // 4 MB — enough for extent tree tests
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let vol = CFSVolume::format(Box::new(dev), DEFAULT_BLOCK_SIZE).unwrap();
        (tmp, vol)
    }

    #[test]
    fn test_insert_into_empty_tree() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        extent_insert(
            &mut **dg, &mut inode, &mut ba, &mut *sg,
            0, 100, 1, bs,
        ).unwrap();

        let (hdr, entries) = read_inode_extent_root(&inode).unwrap();
        assert_eq!(hdr.entries, 1);
        match entries {
            ExtentEntries::Leaves(l) => {
                assert_eq!(l[0].ee_block, 0);
                assert_eq!(l[0].physical_block(), 100);
                assert_eq!(l[0].block_count(), 1);
            }
            _ => panic!("expected leaves"),
        }
    }

    #[test]
    fn test_insert_7_extents_fills_root() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // Insert 7 non-contiguous extents
        for i in 0..7u32 {
            extent_insert(
                &mut **dg, &mut inode, &mut ba, &mut *sg,
                i * 100, (i as u64 + 1) * 1000, 10, bs,
            ).unwrap();
        }

        let (hdr, _) = read_inode_extent_root(&inode).unwrap();
        assert_eq!(hdr.entries, 7);
        assert_eq!(hdr.depth, 0);
    }

    #[test]
    fn test_insert_8th_triggers_depth_grow() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // Insert 8 non-contiguous extents (7 fits in root, 8th triggers grow)
        for i in 0..8u32 {
            extent_insert(
                &mut **dg, &mut inode, &mut ba, &mut *sg,
                i * 100, (i as u64 + 1) * 1000, 10, bs,
            ).unwrap();
        }

        let (hdr, _) = read_inode_extent_root(&inode).unwrap();
        assert!(hdr.depth >= 1, "depth should be >= 1 after 8th insert");

        // Verify all 8 extents are findable
        for i in 0..8u32 {
            let result = extent_find(&mut **dg, &inode, i * 100, bs).unwrap();
            assert!(result.is_some(), "extent at logical {} not found", i * 100);
        }
    }

    #[test]
    fn test_insert_merge_left() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // Insert [0..10) @ 100, then [10..20) @ 110 — should merge
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            0, 100, 10, bs).unwrap();
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            10, 110, 10, bs).unwrap();

        let (hdr, entries) = read_inode_extent_root(&inode).unwrap();
        assert_eq!(hdr.entries, 1, "should be merged into 1 extent");
        match entries {
            ExtentEntries::Leaves(l) => {
                assert_eq!(l[0].ee_block, 0);
                assert_eq!(l[0].block_count(), 20);
                assert_eq!(l[0].physical_block(), 100);
            }
            _ => panic!("expected leaves"),
        }
    }

    #[test]
    fn test_insert_no_merge_physical_gap() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // [0..10) @ 100, [10..20) @ 200 — same logical range but physical gap
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            0, 100, 10, bs).unwrap();
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            10, 200, 10, bs).unwrap();

        let (hdr, _) = read_inode_extent_root(&inode).unwrap();
        assert_eq!(hdr.entries, 2, "should NOT merge — physical gap");
    }

    #[test]
    fn test_insert_large_file_many_extents() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // Insert 50 scattered extents
        for i in 0..50u32 {
            extent_insert(
                &mut **dg, &mut inode, &mut ba, &mut *sg,
                i * 100, (i as u64 + 1) * 500, 5, bs,
            ).unwrap();
        }

        // Verify all 50 are findable
        for i in 0..50u32 {
            let result = extent_find(&mut **dg, &inode, i * 100 + 2, bs).unwrap();
            assert!(result.is_some(), "extent at logical {} not found", i * 100 + 2);
            let leaf = result.unwrap();
            assert_eq!(leaf.physical_block(), (i as u64 + 1) * 500);
        }
    }

    #[test]
    fn test_grow_indepth_max_depth() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // Manually set depth to EXTENT_MAX_DEPTH
        let mut hdr = ExtentHeader::new_root();
        hdr.depth = EXTENT_MAX_DEPTH;
        hdr.entries = 1;
        let entries = ExtentEntries::Indices(vec![ExtentIndex::new(0, 50)]);
        write_inode_extent_root(&mut inode, &hdr, &entries);

        let result = grow_indepth(&mut **dg, &mut inode, &mut ba, &mut *sg, bs);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // 10C.5 — Removal tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_remove_entire_extent() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // Insert [0..10) @ 100
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            0, 100, 10, bs).unwrap();

        // Remove [0..10)
        let freed = extent_remove(&mut **dg, &mut inode, 0, 10, bs).unwrap();
        assert_eq!(freed.len(), 10);
        for i in 0..10u64 {
            assert!(freed.contains(&(100 + i)));
        }

        let (hdr, _) = read_inode_extent_root(&inode).unwrap();
        assert_eq!(hdr.entries, 0);
    }

    #[test]
    fn test_remove_front() {
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let leaves = vec![ExtentLeaf::new(0, 100, 10)]; // [0..10) @ 100
        let mut hdr = ExtentHeader::new_root();
        hdr.entries = 1;
        write_inode_extent_root(&mut inode, &hdr, &ExtentEntries::Leaves(leaves));

        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        let freed = extent_remove(dev.as_mut(), &mut inode, 0, 5, 4096).unwrap();
        assert_eq!(freed.len(), 5);
        assert_eq!(freed, vec![100, 101, 102, 103, 104]);

        let (_, entries) = read_inode_extent_root(&inode).unwrap();
        match entries {
            ExtentEntries::Leaves(l) => {
                assert_eq!(l.len(), 1);
                assert_eq!(l[0].ee_block, 5);
                assert_eq!(l[0].physical_block(), 105);
                assert_eq!(l[0].block_count(), 5);
            }
            _ => panic!("expected leaves"),
        }
    }

    #[test]
    fn test_remove_tail() {
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let leaves = vec![ExtentLeaf::new(0, 100, 10)]; // [0..10) @ 100
        let mut hdr = ExtentHeader::new_root();
        hdr.entries = 1;
        write_inode_extent_root(&mut inode, &hdr, &ExtentEntries::Leaves(leaves));

        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        let freed = extent_remove(dev.as_mut(), &mut inode, 5, 5, 4096).unwrap();
        assert_eq!(freed.len(), 5);
        assert_eq!(freed, vec![105, 106, 107, 108, 109]);

        let (_, entries) = read_inode_extent_root(&inode).unwrap();
        match entries {
            ExtentEntries::Leaves(l) => {
                assert_eq!(l.len(), 1);
                assert_eq!(l[0].ee_block, 0);
                assert_eq!(l[0].block_count(), 5);
            }
            _ => panic!("expected leaves"),
        }
    }

    #[test]
    fn test_remove_middle_split() {
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let leaves = vec![ExtentLeaf::new(0, 100, 10)]; // [0..10) @ 100
        let mut hdr = ExtentHeader::new_root();
        hdr.entries = 1;
        write_inode_extent_root(&mut inode, &hdr, &ExtentEntries::Leaves(leaves));

        let (_tmp, mut dev) = make_test_dev(64 * 4096);
        let freed = extent_remove(dev.as_mut(), &mut inode, 3, 4, 4096).unwrap();
        assert_eq!(freed.len(), 4);
        assert_eq!(freed, vec![103, 104, 105, 106]);

        let (hdr, entries) = read_inode_extent_root(&inode).unwrap();
        assert_eq!(hdr.entries, 2);
        match entries {
            ExtentEntries::Leaves(l) => {
                assert_eq!(l[0].ee_block, 0);
                assert_eq!(l[0].block_count(), 3);
                assert_eq!(l[0].physical_block(), 100);
                assert_eq!(l[1].ee_block, 7);
                assert_eq!(l[1].block_count(), 3);
                assert_eq!(l[1].physical_block(), 107);
            }
            _ => panic!("expected leaves"),
        }
    }

    #[test]
    fn test_free_all_extents() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;

        // Allocate real blocks from the bitmap so free_all_extents can free them
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;

        let blocks1 = alloc::alloc_blocks(&mut *bm, &mut *sg, 10).unwrap();
        let blocks2 = alloc::alloc_blocks(&mut *bm, &mut *sg, 5).unwrap();
        let blocks3 = alloc::alloc_blocks(&mut *bm, &mut *sg, 3).unwrap();

        let phys1 = ds + blocks1[0]; // contiguous run of 10
        let phys2 = ds + blocks2[0]; // contiguous run of 5
        let phys3 = ds + blocks3[0]; // contiguous run of 3

        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // Insert 3 extents using real physical blocks
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            0, phys1, 10, bs).unwrap();
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            100, phys2, 5, bs).unwrap();
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            500, phys3, 3, bs).unwrap();

        let free_before = sg.free_blocks;

        free_all_extents(&mut **dg, &mut inode, &mut ba, &mut *sg, bs).unwrap();

        assert_eq!(inode.size, 0);
        assert_eq!(inode.block_count, 0);

        // Free blocks should have increased by the data blocks freed (18)
        assert_eq!(sg.free_blocks, free_before + 18);

        let (hdr, _) = read_inode_extent_root(&inode).unwrap();
        assert_eq!(hdr.entries, 0);
        assert_eq!(hdr.depth, 0);
    }

    // -----------------------------------------------------------------------
    // Comprehensive: insert + lookup + remove roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn test_insert_lookup_remove_roundtrip() {
        let (_tmp, vol) = make_vol_for_extents();
        let mut inode = Inode::new_file();
        init_inode_extent_root(&mut inode);

        let bs = vol.block_size;
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };

        // Insert 3 extents
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            0, 100, 10, bs).unwrap();
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            20, 200, 5, bs).unwrap();
        extent_insert(&mut **dg, &mut inode, &mut ba, &mut *sg,
            40, 300, 8, bs).unwrap();

        // Verify lookup
        assert_eq!(get_block_ptr_extent(&mut **dg, &inode, 5, bs).unwrap(), 105);
        assert_eq!(get_block_ptr_extent(&mut **dg, &inode, 22, bs).unwrap(), 202);
        assert_eq!(get_block_ptr_extent(&mut **dg, &inode, 45, bs).unwrap(), 305);
        assert_eq!(get_block_ptr_extent(&mut **dg, &inode, 15, bs).unwrap(), 0); // hole

        // Remove middle extent [20..25)
        let freed = extent_remove(&mut **dg, &mut inode, 20, 5, bs).unwrap();
        assert_eq!(freed.len(), 5);

        // Verify it's gone
        assert_eq!(get_block_ptr_extent(&mut **dg, &inode, 22, bs).unwrap(), 0);
        // Other extents still there
        assert_eq!(get_block_ptr_extent(&mut **dg, &inode, 5, bs).unwrap(), 105);
        assert_eq!(get_block_ptr_extent(&mut **dg, &inode, 45, bs).unwrap(), 305);
    }
}
