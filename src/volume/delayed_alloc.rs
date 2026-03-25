//! Delayed allocation manager for CFS (Phase 10I.2).
//!
//! Buffers write data in memory and defers physical block allocation until
//! flush, allowing the contiguous allocator to see the full write size and
//! pick optimal physical placement.

use std::collections::{BTreeMap, HashMap};
use anyhow::{bail, Result};

/// Tracks pending writes and reserved block counts for delayed allocation.
pub struct DelayedAllocManager {
    /// Per-inode pending write buffers.
    /// Key: inode index, Value: logical_block → block-sized data buffer.
    pending: HashMap<u32, BTreeMap<u64, Vec<u8>>>,

    /// Per-inode reserved block count (for cleanup on discard).
    reserved_per_inode: HashMap<u32, u64>,

    /// Total reserved blocks across all inodes.
    total_reserved: u64,

    /// Block size (for sizing data buffers).
    block_size: u32,
}

impl DelayedAllocManager {
    pub fn new(block_size: u32) -> Self {
        Self {
            pending: HashMap::new(),
            reserved_per_inode: HashMap::new(),
            total_reserved: 0,
            block_size,
        }
    }

    /// Available blocks for new reservations.
    pub fn available_blocks(&self, sb_free_blocks: u64) -> u64 {
        sb_free_blocks.saturating_sub(self.total_reserved)
    }

    /// Reserve space for a write and buffer the data.
    ///
    /// Returns ENOSPC if there are not enough unreserved free blocks.
    pub fn reserve(
        &mut self,
        sb_free_blocks: u64,
        inode_idx: u32,
        logical_block: u64,
        data: Vec<u8>,
    ) -> Result<()> {
        assert_eq!(data.len(), self.block_size as usize);

        // Check if overwrite (no new reservation needed)
        if let Some(inode_map) = self.pending.get_mut(&inode_idx) {
            if inode_map.contains_key(&logical_block) {
                inode_map.insert(logical_block, data);
                return Ok(());
            }
        }

        // New reservation — check ENOSPC before borrowing pending
        if self.available_blocks(sb_free_blocks) < 1 {
            bail!(
                "No space left on device (ENOSPC): free_blocks={}, reserved={}",
                sb_free_blocks,
                self.total_reserved
            );
        }

        self.pending
            .entry(inode_idx)
            .or_default()
            .insert(logical_block, data);
        *self.reserved_per_inode.entry(inode_idx).or_insert(0) += 1;
        self.total_reserved += 1;

        Ok(())
    }

    /// Read data from the pending buffer for a specific inode and logical block.
    pub fn read_pending(&self, inode_idx: u32, logical_block: u64) -> Option<&[u8]> {
        self.pending
            .get(&inode_idx)
            .and_then(|map| map.get(&logical_block))
            .map(|v| v.as_slice())
    }

    /// Check if an inode has any pending writes.
    pub fn has_pending(&self, inode_idx: u32) -> bool {
        self.pending
            .get(&inode_idx)
            .map(|m| !m.is_empty())
            .unwrap_or(false)
    }

    pub fn total_reserved(&self) -> u64 {
        self.total_reserved
    }

    /// Discard all pending writes for an inode (e.g., on delete or error).
    pub fn discard_inode(&mut self, inode_idx: u32) {
        if let Some(map) = self.pending.remove(&inode_idx) {
            let count = map.len() as u64;
            self.total_reserved = self.total_reserved.saturating_sub(count);
        }
        self.reserved_per_inode.remove(&inode_idx);
    }

    /// Take all pending writes for an inode (removes from manager).
    /// Returns the BTreeMap of logical_block → data, and the count of blocks.
    pub fn take_pending(&mut self, inode_idx: u32) -> Option<BTreeMap<u64, Vec<u8>>> {
        let map = self.pending.remove(&inode_idx)?;
        if map.is_empty() {
            return None;
        }
        let count = map.len() as u64;
        self.total_reserved = self.total_reserved.saturating_sub(count);
        self.reserved_per_inode.remove(&inode_idx);
        Some(map)
    }

