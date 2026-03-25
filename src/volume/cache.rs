//! LRU caches for inodes, blocks, and group descriptors (Phase 10J).
//!
//! Each cache provides:
//! - O(1) LRU get/put via the `lru` crate
//! - Dirty tracking for write-back semantics
//! - Flush/invalidate/clear operations
//! - Hit/miss/eviction statistics

use lru::LruCache;
use std::num::NonZeroUsize;

use super::group::GroupDescriptor;
use super::inode::Inode;

// ---------------------------------------------------------------------------
// CacheStats — shared statistics tracker
// ---------------------------------------------------------------------------

/// Cache hit/miss/eviction statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub dirty_writebacks: u64,
}

impl CacheStats {
    /// Hit rate as a fraction in [0.0, 1.0]. Returns 0.0 if no lookups.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

// ===========================================================================
// 10J.1 — Inode Cache
// ===========================================================================

/// A cached inode entry with dirty tracking.
#[derive(Debug, Clone)]
struct CachedInode {
    inode: Inode,
    dirty: bool,
}

/// LRU inode cache with dirty tracking and write-back semantics.
///
/// Capacity is configured via `MountOptions::cache_inodes`.
/// Each inode is 128–256 bytes, so 1024 entries ≈ 128–256 KB.
pub struct InodeCache {
    cache: LruCache<u32, CachedInode>,
    stats: CacheStats,
}

impl InodeCache {
    /// Create a new inode cache with the given capacity.
    ///
    /// # Panics
    /// Panics if `capacity == 0`.
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: LruCache::new(
                NonZeroUsize::new(capacity).expect("inode cache capacity must be > 0"),
            ),
            stats: CacheStats::default(),
        }
    }

    /// Look up an inode. Promotes the entry to most-recently-used on hit.
    pub fn get(&mut self, inode_idx: u32) -> Option<&Inode> {
        match self.cache.get(&inode_idx) {
            Some(entry) => {
                self.stats.hits += 1;
                Some(&entry.inode)
            }
            None => {
                self.stats.misses += 1;
                None
            }
        }
    }

    /// Insert a clean inode (after a disk read / cache miss).
    ///
    /// Returns `Some((evicted_idx, evicted_inode))` if a dirty entry was evicted
    /// and needs writeback. The caller is responsible for writing it to disk.
    pub fn insert(&mut self, inode_idx: u32, inode: Inode) -> Option<(u32, Inode)> {
        // If updating an existing key, just replace (no eviction)
        if self.cache.contains(&inode_idx) {
            self.cache.put(inode_idx, CachedInode { inode, dirty: false });
            return None;
        }

        // If at capacity, pop LRU before inserting
        let evicted = if self.cache.len() >= self.cache.cap().get() {
            self.pop_lru_dirty()
        } else {
            None
        };

        self.cache.put(inode_idx, CachedInode { inode, dirty: false });
        evicted
    }

    /// Update a cached inode and mark it dirty (defers disk write).
    ///
    /// Returns dirty evicted entry if the cache was full and a new slot was needed.
    pub fn put_dirty(&mut self, inode_idx: u32, inode: Inode) -> Option<(u32, Inode)> {
        let evicted = if !self.cache.contains(&inode_idx)
            && self.cache.len() >= self.cache.cap().get()
        {
            self.pop_lru_dirty()
        } else {
            None
        };

        self.cache.put(inode_idx, CachedInode { inode, dirty: true });
        evicted
    }

    /// Remove an inode from the cache.
    ///
    /// Returns the inode if the entry was dirty (caller should write it back
    /// unless the inode is being freed/deleted).
    pub fn invalidate(&mut self, inode_idx: u32) -> Option<Inode> {
        self.cache.pop(&inode_idx).and_then(|entry| {
            if entry.dirty {
                Some(entry.inode)
            } else {
                None
            }
        })
    }

    /// Flush all dirty inodes. Returns `(idx, inode)` pairs for writeback.
    /// After calling, all cached entries are marked clean.
    pub fn flush_dirty(&mut self) -> Vec<(u32, Inode)> {
        let mut dirty_entries = Vec::new();
        for (&idx, entry) in self.cache.iter_mut() {
            if entry.dirty {
                dirty_entries.push((idx, entry.inode.clone()));
                entry.dirty = false;
                self.stats.dirty_writebacks += 1;
            }
        }
        dirty_entries
    }

    /// Clear all entries. Returns dirty entries for writeback.
    pub fn clear(&mut self) -> Vec<(u32, Inode)> {
        let dirty = self.flush_dirty();
        self.cache.clear();
        dirty
    }

    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Pop the LRU entry, returning it only if dirty.
    fn pop_lru_dirty(&mut self) -> Option<(u32, Inode)> {
        if let Some((key, entry)) = self.cache.pop_lru() {
            self.stats.evictions += 1;
            if entry.dirty {
                self.stats.dirty_writebacks += 1;
                Some((key, entry.inode))
            } else {
                None
            }
        } else {
            None
        }
    }
}

