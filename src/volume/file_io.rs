use anyhow::{bail, Result};
use std::cmp::min;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::block_device::CFSBlockDevice;
use super::alloc::BlockAlloc;
use super::inode::{Inode, INODE_FLAG_EXTENTS, INODE_FLAG_INLINE_DATA};
use super::superblock::Superblock;
use super::extent;
use super::INODE_FILE;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DIRECT_BLOCKS: u64 = 10;

/// Maximum bytes that fit in the inode's inline_area.
pub const MAX_INLINE_DATA_SIZE: usize = 76;

/// Result of probing an extent-based inode for a logical block's status.
enum BlockStatus {
    /// Block is mapped to an initialized extent — physical addr known.
    Mapped(u64),
    /// Block is part of an uninitialized (preallocated) extent.
    Uninitialized(u64),
    /// Block is not mapped (sparse hole or beyond extents).
    Hole,
}

/// Probe an extent-based inode to get the status of a logical block.
/// Distinguishes initialized mappings, uninit (preallocated), and holes.
fn probe_extent_block(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    logical_block: u32,
    block_size: u32,
) -> Result<BlockStatus> {
    match extent::extent_find(dev, inode, logical_block, block_size)? {
        Some(leaf) => {
            let phys = leaf.map(logical_block).unwrap_or(0);
            if phys == 0 {
                Ok(BlockStatus::Hole)
            } else if leaf.is_uninitialized() {
                Ok(BlockStatus::Uninitialized(phys))
            } else {
                Ok(BlockStatus::Mapped(phys))
            }
        }
        None => Ok(BlockStatus::Hole),
    }
}

// ---------------------------------------------------------------------------
// Pointer‐table helpers
// ---------------------------------------------------------------------------

/// Read a block of u64 pointers from disk.
fn read_ptr_table(
    dev: &mut dyn CFSBlockDevice,
    block_addr: u64,
    block_size: u32,
) -> Result<Vec<u64>> {
    let mut buf = vec![0u8; block_size as usize];
    dev.read(block_addr * block_size as u64, &mut buf)?;
    let count = block_size as usize / 8;
    let mut ptrs = Vec::with_capacity(count);
    for i in 0..count {
        let offset = i * 8;
        ptrs.push(u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap()));
    }
    Ok(ptrs)
}

