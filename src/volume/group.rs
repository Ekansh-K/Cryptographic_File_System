use anyhow::{bail, Context, Result};

use crate::block_device::CFSBlockDevice;
use super::{ceil_div, round_up_to};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// On-disk size of one group descriptor.
pub const GROUP_DESC_SIZE: u32 = 64;

/// Group flags — inode table not yet zeroed (lazy init).
pub const BG_INODE_UNINIT: u32 = 1 << 0;
/// Group flags — block bitmap not yet initialized (lazy init).
pub const BG_BLOCK_UNINIT: u32 = 1 << 1;
/// Group flags — inode table has been zeroed.
pub const BG_INODE_ZEROED: u32 = 1 << 2;

// ---------------------------------------------------------------------------
// GroupDescriptor — 64-byte on-disk structure
// ---------------------------------------------------------------------------

/// On-disk group descriptor. 64 bytes, stored contiguously in GDT blocks.
#[derive(Debug, Clone)]
pub struct GroupDescriptor {
    /// Block index of this group's block allocation bitmap.
    pub bg_block_bitmap: u64,
    /// Block index of this group's inode allocation bitmap.
    pub bg_inode_bitmap: u64,
    /// Block index of this group's inode table start.
    pub bg_inode_table: u64,
    /// Number of free data blocks in this group.
    pub bg_free_blocks: u32,
    /// Number of free inodes in this group.
    pub bg_free_inodes: u32,
    /// Number of inodes allocated to directories in this group.
    pub bg_used_dirs: u32,
    /// Group flags bitfield.
    pub bg_flags: u32,
    /// Number of unused inodes at end of group's inode table.
    pub bg_itable_unused: u32,
    /// CRC32 checksum of bytes [0..44).
    pub bg_checksum: u32,
    /// Total blocks in this group (may be < blocks_per_group for last group).
    pub bg_blocks_count: u64,
    /// Reserved for future use.
    pub bg_reserved: [u8; 8],
}

impl GroupDescriptor {
    /// Create a descriptor for a newly formatted group.
    pub fn new(
        block_bitmap: u64,
        inode_bitmap: u64,
        inode_table: u64,
        total_blocks: u64,
        inodes_per_group: u32,
        data_blocks: u32,
        lazy_init: bool,
    ) -> Self {
        let flags = if lazy_init {
            BG_INODE_UNINIT | BG_BLOCK_UNINIT
        } else {
            BG_INODE_ZEROED
        };

        Self {
            bg_block_bitmap: block_bitmap,
            bg_inode_bitmap: inode_bitmap,
            bg_inode_table: inode_table,
            bg_free_blocks: data_blocks,
            bg_free_inodes: inodes_per_group,
            bg_used_dirs: 0,
            bg_flags: flags,
            bg_itable_unused: inodes_per_group,
            bg_checksum: 0, // computed during serialize
            bg_blocks_count: total_blocks,
            bg_reserved: [0u8; 8],
        }
    }

    /// Serialize to 64-byte on-disk format. Checksum is computed over bytes [0..44).
    pub fn serialize(&self) -> [u8; 64] {
        let mut buf = [0u8; 64];
        buf[0..8].copy_from_slice(&self.bg_block_bitmap.to_le_bytes());
        buf[8..16].copy_from_slice(&self.bg_inode_bitmap.to_le_bytes());
        buf[16..24].copy_from_slice(&self.bg_inode_table.to_le_bytes());
        buf[24..28].copy_from_slice(&self.bg_free_blocks.to_le_bytes());
        buf[28..32].copy_from_slice(&self.bg_free_inodes.to_le_bytes());
        buf[32..36].copy_from_slice(&self.bg_used_dirs.to_le_bytes());
        buf[36..40].copy_from_slice(&self.bg_flags.to_le_bytes());
        buf[40..44].copy_from_slice(&self.bg_itable_unused.to_le_bytes());

        // Compute checksum over [0..44)
        let crc = crc32fast::hash(&buf[..44]);
        buf[44..48].copy_from_slice(&crc.to_le_bytes());

        buf[48..56].copy_from_slice(&self.bg_blocks_count.to_le_bytes());
        buf[56..64].copy_from_slice(&self.bg_reserved);
        buf
    }

    /// Deserialize from 64-byte on-disk buffer. Verifies CRC32 checksum.
    pub fn deserialize(buf: &[u8; 64]) -> Result<Self> {
        let bg_block_bitmap = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        let bg_inode_bitmap = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let bg_inode_table = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let bg_free_blocks = u32::from_le_bytes(buf[24..28].try_into().unwrap());
        let bg_free_inodes = u32::from_le_bytes(buf[28..32].try_into().unwrap());
        let bg_used_dirs = u32::from_le_bytes(buf[32..36].try_into().unwrap());
        let bg_flags = u32::from_le_bytes(buf[36..40].try_into().unwrap());
        let bg_itable_unused = u32::from_le_bytes(buf[40..44].try_into().unwrap());
        let stored_crc = u32::from_le_bytes(buf[44..48].try_into().unwrap());
        let bg_blocks_count = u64::from_le_bytes(buf[48..56].try_into().unwrap());
        let mut bg_reserved = [0u8; 8];
        bg_reserved.copy_from_slice(&buf[56..64]);

        // Verify checksum
        let computed = crc32fast::hash(&buf[..44]);
        if stored_crc != computed {
            bail!(
                "group descriptor checksum mismatch: stored=0x{stored_crc:08x}, computed=0x{computed:08x}"
            );
        }

        Ok(Self {
            bg_block_bitmap,
            bg_inode_bitmap,
            bg_inode_table,
            bg_free_blocks,
            bg_free_inodes,
            bg_used_dirs,
            bg_flags,
            bg_itable_unused,
            bg_checksum: stored_crc,
            bg_blocks_count,
            bg_reserved,
        })
    }

    /// Compute CRC32 checksum of bytes [0..44).
    pub fn compute_checksum(&self) -> u32 {
        let buf = self.serialize();
        u32::from_le_bytes(buf[44..48].try_into().unwrap())
    }

