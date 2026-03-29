use anyhow::{bail, Result};
use crc32fast::Hasher;

use crate::block_device::CFSBlockDevice;
use super::{CFS_MAGIC, CFS_VERSION, CFS_VERSION_V1, CFS_VERSION_V3, INODE_SIZE, MAX_INODE_COUNT, ROOT_INODE, ceil_div, round_up_to, FormatOptions, ErrorBehavior};

// ---------------------------------------------------------------------------
// Feature flags (bitfield in features_flags)
// ---------------------------------------------------------------------------

pub const FEATURE_HAS_INODE_BITMAP: u32 = 1 << 0;
pub const FEATURE_HAS_BACKUP_SB: u32 = 1 << 1;
pub const FEATURE_256B_INODES: u32 = 1 << 2;
pub const FEATURE_JOURNAL: u32 = 1 << 3;
pub const FEATURE_SECURE_DELETE: u32 = 1 << 4;
pub const FEATURE_XATTR: u32 = 1 << 5;
pub const FEATURE_SYMLINKS: u32 = 1 << 6;
pub const FEATURE_METADATA_HMAC: u32 = 1 << 7;
pub const FEATURE_EXTENTS: u32 = 1 << 8;
pub const FEATURE_BLOCK_GROUPS: u32 = 1 << 9;
pub const FEATURE_HTREE: u32 = 1 << 10;
pub const FEATURE_FLEX_BG: u32 = 1 << 11;
pub const FEATURE_DELAYED_ALLOC: u32 = 1 << 12;

/// Mask of all known feature bits for v2.
const KNOWN_FEATURES_V2: u32 = FEATURE_HAS_INODE_BITMAP | FEATURE_HAS_BACKUP_SB;

/// Mask of all known feature bits for v3.
const KNOWN_FEATURES_V3: u32 = FEATURE_HAS_INODE_BITMAP
    | FEATURE_HAS_BACKUP_SB
    | FEATURE_256B_INODES
    | FEATURE_JOURNAL
    | FEATURE_SECURE_DELETE
    | FEATURE_XATTR
    | FEATURE_SYMLINKS
    | FEATURE_METADATA_HMAC
    | FEATURE_EXTENTS
    | FEATURE_BLOCK_GROUPS
    | FEATURE_HTREE
    | FEATURE_FLEX_BG
    | FEATURE_DELAYED_ALLOC;

// ---------------------------------------------------------------------------
// Superblock v2 — 148 meaningful bytes, zero-padded to block_size
// ---------------------------------------------------------------------------

/// On-disk superblock.
///
/// **v2 layout (148 bytes):**
/// ```text
/// Offset  Size  Field
/// 0       4     magic               b"CFS1" or b"CFSE"
/// 4       4     version             2
/// 8       4     block_size          4096
/// 12      4     features_flags      bitfield
/// 16      8     total_blocks
/// 24      4     inode_count
/// 28      4     root_inode          always 0
/// 32      8     inode_table_start
/// 40      8     bitmap_start        (data block bitmap)
/// 48      8     data_start
/// 56      8     free_blocks
/// 64      8     inode_bitmap_start  (0 if feature disabled)
/// 72      4     free_inodes
/// 76      16    uuid                UUID v4
/// 92      32    volume_label        UTF-8, zero-padded
/// 124     4     mount_count
/// 128     8     last_mount_time     Unix timestamp (seconds)
/// 136     8     backup_sb_block     block # of backup (0 if none)
/// 144     4     checksum            CRC32 of bytes [0..144)
/// ```
#[derive(Debug, Clone)]
pub struct Superblock {
    // --- v1/v2 fields (offsets 0..148) ---
    pub magic: [u8; 4],
    pub version: u32,
    pub block_size: u32,
    pub features_flags: u32,
    pub total_blocks: u64,
    pub inode_count: u32,
    pub root_inode: u32,
    pub inode_table_start: u64,
    pub bitmap_start: u64,
    pub data_start: u64,
    pub free_blocks: u64,
    pub inode_bitmap_start: u64,
    pub free_inodes: u32,
    pub uuid: [u8; 16],
    pub volume_label: [u8; 32],
    pub mount_count: u32,
    pub last_mount_time: u64,
    pub backup_sb_block: u64,
    pub checksum: u32,

    // --- v3 fields (offsets 144..256) ---
    pub inode_size: u32,
    pub journal_start: u64,
    pub journal_blocks: u64,
    pub first_orphan_inode: u32,
    pub error_behavior: u32,
    pub default_permissions: u32,
    pub inode_ratio: u32,
    pub journal_sequence: u64,
    pub metadata_hmac: [u8; 8],
    pub dir_entry_size: u32,

    // --- Block group fields ---
    pub blocks_per_group: u32,
    pub inodes_per_group: u32,
    pub group_count: u32,
    pub gdt_start: u64,
    pub gdt_blocks: u32,
    pub desc_size: u32,
    pub reserved_gdt_blocks: u32,
    pub log_groups_per_flex: u32,
    pub first_group_block: u32,
    pub hash_seed: [u8; 8],
}

impl Default for Superblock {
    fn default() -> Self {
        Self {
            magic: *b"CFS1",
            version: 2,
            block_size: 4096,
            features_flags: 0,
            total_blocks: 0,
            inode_count: 0,
            root_inode: 0,
            inode_table_start: 0,
            bitmap_start: 0,
            data_start: 0,
            free_blocks: 0,
            inode_bitmap_start: 0,
            free_inodes: 0,
            uuid: [0; 16],
            volume_label: [0; 32],
            mount_count: 0,
            last_mount_time: 0,
            backup_sb_block: 0,
            checksum: 0,
            inode_size: 128,
            journal_start: 0,
            journal_blocks: 0,
            first_orphan_inode: 0,
            error_behavior: 0,
            default_permissions: 0o755,
            inode_ratio: 16384,
            journal_sequence: 0,
            metadata_hmac: [0; 8],
            dir_entry_size: 128,
            blocks_per_group: 0,
            inodes_per_group: 0,
            group_count: 0,
            gdt_start: 0,
            gdt_blocks: 0,
            desc_size: 64,
            reserved_gdt_blocks: 0,
            log_groups_per_flex: 0,
            first_group_block: 0,
            hash_seed: [0; 8],
        }
    }
}