    /// Get all inode indices that have pending writes.
    pub fn pending_inodes(&self) -> Vec<u32> {
        self.pending
            .iter()
            .filter(|(_, m)| !m.is_empty())
            .map(|(&k, _)| k)
            .collect()
    }
}

/// Groups contiguous logical block numbers into ranges for efficient allocation.
pub struct ContiguousRange {
    pub start_logical: u64,
    pub blocks: Vec<(u64, Vec<u8>)>, // (logical_block, data) in order
}

/// Group a BTreeMap of pending writes into contiguous logical ranges.
pub fn group_contiguous_ranges(pending: BTreeMap<u64, Vec<u8>>) -> Vec<ContiguousRange> {
    let mut ranges = Vec::new();
    let mut current: Option<ContiguousRange> = None;

    for (logical, data) in pending {
        match &mut current {
            Some(range)
                if logical == range.start_logical + range.blocks.len() as u64 =>
            {
                range.blocks.push((logical, data));
            }
            _ => {
                if let Some(range) = current.take() {
                    ranges.push(range);
                }
                current = Some(ContiguousRange {
                    start_logical: logical,
                    blocks: vec![(logical, data)],
                });
            }
        }
    }
    if let Some(range) = current {
        ranges.push(range);
    }
    ranges
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reserve_basic() {
        let mut da = DelayedAllocManager::new(4096);
        let data = vec![0xABu8; 4096];
        da.reserve(100, 1, 0, data).unwrap();
        assert_eq!(da.total_reserved(), 1);
        assert!(da.has_pending(1));
    }

    #[test]
    fn test_reserve_overwrite() {
        let mut da = DelayedAllocManager::new(4096);
        da.reserve(100, 1, 0, vec![0u8; 4096]).unwrap();
        da.reserve(100, 1, 0, vec![1u8; 4096]).unwrap();
        assert_eq!(da.total_reserved(), 1); // no double-count
    }

    #[test]
    fn test_reserve_enospc() {
        let mut da = DelayedAllocManager::new(4096);
        let result = da.reserve(0, 1, 0, vec![0u8; 4096]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ENOSPC"));
    }

    #[test]
    fn test_reserve_near_full() {
        let mut da = DelayedAllocManager::new(4096);
        // Reserve until 1 block left
        da.reserve(2, 1, 0, vec![0u8; 4096]).unwrap();
        da.reserve(2, 1, 1, vec![0u8; 4096]).unwrap(); // last block
        let result = da.reserve(2, 1, 2, vec![0u8; 4096]);
        assert!(result.is_err()); // ENOSPC
    }

    #[test]
    fn test_read_pending() {
        let mut da = DelayedAllocManager::new(4096);
        let data = vec![0x42u8; 4096];
        da.reserve(100, 1, 5, data.clone()).unwrap();
        let read = da.read_pending(1, 5).unwrap();
        assert_eq!(read, &data[..]);
        assert!(da.read_pending(1, 6).is_none());
    }

    #[test]
    fn test_discard_inode() {
        let mut da = DelayedAllocManager::new(4096);
        for i in 0..5 {
            da.reserve(100, 1, i, vec![0u8; 4096]).unwrap();
        }
        assert_eq!(da.total_reserved(), 5);
        da.discard_inode(1);
        assert_eq!(da.total_reserved(), 0);
        assert!(!da.has_pending(1));
    }

    #[test]
    fn test_group_contiguous_ranges() {
        let mut map = BTreeMap::new();
        map.insert(5, vec![5u8; 4096]);
        map.insert(6, vec![6u8; 4096]);
        map.insert(7, vec![7u8; 4096]);
        map.insert(20, vec![20u8; 4096]);
        map.insert(21, vec![21u8; 4096]);

        let ranges = group_contiguous_ranges(map);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].start_logical, 5);
        assert_eq!(ranges[0].blocks.len(), 3);
        assert_eq!(ranges[1].start_logical, 20);
        assert_eq!(ranges[1].blocks.len(), 2);
    }
}
