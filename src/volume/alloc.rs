use anyhow::{bail, Result};

use crate::block_device::CFSBlockDevice;
use super::bitmap::Bitmap;
use super::group::{GroupBitmapManager, GroupDescriptor};
use super::superblock::Superblock;

// ---------------------------------------------------------------------------
// BlockAlloc — unified block allocator abstraction
// ---------------------------------------------------------------------------

/// Unified block allocation interface used by file_io and dir.
///
/// Variants wrap either the legacy flat bitmap or the group-aware manager.
/// All returned block indices are **physical** (absolute) disk block addresses.
pub enum BlockAlloc<'a> {
    /// Legacy flat bitmap. Physical = data_start + bitmap_index.
    Legacy {
        bitmap: &'a mut Bitmap,
        data_start: u64,
    },
    /// Group-aware allocator with per-group bitmaps.
    Group {
        gbm: &'a mut GroupBitmapManager,
        gdt: &'a mut [GroupDescriptor],
    },
}

impl<'a> BlockAlloc<'a> {
    /// Allocate `n` data blocks. Returns physical block addresses.
    pub fn alloc(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        sb: &mut Superblock,
        n: u64,
    ) -> Result<Vec<u64>> {
        match self {
            BlockAlloc::Legacy { bitmap, data_start } => {
                let indices = alloc_blocks(bitmap, sb, n)?;
                Ok(indices.iter().map(|&i| *data_start + i).collect())
            }
            BlockAlloc::Group { gbm, gdt } => {
                super::group::alloc_blocks_group(gbm, dev, n as usize, gdt, sb)
            }
        }
    }

    /// Allocate `n` data blocks near a locality hint. Returns physical block addresses.
    pub fn alloc_near(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        sb: &mut Superblock,
        n: u64,
        hint: u64,
    ) -> Result<Vec<u64>> {
        match self {
            BlockAlloc::Legacy { bitmap, data_start } => {
                let indices = alloc_blocks(bitmap, sb, n)?;
                Ok(indices.iter().map(|&i| *data_start + i).collect())
            }
            BlockAlloc::Group { gbm, gdt } => {
                super::group::alloc_blocks_near(gbm, dev, hint, n as usize, gdt, sb)
            }
        }
    }

    /// Free blocks given their physical addresses.
    pub fn free(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        sb: &mut Superblock,
        blocks: &[u64],
    ) -> Result<()> {
        match self {
            BlockAlloc::Legacy { bitmap, data_start } => {
                let indices: Vec<u64> = blocks.iter().map(|&b| b - *data_start).collect();
                free_blocks(bitmap, sb, &indices)
            }
            BlockAlloc::Group { gbm, gdt } => {
                super::group::free_blocks_group(gbm, dev, blocks, gdt, sb)
            }
        }
    }

    /// Free blocks with secure zeroing: zero block contents on disk before
    /// freeing in bitmap. Falls back to normal free for group mode (group
    /// allocator handles zeroing internally if needed in future).
    pub fn secure_free(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        sb: &mut Superblock,
        blocks: &[u64],
        block_size: u32,
    ) -> Result<()> {
        // Zero all blocks on disk first
        let zero_buf = vec![0u8; block_size as usize];
        for &phys in blocks {
            dev.write(phys * block_size as u64, &zero_buf)?;
        }
        dev.flush()?;
        // Then free normally in bitmap
        self.free(dev, sb, blocks)
    }
}

/// Allocate `n` data blocks. Returns their bitmap indices.
///
/// If the bitmap cannot satisfy the request, everything allocated so far is
/// rolled back and an error is returned.
pub fn alloc_blocks(
    bitmap: &mut Bitmap,
    sb: &mut Superblock,
    n: u64,
) -> Result<Vec<u64>> {
    if n > sb.free_blocks {
        bail!(
            "not enough space: requested {n}, available {}",
            sb.free_blocks
        );
    }

    let mut allocated = Vec::with_capacity(n as usize);
    for _ in 0..n {
        match bitmap.alloc() {
            Some(idx) => allocated.push(idx),
            None => {
                // Rollback
                for &idx in &allocated {
                    let _ = bitmap.free(idx);
                }
                bail!("bitmap exhausted unexpectedly");
            }
        }
    }
    sb.free_blocks -= n;
    Ok(allocated)
}