impl Superblock {
    /// Compute the on-disk layout from device size and block size (v2 format).
    pub fn new(dev_size: u64, block_size: u32) -> Result<Self> {
        let bs = block_size as u64;
        if block_size < 512 || !block_size.is_power_of_two() {
            bail!("block_size must be a power of 2 and >= 512, got {block_size}");
        }
        let total_blocks = dev_size / bs;
        if total_blocks < 4 {
            bail!("device too small: need at least 4 blocks, got {total_blocks}");
        }

        // Reserve last block for backup superblock
        let usable_blocks = total_blocks - 1; // last block = backup SB

        let inode_count = std::cmp::min(usable_blocks / 4, MAX_INODE_COUNT as u64) as u32;
        if inode_count == 0 {
            bail!("device too small for even 1 inode");
        }

        let inode_table_blocks = ceil_div(
            inode_count as u64 * INODE_SIZE as u64,
            bs,
        );
        let inode_table_start: u64 = 1; // block 0 is superblock

        // Inode bitmap: 1 bit per inode
        let inode_bitmap_blocks = ceil_div(ceil_div(inode_count as u64, 8), bs);
        let inode_bitmap_start = inode_table_start + inode_table_blocks;

        let bitmap_start = inode_bitmap_start + inode_bitmap_blocks;

        // Two-pass convergence for data bitmap vs data sizing
        let remaining = usable_blocks - bitmap_start;
        let bitmap_blocks_est = ceil_div(ceil_div(remaining, 8), bs);
        let mut data_start = bitmap_start + bitmap_blocks_est;
        let mut data_block_count = usable_blocks.saturating_sub(data_start);

        let bitmap_blocks = ceil_div(ceil_div(data_block_count, 8), bs);
        if bitmap_blocks < bitmap_blocks_est {
            data_start = bitmap_start + bitmap_blocks;
            data_block_count = usable_blocks.saturating_sub(data_start);
        }

        if data_block_count == 0 {
            bail!("device too small: no space for data blocks");
        }

        let backup_sb_block = total_blocks - 1;

        // Generate UUID v4
        let uuid = *uuid::Uuid::new_v4().as_bytes();

        Ok(Self {
            magic: CFS_MAGIC,
            version: CFS_VERSION,
            block_size,
            features_flags: FEATURE_HAS_INODE_BITMAP | FEATURE_HAS_BACKUP_SB,
            total_blocks,
            inode_count,
            root_inode: ROOT_INODE,
            inode_table_start,
            bitmap_start,
            data_start,
            free_blocks: data_block_count,
            inode_bitmap_start,
            free_inodes: inode_count - 1, // root inode is allocated
            uuid,
            volume_label: [0u8; 32],
            mount_count: 0,
            last_mount_time: 0,
            backup_sb_block,
            checksum: 0, // computed during serialize
            // v3 fields: defaults for v2 compatibility
            inode_size: INODE_SIZE,
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
        })
    }

    /// Number of data blocks (derived from layout).
    pub fn data_block_count(&self) -> u64 {
        if self.version >= CFS_VERSION_V3 && self.group_count > 0 {
            // v3 with block groups: compute from group structure
            // Last group may be smaller
            let full_groups = self.group_count.saturating_sub(1) as u64;
            let per_group_overhead = 2 + ceil_div(
                self.inodes_per_group as u64 * self.inode_size as u64,
                self.block_size as u64,
            );
            let data_per_full_group = self.blocks_per_group as u64 - per_group_overhead;
            let blocks_for_groups = self.total_blocks.saturating_sub(
                self.first_group_block as u64
                    + if self.has_backup_sb() { 1 } else { 0 },
            );
            let last_group_blocks = blocks_for_groups.saturating_sub(
                full_groups * self.blocks_per_group as u64,
            );
            let last_group_data = last_group_blocks.saturating_sub(per_group_overhead);
            full_groups * data_per_full_group + last_group_data
        } else if self.has_backup_sb() {
            self.backup_sb_block.saturating_sub(self.data_start)
        } else {
            self.total_blocks.saturating_sub(self.data_start)
        }
    }

    /// Compute the v3 on-disk layout from device size and format options.
    pub fn new_v3(dev_size: u64, opts: &FormatOptions) -> Result<Self> {
        opts.validate()?;

        let block_size = opts.block_size;
        let bs = block_size as u64;
        let total_blocks = dev_size / bs;
        if total_blocks < 64 {
            bail!("device too small: need at least 64 blocks, got {total_blocks}");
        }

        let blocks_per_group = opts.blocks_per_group as u64;
        let desc_size = 64u64;
        let descriptors_per_block = bs / desc_size;

        // Iterative layout convergence (3 passes)
        let mut group_count;
        let mut gdt_blocks;
        let mut reserved_gdt;
        let backup_sb = 1u64;

        // Journal blocks
        let journal_blocks = if opts.journal_percent == 0.0 {
            0u64
        } else {
            let j = (total_blocks as f64 * opts.journal_percent as f64 / 100.0) as u64;
            std::cmp::max(j, 16)
        };

        // Pass 1
        let group_count_est = ceil_div(total_blocks, blocks_per_group);
        gdt_blocks = ceil_div(group_count_est, descriptors_per_block);
        reserved_gdt = std::cmp::min(gdt_blocks, 128);
        let mut global_overhead = 1 + gdt_blocks + reserved_gdt + backup_sb + journal_blocks;
        let mut blocks_for_groups = total_blocks.saturating_sub(global_overhead);
        group_count = ceil_div(blocks_for_groups, blocks_per_group);

        // Pass 2
        gdt_blocks = ceil_div(group_count, descriptors_per_block);
        reserved_gdt = std::cmp::min(gdt_blocks, 128);
        global_overhead = 1 + gdt_blocks + reserved_gdt + backup_sb + journal_blocks;
        blocks_for_groups = total_blocks.saturating_sub(global_overhead);
        group_count = ceil_div(blocks_for_groups, blocks_per_group);

        // Pass 3
        gdt_blocks = ceil_div(group_count, descriptors_per_block);
        reserved_gdt = std::cmp::min(gdt_blocks, 128);
        global_overhead = 1 + gdt_blocks + reserved_gdt + backup_sb + journal_blocks;
        blocks_for_groups = total_blocks.saturating_sub(global_overhead);
        if blocks_for_groups == 0 {
            bail!("device too small for even 1 block group after overhead");
        }
        group_count = ceil_div(blocks_for_groups, blocks_per_group);

        // Per-group inodes
        let inodes_per_block = bs / opts.inode_size as u64;
        let total_data_bytes = blocks_for_groups * bs;
        let total_inodes_desired = total_data_bytes / opts.inode_ratio as u64;
        let inodes_per_group_raw = ceil_div(total_inodes_desired, group_count);
        let mut inodes_per_group = round_up_to(inodes_per_group_raw, inodes_per_block);
        if inodes_per_group == 0 {
            inodes_per_group = inodes_per_block;
        }

        let inode_table_blocks = ceil_div(inodes_per_group * opts.inode_size as u64, bs);
        let per_group_overhead = 2 + inode_table_blocks; // block_bm + inode_bm + inode_table
        if per_group_overhead >= blocks_per_group {
            bail!(
                "too many inodes for group size: per_group_overhead={} >= blocks_per_group={}",
                per_group_overhead, blocks_per_group
            );
        }

        // Block positions
        let gdt_start = 1u64;
        let journal_start = if journal_blocks > 0 {
            1 + gdt_blocks + reserved_gdt
        } else {
            0
        };
        let backup_sb_block = if journal_blocks > 0 {
            journal_start + journal_blocks
        } else {
            1 + gdt_blocks + reserved_gdt
        };
        let first_group_block = global_overhead;

        // Group 0 compat fields
        let group0_bitmap_start = first_group_block;
        let group0_inode_bitmap = first_group_block + 1;
        let group0_inode_table = first_group_block + 2;
        let group0_data_start = first_group_block + 2 + inode_table_blocks;

        let total_inodes = (inodes_per_group * group_count) as u32;

        // Feature flags
        let mut flags = FEATURE_HAS_INODE_BITMAP | FEATURE_HAS_BACKUP_SB | FEATURE_BLOCK_GROUPS | FEATURE_METADATA_HMAC;
        if opts.inode_size == 256 {
            flags |= FEATURE_256B_INODES | FEATURE_EXTENTS;
        }
        if journal_blocks > 0 {
            flags |= FEATURE_JOURNAL;
        }
        if opts.secure_delete {
            flags |= FEATURE_SECURE_DELETE;
        }
        flags |= FEATURE_HTREE;

        let uuid = *uuid::Uuid::new_v4().as_bytes();

        let mut hash_seed = [0u8; 8];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut hash_seed);

