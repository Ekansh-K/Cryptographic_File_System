pub mod superblock;
pub mod inode;
pub mod bitmap;
pub mod alloc;
pub mod dir;
pub mod file_io;
pub mod path;
pub mod group;
pub mod extent;
pub mod journal;
pub mod htree;
pub mod xattr;
pub mod lock;
pub mod delayed_alloc;
pub mod cache;

pub use superblock::Superblock;
pub use inode::{Inode, InodeTable};
pub use bitmap::Bitmap;
pub use alloc::BlockAlloc;
pub use lock::FileLockManager;

use crate::block_device::CFSBlockDevice;
use anyhow::{bail, Result};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const CFS_MAGIC: [u8; 4] = *b"CFS1";
pub const CFS_VERSION: u32 = 2;
pub const CFS_VERSION_V1: u32 = 1;
pub const CFS_VERSION_V3: u32 = 3;
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;
pub const INODE_SIZE: u32 = 128;
pub const INODE_SIZE_V3: u32 = 256;
pub const MAX_INODE_COUNT: u32 = 65536;
pub const ROOT_INODE: u32 = 0;

/// Fileâ€type constants stored in `Inode.mode`.
pub const INODE_UNUSED: u16 = 0;
pub const INODE_FILE: u16 = 1;
pub const INODE_DIR: u16 = 2;
pub const INODE_SYMLINK: u16 = 3;

// ---------------------------------------------------------------------------
// Helper: ceiling division
// ---------------------------------------------------------------------------

pub(crate) fn ceil_div(a: u64, b: u64) -> u64 {
    (a + b - 1) / b
}

pub(crate) fn round_up_to(value: u64, multiple: u64) -> u64 {
    if multiple == 0 {
        return value;
    }
    ((value + multiple - 1) / multiple) * multiple
}

// ---------------------------------------------------------------------------
// FormatOptions / MountOptions â€” 10A.1
// ---------------------------------------------------------------------------

/// How access time (atime) is updated on reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtimeMode {
    /// Update atime on every read.
    Always,
    /// Update atime only if atime <= mtime or atime is >24h old.
    Relatime,
    /// Never update atime (best performance).
    Never,
}

/// What to do when a checksum or I/O error is detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorBehavior {
    /// Log the error and continue operating.
    Continue,
    /// Switch the volume to read-only mode.
    ReadOnly,
}

/// Parameters set at format time. Immutable once the volume is created.
#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Block allocation unit in bytes. Must be power of 2, 512..=65536.
    pub block_size: u32,
    /// Inode on-disk size in bytes. 128 (v2 compat) or 256 (v3 full features).
    pub inode_size: u32,
    /// Bytes-per-inode ratio. Lower = more inodes. Range: 1024..=65536.
    pub inode_ratio: u32,
    /// Journal region as percentage of total volume. 0.0 = disabled. Range: 0.0 | 0.5..=5.0.
    pub journal_percent: f32,
    /// Human-readable label. Max 31 bytes UTF-8 (32 with null terminator).
    pub volume_label: String,
    /// Whether to zero freed data blocks by default.
    pub secure_delete: bool,
    /// Default permissions for new files (lower 12 bits: rwxrwxrwx + setuid/setgid/sticky).
    pub default_permissions: u32,
    /// Behavior on checksum or I/O errors.
    pub error_behavior: ErrorBehavior,
    /// Blocks per block group. Must be <= block_size * 8. Default = block_size * 8.
    pub blocks_per_group: u32,
}

/// Parameters set per-session when mounting. Can change between mounts.
#[derive(Debug, Clone)]
pub struct MountOptions {
    /// LRU inode cache capacity. 0 = no caching.
    pub cache_inodes: u32,
    /// LRU block cache capacity. 0 = no caching.
    pub cache_blocks: u32,
    /// Override format-level secure delete setting for this session.
    pub secure_delete: bool,
    /// Access time update strategy.
    pub atime_mode: AtimeMode,
    /// Mount as read-only (no writes allowed).
    pub read_only: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            block_size: 4096,
            inode_size: 256,
            inode_ratio: 16384,
            journal_percent: 1.0,
            volume_label: String::new(),
            secure_delete: true,
            default_permissions: 0o755,
            error_behavior: ErrorBehavior::Continue,
            blocks_per_group: 4096 * 8, // 32768
        }
    }
}

impl Default for MountOptions {
    fn default() -> Self {
        Self {
            cache_inodes: 256,
            cache_blocks: 512,
            secure_delete: true,
            atime_mode: AtimeMode::Relatime,
            read_only: false,
        }
    }
}

impl FormatOptions {
    /// General Purpose (default).
    pub fn general_purpose() -> Self {
        Self::default()
    }

    /// Optimized for few large files (video, archives, multi-TB).
    pub fn large_files() -> Self {
        Self {
            block_size: 16384,
            inode_size: 256,
            inode_ratio: 65536,
            journal_percent: 0.5,
            volume_label: String::new(),
            secure_delete: true,
            default_permissions: 0o755,
            error_behavior: ErrorBehavior::Continue,
            blocks_per_group: 16384 * 8,
        }
    }

    /// Optimized for many small files (source code, configs).
    pub fn small_files() -> Self {
        Self {
            block_size: 4096,
            inode_size: 256,
            inode_ratio: 4096,
            journal_percent: 2.0,
            volume_label: String::new(),
            secure_delete: true,
            default_permissions: 0o755,
            error_behavior: ErrorBehavior::Continue,
            blocks_per_group: 4096 * 8,
        }
    }

    /// Maximum security â€” all integrity features on.
    pub fn max_security() -> Self {
        Self {
            block_size: 4096,
            inode_size: 256,
            inode_ratio: 16384,
            journal_percent: 2.0,
            volume_label: String::new(),
            secure_delete: true,
            default_permissions: 0o755,
            error_behavior: ErrorBehavior::ReadOnly,
            blocks_per_group: 4096 * 8,
        }
    }

    /// Minimal legacy â€” v2-compatible, smallest overhead.
    pub fn minimal_legacy() -> Self {
        Self {
            block_size: 4096,
            inode_size: 128,
            inode_ratio: 16384,
            journal_percent: 0.0,
            volume_label: String::new(),
            secure_delete: false,
            default_permissions: 0o755,
            error_behavior: ErrorBehavior::Continue,
            blocks_per_group: 4096 * 8,
        }
    }

    /// Validate all format option fields.
    pub fn validate(&self) -> Result<()> {
        if self.block_size < 512 || self.block_size > 65536 || !self.block_size.is_power_of_two() {
            bail!("block_size must be power of 2 in range 512..=65536, got {}", self.block_size);
        }
        if self.inode_size != 128 && self.inode_size != 256 {
            bail!("inode_size must be 128 or 256, got {}", self.inode_size);
        }
        if self.inode_size > self.block_size {
            bail!("inode_size ({}) must be <= block_size ({})", self.inode_size, self.block_size);
        }
        if self.inode_ratio < 1024 || self.inode_ratio > 65536 {
            bail!("inode_ratio must be in range 1024..=65536, got {}", self.inode_ratio);
        }
        if self.journal_percent != 0.0 && (self.journal_percent < 0.5 || self.journal_percent > 5.0) {
            bail!("journal_percent must be 0.0 or 0.5..=5.0, got {}", self.journal_percent);
        }
        if self.volume_label.len() > 31 {
            bail!("volume_label max 31 bytes, got {}", self.volume_label.len());
        }
        if self.blocks_per_group == 0 || self.blocks_per_group > self.block_size * 8 {
            bail!(
                "blocks_per_group must be in 1..={}, got {}",
                self.block_size * 8,
                self.blocks_per_group
            );
        }
        if self.default_permissions > 0o7777 {
            bail!("default_permissions must be <= 0o7777, got 0o{:o}", self.default_permissions);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// CFSVolume
// ---------------------------------------------------------------------------

pub struct CFSVolume {
    // --- Locked fields (interior mutability) ---

    /// Block device â€” Mutex because read/write need &mut.
    /// Lock order: 7 (acquired last, held briefly).
    dev: Mutex<Box<dyn CFSBlockDevice>>,

    /// Superblock â€” RwLock because reads are frequent, writes are rare.
    /// Lock order: 1 (acquired first).
    superblock: RwLock<Superblock>,

    /// Data block bitmap â€” Mutex (legacy volumes only).
    /// Lock order: 3.
    bitmap: Mutex<Bitmap>,

    /// Inode bitmap (v2 only). `None` for v1 volumes.
    /// Lock order: 3 (same level as bitmap â€” never hold both).
    inode_bitmap: Mutex<Option<Bitmap>>,

    /// Group descriptor table.
    /// Lock order: 3.
    gdt: Mutex<Vec<group::GroupDescriptor>>,

    /// Per-group block bitmap manager (None for legacy volumes).
    /// Lock order: 3.
    group_bitmap_mgr: Mutex<Option<group::GroupBitmapManager>>,

    /// Per-group inode bitmap manager (None for legacy volumes).
    /// Lock order: 3.
    group_inode_bitmap_mgr: Mutex<Option<group::GroupInodeBitmapManager>>,

    /// In-memory journal manager (None when journal feature is disabled).
    /// Lock order: 2.
    journal: Mutex<Option<journal::Journal>>,

    /// Byte-range file lock manager.
    /// Lock order: 6.
    lock_manager: Mutex<FileLockManager>,

    /// Delayed allocation manager (None when FEATURE_DELAYED_ALLOC not set).
    /// Lock order: 4.
    delayed_alloc: Mutex<Option<delayed_alloc::DelayedAllocManager>>,

    /// LRU inode cache (None = bypass/disabled). Lock order: 5.
    inode_cache: Mutex<Option<cache::InodeCache>>,

    /// LRU metadata block cache (None = bypass/disabled). Lock order: 5.
    block_cache: Mutex<Option<cache::BlockCache>>,

    // --- Immutable after construction (no lock needed) ---

    /// Inode table helper â€” stateless config, actual I/O goes through dev.
    pub inode_table: InodeTable,

    /// Per-group inode table (None for legacy volumes) â€” stateless config.
    pub(crate) group_inode_table: Option<group::GroupInodeTable>,

    /// Inode size on disk (128 for v2, 128 or 256 for v3).
    pub(crate) inode_size: u32,

    /// Mount-time options for this session.
    pub(crate) mount_options: MountOptions,

    /// HMAC key for metadata integrity verification.
    pub(crate) hmac_key: [u8; 32],

    /// Block size in bytes (cached from superblock â€” immutable after mount).
    pub(crate) block_size: u32,

    /// Data region start block (cached â€” immutable after mount). Legacy only.
    pub(crate) data_start: u64,

    /// Whether this volume uses block groups (cached from gdt).
    pub(crate) has_groups: bool,
}

impl CFSVolume {
    // -------------------------------------------------------------------
    // Lock-acquiring helpers (10H.1)
    // -------------------------------------------------------------------

    /// Acquire the device lock for I/O.
    #[inline]
    fn dev(&self) -> MutexGuard<'_, Box<dyn CFSBlockDevice>> {
        self.dev.lock().expect("dev lock poisoned")
    }

    /// Acquire the superblock for reading.
    #[inline]
    fn sb_read(&self) -> RwLockReadGuard<'_, Superblock> {
        self.superblock.read().expect("superblock read lock poisoned")
    }

    /// Acquire the superblock for writing.
    #[inline]
    fn sb_write(&self) -> RwLockWriteGuard<'_, Superblock> {
        self.superblock.write().expect("superblock write lock poisoned")
    }