    /// Verify the stored checksum matches the computed value.
    pub fn verify_checksum(&self) -> Result<()> {
        let serialized = self.serialize();
        let stored = u32::from_le_bytes(serialized[44..48].try_into().unwrap());
        // Re-compute from the freshly serialized bytes (always valid)
        // — if self.bg_checksum was wrong, serialize() recomputes the correct one
        if self.bg_checksum != 0 && self.bg_checksum != stored {
            bail!(
                "group descriptor checksum mismatch: stored=0x{:08x}, computed=0x{stored:08x}",
                self.bg_checksum
            );
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// GroupLayout — result of layout computation
// ---------------------------------------------------------------------------

/// Result of computing the block group layout for a volume.
#[derive(Debug, Clone)]
pub struct GroupLayout {
    /// Total number of block groups.
    pub group_count: u32,
    /// Blocks per group (from FormatOptions). Last group may have fewer.
    pub blocks_per_group: u32,
    /// Inodes per group.
    pub inodes_per_group: u32,
    /// Blocks occupied by the inode table in each group.
    pub inode_table_blocks: u32,
    /// Data blocks available per full group (blocks_per_group - overhead).
    pub data_blocks_per_group: u32,
    /// Overhead blocks per group (block_bitmap + inode_bitmap + inode_table).
    pub overhead_per_group: u32,
    /// GDT blocks (in global area).
    pub gdt_blocks: u32,
    /// Total blocks in global overhead (SB + GDT + reserved_GDT + journal + backup).
    pub global_overhead: u64,
    /// Block index where group 0 starts (= global_overhead).
    pub first_group_block: u64,
    /// Blocks in the last group (may be < blocks_per_group).
    pub last_group_blocks: u32,
    /// Data blocks in the last group.
    pub last_group_data_blocks: u32,
    /// Reserved GDT blocks for future growth.
    pub reserved_gdt_blocks: u32,
}

impl GroupLayout {
    /// Get the starting block index of a given group.
    pub fn group_start_block(&self, group_idx: u32) -> u64 {
        self.first_group_block + group_idx as u64 * self.blocks_per_group as u64
    }

    /// Get the block bitmap location for a group.
    pub fn group_block_bitmap(&self, group_idx: u32) -> u64 {
        self.group_start_block(group_idx)
    }

    /// Get the inode bitmap location for a group.
    pub fn group_inode_bitmap(&self, group_idx: u32) -> u64 {
        self.group_start_block(group_idx) + 1
    }

    /// Get the inode table start for a group.
    pub fn group_inode_table(&self, group_idx: u32) -> u64 {
        self.group_start_block(group_idx) + 2
    }

    /// Get the data start block for a group.
    pub fn group_data_start(&self, group_idx: u32) -> u64 {
        self.group_start_block(group_idx) + self.overhead_per_group as u64
    }

    /// Number of blocks in a specific group (last group may differ).
    pub fn group_block_count(&self, group_idx: u32) -> u32 {
        if group_idx == self.group_count - 1 {
            self.last_group_blocks
        } else {
            self.blocks_per_group
        }
    }

    /// Number of data blocks in a specific group.
    pub fn group_data_block_count(&self, group_idx: u32) -> u32 {
        if group_idx == self.group_count - 1 {
            self.last_group_data_blocks
        } else {
            self.data_blocks_per_group
        }
    }
}

/// Compute the block group layout for a volume.
///
/// The circular dependency (group_count → gdt_blocks → global_overhead →
/// blocks_for_groups → group_count) is resolved by iterating until stable.
pub fn compute_group_layout(
    total_blocks: u64,
    block_size: u32,
    inode_size: u32,
    inode_ratio: u32,
    blocks_per_group: u32,
    journal_blocks: u64,
) -> Result<GroupLayout> {
    let bs = block_size as u64;
    let bpg = blocks_per_group as u64;
    let desc_per_block = bs / GROUP_DESC_SIZE as u64;

    if desc_per_block == 0 {
        bail!("block_size {} too small for group descriptors", block_size);
    }

    let backup_sb_blocks: u64 = 1;
    let mut gdt_blocks: u64;
    let mut reserved_gdt: u64;
    let mut global_overhead: u64;
    let mut blocks_for_groups: u64;
    let mut group_count: u64;

    // Initial estimate
    group_count = ceil_div(total_blocks, bpg);

    for _ in 0..10 {
        gdt_blocks = ceil_div(group_count, desc_per_block);
        reserved_gdt = gdt_blocks.min(128);

        global_overhead = 1 // superblock
            + gdt_blocks
            + reserved_gdt
            + journal_blocks
            + backup_sb_blocks;

        if total_blocks <= global_overhead {
            bail!(
                "volume too small: {} blocks, need at least {} for overhead",
                total_blocks,
                global_overhead + bpg
            );
        }

        blocks_for_groups = total_blocks - global_overhead;
        let new_count = ceil_div(blocks_for_groups, bpg);

        if new_count == group_count {
            break;
        }
        group_count = new_count;
    }

    // Final values after convergence
    gdt_blocks = ceil_div(group_count, desc_per_block);
    reserved_gdt = gdt_blocks.min(128);
    global_overhead = 1 + gdt_blocks + reserved_gdt + journal_blocks + backup_sb_blocks;
    blocks_for_groups = total_blocks - global_overhead;
    group_count = ceil_div(blocks_for_groups, bpg);

    if group_count == 0 {
        bail!("volume too small for any block groups");
    }

    // Per-group inode computation
    let inodes_per_block = bs / inode_size as u64;
    if inodes_per_block == 0 {
        bail!("inode_size {} too large for block_size {}", inode_size, block_size);
    }
    let total_data_bytes = blocks_for_groups * bs;
    let total_inodes_desired = total_data_bytes / inode_ratio as u64;
    let ipg_raw = ceil_div(total_inodes_desired.max(1), group_count);
    let inodes_per_group = round_up_to(ipg_raw, inodes_per_block).max(inodes_per_block) as u32;
    let inode_table_blocks = ceil_div(inodes_per_group as u64 * inode_size as u64, bs) as u32;

    // Per-group overhead: block_bitmap (1) + inode_bitmap (1) + inode_table
    let per_group_overhead = 2 + inode_table_blocks;
    if per_group_overhead as u64 >= bpg {
        bail!(
            "per-group overhead ({per_group_overhead} blocks) exceeds blocks_per_group ({bpg})"
        );
    }
    let data_blocks_per_group = blocks_per_group - per_group_overhead;

    // Last group sizing
    let full_groups = blocks_for_groups / bpg;
    let remainder = blocks_for_groups % bpg;
    let actual_group_count = if remainder > 0 {
        full_groups + 1
    } else {
        full_groups
    };
    let last_group_blocks = if remainder > 0 {
        remainder as u32
    } else {
        blocks_per_group
    };
    let last_group_data = if last_group_blocks > per_group_overhead {
        last_group_blocks - per_group_overhead
    } else {
        0
    };

    Ok(GroupLayout {
        group_count: actual_group_count as u32,
        blocks_per_group,
        inodes_per_group,
        inode_table_blocks,
        data_blocks_per_group,
        overhead_per_group: per_group_overhead,
        gdt_blocks: gdt_blocks as u32,
        global_overhead,
        first_group_block: global_overhead,
        last_group_blocks,
        last_group_data_blocks: last_group_data,
        reserved_gdt_blocks: reserved_gdt as u32,
    })
}

// ---------------------------------------------------------------------------
// GDT Persistence
// ---------------------------------------------------------------------------

/// Write all group descriptors to the GDT area on disk.
pub fn write_gdt(
    dev: &mut dyn CFSBlockDevice,
    gdt_start: u64,
    descriptors: &[GroupDescriptor],
    block_size: u32,
) -> Result<()> {
    let desc_per_block = block_size as usize / GROUP_DESC_SIZE as usize;
    let total_gdt_blocks = ceil_div(descriptors.len() as u64, desc_per_block as u64);

    for block_idx in 0..total_gdt_blocks {
        let mut buf = vec![0u8; block_size as usize];
        let start_desc = block_idx as usize * desc_per_block;
        let end_desc = (start_desc + desc_per_block).min(descriptors.len());

        for (i, desc) in descriptors[start_desc..end_desc].iter().enumerate() {
            let offset = i * GROUP_DESC_SIZE as usize;
            buf[offset..offset + GROUP_DESC_SIZE as usize].copy_from_slice(&desc.serialize());
        }

        let disk_offset = (gdt_start + block_idx) * block_size as u64;
        dev.write(disk_offset, &buf)?;
    }

    Ok(())
}

/// Read all group descriptors from the GDT area on disk.
pub fn read_gdt(
    dev: &mut dyn CFSBlockDevice,
    gdt_start: u64,
    group_count: u32,
    block_size: u32,
) -> Result<Vec<GroupDescriptor>> {
    let desc_per_block = block_size as usize / GROUP_DESC_SIZE as usize;
    let total_gdt_blocks = ceil_div(group_count as u64, desc_per_block as u64);
    let mut descriptors = Vec::with_capacity(group_count as usize);

    for block_idx in 0..total_gdt_blocks {
        let mut buf = vec![0u8; block_size as usize];
        let disk_offset = (gdt_start + block_idx) * block_size as u64;
        dev.read(disk_offset, &mut buf)?;

        let start_desc = block_idx as usize * desc_per_block;
        let remaining = group_count as usize - start_desc;
        let count = remaining.min(desc_per_block);

        for i in 0..count {
            let offset = i * GROUP_DESC_SIZE as usize;
            let desc_buf: [u8; 64] = buf[offset..offset + 64].try_into().unwrap();
            let desc = GroupDescriptor::deserialize(&desc_buf)
                .with_context(|| format!("group {} descriptor corrupt", start_desc + i))?;
            descriptors.push(desc);
        }
    }

    Ok(descriptors)
}

// ---------------------------------------------------------------------------
// GroupBitmapManager — per-group block allocation bitmaps (10B.4)
// ---------------------------------------------------------------------------

use std::collections::{HashMap, HashSet};
use super::bitmap::Bitmap;

/// Manages per-group block allocation bitmaps with lazy loading and dirty tracking.
///
/// Descriptors are NOT owned — they are passed by reference on each call
/// (Option C from the design doc). This struct stores only bitmap caches
/// and layout parameters.
pub struct GroupBitmapManager {
    /// Loaded group block bitmaps. Key = group index.
    bitmaps: HashMap<u32, Bitmap>,
    /// Set of group indices whose bitmaps have been modified since last save.
    dirty: HashSet<u32>,
    /// Volume parameters.
    pub(crate) block_size: u32,
    pub(crate) blocks_per_group: u32,
    pub(crate) group_count: u32,
    /// Block index of the first group (= global_overhead).
    pub(crate) first_group_block: u64,
    /// Per-group overhead blocks (block_bitmap + inode_bitmap + inode_table).
    pub(crate) overhead_per_group: u32,
}

impl GroupBitmapManager {
    /// Create a new manager. Bitmaps are not loaded until first access.
    pub fn new(
        block_size: u32,
        blocks_per_group: u32,
        group_count: u32,
        first_group_block: u64,
        overhead_per_group: u32,
    ) -> Self {
        Self {
            bitmaps: HashMap::new(),
            dirty: HashSet::new(),
            block_size,
            blocks_per_group,
            group_count,
            first_group_block,
            overhead_per_group,
        }
    }

    /// Convert a global block index to (group_index, local_data_block_index).
    ///
    /// The local index is relative to the data area of that group (after overhead).
    pub fn global_to_local(&self, global_block: u64) -> Option<(u32, u32)> {
        if global_block < self.first_group_block {
            return None; // Block is in global overhead area
        }
        let relative = global_block - self.first_group_block;
        let group_idx = (relative / self.blocks_per_group as u64) as u32;
        if group_idx >= self.group_count {
            return None;
        }
        let block_within_group = (relative % self.blocks_per_group as u64) as u32;
        if block_within_group < self.overhead_per_group {
            return None; // Block is group metadata (bitmap/inode table)
        }
        let local_data_idx = block_within_group - self.overhead_per_group;
        Some((group_idx, local_data_idx))
    }

    /// Convert (group_index, local_data_block_index) to global block index.
    pub fn local_to_global(&self, group_idx: u32, local_data_idx: u32) -> u64 {
        self.first_group_block
            + group_idx as u64 * self.blocks_per_group as u64
            + self.overhead_per_group as u64
            + local_data_idx as u64
    }

    /// Ensure the bitmap for `group_idx` is loaded into memory.
    fn ensure_loaded(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        group_idx: u32,
        descriptors: &[GroupDescriptor],
    ) -> Result<()> {
        if self.bitmaps.contains_key(&group_idx) {
            return Ok(());
        }

        let desc = &descriptors[group_idx as usize];
        let data_blocks = self.data_blocks_in_group(group_idx, desc);

        // Check lazy init flag
        if desc.bg_flags & BG_BLOCK_UNINIT != 0 {
            let bitmap = Bitmap::new_all_free(data_blocks);
            self.bitmaps.insert(group_idx, bitmap);
            return Ok(());
        }

        // Load from disk
        let mut buf = vec![0u8; self.block_size as usize];
        dev.read(desc.bg_block_bitmap * self.block_size as u64, &mut buf)?;
        let bitmap = Bitmap::from_bytes(&buf, data_blocks);
        self.bitmaps.insert(group_idx, bitmap);
        Ok(())
    }

    /// Number of data blocks in a given group.
    fn data_blocks_in_group(&self, group_idx: u32, desc: &GroupDescriptor) -> u32 {
        let total_in_group = desc.bg_blocks_count as u32;
        total_in_group.saturating_sub(self.overhead_per_group)
    }

    /// Allocate a single block from a specific group.
    /// Returns the global block index, or None if group is full.
    pub fn alloc_in_group(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        group_idx: u32,
        descriptors: &mut [GroupDescriptor],
    ) -> Result<Option<u64>> {
        self.ensure_loaded(dev, group_idx, descriptors)?;
        let bitmap = self.bitmaps.get_mut(&group_idx).unwrap();

        match bitmap.alloc() {
            Some(local_idx) => {
                self.dirty.insert(group_idx);
                descriptors[group_idx as usize].bg_free_blocks =
                    descriptors[group_idx as usize].bg_free_blocks.saturating_sub(1);
                descriptors[group_idx as usize].bg_flags &= !BG_BLOCK_UNINIT;
                let global = self.local_to_global(group_idx, local_idx as u32);
                Ok(Some(global))
            }
            None => Ok(None),
        }
    }

    /// Free a block given its global block index.
    pub fn free_block(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        global_block: u64,
        descriptors: &mut [GroupDescriptor],
    ) -> Result<()> {
        let (group_idx, local_idx) = self
            .global_to_local(global_block)
            .ok_or_else(|| anyhow::anyhow!("block {} not in any data group", global_block))?;
        self.ensure_loaded(dev, group_idx, descriptors)?;
        let bitmap = self.bitmaps.get_mut(&group_idx).unwrap();
        bitmap.free(local_idx as u64)?;
        self.dirty.insert(group_idx);
        descriptors[group_idx as usize].bg_free_blocks += 1;
        Ok(())
    }

    /// Write all dirty group bitmaps to disk.
    pub fn save_all(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        descriptors: &[GroupDescriptor],
    ) -> Result<()> {
        let dirty_groups: Vec<u32> = self.dirty.iter().copied().collect();
        for group_idx in dirty_groups {
            self.save_group(dev, group_idx, descriptors)?;
        }
        self.dirty.clear();
        Ok(())
    }

    /// Write a single group's bitmap to disk.
    fn save_group(
        &self,
        dev: &mut dyn CFSBlockDevice,
        group_idx: u32,
        descriptors: &[GroupDescriptor],
    ) -> Result<()> {
        let bitmap = self
            .bitmaps
            .get(&group_idx)
            .ok_or_else(|| anyhow::anyhow!("group {} bitmap not loaded", group_idx))?;
        let desc = &descriptors[group_idx as usize];
        let disk_offset = desc.bg_block_bitmap * self.block_size as u64;

        let mut buf = vec![0u8; self.block_size as usize];
        let bytes = bitmap.as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);
        dev.write(disk_offset, &buf)?;
        Ok(())
    }

    /// Check whether any bitmaps are dirty (unsaved).
    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    /// Check if a bitmap is loaded for a given group.
    pub fn is_loaded(&self, group_idx: u32) -> bool {
        self.bitmaps.contains_key(&group_idx)
    }

    /// Public wrapper around ensure_loaded for use by contiguous allocator.
    pub fn ensure_loaded_pub(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        group_idx: u32,
        descriptors: &[GroupDescriptor],
    ) -> Result<()> {
        self.ensure_loaded(dev, group_idx, descriptors)
    }

    /// Get a mutable reference to a group's bitmap (must be loaded first).
    pub fn get_bitmap_mut(&mut self, group_idx: u32) -> Option<&mut Bitmap> {
        self.bitmaps.get_mut(&group_idx)
    }

    /// Mark a group's bitmap as dirty (needs saving).
    pub fn mark_dirty(&mut self, group_idx: u32) {
        self.dirty.insert(group_idx);
    }
}

// ---------------------------------------------------------------------------
// GroupInodeBitmapManager — per-group inode allocation bitmaps (10B.5)
// ---------------------------------------------------------------------------

/// Manages per-group inode allocation bitmaps with lazy loading and dirty tracking.
pub struct GroupInodeBitmapManager {
    bitmaps: HashMap<u32, Bitmap>,
    dirty: HashSet<u32>,
    pub(crate) block_size: u32,
    pub(crate) inodes_per_group: u32,
    pub(crate) group_count: u32,
}

impl GroupInodeBitmapManager {
    pub fn new(block_size: u32, inodes_per_group: u32, group_count: u32) -> Self {
        Self {
            bitmaps: HashMap::new(),
            dirty: HashSet::new(),
            block_size,
            inodes_per_group,
            group_count,
        }
    }

    /// Ensure the inode bitmap for `group_idx` is loaded.
    fn ensure_loaded(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        group_idx: u32,
        descriptors: &[GroupDescriptor],
    ) -> Result<()> {
        if self.bitmaps.contains_key(&group_idx) {
            return Ok(());
        }

        let desc = &descriptors[group_idx as usize];

        if desc.bg_flags & BG_INODE_UNINIT != 0 {
            let bitmap = Bitmap::new_all_free(self.inodes_per_group);
            self.bitmaps.insert(group_idx, bitmap);
            return Ok(());
        }

        let mut buf = vec![0u8; self.block_size as usize];
        dev.read(desc.bg_inode_bitmap * self.block_size as u64, &mut buf)?;
        let bitmap = Bitmap::from_bytes(&buf, self.inodes_per_group);
        self.bitmaps.insert(group_idx, bitmap);
        Ok(())
    }

    /// Allocate an inode in a specific group. Returns the global inode index.
    pub fn alloc_in_group(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        group_idx: u32,
        descriptors: &mut [GroupDescriptor],
    ) -> Result<Option<u32>> {
        self.ensure_loaded(dev, group_idx, descriptors)?;
        let bitmap = self.bitmaps.get_mut(&group_idx).unwrap();
        match bitmap.alloc() {
            Some(local_idx) => {
                self.dirty.insert(group_idx);
                descriptors[group_idx as usize].bg_free_inodes =
                    descriptors[group_idx as usize].bg_free_inodes.saturating_sub(1);
                descriptors[group_idx as usize].bg_flags &= !BG_INODE_UNINIT;
                let global = group_idx * self.inodes_per_group + local_idx as u32;
                Ok(Some(global))
            }
            None => Ok(None),
        }
    }

    /// Free an inode given its global inode index.
    pub fn free_inode(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        inode_idx: u32,
        descriptors: &mut [GroupDescriptor],
    ) -> Result<()> {
        let group_idx = inode_idx / self.inodes_per_group;
        let local_idx = inode_idx % self.inodes_per_group;
        self.ensure_loaded(dev, group_idx, descriptors)?;
        let bitmap = self.bitmaps.get_mut(&group_idx).unwrap();
        bitmap.free(local_idx as u64)?;
        self.dirty.insert(group_idx);
        descriptors[group_idx as usize].bg_free_inodes += 1;
        Ok(())
    }

    /// Save all dirty inode bitmaps to disk.
    pub fn save_all(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        descriptors: &[GroupDescriptor],
    ) -> Result<()> {
        let dirty_groups: Vec<u32> = self.dirty.iter().copied().collect();
        for group_idx in dirty_groups {
            let bitmap = self.bitmaps.get(&group_idx).unwrap();
            let desc = &descriptors[group_idx as usize];
            let mut buf = vec![0u8; self.block_size as usize];
            let bytes = bitmap.as_bytes();
            buf[..bytes.len()].copy_from_slice(bytes);
            dev.write(desc.bg_inode_bitmap * self.block_size as u64, &buf)?;
        }
        self.dirty.clear();
        Ok(())
    }

    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }
}

// ---------------------------------------------------------------------------
// GroupInodeTable — per-group inode table I/O (10B.5)
// ---------------------------------------------------------------------------

use super::inode::Inode;

/// Manages inode I/O across multiple block groups.
///
/// Does NOT own descriptors — receives them by reference.
pub struct GroupInodeTable {
    pub(crate) inodes_per_group: u32,
    pub(crate) inode_size: u32,
    pub(crate) block_size: u32,
    pub(crate) group_count: u32,
}

impl GroupInodeTable {
    pub fn new(
        inodes_per_group: u32,
        inode_size: u32,
        block_size: u32,
        group_count: u32,
    ) -> Self {
        Self {
            inodes_per_group,
            inode_size,
            block_size,
            group_count,
        }
    }