/// Write a block of u64 pointers to disk.
fn write_ptr_table(
    dev: &mut dyn CFSBlockDevice,
    block_addr: u64,
    ptrs: &[u64],
    block_size: u32,
) -> Result<()> {
    let mut buf = vec![0u8; block_size as usize];
    for (i, &ptr) in ptrs.iter().enumerate() {
        let offset = i * 8;
        buf[offset..offset + 8].copy_from_slice(&ptr.to_le_bytes());
    }
    dev.write(block_addr * block_size as u64, &buf)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Block pointer get / set
// ---------------------------------------------------------------------------

/// Map a logical block index (within a file) to a physical disk block number.
/// Returns 0 if the logical block is not mapped (sparse hole).
///
/// Dispatches to the extent tree when `INODE_FLAG_EXTENTS` is set.
pub fn get_block_ptr(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    logical_block: u64,
    block_size: u32,
) -> Result<u64> {
    if inode.flags & INODE_FLAG_EXTENTS != 0 {
        return extent::get_block_ptr_extent(dev, inode, logical_block as u32, block_size);
    }
    get_block_ptr_legacy(dev, inode, logical_block, block_size)
}

/// Legacy direct/indirect/double-indirect block pointer lookup.
fn get_block_ptr_legacy(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    logical_block: u64,
    block_size: u32,
) -> Result<u64> {
    let ptrs_per_block = block_size as u64 / 8;

    if logical_block < DIRECT_BLOCKS {
        return Ok(inode.direct_blocks[logical_block as usize]);
    }

    let adj = logical_block - DIRECT_BLOCKS;

    if adj < ptrs_per_block {
        // Single indirect
        if inode.indirect_block == 0 {
            return Ok(0);
        }
        let table = read_ptr_table(dev, inode.indirect_block, block_size)?;
        return Ok(table[adj as usize]);
    }

    let adj = adj - ptrs_per_block;

    if adj < ptrs_per_block * ptrs_per_block {
        // Double indirect
        if inode.double_indirect == 0 {
            return Ok(0);
        }
        let l1_idx = adj / ptrs_per_block;
        let l2_idx = adj % ptrs_per_block;
        let l1_table = read_ptr_table(dev, inode.double_indirect, block_size)?;
        if l1_table[l1_idx as usize] == 0 {
            return Ok(0);
        }
        let l2_table = read_ptr_table(dev, l1_table[l1_idx as usize], block_size)?;
        return Ok(l2_table[l2_idx as usize]);
    }

    bail!("logical block {} exceeds max file size", logical_block)
}

/// Set the physical disk block for a logical block index.
/// Allocates indirect table blocks on demand via `alloc`.
///
/// For extent-based inodes, uses `extent_insert` instead.
pub fn set_block_ptr(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    logical_block: u64,
    physical_block: u64,
    block_size: u32,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
) -> Result<()> {
    if inode.flags & INODE_FLAG_EXTENTS != 0 {
        return extent::extent_insert(
            dev, inode, alloc, sb,
            logical_block as u32, physical_block, 1, block_size,
        );
    }
    set_block_ptr_legacy(dev, inode, logical_block, physical_block, block_size, alloc, sb)
}

/// Legacy set_block_ptr implementation.
fn set_block_ptr_legacy(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    logical_block: u64,
    physical_block: u64,
    block_size: u32,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
) -> Result<()> {
    let ptrs_per_block = block_size as u64 / 8;

    if logical_block < DIRECT_BLOCKS {
        inode.direct_blocks[logical_block as usize] = physical_block;
        return Ok(());
    }

    let adj = logical_block - DIRECT_BLOCKS;

    if adj < ptrs_per_block {
        // Single indirect — allocate table if needed
        if inode.indirect_block == 0 {
            let table_blocks = alloc.alloc(dev, sb, 1)?;
            inode.indirect_block = table_blocks[0];
            let zero = vec![0u8; block_size as usize];
            dev.write(inode.indirect_block * block_size as u64, &zero)?;
        }
        let mut table = read_ptr_table(dev, inode.indirect_block, block_size)?;
        table[adj as usize] = physical_block;
        write_ptr_table(dev, inode.indirect_block, &table, block_size)?;
        return Ok(());
    }

    let adj = adj - ptrs_per_block;

    if adj < ptrs_per_block * ptrs_per_block {
        // Double indirect — allocate L1 table if needed
        if inode.double_indirect == 0 {
            let table_blocks = alloc.alloc(dev, sb, 1)?;
            inode.double_indirect = table_blocks[0];
            let zero = vec![0u8; block_size as usize];
            dev.write(inode.double_indirect * block_size as u64, &zero)?;
        }
        let mut l1_table = read_ptr_table(dev, inode.double_indirect, block_size)?;
        let l1_idx = adj / ptrs_per_block;
        let l2_idx = adj % ptrs_per_block;
        // Allocate L2 table if needed
        if l1_table[l1_idx as usize] == 0 {
            let table_blocks = alloc.alloc(dev, sb, 1)?;
            l1_table[l1_idx as usize] = table_blocks[0];
            let zero = vec![0u8; block_size as usize];
            dev.write(l1_table[l1_idx as usize] * block_size as u64, &zero)?;
            write_ptr_table(dev, inode.double_indirect, &l1_table, block_size)?;
        }
        let mut l2_table = read_ptr_table(dev, l1_table[l1_idx as usize], block_size)?;
        l2_table[l2_idx as usize] = physical_block;
        write_ptr_table(dev, l1_table[l1_idx as usize], &l2_table, block_size)?;
        return Ok(());
    }

    bail!("logical block {} exceeds max file size", logical_block)
}

// ---------------------------------------------------------------------------
// File data I/O
// ---------------------------------------------------------------------------

fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Read `len` bytes from a file inode starting at byte `offset`.
/// Unmapped blocks return zeros (sparse holes). Reading past EOF stops early.
/// If the inode has INODE_FLAG_INLINE_DATA set, reads from inline_area.
pub fn read_data(
    dev: &mut dyn CFSBlockDevice,
    inode: &Inode,
    block_size: u32,
    offset: u64,
    len: u64,
) -> Result<Vec<u8>> {
    // Inline data path
    if inode.flags & INODE_FLAG_INLINE_DATA != 0 {
        let file_size = inode.size as usize;
        if offset as usize >= file_size {
            return Ok(Vec::new());
        }
        let end = std::cmp::min(offset as usize + len as usize, file_size);
        return Ok(inode.inline_area[offset as usize..end].to_vec());
    }

    let bs = block_size as u64;
    let end = min(offset + len, inode.size);
    if offset >= inode.size {
        return Ok(vec![]);
    }

    let mut result = Vec::with_capacity((end - offset) as usize);
    let mut pos = offset;

    while pos < end {
        let logical_block = pos / bs;
        let offset_in_block = pos % bs;
        let chunk = min(bs - offset_in_block, end - pos);

        let physical = get_block_ptr(dev, inode, logical_block, block_size)?;

        if physical == 0 {
            // Sparse hole — return zeros
            result.resize(result.len() + chunk as usize, 0);
        } else {
            let mut block_buf = vec![0u8; bs as usize];
            dev.read(physical * bs, &mut block_buf)?;
            result.extend_from_slice(
                &block_buf[offset_in_block as usize..(offset_in_block + chunk) as usize],
            );
        }

        pos += chunk;
    }

    Ok(result)
}

/// Write `data` to a file inode at byte `offset`.
/// Allocates new blocks via `alloc` as needed. Caller MUST persist the modified inode.
/// Supports inline data for small files (≤76 bytes, mode=FILE, no extents initialized).
pub fn write_data(
    dev: &mut dyn CFSBlockDevice,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    inode: &mut Inode,
    block_size: u32,
    offset: u64,
    data: &[u8],
) -> Result<()> {
    let end_offset = offset + data.len() as u64;

    // Case 1: Currently inline data
    if inode.flags & INODE_FLAG_INLINE_DATA != 0 {
        if end_offset <= MAX_INLINE_DATA_SIZE as u64 {
            // Still fits inline — write directly
            inode.inline_area[offset as usize..end_offset as usize]
                .copy_from_slice(data);
            if end_offset > inode.size {
                inode.size = end_offset;
            }
            inode.modified = now_timestamp();
            return Ok(());
        } else {
            // Must migrate to regular blocks
            migrate_inline_to_blocks(dev, alloc, sb, inode, block_size)?;
            // Fall through to normal write path
        }
    }

    // Case 2: New file, first write, fits inline (only for v3/256B inodes)
    // V3 inodes have INODE_FLAG_EXTENTS set (from init_extent_root) and persist
    // inline_area on disk. V1/V2 inodes (128B) do NOT persist inline_area.
    if inode.mode == INODE_FILE
        && inode.block_count == 0
        && inode.block_count_hi == 0
        && offset == 0
        && data.len() <= MAX_INLINE_DATA_SIZE
        && inode.flags & INODE_FLAG_EXTENTS != 0
        && inode.xattr_inline_size == 0
    {
        inode.inline_area[..data.len()].copy_from_slice(data);
        inode.size = data.len() as u64;
        inode.flags |= INODE_FLAG_INLINE_DATA;
        inode.modified = now_timestamp();
        return Ok(());
    }

    // Case 3: Normal block/extent write path
    let bs = block_size as u64;
    let mut pos = offset;
    let mut data_pos = 0usize;

    // Track ranges of logical blocks that were uninit and need marking initialized.
    // We batch these so extent_mark_initialized can do fewer tree edits.
    let mut uninit_ranges: Vec<(u32, u32)> = Vec::new(); // (start, count)

    while data_pos < data.len() {
        let logical_block = pos / bs;
        let offset_in_block = pos % bs;
        let chunk = min(bs - offset_in_block, (data.len() - data_pos) as u64);

        // For extent-based inodes, check uninit status first
        let physical;
        if inode.flags & INODE_FLAG_EXTENTS != 0 {
            match probe_extent_block(dev, inode, logical_block as u32, block_size)? {
                BlockStatus::Mapped(phys) => {
                    physical = phys;
                }
                BlockStatus::Uninitialized(phys) => {
                    // Block already allocated as uninit — write directly, mark init later
                    physical = phys;
                    // Accumulate for batch mark_initialized
                    let lb = logical_block as u32;
                    match uninit_ranges.last_mut() {
                        Some((start, count)) if *start + *count == lb => {
                            *count += 1;
                        }
                        _ => {
                            uninit_ranges.push((lb, 1));
                        }
                    }
                }
                BlockStatus::Hole => {
                    // Allocate a new data block
                    let new = alloc.alloc(dev, sb, 1)?;
                    physical = new[0];
                    set_block_ptr(dev, inode, logical_block, physical, block_size, alloc, sb)?;
                    inode.block_count += 1;

                    if offset_in_block != 0 || chunk < bs {
                        let zero = vec![0u8; bs as usize];
                        dev.write(physical * bs, &zero)?;
                    }
                }
            }
        } else {
            // Legacy path
            let p = get_block_ptr(dev, inode, logical_block, block_size)?;
            if p == 0 {
                let new = alloc.alloc(dev, sb, 1)?;
                physical = new[0];
                set_block_ptr(dev, inode, logical_block, physical, block_size, alloc, sb)?;
                inode.block_count += 1;

                if offset_in_block != 0 || chunk < bs {
                    let zero = vec![0u8; bs as usize];
                    dev.write(physical * bs, &zero)?;
                }
            } else {
                physical = p;
            }
        };

        // Read-modify-write if partial block
        let mut block_buf = vec![0u8; bs as usize];
        if offset_in_block != 0 || chunk < bs {
            dev.read(physical * bs, &mut block_buf)?;
        }
        block_buf[offset_in_block as usize..(offset_in_block + chunk) as usize]
            .copy_from_slice(&data[data_pos..data_pos + chunk as usize]);
        dev.write(physical * bs, &block_buf)?;

        pos += chunk;
        data_pos += chunk as usize;
    }

    // Now mark any uninit extents as initialized
    for (start, count) in uninit_ranges {
        extent::extent_mark_initialized(dev, inode, alloc, sb, start, count, block_size)?;
    }

    if pos > inode.size {
        inode.size = pos;
    }
    inode.modified = now_timestamp();
    Ok(())
}

/// Migrate inline data to a data block (or extent).
/// Called when a write would exceed MAX_INLINE_DATA_SIZE.
fn migrate_inline_to_blocks(
    dev: &mut dyn CFSBlockDevice,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    inode: &mut Inode,
    block_size: u32,
) -> Result<()> {
    // Save current inline data
    let current_size = inode.size as usize;
    let mut inline_copy = vec![0u8; current_size];
    inline_copy.copy_from_slice(&inode.inline_area[..current_size]);

    // Clear inline state
    inode.flags &= !INODE_FLAG_INLINE_DATA;
    inode.inline_area = [0u8; MAX_INLINE_DATA_SIZE];
    inode.size = 0;
    inode.block_count = 0;
    inode.block_count_hi = 0;

    // Initialize extent tree root (v3 default for newly migrated files)
    inode.init_extent_root();

    // Write old data through the normal block path
    if current_size > 0 {
        let bs = block_size as u64;
        let mut pos = 0u64;
        let mut data_pos = 0usize;

        while data_pos < inline_copy.len() {
            let logical_block = pos / bs;
            let offset_in_block = pos % bs;
            let chunk = min(bs - offset_in_block, (inline_copy.len() - data_pos) as u64);

            let new = alloc.alloc(dev, sb, 1)?;
            let physical = new[0];
            set_block_ptr(dev, inode, logical_block, physical, block_size, alloc, sb)?;
            inode.block_count += 1;

            let mut block_buf = vec![0u8; bs as usize];
            block_buf[offset_in_block as usize..(offset_in_block + chunk) as usize]
                .copy_from_slice(&inline_copy[data_pos..data_pos + chunk as usize]);
            dev.write(physical * bs, &block_buf)?;

            pos += chunk;
            data_pos += chunk as usize;
        }
        inode.size = current_size as u64;
    }

    Ok(())
}

/// Free ALL data blocks (and indirect table / extent tree blocks) belonging to an inode.
///
/// If `secure` is true, block contents are zeroed on disk before freeing in the bitmap.
/// If the inode has inline data, just clears the inline_area (no blocks to free).
pub fn free_all_blocks(
    dev: &mut dyn CFSBlockDevice,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    inode: &mut Inode,
    block_size: u32,
    secure: bool,
) -> Result<()> {
    // Inline data: no blocks to free
    if inode.flags & INODE_FLAG_INLINE_DATA != 0 {
        inode.inline_area = [0u8; MAX_INLINE_DATA_SIZE];
        inode.size = 0;
        inode.flags &= !INODE_FLAG_INLINE_DATA;
        inode.modified = now_timestamp();
        return Ok(());
    }

    if inode.flags & INODE_FLAG_EXTENTS != 0 {
        return free_all_extents_secure(dev, inode, alloc, sb, block_size, secure);
    }
    free_all_blocks_legacy(dev, alloc, sb, inode, block_size, secure)
}

/// Extent-tree free with optional secure zeroing.
fn free_all_extents_secure(
    dev: &mut dyn CFSBlockDevice,
    inode: &mut Inode,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    block_size: u32,
    secure: bool,
) -> Result<()> {
    let (data_blocks, node_blocks) = extent::collect_all_extents(dev, inode, block_size)?;

    if secure {
        if !data_blocks.is_empty() {
            alloc.secure_free(dev, sb, &data_blocks, block_size)?;
        }
        if !node_blocks.is_empty() {
            alloc.secure_free(dev, sb, &node_blocks, block_size)?;
        }
    } else {
        if !data_blocks.is_empty() {
            alloc.free(dev, sb, &data_blocks)?;
        }
        if !node_blocks.is_empty() {
            alloc.free(dev, sb, &node_blocks)?;
        }
    }

    extent::init_inode_extent_root(inode);
    inode.size = 0;
    inode.block_count = 0;
    inode.block_count_hi = 0;
    Ok(())
}

/// Legacy free_all_blocks implementation (direct/indirect/double-indirect).
fn free_all_blocks_legacy(
    dev: &mut dyn CFSBlockDevice,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    inode: &mut Inode,
    block_size: u32,
    secure: bool,
) -> Result<()> {
    // Collect all blocks to free
    let mut data_blocks: Vec<u64> = Vec::new();
    let mut table_blocks: Vec<u64> = Vec::new();

    // Direct blocks
    for i in 0..10 {
        if inode.direct_blocks[i] != 0 {
            data_blocks.push(inode.direct_blocks[i]);
            inode.direct_blocks[i] = 0;
        }
    }

    // Single indirect
    if inode.indirect_block != 0 {
        let table = read_ptr_table(dev, inode.indirect_block, block_size)?;
        for &ptr in &table {
            if ptr != 0 {
                data_blocks.push(ptr);
            }
        }
        table_blocks.push(inode.indirect_block);
        inode.indirect_block = 0;
    }

    // Double indirect
    if inode.double_indirect != 0 {
        let l1_table = read_ptr_table(dev, inode.double_indirect, block_size)?;
        for &l1_ptr in &l1_table {
            if l1_ptr != 0 {
                let l2_table = read_ptr_table(dev, l1_ptr, block_size)?;
                for &l2_ptr in &l2_table {
                    if l2_ptr != 0 {
                        data_blocks.push(l2_ptr);
                    }
                }
                table_blocks.push(l1_ptr);
            }
        }
        table_blocks.push(inode.double_indirect);
        inode.double_indirect = 0;
    }

    // Free all collected blocks (secure or normal)
    if secure {
        if !data_blocks.is_empty() {
            alloc.secure_free(dev, sb, &data_blocks, block_size)?;
        }
        if !table_blocks.is_empty() {
            alloc.secure_free(dev, sb, &table_blocks, block_size)?;
        }
    } else {
        if !data_blocks.is_empty() {
            alloc.free(dev, sb, &data_blocks)?;
        }
        if !table_blocks.is_empty() {
            alloc.free(dev, sb, &table_blocks)?;
        }
    }

    inode.block_count = 0;
    inode.size = 0;
    inode.modified = now_timestamp();
    Ok(())
}

/// Truncate a file to `new_size` bytes.
/// If new_size >= current size, extends file sparsely (updates size only).
/// If new_size == 0, delegates to free_all_blocks.
/// Handles inline data files specially.
pub fn truncate(
    dev: &mut dyn CFSBlockDevice,
    alloc: &mut BlockAlloc<'_>,
    sb: &mut Superblock,
    inode: &mut Inode,
    block_size: u32,
    new_size: u64,
    secure: bool,
) -> Result<()> {
    // Inline data special handling
    if inode.flags & INODE_FLAG_INLINE_DATA != 0 {
        if new_size == 0 {
            inode.inline_area = [0u8; MAX_INLINE_DATA_SIZE];
            inode.size = 0;
            inode.flags &= !INODE_FLAG_INLINE_DATA;
            inode.modified = now_timestamp();
            return Ok(());
        }
        if new_size <= MAX_INLINE_DATA_SIZE as u64 {
            let old_size = inode.size as usize;
            let new = new_size as usize;
            if new < old_size {
                inode.inline_area[new..old_size].fill(0);
            }
            inode.size = new_size;
            inode.modified = now_timestamp();
            return Ok(());
        } else {
            // Growing beyond inline: migrate first, then extend normally
            migrate_inline_to_blocks(dev, alloc, sb, inode, block_size)?;
            // Fall through to normal truncate (which will just set size since new_size >= current)
        }
    }

    if new_size >= inode.size {
        inode.size = new_size;
        return Ok(());
    }

    if new_size == 0 {
        return free_all_blocks(dev, alloc, sb, inode, block_size, secure);
    }

    let bs = block_size as u64;
    let new_last_block = (new_size - 1) / bs;
    let old_last_block = if inode.size == 0 { 0 } else { (inode.size - 1) / bs };

    if inode.flags & INODE_FLAG_EXTENTS != 0 {
        // Extent-based truncation: remove logical blocks [new_last_block+1..)
        let remove_start = (new_last_block + 1) as u32;
        let remove_len = (old_last_block - new_last_block) as u32;
        if remove_len > 0 {
            let freed = extent::extent_remove(dev, inode, remove_start, remove_len, block_size)?;
            if !freed.is_empty() {
                if secure {
                    alloc.secure_free(dev, sb, &freed, block_size)?;
                } else {
                    alloc.free(dev, sb, &freed)?;
                }
            }
            if inode.block_count as u32 >= freed.len() as u32 {
                inode.block_count -= freed.len() as u32;
            }
        }
    } else {
        // Legacy per-block truncation
        let mut blocks_to_free: Vec<u64> = Vec::new();
        for logical in (new_last_block + 1)..=old_last_block {
            let physical = get_block_ptr(dev, inode, logical, block_size)?;
            if physical != 0 {
                blocks_to_free.push(physical);
                set_block_ptr(dev, inode, logical, 0, block_size, alloc, sb)?;
                inode.block_count -= 1;
            }
        }
        if !blocks_to_free.is_empty() {
            if secure {
                alloc.secure_free(dev, sb, &blocks_to_free, block_size)?;
            } else {
                alloc.free(dev, sb, &blocks_to_free)?;
            }
        }
    }

    // Zero trailing bytes in the last kept block
    let tail = new_size % bs;
    if tail != 0 {
        let physical = get_block_ptr(dev, inode, new_last_block, block_size)?;
        if physical != 0 {
            let mut buf = vec![0u8; bs as usize];
            dev.read(physical * bs, &mut buf)?;
            for b in &mut buf[tail as usize..] {
                *b = 0;
            }
            dev.write(physical * bs, &buf)?;
        }
    }

    inode.size = new_size;
    inode.modified = now_timestamp();
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use crate::volume::{alloc, BlockAlloc, CFSVolume, DEFAULT_BLOCK_SIZE};
    use tempfile::NamedTempFile;

    fn make_vol() -> (NamedTempFile, CFSVolume) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        // 2 MB — enough room for indirect/double-indirect tests
        let dev = FileBlockDevice::open(&path, Some(2 * 1024 * 1024)).unwrap();
        let vol = CFSVolume::format(Box::new(dev), DEFAULT_BLOCK_SIZE).unwrap();
        (tmp, vol)
    }

    #[test]
    fn test_direct_block_set_get() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        // Allocate 2 data blocks
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();

        let blocks = alloc::alloc_blocks(&mut *bm, &mut *sg, 2).unwrap();
        let ds = vol.data_start;
        let phys0 = ds + blocks[0];
        let phys9 = ds + blocks[1];

        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        set_block_ptr(
            &mut **dg, &mut inode, 0, phys0, bs, &mut ba, &mut *sg,
        ).unwrap();
        set_block_ptr(
            &mut **dg, &mut inode, 9, phys9, bs, &mut ba, &mut *sg,
        ).unwrap();

        assert_eq!(get_block_ptr(&mut **dg, &inode, 0, bs).unwrap(), phys0);
        assert_eq!(get_block_ptr(&mut **dg, &inode, 9, bs).unwrap(), phys9);
    }

    #[test]
    fn test_indirect_block_auto_alloc() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();

        // Allocate a data block
        let blocks = alloc::alloc_blocks(&mut *bm, &mut *sg, 1).unwrap();
        let ds = vol.data_start;
        let phys = ds + blocks[0];

        assert_eq!(inode.indirect_block, 0);

        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        set_block_ptr(
            &mut **dg, &mut inode, 10, phys, bs, &mut ba, &mut *sg,
        ).unwrap();

        // indirect table was auto-allocated
        assert_ne!(inode.indirect_block, 0);
        assert_eq!(get_block_ptr(&mut **dg, &inode, 10, bs).unwrap(), phys);
    }

    #[test]
    fn test_double_indirect_set_get() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        let ptrs_per_block = bs as u64 / 8; // 512
        let first_double = DIRECT_BLOCKS + ptrs_per_block; // 522

        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();

        let blocks = alloc::alloc_blocks(&mut *bm, &mut *sg, 1).unwrap();
        let ds = vol.data_start;
        let phys = ds + blocks[0];

        assert_eq!(inode.double_indirect, 0);

        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        set_block_ptr(
            &mut **dg, &mut inode, first_double, phys, bs, &mut ba, &mut *sg,
        ).unwrap();

        assert_ne!(inode.double_indirect, 0);
        assert_eq!(
            get_block_ptr(&mut **dg, &inode, first_double, bs).unwrap(),
            phys
        );
    }

    #[test]
    fn test_get_unallocated_returns_zero() {
        let (_tmp, vol) = make_vol();
        let inode = Inode::new_file();
        let bs = vol.block_size;

        let mut dg = vol.dev();
        assert_eq!(get_block_ptr(&mut **dg, &inode, 0, bs).unwrap(), 0);
        assert_eq!(get_block_ptr(&mut **dg, &inode, 5, bs).unwrap(), 0);
        assert_eq!(get_block_ptr(&mut **dg, &inode, 10, bs).unwrap(), 0);
    }

    // -----------------------------------------------------------------------
    // 3C — File Data I/O Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_write_read_small() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        let data = b"Hello, CFS filesystem!";
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        write_data(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 0, data,
        ).unwrap();

        assert_eq!(inode.size, data.len() as u64);

        let read_back = read_data(&mut **dg, &inode, bs, 0, data.len() as u64).unwrap();
        assert_eq!(&read_back, data);
    }

    #[test]
    fn test_write_read_multi_block() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        // 10,240 bytes = 2.5 blocks
        let data: Vec<u8> = (0..10240).map(|i| (i % 251) as u8).collect();
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        write_data(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 0, &data,
        ).unwrap();

        assert_eq!(inode.size, 10240);
        assert_eq!(inode.block_count, 3); // 3 blocks

        let read_back = read_data(&mut **dg, &inode, bs, 0, 10240).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_write_at_offset() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        let data = b"world";
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        // Write at offset 2048 (mid-block)
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        write_data(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 2048, data,
        ).unwrap();

        assert_eq!(inode.size, 2048 + 5);

        // Read full file — leading part should be zeros
        let read_back = read_data(&mut **dg, &inode, bs, 0, inode.size).unwrap();
        assert!(read_back[..2048].iter().all(|&b| b == 0));
        assert_eq!(&read_back[2048..2053], b"world");
    }

    #[test]
    fn test_write_extends_size() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        write_data(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 0, &[1u8; 100],
        ).unwrap();
        assert_eq!(inode.size, 100);

        // Write 200 bytes at offset 300 → size should be 500
        write_data(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 300, &[2u8; 200],
        ).unwrap();
        assert_eq!(inode.size, 500);
    }

    #[test]
    fn test_free_all_blocks() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let initial_free = sg.free_blocks;

        // Write 10 KB (3 blocks)
        let data = vec![0xABu8; 10240];
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        write_data(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 0, &data,
        ).unwrap();
        assert_eq!(inode.block_count, 3);
        assert_eq!(sg.free_blocks, initial_free - 3);

        free_all_blocks(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, false,
        ).unwrap();

        assert_eq!(inode.size, 0);
        assert_eq!(inode.block_count, 0);
        assert_eq!(sg.free_blocks, initial_free);
    }

    #[test]
    fn test_truncate_shrinks() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        // Write 10 KB
        let data: Vec<u8> = (0..10240).map(|i| (i % 251) as u8).collect();
        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        write_data(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 0, &data,
        ).unwrap();
        assert_eq!(inode.block_count, 3);

        // Truncate to 2 KB (keeps 1 block, frees 2)
        truncate(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 2048, false,
        ).unwrap();

        assert_eq!(inode.size, 2048);
        assert_eq!(inode.block_count, 1);

        let read_back = read_data(&mut **dg, &inode, bs, 0, 2048).unwrap();
        assert_eq!(&read_back, &data[..2048]);
    }

    #[test]
    fn test_truncate_to_zero() {
        let (_tmp, vol) = make_vol();
        let mut inode = Inode::new_file();
        let bs = vol.block_size;

        let mut sg = vol.sb_write();
        let mut bm = vol.bitmap_lock();
        let mut dg = vol.dev();
        let ds = vol.data_start;
        let initial_free = sg.free_blocks;

        let mut ba = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
        write_data(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 0, &[1u8; 5000],
        ).unwrap();

        truncate(
            &mut **dg, &mut ba, &mut *sg,
            &mut inode, bs, 0, false,
        ).unwrap();

        assert_eq!(inode.size, 0);
        assert_eq!(inode.block_count, 0);
        assert_eq!(sg.free_blocks, initial_free);
    }
}