/// Free the given data blocks (by bitmap index).
pub fn free_blocks(
    bitmap: &mut Bitmap,
    sb: &mut Superblock,
    blocks: &[u64],
) -> Result<()> {
    for &idx in blocks {
        bitmap.free(idx)?;
    }
    sb.free_blocks += blocks.len() as u64;
    Ok(())
}

/// Free data blocks with secure zeroing (by bitmap index).
///
/// Overwrites each block with zeros on disk BEFORE flipping the bitmap bit.
/// This ensures that even after the bitmap says "free", the old data is gone.
pub fn secure_free_blocks(
    dev: &mut dyn CFSBlockDevice,
    bitmap: &mut Bitmap,
    sb: &mut Superblock,
    blocks: &[u64],
    data_start: u64,
    block_size: u32,
) -> Result<()> {
    let zero_buf = vec![0u8; block_size as usize];

    for &bitmap_idx in blocks {
        let disk_block = data_start + bitmap_idx;
        let disk_offset = disk_block * block_size as u64;
        dev.write(disk_offset, &zero_buf)?;
    }
    // Flush zeros to disk before updating bitmap
    dev.flush()?;

    for &bitmap_idx in blocks {
        bitmap.free(bitmap_idx)?;
    }
    sb.free_blocks += blocks.len() as u64;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
// Contiguous allocation (10I.1)
// ---------------------------------------------------------------------------

/// Result of a contiguous allocation attempt — a run of physically
/// adjacent blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocRun {
    /// Starting physical (global) block of this run.
    pub start: u64,
    /// Number of contiguous blocks in this run.
    pub count: u64,
}

impl BlockAlloc<'_> {
    /// Allocate `n` blocks, preferring a single contiguous run.
    ///
    /// Strategy:
    /// 1. Try to find `n` contiguous blocks in `hint_group` (or flat bitmap).
    /// 2. If group mode: try other groups sorted by free-block count.
    /// 3. Split into multiple contiguous runs across groups.
    /// 4. Fall back to single-block scattered allocation.
    ///
    /// Returns a list of `AllocRun`s. The caller should create one extent per run.
    pub fn alloc_contiguous(
        &mut self,
        dev: &mut dyn CFSBlockDevice,
        sb: &mut Superblock,
        n: u64,
        hint_group: u32,
    ) -> Result<Vec<AllocRun>> {
        if n == 0 {
            return Ok(vec![]);
        }
        if sb.free_blocks < n {
            bail!("not enough free blocks: need {}, have {}", n, sb.free_blocks);
        }
        match self {
            BlockAlloc::Legacy { bitmap, data_start } => {
                alloc_contiguous_legacy(bitmap, sb, n, *data_start)
            }
            BlockAlloc::Group { gbm, gdt } => {
                alloc_contiguous_group(gbm, dev, sb, gdt, n, hint_group)
            }
        }
    }
}

/// Contiguous allocation on a flat (legacy) bitmap.
fn alloc_contiguous_legacy(
    bitmap: &mut Bitmap,
    sb: &mut Superblock,
    n: u64,
    data_start: u64,
) -> Result<Vec<AllocRun>> {
    // Try to find a single contiguous run
    if let Some(start_bit) = bitmap.find_contiguous_run(n, 0) {
        for i in 0..n {
            bitmap.set_allocated(start_bit + i);
        }
        sb.free_blocks -= n;
        return Ok(vec![AllocRun {
            start: data_start + start_bit,
            count: n,
        }]);
    }

    // Fallback: allocate one at a time, then merge adjacent runs
    let mut blocks = Vec::with_capacity(n as usize);
    for _ in 0..n {
        match bitmap.alloc() {
            Some(idx) => blocks.push(data_start + idx),
            None => {
                // Rollback
                for &b in &blocks {
                    let _ = bitmap.free(b - data_start);
                }
                bail!("bitmap exhausted during contiguous fallback");
            }
        }
    }
    sb.free_blocks -= n;
    blocks.sort();
    Ok(merge_adjacent_runs_from_blocks(&blocks))
}