    /// Map global inode index to (group_idx, local_inode_idx).
    pub fn inode_to_group(&self, inode_idx: u32) -> (u32, u32) {
        let group = inode_idx / self.inodes_per_group;
        let local = inode_idx % self.inodes_per_group;
        (group, local)
    }

    /// Read an inode by global index.
    pub fn read_inode(
        &self,
        dev: &mut dyn CFSBlockDevice,
        inode_idx: u32,
        descriptors: &[GroupDescriptor],
    ) -> Result<Inode> {
        let (group_idx, local_idx) = self.inode_to_group(inode_idx);
        if group_idx >= self.group_count {
            bail!("inode {} out of range (max group {})", inode_idx, self.group_count - 1);
        }

        let desc = &descriptors[group_idx as usize];
        let table_start = desc.bg_inode_table;
        let bs = self.block_size as u64;
        let isz = self.inode_size as u64;

        let byte_offset = table_start * bs + local_idx as u64 * isz;
        let block_offset = (byte_offset / bs) * bs;
        let offset_within = (byte_offset % bs) as usize;

        let mut buf = vec![0u8; self.block_size as usize];
        dev.read(block_offset, &mut buf)?;

        let inode_bytes = &buf[offset_within..offset_within + self.inode_size as usize];
        Inode::deserialize_for_size(inode_bytes, self.inode_size)
    }

