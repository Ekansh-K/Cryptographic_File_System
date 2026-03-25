//! HTree directory indexing — hash-indexed B-tree for fast directory lookups.
//!
//! Provides O(1) average-case lookup for directories with thousands of entries,
//! replacing the linear O(n) scan used by small directories. Design inspired by
//! ext4's dx_tree, adapted for CFS's 128-byte DirEntry format.
//!
//! # On-Disk Layout
//!
//! An HTree directory's first data block (logical block 0) has a fixed header:
//!
//! ```text
//! Offset  Size   Field
//! 0       4      dot_inode       u32 — inode of this directory
//! 4       4      dotdot_inode    u32 — inode of parent directory
//! 8       24     dx_root_info    HTree metadata
//! 32      ...    dx_entries[]    Root-level hash → block mappings (8 bytes each)
//! BS-C    C      tail            v3 checksum record (DIR_ENTRY_SIZE bytes)
//! ```
//!
//! Leaf blocks use the standard DirEntry format (with v3 checksums).

use anyhow::{bail, Result};
use siphasher::sip::SipHasher;
use std::hash::Hasher;

use crate::block_device::CFSBlockDevice;
use super::alloc::BlockAlloc;
use super::dir::{self, DirEntry, DIR_ENTRY_SIZE, stamp_checksum};
use super::file_io::{get_block_ptr, set_block_ptr};
use super::inode::{Inode, INODE_FLAG_HTREE};
use super::superblock::{Superblock, FEATURE_HTREE};
use super::INODE_DIR;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Size of a DxEntry on disk (hash: u32 + block: u32).
pub const DX_ENTRY_SIZE: usize = 8;

/// Fixed header size in the root block: dot_inode(4) + dotdot_inode(4) + DxRootInfo(24) = 32 bytes.
pub const DX_ROOT_HEADER_SIZE: usize = 32;

/// Header size for internal DxNode blocks.
pub const DX_NODE_HEADER_SIZE: usize = 32;

/// Hash algorithm identifier for SipHash-2-4.
pub const HASH_VERSION_SIPHASH24: u8 = 0;

/// Maximum HTree depth (root + 1 internal level + leaves).
pub const MAX_HTREE_DEPTH: u8 = 2;

// ---------------------------------------------------------------------------
// Hash Function
// ---------------------------------------------------------------------------

/// Compute the directory entry hash using SipHash-2-4 with a volume-specific seed.
///
/// Returns a 32-bit hash value. The value 0 is reserved (unused entry marker),
/// so inputs that hash to 0 are mapped to 1.
pub fn dir_hash(name: &[u8], hash_seed: &[u8; 8]) -> u32 {
    let k0 = u64::from_le_bytes(*hash_seed);
    let k1 = k0 ^ 0x5A5A_5A5A_5A5A_5A5A;

    let mut hasher = SipHasher::new_with_keys(k0, k1);
    hasher.write(name);
    let hash64 = hasher.finish();

    let hash32 = (hash64 & 0xFFFF_FFFF) as u32;
    if hash32 == 0 { 1 } else { hash32 }
}

// ---------------------------------------------------------------------------
// DxEntry — 8-byte hash→block index entry
// ---------------------------------------------------------------------------

/// A single hash → block mapping in an HTree index.
/// Entries are sorted by hash. Each covers the range [self.hash, next.hash).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DxEntry {
    pub hash: u32,
    pub block: u32,
}

impl DxEntry {
    pub fn new(hash: u32, block: u32) -> Self {
        Self { hash, block }
    }

    pub fn serialize(&self) -> [u8; DX_ENTRY_SIZE] {
        let mut buf = [0u8; DX_ENTRY_SIZE];
        buf[0..4].copy_from_slice(&self.hash.to_le_bytes());
        buf[4..8].copy_from_slice(&self.block.to_le_bytes());
        buf
    }

    pub fn deserialize(buf: &[u8]) -> Self {
        Self {
            hash: u32::from_le_bytes(buf[0..4].try_into().unwrap()),
            block: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
        }
    }
}

// ---------------------------------------------------------------------------
// DxRootInfo — HTree metadata in root block
// ---------------------------------------------------------------------------

/// HTree root metadata (24 bytes on disk at offset 8 in the root block).
#[derive(Debug, Clone)]
pub struct DxRootInfo {
    pub hash_version: u8,
    pub depth: u8,
    pub indirect_levels: u8,
    pub count: u32,
    pub limit: u32,
    pub hash_seed_copy: [u8; 8],
}

impl DxRootInfo {
    pub fn new(hash_seed: &[u8; 8], block_size: u32) -> Self {
        Self {
            hash_version: HASH_VERSION_SIPHASH24,
            depth: 0,
            indirect_levels: 0,
            count: 1,
            limit: Self::compute_limit(block_size),
            hash_seed_copy: *hash_seed,
        }
    }

    /// Maximum dx_entries that fit in the root block.
    /// Available space = block_size - header(32) - tail(DIR_ENTRY_SIZE for v3 checksum).
    pub fn compute_limit(block_size: u32) -> u32 {
        let tail_size = DIR_ENTRY_SIZE; // v3 checksum slot
        let available = block_size as usize - DX_ROOT_HEADER_SIZE - tail_size;
        (available / DX_ENTRY_SIZE) as u32
    }

    pub fn serialize(&self, buf: &mut [u8]) {
        buf[0] = self.hash_version;
        buf[1] = self.depth;
        buf[2] = self.indirect_levels;
        buf[3] = 0; // reserved
        buf[4..8].copy_from_slice(&self.count.to_le_bytes());
        buf[8..12].copy_from_slice(&self.limit.to_le_bytes());
        buf[12..16].fill(0); // reserved
        buf[16..24].copy_from_slice(&self.hash_seed_copy);
    }