/// Contiguous allocation on group-aware bitmaps.
fn alloc_contiguous_group(
    gbm: &mut GroupBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    sb: &mut Superblock,
    gdt: &mut [GroupDescriptor],
    n: u64,
    hint_group: u32,
) -> Result<Vec<AllocRun>> {
    // Build group priority: hint_group first, then others sorted by free_blocks desc
    let group_count = gbm.group_count;

    // Phase 1: Try contiguous in hint_group
    if let Some(run) = try_contiguous_in_group(gbm, dev, gdt, hint_group, n)? {
        sb.free_blocks -= run.count;
        return Ok(vec![run]);
    }

    // Phase 2: Try other groups sorted by free_blocks desc
    let mut others: Vec<u32> = (0..group_count).filter(|&g| g != hint_group).collect();
    others.sort_by(|a, b| gdt[*b as usize].bg_free_blocks.cmp(&gdt[*a as usize].bg_free_blocks));

    for &g in &others {
        if (gdt[g as usize].bg_free_blocks as u64) < n {
            break;
        }
        if let Some(run) = try_contiguous_in_group(gbm, dev, gdt, g, n)? {
            sb.free_blocks -= run.count;
            return Ok(vec![run]);
        }
    }

    // Phase 3: Split across groups — find largest contiguous runs
    let mut remaining = n;
    let mut runs = Vec::new();
    let all_groups: Vec<u32> =
        std::iter::once(hint_group).chain(others.iter().copied()).collect();

    for &g in &all_groups {
        if remaining == 0 {
            break;
        }
        let max_in_group = std::cmp::min(remaining, gdt[g as usize].bg_free_blocks as u64);
        if max_in_group == 0 {
            continue;
        }
        // Binary search for the largest contiguous run
        let mut try_len = max_in_group;
        while try_len > 0 && remaining > 0 {
            if let Some(run) = try_contiguous_in_group(gbm, dev, gdt, g, try_len)? {
                remaining -= run.count;
                runs.push(run);
                break;
            }
            try_len /= 2;
        }
    }

    // Phase 4: Scattered fallback for remaining blocks
    if remaining > 0 {
        let scattered = super::group::alloc_blocks_group(
            gbm, dev, remaining as usize, gdt, sb,
        )?;
        // sb.free_blocks already decremented by alloc_blocks_group
        for b in scattered {
            runs.push(AllocRun { start: b, count: 1 });
        }
    }

    // Account for blocks allocated in phases 1-3
    let phase123_blocks = n - remaining;
    if remaining > 0 {
        // Phase 4 already decremented sb.free_blocks
        sb.free_blocks -= phase123_blocks;
    } else {
        sb.free_blocks -= n;
    }

    // Merge adjacent runs
    runs.sort_by_key(|r| r.start);
    Ok(merge_adjacent_runs(runs))
}

/// Try to allocate `n` contiguous blocks within a single group.
fn try_contiguous_in_group(
    gbm: &mut GroupBitmapManager,
    dev: &mut dyn CFSBlockDevice,
    gdt: &mut [GroupDescriptor],
    group_idx: u32,
    n: u64,
) -> Result<Option<AllocRun>> {
    if group_idx >= gbm.group_count {
        return Ok(None);
    }
    if (gdt[group_idx as usize].bg_free_blocks as u64) < n {
        return Ok(None);
    }

    // Ensure bitmap is loaded
    gbm.ensure_loaded_pub(dev, group_idx, gdt)?;
    let bitmap = match gbm.get_bitmap_mut(group_idx) {
        Some(bm) => bm,
        None => return Ok(None),
    };

    // Search for contiguous run
    let local_start = match bitmap.find_contiguous_run(n, 0) {
        Some(s) => s,
        None => return Ok(None),
    };

    // Mark all bits as allocated
    for i in 0..n {
        bitmap.set_allocated(local_start + i);
    }

    // Update group descriptor
    gdt[group_idx as usize].bg_free_blocks -= n as u32;

    // Mark bitmap dirty
    gbm.mark_dirty(group_idx);

    // Convert to global
    let global_start = gbm.local_to_global(group_idx, local_start as u32);

    Ok(Some(AllocRun {
        start: global_start,
        count: n,
    }))
}