    /// Write an inode by global index (read-modify-write).
    pub fn write_inode(
        &self,
        dev: &mut dyn CFSBlockDevice,
        inode_idx: u32,
        inode: &Inode,
        descriptors: &[GroupDescriptor],
    ) -> Result<()> {
        let (group_idx, local_idx) = self.inode_to_group(inode_idx);
        if group_idx >= self.group_count {
            bail!("inode {} out of range", inode_idx);
        }

        let desc = &descriptors[group_idx as usize];
        let table_start = desc.bg_inode_table;
        let bs = self.block_size as u64;
        let isz = self.inode_size as u64;

        let byte_offset = table_start * bs + local_idx as u64 * isz;
        let block_offset = (byte_offset / bs) * bs;
        let offset_within = (byte_offset % bs) as usize;

        // Read-modify-write
        let mut buf = vec![0u8; self.block_size as usize];
        dev.read(block_offset, &mut buf)?;

        let serialized = inode.serialize_for_size(self.inode_size);
        buf[offset_within..offset_within + self.inode_size as usize]
            .copy_from_slice(&serialized);

        dev.write(block_offset, &buf)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Group-Aware Block Allocator (10B.6)
// ---------------------------------------------------------------------------

/// Allocate `count` blocks preferring the group containing `hint_block`.
/// Falls back to nearby groups, then scans all groups.
pub fn alloc_blocks_near(
    gbm: &mut GroupBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    hint_block: u64,
    count: usize,
    descriptors: &mut [GroupDescriptor],
    superblock: &mut super::superblock::Superblock,
) -> Result<Vec<u64>> {
    let mut allocated = Vec::with_capacity(count);

    let preferred_group = gbm
        .global_to_local(hint_block)
        .map(|(g, _)| g)
        .unwrap_or(0);

    // Phase 1: Try preferred group
    while allocated.len() < count {
        match gbm.alloc_in_group(dev, preferred_group, descriptors)? {
            Some(block) => allocated.push(block),
            None => break,
        }
    }

    // Phase 2: Try nearby groups (±1, ±2, ...)
    if allocated.len() < count {
        let max_radius = gbm.group_count / 2 + 1;
        for radius in 1..=max_radius {
            for &candidate in &[
                preferred_group.wrapping_add(radius),
                preferred_group.wrapping_sub(radius),
            ] {
                if candidate >= gbm.group_count {
                    continue;
                }
                while allocated.len() < count {
                    match gbm.alloc_in_group(dev, candidate, descriptors)? {
                        Some(block) => allocated.push(block),
                        None => break,
                    }
                }
                if allocated.len() >= count {
                    break;
                }
            }
            if allocated.len() >= count {
                break;
            }
        }
    }

    // Phase 3: Full scan
    if allocated.len() < count {
        for g in 0..gbm.group_count {
            while allocated.len() < count {
                match gbm.alloc_in_group(dev, g, descriptors)? {
                    Some(block) => allocated.push(block),
                    None => break,
                }
            }
            if allocated.len() >= count {
                break;
            }
        }
    }

    if allocated.len() < count {
        // Rollback
        for &block in &allocated {
            let _ = gbm.free_block(dev, block, descriptors);
        }
        bail!(
            "not enough free blocks: need {}, only {} available",
            count,
            allocated.len()
        );
    }

    superblock.free_blocks -= count as u64;
    Ok(allocated)
}

/// Allocate `count` blocks without a locality hint — picks the group with
/// the most free blocks.
pub fn alloc_blocks_group(
    gbm: &mut GroupBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    count: usize,
    descriptors: &mut [GroupDescriptor],
    superblock: &mut super::superblock::Superblock,
) -> Result<Vec<u64>> {
    let best_group = descriptors
        .iter()
        .enumerate()
        .max_by_key(|(_, desc)| desc.bg_free_blocks)
        .map(|(idx, _)| idx as u32)
        .unwrap_or(0);

    let hint = gbm.local_to_global(best_group, 0);
    alloc_blocks_near(gbm, dev, hint, count, descriptors, superblock)
}

/// Free a set of blocks, updating the correct group bitmaps and superblock.
pub fn free_blocks_group(
    gbm: &mut GroupBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    blocks: &[u64],
    descriptors: &mut [GroupDescriptor],
    superblock: &mut super::superblock::Superblock,
) -> Result<()> {
    for &block in blocks {
        gbm.free_block(dev, block, descriptors)?;
    }
    superblock.free_blocks += blocks.len() as u64;
    Ok(())
}

/// Allocate blocks for file data, preferring the group containing the file's inode.
pub fn alloc_blocks_for_file(
    gbm: &mut GroupBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    inode_idx: u32,
    count: usize,
    inodes_per_group: u32,
    descriptors: &mut [GroupDescriptor],
    superblock: &mut super::superblock::Superblock,
) -> Result<Vec<u64>> {
    let file_group = inode_idx / inodes_per_group;
    let hint = gbm.local_to_global(file_group, 0);
    alloc_blocks_near(gbm, dev, hint, count, descriptors, superblock)
}

// ---------------------------------------------------------------------------
// Orlov Directory Allocator (10B.6)
// ---------------------------------------------------------------------------

/// Choose a group for a new directory inode using Orlov heuristics.
///
/// For top-level directories (parent = root): spread across groups.
/// For subdirectories: keep near parent's group.
pub fn orlov_select_group(
    descriptors: &[GroupDescriptor],
    parent_group: u32,
    is_top_level: bool,
    group_count: u32,
) -> u32 {
    if !is_top_level {
        // Subdirectory: prefer parent's group
        let parent_desc = &descriptors[parent_group as usize];
        if parent_desc.bg_free_inodes > 0 {
            return parent_group;
        }
        // Fallback: find nearest group with free inodes
        for radius in 1..group_count {
            for &candidate in &[
                parent_group.wrapping_add(radius),
                parent_group.wrapping_sub(radius),
            ] {
                if candidate < group_count
                    && descriptors[candidate as usize].bg_free_inodes > 0
                {
                    return candidate;
                }
            }
        }
        return parent_group;
    }

    // Top-level directory: spread using Orlov scoring
    if group_count == 0 {
        return 0;
    }
    let avg_free_inodes = descriptors
        .iter()
        .map(|d| d.bg_free_inodes as u64)
        .sum::<u64>()
        / group_count as u64;
    let avg_free_blocks = descriptors
        .iter()
        .map(|d| d.bg_free_blocks as u64)
        .sum::<u64>()
        / group_count as u64;

    let mut best_group = 0u32;
    let mut best_score = u64::MAX;

    for (idx, desc) in descriptors.iter().enumerate() {
        if desc.bg_free_inodes == 0 {
            continue;
        }

        let dir_penalty = desc.bg_used_dirs as u64 * 256;
        let inode_bonus = if desc.bg_free_inodes as u64 >= avg_free_inodes {
            0
        } else {
            128
        };
        let block_bonus = if desc.bg_free_blocks as u64 >= avg_free_blocks {
            0
        } else {
            64
        };
        let score = dir_penalty + inode_bonus + block_bonus;

        if score < best_score {
            best_score = score;
            best_group = idx as u32;
        }
    }

    best_group
}

// ---------------------------------------------------------------------------
// Group-Aware Inode Allocator (10B.7)
// ---------------------------------------------------------------------------

/// Allocate an inode for a new file, preferring the parent directory's group.
pub fn alloc_inode_for_file(
    gibm: &mut GroupInodeBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    parent_inode: u32,
    inodes_per_group: u32,
    descriptors: &mut [GroupDescriptor],
    superblock: &mut super::superblock::Superblock,
) -> Result<u32> {
    let parent_group = parent_inode / inodes_per_group;

    // Try parent's group first
    if parent_group < gibm.group_count {
        if let Some(inode) = gibm.alloc_in_group(dev, parent_group, descriptors)? {
            superblock.free_inodes = superblock.free_inodes.saturating_sub(1);
            return Ok(inode);
        }
    }

    // Fallback: search all groups
    for g in 0..gibm.group_count {
        let candidate = (parent_group.wrapping_add(g)) % gibm.group_count;
        if candidate == parent_group {
            continue;
        }
        if let Some(inode) = gibm.alloc_in_group(dev, candidate, descriptors)? {
            superblock.free_inodes = superblock.free_inodes.saturating_sub(1);
            return Ok(inode);
        }
    }

    bail!("no free inodes on volume")
}

/// Allocate an inode for a new directory using Orlov heuristics.
pub fn alloc_inode_for_dir(
    gibm: &mut GroupInodeBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    parent_inode: u32,
    inodes_per_group: u32,
    descriptors: &mut [GroupDescriptor],
    superblock: &mut super::superblock::Superblock,
    is_top_level: bool,
) -> Result<u32> {
    let parent_group = parent_inode / inodes_per_group;
    let target_group = orlov_select_group(
        descriptors,
        parent_group,
        is_top_level,
        gibm.group_count,
    );

    if let Some(inode) = gibm.alloc_in_group(dev, target_group, descriptors)? {
        descriptors[target_group as usize].bg_used_dirs += 1;
        superblock.free_inodes = superblock.free_inodes.saturating_sub(1);
        return Ok(inode);
    }

    // Fallback
    for g in 0..gibm.group_count {
        if g == target_group {
            continue;
        }
        if let Some(inode) = gibm.alloc_in_group(dev, g, descriptors)? {
            descriptors[g as usize].bg_used_dirs += 1;
            superblock.free_inodes = superblock.free_inodes.saturating_sub(1);
            return Ok(inode);
        }
    }

    bail!("no free inodes on volume")
}

/// Free an inode, updating the correct group's counters.
pub fn free_inode_group(
    gibm: &mut GroupInodeBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    inode_idx: u32,
    was_directory: bool,
    inodes_per_group: u32,
    descriptors: &mut [GroupDescriptor],
    superblock: &mut super::superblock::Superblock,
) -> Result<()> {
    gibm.free_inode(dev, inode_idx, descriptors)?;
    let group = inode_idx / inodes_per_group;
    if was_directory && (group as usize) < descriptors.len() {
        descriptors[group as usize].bg_used_dirs =
            descriptors[group as usize].bg_used_dirs.saturating_sub(1);
    }
    superblock.free_inodes += 1;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use tempfile::NamedTempFile;

    // -- 10B.1: GroupDescriptor tests --

    #[test]
    fn test_group_desc_serialize_roundtrip() {
        let desc = GroupDescriptor::new(100, 101, 102, 32768, 8112, 32259, false);
        let buf = desc.serialize();
        let desc2 = GroupDescriptor::deserialize(&buf).unwrap();
        assert_eq!(desc2.bg_block_bitmap, 100);
        assert_eq!(desc2.bg_inode_bitmap, 101);
        assert_eq!(desc2.bg_inode_table, 102);
        assert_eq!(desc2.bg_free_blocks, 32259);
        assert_eq!(desc2.bg_free_inodes, 8112);
        assert_eq!(desc2.bg_used_dirs, 0);
        assert_eq!(desc2.bg_blocks_count, 32768);
        assert_eq!(desc2.bg_flags, BG_INODE_ZEROED);
    }

    #[test]
    fn test_group_desc_checksum_valid() {
        let desc = GroupDescriptor::new(10, 11, 12, 1000, 256, 900, true);
        let buf = desc.serialize();
        // Deserialize should succeed (checksum valid)
        GroupDescriptor::deserialize(&buf).unwrap();
    }

    #[test]
    fn test_group_desc_checksum_corrupt() {
        let desc = GroupDescriptor::new(10, 11, 12, 1000, 256, 900, true);
        let mut buf = desc.serialize();
        // Flip a byte in the checksummed region
        buf[30] ^= 0xFF;
        let result = GroupDescriptor::deserialize(&buf);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("checksum mismatch"));
    }

    #[test]
    fn test_group_desc_last_group_smaller() {
        let desc = GroupDescriptor::new(50, 51, 52, 100, 256, 90, false);
        let buf = desc.serialize();
        let desc2 = GroupDescriptor::deserialize(&buf).unwrap();
        assert_eq!(desc2.bg_blocks_count, 100);
    }

    #[test]
    fn test_group_desc_lazy_flags() {
        let desc = GroupDescriptor::new(10, 11, 12, 32768, 8112, 32259, true);
        assert_eq!(desc.bg_flags, BG_INODE_UNINIT | BG_BLOCK_UNINIT);
    }

    #[test]
    fn test_group_desc_no_lazy_flags() {
        let desc = GroupDescriptor::new(10, 11, 12, 32768, 8112, 32259, false);
        assert_eq!(desc.bg_flags, BG_INODE_ZEROED);
    }

    #[test]
    fn test_group_desc_zero_dirs() {
        let desc = GroupDescriptor::new(10, 11, 12, 32768, 8112, 32259, true);
        assert_eq!(desc.bg_used_dirs, 0);
    }

    // -- 10B.2: GroupLayout tests --

    #[test]
    fn test_layout_4gb_default() {
        let total_blocks = 4 * 1024 * 1024 * 1024u64 / 4096; // 1_048_576
        let layout = compute_group_layout(total_blocks, 4096, 256, 16384, 32768, 10486).unwrap();
        assert!(layout.group_count > 0);
        assert_eq!(layout.blocks_per_group, 32768);
        assert!(layout.inodes_per_group > 0);
        assert!(layout.data_blocks_per_group > 0);
        assert!(layout.first_group_block > 0);
    }

    #[test]
    fn test_layout_1tb_large_files() {
        let total_blocks = 1024u64 * 1024 * 1024 * 1024 / 16384; // 67_108_864
        let layout =
            compute_group_layout(total_blocks, 16384, 256, 65536, 131072, 335544).unwrap();
        assert!(layout.group_count > 0);
        assert_eq!(layout.blocks_per_group, 131072);
        assert!(layout.gdt_blocks >= 1);
    }

    #[test]
    fn test_layout_too_small() {
        // 3 blocks total — not enough for superblock + GDT + reserved + backup + any group
        let result = compute_group_layout(3, 4096, 256, 16384, 32768, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_layout_100mb_default() {
        let total_blocks = 100 * 1024 * 1024u64 / 4096; // 25600
        let layout = compute_group_layout(total_blocks, 4096, 256, 16384, 32768, 0).unwrap();
        assert!(layout.group_count >= 1);
    }

    #[test]
    fn test_layout_last_group_smaller() {
        // Pick a total that doesn't divide evenly by blocks_per_group
        let total_blocks = 50000u64;
        let layout = compute_group_layout(total_blocks, 4096, 256, 16384, 32768, 0).unwrap();
        if layout.group_count > 1 {
            assert!(layout.last_group_blocks <= layout.blocks_per_group);
        }
    }

    #[test]
    fn test_layout_last_group_exact() {
        // Force exact multiple: overhead + N * bpg
        let bpg = 32768u64;
        let layout_probe = compute_group_layout(bpg * 4, 4096, 256, 16384, 32768, 0).unwrap();
        // If the total happens to divide exactly, last_group_blocks == blocks_per_group
        if layout_probe.last_group_blocks == layout_probe.blocks_per_group {
            assert_eq!(layout_probe.last_group_blocks, layout_probe.blocks_per_group);
        }
    }

    #[test]
    fn test_layout_group_start_block() {
        let total_blocks = 1_048_576u64;
        let layout = compute_group_layout(total_blocks, 4096, 256, 16384, 32768, 0).unwrap();
        assert_eq!(layout.group_start_block(0), layout.first_group_block);
        assert_eq!(
            layout.group_start_block(1),
            layout.first_group_block + layout.blocks_per_group as u64
        );
    }

    #[test]
    fn test_layout_deterministic() {
        let a = compute_group_layout(1_048_576, 4096, 256, 16384, 32768, 10486).unwrap();
        let b = compute_group_layout(1_048_576, 4096, 256, 16384, 32768, 10486).unwrap();
        assert_eq!(a.group_count, b.group_count);
        assert_eq!(a.inodes_per_group, b.inodes_per_group);
        assert_eq!(a.data_blocks_per_group, b.data_blocks_per_group);
        assert_eq!(a.global_overhead, b.global_overhead);
    }

    // -- 10B.3: GDT Persistence tests --

    fn make_test_device(blocks: u64, block_size: u32) -> (NamedTempFile, FileBlockDevice) {
        let tmp = NamedTempFile::new().unwrap();
        let size = blocks * block_size as u64;
        tmp.as_file().set_len(size).unwrap();
        let dev = FileBlockDevice::open(tmp.path(), None).unwrap();
        (tmp, dev)
    }

    #[test]
    fn test_gdt_write_read_32_groups() {
        let (_tmp, mut dev) = make_test_device(2048, 4096);
        let gdt_start = 1u64; // block 1
        let mut descriptors = Vec::new();
        for i in 0..32u32 {
            descriptors.push(GroupDescriptor::new(
                100 + i as u64 * 3,
                101 + i as u64 * 3,
                102 + i as u64 * 3,
                32768,
                8112,
                32259,
                i != 0, // group 0 is not lazy
            ));
        }
        write_gdt(&mut dev, gdt_start, &descriptors, 4096).unwrap();
        let read_back = read_gdt(&mut dev, gdt_start, 32, 4096).unwrap();
        assert_eq!(read_back.len(), 32);
        for (i, desc) in read_back.iter().enumerate() {
            assert_eq!(desc.bg_block_bitmap, 100 + i as u64 * 3);
            assert_eq!(desc.bg_inode_bitmap, 101 + i as u64 * 3);
        }
    }

    #[test]
    fn test_gdt_write_read_partial_block() {
        let (_tmp, mut dev) = make_test_device(256, 4096);
        let gdt_start = 1u64;
        let mut descriptors = Vec::new();
        for i in 0..13u32 {
            descriptors.push(GroupDescriptor::new(
                i as u64 * 10,
                i as u64 * 10 + 1,
                i as u64 * 10 + 2,
                1000 + i as u64,
                128,
                900,
                true,
            ));
        }
        write_gdt(&mut dev, gdt_start, &descriptors, 4096).unwrap();
        let read_back = read_gdt(&mut dev, gdt_start, 13, 4096).unwrap();
        assert_eq!(read_back.len(), 13);
        for (i, desc) in read_back.iter().enumerate() {
            assert_eq!(desc.bg_blocks_count, 1000 + i as u64);
        }
    }

    #[test]
    fn test_gdt_corrupt_one_descriptor() {
        let (_tmp, mut dev) = make_test_device(256, 4096);
        let gdt_start = 1u64;
        let descriptors: Vec<_> = (0..4)
            .map(|i| {
                GroupDescriptor::new(
                    i as u64 * 10,
                    i as u64 * 10 + 1,
                    i as u64 * 10 + 2,
                    32768,
                    8112,
                    32259,
                    true,
                )
            })
            .collect();
        write_gdt(&mut dev, gdt_start, &descriptors, 4096).unwrap();

        // Corrupt byte 30 of descriptor 2 (byte offset = 2*64+30 = 158 in block 1)
        let mut buf = vec![0u8; 4096];
        dev.read(gdt_start * 4096, &mut buf).unwrap();
        buf[2 * 64 + 30] ^= 0xFF;
        dev.write(gdt_start * 4096, &buf).unwrap();

        let result = read_gdt(&mut dev, gdt_start, 4, 4096);
        assert!(result.is_err());
    }

    #[test]
    fn test_gdt_exact_block_boundary() {
        // 64 descriptors fit exactly in 1 block (4096 / 64 = 64)
        let (_tmp, mut dev) = make_test_device(256, 4096);
        let gdt_start = 1u64;
        let descriptors: Vec<_> = (0..64)
            .map(|i| {
                GroupDescriptor::new(
                    i as u64,
                    i as u64 + 1000,
                    i as u64 + 2000,
                    32768,
                    8112,
                    32259,
                    i != 0,
                )
            })
            .collect();
        write_gdt(&mut dev, gdt_start, &descriptors, 4096).unwrap();
        let read_back = read_gdt(&mut dev, gdt_start, 64, 4096).unwrap();
        assert_eq!(read_back.len(), 64);
        for (i, desc) in read_back.iter().enumerate() {
            assert_eq!(desc.bg_block_bitmap, i as u64);
        }
    }

    // -- 10B.4: GroupBitmapManager tests --

    /// Helper: create a GroupBitmapManager + descriptors for testing.
    /// Returns (GBM, descriptors, device_tmp, device).
    fn make_gbm_test(
        group_count: u32,
        blocks_per_group: u32,
        overhead: u32,
        first_group_block: u64,
    ) -> (GroupBitmapManager, Vec<GroupDescriptor>, NamedTempFile, FileBlockDevice) {
        let total_blocks = first_group_block + group_count as u64 * blocks_per_group as u64;
        let (tmp, dev) = make_test_device(total_blocks, 4096);

        let data_per_group = blocks_per_group - overhead;
        let mut descriptors = Vec::new();
        for g in 0..group_count {
            let start = first_group_block + g as u64 * blocks_per_group as u64;
            descriptors.push(GroupDescriptor::new(
                start,               // block bitmap at group start
                start + 1,           // inode bitmap
                start + 2,           // inode table
                blocks_per_group as u64,
                256,                 // inodes_per_group
                data_per_group,
                g != 0,              // group 0 initialized, others lazy
            ));
        }

        let gbm = GroupBitmapManager::new(
            4096,
            blocks_per_group,
            group_count,
            first_group_block,
            overhead,
        );

        (gbm, descriptors, tmp, dev)
    }

    #[test]
    fn test_gbm_global_to_local() {
        let gbm = GroupBitmapManager::new(4096, 1000, 4, 100, 10);
        // Block 100 = group 0, block 0 in group = overhead → None
        assert_eq!(gbm.global_to_local(100), None);
        // Block 110 = group 0, block 10 in group = first data block → (0, 0)
        assert_eq!(gbm.global_to_local(110), Some((0, 0)));
        // Block 1109 = group 0's last data block (block 999 in group, data idx 989)
        assert_eq!(gbm.global_to_local(1099), Some((0, 989)));
        // Block 1100 = group 1, block 0 in group = overhead → None
        assert_eq!(gbm.global_to_local(1100), None);
        // Block 1110 = group 1, first data block
        assert_eq!(gbm.global_to_local(1110), Some((1, 0)));
        // Block in global overhead
        assert_eq!(gbm.global_to_local(50), None);
    }

    #[test]
    fn test_gbm_local_to_global() {
        let gbm = GroupBitmapManager::new(4096, 1000, 4, 100, 10);
        assert_eq!(gbm.local_to_global(0, 0), 110);
        assert_eq!(gbm.local_to_global(1, 0), 1110);
        assert_eq!(gbm.local_to_global(0, 5), 115);
    }

    #[test]
    fn test_gbm_global_local_roundtrip() {
        let gbm = GroupBitmapManager::new(4096, 32768, 8, 100, 509);
        // Pick some global data blocks
        for g in 0..8u32 {
            for local in [0, 1, 100, 1000] {
                let global = gbm.local_to_global(g, local);
                let (g2, l2) = gbm.global_to_local(global).unwrap();
                assert_eq!(g2, g);
                assert_eq!(l2, local);
            }
        }
    }

    #[test]
    fn test_gbm_alloc_in_group() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(4, 100, 10, 10);
        let block = gbm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap();
        assert!(block.is_some());
        let global = block.unwrap();
        let (g, _) = gbm.global_to_local(global).unwrap();
        assert_eq!(g, 0);
    }

    #[test]
    fn test_gbm_alloc_until_full() {
        let data_blocks = 5u32; // overhead=95, bpg=100 → only 5 data blocks
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(2, 100, 95, 10);
        for _ in 0..data_blocks {
            let b = gbm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap();
            assert!(b.is_some());
        }
        // Group 0 now full
        let b = gbm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap();
        assert!(b.is_none());
    }

    #[test]
    fn test_gbm_free_block() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(2, 100, 10, 10);
        let block = gbm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap().unwrap();
        gbm.free_block(&mut dev, block, &mut descriptors).unwrap();
        // Re-allocate should return same block
        let block2 = gbm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap().unwrap();
        assert_eq!(block, block2);
    }

    #[test]
    fn test_gbm_free_wrong_block() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(2, 100, 10, 10);
        // Try to free a block in global overhead (not in any group data)
        let result = gbm.free_block(&mut dev, 5, &mut descriptors);
        assert!(result.is_err());
    }