    pub fn deserialize(buf: &[u8]) -> Result<Self> {
        if buf.len() < 24 {
            bail!("DxRootInfo too short: {} < 24", buf.len());
        }
        Ok(Self {
            hash_version: buf[0],
            depth: buf[1],
            indirect_levels: buf[2],
            count: u32::from_le_bytes(buf[4..8].try_into().unwrap()),
            limit: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
            hash_seed_copy: buf[16..24].try_into().unwrap(),
        })
    }
}

// ---------------------------------------------------------------------------
// DxNode — internal node (depth > 0)
// ---------------------------------------------------------------------------

/// Internal HTree node, used when depth > 0.
#[derive(Debug, Clone)]
pub struct DxNode {
    pub count: u32,
    pub limit: u32,
    pub entries: Vec<DxEntry>,
}

impl DxNode {
    /// Maximum dx_entries in an internal node block.
    pub fn compute_limit(block_size: u32) -> u32 {
        let tail_size = DIR_ENTRY_SIZE;
        let available = block_size as usize - DX_NODE_HEADER_SIZE - tail_size;
        (available / DX_ENTRY_SIZE) as u32
    }

    pub fn new(block_size: u32) -> Self {
        Self {
            count: 0,
            limit: Self::compute_limit(block_size),
            entries: Vec::new(),
        }
    }

    pub fn serialize(&self, block_size: u32) -> Vec<u8> {
        let mut buf = vec![0u8; block_size as usize];
        // Fake dir entry header area (compatibility / identification)
        buf[4..8].copy_from_slice(&block_size.to_le_bytes());
        buf[12..16].copy_from_slice(&self.count.to_le_bytes());
        buf[16..20].copy_from_slice(&self.limit.to_le_bytes());
        for (i, entry) in self.entries.iter().enumerate() {
            let offset = DX_NODE_HEADER_SIZE + i * DX_ENTRY_SIZE;
            buf[offset..offset + DX_ENTRY_SIZE].copy_from_slice(&entry.serialize());
        }
        // Stamp v3 checksum on the node block
        stamp_checksum(&mut buf, block_size);
        buf
    }

    pub fn deserialize(buf: &[u8]) -> Result<Self> {
        if buf.len() < DX_NODE_HEADER_SIZE {
            bail!("DxNode too short");
        }
        let count = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        let limit = u32::from_le_bytes(buf[16..20].try_into().unwrap());

        let mut entries = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let offset = DX_NODE_HEADER_SIZE + i * DX_ENTRY_SIZE;
            if offset + DX_ENTRY_SIZE > buf.len() {
                break;
            }
            entries.push(DxEntry::deserialize(&buf[offset..offset + DX_ENTRY_SIZE]));
        }

        Ok(Self { count, limit, entries })
    }
}

// ---------------------------------------------------------------------------
// HTree — in-memory representation of an HTree directory index
// ---------------------------------------------------------------------------

/// In-memory HTree directory index.
pub struct HTree {
    /// Directory inode index.
    pub dir_inode_idx: u32,
    /// Root block info.
    pub root_info: DxRootInfo,
    /// Root-level dx_entries.
    pub root_entries: Vec<DxEntry>,
    /// Volume block size.
    pub block_size: u32,
    /// Hash seed from superblock.
    pub hash_seed: [u8; 8],
    /// Dot inode (self-reference, stored in root block header).
    dot_inode: u32,
    /// Dotdot inode (parent reference, stored in root block header).
    dotdot_inode: u32,
}

impl HTree {
    // -------------------------------------------------------------------
    // Load / Save
    // -------------------------------------------------------------------

    /// Load an HTree from the root block of a directory inode.
    pub fn load(
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &Inode,
        dir_inode_idx: u32,
        sb: &Superblock,
    ) -> Result<Self> {
        let root_phys = get_block_ptr(dev, dir_inode, 0, sb.block_size)?;
        if root_phys == 0 {
            bail!("directory has no root block");
        }

        let bs = sb.block_size;
        let mut buf = vec![0u8; bs as usize];
        dev.read(root_phys * bs as u64, &mut buf)?;

        let dot_inode = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let dotdot_inode = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let root_info = DxRootInfo::deserialize(&buf[8..32])?;

        if root_info.hash_seed_copy != sb.hash_seed {
            bail!("HTree hash_seed mismatch — directory may be from a different volume");
        }

        let tail_size = DIR_ENTRY_SIZE;
        let max_offset = bs as usize - tail_size;
        let mut entries = Vec::with_capacity(root_info.count as usize);
        for i in 0..root_info.count as usize {
            let offset = DX_ROOT_HEADER_SIZE + i * DX_ENTRY_SIZE;
            if offset + DX_ENTRY_SIZE > max_offset {
                break;
            }
            entries.push(DxEntry::deserialize(&buf[offset..offset + DX_ENTRY_SIZE]));
        }

        Ok(Self {
            dir_inode_idx,
            root_info,
            root_entries: entries,
            block_size: bs,
            hash_seed: sb.hash_seed,
            dot_inode,
            dotdot_inode,
        })
    }