// ===========================================================================
// 10J.2 — Block Cache
// ===========================================================================

/// A cached block entry with dirty tracking.
#[derive(Debug, Clone)]
struct CachedBlock {
    data: Vec<u8>,
    dirty: bool,
}

/// LRU block cache for metadata blocks (bitmap, extent tree, directory blocks).
///
/// Capacity is configured via `MountOptions::cache_blocks`.
/// Each block is `block_size` bytes (typically 4096), so 256 entries ≈ 1 MB.
///
/// File data blocks should bypass this cache to avoid evicting valuable metadata.
pub struct BlockCache {
    cache: LruCache<u64, CachedBlock>,
    block_size: u32,
    stats: CacheStats,
}

impl BlockCache {
    /// Create a new block cache.
    ///
    /// # Panics
    /// Panics if `capacity == 0`.
    pub fn new(capacity: usize, block_size: u32) -> Self {
        Self {
            cache: LruCache::new(
                NonZeroUsize::new(capacity).expect("block cache capacity must be > 0"),
            ),
            block_size,
            stats: CacheStats::default(),
        }
    }

    /// Read a block from the cache. Returns a reference to the data on hit.
    pub fn get(&mut self, block_addr: u64) -> Option<&[u8]> {
        match self.cache.get(&block_addr) {
            Some(entry) => {
                self.stats.hits += 1;
                Some(&entry.data)
            }
            None => {
                self.stats.misses += 1;
                None
            }
        }
    }

    /// Insert a clean block (after a disk read).
    ///
    /// Returns dirty evicted block `(addr, data)` if one was displaced.
    pub fn insert(&mut self, block_addr: u64, data: Vec<u8>) -> Option<(u64, Vec<u8>)> {
        debug_assert_eq!(data.len(), self.block_size as usize);

        if self.cache.contains(&block_addr) {
            self.cache.put(block_addr, CachedBlock { data, dirty: false });
            return None;
        }

        let evicted = if self.cache.len() >= self.cache.cap().get() {
            self.pop_lru_dirty()
        } else {
            None
        };

        self.cache.put(block_addr, CachedBlock { data, dirty: false });
        evicted
    }

    /// Write a block to the cache and mark dirty (defers disk write).
    ///
    /// Returns dirty evicted block if one was displaced.
    pub fn put_dirty(&mut self, block_addr: u64, data: Vec<u8>) -> Option<(u64, Vec<u8>)> {
        debug_assert_eq!(data.len(), self.block_size as usize);

        let evicted = if !self.cache.contains(&block_addr)
            && self.cache.len() >= self.cache.cap().get()
        {
            self.pop_lru_dirty()
        } else {
            None
        };

        self.cache.put(block_addr, CachedBlock { data, dirty: true });
        evicted
    }