/// Merge runs that are physically adjacent into a single run.
fn merge_adjacent_runs(mut runs: Vec<AllocRun>) -> Vec<AllocRun> {
    if runs.is_empty() {
        return runs;
    }
    runs.sort_by_key(|r| r.start);
    let mut merged = vec![runs[0].clone()];
    for r in &runs[1..] {
        let last = merged.last_mut().unwrap();
        if last.start + last.count == r.start {
            last.count += r.count;
        } else {
            merged.push(r.clone());
        }
    }
    merged
}

/// Convert a flat list of sorted physical block addresses into merged runs.
fn merge_adjacent_runs_from_blocks(blocks: &[u64]) -> Vec<AllocRun> {
    if blocks.is_empty() {
        return vec![];
    }
    let mut runs = Vec::new();
    let mut start = blocks[0];
    let mut count = 1u64;
    for &b in &blocks[1..] {
        if b == start + count {
            count += 1;
        } else {
            runs.push(AllocRun { start, count });
            start = b;
            count = 1;
        }
    }
    runs.push(AllocRun { start, count });
    runs
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::superblock::Superblock;
    use crate::volume::bitmap::Bitmap;

    fn make_test_state(data_blocks: u64) -> (Bitmap, Superblock) {
        let bm = Bitmap::new_empty(data_blocks, 3, 1, 4096);
        let sb = Superblock {
            magic: *b"CFS1",
            version: 2,
            block_size: 4096,
            features_flags: 0,
            total_blocks: data_blocks + 4,
            inode_count: 4,
            root_inode: 0,
            inode_table_start: 1,
            bitmap_start: 3,
            data_start: 4,
            free_blocks: data_blocks,
            inode_bitmap_start: 0,
            free_inodes: 3,
            uuid: [0u8; 16],
            volume_label: [0u8; 32],
            mount_count: 0,
            last_mount_time: 0,
            backup_sb_block: 0,
            checksum: 0,
            // v3 defaults
            inode_size: 128,
            journal_start: 0,
            journal_blocks: 0,
            first_orphan_inode: 0,
            error_behavior: 0,
            default_permissions: 0o755,
            inode_ratio: 16384,
            journal_sequence: 0,
            metadata_hmac: [0u8; 8],
            dir_entry_size: 128,
            blocks_per_group: 0,
            inodes_per_group: 0,
            group_count: 0,
            gdt_start: 0,
            gdt_blocks: 0,
            desc_size: 0,
            reserved_gdt_blocks: 0,
            log_groups_per_flex: 0,
            first_group_block: 0,
            hash_seed: [0u8; 8],
        };
        (bm, sb)
    }

    #[test]
    fn test_alloc_decrements_free_blocks() {
        let (mut bm, mut sb) = make_test_state(100);
        let blocks = alloc_blocks(&mut bm, &mut sb, 10).unwrap();
        assert_eq!(blocks.len(), 10);
        assert_eq!(sb.free_blocks, 90);
    }

    #[test]
    fn test_free_increments_free_blocks() {
        let (mut bm, mut sb) = make_test_state(100);
        let blocks = alloc_blocks(&mut bm, &mut sb, 10).unwrap();
        free_blocks(&mut bm, &mut sb, &blocks[..5]).unwrap();
        assert_eq!(sb.free_blocks, 95);
    }

    #[test]
    fn test_alloc_fails_when_full() {
        let (mut bm, mut sb) = make_test_state(8);
        let _all = alloc_blocks(&mut bm, &mut sb, 8).unwrap();
        assert_eq!(sb.free_blocks, 0);
        assert!(alloc_blocks(&mut bm, &mut sb, 1).is_err());
    }

    #[test]
    fn test_alloc_free_stress() {
        let (mut bm, mut sb) = make_test_state(100);
        let blocks = alloc_blocks(&mut bm, &mut sb, 50).unwrap();
        assert_eq!(sb.free_blocks, 50);

        // Free even indices
        let even: Vec<u64> = blocks.iter().copied().filter(|i| i % 2 == 0).collect();
        let freed_count = even.len() as u64;
        free_blocks(&mut bm, &mut sb, &even).unwrap();
        assert_eq!(sb.free_blocks, 50 + freed_count);

        // Alloc 25 more
        let more = alloc_blocks(&mut bm, &mut sb, 25).unwrap();
        assert_eq!(more.len(), 25);
        assert_eq!(sb.free_blocks, 50 + freed_count - 25);
    }
}