        let error_behavior_val = match opts.error_behavior {
            ErrorBehavior::Continue => 0u32,
            ErrorBehavior::ReadOnly => 1u32,
        };

        // Volume label
        let mut volume_label = [0u8; 32];
        let label_bytes = opts.volume_label.as_bytes();
        let label_len = std::cmp::min(label_bytes.len(), 31);
        volume_label[..label_len].copy_from_slice(&label_bytes[..label_len]);

        // Compute total free data blocks (all groups, accounting for last group being smaller)
        let data_blocks_per_full_group = blocks_per_group - per_group_overhead;
        let total_data_blocks = if group_count == 1 {
            // Only one group, which may be smaller than blocks_per_group
            blocks_for_groups.saturating_sub(per_group_overhead)
        } else {
            let full_groups = group_count - 1;
            let last_group_blocks = blocks_for_groups - full_groups * blocks_per_group;
            let last_group_data = last_group_blocks.saturating_sub(per_group_overhead);
            full_groups * data_blocks_per_full_group + last_group_data
        };

        Ok(Self {
            magic: CFS_MAGIC,
            version: CFS_VERSION_V3,
            block_size,
            features_flags: flags,
            total_blocks,
            inode_count: total_inodes,
            root_inode: ROOT_INODE,
            inode_table_start: group0_inode_table,
            bitmap_start: group0_bitmap_start,
            data_start: group0_data_start,
            free_blocks: total_data_blocks,
            inode_bitmap_start: group0_inode_bitmap,
            free_inodes: total_inodes.saturating_sub(1),
            uuid,
            volume_label,
            mount_count: 0,
            last_mount_time: 0,
            backup_sb_block,
            checksum: 0,
            // v3 fields
            inode_size: opts.inode_size,
            journal_start,
            journal_blocks,
            first_orphan_inode: 0,
            error_behavior: error_behavior_val,
            default_permissions: opts.default_permissions,
            inode_ratio: opts.inode_ratio,
            journal_sequence: 0,
            metadata_hmac: [0u8; 8],
            dir_entry_size: 128,
            // Block group fields
            blocks_per_group: opts.blocks_per_group,
            inodes_per_group: inodes_per_group as u32,
            group_count: group_count as u32,
            gdt_start: gdt_start,
            gdt_blocks: gdt_blocks as u32,
            desc_size: desc_size as u32,
            reserved_gdt_blocks: reserved_gdt as u32,
            log_groups_per_flex: 0,
            first_group_block: first_group_block as u32,
            hash_seed,
        })
    }

    /// Whether the inode bitmap feature is enabled.
    pub fn has_inode_bitmap(&self) -> bool {
        self.features_flags & FEATURE_HAS_INODE_BITMAP != 0
    }

    /// Whether the backup superblock feature is enabled.
    pub fn has_backup_sb(&self) -> bool {
        self.features_flags & FEATURE_HAS_BACKUP_SB != 0
    }

    /// Whether the journal feature is enabled.
    pub fn has_journal(&self) -> bool {
        self.features_flags & FEATURE_JOURNAL != 0 && self.journal_blocks > 0
    }

    /// Whether the secure-delete feature is enabled.
    pub fn has_secure_delete(&self) -> bool {
        self.features_flags & FEATURE_SECURE_DELETE != 0
    }

    /// Serialize to a `block_size`-byte buffer (little-endian).
    /// v2: CRC32 at offset 144. v3: CRC32 at offset 252.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = vec![0u8; self.block_size as usize];

        // Common fields (0..144) — same for v2 and v3
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..8].copy_from_slice(&self.version.to_le_bytes());
        buf[8..12].copy_from_slice(&self.block_size.to_le_bytes());
        buf[12..16].copy_from_slice(&self.features_flags.to_le_bytes());
        buf[16..24].copy_from_slice(&self.total_blocks.to_le_bytes());
        buf[24..28].copy_from_slice(&self.inode_count.to_le_bytes());
        buf[28..32].copy_from_slice(&self.root_inode.to_le_bytes());
        buf[32..40].copy_from_slice(&self.inode_table_start.to_le_bytes());
        buf[40..48].copy_from_slice(&self.bitmap_start.to_le_bytes());
        buf[48..56].copy_from_slice(&self.data_start.to_le_bytes());
        buf[56..64].copy_from_slice(&self.free_blocks.to_le_bytes());
        buf[64..72].copy_from_slice(&self.inode_bitmap_start.to_le_bytes());
        buf[72..76].copy_from_slice(&self.free_inodes.to_le_bytes());
        buf[76..92].copy_from_slice(&self.uuid);
        buf[92..124].copy_from_slice(&self.volume_label);
        buf[124..128].copy_from_slice(&self.mount_count.to_le_bytes());
        buf[128..136].copy_from_slice(&self.last_mount_time.to_le_bytes());
        buf[136..144].copy_from_slice(&self.backup_sb_block.to_le_bytes());

        if self.version >= CFS_VERSION_V3 {
            // v3 fields: 144..252
            buf[144..148].copy_from_slice(&self.inode_size.to_le_bytes());
            buf[148..156].copy_from_slice(&self.journal_start.to_le_bytes());
            buf[156..164].copy_from_slice(&self.journal_blocks.to_le_bytes());
            buf[164..168].copy_from_slice(&self.first_orphan_inode.to_le_bytes());
            buf[168..172].copy_from_slice(&self.error_behavior.to_le_bytes());
            buf[172..176].copy_from_slice(&self.default_permissions.to_le_bytes());
            buf[176..180].copy_from_slice(&self.inode_ratio.to_le_bytes());
            buf[180..188].copy_from_slice(&self.journal_sequence.to_le_bytes());
            buf[188..196].copy_from_slice(&self.metadata_hmac);
            buf[196..200].copy_from_slice(&self.dir_entry_size.to_le_bytes());

            // Block group fields: 200..248
            buf[200..204].copy_from_slice(&self.blocks_per_group.to_le_bytes());
            buf[204..208].copy_from_slice(&self.inodes_per_group.to_le_bytes());
            buf[208..212].copy_from_slice(&self.group_count.to_le_bytes());
            buf[212..220].copy_from_slice(&self.gdt_start.to_le_bytes());
            buf[220..224].copy_from_slice(&self.gdt_blocks.to_le_bytes());
            buf[224..228].copy_from_slice(&self.desc_size.to_le_bytes());
            buf[228..232].copy_from_slice(&self.reserved_gdt_blocks.to_le_bytes());
            buf[232..236].copy_from_slice(&self.log_groups_per_flex.to_le_bytes());
            buf[236..240].copy_from_slice(&self.first_group_block.to_le_bytes());
            buf[240..248].copy_from_slice(&self.hash_seed);
            // 248..252: reserved (zeros)

            // CRC32 at offset 252
            let mut hasher = Hasher::new();
            hasher.update(&buf[..252]);
            buf[252..256].copy_from_slice(&hasher.finalize().to_le_bytes());
        } else {
            // v2: CRC32 at offset 144
            let mut hasher = Hasher::new();
            hasher.update(&buf[..144]);
            let crc = hasher.finalize();
            buf[144..148].copy_from_slice(&crc.to_le_bytes());
        }

        buf
    }

    /// Deserialize from a buffer. Supports both v1 (68-byte) and v2 (148-byte) layouts.
    pub fn deserialize(buf: &[u8]) -> Result<Self> {
        if buf.len() < 68 {
            bail!("superblock buffer too small: {} bytes (need ≥68)", buf.len());
        }

        let magic: [u8; 4] = buf[0..4].try_into().unwrap();
        if magic != CFS_MAGIC {
            bail!(
                "bad magic: expected {:?}, got {:?}",
                CFS_MAGIC,
                magic
            );
        }

        let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());

        match version {
            CFS_VERSION_V1 => Self::deserialize_v1(buf, magic),
            CFS_VERSION => Self::deserialize_v2(buf, magic),
            CFS_VERSION_V3 => Self::deserialize_v3(buf, magic),
            _ => bail!("unsupported CFS version: {version}"),
        }
    }

    /// Deserialize v1 layout (68 bytes, CRC32 at offset 64).
    fn deserialize_v1(buf: &[u8], magic: [u8; 4]) -> Result<Self> {
        let block_size = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let total_blocks = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let inode_count = u32::from_le_bytes(buf[24..28].try_into().unwrap());
        let root_inode = u32::from_le_bytes(buf[28..32].try_into().unwrap());
        let inode_table_start = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        let bitmap_start = u64::from_le_bytes(buf[40..48].try_into().unwrap());
        let data_start = u64::from_le_bytes(buf[48..56].try_into().unwrap());
        let free_blocks = u64::from_le_bytes(buf[56..64].try_into().unwrap());
        let stored_crc = u32::from_le_bytes(buf[64..68].try_into().unwrap());

        // Verify v1 checksum (CRC32 of [0..64))
        let mut hasher = Hasher::new();
        hasher.update(&buf[..64]);
        let computed_crc = hasher.finalize();
        if stored_crc != computed_crc {
            bail!(
                "superblock checksum mismatch: stored=0x{stored_crc:08x}, computed=0x{computed_crc:08x}"
            );
        }

        // v1 volumes don't have inode bitmap, backup SB, UUID, etc.
        Ok(Self {
            magic,
            version: CFS_VERSION_V1,
            block_size,
            features_flags: 0,
            total_blocks,
            inode_count,
            root_inode,
            inode_table_start,
            bitmap_start,
            data_start,
            free_blocks,
            inode_bitmap_start: 0,
            free_inodes: 0, // unknown for v1
            uuid: [0u8; 16],
            volume_label: [0u8; 32],
            mount_count: 0,
            last_mount_time: 0,
            backup_sb_block: 0,
            checksum: stored_crc,
            // v3 fields: zeroed/defaulted
            inode_size: INODE_SIZE,
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
        })
    }

    /// Deserialize v2 layout (148 bytes, CRC32 at offset 144).
    fn deserialize_v2(buf: &[u8], magic: [u8; 4]) -> Result<Self> {
        if buf.len() < 148 {
            bail!("superblock buffer too small for v2: {} bytes (need ≥148)", buf.len());
        }

        let block_size = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let features_flags = u32::from_le_bytes(buf[12..16].try_into().unwrap());

        // Reject unknown features
        if features_flags & !KNOWN_FEATURES_V2 != 0 {
            bail!(
                "superblock has unknown feature flags: 0x{:08x}",
                features_flags & !KNOWN_FEATURES_V2
            );
        }

        let total_blocks = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let inode_count = u32::from_le_bytes(buf[24..28].try_into().unwrap());
        let root_inode = u32::from_le_bytes(buf[28..32].try_into().unwrap());
        let inode_table_start = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        let bitmap_start = u64::from_le_bytes(buf[40..48].try_into().unwrap());
        let data_start = u64::from_le_bytes(buf[48..56].try_into().unwrap());
        let free_blocks = u64::from_le_bytes(buf[56..64].try_into().unwrap());
        let inode_bitmap_start = u64::from_le_bytes(buf[64..72].try_into().unwrap());
        let free_inodes = u32::from_le_bytes(buf[72..76].try_into().unwrap());

        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&buf[76..92]);

        let mut volume_label = [0u8; 32];
        volume_label.copy_from_slice(&buf[92..124]);

        let mount_count = u32::from_le_bytes(buf[124..128].try_into().unwrap());
        let last_mount_time = u64::from_le_bytes(buf[128..136].try_into().unwrap());
        let backup_sb_block = u64::from_le_bytes(buf[136..144].try_into().unwrap());
        let stored_crc = u32::from_le_bytes(buf[144..148].try_into().unwrap());

        // Verify v2 checksum (CRC32 of [0..144))
        let mut hasher = Hasher::new();
        hasher.update(&buf[..144]);
        let computed_crc = hasher.finalize();
        if stored_crc != computed_crc {
            bail!(
                "superblock checksum mismatch: stored=0x{stored_crc:08x}, computed=0x{computed_crc:08x}"
            );
        }

        Ok(Self {
            magic,
            version: CFS_VERSION,
            block_size,
            features_flags,
            total_blocks,
            inode_count,
            root_inode,
            inode_table_start,
            bitmap_start,
            data_start,
            free_blocks,
            inode_bitmap_start,
            free_inodes,
            uuid,
            volume_label,
            mount_count,
            last_mount_time,
            backup_sb_block,
            checksum: stored_crc,
            // v3 fields: v2-compatible defaults
            inode_size: INODE_SIZE,
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
        })
    }

    /// Deserialize v3 layout (256 bytes, CRC32 at offset 252).
    fn deserialize_v3(buf: &[u8], magic: [u8; 4]) -> Result<Self> {
        if buf.len() < 256 {
            bail!("superblock buffer too small for v3: {} bytes (need ≥256)", buf.len());
        }

        // Verify CRC32 at offset 252 over bytes [0..252)
        let stored_crc = u32::from_le_bytes(buf[252..256].try_into().unwrap());
        let mut hasher = Hasher::new();
        hasher.update(&buf[..252]);
        let computed = hasher.finalize();
        if stored_crc != computed {
            bail!("superblock v3 checksum mismatch: stored=0x{stored_crc:08x}, computed=0x{computed:08x}");
        }

        let block_size = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let features_flags = u32::from_le_bytes(buf[12..16].try_into().unwrap());

        // Check feature flags
        if features_flags & !KNOWN_FEATURES_V3 != 0 {
            bail!("unknown feature flags: 0x{:08x}", features_flags & !KNOWN_FEATURES_V3);
        }

        let total_blocks = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let inode_count = u32::from_le_bytes(buf[24..28].try_into().unwrap());
        let root_inode = u32::from_le_bytes(buf[28..32].try_into().unwrap());
        let inode_table_start = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        let bitmap_start = u64::from_le_bytes(buf[40..48].try_into().unwrap());
        let data_start = u64::from_le_bytes(buf[48..56].try_into().unwrap());
        let free_blocks = u64::from_le_bytes(buf[56..64].try_into().unwrap());
        let inode_bitmap_start = u64::from_le_bytes(buf[64..72].try_into().unwrap());
        let free_inodes = u32::from_le_bytes(buf[72..76].try_into().unwrap());

        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&buf[76..92]);
        let mut volume_label = [0u8; 32];
        volume_label.copy_from_slice(&buf[92..124]);

        let mount_count = u32::from_le_bytes(buf[124..128].try_into().unwrap());
        let last_mount_time = u64::from_le_bytes(buf[128..136].try_into().unwrap());
        let backup_sb_block = u64::from_le_bytes(buf[136..144].try_into().unwrap());

        // v3 fields: 144..252
        let inode_size = u32::from_le_bytes(buf[144..148].try_into().unwrap());
        let journal_start = u64::from_le_bytes(buf[148..156].try_into().unwrap());
        let journal_blocks = u64::from_le_bytes(buf[156..164].try_into().unwrap());
        let first_orphan_inode = u32::from_le_bytes(buf[164..168].try_into().unwrap());
        let error_behavior = u32::from_le_bytes(buf[168..172].try_into().unwrap());
        let default_permissions = u32::from_le_bytes(buf[172..176].try_into().unwrap());
        let inode_ratio = u32::from_le_bytes(buf[176..180].try_into().unwrap());
        let journal_sequence = u64::from_le_bytes(buf[180..188].try_into().unwrap());
        let mut metadata_hmac = [0u8; 8];
        metadata_hmac.copy_from_slice(&buf[188..196]);
        let dir_entry_size = u32::from_le_bytes(buf[196..200].try_into().unwrap());

        // Block group fields: 200..248
        let blocks_per_group = u32::from_le_bytes(buf[200..204].try_into().unwrap());
        let inodes_per_group = u32::from_le_bytes(buf[204..208].try_into().unwrap());
        let group_count = u32::from_le_bytes(buf[208..212].try_into().unwrap());
        let gdt_start = u64::from_le_bytes(buf[212..220].try_into().unwrap());
        let gdt_blocks = u32::from_le_bytes(buf[220..224].try_into().unwrap());
        let desc_size = u32::from_le_bytes(buf[224..228].try_into().unwrap());
        let reserved_gdt_blocks = u32::from_le_bytes(buf[228..232].try_into().unwrap());
        let log_groups_per_flex = u32::from_le_bytes(buf[232..236].try_into().unwrap());
        let first_group_block = u32::from_le_bytes(buf[236..240].try_into().unwrap());
        let mut hash_seed = [0u8; 8];
        hash_seed.copy_from_slice(&buf[240..248]);

        Ok(Self {
            magic,
            version: CFS_VERSION_V3,
            block_size,
            features_flags,
            total_blocks,
            inode_count,
            root_inode,
            inode_table_start,
            bitmap_start,
            data_start,
            free_blocks,
            inode_bitmap_start,
            free_inodes,
            uuid,
            volume_label,
            mount_count,
            last_mount_time,
            backup_sb_block,
            checksum: stored_crc,
            inode_size,
            journal_start,
            journal_blocks,
            first_orphan_inode,
            error_behavior,
            default_permissions,
            inode_ratio,
            journal_sequence,
            metadata_hmac,
            dir_entry_size,
            blocks_per_group,
            inodes_per_group,
            group_count,
            gdt_start,
            gdt_blocks,
            desc_size,
            reserved_gdt_blocks,
            log_groups_per_flex,
            first_group_block,
            hash_seed,
        })
    }

    /// Write the superblock to block 0 of the device.
    /// If backup superblock is enabled, also writes to the backup block
    /// and to the last block (for recovery when primary is corrupted).
    pub fn write_to(&self, dev: &mut dyn CFSBlockDevice) -> Result<()> {
        let buf = self.serialize();
        dev.write(0, &buf)?;

        // Write backup superblock if enabled
        if self.has_backup_sb() && self.backup_sb_block > 0 {
            let offset = self.backup_sb_block * self.block_size as u64;
            dev.write(offset, &buf)?;

            // Also write to the last block for read_from() fallback
            let last_block = dev.size() / self.block_size as u64 - 1;
            if last_block != self.backup_sb_block {
                let last_offset = last_block * self.block_size as u64;
                dev.write(last_offset, &buf)?;
            }
        }

        Ok(())
    }

    /// Read and validate the superblock from block 0.
    /// Falls back to backup superblock (last block) if primary fails CRC check.
    pub fn read_from(dev: &mut dyn CFSBlockDevice, block_size: u32) -> Result<Self> {
        let mut buf = vec![0u8; block_size as usize];
        dev.read(0, &mut buf)?;

        match Self::deserialize(&buf) {
            Ok(sb) => Ok(sb),
            Err(primary_err) => {
                // Try backup superblock at the last block
                let dev_size = dev.size();
                let total_blocks = dev_size / block_size as u64;
                if total_blocks < 2 {
                    return Err(primary_err);
                }
                let backup_offset = (total_blocks - 1) * block_size as u64;
                let mut backup_buf = vec![0u8; block_size as usize];
                if dev.read(backup_offset, &mut backup_buf).is_err() {
                    return Err(primary_err);
                }
                match Self::deserialize(&backup_buf) {
                    Ok(sb) => {
                        // Backup is valid — attempt to restore primary
                        if let Err(e) = dev.write(0, &backup_buf) {
                            eprintln!("WARNING: failed to restore primary superblock from backup: {}", e);
                        }
                        Ok(sb)
                    }
                    Err(_) => Err(primary_err),
                }
            }
        }
    }

    /// Get the volume label as a UTF-8 string (trimmed of null bytes).
    pub fn label(&self) -> &str {
        let end = self.volume_label.iter()
            .position(|&b| b == 0)
            .unwrap_or(self.volume_label.len());
        std::str::from_utf8(&self.volume_label[..end]).unwrap_or("")
    }

    /// Set the volume label (max 31 bytes UTF-8).
    pub fn set_label(&mut self, label: &str) {
        self.volume_label = [0u8; 32];
        let bytes = label.as_bytes();
        let len = std::cmp::min(bytes.len(), 31); // leave room for null terminator
        self.volume_label[..len].copy_from_slice(&bytes[..len]);
    }

    /// Get the UUID as a formatted string.
    pub fn uuid_str(&self) -> String {
        uuid::Uuid::from_bytes(self.uuid).to_string()
    }
}