    /// Acquire the bitmap lock.
    #[inline]
    fn bitmap_lock(&self) -> MutexGuard<'_, Bitmap> {
        self.bitmap.lock().expect("bitmap lock poisoned")
    }

    /// Acquire the inode bitmap lock.
    #[inline]
    fn inode_bitmap_lock(&self) -> MutexGuard<'_, Option<Bitmap>> {
        self.inode_bitmap.lock().expect("inode_bitmap lock poisoned")
    }

    /// Acquire the GDT lock.
    #[inline]
    fn gdt_lock(&self) -> MutexGuard<'_, Vec<group::GroupDescriptor>> {
        self.gdt.lock().expect("gdt lock poisoned")
    }

    /// Acquire the group bitmap manager lock.
    #[inline]
    fn gbm_lock(&self) -> MutexGuard<'_, Option<group::GroupBitmapManager>> {
        self.group_bitmap_mgr.lock().expect("group_bitmap_mgr lock poisoned")
    }

    /// Acquire the group inode bitmap manager lock.
    #[inline]
    fn gibm_lock(&self) -> MutexGuard<'_, Option<group::GroupInodeBitmapManager>> {
        self.group_inode_bitmap_mgr.lock().expect("group_inode_bitmap_mgr lock poisoned")
    }

    /// Acquire the journal lock.
    #[inline]
    fn journal_lock(&self) -> MutexGuard<'_, Option<journal::Journal>> {
        self.journal.lock().expect("journal lock poisoned")
    }

    /// Acquire the inode cache lock.
    #[inline]
    fn inode_cache_lock(&self) -> MutexGuard<'_, Option<cache::InodeCache>> {
        self.inode_cache.lock().expect("inode_cache lock poisoned")
    }

    /// Acquire the block cache lock.
    #[inline]
    fn block_cache_lock(&self) -> MutexGuard<'_, Option<cache::BlockCache>> {
        self.block_cache.lock().expect("block_cache lock poisoned")
    }

    /// Acquire the lock manager.
    #[inline]
    pub fn lock_mgr(&self) -> MutexGuard<'_, FileLockManager> {
        self.lock_manager.lock().expect("lock_manager lock poisoned")
    }

    // -------------------------------------------------------------------
    // Public accessors for locked fields
    // -------------------------------------------------------------------

    /// Read-only borrow of the superblock (acquires read lock).
    pub fn superblock(&self) -> RwLockReadGuard<'_, Superblock> {
        self.sb_read()
    }

    /// Whether this volume uses block groups (v3 with group_count > 0).
    pub fn has_block_groups(&self) -> bool {
        self.has_groups
    }

    /// Whether this volume is read-only.
    pub fn is_read_only(&self) -> bool {
        self.mount_options.read_only
    }

    /// Guard: returns error if volume is read-only.
    fn check_writable(&self) -> Result<()> {
        if self.mount_options.read_only {
            bail!("volume is read-only");
        }
        Ok(())
    }

    /// Whether secure deletion should be used based on mount options.
    fn should_secure_delete(&self) -> bool {
        self.mount_options.secure_delete
    }

    // -----------------------------------------------------------------------
    // Journal helpers (10E)
    // -----------------------------------------------------------------------

    /// Whether the journal is active on this volume.
    pub fn has_journal(&self) -> bool {
        self.journal_lock().is_some()
    }

    /// Compute the physical disk block that contains a given inode index.
    fn inode_disk_block(&self, inode_idx: u32) -> Result<u64> {
        let bs = self.block_size as u64;
        let isz = self.inode_size as u64;
        if self.has_groups {
            let git = self.group_inode_table.as_ref()
                .ok_or_else(|| anyhow::anyhow!("no group inode table"))?;
            let ipg = git.inodes_per_group;
            let group = inode_idx / ipg;
            let local = inode_idx % ipg;
            let gdt = self.gdt_lock();
            let g_desc = gdt.get(group as usize)
                .ok_or_else(|| anyhow::anyhow!("group {} out of range", group))?;
            let byte_off = local as u64 * isz;
            Ok(g_desc.bg_inode_table + byte_off / bs)
        } else {
            let sb = self.sb_read();
            let byte_off = inode_idx as u64 * isz;
            Ok(sb.inode_table_start + byte_off / bs)
        }
    }

    /// Journal a metadata block BEFORE modifying it.
    /// No-op if journal is disabled.
    fn journal_metadata_block(&self, txn_id: u64, block: u64) -> Result<()> {
        let mut jnl = self.journal_lock();
        if let Some(ref mut j) = *jnl {
            let mut dev = self.dev();
            j.journal_block(&mut **dev, txn_id, block)?;
        }
        Ok(())
    }

    /// Begin a journal transaction. Returns the txn_id (or 0 if no journal).
    fn journal_begin(&self) -> Result<u64> {
        let mut jnl = self.journal_lock();
        if let Some(ref mut j) = *jnl {
            let mut dev = self.dev();
            j.begin_txn(&mut **dev)
        } else {
            Ok(0)
        }
    }

    /// Commit a journal transaction. Runs checkpoint every 16 transactions.
    fn journal_commit(&self, txn_id: u64) -> Result<()> {
        let mut jnl = self.journal_lock();
        if let Some(ref mut j) = *jnl {
            let mut dev = self.dev();
            j.commit_txn(&mut **dev, txn_id)?;
            let used = j.used_entries();
            let cap = j.jsb.capacity;
            if cap > 0 && (used * 4 > cap * 3 || j.jsb.sequence % 16 == 0) {
                j.checkpoint(&mut **dev)?;
            }
        }
        Ok(())
    }

    /// Abort a journal transaction.
    fn journal_abort(&self, txn_id: u64) -> Result<()> {
        let mut jnl = self.journal_lock();
        if let Some(ref mut j) = *jnl {
            let mut dev = self.dev();
            j.abort_txn(&mut **dev, txn_id)?;
        }
        Ok(())
    }

    /// Return a displayable journal status (None if no journal).
    pub fn journal_status(&self) -> Option<journal::JournalStatus> {
        self.journal_lock().as_ref().map(|j| j.status())
    }

    // -----------------------------------------------------------------------
    // Orphan list helpers (10E)
    // -----------------------------------------------------------------------

    /// Add an inode to the head of the orphan list.
    fn orphan_add(&self, inode_idx: u32) -> Result<()> {
        let mut inode = self.read_inode(inode_idx)?;
        let first_orphan = self.sb_read().first_orphan_inode;
        inode.set_next_orphan(first_orphan);
        self.write_inode(inode_idx, &inode)?;
        self.sb_write().first_orphan_inode = inode_idx;
        Ok(())
    }

    /// Remove an inode from the orphan list.
    fn orphan_remove(&self, inode_idx: u32) -> Result<()> {
        let first_orphan = self.sb_read().first_orphan_inode;
        if first_orphan == inode_idx {
            let inode = self.read_inode(inode_idx)?;
            self.sb_write().first_orphan_inode = inode.get_next_orphan();
            let mut inode = inode;
            inode.clear_orphan();
            self.write_inode(inode_idx, &inode)?;
        } else {
            let mut prev = first_orphan;
            while prev != 0 {
                let prev_inode = self.read_inode(prev)?;
                let next = prev_inode.get_next_orphan();
                if next == inode_idx {
                    let target = self.read_inode(inode_idx)?;
                    let target_next = target.get_next_orphan();
                    let mut prev_inode = prev_inode;
                    prev_inode.set_next_orphan(target_next);
                    self.write_inode(prev, &prev_inode)?;
                    let mut target = target;
                    target.clear_orphan();
                    self.write_inode(inode_idx, &target)?;
                    return Ok(());
                }
                prev = next;
            }
            let mut inode = self.read_inode(inode_idx)?;
            inode.clear_orphan();
            self.write_inode(inode_idx, &inode)?;
        }
        Ok(())
    }

    /// Walk the orphan list and free any leaked inodes+blocks (called on mount).
    /// NOTE: Called during mount before the volume is shared â€” single-threaded context.
    fn orphan_cleanup(&self) -> Result<u32> {
        let mut cleaned = 0u32;
        let secure = self.should_secure_delete();
        loop {
            let first = self.sb_read().first_orphan_inode;
            if first == 0 {
                break;
            }
            let idx = first;
            let mut inode = self.read_inode(idx)?;
            let next = inode.get_next_orphan();

            let bs = self.block_size;
            if self.has_groups {
                let mut sb = self.sb_write();     // level 1
                let mut gbm = self.gbm_lock();    // level 3
                let mut gdt = self.gdt_lock();    // level 3
                let mut dev = self.dev();         // level 7
                let mut alloc = BlockAlloc::Group {
                    gbm: gbm.as_mut().unwrap(),
                    gdt: &mut *gdt,
                };
                let _ = file_io::free_all_blocks(&mut **dev, &mut alloc, &mut *sb, &mut inode, bs, secure);
            } else {
                let mut sb = self.sb_write();     // level 1
                let mut bm = self.bitmap_lock();  // level 3
                let mut dev = self.dev();         // level 7
                let ds = sb.data_start;
                let mut alloc = BlockAlloc::Legacy { bitmap: &mut *bm, data_start: ds };
                let _ = file_io::free_all_blocks(&mut **dev, &mut alloc, &mut *sb, &mut inode, bs, secure);
            }

            inode.clear_orphan();
            self.free_inode(idx)?;
            self.sb_write().first_orphan_inode = next;
            cleaned += 1;
        }
        Ok(cleaned)
    }

    /// Create a fresh CFS filesystem on `dev`.
    pub fn format(mut dev: Box<dyn CFSBlockDevice>, block_size: u32) -> Result<Self> {
        let mut sb = Superblock::new(dev.size(), block_size)?;
        sb.write_to(&mut *dev)?;

        let bs = block_size as u64;
        let zero_block = vec![0u8; block_size as usize];

        // Zero inode table blocks
        let inode_table_blocks = ceil_div(
            sb.inode_count as u64 * INODE_SIZE as u64,
            bs,
        );
        for b in sb.inode_table_start..sb.inode_table_start + inode_table_blocks {
            dev.write(b * bs, &zero_block)?;
        }

        // Zero inode bitmap blocks and initialize
        let inode_bitmap_blocks = ceil_div(ceil_div(sb.inode_count as u64, 8), bs);
        for b in sb.inode_bitmap_start..sb.inode_bitmap_start + inode_bitmap_blocks {
            dev.write(b * bs, &zero_block)?;
        }

        // Zero data bitmap blocks
        let data_block_count = sb.data_block_count();
        let bitmap_blocks = ceil_div(ceil_div(data_block_count, 8), bs);
        for b in sb.bitmap_start..sb.bitmap_start + bitmap_blocks {
            dev.write(b * bs, &zero_block)?;
        }

        // Create root directory inode (inode 0)
        let inode_table =
            InodeTable::new(sb.inode_table_start, sb.inode_count, block_size);
        let root = Inode::new_dir();
        inode_table.write_inode(&mut *dev, 0, &root)?;

        // Build in-memory data bitmap (all free)
        let mut bitmap = Bitmap::new_empty(
            data_block_count,
            sb.bitmap_start,
            bitmap_blocks,
            block_size,
        );

        // Build in-memory inode bitmap â€” bit 0 set (root inode allocated)
        let mut inode_bitmap = Bitmap::new_empty(
            sb.inode_count as u64,
            sb.inode_bitmap_start,
            inode_bitmap_blocks,
            block_size,
        );
        inode_bitmap.alloc(); // allocates bit 0 = root inode

        // Initialize root directory with "." and ".." entries using new API
        let mut root_inode = inode_table.read_inode(&mut *dev, ROOT_INODE)?;
        let ds = sb.data_start;
        let mut alloc = BlockAlloc::Legacy { bitmap: &mut bitmap, data_start: ds };
        dir::init_dir_block(&mut *dev, &mut root_inode, &mut alloc, &mut sb, ROOT_INODE, ROOT_INODE)?;
        inode_table.write_inode(&mut *dev, ROOT_INODE, &root_inode)?;

        // Persist updated superblock (free_blocks changed), bitmaps, and backup
        sb.write_to(&mut *dev)?;
        bitmap.save(&mut *dev)?;
        inode_bitmap.save(&mut *dev)?;
        dev.flush()?;

        Ok(CFSVolume {
            block_size,
            data_start: sb.data_start,
            has_groups: false,
            dev: Mutex::new(dev),
            superblock: RwLock::new(sb),
            bitmap: Mutex::new(bitmap),
            inode_table,
            inode_bitmap: Mutex::new(Some(inode_bitmap)),
            inode_size: INODE_SIZE,
            mount_options: MountOptions::default(),
            gdt: Mutex::new(Vec::new()),
            group_bitmap_mgr: Mutex::new(None),
            group_inode_bitmap_mgr: Mutex::new(None),
            group_inode_table: None,
            hmac_key: superblock::derive_hmac_key(None),
            journal: Mutex::new(None),
            lock_manager: Mutex::new(FileLockManager::new()),
            delayed_alloc: Mutex::new(None),
            inode_cache: Mutex::new(None),
            block_cache: Mutex::new(None),
        })
    }

    /// Create a v3 filesystem with full format options.
    pub fn format_v3(mut dev: Box<dyn CFSBlockDevice>, opts: &FormatOptions) -> Result<Self> {
        opts.validate()?;
        let mut sb = Superblock::new_v3(dev.size(), opts)?;
        sb.write_to(&mut *dev)?;

        let bs = sb.block_size as u64;
        let zero_block = vec![0u8; sb.block_size as usize];

        if sb.group_count > 0 {
            // --- Block-group layout ---
            let layout = group::compute_group_layout(
                sb.total_blocks,
                sb.block_size,
                sb.inode_size,
                sb.inode_ratio,
                sb.blocks_per_group,
                sb.journal_blocks,
            )?;

            // Build initial GDT
            let mut gdt: Vec<group::GroupDescriptor> = (0..sb.group_count)
                .map(|g| {
                    let block_bitmap = layout.group_block_bitmap(g);
                    let inode_bitmap = layout.group_inode_bitmap(g);
                    let inode_table = layout.group_inode_table(g);
                    let data_blocks = layout.group_data_block_count(g);
                    let total_blocks = layout.group_block_count(g) as u64;
                    group::GroupDescriptor::new(
                        block_bitmap,
                        inode_bitmap,
                        inode_table,
                        total_blocks,
                        sb.inodes_per_group,
                        data_blocks,
                        true, // lazy init
                    )
                })
                .collect();

            // Write GDT to disk
            group::write_gdt(&mut *dev, sb.gdt_start, &gdt, sb.block_size)?;

            // Initialize group 0: allocate bitmaps / inode table blocks
            let g0_block_bitmap = layout.group_block_bitmap(0);
            let g0_inode_bitmap = layout.group_inode_bitmap(0);
            let g0_inode_table = layout.group_inode_table(0);
            let g0_inode_table_blocks = ceil_div(
                sb.inodes_per_group as u64 * sb.inode_size as u64, bs,
            );

            // Zero group 0 bitmaps and inode table
            dev.write(g0_block_bitmap * bs, &zero_block)?;
            dev.write(g0_inode_bitmap * bs, &zero_block)?;
            for b in 0..g0_inode_table_blocks {
                dev.write((g0_inode_table + b) * bs, &zero_block)?;
            }

            // Create root dir inode via group inode table
            let git = group::GroupInodeTable::new(
                sb.inodes_per_group, sb.inode_size, sb.block_size, sb.group_count,
            );
            let root = if sb.inode_size == 256 {
                Inode::new_dir_v3(opts.default_permissions)
            } else {
                Inode::new_dir()
            };
            git.write_inode(&mut *dev, ROOT_INODE, &root, &gdt)?;

            // Create group managers
            let overhead_per_group = layout.overhead_per_group;
            let gbm = group::GroupBitmapManager::new(
                sb.block_size,
                sb.blocks_per_group,
                sb.group_count,
                layout.global_overhead,
                overhead_per_group,
            );
            let mut gibm = group::GroupInodeBitmapManager::new(
                sb.block_size, sb.inodes_per_group, sb.group_count,
            );

            // Allocate root inode (inode 0) in inode bitmap
            let root_alloc = gibm.alloc_in_group(&mut *dev, 0, &mut gdt)?;
            if root_alloc.is_none() {
                bail!("failed to allocate root inode in group 0");
            }
            sb.free_inodes = sb.free_inodes.saturating_sub(1);

            // Initialize root dir block using group allocator
            let mut gbm_init = group::GroupBitmapManager::new(
                sb.block_size,
                sb.blocks_per_group,
                sb.group_count,
                layout.global_overhead,
                overhead_per_group,
            );
            {
                let mut root_inode = git.read_inode(&mut *dev, ROOT_INODE, &gdt)?;
                let mut alloc = BlockAlloc::Group { gbm: &mut gbm_init, gdt: &mut gdt };
                dir::init_dir_block(&mut *dev, &mut root_inode, &mut alloc, &mut sb, ROOT_INODE, ROOT_INODE)?;
                git.write_inode(&mut *dev, ROOT_INODE, &root_inode, &gdt)?;
            }

            // Save group 0 bitmaps
            gibm.save_all(&mut *dev, &gdt)?;
            gbm_init.save_all(&mut *dev, &gdt)?;

            // Reupdate GDT descriptors (group 0 block/inode bitmaps are now initialized)
            gdt[0].bg_flags &= !(group::BG_BLOCK_UNINIT | group::BG_INODE_UNINIT);
            gdt[0].bg_checksum = gdt[0].compute_checksum();
            group::write_gdt(&mut *dev, sb.gdt_start, &gdt, sb.block_size)?;

            // Persist superblock
            sb.write_to(&mut *dev)?;
            dev.flush()?;

            // Initialize journal if enabled
            let jnl = if sb.has_journal() {
                journal::Journal::init(
                    &mut *dev,
                    sb.journal_start,
                    sb.journal_blocks,
                    sb.block_size,
                )?;
                Some(journal::Journal::load(
                    &mut *dev,
                    sb.journal_start,
                    sb.journal_blocks,
                    sb.block_size,
                )?)
            } else {
                None
            };

            // Merge the two GBM instances: gbm_init has loaded group 0 bitmap
            // Start fresh with the final gbm (will lazy-load as needed)
            Ok(CFSVolume {
                block_size: sb.block_size,
                data_start: sb.data_start,
                has_groups: true,
                dev: Mutex::new(dev),
                superblock: RwLock::new(sb),
                bitmap: Mutex::new(Bitmap::new_empty(0, 0, 0, bs as u32)),
                inode_table: InodeTable::new(0, 0, bs as u32),
                inode_bitmap: Mutex::new(None),
                inode_size: opts.inode_size,
                mount_options: MountOptions::default(),
                gdt: Mutex::new(gdt),
                group_bitmap_mgr: Mutex::new(Some(gbm)),
                group_inode_bitmap_mgr: Mutex::new(Some(gibm)),
                group_inode_table: Some(git),
                hmac_key: superblock::derive_hmac_key(None),
                journal: Mutex::new(jnl),
                lock_manager: Mutex::new(FileLockManager::new()),
                delayed_alloc: Mutex::new(None),
                inode_cache: Mutex::new(None),
                block_cache: Mutex::new(None),
            })
        } else {
            // --- Single-group / legacy path ---
            let inode_count = sb.inode_count;
            let inode_table = InodeTable::new_with_inode_size(
                sb.inode_table_start, inode_count, sb.block_size, sb.inode_size,
            );

            // Zero inode table blocks
            let inode_table_blocks = ceil_div(inode_count as u64 * sb.inode_size as u64, bs);
            for b in sb.inode_table_start..sb.inode_table_start + inode_table_blocks {
                dev.write(b * bs, &zero_block)?;
            }
            dev.write(sb.inode_bitmap_start * bs, &zero_block)?;
            dev.write(sb.bitmap_start * bs, &zero_block)?;

            let root = if sb.inode_size == 256 {
                Inode::new_dir_v3(opts.default_permissions)
            } else {
                Inode::new_dir()
            };
            inode_table.write_inode(&mut *dev, ROOT_INODE, &root)?;

            let data_block_count = sb.data_block_count();
            let bitmap_blocks = ceil_div(ceil_div(data_block_count, 8), bs);
            let mut bitmap = Bitmap::new_empty(data_block_count, sb.bitmap_start, bitmap_blocks, sb.block_size);

            let inode_bm_blocks = ceil_div(ceil_div(inode_count as u64, 8), bs);
            let mut inode_bitmap = Bitmap::new_empty(inode_count as u64, sb.inode_bitmap_start, inode_bm_blocks, sb.block_size);
            inode_bitmap.alloc(); // root inode

            let mut root_inode = inode_table.read_inode(&mut *dev, ROOT_INODE)?;
            let ds = sb.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut bitmap, data_start: ds };
            dir::init_dir_block(&mut *dev, &mut root_inode, &mut alloc, &mut sb, ROOT_INODE, ROOT_INODE)?;
            inode_table.write_inode(&mut *dev, ROOT_INODE, &root_inode)?;

            sb.write_to(&mut *dev)?;
            if sb.version >= 3 {
                bitmap.save_with_checksums(&mut *dev)?;
                inode_bitmap.save_with_checksums(&mut *dev)?;
            } else {
                bitmap.save(&mut *dev)?;
                inode_bitmap.save(&mut *dev)?;
            }
            dev.flush()?;

            // Initialize journal if enabled
            let jnl = if sb.has_journal() {
                journal::Journal::init(
                    &mut *dev,
                    sb.journal_start,
                    sb.journal_blocks,
                    sb.block_size,
                )?;
                Some(journal::Journal::load(
                    &mut *dev,
                    sb.journal_start,
                    sb.journal_blocks,
                    sb.block_size,
                )?)
            } else {
                None
            };

            Ok(CFSVolume {
                block_size: sb.block_size,
                data_start: sb.data_start,
                has_groups: false,
                dev: Mutex::new(dev),
                superblock: RwLock::new(sb),
                bitmap: Mutex::new(bitmap),
                inode_table,
                inode_bitmap: Mutex::new(Some(inode_bitmap)),
                inode_size: opts.inode_size,
                mount_options: MountOptions::default(),
                gdt: Mutex::new(Vec::new()),
                group_bitmap_mgr: Mutex::new(None),
                group_inode_bitmap_mgr: Mutex::new(None),
                group_inode_table: None,
                hmac_key: superblock::derive_hmac_key(None),
                journal: Mutex::new(jnl),
                lock_manager: Mutex::new(FileLockManager::new()),
                delayed_alloc: Mutex::new(None),
                inode_cache: Mutex::new(None),
                block_cache: Mutex::new(None),
            })
        }
    }

    /// Open an existing CFS filesystem with mount options.
    pub fn mount_v3(mut dev: Box<dyn CFSBlockDevice>, block_size: u32, opts: &MountOptions) -> Result<Self> {
        let mut buf = vec![0u8; block_size as usize];
        dev.read(0, &mut buf)?;
        let mut sb = match Superblock::deserialize(&buf) {
            Ok(s) => s,
            Err(primary_err) => {
                // Try backup superblock at last block
                let dev_size = dev.size();
                let total_blocks = dev_size / block_size as u64;
                if total_blocks < 2 {
                    return Err(primary_err);
                }
                let backup_offset = (total_blocks - 1) * block_size as u64;
                let mut backup_buf = vec![0u8; block_size as usize];
                dev.read(backup_offset, &mut backup_buf)
                    .map_err(|_| primary_err.context("backup SB read failed"))?;
                let sb = Superblock::deserialize(&backup_buf)?;
                // Restore primary from backup
                let _ = dev.write(0, &backup_buf);
                sb
            }
        };

        let bs = block_size as u64;
        let inode_size = if sb.version >= CFS_VERSION_V3 { sb.inode_size } else { INODE_SIZE };

        // Verify metadata HMAC if the feature is enabled (warning-only on
        // mismatch â€” blocking mount could lock users out of their data)
        let hmac_key = superblock::derive_hmac_key(None);
        if sb.version >= CFS_VERSION_V3
            && sb.features_flags & superblock::FEATURE_METADATA_HMAC != 0
        {
            match superblock::compute_metadata_hmac(&mut *dev, &sb, &hmac_key) {
                Ok(computed) => {
                    if sb.metadata_hmac != computed {
                        eprintln!(
                            "WARNING: metadata HMAC mismatch: volume metadata may have been modified externally"
                        );
                    }
                }
                Err(e) => {
                    eprintln!("WARNING: failed to verify metadata HMAC: {}", e);
                }
            }
        }

        // Update mount statistics
        sb.mount_count += 1;
        sb.last_mount_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if sb.group_count > 0 {
            // --- Block-group mount path ---
            let gdt = group::read_gdt(&mut *dev, sb.gdt_start, sb.group_count, sb.block_size)?;

            let layout = group::compute_group_layout(
                sb.total_blocks,
                sb.block_size,
                inode_size,
                sb.inode_ratio,
                sb.blocks_per_group,
                sb.journal_blocks,
            )?;
            let overhead_per_group = layout.overhead_per_group;

            let gbm = group::GroupBitmapManager::new(
                sb.block_size,
                sb.blocks_per_group,
                sb.group_count,
                layout.global_overhead,
                overhead_per_group,
            );
            let gibm = group::GroupInodeBitmapManager::new(
                sb.block_size, sb.inodes_per_group, sb.group_count,
            );
            let git = group::GroupInodeTable::new(
                sb.inodes_per_group, inode_size, sb.block_size, sb.group_count,
            );

            sb.write_to(&mut *dev)?;
            dev.flush()?;

            // Load journal if enabled (with automatic recovery/replay)
            let jnl = if sb.has_journal() {
                Some(journal::Journal::load(
                    &mut *dev,
                    sb.journal_start,
                    sb.journal_blocks,
                    sb.block_size,
                )?)
            } else {
                None
            };

            let vol = CFSVolume {
                block_size: sb.block_size,
                data_start: sb.data_start,
                has_groups: true,
                dev: Mutex::new(dev),
                inode_cache: Mutex::new(
                    if opts.cache_inodes > 0 {
                        Some(cache::InodeCache::new(opts.cache_inodes as usize))
                    } else {
                        None
                    },
                ),
                block_cache: Mutex::new(
                    if opts.cache_blocks > 0 {
                        Some(cache::BlockCache::new(opts.cache_blocks as usize, sb.block_size))
                    } else {
                        None
                    },
                ),
                superblock: RwLock::new(sb),
                bitmap: Mutex::new(Bitmap::new_empty(0, 0, 0, block_size)),
                inode_table: InodeTable::new(0, 0, block_size),
                inode_bitmap: Mutex::new(None),
                inode_size,
                mount_options: opts.clone(),
                gdt: Mutex::new(gdt),
                group_bitmap_mgr: Mutex::new(Some(gbm)),
                group_inode_bitmap_mgr: Mutex::new(Some(gibm)),
                group_inode_table: Some(git),
                hmac_key,
                journal: Mutex::new(jnl),
                lock_manager: Mutex::new(FileLockManager::new()),
                delayed_alloc: Mutex::new(None),
            };

            // Clean up any orphaned inodes from prior crashes
            let cleaned = vol.orphan_cleanup()?;
            if cleaned > 0 {
                eprintln!("mount: cleaned up {} orphaned inode(s)", cleaned);
            }

            Ok(vol)
        } else {
            // --- Legacy / single-group mount path ---
            let data_block_count = sb.data_block_count();
            let bitmap_blocks = ceil_div(ceil_div(data_block_count, 8), bs);

            let mut bitmap = Bitmap::new_empty(
                data_block_count, sb.bitmap_start, bitmap_blocks, block_size,
            );
            if sb.version >= 3 {
                bitmap.load_with_checksums(&mut *dev)?;
            } else {
                bitmap.load(&mut *dev)?;
            }

            let inode_table = InodeTable::new_with_inode_size(
                sb.inode_table_start, sb.inode_count, block_size, inode_size,
            );

            // Load inode bitmap if available
            let inode_bitmap = if sb.has_inode_bitmap() && sb.inode_bitmap_start > 0 {
                let inode_bitmap_blocks = ceil_div(ceil_div(sb.inode_count as u64, 8), bs);
                let mut ibm = Bitmap::new_empty(
                    sb.inode_count as u64, sb.inode_bitmap_start, inode_bitmap_blocks, block_size,
                );
                if sb.version >= 3 {
                    ibm.load_with_checksums(&mut *dev)?;
                } else {
                    ibm.load(&mut *dev)?;
                }
                Some(ibm)
            } else {
                None
            };

            sb.write_to(&mut *dev)?;
            dev.flush()?;

            // Load journal if enabled (with automatic recovery/replay)
            let jnl = if sb.has_journal() {
                Some(journal::Journal::load(
                    &mut *dev,
                    sb.journal_start,
                    sb.journal_blocks,
                    sb.block_size,
                )?)
            } else {
                None
            };

            let vol = CFSVolume {
                block_size: sb.block_size,
                data_start: sb.data_start,
                has_groups: false,
                dev: Mutex::new(dev),
                superblock: RwLock::new(sb),
                bitmap: Mutex::new(bitmap),
                inode_table,
                inode_bitmap: Mutex::new(inode_bitmap),
                inode_size,
                mount_options: opts.clone(),
                gdt: Mutex::new(Vec::new()),
                group_bitmap_mgr: Mutex::new(None),
                group_inode_bitmap_mgr: Mutex::new(None),
                group_inode_table: None,
                hmac_key,
                journal: Mutex::new(jnl),
                lock_manager: Mutex::new(FileLockManager::new()),
                delayed_alloc: Mutex::new(None),
                inode_cache: Mutex::new(
                    if opts.cache_inodes > 0 {
                        Some(cache::InodeCache::new(opts.cache_inodes as usize))
                    } else {
                        None
                    },
                ),
                block_cache: Mutex::new(
                    if opts.cache_blocks > 0 {
                        Some(cache::BlockCache::new(opts.cache_blocks as usize, block_size))
                    } else {
                        None
                    },
                ),
            };

            // Clean up any orphaned inodes from prior crashes
            let cleaned = vol.orphan_cleanup()?;
            if cleaned > 0 {
                eprintln!("mount: cleaned up {} orphaned inode(s)", cleaned);
            }

            Ok(vol)
        }
    }

    /// Open an existing CFS filesystem from `dev` (backward-compat wrapper).
    pub fn mount(dev: Box<dyn CFSBlockDevice>, block_size: u32) -> Result<Self> {
        Self::mount_v3(dev, block_size, &MountOptions::default())
    }

    /// Persist superblock + bitmaps + caches to disk and flush.
    pub fn sync(&self) -> Result<()> {
        // --- 1. Flush inode cache (dirty inodes written to disk) ---
        {
            let mut ic = self.inode_cache_lock();
            if let Some(ref mut cache) = *ic {
                let dirty_inodes = cache.flush_dirty();
                drop(ic); // release cache lock before disk I/O
                for (idx, inode) in dirty_inodes {
                    self.write_inode_uncached(idx, &inode)?;
                }
            }
        }

        // --- 2. Flush block cache (dirty metadata blocks written to disk) ---
        {
            let mut bc = self.block_cache_lock();
            if let Some(ref mut cache) = *bc {
                let dirty_blocks = cache.flush_dirty();
                let bs = self.block_size as u64;
                drop(bc);
                let mut dev = self.dev();
                for (addr, data) in dirty_blocks {
                    dev.write(addr * bs, &data)?;
                }
            }
        }

        // --- 3. Persist bitmaps, GDT, superblock ---
        let mut sb = self.sb_write(); // level 1
        // Write bitmaps and GDT first (HMAC covers GDT)
        if self.has_groups {
            let mut gbm = self.gbm_lock(); // level 3
            let mut gibm = self.gibm_lock(); // level 3
            let gdt = self.gdt_lock(); // level 3
            let mut dev = self.dev(); // level 7
            if let Some(ref mut g) = *gbm {
                g.save_all(&mut **dev, &gdt)?;
            }
            if let Some(ref mut g) = *gibm {
                g.save_all(&mut **dev, &gdt)?;
            }
            group::write_gdt(&mut **dev, sb.gdt_start, &gdt, sb.block_size)?;
        } else {
            let bm = self.bitmap_lock(); // level 3
            let ibm = self.inode_bitmap_lock(); // level 3
            let mut dev = self.dev(); // level 7
            if sb.version >= 3 {
                bm.save_with_checksums(&mut **dev)?;
                if let Some(ref ib) = *ibm {
                    ib.save_with_checksums(&mut **dev)?;
                }
            } else {
                bm.save(&mut **dev)?;
                if let Some(ref ib) = *ibm {
                    ib.save(&mut **dev)?;
                }
            }
        }

        // Compute and store metadata HMAC (v3 with FEATURE_METADATA_HMAC)
        {
            let mut dev = self.dev();
            if sb.version >= 3
                && sb.features_flags & superblock::FEATURE_METADATA_HMAC != 0
            {
                let hmac_val = superblock::compute_metadata_hmac(
                    &mut **dev, &sb, &self.hmac_key,
                )?;
                sb.metadata_hmac = hmac_val;
            }

            // Write superblock (with updated HMAC and CRC32)
            sb.write_to(&mut **dev)?;
            dev.flush()?;
        }

        // Checkpoint and mark journal clean
        {
            let mut jnl = self.journal_lock(); // level 2
            let mut dev = self.dev(); // level 7
            if let Some(ref mut j) = *jnl {
                j.checkpoint(&mut **dev)?;
                j.mark_clean(&mut **dev)?;
            }
        }

        Ok(())
    }

    /// Allocate `n` data blocks. Returns their physical disk block addresses.
    pub fn allocate(&self, n: u64) -> Result<Vec<u64>> {
        let mut sb = self.sb_write(); // level 1
        if self.has_groups {
            let mut gbm = self.gbm_lock(); // level 3
            let mut gdt = self.gdt_lock(); // level 3
            let mut dev = self.dev(); // level 7
            group::alloc_blocks_group(
                gbm.as_mut().unwrap(),
                &mut **dev, n as usize, &mut gdt, &mut sb,
            )
        } else {
            let mut bm = self.bitmap_lock(); // level 3
            let blocks = alloc::alloc_blocks(&mut bm, &mut sb, n)?;
            let ds = sb.data_start;
            Ok(blocks.iter().map(|&i| ds + i).collect())
        }
    }

    /// Free the given data blocks (physical addresses).
    pub fn deallocate(&self, blocks: &[u64]) -> Result<()> {
        let mut sb = self.sb_write(); // level 1
        if self.has_groups {
            let mut gbm = self.gbm_lock(); // level 3
            let mut gdt = self.gdt_lock(); // level 3
            let mut dev = self.dev(); // level 7
            group::free_blocks_group(
                gbm.as_mut().unwrap(),
                &mut **dev, blocks, &mut gdt, &mut sb,
            )
        } else {
            let mut bm = self.bitmap_lock(); // level 3
            let ds = sb.data_start;
            let indices: Vec<u64> = blocks.iter().map(|&b| b - ds).collect();
            alloc::free_blocks(&mut bm, &mut sb, &indices)
        }
    }

    // -----------------------------------------------------------------------
    // Block cache helpers (10J.2)
    // -----------------------------------------------------------------------

    /// Read a metadata block through the block cache.
    ///
    /// On cache hit: returns cached data (no disk I/O).
    /// On cache miss: reads from disk, inserts into cache.
    ///
    /// File data blocks should NOT use this — use direct I/O to avoid
    /// evicting valuable metadata from the cache.
    pub fn read_block_cached(&self, block_addr: u64) -> Result<Vec<u8>> {
        // Check cache
        {
            let mut bc = self.block_cache_lock();
            if let Some(ref mut cache) = *bc {
                if let Some(data) = cache.get(block_addr) {
                    return Ok(data.to_vec());
                }
            }
        }

        // Cache miss — read from disk
        let bs = self.block_size as usize;
        let mut buf = vec![0u8; bs];
        {
            let mut dev = self.dev();
            dev.read(block_addr * bs as u64, &mut buf)?;
        }

        // Insert into cache (handle dirty eviction)
        {
            let mut bc = self.block_cache_lock();
            if let Some(ref mut cache) = *bc {
                if let Some((evicted_addr, evicted_data)) = cache.insert(block_addr, buf.clone()) {
                    let bs64 = self.block_size as u64;
                    drop(bc);
                    let mut dev = self.dev();
                    dev.write(evicted_addr * bs64, &evicted_data)?;
                }
            }
        }

        Ok(buf)
    }

    /// Write a metadata block through the block cache (deferred disk write).
    pub fn write_block_cached(&self, block_addr: u64, data: Vec<u8>) -> Result<()> {
        let mut bc = self.block_cache_lock();
        if let Some(ref mut cache) = *bc {
            if let Some((evicted_addr, evicted_data)) = cache.put_dirty(block_addr, data) {
                let bs = self.block_size as u64;
                drop(bc);
                let mut dev = self.dev();
                dev.write(evicted_addr * bs, &evicted_data)?;
            }
            Ok(())
        } else {
            drop(bc);
            let bs = self.block_size as u64;
            let mut dev = self.dev();
            dev.write(block_addr * bs, &data)?;
            Ok(())
        }
    }

    /// Get cache statistics (inode cache hit rate, block cache hit rate).
    pub fn cache_stats(&self) -> (Option<cache::CacheStats>, Option<cache::CacheStats>) {
        let ic_stats = self.inode_cache_lock()
            .as_ref()
            .map(|c| c.stats().clone());
        let bc_stats = self.block_cache_lock()
            .as_ref()
            .map(|c| c.stats().clone());
        (ic_stats, bc_stats)
    }

    /// Read an inode by index. Uses the inode cache if enabled.
    pub fn read_inode(&self, index: u32) -> Result<Inode> {
        // Check cache first
        {
            let mut ic = self.inode_cache_lock();
            if let Some(ref mut cache) = *ic {
                if let Some(cached) = cache.get(index) {
                    return Ok(cached.clone());
                }
            }
        }

        // Cache miss or disabled — read from disk
        let inode = self.read_inode_uncached(index)?;

        // Insert into cache (handle dirty eviction writeback)
        {
            let mut ic = self.inode_cache_lock();
            if let Some(ref mut cache) = *ic {
                if let Some((evicted_idx, evicted_inode)) = cache.insert(index, inode.clone()) {
                    drop(ic);
                    self.write_inode_uncached(evicted_idx, &evicted_inode)?;
                }
            }
        }

        Ok(inode)
    }

    /// Write an inode. If cache is enabled, updates the cache and marks dirty
    /// (defers disk write). Dirty inodes are flushed on `sync()` or eviction.
    pub fn write_inode(&self, index: u32, inode: &Inode) -> Result<()> {
        let mut ic = self.inode_cache_lock();
        if let Some(ref mut cache) = *ic {
            if let Some((evicted_idx, evicted_inode)) = cache.put_dirty(index, inode.clone()) {
                drop(ic);
                self.write_inode_uncached(evicted_idx, &evicted_inode)?;
            }
            Ok(())
        } else {
            drop(ic);
            self.write_inode_uncached(index, inode)
        }
    }

    /// Read an inode directly from disk (bypasses cache).
    fn read_inode_uncached(&self, index: u32) -> Result<Inode> {
        if self.has_groups {
            let gdt = self.gdt_lock();   // level 3
            let mut dev = self.dev();    // level 7
            self.group_inode_table.as_ref().unwrap()
                .read_inode(&mut **dev, index, &gdt)
        } else {
            let mut dev = self.dev();
            self.inode_table.read_inode(&mut **dev, index)
        }
    }

    /// Write an inode directly to disk (bypasses cache).
    fn write_inode_uncached(&self, index: u32, inode: &Inode) -> Result<()> {
        if self.has_groups {
            let gdt = self.gdt_lock();   // level 3
            let mut dev = self.dev();    // level 7
            self.group_inode_table.as_ref().unwrap()
                .write_inode(&mut **dev, index, inode, &gdt)
        } else {
            let mut dev = self.dev();
            self.inode_table.write_inode(&mut **dev, index, inode)
        }
    }

    /// Find and allocate the first free inode.
    /// Uses group inode bitmap (v3 group), inode bitmap (v2), or linear scan (v1).
    /// Skips inode 0 (root â€” always reserved). Returns the inode index.
    pub fn alloc_inode(&self) -> Result<u32> {
        if self.has_groups {
            let mut sb = self.sb_write();
            let mut gibm = self.gibm_lock();
            let mut gdt = self.gdt_lock();
            let mut dev = self.dev();
            let ipg = sb.inodes_per_group;
            let idx = group::alloc_inode_for_file(
                gibm.as_mut().unwrap(),
                &mut **dev,
                ROOT_INODE,
                ipg,
                &mut gdt,
                &mut sb,
            )?;
            if idx == ROOT_INODE {
                let idx2 = group::alloc_inode_for_file(
                    gibm.as_mut().unwrap(),
                    &mut **dev,
                    ROOT_INODE,
                    ipg,
                    &mut gdt,
                    &mut sb,
                )?;
                let placeholder = Inode { mode: INODE_FILE, ..Inode::new_file() };
                self.group_inode_table.as_ref().unwrap()
                    .write_inode(&mut **dev, idx2, &placeholder, &gdt)?;
                return Ok(idx2);
            }
            let placeholder = Inode { mode: INODE_FILE, ..Inode::new_file() };
            self.group_inode_table.as_ref().unwrap()
                .write_inode(&mut **dev, idx, &placeholder, &gdt)?;
            Ok(idx)
        } else {
            let mut sb = self.sb_write(); // level 1 â€” before ibm (level 3)
            let mut ibm = self.inode_bitmap_lock(); // level 3
            let mut dev = self.dev(); // level 7
            if let Some(ref mut ibm_inner) = *ibm {
                match ibm_inner.alloc() {
                    Some(idx) => {
                        let idx = idx as u32;
                        if idx == 0 {
                            // Inode 0 is ROOT_INODE — skip it but account for its allocation
                            sb.free_inodes = sb.free_inodes.saturating_sub(1);
                            match ibm_inner.alloc() {
                                Some(idx2) => {
                                    sb.free_inodes = sb.free_inodes.saturating_sub(1);
                                    let placeholder = Inode { mode: INODE_FILE, ..Inode::new_file() };
                                    self.inode_table.write_inode(&mut **dev, idx2 as u32, &placeholder)?;
                                    Ok(idx2 as u32)
                                }
                                None => bail!(
                                    "no free inodes available (all {} inodes in use)",
                                    sb.inode_count
                                ),
                            }
                        } else {
                            sb.free_inodes = sb.free_inodes.saturating_sub(1);
                            let placeholder = Inode { mode: INODE_FILE, ..Inode::new_file() };
                            self.inode_table.write_inode(&mut **dev, idx, &placeholder)?;
                            Ok(idx)
                        }
                    }
                    None => bail!(
                        "no free inodes available (all {} inodes in use)",
                        sb.inode_count
                    ),
                }
            } else {
                // v1 fallback: linear scan
                drop(dev);
                drop(ibm);
                drop(sb);
                self.alloc_inode_linear_scan()
            }
        }
    }

    /// Linear scan fallback for v1 volumes without inode bitmap.
    fn alloc_inode_linear_scan(&self) -> Result<u32> {
        let sb = self.sb_read();
        let bs = sb.block_size;
        let inodes_per_block = bs / INODE_SIZE;
        let inode_blocks = ceil_div(
            sb.inode_count as u64 * INODE_SIZE as u64,
            bs as u64,
        ) as u32;
        let inode_table_start = sb.inode_table_start;
        let inode_count = sb.inode_count;
        drop(sb);

        let mut dev = self.dev();
        for block_idx in 0..inode_blocks {
            let disk_offset =
                (inode_table_start + block_idx as u64) * bs as u64;
            let mut buf = vec![0u8; bs as usize];
            dev.read(disk_offset, &mut buf)?;

            for i in 0..inodes_per_block {
                let global_idx = block_idx * inodes_per_block + i;
                if global_idx == 0 {
                    continue;
                }
                if global_idx >= inode_count {
                    break;
                }

                let offset = (i * INODE_SIZE) as usize;
                let inode_bytes: &[u8; 128] =
                    buf[offset..offset + 128].try_into().unwrap();
                let inode = Inode::deserialize(inode_bytes);

                if inode.mode == INODE_UNUSED {
                    let placeholder = Inode {
                        mode: INODE_FILE,
                        ..inode
                    };
                    self.inode_table.write_inode(
                        &mut **dev,
                        global_idx,
                        &placeholder,
                    )?;
                    return Ok(global_idx);
                }
            }
        }

        let sb = self.sb_read();
        bail!(
            "no free inodes available (all {} inodes in use)",
            sb.inode_count
        )
    }

    /// Free an inode by resetting it to INODE_UNUSED (all zeros).
    /// Does NOT free data blocks â€” caller must do that first.
    pub fn free_inode(&self, index: u32) -> Result<()> {
        if index == 0 {
            bail!("cannot free root inode");
        }
        {
            let sb = self.sb_read();
            let inode_count = if self.has_groups {
                sb.group_count as u64 * sb.inodes_per_group as u64
            } else {
                sb.inode_count as u64
            };
            if index as u64 >= inode_count {
                bail!("inode index out of range");
            }
        }
        let zeroed = Inode {
            mode: INODE_UNUSED,
            nlinks: 0,
            block_count: 0,
            size: 0,
            created: 0,
            modified: 0,
            direct_blocks: [0; 10],
            indirect_block: 0,
            double_indirect: 0,
            accessed_ns: 0,
            changed_ns: 0,
            owner_id: 0,
            group_id: 0,
            permissions: 0,
            flags: 0,
            xattr_block: 0,
            xattr_inline_size: 0,
            checksum: 0,
            block_count_hi: 0,
            inline_area: [0u8; 76],
        };
        self.write_inode(index, &zeroed)?;

        // Invalidate from cache — inode is being freed, no need to keep it
        {
            let mut ic = self.inode_cache_lock();
            if let Some(ref mut cache) = *ic {
                let _ = cache.invalidate(index);
            }
        }

        if self.has_groups {
            let mut sb = self.sb_write(); // level 1
            let mut gibm = self.gibm_lock(); // level 3
            let mut gdt = self.gdt_lock(); // level 3
            let mut dev = self.dev(); // level 7
            gibm.as_mut().unwrap().free_inode(&mut **dev, index, &mut gdt)?;
            sb.free_inodes += 1;
        } else {
            let mut sb = self.sb_write(); // level 1
            let mut ibm = self.inode_bitmap_lock(); // level 3
            if let Some(ref mut ibm_inner) = *ibm {
                let _ = ibm_inner.free(index as u64);
                sb.free_inodes += 1;
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 3G â€” High-Level File API
    // -----------------------------------------------------------------------

    /// Resolve a path to its inode index, dispatching inode reads correctly.
    fn resolve_path_inline(&self, path: &str) -> Result<u32> {
        if path == "/" {
            return Ok(ROOT_INODE);
        }
        let normalized = path.trim_start_matches('/');
        let components: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
        let mut current = ROOT_INODE;
        for component in &components {
            let inode = self.read_inode(current)?;
            if inode.mode != INODE_DIR {
                bail!("not a directory in path '{}'", path);
            }
            let sb = self.sb_read();
            let mut dev = self.dev();
            match dir::lookup_dispatch(&mut **dev, &inode, current, &sb, component)? {
                Some(entry) => current = entry.inode_index,
                None => bail!("not found: '{}' in path '{}'", component, path),
            }
        }
        Ok(current)
    }

    /// Resolve parent directory and extract the final name component.
    fn resolve_parent_inline(&self, path: &str) -> Result<(u32, String)> {
        let trimmed = path.trim_end_matches('/');
        let slash = trimmed.rfind('/');
        let (parent_path, name) = match slash {
            Some(0) | None => ("/", trimmed.trim_start_matches('/')),
            Some(pos) => (&trimmed[..pos], &trimmed[pos + 1..]),
        };
        if name.is_empty() {
            bail!("invalid path: '{}'", path);
        }
        let parent_idx = self.resolve_path_inline(parent_path)?;
        Ok((parent_idx, name.to_string()))
    }

    /// Helper: run a closure with alloc + dev + sb locks. Lock order: sb(1) â†’ alloc(3) â†’ dev(7).
    fn with_alloc<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut dyn CFSBlockDevice, &mut BlockAlloc<'_>, &mut Superblock) -> Result<R>,
    {
        let mut sb = self.sb_write(); // level 1
        if self.has_groups {
            let mut gbm = self.gbm_lock(); // level 3
            let mut gdt = self.gdt_lock(); // level 3
            let mut dev = self.dev(); // level 7
            let mut alloc = BlockAlloc::Group {
                gbm: gbm.as_mut().unwrap(),
                gdt: &mut gdt,
            };
            f(&mut **dev, &mut alloc, &mut sb)
        } else {
            let mut bm = self.bitmap_lock(); // level 3
            let mut dev = self.dev(); // level 7
            let ds = sb.data_start;
            let mut alloc = BlockAlloc::Legacy { bitmap: &mut bm, data_start: ds };
            f(&mut **dev, &mut alloc, &mut sb)
        }
    }

    /// Create a new empty file at `path`. Returns the new inode index.
    pub fn create_file(&self, path: &str) -> Result<u32> {
        self.check_writable()?;
        let (parent_idx, filename) = self.resolve_parent_inline(path)?;

        {
            let parent_inode = self.read_inode(parent_idx)?;
            let sb = self.sb_read();
            let mut dev = self.dev();
            if dir::lookup_dispatch(&mut **dev, &parent_inode, parent_idx, &sb, &filename)?.is_some() {
                bail!("file already exists: {}", path);
            }
        }

        let txn_id = self.journal_begin()?;
        let result = (|| -> Result<u32> {
            let new_idx = self.alloc_inode()?;
            let inode = if self.inode_size == INODE_SIZE_V3 {
                let sb = self.sb_read();
                inode::Inode::new_file_v3(sb.default_permissions & 0o7777)
            } else {
                Inode::new_file()
            };

            // Journal the inode block before writing
            if let Ok(blk) = self.inode_disk_block(new_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }
            self.write_inode(new_idx, &inode)?;

            let entry = dir::DirEntry::new(new_idx, INODE_FILE as u8, &filename)?;
            let mut parent_inode = self.read_inode(parent_idx)?;
            self.with_alloc(|dev, alloc, sb| {
                dir::add_dir_entry_dispatch(dev, &mut parent_inode, parent_idx, alloc, sb, &entry)
            })?;
            // Journal parent inode block before updating timestamps
            if let Ok(blk) = self.inode_disk_block(parent_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }
            parent_inode.touch_mtime();
            self.write_inode(parent_idx, &parent_inode)?;

            Ok(new_idx)
        })();

        match result {
            Ok(idx) => {
                self.journal_commit(txn_id)?;
                Ok(idx)
            }
            Err(e) => {
                let _ = self.journal_abort(txn_id);
                Err(e)
            }
        }
    }

    /// Read `len` bytes from the file at `path` starting at `offset`.
    pub fn read_file(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        if inode.mode != INODE_FILE {
            bail!("not a file: {}", path);
        }
        let bs = self.block_size;
        let data = {
            let mut dev = self.dev();
            file_io::read_data(&mut **dev, &inode, bs, offset, len)?
        };
        // Update atime if mode allows
        if inode.should_update_atime(self.mount_options.atime_mode) {
            inode.touch_atime();
            self.write_inode(inode_idx, &inode)?;
        }
        Ok(data)
    }

    /// Write `data` to the file at `path` starting at `offset`.
    pub fn write_file(&self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        self.check_writable()?;
        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        if inode.mode != INODE_FILE {
            bail!("not a file: {}", path);
        }
        let bs = self.block_size;
        self.with_alloc(|dev, alloc, sb| {
            file_io::write_data(dev, alloc, sb, &mut inode, bs, offset, data)
        })?;
        // Update mtime + ctime on data write
        inode.touch_mtime();
        self.write_inode(inode_idx, &inode)
    }

    /// Truncate the file at `path` to `new_size` bytes.
    pub fn truncate(&self, path: &str, new_size: u64) -> Result<()> {
        self.check_writable()?;
        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        if inode.mode != INODE_FILE {
            bail!("not a file: {}", path);
        }

        let txn_id = self.journal_begin()?;
        let result = (|| -> Result<()> {
            // Journal inode block before truncation
            if let Ok(blk) = self.inode_disk_block(inode_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }

            let bs = self.block_size;
            let secure = self.should_secure_delete();
            self.with_alloc(|dev, alloc, sb| {
                file_io::truncate(dev, alloc, sb, &mut inode, bs, new_size, secure)
            })?;
            inode.touch_mtime();
            self.write_inode(inode_idx, &inode)
        })();

        match result {
            Ok(()) => { self.journal_commit(txn_id)?; Ok(()) }
            Err(e) => { let _ = self.journal_abort(txn_id); Err(e) }
        }
    }

    /// Delete the file or symlink at `path`.
    /// If nlinks > 1 (hard links exist), only removes the directory entry and decrements nlinks.
    /// When nlinks reaches 0, frees all data blocks and the inode.
    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.check_writable()?;
        let (parent_idx, filename) = self.resolve_parent_inline(path)?;

        let parent_inode = self.read_inode(parent_idx)?;
        let entry = {
            let sb = self.sb_read();
            let mut dev = self.dev();
            dir::lookup_dispatch(&mut **dev, &parent_inode, parent_idx, &sb, &filename)?
        };
        let entry = match entry {
            Some(e) => e,
            None => bail!("file not found: {}", path),
        };

        let inode_idx = entry.inode_index;
        let mut inode = self.read_inode(inode_idx)?;
        if inode.mode != INODE_FILE && inode.mode != INODE_SYMLINK {
            bail!("not a file or symlink, use rmdir for directories: {}", path);
        }

        let txn_id = self.journal_begin()?;

        // Add to orphan list BEFORE removing directory entry
        self.orphan_add(inode_idx)?;

        let result = (|| -> Result<()> {
            // Remove directory entry
            {
                let parent_inode2 = self.read_inode(parent_idx)?;
                let sb = self.sb_read();
                let mut dev = self.dev();
                dir::remove_dir_entry_dispatch(&mut **dev, &parent_inode2, parent_idx, &sb, &filename)?;
            }

            // Journal inode block before modifying
            if let Ok(blk) = self.inode_disk_block(inode_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }

            // Decrement nlinks
            inode.nlinks = inode.nlinks.saturating_sub(1);

            if inode.nlinks == 0 {
                // Last link removed â€” free all resources
                let bs = self.block_size;
                let secure = self.should_secure_delete();
                self.with_alloc(|dev, alloc, sb| {
                    file_io::free_all_blocks(dev, alloc, sb, &mut inode, bs, secure)
                })?;
                self.free_inode(inode_idx)?;
            } else {
                // Other links still exist â€” keep inode, update metadata
                inode.touch_ctime();
                self.write_inode(inode_idx, &inode)?;
            }

            // Journal parent inode block before timestamp update
            if let Ok(blk) = self.inode_disk_block(parent_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }
            let mut parent_inode3 = self.read_inode(parent_idx)?;
            parent_inode3.touch_mtime();
            self.write_inode(parent_idx, &parent_inode3)?;

            Ok(())
        })();

        match result {
            Ok(()) => {
                // Remove from orphan list on success
                {
                    let mut sb = self.sb_write();
                    sb.first_orphan_inode = self.read_inode(inode_idx).ok()
                        .map(|i| i.get_next_orphan())
                        .unwrap_or(0);
                }
                self.journal_commit(txn_id)?;
                Ok(())
            }
            Err(e) => {
                // On failure, orphan list ensures cleanup on next mount
                let _ = self.journal_abort(txn_id);
                Err(e)
            }
        }
    }

    // -----------------------------------------------------------------------
    // 3H â€” High-Level Directory API
    // -----------------------------------------------------------------------

    /// Create a new directory at `path`. Returns the new inode index.
    pub fn mkdir(&self, path: &str) -> Result<u32> {
        self.check_writable()?;
        let (parent_idx, dirname) = self.resolve_parent_inline(path)?;

        {
            let parent_inode = self.read_inode(parent_idx)?;
            let sb = self.sb_read();
            let mut dev = self.dev();
            if dir::lookup_dispatch(&mut **dev, &parent_inode, parent_idx, &sb, &dirname)?.is_some() {
                bail!("already exists: {}", path);
            }
        }

        let txn_id = self.journal_begin()?;
        let result = (|| -> Result<u32> {
            let new_idx = self.alloc_inode()?;
            let inode = if self.inode_size == INODE_SIZE_V3 {
                let sb = self.sb_read();
                inode::Inode::new_dir_v3(sb.default_permissions & 0o7777)
            } else {
                Inode::new_dir()
            };

            // Journal inode block before writing
            if let Ok(blk) = self.inode_disk_block(new_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }
            self.write_inode(new_idx, &inode)?;

            {
                let mut new_inode = self.read_inode(new_idx)?;
                self.with_alloc(|dev, alloc, sb| {
                    dir::init_dir_block(dev, &mut new_inode, alloc, sb, new_idx, parent_idx)?;
                    Ok(new_inode)
                }).and_then(|new_inode| self.write_inode(new_idx, &new_inode))?;
            }

            let entry = dir::DirEntry::new(new_idx, INODE_DIR as u8, &dirname)?;
            let mut parent_inode2 = self.read_inode(parent_idx)?;
            self.with_alloc(|dev, alloc, sb| {
                dir::add_dir_entry_dispatch(dev, &mut parent_inode2, parent_idx, alloc, sb, &entry)
            })?;
            // Journal parent inode block before nlinks/timestamps update
            if let Ok(blk) = self.inode_disk_block(parent_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }
            parent_inode2.nlinks += 1;
            parent_inode2.touch_mtime();
            self.write_inode(parent_idx, &parent_inode2)?;

            Ok(new_idx)
        })();

        match result {
            Ok(idx) => {
                self.journal_commit(txn_id)?;
                Ok(idx)
            }
            Err(e) => {
                let _ = self.journal_abort(txn_id);
                Err(e)
            }
        }
    }

    /// Remove an empty directory at `path`.
    pub fn rmdir(&self, path: &str) -> Result<()> {
        self.check_writable()?;
        let (parent_idx, dirname) = self.resolve_parent_inline(path)?;

        let inode_idx;
        let mut inode;
        {
            let parent_inode = self.read_inode(parent_idx)?;
            let sb = self.sb_read();
            let mut dev = self.dev();
            let entry = dir::lookup_dispatch(&mut **dev, &parent_inode, parent_idx, &sb, &dirname)?;
            let entry = match entry {
                Some(e) => e,
                None => bail!("not found: {}", path),
            };
            inode_idx = entry.inode_index;
        }

        inode = self.read_inode(inode_idx)?;
        if inode.mode != INODE_DIR {
            bail!("not a directory: {}", path);
        }

        // Verify directory is empty (only "." and "..")
        {
            let sb = self.sb_read();
            let mut dev = self.dev();
            let entries = dir::read_dir_entries_dispatch(&mut **dev, &inode, inode_idx, &sb)?;
            let real_entries = entries.iter()
                .filter(|e| e.name_str() != "." && e.name_str() != "..")
                .count();
            if real_entries > 0 {
                bail!("directory not empty: {}", path);
            }
        }

        let txn_id = self.journal_begin()?;

        // Add to orphan list BEFORE removing directory entry
        self.orphan_add(inode_idx)?;

        let result = (|| -> Result<()> {
            // Journal inode block before freeing
            if let Ok(blk) = self.inode_disk_block(inode_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }

            let bs = self.block_size;
            let secure = self.should_secure_delete();
            self.with_alloc(|dev, alloc, sb| {
                file_io::free_all_blocks(dev, alloc, sb, &mut inode, bs, secure)
            })?;
            self.free_inode(inode_idx)?;

            {
                let parent_inode2 = self.read_inode(parent_idx)?;
                let sb = self.sb_read();
                let mut dev = self.dev();
                dir::remove_dir_entry_dispatch(&mut **dev, &parent_inode2, parent_idx, &sb, &dirname)?;
            }

            // Journal parent inode block before nlinks/timestamps update
            if let Ok(blk) = self.inode_disk_block(parent_idx) {
                self.journal_metadata_block(txn_id, blk)?;
            }
            let mut parent3 = self.read_inode(parent_idx)?;
            parent3.nlinks -= 1;
            parent3.touch_mtime();
            self.write_inode(parent_idx, &parent3)?;

            Ok(())
        })();

        match result {
            Ok(()) => {
                {
                    let mut sb = self.sb_write();
                    sb.first_orphan_inode = self.read_inode(inode_idx).ok()
                        .map(|i| i.get_next_orphan())
                        .unwrap_or(0);
                }
                self.journal_commit(txn_id)?;
                Ok(())
            }
            Err(e) => {
                let _ = self.journal_abort(txn_id);
                Err(e)
            }
        }
    }

    /// List all entries in the directory at `path`.
    pub fn list_dir(&self, path: &str) -> Result<Vec<dir::DirEntry>> {
        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        if inode.mode != INODE_DIR {
            bail!("not a directory: {}", path);
        }
        let entries = {
            let sb = self.sb_read();
            let mut dev = self.dev();
            dir::read_dir_entries_dispatch(&mut **dev, &inode, inode_idx, &sb)?
        };
        // Update atime if mode allows
        if inode.should_update_atime(self.mount_options.atime_mode) {
            inode.touch_atime();
            self.write_inode(inode_idx, &inode)?;
        }
        Ok(entries)
    }

    /// Get the inode for a path (stat-like).
    pub fn stat(&self, path: &str) -> Result<Inode> {
        let inode_idx = self.resolve_path_inline(path)?;
        self.read_inode(inode_idx)
    }

    /// Check whether a path exists.
    pub fn exists(&self, path: &str) -> Result<bool> {
        match self.resolve_path_inline(path) {
            Ok(_) => Ok(true),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not found") {
                    Ok(false)
                } else {
                    Err(e)
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // 3I â€” Rename & Move
    // -----------------------------------------------------------------------

    /// Rename or move a file/directory from `old_path` to `new_path`.
    /// If `new_path` exists and is a file, it is overwritten.
    /// Cannot overwrite an existing directory.
    pub fn rename(&self, old_path: &str, new_path: &str) -> Result<()> {
        self.check_writable()?;
        let (old_parent, old_name) = self.resolve_parent_inline(old_path)?;
        let old_entry = {
            let old_parent_inode = self.read_inode(old_parent)?;
            let sb = self.sb_read();
            let mut dev = self.dev();
            dir::lookup_dispatch(&mut **dev, &old_parent_inode, old_parent, &sb, &old_name)?
                .ok_or_else(|| anyhow::anyhow!("source not found: {}", old_path))?
        };

        let (new_parent, new_name) = self.resolve_parent_inline(new_path)?;

        let txn_id = self.journal_begin()?;
        let result = (|| -> Result<()> {
            let bs = self.block_size;
            let secure = self.should_secure_delete();

            // Check if target exists
            let existing = {
                let new_parent_inode = self.read_inode(new_parent)?;
                let sb = self.sb_read();
                let mut dev = self.dev();
                dir::lookup_dispatch(&mut **dev, &new_parent_inode, new_parent, &sb, &new_name)?
            };
            if let Some(existing) = existing {
                if existing.file_type == INODE_DIR as u8 {
                    bail!("cannot overwrite directory with rename");
                }
                let ex_idx = existing.inode_index;
                let mut ex_inode = self.read_inode(ex_idx)?;

                // Journal overwritten inode block
                if let Ok(blk) = self.inode_disk_block(ex_idx) {
                    self.journal_metadata_block(txn_id, blk)?;
                }

                self.with_alloc(|dev, alloc, sb| {
                    file_io::free_all_blocks(dev, alloc, sb, &mut ex_inode, bs, secure)
                })?;
                self.free_inode(ex_idx)?;
                {
                    let new_parent_inode2 = self.read_inode(new_parent)?;
                    let sb = self.sb_read();
                    let mut dev = self.dev();
                    dir::remove_dir_entry_dispatch(&mut **dev, &new_parent_inode2, new_parent, &sb, &new_name)?;
                }
            }

            // Remove from old parent
            {
                let old_parent_inode2 = self.read_inode(old_parent)?;
                let sb = self.sb_read();
                let mut dev = self.dev();
                dir::remove_dir_entry_dispatch(&mut **dev, &old_parent_inode2, old_parent, &sb, &old_name)?;
            }

            // Add to new parent
            let new_entry = dir::DirEntry::new(old_entry.inode_index, old_entry.file_type, &new_name)?;
            let mut new_parent_inode3 = self.read_inode(new_parent)?;
            self.with_alloc(|dev, alloc, sb| {
                dir::add_dir_entry_dispatch(dev, &mut new_parent_inode3, new_parent, alloc, sb, &new_entry)
            })?;
            self.write_inode(new_parent, &new_parent_inode3)?;

            // Journal moved inode block before ctime update
            if let Ok(blk) = self.inode_disk_block(old_entry.inode_index) {
                self.journal_metadata_block(txn_id, blk)?;
            }
            let mut moved_inode = self.read_inode(old_entry.inode_index)?;
            moved_inode.touch_ctime();
            self.write_inode(old_entry.inode_index, &moved_inode)?;

            // Journal parent inode blocks before timestamp/nlinks update
            if let Ok(blk) = self.inode_disk_block(old_parent) {
                self.journal_metadata_block(txn_id, blk)?;
            }
            let mut old_p = self.read_inode(old_parent)?;
            old_p.touch_mtime();
            let mut new_p_ts = self.read_inode(new_parent)?;
            new_p_ts.touch_mtime();

            if old_entry.file_type == INODE_DIR as u8 && old_parent != new_parent {
                self.update_dotdot(old_entry.inode_index, new_parent)?;
                old_p.nlinks -= 1;
                new_p_ts.nlinks += 1;
            }

            self.write_inode(old_parent, &old_p)?;
            if new_parent != old_parent {
                if let Ok(blk) = self.inode_disk_block(new_parent) {
                    self.journal_metadata_block(txn_id, blk)?;
                }
                self.write_inode(new_parent, &new_p_ts)?;
            }

            Ok(())
        })();

        match result {
            Ok(()) => { self.journal_commit(txn_id)?; Ok(()) }
            Err(e) => { let _ = self.journal_abort(txn_id); Err(e) }
        }
    }

    // -----------------------------------------------------------------------
    // 10D.5 â€” chmod / chown
    // -----------------------------------------------------------------------

    /// Change file/directory permissions.
    pub fn chmod(&self, path: &str, permissions: u32) -> Result<()> {
        self.check_writable()?;
        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        inode.permissions = permissions & 0o7777;
        inode.touch_ctime();
        self.write_inode(inode_idx, &inode)
    }

    /// Change file/directory owner and/or group.
    pub fn chown(&self, path: &str, owner: Option<u32>, group: Option<u32>) -> Result<()> {
        self.check_writable()?;
        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        if let Some(uid) = owner {
            inode.owner_id = uid;
        }
        if let Some(gid) = group {
            inode.group_id = gid;
        }
        inode.touch_ctime();
        self.write_inode(inode_idx, &inode)
    }

    // -----------------------------------------------------------------------
    // 10G.3 â€” Hard Links
    // -----------------------------------------------------------------------

    /// Create a hard link. `existing_path` must be a regular file.
    /// `new_path` is where the new directory entry will be created.
    pub fn link(&self, existing_path: &str, new_path: &str) -> Result<()> {
        self.check_writable()?;

        // Resolve existing path â†’ inode
        let existing_inode_idx = self.resolve_path_inline(existing_path)?;
        let mut inode = self.read_inode(existing_inode_idx)?;

        if inode.mode != INODE_FILE {
            bail!("hard links only supported for regular files (mode={})", inode.mode);
        }
        if inode.nlinks >= u16::MAX {
            bail!("maximum link count reached ({}) for '{}'", inode.nlinks, existing_path);
        }

        // Resolve parent of new path
        let (new_parent_idx, new_name) = self.resolve_parent_inline(new_path)?;

        // Check no duplicate name
        {
            let parent_inode = self.read_inode(new_parent_idx)?;
            let sb = self.sb_read();
            let mut dev = self.dev();
            if dir::lookup_dispatch(&mut **dev, &parent_inode, new_parent_idx, &sb, &new_name)?.is_some() {
                bail!("entry '{}' already exists in target directory", new_name);
            }
        }

        // Add directory entry pointing to existing inode
        let entry = dir::DirEntry::new(existing_inode_idx, dir::DIR_ENTRY_FILE, &new_name)?;
        let mut parent_inode = self.read_inode(new_parent_idx)?;
        self.with_alloc(|dev, alloc, sb| {
            dir::add_dir_entry_dispatch(dev, &mut parent_inode, new_parent_idx, alloc, sb, &entry)
        })?;
        self.write_inode(new_parent_idx, &parent_inode)?;

        // Increment nlinks
        inode.nlinks += 1;
        inode.touch_ctime();
        self.write_inode(existing_inode_idx, &inode)?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // 10G.2 â€” Symbolic Links
    // -----------------------------------------------------------------------

    /// Maximum symlink target length.
    const MAX_SYMLINK_TARGET: usize = 4095;

    /// Create a symbolic link at `link_path` pointing to `target`.
    pub fn symlink(&self, target: &str, link_path: &str) -> Result<u32> {
        self.check_writable()?;

        if target.is_empty() {
            bail!("symlink target cannot be empty");
        }
        if target.len() > Self::MAX_SYMLINK_TARGET {
            bail!("symlink target too long ({} bytes, max {})", target.len(), Self::MAX_SYMLINK_TARGET);
        }

        let (parent_idx, link_name) = self.resolve_parent_inline(link_path)?;

        // Check no duplicate
        {
            let parent_inode = self.read_inode(parent_idx)?;
            let sb = self.sb_read();
            let mut dev = self.dev();
            if dir::lookup_dispatch(&mut **dev, &parent_inode, parent_idx, &sb, &link_name)?.is_some() {
                bail!("entry '{}' already exists in parent directory", link_name);
            }
        }

        // Allocate inode
        let symlink_inode_idx = self.alloc_inode()?;
        let mut symlink_inode = Inode::new_symlink(target);

        // For slow symlinks (target > 76 bytes): allocate data block and write target
        let target_bytes = target.as_bytes();
        if target_bytes.len() > 76 {
            let bs = self.block_size;
            let disk_block = self.with_alloc(|dev, alloc, sb| {
                let blocks = alloc.alloc(dev, sb, 1)?;
                Ok(blocks[0])
            })?;

            // Write target to block (zero-padded)
            let mut block_buf = vec![0u8; bs as usize];
            block_buf[..target_bytes.len()].copy_from_slice(target_bytes);
            {
                let mut dev = self.dev();
                dev.write(disk_block * bs as u64, &block_buf)?;
            }

            // Set direct block pointer
            if symlink_inode.flags & inode::INODE_FLAG_EXTENTS != 0 {
                self.with_alloc(|dev, alloc, sb| {
                    file_io::set_block_ptr(dev, &mut symlink_inode, 0, disk_block, bs, alloc, sb)
                })?;
            } else {
                symlink_inode.direct_blocks[0] = disk_block;
            }
            symlink_inode.block_count = 1;
        }

        // Write symlink inode
        self.write_inode(symlink_inode_idx, &symlink_inode)?;

        // Add directory entry with symlink file type
        let entry = dir::DirEntry::new(symlink_inode_idx, dir::DIR_ENTRY_SYMLINK, &link_name)?;
        let mut parent_inode = self.read_inode(parent_idx)?;
        self.with_alloc(|dev, alloc, sb| {
            dir::add_dir_entry_dispatch(dev, &mut parent_inode, parent_idx, alloc, sb, &entry)
        })?;
        self.write_inode(parent_idx, &parent_inode)?;

        Ok(symlink_inode_idx)
    }

    /// Read the target of a symbolic link (does not follow the final symlink).
    pub fn readlink(&self, path: &str) -> Result<String> {
        let inode_idx = {
            let sb = self.sb_read();
            let mut dev = self.dev();
            path::resolve_path_no_final_follow(
                &mut **dev,
                &self.inode_table,
                &sb,
                path,
            )?
        };
        let inode = self.read_inode(inode_idx)?;

        if inode.mode != INODE_SYMLINK {
            bail!("'{}' is not a symbolic link (mode={})", path, inode.mode);
        }

        self.readlink_inode(&inode)
    }

    /// Read symlink target from inode data.
    fn readlink_inode(&self, inode: &Inode) -> Result<String> {
        let target_len = inode.size as usize;

        if inode.flags & inode::INODE_FLAG_INLINE_DATA != 0 {
            // Fast symlink: stored in inline_area
            let target = &inode.inline_area[..target_len.min(76)];
            String::from_utf8(target.to_vec())
                .map_err(|_| anyhow::anyhow!("symlink target is not valid UTF-8"))
        } else {
            // Slow symlink: stored in data block
            let mut dev = self.dev();
            let data = file_io::read_data(
                &mut **dev,
                inode,
                self.block_size,
                0,
                target_len as u64,
            )?;
            String::from_utf8(data)
                .map_err(|_| anyhow::anyhow!("symlink target is not valid UTF-8"))
        }
    }

    /// Stat without following the final symlink (like POSIX lstat).
    pub fn lstat(&self, path: &str) -> Result<Inode> {
        let inode_idx = {
            let sb = self.sb_read();
            let mut dev = self.dev();
            path::resolve_path_no_final_follow(
                &mut **dev,
                &self.inode_table,
                &sb,
                path,
            )?
        };
        self.read_inode(inode_idx)
    }

    // -----------------------------------------------------------------------
    // 10G.4 â€” Extended Attributes
    // -----------------------------------------------------------------------

    /// Get an extended attribute value.
    pub fn get_xattr(&self, path: &str, key: &str) -> Result<Option<Vec<u8>>> {
        xattr::validate_xattr_key(key)?;
        let inode_idx = self.resolve_path_inline(path)?;
        let inode = self.read_inode(inode_idx)?;
        let sb = self.sb_read();
        let mut dev = self.dev();
        xattr::get_xattr(&mut **dev, &inode, &sb, key)
    }

    /// Set an extended attribute (creates or updates).
    pub fn set_xattr(&self, path: &str, key: &str, value: &[u8]) -> Result<()> {
        self.check_writable()?;
        xattr::validate_xattr_key(key)?;
        if value.len() > xattr::XATTR_MAX_VALUE_LEN {
            bail!("xattr value too long ({} bytes, max {})", value.len(), xattr::XATTR_MAX_VALUE_LEN);
        }

        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        self.with_alloc(|dev, alloc, sb| {
            xattr::set_xattr(dev, &mut inode, sb, alloc, key, value)
        })?;
        self.write_inode(inode_idx, &inode)
    }

    /// List all extended attribute keys.
    pub fn list_xattr(&self, path: &str) -> Result<Vec<String>> {
        let inode_idx = self.resolve_path_inline(path)?;
        let inode = self.read_inode(inode_idx)?;
        let sb = self.sb_read();
        let mut dev = self.dev();
        xattr::list_xattr(&mut **dev, &inode, &sb)
    }

    /// Remove an extended attribute.
    pub fn remove_xattr(&self, path: &str, key: &str) -> Result<()> {
        self.check_writable()?;
        xattr::validate_xattr_key(key)?;
        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        self.with_alloc(|dev, alloc, sb| {
            xattr::remove_xattr(dev, &mut inode, sb, alloc, key)
        })?;
        self.write_inode(inode_idx, &inode)
    }

    /// Update the ".." entry in a directory to point to a new parent.
    fn update_dotdot(&self, dir_inode_idx: u32, new_parent: u32) -> Result<()> {
        let dir_inode = self.read_inode(dir_inode_idx)?;
        let bs = self.block_size;
        let bsu = bs as u64;

        let mut dev = self.dev();
        // ".." is always in the first block, second entry
        let physical = file_io::get_block_ptr(&mut **dev, &dir_inode, 0, bs)?;
        if physical == 0 {
            bail!("directory has no data block");
        }

        let mut buf = vec![0u8; bs as usize];
        dev.read(physical * bsu, &mut buf)?;

        // Scan entries in first block for ".."
        let n_entries = if dir::block_has_checksum(&buf, bs) {
            dir::entries_per_block_v3(bs) as usize
        } else {
            bs as usize / dir::DIR_ENTRY_SIZE
        };
        let has_cksum = dir::block_has_checksum(&buf, bs);
        for i in 0..n_entries {
            let offset = i * dir::DIR_ENTRY_SIZE;
            let entry_buf: &[u8; 128] =
                buf[offset..offset + dir::DIR_ENTRY_SIZE].try_into().unwrap();
            let entry = dir::DirEntry::deserialize(entry_buf);
            if !entry.is_unused() && entry.name_str() == ".." {
                // Overwrite inode_index with new_parent
                buf[offset..offset + 4].copy_from_slice(&new_parent.to_le_bytes());
                if has_cksum {
                    dir::stamp_checksum(&mut buf, bs);
                }
                dev.write(physical * bsu, &buf)?;
                return Ok(());
            }
        }

        bail!("'..' entry not found in directory")
    }

    // -----------------------------------------------------------------------
    // Phase 5 â€” API extensions for WinFSP integration
    // -----------------------------------------------------------------------

    /// Resolve a path to its inode index.
    pub fn resolve_path(&self, path: &str) -> Result<u32> {
        self.resolve_path_inline(path)
    }

    /// Read data from a file by inode index (no path resolution).
    pub fn read_file_by_inode(&self, idx: u32, offset: u64, len: u64) -> Result<Vec<u8>> {
        let inode = self.read_inode(idx)?;
        if inode.mode != INODE_FILE {
            bail!("not a file");
        }
        let mut dev = self.dev();
        file_io::read_data(&mut **dev, &inode, self.block_size, offset, len)
    }

    /// Write data to a file by inode index (no path resolution).
    pub fn write_file_by_inode(&self, idx: u32, offset: u64, data: &[u8]) -> Result<()> {
        self.check_writable()?;
        let mut inode = self.read_inode(idx)?;
        if inode.mode != INODE_FILE {
            bail!("not a file");
        }
        let bs = self.block_size;
        self.with_alloc(|dev, alloc, sb| {
            file_io::write_data(dev, alloc, sb, &mut inode, bs, offset, data)
        })?;
        self.write_inode(idx, &inode)
    }

    /// Truncate a file by inode index (no path resolution).
    pub fn truncate_by_inode(&self, idx: u32, new_size: u64) -> Result<()> {
        self.check_writable()?;
        let mut inode = self.read_inode(idx)?;
        if inode.mode != INODE_FILE {
            bail!("not a file");
        }
        let bs = self.block_size;
        let secure = self.should_secure_delete();
        self.with_alloc(|dev, alloc, sb| {
            file_io::truncate(dev, alloc, sb, &mut inode, bs, new_size, secure)
        })?;
        self.write_inode(idx, &inode)
    }

    // -----------------------------------------------------------------------
    // Phase 10I â€" Preallocation, Hole Punching, Defragmentation
    // -----------------------------------------------------------------------

    /// Preallocate disk space for a file without writing data.
    ///
    /// Allocated blocks are recorded as uninitialized extents —
    /// reads return zeros, writes convert to initialized.
    /// `inode.size` is NOT changed; `inode.block_count` increases.
    pub fn fallocate(&self, path: &str, offset: u64, length: u64) -> Result<()> {
        self.check_writable()?;
        let inode_idx = self.resolve_path_inline(path)?;
        let mut inode = self.read_inode(inode_idx)?;
        if inode.mode != INODE_FILE {
            bail!("fallocate: not a regular file");
        }
        if inode.flags & inode::INODE_FLAG_EXTENTS == 0 {
            bail!("fallocate: requires extent-based inode");
        }

        let bs = self.block_size as u64;
        let start_block = offset / bs;
        let end_block = (offset + length + bs - 1) / bs;
        let block_count = end_block - start_block;

        if block_count == 0 {
            return Ok(());
        }

        self.with_alloc(|dev, alloc, sb| {
            let hint_group = 0u32; // default hint
            let runs = alloc.alloc_contiguous(dev, sb, block_count, hint_group)?;

            // Insert as uninitialized extents
            let mut logical = start_block as u32;
            for run in &runs {
                let mut offset = 0u64;
                while offset < run.count {
                    let chunk = std::cmp::min(run.count - offset, extent::EXTENT_MAX_LEN_INIT as u64) as u16;
                    extent::extent_insert_uninit(
                        dev, &mut inode, alloc, sb,
                        logical, run.start + offset, chunk, self.block_size,
                    )?;
                    logical += chunk as u32;
                    offset += chunk as u64;
                }
            }

            inode.block_count += block_count as u32;
            Ok(())
        })?;

        self.write_inode(inode_idx, &inode)
    }

    /// Preallocate disk space for a file by inode index.
    pub fn fallocate_by_inode(&self, idx: u32, offset: u64, length: u64) -> Result<()> {
        self.check_writable()?;
        let mut inode = self.read_inode(idx)?;
        if inode.mode != INODE_FILE {
            bail!("fallocate: not a regular file");
        }
        if inode.flags & inode::INODE_FLAG_EXTENTS == 0 {
            bail!("fallocate: requires extent-based inode");
        }

        let bs = self.block_size as u64;
        let start_block = offset / bs;
        let end_block = (offset + length + bs - 1) / bs;
        let block_count = end_block - start_block;

        if block_count == 0 {
            return Ok(());
        }

        self.with_alloc(|dev, alloc, sb| {
            let hint_group = 0u32;
            let runs = alloc.alloc_contiguous(dev, sb, block_count, hint_group)?;

            let mut logical = start_block as u32;
            for run in &runs {
                // Chunk each run into extents of at most EXTENT_MAX_LEN_INIT (32767)
                let mut offset = 0u64;
                while offset < run.count {
                    let chunk = std::cmp::min(run.count - offset, extent::EXTENT_MAX_LEN_INIT as u64) as u16;
                    extent::extent_insert_uninit(
                        dev, &mut inode, alloc, sb,
                        logical, run.start + offset, chunk, self.block_size,
                    )?;
                    logical += chunk as u32;
                    offset += chunk as u64;
                }
            }

            inode.block_count += block_count as u32;
            Ok(())
        })?;

        self.write_inode(idx, &inode)
    }

    /// Punch a hole in a file — free blocks in a range without changing file size.
    ///
    /// Reads to the punched region return zeros (sparse hole).
    pub fn punch_hole(&self, path: &str, offset: u64, length: u64) -> Result<u64> {
        self.check_writable()?;
        let inode_idx = self.resolve_path_inline(path)?;
        self.punch_hole_by_inode(inode_idx, offset, length)
    }

    /// Punch a hole by inode index. Returns the number of blocks freed.
    pub fn punch_hole_by_inode(&self, idx: u32, offset: u64, length: u64) -> Result<u64> {
        self.check_writable()?;
        let mut inode = self.read_inode(idx)?;
        if inode.mode != INODE_FILE {
            bail!("punch_hole: not a regular file");
        }
        if inode.flags & inode::INODE_FLAG_EXTENTS == 0 {
            bail!("punch_hole: requires extent-based inode");
        }

        let bs = self.block_size as u64;
        let start_block = (offset / bs) as u32;
        let end_block = ((offset + length + bs - 1) / bs) as u32;
        let remove_count = end_block - start_block;

        if remove_count == 0 {
            return Ok(0);
        }

        let n = self.with_alloc(|dev, alloc, sb| {
            let freed = extent::extent_remove(
                dev, &mut inode, start_block, remove_count, self.block_size,
            )?;
            let n = freed.len() as u64;
            if !freed.is_empty() {
                alloc.free(dev, sb, &freed)?;
            }
            inode.block_count = inode.block_count.saturating_sub(n as u32);
            Ok(n)
        })?;

        self.write_inode(idx, &inode)?;
        Ok(n)
    }

    /// Report about a file's fragmentation state.
    pub fn fragmentation_report(&self, path: &str) -> Result<FragReport> {
        let inode_idx = self.resolve_path_inline(path)?;
        self.fragmentation_report_by_inode(inode_idx, path.to_string())
    }

    fn fragmentation_report_by_inode(&self, idx: u32, path: String) -> Result<FragReport> {
        let inode = self.read_inode(idx)?;
        if inode.mode != INODE_FILE {
            bail!("fragstat: not a regular file");
        }
        if inode.flags & inode::INODE_FLAG_EXTENTS == 0 {
            bail!("fragstat: requires extent-based inode");
        }

        let mut dev = self.dev();
        let extents = extent::extent_list_leaves(&mut **dev, &inode, self.block_size)?;

        let extent_count = extents.len() as u32;
        let total_blocks: u64 = extents.iter().map(|e| e.block_count() as u64).sum();
        let largest = extents.iter().map(|e| e.block_count() as u64).max().unwrap_or(0);
        let smallest = extents.iter().map(|e| e.block_count() as u64).min().unwrap_or(0);

        Ok(FragReport {
            path,
            file_size: inode.size,
            total_blocks,
            extent_count,
            contiguity_score: if extent_count == 0 {
                1.0
            } else {
                1.0 / extent_count as f64
            },
            largest_extent: largest,
            smallest_extent: smallest,
            needs_defrag: extent_count > 1,
        })
    }

    /// Defragment a single file by consolidating its extents into one
    /// contiguous run. Returns statistics about the operation.
    pub fn defragment_file(&self, path: &str) -> Result<DefragStats> {
        self.check_writable()?;
        let inode_idx = self.resolve_path_inline(path)?;
        self.defragment_by_inode(inode_idx)
    }

    /// Defragment a file by inode index.
    pub fn defragment_by_inode(&self, idx: u32) -> Result<DefragStats> {
        self.check_writable()?;
        let mut inode = self.read_inode(idx)?;
        if inode.mode != INODE_FILE {
            bail!("defrag: not a regular file");
        }
        if inode.flags & inode::INODE_FLAG_EXTENTS == 0 {
            bail!("defrag: requires extent-based inode");
        }

        let bs = self.block_size;
        let old_extents;
        {
            let mut dev = self.dev();
            old_extents = extent::extent_list_leaves(&mut **dev, &inode, bs)?;
        }

        let extent_count = old_extents.len() as u32;
        if extent_count <= 1 {
            return Ok(DefragStats {
                files_checked: 1,
                files_defragmented: 0,
                extents_before: extent_count,
                extents_after: extent_count,
                blocks_moved: 0,
                already_contiguous: 1,
                skipped_errors: 0,
            });
        }

        let total_blocks: u64 = old_extents.iter().map(|e| e.block_count() as u64).sum();
        let first_logical = old_extents[0].ee_block;

        self.with_alloc(|dev, alloc, sb| {
            // 1. Allocate new contiguous run
            let runs = alloc.alloc_contiguous(dev, sb, total_blocks, 0)?;

            // Verify we got a single contiguous run (best case)
            // If not, we still proceed — still fewer extents
            let new_runs = &runs;

            // 2. Copy data: old extents → new run(s)
            let mut dest_offset = 0u64;
            for ext in &old_extents {
                let phys = ext.physical_block();
                let count = ext.block_count() as u64;
                // Find dest run for this offset
                let mut remaining = count;
                let mut src_off = 0u64;
                let mut global_dest = dest_offset;

                while remaining > 0 {
                    // Find which new run contains global_dest
                    let mut run_base = 0u64;
                    let mut found = false;
                    for run in new_runs {
                        if global_dest >= run_base && global_dest < run_base + run.count {
                            let run_off = global_dest - run_base;
                            let copy_count = std::cmp::min(remaining, run.count - run_off);
                            for i in 0..copy_count {
                                let mut buf = vec![0u8; bs as usize];
                                dev.read((phys + src_off + i) * bs as u64, &mut buf)?;
                                dev.write((run.start + run_off + i) * bs as u64, &buf)?;
                            }
                            src_off += copy_count;
                            global_dest += copy_count;
                            remaining -= copy_count;
                            found = true;
                            break;
                        }
                        run_base += run.count;
                    }
                    if !found {
                        bail!("defrag: internal error mapping dest offset");
                    }
                }
                dest_offset += count;
            }

            // 3. Collect old physical blocks to free
            let mut old_blocks = Vec::new();
            for ext in &old_extents {
                for i in 0..ext.block_count() as u64 {
                    old_blocks.push(ext.physical_block() + i);
                }
            }

            // 4. Replace extent tree with new mapping
            extent::init_inode_extent_root(&mut inode);
            let mut logical = first_logical;
            for run in new_runs {
                extent::extent_insert(
                    dev, &mut inode, alloc, sb,
                    logical, run.start, run.count as u16, bs,
                )?;
                logical += run.count as u32;
            }

            // 5. Free old physical blocks
            alloc.free(dev, sb, &old_blocks)?;

            Ok(())
        })?;

        self.write_inode(idx, &inode)?;

        Ok(DefragStats {
            files_checked: 1,
            files_defragmented: 1,
            extents_before: extent_count,
            extents_after: 1,
            blocks_moved: total_blocks,
            already_contiguous: 0,
            skipped_errors: 0,
        })
    }

    /// Defragment all fragmented files on the volume.
    pub fn defragment_volume(&self) -> Result<DefragStats> {
        self.check_writable()?;
        let sb = self.sb_read();
        let inode_count = sb.inode_count;
        drop(sb);

        let mut stats = DefragStats {
            files_checked: 0,
            files_defragmented: 0,
            extents_before: 0,
            extents_after: 0,
            blocks_moved: 0,
            already_contiguous: 0,
            skipped_errors: 0,
        };

        for inode_idx in 0..inode_count {
            let inode = match self.read_inode(inode_idx) {
                Ok(i) => i,
                Err(_) => continue,
            };

            if inode.mode != INODE_FILE
                || inode.flags & inode::INODE_FLAG_EXTENTS == 0
            {
                continue;
            }

            stats.files_checked += 1;

            match self.defragment_by_inode(inode_idx) {
                Ok(file_stats) => {
                    stats.extents_before += file_stats.extents_before;
                    stats.extents_after += file_stats.extents_after;
                    stats.blocks_moved += file_stats.blocks_moved;
                    if file_stats.files_defragmented > 0 {
                        stats.files_defragmented += 1;
                    }
                    if file_stats.already_contiguous > 0 {
                        stats.already_contiguous += 1;
                    }
                }
                Err(e) => {
                    eprintln!("defrag inode {}: {}", inode_idx, e);
                    stats.skipped_errors += 1;
                }
            }
        }

        Ok(stats)
    }
}

// ---------------------------------------------------------------------------
// Fragmentation / defrag report structures
// ---------------------------------------------------------------------------

/// Report about a file's fragmentation state.
#[derive(Debug, Clone)]
pub struct FragReport {
    pub path: String,
    pub file_size: u64,
    pub total_blocks: u64,
    pub extent_count: u32,
    /// Contiguity score: 1.0 = perfect, lower = more fragmented.
    pub contiguity_score: f64,
    pub largest_extent: u64,
    pub smallest_extent: u64,
    pub needs_defrag: bool,
}

/// Statistics returned after defragmentation.
#[derive(Debug, Clone, Default)]
pub struct DefragStats {
    pub files_checked: u32,
    pub files_defragmented: u32,
    pub extents_before: u32,
    pub extents_after: u32,
    pub blocks_moved: u64,
    pub already_contiguous: u32,
    pub skipped_errors: u32,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use tempfile::NamedTempFile;

    fn make_dev(size: u64) -> (NamedTempFile, Box<dyn CFSBlockDevice>) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(size)).unwrap();
        (tmp, Box::new(dev))
    }

    #[test]
    fn test_format_then_mount() {
        let (tmp, dev) = make_dev(1_048_576); // 1 MB
        let vol = CFSVolume::format(dev, DEFAULT_BLOCK_SIZE).unwrap();
        let sb1 = vol.sb_read().clone();
        drop(vol);

        let path = tmp.path().to_path_buf();
        let dev2 = FileBlockDevice::open(&path, None).unwrap();
        let vol2 = CFSVolume::mount(Box::new(dev2), DEFAULT_BLOCK_SIZE).unwrap();
        assert_eq!(vol2.sb_read().magic, CFS_MAGIC);
        assert_eq!(vol2.sb_read().total_blocks, sb1.total_blocks);
        // free_blocks should match (mount increments mount_count but doesn't change free_blocks)
        assert_eq!(vol2.sb_read().free_blocks, sb1.free_blocks);
        assert_eq!(vol2.sb_read().version, CFS_VERSION);
        // Mount should have incremented mount_count
        assert_eq!(vol2.sb_read().mount_count, 1);
    }

    #[test]
    fn test_format_creates_root_inode() {
        let (_tmp, dev) = make_dev(1_048_576);
        let vol = CFSVolume::format(dev, DEFAULT_BLOCK_SIZE).unwrap();
        let root = vol.read_inode(0).unwrap();
        assert_eq!(root.mode, INODE_DIR);
        assert_eq!(root.nlinks, 2);
    }

    #[test]
    fn test_mount_bad_magic() {
        let (tmp, mut dev) = make_dev(1_048_576);
        // Write garbage to block 0
        let garbage = vec![0xFFu8; DEFAULT_BLOCK_SIZE as usize];
        dev.write(0, &garbage).unwrap();
        dev.flush().unwrap();
        drop(dev);

        let path = tmp.path().to_path_buf();
        let dev2 = FileBlockDevice::open(&path, None).unwrap();
        assert!(CFSVolume::mount(Box::new(dev2), DEFAULT_BLOCK_SIZE).is_err());
    }

    #[test]
    fn test_allocate_and_sync() {
        let (tmp, dev) = make_dev(1_048_576);
        let vol = CFSVolume::format(dev, DEFAULT_BLOCK_SIZE).unwrap();
        let initial_free = vol.sb_read().free_blocks;

        let blocks = vol.allocate(5).unwrap();
        assert_eq!(blocks.len(), 5);
        assert_eq!(vol.sb_read().free_blocks, initial_free - 5);

        vol.sync().unwrap();
        drop(vol);

        // Re-mount and verify
        let path = tmp.path().to_path_buf();
        let dev2 = FileBlockDevice::open(&path, None).unwrap();
        let vol2 = CFSVolume::mount(Box::new(dev2), DEFAULT_BLOCK_SIZE).unwrap();
        assert_eq!(vol2.sb_read().free_blocks, initial_free - 5);
    }

    // -----------------------------------------------------------------------
    // 3D â€” Inode Allocation Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_alloc_inode_sequential() {
        let (_tmp, dev) = make_dev(1_048_576); // 1 MB
        let vol = CFSVolume::format(dev, DEFAULT_BLOCK_SIZE).unwrap();
        let mut indices = Vec::new();
        for _ in 0..5 {
            indices.push(vol.alloc_inode().unwrap());
        }
        // Should skip inode 0 (root) and return 1, 2, 3, 4, 5
        assert_eq!(indices, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_free_inode_reuse() {
        let (_tmp, dev) = make_dev(1_048_576);
        let vol = CFSVolume::format(dev, DEFAULT_BLOCK_SIZE).unwrap();

        let i1 = vol.alloc_inode().unwrap(); // 1
        let i2 = vol.alloc_inode().unwrap(); // 2
        let i3 = vol.alloc_inode().unwrap(); // 3

        // Mark inode 2 as allocated (write a file inode)
        vol.write_inode(i1, &Inode::new_file()).unwrap();
        vol.write_inode(i2, &Inode::new_file()).unwrap();
        vol.write_inode(i3, &Inode::new_file()).unwrap();

        // Free inode 2
        vol.free_inode(i2).unwrap();

        // Next alloc should return 2 (reuses freed slot)
        let reused = vol.alloc_inode().unwrap();
        assert_eq!(reused, 2);
    }

    #[test]
    fn test_alloc_inode_exhaustion() {
        // V2 layout needs: SB(1) + inode_table(1) + inode_bitmap(1) + data_bitmap(1)
        //   + data(4) + backup_SB(1) = 9 blocks minimum for inode_count=2.
        // Root takes inode 0, so only 1 allocatable inode.
        let (_tmp, dev) = make_dev(9 * 4096);
        let vol = CFSVolume::format(dev, DEFAULT_BLOCK_SIZE).unwrap();
        // The inode count is very small â€” try to exhaust it
        let mut count = 0u32;
        loop {
            match vol.alloc_inode() {
                Ok(idx) => {
                    vol.write_inode(idx, &Inode::new_file()).unwrap();
                    count += 1;
                }
                Err(_) => break,
            }
        }
        // We should have been able to allocate (inode_count - 1) inodes
        assert_eq!(count, vol.sb_read().inode_count - 1);
    }

    #[test]
    fn test_free_inode_resets_mode() {
        let (_tmp, dev) = make_dev(1_048_576);
        let vol = CFSVolume::format(dev, DEFAULT_BLOCK_SIZE).unwrap();

        let idx = vol.alloc_inode().unwrap();
        vol.write_inode(idx, &Inode::new_file()).unwrap();
        let inode = vol.read_inode(idx).unwrap();
        assert_eq!(inode.mode, INODE_FILE);

        vol.free_inode(idx).unwrap();
        let freed = vol.read_inode(idx).unwrap();
        assert_eq!(freed.mode, INODE_UNUSED);
    }

    // -----------------------------------------------------------------------
    // 3G â€” High-Level File API Tests
    // -----------------------------------------------------------------------

    fn make_vol(size: u64) -> (NamedTempFile, CFSVolume) {
        let (tmp, dev) = make_dev(size);
        let vol = CFSVolume::format(dev, DEFAULT_BLOCK_SIZE).unwrap();
        (tmp, vol)
    }

    #[test]
    fn test_create_and_read_file() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.create_file("/test.txt").unwrap();
        vol.write_file("/test.txt", 0, b"hello").unwrap();
        let data = vol.read_file("/test.txt", 0, 5).unwrap();
        assert_eq!(&data, b"hello");
    }

    #[test]
    fn test_create_file_nested() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/dir").unwrap();
        vol.create_file("/dir/test.txt").unwrap();
        vol.write_file("/dir/test.txt", 0, b"nested").unwrap();
        let data = vol.read_file("/dir/test.txt", 0, 6).unwrap();
        assert_eq!(&data, b"nested");
    }

    #[test]
    fn test_write_file_10kb() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.create_file("/big.bin").unwrap();
        let data: Vec<u8> = (0..10240).map(|i| (i % 251) as u8).collect();
        vol.write_file("/big.bin", 0, &data).unwrap();
        let read_back = vol.read_file("/big.bin", 0, 10240).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_write_file_indirect() {
        let (_tmp, vol) = make_vol(2 * 1024 * 1024);
        vol.create_file("/large.bin").unwrap();
        // 45 KB â€” requires direct (40 KB) + indirect blocks
        let data: Vec<u8> = (0..45_000).map(|i| (i % 199) as u8).collect();
        vol.write_file("/large.bin", 0, &data).unwrap();
        let read_back = vol.read_file("/large.bin", 0, 45_000).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_delete_file() {
        let (_tmp, vol) = make_vol(1_048_576);
        let initial_free = vol.sb_read().free_blocks;
        vol.create_file("/doomed.txt").unwrap();
        vol.write_file("/doomed.txt", 0, b"will be deleted").unwrap();
        let free_after_write = vol.sb_read().free_blocks;
        assert!(free_after_write < initial_free);

        vol.delete_file("/doomed.txt").unwrap();
        // Blocks should be restored
        assert_eq!(vol.sb_read().free_blocks, initial_free);

        // File should no longer exist
        assert!(!vol.exists("/doomed.txt").unwrap());
    }

    #[test]
    fn test_create_duplicate_fails() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.create_file("/a.txt").unwrap();
        assert!(vol.create_file("/a.txt").is_err());
    }

    #[test]
    fn test_truncate_file() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.create_file("/trunc.txt").unwrap();
        let data: Vec<u8> = (0..10240).map(|i| (i % 101) as u8).collect();
        vol.write_file("/trunc.txt", 0, &data).unwrap();

        vol.truncate("/trunc.txt", 1024).unwrap();
        let read_back = vol.read_file("/trunc.txt", 0, 10240).unwrap();
        assert_eq!(read_back.len(), 1024);
        assert_eq!(&read_back, &data[..1024]);
    }

    // -----------------------------------------------------------------------
    // 3H â€” High-Level Directory API Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_mkdir_and_list() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/mydir").unwrap();
        let entries = vol.list_dir("/").unwrap();
        assert!(entries.iter().any(|e| e.name_str() == "mydir"));
    }

    #[test]
    fn test_nested_mkdir() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/a").unwrap();
        vol.mkdir("/a/b").unwrap();
        vol.mkdir("/a/b/c").unwrap();
        let entries = vol.list_dir("/a/b").unwrap();
        assert!(entries.iter().any(|e| e.name_str() == "c"));
    }

    #[test]
    fn test_rmdir_empty() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/tmp").unwrap();
        vol.rmdir("/tmp").unwrap();
        let entries = vol.list_dir("/").unwrap();
        assert!(!entries.iter().any(|e| e.name_str() == "tmp"));
    }

    #[test]
    fn test_rmdir_non_empty_fails() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/dir").unwrap();
        vol.create_file("/dir/f.txt").unwrap();
        assert!(vol.rmdir("/dir").is_err());
    }

    #[test]
    fn test_stat_file() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.create_file("/f.txt").unwrap();
        vol.write_file("/f.txt", 0, &[0u8; 100]).unwrap();
        let inode = vol.stat("/f.txt").unwrap();
        assert_eq!(inode.size, 100);
        assert_eq!(inode.mode, INODE_FILE);
    }

    #[test]
    fn test_stat_dir() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/d").unwrap();
        let inode = vol.stat("/d").unwrap();
        assert_eq!(inode.mode, INODE_DIR);
        assert_eq!(inode.nlinks, 2);
    }

    #[test]
    fn test_exists() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.create_file("/x.txt").unwrap();
        assert!(vol.exists("/x.txt").unwrap());
        assert!(!vol.exists("/y.txt").unwrap());
    }

    // -----------------------------------------------------------------------
    // 3I â€” Rename & Move Tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rename_same_dir() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.create_file("/a.txt").unwrap();
        vol.write_file("/a.txt", 0, b"aaa").unwrap();
        vol.rename("/a.txt", "/b.txt").unwrap();
        assert!(!vol.exists("/a.txt").unwrap());
        let data = vol.read_file("/b.txt", 0, 3).unwrap();
        assert_eq!(&data, b"aaa");
    }

    #[test]
    fn test_rename_across_dirs() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/d1").unwrap();
        vol.mkdir("/d2").unwrap();
        vol.create_file("/d1/f.txt").unwrap();
        vol.write_file("/d1/f.txt", 0, b"moved").unwrap();
        vol.rename("/d1/f.txt", "/d2/f.txt").unwrap();
        assert!(!vol.exists("/d1/f.txt").unwrap());
        let data = vol.read_file("/d2/f.txt", 0, 5).unwrap();
        assert_eq!(&data, b"moved");
    }

    #[test]
    fn test_rename_overwrites_file() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.create_file("/a.txt").unwrap();
        vol.write_file("/a.txt", 0, b"aaa").unwrap();
        vol.create_file("/b.txt").unwrap();
        vol.write_file("/b.txt", 0, b"bbb").unwrap();
        vol.rename("/a.txt", "/b.txt").unwrap();
        let data = vol.read_file("/b.txt", 0, 3).unwrap();
        assert_eq!(&data, b"aaa");
    }

    #[test]
    fn test_rename_dir() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/old").unwrap();
        vol.create_file("/old/f.txt").unwrap();
        vol.write_file("/old/f.txt", 0, b"inner").unwrap();
        vol.rename("/old", "/new").unwrap();
        assert!(!vol.exists("/old").unwrap());
        let data = vol.read_file("/new/f.txt", 0, 5).unwrap();
        assert_eq!(&data, b"inner");
    }

    #[test]
    fn test_rename_to_existing_dir_fails() {
        let (_tmp, vol) = make_vol(1_048_576);
        vol.mkdir("/a").unwrap();
        vol.mkdir("/b").unwrap();
        assert!(vol.rename("/a", "/b").is_err());
    }

    // -----------------------------------------------------------------------
    // 3K â€” Integration / E2E Test
    // -----------------------------------------------------------------------

    #[test]
    fn test_end_to_end_workflow() {
        let (_tmp, vol) = make_vol(2 * 1024 * 1024);

        // Create directory structure
        vol.mkdir("/docs").unwrap();
        vol.mkdir("/docs/notes").unwrap();

        // Create and write files
        vol.create_file("/readme.txt").unwrap();
        vol.write_file("/readme.txt", 0, b"Welcome to CFS!").unwrap();

        vol.create_file("/docs/plan.md").unwrap();
        vol.write_file("/docs/plan.md", 0, b"# Phase 3 Plan").unwrap();

        vol.create_file("/docs/notes/todo.txt").unwrap();
        vol.write_file("/docs/notes/todo.txt", 0, b"finish phase 3").unwrap();

        // Verify reads
        assert_eq!(vol.read_file("/readme.txt", 0, 100).unwrap(), b"Welcome to CFS!");
        assert_eq!(vol.read_file("/docs/plan.md", 0, 100).unwrap(), b"# Phase 3 Plan");

        // List directories
        let root_entries = vol.list_dir("/").unwrap();
        let root_names: Vec<&str> = root_entries.iter().map(|e| e.name_str()).collect();
        assert!(root_names.contains(&"docs"));
        assert!(root_names.contains(&"readme.txt"));

        // Rename
        vol.rename("/readme.txt", "/docs/readme.txt").unwrap();
        assert!(!vol.exists("/readme.txt").unwrap());
        assert_eq!(vol.read_file("/docs/readme.txt", 0, 100).unwrap(), b"Welcome to CFS!");

        // Truncate
        vol.truncate("/docs/readme.txt", 7).unwrap();
        assert_eq!(vol.read_file("/docs/readme.txt", 0, 100).unwrap(), b"Welcome");

        // Delete
        vol.delete_file("/docs/notes/todo.txt").unwrap();
        assert!(!vol.exists("/docs/notes/todo.txt").unwrap());

        // Rmdir (empty after delete)
        vol.rmdir("/docs/notes").unwrap();
        assert!(!vol.exists("/docs/notes").unwrap());

        // Stat
        let plan_stat = vol.stat("/docs/plan.md").unwrap();
        assert_eq!(plan_stat.mode, INODE_FILE);
        assert_eq!(plan_stat.size, 14); // "# Phase 3 Plan"
    }

    // -----------------------------------------------------------------------
    // 10A.1 â€” FormatOptions / MountOptions tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_options_default_validates() {
        assert!(FormatOptions::default().validate().is_ok());
    }

    #[test]
    fn test_format_options_all_presets_validate() {
        assert!(FormatOptions::general_purpose().validate().is_ok());
        assert!(FormatOptions::large_files().validate().is_ok());
        assert!(FormatOptions::small_files().validate().is_ok());
        assert!(FormatOptions::max_security().validate().is_ok());
        assert!(FormatOptions::minimal_legacy().validate().is_ok());
    }

    #[test]
    fn test_format_options_bad_block_size() {
        let mut o = FormatOptions::default();
        o.block_size = 100;
        assert!(o.validate().is_err());
        o.block_size = 0;
        assert!(o.validate().is_err());
        o.block_size = 131072;
        assert!(o.validate().is_err());
    }

    #[test]
    fn test_format_options_bad_inode_size() {
        let mut o = FormatOptions::default();
        o.inode_size = 64;
        assert!(o.validate().is_err());
        o.inode_size = 512;
        assert!(o.validate().is_err());
    }

    #[test]
    fn test_format_options_inode_gt_block() {
        let mut o = FormatOptions::default();
        o.inode_size = 256;
        o.block_size = 128;
        assert!(o.validate().is_err());
    }

    #[test]
    fn test_format_options_bad_inode_ratio() {
        let mut o = FormatOptions::default();
        o.inode_ratio = 512;
        assert!(o.validate().is_err());
        o.inode_ratio = 100_000;
        assert!(o.validate().is_err());
    }

    #[test]
    fn test_format_options_bad_journal() {
        let mut o = FormatOptions::default();
        o.journal_percent = 0.3;
        assert!(o.validate().is_err());
        o.journal_percent = 10.0;
        assert!(o.validate().is_err());
    }

    #[test]
    fn test_format_options_bad_label() {
        let mut o = FormatOptions::default();
        o.volume_label = "x".repeat(32);
        assert!(o.validate().is_err());
    }

    #[test]
    fn test_format_options_bad_blocks_per_group() {
        let mut o = FormatOptions::default();
        o.blocks_per_group = o.block_size * 8 + 1;
        assert!(o.validate().is_err());
    }

    #[test]
    fn test_mount_options_default() {
        let m = MountOptions::default();
        assert_eq!(m.cache_inodes, 256);
        assert_eq!(m.cache_blocks, 512);
        assert!(m.secure_delete);
        assert_eq!(m.atime_mode, AtimeMode::Relatime);
        assert!(!m.read_only);
    }

    // --- 10A.5: CFSVolume API refactor tests ---

    #[test]
    fn test_format_v3_default() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();

        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        assert_eq!(vol.sb_read().version, CFS_VERSION_V3);
        assert_eq!(vol.inode_size, 256);

        // Should have root dir with "." and ".."
        let entries = vol.list_dir("/").unwrap();
        let names: Vec<String> = entries.iter().map(|e| {
            String::from_utf8_lossy(&e.name[..e.name_len as usize]).to_string()
        }).collect();
        assert!(names.iter().any(|n| n == "."));
        assert!(names.iter().any(|n| n == ".."));
    }

    #[test]
    fn test_format_v3_large_files_preset() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(100 * 1024 * 1024)).unwrap();

        let opts = FormatOptions::large_files();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        assert_eq!(vol.sb_read().version, CFS_VERSION_V3);
        assert_eq!(vol.sb_read().block_size, 16384);
        assert_eq!(vol.sb_read().inode_size, 256);
    }

    #[test]
    fn test_format_v3_with_journal() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(10 * 1024 * 1024)).unwrap();

        let mut opts = FormatOptions::default();
        opts.journal_percent = 1.0;
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        assert!(vol.sb_read().journal_blocks > 0);
        assert!(vol.sb_read().journal_start > 0);
    }

    #[test]
    fn test_format_v3_without_journal() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(10 * 1024 * 1024)).unwrap();

        let mut opts = FormatOptions::default();
        opts.journal_percent = 0.0;
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        assert_eq!(vol.sb_read().journal_start, 0);
        assert_eq!(vol.sb_read().journal_blocks, 0);
    }

    #[test]
    fn test_format_v3_256b_inodes() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();

        let opts = FormatOptions::default(); // inode_size=256
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();
        assert_eq!(vol.inode_size, 256);

        // Create and read a file
        vol.create_file("/test.txt").unwrap();
        vol.write_file("/test.txt", 0, b"hello v3").unwrap();
        let data = vol.read_file("/test.txt", 0, 8).unwrap();
        assert_eq!(data, b"hello v3");
    }

    #[test]
    fn test_format_v3_128b_legacy() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();

        let opts = FormatOptions::minimal_legacy(); // inode_size=128
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();
        assert_eq!(vol.inode_size, 128);

        // Should still work
        vol.create_file("/test.txt").unwrap();
        vol.write_file("/test.txt", 0, b"hello legacy").unwrap();
        let data = vol.read_file("/test.txt", 0, 12).unwrap();
        assert_eq!(data, b"hello legacy");
    }

    #[test]
    fn test_format_v2_still_works() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();

        let vol = CFSVolume::format(Box::new(dev), 4096).unwrap();
        assert_eq!(vol.sb_read().version, CFS_VERSION);
        assert_eq!(vol.inode_size, INODE_SIZE);

        vol.create_file("/v2file.txt").unwrap();
        vol.write_file("/v2file.txt", 0, b"v2 works").unwrap();
        let data = vol.read_file("/v2file.txt", 0, 8).unwrap();
        assert_eq!(data, b"v2 works");
    }

    #[test]
    fn test_mount_v3_reads_v2() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        // Format as v2
        {
            let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
            let _vol = CFSVolume::format(Box::new(dev), 4096).unwrap();
        }

        // Mount with mount_v3
        let dev = FileBlockDevice::open(&path, None).unwrap();
        let vol = CFSVolume::mount_v3(Box::new(dev), 4096, &MountOptions::default()).unwrap();
        assert_eq!(vol.sb_read().version, CFS_VERSION);
        assert_eq!(vol.inode_size, INODE_SIZE); // 128
    }

    // -----------------------------------------------------------------------
    // 10B.8.7 â€” Group volume integration tests
    // -----------------------------------------------------------------------

    /// Helper: build FormatOptions with a small blocks_per_group so even a
    /// 4 MB image spans multiple block groups (group_count > 1).
    fn small_group_opts() -> FormatOptions {
        FormatOptions {
            block_size: 4096,
            inode_size: 256,
            inode_ratio: 16384,
            journal_percent: 0.0,
            volume_label: String::new(),
            secure_delete: false,
            default_permissions: 0o755,
            error_behavior: ErrorBehavior::Continue,
            // 128 blocks/group Ã— 4096 B = 512 KB per group â†’ ~8 groups in 4 MB
            blocks_per_group: 128,
        }
    }

    #[test]
    fn test_format_v3_creates_groups() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();

        let opts = small_group_opts();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        // With 128 blocks/group we must have more than 1 group
        assert!(vol.sb_read().group_count > 1,
            "expected multiple block groups, got {}", vol.sb_read().group_count);
        assert!(vol.gbm_lock().is_some());
        assert!(vol.gibm_lock().is_some());
        assert!(vol.group_inode_table.is_some());
        assert!(!vol.gdt_lock().is_empty());
    }

    #[test]
    fn test_format_v3_multi_group_create_file() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();

        let opts = small_group_opts();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        // Create several files across the multi-group volume
        vol.create_file("/alpha.txt").unwrap();
        vol.write_file("/alpha.txt", 0, b"alpha data").unwrap();

        vol.create_file("/beta.txt").unwrap();
        vol.write_file("/beta.txt", 0, b"beta data").unwrap();

        vol.mkdir("/subdir").unwrap();
        vol.create_file("/subdir/gamma.txt").unwrap();
        vol.write_file("/subdir/gamma.txt", 0, b"gamma data").unwrap();

        assert_eq!(vol.read_file("/alpha.txt", 0, 10).unwrap(), b"alpha data");
        assert_eq!(vol.read_file("/beta.txt", 0, 9).unwrap(), b"beta data");
        assert_eq!(vol.read_file("/subdir/gamma.txt", 0, 10).unwrap(), b"gamma data");
    }

    #[test]
    fn test_format_v3_multi_group_mount_persists() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        // Format multi-group + write file
        {
            let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
            let opts = small_group_opts();
            let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();
            vol.create_file("/persist.txt").unwrap();
            vol.write_file("/persist.txt", 0, b"persistent").unwrap();
            vol.sync().unwrap();
        }

        // Re-mount and read back
        {
            let dev = FileBlockDevice::open(&path, None).unwrap();
            let vol = CFSVolume::mount_v3(Box::new(dev), 4096, &MountOptions::default()).unwrap();
            assert!(vol.sb_read().group_count > 1);
            let data = vol.read_file("/persist.txt", 0, 10).unwrap();
            assert_eq!(data, b"persistent");
        }
    }

    #[test]
    fn test_format_v3_multi_group_list_dir() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();

        let opts = small_group_opts();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        vol.create_file("/a.txt").unwrap();
        vol.create_file("/b.txt").unwrap();
        vol.mkdir("/dir").unwrap();

        let entries = vol.list_dir("/").unwrap();
        let names: Vec<String> = entries.iter()
            .map(|e| String::from_utf8_lossy(&e.name[..e.name_len as usize]).to_string())
            .collect();

        assert!(names.contains(&"a.txt".to_string()));
        assert!(names.contains(&"b.txt".to_string()));
        assert!(names.contains(&"dir".to_string()));
    }

    /// Regression: inode_ratio must be read from opts/superblock, NOT hardcoded to 16384.
    /// large_files() uses inode_ratio=65536 â†’ less inodes â†’ smaller overhead_per_group.
    /// With the old hardcode the GroupBitmapManager would use the wrong overhead and
    /// either lose allocatable blocks or corrupt the inode table.
    #[test]
    fn test_format_v3_large_files_file_io() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(100 * 1024 * 1024)).unwrap();

        let opts = FormatOptions::large_files(); // inode_ratio=65536
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();
        assert_eq!(vol.sb_read().inode_ratio, 65536);

        // Write/read data â€” exercises block allocation with the non-default inode_ratio
        vol.create_file("/large.bin").unwrap();
        let payload: Vec<u8> = (0..8192).map(|i| (i % 251) as u8).collect();
        vol.write_file("/large.bin", 0, &payload).unwrap();
        let back = vol.read_file("/large.bin", 0, 8192).unwrap();
        assert_eq!(back, payload);
    }

    /// Regression: inode_ratio=4096 (small_files preset) gives MORE inodes per group â†’
    /// larger overhead_per_group. With the old hardcode data would be written into
    /// inode-table space.
    #[test]
    fn test_format_v3_small_files_file_io() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(8 * 1024 * 1024)).unwrap();

        let opts = FormatOptions::small_files(); // inode_ratio=4096
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();
        assert_eq!(vol.sb_read().inode_ratio, 4096);

        for i in 0..4u32 {
            let name = format!("/file{i}.txt");
            vol.create_file(&name).unwrap();
            let data = format!("content of file {i}");
            vol.write_file(&name, 0, data.as_bytes()).unwrap();
        }

        for i in 0..4u32 {
            let name = format!("/file{i}.txt");
            let expected = format!("content of file {i}");
            let got = vol.read_file(&name, 0, expected.len() as u64).unwrap();
            assert_eq!(got, expected.as_bytes());
        }
    }

    // -----------------------------------------------------------------------
    // 10D â€” Integrity & Access Control tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_only_guard() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();
        vol.create_file("/test.txt").unwrap();
        vol.sync().unwrap();
        drop(vol);

        let dev2 = FileBlockDevice::open(&path, None).unwrap();
        let mopts = MountOptions { read_only: true, ..MountOptions::default() };
        let vol2 = CFSVolume::mount_v3(Box::new(dev2), 4096, &mopts).unwrap();
        assert!(vol2.is_read_only());

        // All write operations should fail
        assert!(vol2.create_file("/new.txt").is_err());
        assert!(vol2.write_file("/test.txt", 0, b"data").is_err());
        assert!(vol2.mkdir("/newdir").is_err());
        assert!(vol2.delete_file("/test.txt").is_err());
        assert!(vol2.truncate("/test.txt", 0).is_err());
        assert!(vol2.rename("/test.txt", "/moved.txt").is_err());

        // Read operations should succeed
        assert!(vol2.read_file("/test.txt", 0, 10).is_ok());
        assert!(vol2.list_dir("/").is_ok());
        assert!(vol2.stat("/test.txt").is_ok());
    }

    #[test]
    fn test_chmod() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        vol.create_file("/file.txt").unwrap();
        vol.chmod("/file.txt", 0o644).unwrap();
        let st = vol.stat("/file.txt").unwrap();
        assert_eq!(st.permissions & 0o7777, 0o644);

        vol.chmod("/file.txt", 0o755).unwrap();
        let st2 = vol.stat("/file.txt").unwrap();
        assert_eq!(st2.permissions & 0o7777, 0o755);
    }

    #[test]
    fn test_chown() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        vol.create_file("/file.txt").unwrap();
        vol.chown("/file.txt", Some(1000), Some(1000)).unwrap();
        let st = vol.stat("/file.txt").unwrap();
        assert_eq!(st.owner_id, 1000);
        assert_eq!(st.group_id, 1000);

        // Change only owner
        vol.chown("/file.txt", Some(2000), None).unwrap();
        let st2 = vol.stat("/file.txt").unwrap();
        assert_eq!(st2.owner_id, 2000);
        assert_eq!(st2.group_id, 1000); // unchanged

        // Change only group
        vol.chown("/file.txt", None, Some(3000)).unwrap();
        let st3 = vol.stat("/file.txt").unwrap();
        assert_eq!(st3.owner_id, 2000); // unchanged
        assert_eq!(st3.group_id, 3000);
    }

    #[test]
    fn test_v3_format_mount_roundtrip_with_integrity() {
        // Format v3 â†’ create files â†’ sync â†’ remount â†’ verify data
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        // HMAC feature should be enabled
        assert_ne!(vol.sb_read().features_flags & superblock::FEATURE_METADATA_HMAC, 0);

        vol.create_file("/hello.txt").unwrap();
        vol.write_file("/hello.txt", 0, b"hello world").unwrap();
        vol.mkdir("/subdir").unwrap();
        vol.create_file("/subdir/nested.txt").unwrap();
        vol.write_file("/subdir/nested.txt", 0, b"nested content").unwrap();
        vol.sync().unwrap();

        // HMAC should now be non-zero
        assert_ne!(vol.sb_read().metadata_hmac, [0u8; 8]);
        drop(vol);

        // Remount and verify
        let dev2 = FileBlockDevice::open(&path, None).unwrap();
        let vol2 = CFSVolume::mount_v3(Box::new(dev2), 4096, &MountOptions::default()).unwrap();
        assert_eq!(vol2.sb_read().version, CFS_VERSION_V3);

        let data = vol2.read_file("/hello.txt", 0, 11).unwrap();
        assert_eq!(&data, b"hello world");

        let data2 = vol2.read_file("/subdir/nested.txt", 0, 14).unwrap();
        assert_eq!(&data2, b"nested content");

        let entries = vol2.list_dir("/subdir").unwrap();
        assert!(entries.iter().any(|e| e.name_str() == "nested.txt"));
    }

    #[test]
    fn test_bitmap_checksums_roundtrip() {
        // Format v3, write files (allocate blocks), sync, remount â€” verify bitmaps load without error
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        // Allocate some blocks by creating files
        for i in 0..10 {
            let name = format!("/f{i}.bin");
            vol.create_file(&name).unwrap();
            vol.write_file(&name, 0, &vec![0xAB; 4096]).unwrap();
        }
        vol.sync().unwrap();
        drop(vol);

        // Remount â€” should load bitmap checksums without error
        let dev2 = FileBlockDevice::open(&path, None).unwrap();
        let vol2 = CFSVolume::mount_v3(Box::new(dev2), 4096, &MountOptions::default()).unwrap();
        // Verify files are still readable
        for i in 0..10 {
            let name = format!("/f{i}.bin");
            let data = vol2.read_file(&name, 0, 4096).unwrap();
            assert_eq!(data, vec![0xAB; 4096]);
        }
    }

    #[test]
    fn test_dir_checksum_v3_entries() {
        // Format v3, create many files in a directory, verify list_dir works (checksums verified internally)
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        // Create enough files to fill the root dir block and exercise checksum logic
        let count = 20;
        for i in 0..count {
            let name = format!("/entry{i:03}.txt");
            vol.create_file(&name).unwrap();
        }
        vol.sync().unwrap();

        let entries = vol.list_dir("/").unwrap();
        // Should contain all files plus . and ..
        let file_entries: Vec<_> = entries.iter().filter(|e| e.name_str().starts_with("entry")).collect();
        assert_eq!(file_entries.len(), count);
    }

    #[test]
    fn test_hmac_key_derivation_deterministic() {
        // Same input â†’ same key
        let k1 = superblock::derive_hmac_key(None);
        let k2 = superblock::derive_hmac_key(None);
        assert_eq!(k1, k2);

        // With a master key
        let master = [0x42u8; 32];
        let k3 = superblock::derive_hmac_key(Some(&master));
        let k4 = superblock::derive_hmac_key(Some(&master));
        assert_eq!(k3, k4);

        // Different inputs â†’ different keys
        assert_ne!(k1, k3);
    }

    #[test]
    fn test_nanosecond_timestamps_v3() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        vol.create_file("/ts.txt").unwrap();
        let st = vol.stat("/ts.txt").unwrap();
        // v3 inode timestamps should be recent (non-zero)
        assert!(st.changed_ns > 0);
        assert!(st.modified > 0);

        // Write to update mtime
        vol.write_file("/ts.txt", 0, b"data").unwrap();
        let st2 = vol.stat("/ts.txt").unwrap();
        assert!(st2.modified >= st.modified);
    }

    // -----------------------------------------------------------------------
    // Journal integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_journal_create_delete_status() {
        // Format with journal, do mutations, check journal_status reports transactions
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(8 * 1024 * 1024)).unwrap();
        let mut opts = FormatOptions::default();
        opts.journal_percent = 2.0;
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        assert!(vol.has_journal());
        let st = vol.journal_status().unwrap();
        assert!(st.capacity > 0);
        assert!(st.clean); // freshly formatted

        // Create and delete files to generate journal transactions
        vol.create_file("/a.txt").unwrap();
        vol.write_file("/a.txt", 0, b"hello").unwrap();
        vol.create_file("/b.txt").unwrap();
        vol.delete_file("/b.txt").unwrap();

        let st2 = vol.journal_status().unwrap();
        assert!(st2.sequence > st.sequence, "sequence should advance with txns");
    }

    #[test]
    fn test_journal_survives_remount() {
        // Format with journal, create file, sync, remount â€” file should persist
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(8 * 1024 * 1024)).unwrap();
        let mut opts = FormatOptions::default();
        opts.journal_percent = 2.0;
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        vol.create_file("/persist.txt").unwrap();
        vol.write_file("/persist.txt", 0, b"survive remount").unwrap();
        vol.mkdir("/mydir").unwrap();
        vol.sync().unwrap();
        drop(vol);

        let dev2 = FileBlockDevice::open(&path, None).unwrap();
        let vol2 = CFSVolume::mount_v3(Box::new(dev2), 4096, &MountOptions::default()).unwrap();
        assert!(vol2.has_journal());
        let data = vol2.read_file("/persist.txt", 0, 15).unwrap();
        assert_eq!(&data, b"survive remount");
        assert!(vol2.exists("/mydir").unwrap());
    }

    #[test]
    fn test_journal_no_journal_volume() {
        // Format without journal â€” journal_status returns None
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let mut opts = FormatOptions::default();
        opts.journal_percent = 0.0;
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        assert!(!vol.has_journal());
        assert!(vol.journal_status().is_none());
    }

    #[test]
    fn test_journal_many_ops_checkpoint() {
        // Many mutations on a small journal to verify auto-checkpoint prevents overflow
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default(); // default journal_percent=1.0
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        // Create and delete 30 files â€” should not overflow thanks to auto-checkpoint
        for i in 0..30 {
            let name = format!("/f{i}.dat");
            vol.create_file(&name).unwrap();
            vol.write_file(&name, 0, &[0u8; 128]).unwrap();
        }
        for i in 0..15 {
            let name = format!("/f{i}.dat");
            vol.delete_file(&name).unwrap();
        }
        vol.sync().unwrap();

        // Remaining 15 files should still be present
        let entries = vol.list_dir("/").unwrap();
        let files: Vec<_> = entries.iter().filter(|e| e.name_str().starts_with("f")).collect();
        assert_eq!(files.len(), 15);
    }

    #[test]
    fn test_journal_mkdir_rmdir_rename() {
        // Test journal wrapping for directory operations
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(8 * 1024 * 1024)).unwrap();
        let mut opts = FormatOptions::default();
        opts.journal_percent = 2.0;
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        vol.mkdir("/dir1").unwrap();
        vol.mkdir("/dir2").unwrap();
        vol.create_file("/dir1/file.txt").unwrap();
        vol.write_file("/dir1/file.txt", 0, b"content").unwrap();

        // Rename file across directories
        vol.rename("/dir1/file.txt", "/dir2/moved.txt").unwrap();
        assert!(!vol.exists("/dir1/file.txt").unwrap());
        let data = vol.read_file("/dir2/moved.txt", 0, 7).unwrap();
        assert_eq!(&data, b"content");

        // rmdir empty dir
        vol.rmdir("/dir1").unwrap();
        assert!(!vol.exists("/dir1").unwrap());

        vol.sync().unwrap();
    }

    #[test]
    fn test_journal_truncate_journaled() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(8 * 1024 * 1024)).unwrap();
        let mut opts = FormatOptions::default();
        opts.journal_percent = 2.0;
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        vol.create_file("/big.bin").unwrap();
        vol.write_file("/big.bin", 0, &[0xCC; 8192]).unwrap();
        let st = vol.stat("/big.bin").unwrap();
        assert_eq!(st.size, 8192);

        vol.truncate("/big.bin", 100).unwrap();
        let st2 = vol.stat("/big.bin").unwrap();
        assert_eq!(st2.size, 100);

        let data = vol.read_file("/big.bin", 0, 100).unwrap();
        assert_eq!(data, vec![0xCC; 100]);
    }

    // -----------------------------------------------------------------------
    // 10H — Concurrency tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_concurrent_reads_from_multiple_threads() {
        use std::sync::Arc;
        use std::thread;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        // Write test data
        vol.mkdir("/docs").unwrap();
        for i in 0..10 {
            let name = format!("/docs/file_{:02}.txt", i);
            let content = format!("content of file {}", i);
            vol.create_file(&name).unwrap();
            vol.write_file(&name, 0, content.as_bytes()).unwrap();
        }

        let vol = Arc::new(vol);
        let mut handles = Vec::new();

        // Spawn 8 reader threads, each reading all 10 files
        for t in 0..8 {
            let vol = Arc::clone(&vol);
            handles.push(thread::spawn(move || {
                for i in 0..10 {
                    let name = format!("/docs/file_{:02}.txt", i);
                    let expected = format!("content of file {}", i);
                    let data = vol.read_file(&name, 0, expected.len() as u64).unwrap();
                    assert_eq!(data, expected.as_bytes(),
                        "thread {} read wrong data for {}", t, name);

                    let entries = vol.list_dir("/docs").unwrap();
                    assert!(entries.len() >= 12,
                        "thread {} saw only {} entries", t, entries.len());

                    let st = vol.stat(&name).unwrap();
                    assert_eq!(st.size, expected.len() as u64,
                        "thread {} got wrong size for {}", t, name);
                }
            }));
        }

        for h in handles {
            h.join().expect("reader thread panicked");
        }
    }

    #[test]
    fn test_concurrent_reads_and_writes() {
        use std::sync::Arc;
        use std::thread;

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();
        // Disable journal — this test validates lock ordering, not journal concurrency.
        let opts = FormatOptions { journal_percent: 0.0, ..FormatOptions::default() };
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();

        // Pre-create files for readers
        for i in 0..5 {
            let name = format!("/pre_{}.txt", i);
            vol.create_file(&name).unwrap();
            vol.write_file(&name, 0, b"initial").unwrap();
        }

        let vol = Arc::new(vol);
        let mut handles = Vec::new();

        // 4 reader threads reading pre-existing files
        for t in 0..4 {
            let vol = Arc::clone(&vol);
            handles.push(thread::spawn(move || {
                for _ in 0..20 {
                    for i in 0..5 {
                        let name = format!("/pre_{}.txt", i);
                        let data = vol.read_file(&name, 0, 7).unwrap();
                        assert_eq!(data, b"initial",
                            "reader {} got wrong data for {}", t, name);
                    }
                }
            }));
        }

        // 2 writer threads creating new files (non-overlapping names)
        for t in 0..2 {
            let vol = Arc::clone(&vol);
            handles.push(thread::spawn(move || {
                for i in 0..10 {
                    let name = format!("/w{}_{}.txt", t, i);
                    vol.create_file(&name).unwrap();
                    vol.write_file(&name, 0, b"written").unwrap();
                    let data = vol.read_file(&name, 0, 7).unwrap();
                    assert_eq!(data, b"written");
                }
            }));
        }

        for h in handles {
            h.join().expect("concurrent thread panicked");
        }

        // Verify all writer files exist
        for t in 0..2 {
            for i in 0..10 {
                let name = format!("/w{}_{}.txt", t, i);
                assert!(vol.exists(&name).unwrap(), "missing: {}", name);
            }
        }
    }

    // -----------------------------------------------------------------------
    // 10I — Contiguous Allocator, Fallocate, Punch Hole, Defrag tests
    // -----------------------------------------------------------------------

    fn make_v3_vol(size_mb: u64) -> (NamedTempFile, CFSVolume) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let dev = FileBlockDevice::open(&path, Some(size_mb * 1024 * 1024)).unwrap();
        let opts = FormatOptions::default();
        let vol = CFSVolume::format_v3(Box::new(dev), &opts).unwrap();
        (tmp, vol)
    }

    #[test]
    fn test_fallocate_basic() {
        let (_tmp, vol) = make_v3_vol(4);
        vol.create_file("/prealloc.bin").unwrap();

        let inode_before = vol.read_inode(vol.resolve_path("/prealloc.bin").unwrap()).unwrap();
        let bc_before = inode_before.block_count;

        vol.fallocate("/prealloc.bin", 0, 40960).unwrap(); // 10 blocks

        let inode_after = vol.read_inode(vol.resolve_path("/prealloc.bin").unwrap()).unwrap();
        // block_count increased
        assert!(inode_after.block_count > bc_before);
        // size NOT changed (fallocate doesn't change size)
        assert_eq!(inode_after.size, 0);
    }

    #[test]
    fn test_fallocate_read_returns_zeros() {
        let (_tmp, vol) = make_v3_vol(4);
        vol.create_file("/zeros.bin").unwrap();

        // Fallocate 8 blocks (32768 bytes)
        vol.fallocate("/zeros.bin", 0, 32768).unwrap();

        // Manually set size so reads work (fallocate doesn't change size)
        {
            let idx = vol.resolve_path("/zeros.bin").unwrap();
            let mut inode = vol.read_inode(idx).unwrap();
            inode.size = 32768;
            vol.write_inode(idx, &inode).unwrap();
        }

        // Read should return zeros
        let data = vol.read_file("/zeros.bin", 0, 32768).unwrap();
        assert_eq!(data.len(), 32768);
        assert!(data.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_fallocate_write_converts_to_init() {
        let (_tmp, vol) = make_v3_vol(4);
        vol.create_file("/convert.bin").unwrap();

        // Fallocate 4 blocks
        vol.fallocate("/convert.bin", 0, 16384).unwrap();

        // Write data at offset 0
        let pattern = vec![0xABu8; 4096];
        vol.write_file("/convert.bin", 0, &pattern).unwrap();

        // Read back — should get our pattern
        let data = vol.read_file("/convert.bin", 0, 4096).unwrap();
        assert_eq!(data, pattern);
    }

    #[test]
    fn test_punch_hole_middle() {
        let (_tmp, vol) = make_v3_vol(4);
        vol.create_file("/holed.bin").unwrap();

        // Write 10 blocks of data
        let bs = 4096;
        for i in 0..10u64 {
            let data = vec![i as u8 + 1; bs];
            vol.write_file("/holed.bin", i * bs as u64, &data).unwrap();
        }

        // Punch hole in blocks 4-6 (3 blocks)
        let freed = vol.punch_hole("/holed.bin", 4 * bs as u64, 3 * bs as u64).unwrap();
        assert!(freed > 0);

        // Read back — blocks 4-6 should be zeros, others should have data
        let data = vol.read_file("/holed.bin", 0, 10 * bs as u64).unwrap();
        // Block 0 should be 0x01
        assert!(data[0..bs].iter().all(|&b| b == 1));
        // Block 3 should be 0x04
        assert!(data[3*bs..4*bs].iter().all(|&b| b == 4));
        // Block 4 should be zeros (punched)
        assert!(data[4*bs..5*bs].iter().all(|&b| b == 0));
        // Block 7 should be 0x08
        assert!(data[7*bs..8*bs].iter().all(|&b| b == 8));
    }

    #[test]
    fn test_punch_hole_preserves_size() {
        let (_tmp, vol) = make_v3_vol(4);
        vol.create_file("/preserve.bin").unwrap();
        let data = vec![0xFFu8; 20480]; // 5 blocks
        vol.write_file("/preserve.bin", 0, &data).unwrap();

        let idx = vol.resolve_path("/preserve.bin").unwrap();
        let size_before = vol.read_inode(idx).unwrap().size;

        vol.punch_hole("/preserve.bin", 4096, 8192).unwrap(); // punch blocks 1-2

        let size_after = vol.read_inode(idx).unwrap().size;
        assert_eq!(size_before, size_after);
    }

    #[test]
    fn test_fragmentation_report_single_extent() {
        let (_tmp, vol) = make_v3_vol(4);
        vol.create_file("/contiguous.bin").unwrap();
        vol.write_file("/contiguous.bin", 0, &vec![0u8; 8192]).unwrap();

        let report = vol.fragmentation_report("/contiguous.bin").unwrap();
        assert!(!report.needs_defrag);
        assert_eq!(report.contiguity_score, 1.0);
    }

    #[test]
    fn test_defrag_already_contiguous() {
        let (_tmp, vol) = make_v3_vol(4);
        vol.create_file("/already.bin").unwrap();
        vol.write_file("/already.bin", 0, &vec![0xABu8; 8192]).unwrap();

        let stats = vol.defragment_file("/already.bin").unwrap();
        assert_eq!(stats.files_defragmented, 0);
        assert_eq!(stats.already_contiguous, 1);
        assert_eq!(stats.blocks_moved, 0);

        // Data should be unchanged
        let data = vol.read_file("/already.bin", 0, 8192).unwrap();
        assert!(data.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn test_contiguous_bitmap_find_run() {
        use super::bitmap::Bitmap;

        let mut bm = Bitmap::new_all_free(100);
        // All free — should find run of 10 at bit 0
        assert_eq!(bm.find_contiguous_run(10, 0), Some(0));

        // Allocate bits 0-4
        for _ in 0..5 {
            bm.alloc();
        }

        // Now first free run of 10 should start at bit 5
        assert_eq!(bm.find_contiguous_run(10, 0), Some(5));

        // Fragment: allocate bit 10
        bm.set_allocated(10);
        // Run of 5 starting at 5 exists, but 10 blocks requires searching further
        assert_eq!(bm.find_contiguous_run(5, 0), Some(5));
        // Run of 6 starting at 5 is broken by bit 10 → search finds later run
        assert!(bm.find_contiguous_run(6, 0).is_some());
    }

    #[test]
    fn test_contiguous_bitmap_too_large() {
        use super::bitmap::Bitmap;

        let bm = Bitmap::new_all_free(100);
        // Can't find 200 contiguous in a 100-bit bitmap
        assert_eq!(bm.find_contiguous_run(200, 0), None);
    }
}