    /// Write the root block back to disk.
    pub fn save_root(
        &self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &Inode,
    ) -> Result<()> {
        let root_phys = get_block_ptr(dev, dir_inode, 0, self.block_size)?;
        if root_phys == 0 {
            bail!("directory has no root block");
        }

        let mut buf = vec![0u8; self.block_size as usize];
        // Fixed header
        buf[0..4].copy_from_slice(&self.dot_inode.to_le_bytes());
        buf[4..8].copy_from_slice(&self.dotdot_inode.to_le_bytes());

        let mut info = self.root_info.clone();
        info.count = self.root_entries.len() as u32;
        info.serialize(&mut buf[8..32]);

        // dx_entries
        for (i, entry) in self.root_entries.iter().enumerate() {
            let offset = DX_ROOT_HEADER_SIZE + i * DX_ENTRY_SIZE;
            buf[offset..offset + DX_ENTRY_SIZE].copy_from_slice(&entry.serialize());
        }

        // v3 checksum
        stamp_checksum(&mut buf, self.block_size);

        dev.write(root_phys * self.block_size as u64, &buf)?;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Lookup
    // -------------------------------------------------------------------

    /// Look up a directory entry by name. Returns the entry if found.
    pub fn lookup(
        &self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &Inode,
        name: &str,
    ) -> Result<Option<DirEntry>> {
        let hash = dir_hash(name.as_bytes(), &self.hash_seed);
        let leaf_logical = self.find_leaf_block(dev, dir_inode, hash)?;

        let leaf_phys = get_block_ptr(dev, dir_inode, leaf_logical, self.block_size)?;
        if leaf_phys == 0 {
            return Ok(None);
        }

        let mut buf = vec![0u8; self.block_size as usize];
        dev.read(leaf_phys * self.block_size as u64, &mut buf)?;

        let entries = parse_leaf_entries(&buf, self.block_size);
        for entry in entries {
            if !entry.is_unused() && entry.name_str() == name {
                return Ok(Some(entry));
            }
        }

        Ok(None)
    }

    /// Find the logical block number of the leaf block for a given hash.
    fn find_leaf_block(
        &self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &Inode,
        hash: u32,
    ) -> Result<u64> {
        let root_block_logical = binary_search_entries(&self.root_entries, hash);

        if self.root_info.depth == 0 {
            return Ok(root_block_logical as u64);
        }

        // Depth > 0: root entry points to an internal node (by logical block number)
        let node_phys = get_block_ptr(dev, dir_inode, root_block_logical as u64, self.block_size)?;
        if node_phys == 0 {
            bail!("HTree internal node not allocated at logical block {}", root_block_logical);
        }
        let mut buf = vec![0u8; self.block_size as usize];
        dev.read(node_phys * self.block_size as u64, &mut buf)?;
        let node = DxNode::deserialize(&buf)?;

        let leaf_logical = binary_search_entries(&node.entries, hash);
        Ok(leaf_logical as u64)
    }

    // -------------------------------------------------------------------
    // Insert
    // -------------------------------------------------------------------

    /// Insert a new directory entry into the HTree.
    pub fn insert(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &mut Inode,
        alloc: &mut BlockAlloc<'_>,
        sb: &mut Superblock,
        name: &str,
        inode_index: u32,
        file_type: u8,
    ) -> Result<()> {
        // Check for duplicates first
        if let Some(_) = self.lookup(dev, dir_inode, name)? {
            bail!("entry already exists: {}", name);
        }

        let hash = dir_hash(name.as_bytes(), &self.hash_seed);
        let leaf_logical = self.find_leaf_block(dev, dir_inode, hash)?;
        let leaf_phys = get_block_ptr(dev, dir_inode, leaf_logical, self.block_size)?;
        if leaf_phys == 0 {
            bail!("HTree leaf not allocated at logical block {}", leaf_logical);
        }

        let mut buf = vec![0u8; self.block_size as usize];
        dev.read(leaf_phys * self.block_size as u64, &mut buf)?;

        let new_entry = DirEntry::new(inode_index, file_type, name)?;

        // Try to add to existing leaf
        if try_add_entry_to_block(&mut buf, &new_entry, self.block_size) {
            stamp_checksum(&mut buf, self.block_size);
            dev.write(leaf_phys * self.block_size as u64, &buf)?;
            return Ok(());
        }

        // Leaf is full — split
        self.split_leaf(dev, dir_inode, alloc, sb, leaf_logical, leaf_phys, &buf, new_entry)?;
        Ok(())
    }

    /// Split a full leaf block to make room for a new entry.
    fn split_leaf(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &mut Inode,
        alloc: &mut BlockAlloc<'_>,
        sb: &mut Superblock,
        _old_leaf_logical: u64,
        old_leaf_phys: u64,
        old_buf: &[u8],
        new_entry: DirEntry,
    ) -> Result<()> {
        // Collect all entries from the full leaf + the new one
        let mut entries = parse_leaf_entries(old_buf, self.block_size);
        entries.push(new_entry);

        // Sort by hash
        let mut hashed: Vec<(u32, DirEntry)> = entries
            .into_iter()
            .map(|e| (dir_hash(e.name_str().as_bytes(), &self.hash_seed), e))
            .collect();
        hashed.sort_by_key(|(h, _)| *h);

        // Split in half
        let mid = hashed.len() / 2;
        let left_entries: Vec<DirEntry> = hashed[..mid].iter().map(|(_, e)| e.clone()).collect();
        let right_entries: Vec<DirEntry> = hashed[mid..].iter().map(|(_, e)| e.clone()).collect();
        let split_hash = hashed[mid].0;

        // Write left entries back to old leaf
        let mut left_buf = vec![0u8; self.block_size as usize];
        write_entries_to_block(&mut left_buf, &left_entries, self.block_size);
        stamp_checksum(&mut left_buf, self.block_size);
        dev.write(old_leaf_phys * self.block_size as u64, &left_buf)?;

        // Allocate a new leaf block
        let new_blocks = alloc.alloc(dev, sb, 1)?;
        let new_leaf_phys = new_blocks[0];

        // Find next logical block number for the directory
        let new_leaf_logical = self.next_logical_block(dir_inode);
        set_block_ptr(dev, dir_inode, new_leaf_logical, new_leaf_phys, self.block_size, alloc, sb)?;
        dir_inode.block_count += 1;
        dir_inode.size += self.block_size as u64;

        // Write right entries to new leaf
        let mut right_buf = vec![0u8; self.block_size as usize];
        write_entries_to_block(&mut right_buf, &right_entries, self.block_size);
        stamp_checksum(&mut right_buf, self.block_size);
        dev.write(new_leaf_phys * self.block_size as u64, &right_buf)?;

        // Add dx_entry for the new leaf
        let new_dx = DxEntry::new(split_hash, new_leaf_logical as u32);
        self.insert_dx_entry(dev, dir_inode, alloc, sb, new_dx)?;

        Ok(())
    }

    /// Insert a new dx_entry into the root index (or increase depth).
    fn insert_dx_entry(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &mut Inode,
        alloc: &mut BlockAlloc<'_>,
        sb: &mut Superblock,
        new_entry: DxEntry,
    ) -> Result<()> {
        if (self.root_entries.len() as u32) < self.root_info.limit {
            // Room in root: insert sorted
            let pos = self.root_entries
                .binary_search_by_key(&new_entry.hash, |e| e.hash)
                .unwrap_or_else(|pos| pos);
            self.root_entries.insert(pos, new_entry);
            self.root_info.count = self.root_entries.len() as u32;
            self.save_root(dev, dir_inode)?;
            Ok(())
        } else if self.root_info.depth == 0 {
            // Root is full, depth=0 → push entries into a new internal node
            self.increase_depth(dev, dir_inode, alloc, sb, new_entry)
        } else {
            bail!("directory too large: HTree depth limit reached (max {})", MAX_HTREE_DEPTH)
        }
    }

    /// Increase HTree depth from 0 to 1. Moves current root dx_entries into a
    /// new internal node, then root points to the node.
    fn increase_depth(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &mut Inode,
        alloc: &mut BlockAlloc<'_>,
        sb: &mut Superblock,
        new_entry: DxEntry,
    ) -> Result<()> {
        // Allocate a block for the new internal node
        let node_blocks = alloc.alloc(dev, sb, 1)?;
        let node_phys = node_blocks[0];
        let node_logical = self.next_logical_block(dir_inode);
        set_block_ptr(dev, dir_inode, node_logical, node_phys, self.block_size, alloc, sb)?;
        dir_inode.block_count += 1;
        dir_inode.size += self.block_size as u64;

        // Build the internal node with all current root entries + the new one
        let mut node = DxNode::new(self.block_size);
        node.entries = self.root_entries.clone();
        let pos = node.entries
            .binary_search_by_key(&new_entry.hash, |e| e.hash)
            .unwrap_or_else(|pos| pos);
        node.entries.insert(pos, new_entry);
        node.count = node.entries.len() as u32;

        // Write the node to disk
        let buf = node.serialize(self.block_size);
        dev.write(node_phys * self.block_size as u64, &buf)?;

        // Replace root entries with a single entry pointing to the node
        self.root_entries = vec![DxEntry::new(0, node_logical as u32)];
        self.root_info.count = 1;
        self.root_info.depth = 1;
        self.root_info.indirect_levels = 1;

        self.save_root(dev, dir_inode)?;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Remove
    // -------------------------------------------------------------------

    /// Remove a directory entry by name.
    pub fn remove(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &Inode,
        name: &str,
    ) -> Result<()> {
        let hash = dir_hash(name.as_bytes(), &self.hash_seed);
        let leaf_logical = self.find_leaf_block(dev, dir_inode, hash)?;
        let leaf_phys = get_block_ptr(dev, dir_inode, leaf_logical, self.block_size)?;
        if leaf_phys == 0 {
            bail!("entry not found in HTree: {}", name);
        }

        let mut buf = vec![0u8; self.block_size as usize];
        dev.read(leaf_phys * self.block_size as u64, &mut buf)?;

        if !remove_entry_from_block(&mut buf, name, self.block_size) {
            bail!("entry not found in HTree leaf: {}", name);
        }

        // Write back the leaf
        stamp_checksum(&mut buf, self.block_size);
        dev.write(leaf_phys * self.block_size as u64, &buf)?;

        // NOTE: We do NOT free empty leaf blocks or coalesce in this phase.
        // Empty leaves are left allocated to avoid complex dx_entry removal.
        // This avoids corruption risk and is the same approach ext4 takes.

        Ok(())
    }

    // -------------------------------------------------------------------
    // Readdir
    // -------------------------------------------------------------------

    /// Read all directory entries for readdir/ls.
    /// Returns entries in hash order (not alphabetical). Includes "." and "..".
    pub fn readdir(
        &self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &Inode,
    ) -> Result<Vec<DirEntry>> {
        let mut all_entries = Vec::new();

        // Add "." and ".." from root block header
        all_entries.push(DirEntry::new(self.dot_inode, INODE_DIR as u8, ".")?);
        all_entries.push(DirEntry::new(self.dotdot_inode, INODE_DIR as u8, "..")?);

        // Walk all leaf blocks via the index
        let leaf_logicals = self.collect_all_leaf_blocks(dev, dir_inode)?;

        for leaf_logical in leaf_logicals {
            let leaf_phys = get_block_ptr(dev, dir_inode, leaf_logical as u64, self.block_size)?;
            if leaf_phys == 0 {
                continue;
            }
            let mut buf = vec![0u8; self.block_size as usize];
            dev.read(leaf_phys * self.block_size as u64, &mut buf)?;

            let entries = parse_leaf_entries(&buf, self.block_size);
            all_entries.extend(entries);
        }

        Ok(all_entries)
    }

    /// Collect all leaf block logical numbers by walking the index tree.
    fn collect_all_leaf_blocks(
        &self,
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &Inode,
    ) -> Result<Vec<u32>> {
        let mut leaves = Vec::new();

        if self.root_info.depth == 0 {
            for entry in &self.root_entries {
                leaves.push(entry.block);
            }
        } else {
            // Root entries point to internal nodes
            for root_entry in &self.root_entries {
                let node_phys = get_block_ptr(
                    dev, dir_inode, root_entry.block as u64, self.block_size,
                )?;
                if node_phys == 0 {
                    continue;
                }
                let mut buf = vec![0u8; self.block_size as usize];
                dev.read(node_phys * self.block_size as u64, &mut buf)?;
                let node = DxNode::deserialize(&buf)?;

                for entry in &node.entries {
                    leaves.push(entry.block);
                }
            }
        }

        Ok(leaves)
    }

    // -------------------------------------------------------------------
    // Conversion: linear → HTree
    // -------------------------------------------------------------------

    /// Convert a linear directory to HTree format.
    ///
    /// Reads all existing entries from the directory, reorganizes block 0 as
    /// the HTree root, and writes real entries into leaf block(s) starting at
    /// logical block 1.
    pub fn convert_from_linear(
        dev: &mut dyn CFSBlockDevice,
        dir_inode: &mut Inode,
        dir_inode_idx: u32,
        alloc: &mut BlockAlloc<'_>,
        sb: &mut Superblock,
    ) -> Result<Self> {
        let bs = sb.block_size;

        // Step 1: Read all existing entries
        let all_entries = dir::read_dir_entries(dev, dir_inode, bs)?;

        let dot_inode = dir_inode_idx;
        let dotdot_inode = all_entries
            .iter()
            .find(|e| e.name_str() == "..")
            .map(|e| e.inode_index)
            .unwrap_or(0);

        // Filter out "." and ".." — they go in the root header
        let real_entries: Vec<DirEntry> = all_entries
            .into_iter()
            .filter(|e| {
                let n = e.name_str();
                n != "." && n != ".."
            })
            .collect();

        // Step 2: Hash and sort entries
        let mut hashed: Vec<(u32, DirEntry)> = real_entries
            .into_iter()
            .map(|e| (dir_hash(e.name_str().as_bytes(), &sb.hash_seed), e))
            .collect();
        hashed.sort_by_key(|(h, _)| *h);

        // Step 3: Determine how many leaf blocks we need
        let entries_per_leaf = leaf_capacity(bs);
        let leaf_count = if hashed.is_empty() {
            1 // At least one leaf block
        } else {
            (hashed.len() + entries_per_leaf - 1) / entries_per_leaf
        };

        // Step 4: Allocate leaf blocks. We need (leaf_count - existing_non_root_blocks)
        // new blocks. The directory currently has blocks 0..block_count-1 in linear mode.
        // Block 0 becomes the root. Blocks 1+ can be reused as leaves.
        // If we need more leaves than existing extra blocks, we allocate more.
        let existing_extra = if dir_inode.block_count > 1 {
            (dir_inode.block_count - 1) as usize
        } else {
            0
        };

        // We re-use blocks 1..existing_extra as leaf blocks, then allocate
        // any additional ones needed.
        let mut leaf_logicals: Vec<u64> = (1..=existing_extra as u64).collect();

        let additional_needed = leaf_count.saturating_sub(existing_extra);
        for _ in 0..additional_needed {
            let new_blocks = alloc.alloc(dev, sb, 1)?;
            let new_phys = new_blocks[0];
            let logical = leaf_logicals.len() as u64 + 1; // +1 because block 0 is root
            set_block_ptr(dev, dir_inode, logical, new_phys, bs, alloc, sb)?;
            leaf_logicals.push(logical);
            dir_inode.block_count += 1;
        }

        // Step 5: Distribute entries across leaf blocks
        let mut dx_entries = Vec::new();
        for (leaf_idx, chunk) in hashed.chunks(entries_per_leaf).enumerate() {
            let leaf_logical = leaf_logicals[leaf_idx];
            let leaf_phys = get_block_ptr(dev, dir_inode, leaf_logical, bs)?;

            let entries_vec: Vec<DirEntry> = chunk.iter().map(|(_, e)| e.clone()).collect();
            let mut leaf_buf = vec![0u8; bs as usize];
            write_entries_to_block(&mut leaf_buf, &entries_vec, bs);
            stamp_checksum(&mut leaf_buf, bs);
            dev.write(leaf_phys * bs as u64, &leaf_buf)?;

            // First hash of this chunk (or 0 for the first leaf)
            let hash_val = if leaf_idx == 0 { 0 } else { chunk[0].0 };
            dx_entries.push(DxEntry::new(hash_val, leaf_logical as u32));
        }

        // Handle case where hashed is empty — still need one leaf
        if dx_entries.is_empty() {
            let leaf_logical = leaf_logicals[0];
            let leaf_phys = get_block_ptr(dev, dir_inode, leaf_logical, bs)?;
            let mut leaf_buf = vec![0u8; bs as usize];
            stamp_checksum(&mut leaf_buf, bs);
            dev.write(leaf_phys * bs as u64, &leaf_buf)?;
            dx_entries.push(DxEntry::new(0, leaf_logical as u32));
        }

        // Step 6: Build HTree root
        let root_info = DxRootInfo {
            hash_version: HASH_VERSION_SIPHASH24,
            depth: 0,
            indirect_levels: 0,
            count: dx_entries.len() as u32,
            limit: DxRootInfo::compute_limit(bs),
            hash_seed_copy: sb.hash_seed,
        };

        let htree = Self {
            dir_inode_idx,
            root_info,
            root_entries: dx_entries,
            block_size: bs,
            hash_seed: sb.hash_seed,
            dot_inode,
            dotdot_inode,
        };

        // Write root block
        htree.save_root(dev, dir_inode)?;

        // Step 7: Set INODE_FLAG_HTREE on directory inode
        dir_inode.flags |= INODE_FLAG_HTREE;

        // Update directory size to reflect all blocks
        dir_inode.size = dir_inode.block_count as u64 * bs as u64;

        Ok(htree)
    }

    // -------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------

    /// Find the next available logical block number for this directory.
    fn next_logical_block(&self, dir_inode: &Inode) -> u64 {
        // Directory size / block_size gives the next logical block to use
        dir_inode.size / self.block_size as u64
    }
}

// ---------------------------------------------------------------------------
// Free functions: block-level helpers
// ---------------------------------------------------------------------------

/// Binary search a sorted dx_entries array for the entry whose hash range
/// contains the target hash. Returns the block number from the matching entry.
fn binary_search_entries(entries: &[DxEntry], hash: u32) -> u32 {
    if entries.is_empty() {
        return 0;
    }
    // Find the last entry where entry.hash <= hash
    let mut lo = 0usize;
    let mut hi = entries.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if entries[mid].hash <= hash {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    // lo is the first entry with hash > target.
    // We want the one before it (last with hash <= target).
    if lo == 0 { entries[0].block } else { entries[lo - 1].block }
}

/// How many DirEntries fit in a leaf block (v3 with checksum).
pub fn leaf_capacity(block_size: u32) -> usize {
    // Last slot is reserved for checksum record
    (block_size as usize / DIR_ENTRY_SIZE) - 1
}

/// Parse non-unused DirEntries from a leaf block.
fn parse_leaf_entries(buf: &[u8], block_size: u32) -> Vec<DirEntry> {
    let n = leaf_capacity(block_size);
    let mut entries = Vec::new();
    for i in 0..n {
        let offset = i * DIR_ENTRY_SIZE;
        let entry_buf: &[u8; DIR_ENTRY_SIZE] =
            buf[offset..offset + DIR_ENTRY_SIZE].try_into().unwrap();
        let entry = DirEntry::deserialize(entry_buf);
        if !entry.is_unused() {
            entries.push(entry);
        }
    }
    entries
}

/// Try to add a DirEntry to a block. Returns true if a free slot was found.
fn try_add_entry_to_block(buf: &mut [u8], entry: &DirEntry, block_size: u32) -> bool {
    let n = leaf_capacity(block_size);
    for i in 0..n {
        let offset = i * DIR_ENTRY_SIZE;
        let existing: &[u8; DIR_ENTRY_SIZE] =
            buf[offset..offset + DIR_ENTRY_SIZE].try_into().unwrap();
        let existing_entry = DirEntry::deserialize(existing);
        if existing_entry.is_unused() {
            buf[offset..offset + DIR_ENTRY_SIZE].copy_from_slice(&entry.serialize());
            return true;
        }
    }
    false
}

/// Write a list of DirEntries into a block buffer (starting from offset 0).
fn write_entries_to_block(buf: &mut [u8], entries: &[DirEntry], block_size: u32) {
    buf.fill(0);
    let cap = leaf_capacity(block_size);
    for (i, entry) in entries.iter().take(cap).enumerate() {
        let offset = i * DIR_ENTRY_SIZE;
        buf[offset..offset + DIR_ENTRY_SIZE].copy_from_slice(&entry.serialize());
    }
}

/// Remove an entry by name from a block. Returns true if found and removed.
fn remove_entry_from_block(buf: &mut [u8], name: &str, block_size: u32) -> bool {
    let n = leaf_capacity(block_size);
    for i in 0..n {
        let offset = i * DIR_ENTRY_SIZE;
        let entry_buf: &[u8; DIR_ENTRY_SIZE] =
            buf[offset..offset + DIR_ENTRY_SIZE].try_into().unwrap();
        let entry = DirEntry::deserialize(entry_buf);
        if !entry.is_unused() && entry.name_str() == name {
            buf[offset..offset + DIR_ENTRY_SIZE].fill(0);
            return true;
        }
    }
    false
}

/// Check if a directory should be converted to HTree.
pub fn should_convert_to_htree(dir_inode: &Inode, sb: &Superblock) -> bool {
    // Already HTree
    if dir_inode.flags & INODE_FLAG_HTREE != 0 {
        return false;
    }
    // Feature not enabled
    if sb.features_flags & FEATURE_HTREE == 0 {
        return false;
    }
    // Not a directory
    if dir_inode.mode != INODE_DIR {
        return false;
    }
    // Convert when the directory occupies more than 1 block
    dir_inode.block_count > 1
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use crate::volume::{CFSVolume, INODE_DIR, FormatOptions, ErrorBehavior};
    use tempfile::NamedTempFile;

    // -----------------------------------------------------------------------
    // Hash function tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dir_hash_deterministic() {
        let seed = [1, 2, 3, 4, 5, 6, 7, 8];
        let h1 = dir_hash(b"hello.txt", &seed);
        let h2 = dir_hash(b"hello.txt", &seed);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_dir_hash_different_names() {
        let seed = [1, 2, 3, 4, 5, 6, 7, 8];
        let h1 = dir_hash(b"file_a.txt", &seed);
        let h2 = dir_hash(b"file_b.txt", &seed);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_dir_hash_different_seeds() {
        let s1 = [1, 2, 3, 4, 5, 6, 7, 8];
        let s2 = [8, 7, 6, 5, 4, 3, 2, 1];
        let h1 = dir_hash(b"test", &s1);
        let h2 = dir_hash(b"test", &s2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_dir_hash_never_zero() {
        let seed = [0, 0, 0, 0, 0, 0, 0, 0];
        // Test a variety of inputs — none should produce 0
        for i in 0..1000 {
            let name = format!("file_{}", i);
            assert_ne!(dir_hash(name.as_bytes(), &seed), 0);
        }
    }

    #[test]
    fn test_dir_hash_empty_name() {
        let seed = [1, 2, 3, 4, 5, 6, 7, 8];
        let h = dir_hash(b"", &seed);
        assert_ne!(h, 0);
    }

    #[test]
    fn test_dir_hash_long_name() {
        let seed = [1, 2, 3, 4, 5, 6, 7, 8];
        let long_name = "a".repeat(122); // MAX_NAME_LEN
        let h = dir_hash(long_name.as_bytes(), &seed);
        assert_ne!(h, 0);
    }

    #[test]
    fn test_dir_hash_distribution() {
        // Chi-squared test: hash 10,000 random names into 256 buckets
        let seed = [42, 43, 44, 45, 46, 47, 48, 49];
        let mut buckets = vec![0u32; 256];
        let n = 10_000;
        for i in 0..n {
            let name = format!("test_file_{:06}", i);
            let h = dir_hash(name.as_bytes(), &seed);
            buckets[(h & 0xFF) as usize] += 1;
        }
        let expected = n as f64 / 256.0;
        let chi2: f64 = buckets
            .iter()
            .map(|&b| {
                let diff = b as f64 - expected;
                diff * diff / expected
            })
            .sum();
        // For 255 degrees of freedom, chi-squared < 350 at p=0.001
        assert!(chi2 < 400.0, "poor hash distribution: chi2={:.1}", chi2);
    }

    // -----------------------------------------------------------------------
    // DxEntry tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dx_entry_roundtrip() {
        let e = DxEntry::new(0xDEAD_BEEF, 42);
        let buf = e.serialize();
        let e2 = DxEntry::deserialize(&buf);
        assert_eq!(e, e2);
    }

    // -----------------------------------------------------------------------
    // DxRootInfo tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dx_root_info_roundtrip() {
        let seed = [10, 20, 30, 40, 50, 60, 70, 80];
        let info = DxRootInfo::new(&seed, 4096);
        let mut buf = [0u8; 24];
        info.serialize(&mut buf);
        let info2 = DxRootInfo::deserialize(&buf).unwrap();
        assert_eq!(info2.hash_version, HASH_VERSION_SIPHASH24);
        assert_eq!(info2.depth, 0);
        assert_eq!(info2.count, 1);
        assert_eq!(info2.hash_seed_copy, seed);
    }

    #[test]
    fn test_dx_root_info_compute_limit_4k() {
        // 4096 - 32 (header) - 128 (checksum slot) = 3936 / 8 = 492
        let limit = DxRootInfo::compute_limit(4096);
        assert_eq!(limit, 492);
    }

    #[test]
    fn test_dx_root_info_compute_limit_8k() {
        // 8192 - 32 - 128 = 8032 / 8 = 1004
        let limit = DxRootInfo::compute_limit(8192);
        assert_eq!(limit, 1004);
    }

    // -----------------------------------------------------------------------
    // DxNode tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dx_node_roundtrip() {
        let mut node = DxNode::new(4096);
        node.entries.push(DxEntry::new(0, 5));
        node.entries.push(DxEntry::new(100, 8));
        node.count = 2;

        let buf = node.serialize(4096);
        let node2 = DxNode::deserialize(&buf).unwrap();
        assert_eq!(node2.count, 2);
        assert_eq!(node2.entries.len(), 2);
        assert_eq!(node2.entries[0], DxEntry::new(0, 5));
        assert_eq!(node2.entries[1], DxEntry::new(100, 8));
    }

    #[test]
    fn test_dx_node_compute_limit_4k() {
        // 4096 - 32 - 128 = 3936 / 8 = 492
        let limit = DxNode::compute_limit(4096);
        assert_eq!(limit, 492);
    }

    // -----------------------------------------------------------------------
    // Binary search tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_binary_search_single_entry() {
        let entries = vec![DxEntry::new(0, 5)];
        assert_eq!(binary_search_entries(&entries, 0), 5);
        assert_eq!(binary_search_entries(&entries, 999), 5);
    }

    #[test]
    fn test_binary_search_multiple() {
        let entries = vec![
            DxEntry::new(0, 10),
            DxEntry::new(100, 20),
            DxEntry::new(200, 30),
        ];
        // hash < 100 → block 10
        assert_eq!(binary_search_entries(&entries, 50), 10);
        // hash == 100 → block 20
        assert_eq!(binary_search_entries(&entries, 100), 20);
        // 100 < hash < 200 → block 20
        assert_eq!(binary_search_entries(&entries, 150), 20);
        // hash >= 200 → block 30
        assert_eq!(binary_search_entries(&entries, 200), 30);
        assert_eq!(binary_search_entries(&entries, 999), 30);
    }

    #[test]
    fn test_binary_search_exact_boundaries() {
        let entries = vec![
            DxEntry::new(0, 1),
            DxEntry::new(50, 2),
        ];
        assert_eq!(binary_search_entries(&entries, 0), 1);
        assert_eq!(binary_search_entries(&entries, 49), 1);
        assert_eq!(binary_search_entries(&entries, 50), 2);
        assert_eq!(binary_search_entries(&entries, 51), 2);
    }

    // -----------------------------------------------------------------------
    // Leaf helpers tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_leaf_capacity_4k() {
        // 4096 / 128 = 32 - 1 = 31
        assert_eq!(leaf_capacity(4096), 31);
    }

    #[test]
    fn test_write_parse_roundtrip() {
        let entries = vec![
            DirEntry::new(1, 1, "foo.txt").unwrap(),
            DirEntry::new(2, 2, "bar").unwrap(),
        ];
        let mut buf = vec![0u8; 4096];
        write_entries_to_block(&mut buf, &entries, 4096);
        let parsed = parse_leaf_entries(&buf, 4096);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name_str(), "foo.txt");
        assert_eq!(parsed[1].name_str(), "bar");
    }