    /// Modify a cached block in-place and mark it dirty.
    ///
    /// Returns `true` if the block was found and modified, `false` if not cached.
    pub fn modify<F>(&mut self, block_addr: u64, f: F) -> bool
    where
        F: FnOnce(&mut [u8]),
    {
        if let Some(entry) = self.cache.get_mut(&block_addr) {
            f(&mut entry.data);
            entry.dirty = true;
            true
        } else {
            false
        }
    }

    /// Invalidate a single block.
    /// Returns dirty data if the entry was dirty, for writeback.
    pub fn invalidate(&mut self, block_addr: u64) -> Option<Vec<u8>> {
        self.cache.pop(&block_addr).and_then(|entry| {
            if entry.dirty {
                Some(entry.data)
            } else {
                None
            }
        })
    }

    /// Invalidate a range of blocks. Returns all dirty entries for writeback.
    pub fn invalidate_range(&mut self, start: u64, count: u64) -> Vec<(u64, Vec<u8>)> {
        let mut dirty = Vec::new();
        for addr in start..start.saturating_add(count) {
            if let Some(data) = self.invalidate(addr) {
                dirty.push((addr, data));
            }
        }
        dirty
    }

    /// Flush all dirty blocks. Returns `(addr, data)` pairs for writeback.
    pub fn flush_dirty(&mut self) -> Vec<(u64, Vec<u8>)> {
        let mut dirty = Vec::new();
        for (&addr, entry) in self.cache.iter_mut() {
            if entry.dirty {
                dirty.push((addr, entry.data.clone()));
                entry.dirty = false;
                self.stats.dirty_writebacks += 1;
            }
        }
        dirty
    }

    /// Clear all entries. Returns dirty entries for writeback.
    pub fn clear(&mut self) -> Vec<(u64, Vec<u8>)> {
        let dirty = self.flush_dirty();
        self.cache.clear();
        dirty
    }

    /// Estimated memory usage in bytes.
    pub fn memory_usage(&self) -> usize {
        self.cache.len() * (self.block_size as usize + std::mem::size_of::<CachedBlock>())
    }

    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    fn pop_lru_dirty(&mut self) -> Option<(u64, Vec<u8>)> {
        if let Some((key, entry)) = self.cache.pop_lru() {
            self.stats.evictions += 1;
            if entry.dirty {
                self.stats.dirty_writebacks += 1;
                Some((key, entry.data))
            } else {
                None
            }
        } else {
            None
        }
    }
}

// ===========================================================================
// 10J.3 — Group Descriptor Table Cache
// ===========================================================================

/// Cached GDT entry with per-entry dirty tracking.
#[derive(Debug, Clone)]
struct CachedGd {
    gd: GroupDescriptor,
    dirty: bool,
}

/// Group descriptor table cache.
///
/// Keeps ALL group descriptors in memory (they're small: ~64 bytes each).
/// A 5 GB volume with 128 MB groups has ~40 entries = ~2.5 KB.
///
/// No LRU eviction needed — the entire table fits in memory.
pub struct GdtCache {
    entries: Vec<CachedGd>,
    stats: CacheStats,
}

impl GdtCache {
    /// Create a GDT cache from an already-loaded vector of group descriptors.
    pub fn from_vec(gdt: Vec<GroupDescriptor>) -> Self {
        let entries = gdt
            .into_iter()
            .map(|gd| CachedGd { gd, dirty: false })
            .collect();
        Self {
            entries,
            stats: CacheStats::default(),
        }
    }