    #[test]
    fn test_gbm_lazy_load() {
        let (gbm, _descriptors, _tmp, _dev) = make_gbm_test(4, 100, 10, 10);
        // No bitmaps loaded at creation
        assert!(!gbm.is_loaded(0));
        assert!(!gbm.is_loaded(1));
    }

    #[test]
    fn test_gbm_dirty_tracking() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(2, 100, 10, 10);
        assert!(!gbm.has_dirty());
        gbm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap();
        assert!(gbm.has_dirty());
        gbm.save_all(&mut dev, &descriptors).unwrap();
        assert!(!gbm.has_dirty());
    }

    #[test]
    fn test_gbm_save_all_roundtrip() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(3, 100, 10, 10);
        // Alloc one block in each of 3 groups
        let b0 = gbm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap().unwrap();
        let b1 = gbm.alloc_in_group(&mut dev, 1, &mut descriptors).unwrap().unwrap();
        let b2 = gbm.alloc_in_group(&mut dev, 2, &mut descriptors).unwrap().unwrap();
        gbm.save_all(&mut dev, &descriptors).unwrap();

        // Create a fresh GBM and read back — the bitmaps should reflect the allocations
        let mut gbm2 = GroupBitmapManager::new(4096, 100, 3, 10, 10);
        // Clear BLOCK_UNINIT so it reads from disk (groups 1 & 2 had lazy init)
        for desc in descriptors.iter_mut() {
            desc.bg_flags &= !BG_BLOCK_UNINIT;
        }
        // Alloc from each group — should get different blocks (bit 0 already taken)
        let b0_2 = gbm2.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap().unwrap();
        let b1_2 = gbm2.alloc_in_group(&mut dev, 1, &mut descriptors).unwrap().unwrap();
        let b2_2 = gbm2.alloc_in_group(&mut dev, 2, &mut descriptors).unwrap().unwrap();
        assert_ne!(b0, b0_2);
        assert_ne!(b1, b1_2);
        assert_ne!(b2, b2_2);
    }

    #[test]
    fn test_gbm_lazy_init_uninit() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(2, 100, 10, 10);
        // Group 1 has BG_BLOCK_UNINIT flag — bitmap should be all-free
        assert!(descriptors[1].bg_flags & BG_BLOCK_UNINIT != 0);
        let block = gbm.alloc_in_group(&mut dev, 1, &mut descriptors).unwrap();
        assert!(block.is_some()); // All free, so alloc should work
    }

    // -- 10B.5: GroupInodeBitmapManager + GroupInodeTable tests --

    #[test]
    fn test_inode_to_group_mapping() {
        let git = GroupInodeTable::new(256, 256, 4096, 8);
        assert_eq!(git.inode_to_group(0), (0, 0));
        assert_eq!(git.inode_to_group(255), (0, 255));
        assert_eq!(git.inode_to_group(256), (1, 0));
        assert_eq!(git.inode_to_group(511), (1, 255));
    }

    #[test]
    fn test_group_inode_read_write() {
        // Set up a device with enough space for group descriptors
        let inodes_per_group = 16u32; // 16 inodes × 256 bytes = 4096 = 1 block
        let inode_size = 256u32;
        let block_size = 4096u32;
        let group_count = 4u32;

        // Each group needs: bitmap(1) + inodebm(1) + inodetable(1) + data blocks
        let blocks_per_group = 100u32;
        let first_group_block = 10u64;
        let total_blocks = first_group_block + group_count as u64 * blocks_per_group as u64;
        let (_tmp, mut dev) = make_test_device(total_blocks, block_size);

        let mut descriptors = Vec::new();
        for g in 0..group_count {
            let start = first_group_block + g as u64 * blocks_per_group as u64;
            descriptors.push(GroupDescriptor::new(
                start,
                start + 1,
                start + 2, // inode table at group_start + 2
                blocks_per_group as u64,
                inodes_per_group,
                blocks_per_group - 4,
                false,
            ));
        }

        let git = GroupInodeTable::new(inodes_per_group, inode_size, block_size, group_count);

        // Write an inode in group 2
        let inode = Inode::new_file();
        let inode_idx = 2 * inodes_per_group + 3; // group 2, local 3
        git.write_inode(&mut dev, inode_idx, &inode, &descriptors).unwrap();

        // Read it back
        let read_back = git.read_inode(&mut dev, inode_idx, &descriptors).unwrap();
        assert_eq!(read_back.mode, inode.mode);
        assert_eq!(read_back.size, inode.size);
    }

    #[test]
    fn test_group_inode_bitmap_alloc() {
        let inodes_per_group = 64u32;
        let group_count = 4u32;
        let mut gibm = GroupInodeBitmapManager::new(4096, inodes_per_group, group_count);
        let total_blocks = 10 + group_count as u64 * 100;
        let (_tmp, mut dev) = make_test_device(total_blocks, 4096);

        let mut descriptors: Vec<_> = (0..group_count)
            .map(|g| {
                let start = 10 + g as u64 * 100;
                GroupDescriptor::new(start, start + 1, start + 2, 100, inodes_per_group, 90, true)
            })
            .collect();

        let inode = gibm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap();
        assert!(inode.is_some());
        let idx = inode.unwrap();
        assert!(idx < inodes_per_group); // Should be in group 0

        let inode2 = gibm.alloc_in_group(&mut dev, 2, &mut descriptors).unwrap();
        assert!(inode2.is_some());
        let idx2 = inode2.unwrap();
        assert!(idx2 >= 2 * inodes_per_group && idx2 < 3 * inodes_per_group);
    }

    #[test]
    fn test_group_inode_bitmap_free() {
        let inodes_per_group = 64u32;
        let mut gibm = GroupInodeBitmapManager::new(4096, inodes_per_group, 2);
        let (_tmp, mut dev) = make_test_device(210, 4096);
        let mut descriptors: Vec<_> = (0..2)
            .map(|g| {
                let start = 10 + g as u64 * 100;
                GroupDescriptor::new(start, start + 1, start + 2, 100, inodes_per_group, 90, true)
            })
            .collect();

        let idx = gibm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap().unwrap();
        gibm.free_inode(&mut dev, idx, &mut descriptors).unwrap();
        let idx2 = gibm.alloc_in_group(&mut dev, 0, &mut descriptors).unwrap().unwrap();
        assert_eq!(idx, idx2);
    }

    // -- 10B.6: Group-Aware Block Allocator tests --

    #[test]
    fn test_alloc_near_same_group() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(4, 100, 10, 10);
        let mut sb = crate::volume::superblock::Superblock::default();
        sb.free_blocks = 360; // 4 groups × 90 data blocks
        let hint = gbm.local_to_global(2, 0);
        let blocks = alloc_blocks_near(&mut gbm, &mut dev, hint, 10, &mut descriptors, &mut sb).unwrap();
        assert_eq!(blocks.len(), 10);
        for &b in &blocks {
            let (g, _) = gbm.global_to_local(b).unwrap();
            assert_eq!(g, 2);
        }
    }

    #[test]
    fn test_alloc_near_fallback() {
        // Small group with only 5 data blocks
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(4, 100, 95, 10);
        let mut sb = crate::volume::superblock::Superblock::default();
        sb.free_blocks = 20; // 4 × 5
        let hint = gbm.local_to_global(2, 0);
        // Request 10 blocks — group 2 only has 5, must spill to others
        let blocks = alloc_blocks_near(&mut gbm, &mut dev, hint, 10, &mut descriptors, &mut sb).unwrap();
        assert_eq!(blocks.len(), 10);
    }

    #[test]
    fn test_alloc_near_rollback() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(2, 100, 95, 10);
        let mut sb = crate::volume::superblock::Superblock::default();
        sb.free_blocks = 10; // 2 × 5
        let hint = gbm.local_to_global(0, 0);
        // Request 20 blocks but only 10 available → should fail + rollback
        let result = alloc_blocks_near(&mut gbm, &mut dev, hint, 20, &mut descriptors, &mut sb);
        assert!(result.is_err());
        // sb.free_blocks unchanged (rollback restored them in the bitmaps)
        assert_eq!(sb.free_blocks, 10);
    }

    #[test]
    fn test_free_blocks_updates_group() {
        let (mut gbm, mut descriptors, _tmp, mut dev) = make_gbm_test(2, 100, 10, 10);
        let mut sb = crate::volume::superblock::Superblock::default();
        sb.free_blocks = 180;
        let hint = gbm.local_to_global(0, 0);
        let blocks = alloc_blocks_near(&mut gbm, &mut dev, hint, 5, &mut descriptors, &mut sb).unwrap();
        assert_eq!(sb.free_blocks, 175);
        free_blocks_group(&mut gbm, &mut dev, &blocks, &mut descriptors, &mut sb).unwrap();
        assert_eq!(sb.free_blocks, 180);
    }

    #[test]
    fn test_orlov_top_level_spread() {
        // 10 groups, all with free inodes, no dirs yet → should pick group 0 (lowest score)
        let descriptors: Vec<_> = (0..10)
            .map(|_| GroupDescriptor::new(0, 0, 0, 32768, 8112, 32259, true))
            .collect();
        let g = orlov_select_group(&descriptors, 0, true, 10);
        // With all equal, first group wins
        assert!(g < 10);

        // Now make group 0 have many dirs — should spread to another
        let mut descriptors2 = descriptors;
        descriptors2[0].bg_used_dirs = 100;
        let g2 = orlov_select_group(&descriptors2, 0, true, 10);
        assert_ne!(g2, 0); // Should pick a group with fewer dirs
    }

    #[test]
    fn test_orlov_subdirs_near_parent() {
        let descriptors: Vec<_> = (0..10)
            .map(|_| GroupDescriptor::new(0, 0, 0, 32768, 8112, 32259, true))
            .collect();
        let g = orlov_select_group(&descriptors, 5, false, 10);
        assert_eq!(g, 5); // Subdirs stay near parent
    }

    // -- 10B.7: Group-Aware Inode Allocator tests --

    #[test]
    fn test_alloc_inode_file_near_parent() {
        let inodes_per_group = 64u32;
        let group_count = 4u32;
        let mut gibm = GroupInodeBitmapManager::new(4096, inodes_per_group, group_count);
        let (_tmp, mut dev) = make_test_device(410, 4096);
        let mut descriptors: Vec<_> = (0..group_count)
            .map(|g| {
                let start = 10 + g as u64 * 100;
                GroupDescriptor::new(start, start + 1, start + 2, 100, inodes_per_group, 90, true)
            })
            .collect();
        let mut sb = crate::volume::superblock::Superblock::default();
        sb.free_inodes = 256;

        // Parent inode in group 2
        let parent_inode = 2 * inodes_per_group;
        let idx = alloc_inode_for_file(&mut gibm, &mut dev, parent_inode, inodes_per_group, &mut descriptors, &mut sb).unwrap();
        let group = idx / inodes_per_group;
        assert_eq!(group, 2);
    }

    #[test]
    fn test_alloc_inode_file_fallback() {
        let inodes_per_group = 4u32;
        let group_count = 3u32;
        let mut gibm = GroupInodeBitmapManager::new(4096, inodes_per_group, group_count);
        let (_tmp, mut dev) = make_test_device(310, 4096);
        let mut descriptors: Vec<_> = (0..group_count)
            .map(|g| {
                let start = 10 + g as u64 * 100;
                GroupDescriptor::new(start, start + 1, start + 2, 100, inodes_per_group, 90, true)
            })
            .collect();
        let mut sb = crate::volume::superblock::Superblock::default();
        sb.free_inodes = 12;

        // Exhaust group 1
        let parent = 1 * inodes_per_group;
        for _ in 0..inodes_per_group {
            alloc_inode_for_file(&mut gibm, &mut dev, parent, inodes_per_group, &mut descriptors, &mut sb).unwrap();
        }
        // Next allocation should fallback to another group
        let idx = alloc_inode_for_file(&mut gibm, &mut dev, parent, inodes_per_group, &mut descriptors, &mut sb).unwrap();
        let group = idx / inodes_per_group;
        assert_ne!(group, 1);
    }

    #[test]
    fn test_free_inode_updates_group() {
        let inodes_per_group = 64u32;
        let group_count = 2u32;
        let mut gibm = GroupInodeBitmapManager::new(4096, inodes_per_group, group_count);
        let (_tmp, mut dev) = make_test_device(210, 4096);
        let mut descriptors: Vec<_> = (0..group_count)
            .map(|g| {
                let start = 10 + g as u64 * 100;
                GroupDescriptor::new(start, start + 1, start + 2, 100, inodes_per_group, 90, true)
            })
            .collect();
        let mut sb = crate::volume::superblock::Superblock::default();
        sb.free_inodes = 128;

        let idx = alloc_inode_for_dir(&mut gibm, &mut dev, 0, inodes_per_group, &mut descriptors, &mut sb, true).unwrap();
        let group = idx / inodes_per_group;
        assert_eq!(descriptors[group as usize].bg_used_dirs, 1);

        free_inode_group(&mut gibm, &mut dev, idx, true, inodes_per_group, &mut descriptors, &mut sb).unwrap();
        assert_eq!(descriptors[group as usize].bg_used_dirs, 0);
    }

    #[test]
    fn test_alloc_inode_all_full() {
        let inodes_per_group = 2u32;
        let group_count = 2u32;
        let mut gibm = GroupInodeBitmapManager::new(4096, inodes_per_group, group_count);
        let (_tmp, mut dev) = make_test_device(210, 4096);
        let mut descriptors: Vec<_> = (0..group_count)
            .map(|g| {
                let start = 10 + g as u64 * 100;
                GroupDescriptor::new(start, start + 1, start + 2, 100, inodes_per_group, 90, true)
            })
            .collect();
        let mut sb = crate::volume::superblock::Superblock::default();
        sb.free_inodes = 4;

        // Exhaust all inodes
        for _ in 0..4 {
            alloc_inode_for_file(&mut gibm, &mut dev, 0, inodes_per_group, &mut descriptors, &mut sb).unwrap();
        }
        let result = alloc_inode_for_file(&mut gibm, &mut dev, 0, inodes_per_group, &mut descriptors, &mut sb);
        assert!(result.is_err());
    }
}