// ---------------------------------------------------------------------------
// Metadata HMAC (10D.6)
// ---------------------------------------------------------------------------

/// Derive the HMAC key for metadata integrity verification.
///
/// - Encrypted volumes: HMAC-SHA256(master_key, "CFS-METADATA-HMAC")
/// - Plaintext volumes: SHA-256("CFS-PLAINTEXT-HMAC-KEY-V1") — only detects
///   accidental corruption, not adversarial tampering.
pub fn derive_hmac_key(master_key: Option<&[u8; 32]>) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    match master_key {
        Some(key) => {
            type HmacSha256 = Hmac<Sha256>;
            let mut mac = HmacSha256::new_from_slice(key)
                .expect("HMAC key length is valid");
            mac.update(b"CFS-METADATA-HMAC");
            let result = mac.finalize();
            let mut out = [0u8; 32];
            out.copy_from_slice(&result.into_bytes());
            out
        }
        None => {
            use sha2::Digest;
            let mut hasher = Sha256::new();
            hasher.update(b"CFS-PLAINTEXT-HMAC-KEY-V1");
            let result = hasher.finalize();
            let mut out = [0u8; 32];
            out.copy_from_slice(&result);
            out
        }
    }
}

/// Compute HMAC-SHA256 over critical metadata, truncated to 8 bytes.
///
/// Input = superblock_bytes (with hmac+crc zeroed) ‖ GDT bytes (if any).
pub fn compute_metadata_hmac(
    dev: &mut dyn CFSBlockDevice,
    sb: &Superblock,
    hmac_key: &[u8; 32],
) -> Result<[u8; 8]> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut data = Vec::new();

    // 1. Superblock bytes with metadata_hmac and CRC32 fields zeroed
    let sb_buf = sb.serialize(); // block_size-length buffer
    // Only HMAC the first 256 bytes (v3 header), with hmac+crc zeroed
    let mut hdr = [0u8; 256];
    let copy_len = sb_buf.len().min(256);
    hdr[..copy_len].copy_from_slice(&sb_buf[..copy_len]);
    // Zero metadata_hmac at [188..196)
    hdr[188..196].fill(0);
    // Zero CRC32 at [252..256)
    hdr[252..256].fill(0);
    data.extend_from_slice(&hdr);

    // 2. GDT bytes (if block groups are in use)
    if sb.group_count > 0 && sb.gdt_start > 0 {
        let gdt_total_bytes = sb.group_count as usize * sb.desc_size as usize;
        // Read whole blocks to satisfy sector alignment
        let gdt_disk_blocks = sb.gdt_blocks.max(1) as usize;
        let read_len = gdt_disk_blocks * sb.block_size as usize;
        if gdt_total_bytes > read_len {
            bail!("corrupted superblock: GDT size ({}) exceeds readable region ({})", gdt_total_bytes, read_len);
        }
        let mut gdt_buf = vec![0u8; read_len];
        let gdt_disk_offset = sb.gdt_start * sb.block_size as u64;
        dev.read(gdt_disk_offset, &mut gdt_buf)?;
        // Only include the meaningful GDT bytes in the HMAC
        data.extend_from_slice(&gdt_buf[..gdt_total_bytes]);
    }

    // Compute HMAC-SHA256 and truncate to 8 bytes
    let mut mac = HmacSha256::new_from_slice(hmac_key)
        .expect("HMAC key length is valid");
    mac.update(&data);
    let result = mac.finalize().into_bytes();

    let mut hmac_bytes = [0u8; 8];
    hmac_bytes.copy_from_slice(&result[..8]);
    Ok(hmac_bytes)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use tempfile::NamedTempFile;

    #[test]
    fn test_superblock_serialize_roundtrip() {
        let sb = Superblock::new(1_048_576, 4096).unwrap();
        let buf = sb.serialize();
        let sb2 = Superblock::deserialize(&buf).unwrap();

        assert_eq!(sb.magic, sb2.magic);
        assert_eq!(sb.version, sb2.version);
        assert_eq!(sb.block_size, sb2.block_size);
        assert_eq!(sb.features_flags, sb2.features_flags);
        assert_eq!(sb.total_blocks, sb2.total_blocks);
        assert_eq!(sb.inode_count, sb2.inode_count);
        assert_eq!(sb.root_inode, sb2.root_inode);
        assert_eq!(sb.inode_table_start, sb2.inode_table_start);
        assert_eq!(sb.bitmap_start, sb2.bitmap_start);
        assert_eq!(sb.data_start, sb2.data_start);
        assert_eq!(sb.free_blocks, sb2.free_blocks);
        assert_eq!(sb.inode_bitmap_start, sb2.inode_bitmap_start);
        assert_eq!(sb.free_inodes, sb2.free_inodes);
        assert_eq!(sb.uuid, sb2.uuid);
        assert_eq!(sb.volume_label, sb2.volume_label);
        assert_eq!(sb.mount_count, sb2.mount_count);
        assert_eq!(sb.last_mount_time, sb2.last_mount_time);
        assert_eq!(sb.backup_sb_block, sb2.backup_sb_block);
    }

    #[test]
    fn test_superblock_device_roundtrip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        let sb = Superblock::new(1_048_576, 4096).unwrap();
        sb.write_to(&mut dev).unwrap();

        let sb2 = Superblock::read_from(&mut dev, 4096).unwrap();
        assert_eq!(sb.total_blocks, sb2.total_blocks);
        assert_eq!(sb.free_blocks, sb2.free_blocks);
        assert_eq!(sb.data_start, sb2.data_start);
        assert_eq!(sb.uuid, sb2.uuid);
    }

    #[test]
    fn test_superblock_bad_magic() {
        let sb = Superblock::new(1_048_576, 4096).unwrap();
        let mut buf = sb.serialize();
        buf[0] = b'X'; // corrupt magic
        assert!(Superblock::deserialize(&buf).is_err());
    }

    #[test]
    fn test_superblock_bad_checksum() {
        let sb = Superblock::new(1_048_576, 4096).unwrap();
        let mut buf = sb.serialize();
        // Corrupt a field without recomputing CRC
        buf[16] ^= 0xFF;
        assert!(Superblock::deserialize(&buf).is_err());
    }

    #[test]
    fn test_layout_sanity() {
        let sb = Superblock::new(1_048_576, 4096).unwrap();
        assert!(sb.inode_table_start < sb.inode_bitmap_start);
        assert!(sb.inode_bitmap_start < sb.bitmap_start);
        assert!(sb.bitmap_start < sb.data_start);
        assert!(sb.data_start < sb.total_blocks);
        assert_eq!(sb.free_blocks, sb.data_block_count());
        assert!(sb.has_inode_bitmap());
        assert!(sb.has_backup_sb());
        assert_eq!(sb.backup_sb_block, sb.total_blocks - 1);
    }

    #[test]
    fn test_layout_1mb() {
        let sb = Superblock::new(1_048_576, 4096).unwrap();
        assert_eq!(sb.total_blocks, 256);
        assert_eq!(sb.inode_count, 63); // (256-1)/4 = 63 (backup block reserved)
        assert_eq!(sb.version, CFS_VERSION);
        assert!(sb.inode_bitmap_start > 0);
        assert!(sb.backup_sb_block == 255);
    }

    #[test]
    fn test_layout_minimum() {
        // 64 KB = 16 blocks; usable = 15 (minus backup)
        let sb = Superblock::new(65_536, 4096).unwrap();
        assert_eq!(sb.total_blocks, 16);
        assert_eq!(sb.backup_sb_block, 15);
        assert!(sb.data_start < 15);
    }

    #[test]
    fn test_uuid_generated() {
        let sb = Superblock::new(1_048_576, 4096).unwrap();
        // UUID should not be all zeros
        assert_ne!(sb.uuid, [0u8; 16]);
        // UUID string should be valid
        let uuid_str = sb.uuid_str();
        assert_eq!(uuid_str.len(), 36); // "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
    }

    #[test]
    fn test_volume_label() {
        let mut sb = Superblock::new(1_048_576, 4096).unwrap();
        assert_eq!(sb.label(), "");

        sb.set_label("My Volume");
        assert_eq!(sb.label(), "My Volume");

        let buf = sb.serialize();
        let sb2 = Superblock::deserialize(&buf).unwrap();
        assert_eq!(sb2.label(), "My Volume");
    }

    #[test]
    fn test_backup_superblock_recovery() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        let sb = Superblock::new(1_048_576, 4096).unwrap();
        sb.write_to(&mut dev).unwrap(); // writes to block 0 and backup
        dev.flush().unwrap();

        // Corrupt primary superblock (block 0)
        let garbage = vec![0xFFu8; 4096];
        dev.write(0, &garbage).unwrap();

        // read_from should fall back to backup
        let recovered = Superblock::read_from(&mut dev, 4096).unwrap();
        assert_eq!(recovered.total_blocks, sb.total_blocks);
        assert_eq!(recovered.uuid, sb.uuid);
    }

    #[test]
    fn test_feature_flags_reject_unknown() {
        let mut sb = Superblock::new(1_048_576, 4096).unwrap();
        sb.features_flags |= 1 << 31; // unknown flag
        let buf = sb.serialize();
        assert!(Superblock::deserialize(&buf).is_err());
    }

    // -----------------------------------------------------------------------
    // 10A.2 — Superblock v3 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_superblock_v3_serialize_roundtrip() {
        let opts = FormatOptions::default();
        let sb = Superblock::new_v3(4 * 1024 * 1024 * 1024, &opts).unwrap(); // 4 GB
        assert_eq!(sb.version, CFS_VERSION_V3);

        let buf = sb.serialize();
        let sb2 = Superblock::deserialize(&buf).unwrap();

        assert_eq!(sb.version, sb2.version);
        assert_eq!(sb.block_size, sb2.block_size);
        assert_eq!(sb.total_blocks, sb2.total_blocks);
        assert_eq!(sb.inode_count, sb2.inode_count);
        assert_eq!(sb.inode_size, sb2.inode_size);
        assert_eq!(sb.group_count, sb2.group_count);
        assert_eq!(sb.blocks_per_group, sb2.blocks_per_group);
        assert_eq!(sb.inodes_per_group, sb2.inodes_per_group);
        assert_eq!(sb.journal_start, sb2.journal_start);
        assert_eq!(sb.journal_blocks, sb2.journal_blocks);
        assert_eq!(sb.gdt_blocks, sb2.gdt_blocks);
        assert_eq!(sb.features_flags, sb2.features_flags);
        assert_eq!(sb.uuid, sb2.uuid);
        assert_eq!(sb.hash_seed, sb2.hash_seed);
        assert_eq!(sb.error_behavior, sb2.error_behavior);
        assert_eq!(sb.default_permissions, sb2.default_permissions);
    }

    #[test]
    fn test_superblock_v3_checksum() {
        let opts = FormatOptions::default();
        let sb = Superblock::new_v3(4 * 1024 * 1024 * 1024, &opts).unwrap();
        let mut buf = sb.serialize();
        buf[100] ^= 0xFF; // corrupt data
        assert!(Superblock::deserialize(&buf).is_err());
    }

    #[test]
    fn test_superblock_v3_unknown_features() {
        let opts = FormatOptions::default();
        let mut sb = Superblock::new_v3(4 * 1024 * 1024 * 1024, &opts).unwrap();
        sb.features_flags |= 1 << 20; // unknown bit
        let buf = sb.serialize();
        assert!(Superblock::deserialize(&buf).is_err());
    }

    #[test]
    fn test_superblock_v3_layout_4gb() {
        let opts = FormatOptions::default();
        let sb = Superblock::new_v3(4u64 * 1024 * 1024 * 1024, &opts).unwrap();
        assert!(sb.group_count > 0);
        assert!(sb.blocks_per_group == 32768);
        assert!(sb.inode_count > 0);
        assert!(sb.features_flags & FEATURE_BLOCK_GROUPS != 0);
        assert!(sb.features_flags & FEATURE_256B_INODES != 0);
    }

    #[test]
    fn test_superblock_v3_layout_small() {
        // 512 KB = enough for a small v3 volume
        let mut opts = FormatOptions::default();
        opts.journal_percent = 0.0;
        opts.blocks_per_group = 4096 * 8;
        let sb = Superblock::new_v3(512 * 1024, &opts).unwrap();
        assert!(sb.group_count >= 1);
        assert!(sb.total_blocks > 0);
    }

    #[test]
    fn test_superblock_v3_layout_too_small() {
        let opts = FormatOptions::default();
        // 32 KB = 8 blocks at 4096, far too small for v3
        assert!(Superblock::new_v3(32 * 1024, &opts).is_err());
    }

    #[test]
    fn test_superblock_v3_journal_disabled() {
        let mut opts = FormatOptions::default();
        opts.journal_percent = 0.0;
        let sb = Superblock::new_v3(10 * 1024 * 1024, &opts).unwrap();
        assert_eq!(sb.journal_start, 0);
        assert_eq!(sb.journal_blocks, 0);
        assert_eq!(sb.features_flags & FEATURE_JOURNAL, 0);
    }

    #[test]
    fn test_superblock_v3_journal_sized() {
        let mut opts = FormatOptions::default();
        opts.journal_percent = 2.0;
        let sb = Superblock::new_v3(1024 * 1024 * 1024, &opts).unwrap(); // 1 GB
        assert!(sb.journal_blocks > 0);
        assert!(sb.journal_start > 0);
        assert!(sb.features_flags & FEATURE_JOURNAL != 0);
    }

    #[test]
    fn test_superblock_v2_backward_compat() {
        // Read a v2 superblock — v3 fields should have safe defaults
        let sb = Superblock::new(1_048_576, 4096).unwrap();
        assert_eq!(sb.version, CFS_VERSION);
        let buf = sb.serialize();
        let sb2 = Superblock::deserialize(&buf).unwrap();
        assert_eq!(sb2.inode_size, INODE_SIZE); // 128
        assert_eq!(sb2.group_count, 0);
        assert_eq!(sb2.blocks_per_group, 0);
        assert_eq!(sb2.journal_blocks, 0);
    }

    #[test]
    fn test_superblock_v3_write_read_device() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap(); // 4 MB

        let mut opts = FormatOptions::default();
        opts.journal_percent = 0.0;
        let sb = Superblock::new_v3(dev.size(), &opts).unwrap();
        sb.write_to(&mut dev).unwrap();
        dev.flush().unwrap();

        let sb2 = Superblock::read_from(&mut dev, 4096).unwrap();
        assert_eq!(sb.version, sb2.version);
        assert_eq!(sb.total_blocks, sb2.total_blocks);
        assert_eq!(sb.group_count, sb2.group_count);
        assert_eq!(sb.inode_size, sb2.inode_size);
    }

    #[test]
    fn test_superblock_v3_backup_fallback() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(4 * 1024 * 1024)).unwrap();

        let mut opts = FormatOptions::default();
        opts.journal_percent = 0.0;
        let sb = Superblock::new_v3(dev.size(), &opts).unwrap();
        sb.write_to(&mut dev).unwrap();
        dev.flush().unwrap();

        // Corrupt primary
        let garbage = vec![0xFFu8; 4096];
        dev.write(0, &garbage).unwrap();

        let recovered = Superblock::read_from(&mut dev, 4096).unwrap();
        assert_eq!(recovered.total_blocks, sb.total_blocks);
        assert_eq!(recovered.version, CFS_VERSION_V3);
    }
}