    /// Create an empty GDT cache (for legacy/non-group volumes).
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            stats: CacheStats::default(),
        }
    }

    /// Get a group descriptor by index.
    pub fn get(&mut self, group_idx: u32) -> Option<&GroupDescriptor> {
        let idx = group_idx as usize;
        if idx >= self.entries.len() {
            return None;
        }
        self.stats.hits += 1;
        Some(&self.entries[idx].gd)
    }

    /// Get a group descriptor by index (immutable, no stats tracking).
    pub fn get_ref(&self, group_idx: u32) -> Option<&GroupDescriptor> {
        self.entries.get(group_idx as usize).map(|e| &e.gd)
    }

    /// Update a group descriptor and mark it dirty.
    pub fn put(&mut self, group_idx: u32, gd: GroupDescriptor) {
        let idx = group_idx as usize;
        if idx < self.entries.len() {
            self.entries[idx].gd = gd;
            self.entries[idx].dirty = true;
        }
    }

    /// Modify a group descriptor in-place and mark it dirty.
    pub fn modify<F>(&mut self, group_idx: u32, f: F) -> bool
    where
        F: FnOnce(&mut GroupDescriptor),
    {
        let idx = group_idx as usize;
        if idx < self.entries.len() {
            f(&mut self.entries[idx].gd);
            self.entries[idx].dirty = true;
            true
        } else {
            false
        }
    }

    /// Flush all dirty descriptors. Returns `(group_idx, descriptor)` pairs.
    /// After calling, all entries are marked clean.
    pub fn flush_dirty(&mut self) -> Vec<(u32, GroupDescriptor)> {
        let mut dirty = Vec::new();
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.dirty {
                dirty.push((i as u32, entry.gd.clone()));
                entry.dirty = false;
                self.stats.dirty_writebacks += 1;
            }
        }
        dirty
    }

    /// Mark all entries as clean (after a full GDT write).
    pub fn mark_all_clean(&mut self) {
        for entry in &mut self.entries {
            entry.dirty = false;
        }
    }

    /// Get total free blocks across all groups.
    pub fn total_free_blocks(&self) -> u64 {
        self.entries.iter().map(|e| e.gd.bg_free_blocks as u64).sum()
    }

    /// Get total free inodes across all groups.
    pub fn total_free_inodes(&self) -> u64 {
        self.entries.iter().map(|e| e.gd.bg_free_inodes as u64).sum()
    }

    /// Export the current state as a plain `Vec<GroupDescriptor>`.
    /// This is used when functions need `&[GroupDescriptor]` or `&mut [GroupDescriptor]`.
    pub fn as_slice(&self) -> Vec<GroupDescriptor> {
        self.entries.iter().map(|e| e.gd.clone()).collect()
    }

    /// Provide read-only slice access for functions that need `&[GroupDescriptor]`.
    pub fn descriptors(&self) -> Vec<&GroupDescriptor> {
        self.entries.iter().map(|e| &e.gd).collect()
    }

    /// Number of groups.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume::inode::Inode;

    // -----------------------------------------------------------------------
    // InodeCache tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_inode_cache_hit() {
        let mut cache = InodeCache::new(16);
        let inode = Inode::new_file();
        cache.insert(42, inode.clone());
        let got = cache.get(42).unwrap();
        assert_eq!(got, &inode);
        assert_eq!(cache.stats().hits, 1);
        assert_eq!(cache.stats().misses, 0);
    }

    #[test]
    fn test_inode_cache_miss() {
        let mut cache = InodeCache::new(16);
        assert!(cache.get(99).is_none());
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn test_inode_cache_eviction_clean() {
        let mut cache = InodeCache::new(2);
        cache.insert(1, Inode::new_file());
        cache.insert(2, Inode::new_file());
        // Third insert should evict LRU (key=1), but it's clean so no writeback
        let evicted = cache.insert(3, Inode::new_file());
        assert!(evicted.is_none());
        assert_eq!(cache.stats().evictions, 1);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_inode_cache_eviction_dirty() {
        let mut cache = InodeCache::new(2);
        cache.put_dirty(1, Inode::new_file());
        cache.insert(2, Inode::new_file());
        // Evicts key=1 (dirty) → should return for writeback
        let evicted = cache.insert(3, Inode::new_file());
        assert!(evicted.is_some());
        assert_eq!(evicted.unwrap().0, 1);
    }

    #[test]
    fn test_inode_cache_put_dirty() {
        let mut cache = InodeCache::new(16);
        cache.put_dirty(5, Inode::new_dir());
        let dirty = cache.flush_dirty();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].0, 5);
    }

    #[test]
    fn test_inode_cache_flush_dirty() {
        let mut cache = InodeCache::new(16);
        cache.put_dirty(1, Inode::new_file());
        cache.put_dirty(2, Inode::new_dir());
        cache.insert(3, Inode::new_file()); // clean

        let dirty = cache.flush_dirty();
        assert_eq!(dirty.len(), 2);

        // After flush, all should be clean
        let dirty2 = cache.flush_dirty();
        assert_eq!(dirty2.len(), 0);
    }

    #[test]
    fn test_inode_cache_invalidate_dirty() {
        let mut cache = InodeCache::new(16);
        cache.put_dirty(10, Inode::new_file());
        let evicted = cache.invalidate(10);
        assert!(evicted.is_some());
        assert!(cache.is_empty());
    }

    #[test]
    fn test_inode_cache_invalidate_clean() {
        let mut cache = InodeCache::new(16);
        cache.insert(10, Inode::new_file());
        let evicted = cache.invalidate(10);
        assert!(evicted.is_none()); // clean → no writeback needed
    }

    #[test]
    fn test_inode_cache_clear() {
        let mut cache = InodeCache::new(16);
        cache.put_dirty(1, Inode::new_file());
        cache.put_dirty(2, Inode::new_file());
        cache.insert(3, Inode::new_file());

        let dirty = cache.clear();
        assert_eq!(dirty.len(), 2);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_inode_cache_hit_rate() {
        let mut cache = InodeCache::new(16);
        cache.insert(1, Inode::new_file());
        // 3 hits
        cache.get(1);
        cache.get(1);
        cache.get(1);
        // 1 miss
        cache.get(99);

        let rate = cache.stats().hit_rate();
        assert!((rate - 0.75).abs() < 0.001);
    }

    #[test]
    #[should_panic(expected = "inode cache capacity must be > 0")]
    fn test_inode_cache_capacity_zero() {
        InodeCache::new(0);
    }

    #[test]
    fn test_inode_cache_lru_order() {
        let mut cache = InodeCache::new(2);
        cache.put_dirty(1, Inode::new_file()); // LRU=1
        cache.put_dirty(2, Inode::new_file()); // LRU=1, MRU=2
        // Access key 1 to make it MRU: LRU=2, MRU=1
        cache.get(1);
        // Insert 3 → evicts LRU which is now key 2
        let evicted = cache.insert(3, Inode::new_file());
        assert!(evicted.is_some());
        assert_eq!(evicted.unwrap().0, 2);
    }

    // -----------------------------------------------------------------------
    // BlockCache tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_block_cache_hit() {
        let mut cache = BlockCache::new(16, 4096);
        let data = vec![0xABu8; 4096];
        cache.insert(100, data.clone());
        let got = cache.get(100).unwrap();
        assert_eq!(got, &data[..]);
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn test_block_cache_miss() {
        let mut cache = BlockCache::new(16, 4096);
        assert!(cache.get(99).is_none());
        assert_eq!(cache.stats().misses, 1);
    }

    #[test]
    fn test_block_cache_eviction_dirty() {
        let mut cache = BlockCache::new(2, 512);
        cache.put_dirty(1, vec![1u8; 512]);
        cache.insert(2, vec![2u8; 512]);
        let evicted = cache.insert(3, vec![3u8; 512]);
        assert!(evicted.is_some());
        assert_eq!(evicted.unwrap().0, 1);
    }

    #[test]
    fn test_block_cache_modify_in_place() {
        let mut cache = BlockCache::new(16, 4096);
        cache.insert(10, vec![0u8; 4096]);
        let modified = cache.modify(10, |data| {
            data[0] = 0xFF;
            data[1] = 0xEE;
        });
        assert!(modified);

        let got = cache.get(10).unwrap();
        assert_eq!(got[0], 0xFF);
        assert_eq!(got[1], 0xEE);
    }

    #[test]
    fn test_block_cache_invalidate_range() {
        let mut cache = BlockCache::new(16, 512);
        for addr in 0..10u64 {
            cache.put_dirty(addr, vec![addr as u8; 512]);
        }
        let dirty = cache.invalidate_range(3, 5); // blocks 3..8
        assert_eq!(dirty.len(), 5);
        assert_eq!(cache.len(), 5);
    }

    #[test]
    fn test_block_cache_flush_dirty() {
        let mut cache = BlockCache::new(16, 512);
        cache.put_dirty(1, vec![1u8; 512]);
        cache.put_dirty(2, vec![2u8; 512]);
        cache.insert(3, vec![3u8; 512]); // clean

        let dirty = cache.flush_dirty();
        assert_eq!(dirty.len(), 2);

        // After flush all clean
        let dirty2 = cache.flush_dirty();
        assert_eq!(dirty2.len(), 0);
    }

    #[test]
    fn test_block_cache_memory_usage() {
        let cache = BlockCache::new(256, 4096);
        // Empty cache should have 0 usage
        assert_eq!(cache.memory_usage(), 0);
    }

    // -----------------------------------------------------------------------
    // GdtCache tests
    // -----------------------------------------------------------------------

    fn make_gd(free_blocks: u32, free_inodes: u32) -> GroupDescriptor {
        let mut gd = GroupDescriptor::new(0, 0, 0, 0, 0, free_blocks, false);
        gd.bg_free_inodes = free_inodes;
        gd
    }

    #[test]
    fn test_gdt_cache_from_vec() {
        let gdt = vec![make_gd(100, 50), make_gd(200, 80), make_gd(50, 20)];
        let cache = GdtCache::from_vec(gdt);
        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn test_gdt_cache_get() {
        let gdt = vec![make_gd(100, 50), make_gd(200, 80)];
        let mut cache = GdtCache::from_vec(gdt);
        let gd = cache.get(0).unwrap();
        assert_eq!(gd.bg_free_blocks, 100);
        assert_eq!(cache.stats().hits, 1);
    }

    #[test]
    fn test_gdt_cache_get_out_of_range() {
        let mut cache = GdtCache::from_vec(vec![make_gd(100, 50)]);
        assert!(cache.get(99).is_none());
    }

    #[test]
    fn test_gdt_cache_put_dirty() {
        let mut cache = GdtCache::from_vec(vec![make_gd(100, 50)]);
        let mut gd = cache.get(0).unwrap().clone();
        gd.bg_free_blocks = 99;
        cache.put(0, gd);

        let dirty = cache.flush_dirty();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].1.bg_free_blocks, 99);
    }

    #[test]
    fn test_gdt_cache_modify() {
        let mut cache = GdtCache::from_vec(vec![make_gd(100, 50)]);
        let modified = cache.modify(0, |gd| gd.bg_free_blocks -= 1);
        assert!(modified);

        let dirty = cache.flush_dirty();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].1.bg_free_blocks, 99);
    }

    #[test]
    fn test_gdt_cache_total_free_blocks() {
        let cache = GdtCache::from_vec(vec![make_gd(100, 50), make_gd(200, 80), make_gd(50, 20)]);
        assert_eq!(cache.total_free_blocks(), 350);
    }

    #[test]
    fn test_gdt_cache_total_free_inodes() {
        let cache = GdtCache::from_vec(vec![make_gd(100, 50), make_gd(200, 80), make_gd(50, 20)]);
        assert_eq!(cache.total_free_inodes(), 150);
    }

    #[test]
    fn test_gdt_cache_mark_all_clean() {
        let mut cache = GdtCache::from_vec(vec![make_gd(100, 50), make_gd(200, 80)]);
        cache.modify(0, |gd| gd.bg_free_blocks = 99);
        cache.modify(1, |gd| gd.bg_free_blocks = 199);
        cache.mark_all_clean();

        let dirty = cache.flush_dirty();
        assert_eq!(dirty.len(), 0);
    }

    #[test]
    fn test_gdt_cache_empty() {
        let cache = GdtCache::empty();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }
}