    #[test]
    fn test_try_add_and_remove() {
        let mut buf = vec![0u8; 4096];
        let e1 = DirEntry::new(1, 1, "hello.txt").unwrap();
        assert!(try_add_entry_to_block(&mut buf, &e1, 4096));

        let e2 = DirEntry::new(2, 2, "world").unwrap();
        assert!(try_add_entry_to_block(&mut buf, &e2, 4096));

        let entries = parse_leaf_entries(&buf, 4096);
        assert_eq!(entries.len(), 2);

        assert!(remove_entry_from_block(&mut buf, "hello.txt", 4096));
        let entries2 = parse_leaf_entries(&buf, 4096);
        assert_eq!(entries2.len(), 1);
        assert_eq!(entries2[0].name_str(), "world");
    }

    // -----------------------------------------------------------------------
    // should_convert_to_htree tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_should_convert_not_dir() {
        let mut inode = Inode::new_file();
        inode.block_count = 1;
        inode.size = 4096;
        let sb = test_superblock();
        assert!(!should_convert_to_htree(&inode, &sb));
    }

    #[test]
    fn test_should_convert_already_htree() {
        let mut inode = Inode::new_dir();
        inode.flags |= INODE_FLAG_HTREE;
        inode.block_count = 1;
        inode.size = 4096;
        let sb = test_superblock();
        assert!(!should_convert_to_htree(&inode, &sb));
    }

    #[test]
    fn test_should_convert_feature_disabled() {
        let mut inode = Inode::new_dir();
        inode.block_count = 1;
        inode.size = 4096;
        let mut sb = test_superblock();
        sb.features_flags &= !FEATURE_HTREE;
        assert!(!should_convert_to_htree(&inode, &sb));
    }

    #[test]
    fn test_should_convert_small_dir() {
        let inode = Inode::new_dir();
        let sb = test_superblock();
        assert!(!should_convert_to_htree(&inode, &sb));
    }

    #[test]
    fn test_should_convert_trigger() {
        let mut inode = Inode::new_dir();
        inode.block_count = 2;
        inode.size = 8192;
        let sb = test_superblock();
        assert!(should_convert_to_htree(&inode, &sb));
    }

    // -----------------------------------------------------------------------
    // Integration tests using CFSVolume
    // -----------------------------------------------------------------------

    fn test_superblock() -> Superblock {
        let mut sb = Superblock::default();
        sb.block_size = 4096;
        sb.features_flags = FEATURE_HTREE;
        sb.hash_seed = [1, 2, 3, 4, 5, 6, 7, 8];
        sb
    }

    /// Create a fresh v3 CFS volume on a temp file for testing.
    fn create_test_volume() -> (CFSVolume, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        let dev = FileBlockDevice::open(&path, Some(64 * 1024 * 1024)).unwrap();
        let opts = FormatOptions {
            block_size: 4096,
            inode_size: 256,
            inode_ratio: 4096,
            journal_percent: 0.0,
            secure_delete: false,
            volume_label: String::new(),
            default_permissions: 0o755,
            error_behavior: ErrorBehavior::Continue,
            blocks_per_group: 4096 * 8,
        };
        CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        let dev2 = FileBlockDevice::open(&path, None).unwrap();
        let vol = CFSVolume::mount(Box::new(dev2), 4096).unwrap();
        (vol, tmp)
    }

    #[test]
    fn test_htree_convert_and_lookup() {
        let (vol, _tmp) = create_test_volume();

        // Create a directory with enough files to trigger conversion
        vol.mkdir("/big").unwrap();

        // Fill the directory to exceed 1 block
        let entries_per_leaf = leaf_capacity(4096);
        let file_count = entries_per_leaf + 5; // More than 1 block can hold
        for i in 0..file_count {
            let path = format!("/big/file_{:04}", i);
            vol.create_file(&path).unwrap();
        }

        // Verify we can still list all entries
        let entries = vol.list_dir("/big").unwrap();
        // entries - "." and ".." = file_count
        let real_count = entries
            .iter()
            .filter(|e| e.name_str() != "." && e.name_str() != "..")
            .count();
        assert_eq!(real_count, file_count);

        // Verify each file is resolvable by path
        for i in 0..file_count {
            let path = format!("/big/file_{:04}", i);
            assert!(vol.exists(&path).unwrap(), "missing: {}", path);
        }
    }

    #[test]
    fn test_htree_convert_preserves_dot_entries() {
        let (vol, _tmp) = create_test_volume();
        vol.mkdir("/testdir").unwrap();

        let cap = leaf_capacity(4096);
        for i in 0..cap + 2 {
            vol.create_file(&format!("/testdir/f_{:04}", i)).unwrap();
        }

        let entries = vol.list_dir("/testdir").unwrap();
        let has_dot = entries.iter().any(|e| e.name_str() == ".");
        let has_dotdot = entries.iter().any(|e| e.name_str() == "..");
        assert!(has_dot, ". missing after HTree conversion");
        assert!(has_dotdot, ".. missing after HTree conversion");
    }

    #[test]
    fn test_htree_remove_entry() {
        let (vol, _tmp) = create_test_volume();
        vol.mkdir("/rmtest").unwrap();

        let cap = leaf_capacity(4096);
        for i in 0..cap + 2 {
            vol.create_file(&format!("/rmtest/f_{:04}", i)).unwrap();
        }

        // Delete a file
        vol.delete_file("/rmtest/f_0005").unwrap();
        assert!(!vol.exists("/rmtest/f_0005").unwrap());

        // Others still exist
        assert!(vol.exists("/rmtest/f_0000").unwrap());
        assert!(vol.exists(&format!("/rmtest/f_{:04}", cap + 1)).unwrap());
    }

    #[test]
    fn test_htree_many_inserts() {
        let (vol, _tmp) = create_test_volume();
        vol.mkdir("/many").unwrap();

        let count = 200;
        for i in 0..count {
            vol.create_file(&format!("/many/item_{:05}", i)).unwrap();
        }

        let entries = vol.list_dir("/many").unwrap();
        let real_count = entries
            .iter()
            .filter(|e| e.name_str() != "." && e.name_str() != "..")
            .count();
        assert_eq!(real_count, count);

        for i in 0..count {
            assert!(vol.exists(&format!("/many/item_{:05}", i)).unwrap());
        }
    }

    #[test]
    fn test_htree_mixed_linear_and_htree() {
        let (vol, _tmp) = create_test_volume();

        // Small directory — stays linear
        vol.mkdir("/small").unwrap();
        vol.create_file("/small/a.txt").unwrap();
        vol.create_file("/small/b.txt").unwrap();

        // Large directory — gets converted to HTree
        vol.mkdir("/large").unwrap();
        let cap = leaf_capacity(4096);
        for i in 0..cap + 5 {
            vol.create_file(&format!("/large/f_{:04}", i)).unwrap();
        }

        // Both directories work correctly
        let small = vol.list_dir("/small").unwrap();
        assert_eq!(
            small.iter().filter(|e| e.name_str() != "." && e.name_str() != "..").count(),
            2
        );

        let large = vol.list_dir("/large").unwrap();
        assert_eq!(
            large.iter().filter(|e| e.name_str() != "." && e.name_str() != "..").count(),
            cap + 5
        );
    }

    #[test]
    fn test_htree_readdir_no_duplicates() {
        let (vol, _tmp) = create_test_volume();
        vol.mkdir("/nodup").unwrap();

        let count = 100;
        for i in 0..count {
            vol.create_file(&format!("/nodup/x_{:04}", i)).unwrap();
        }

        let entries = vol.list_dir("/nodup").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name_str()).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "duplicate entries in readdir");
    }
}
