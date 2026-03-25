use anyhow::{bail, Result};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::block_device::CFSBlockDevice;
use super::{INODE_SIZE, INODE_FILE, INODE_DIR, INODE_SYMLINK};

// ---------------------------------------------------------------------------
// Inode constants
// ---------------------------------------------------------------------------

pub const INODE_SIZE_V2: u32 = 128;
pub const INODE_SIZE_V3: u32 = 256;

/// Inode flags (bit field stored in `flags: u32`).
pub const INODE_FLAG_INLINE_DATA: u32    = 1 << 0;
pub const INODE_FLAG_HAS_XATTR: u32     = 1 << 1;
pub const INODE_FLAG_IMMUTABLE: u32     = 1 << 2;
pub const INODE_FLAG_APPEND_ONLY: u32   = 1 << 3;
pub const INODE_FLAG_SECURE_DELETE: u32 = 1 << 4;
pub const INODE_FLAG_PER_FILE_KEY: u32  = 1 << 5;
pub const INODE_FLAG_EXTENTS: u32       = 1 << 6;
pub const INODE_FLAG_HTREE: u32         = 1 << 7;
pub const INODE_FLAG_ORPHAN: u32        = 1 << 8;

// ---------------------------------------------------------------------------
// Inode struct — supports both v2 (128B) and v3 (256B) on-disk layouts
// ---------------------------------------------------------------------------

/// On-disk inode. v2 uses 128 bytes, v3 uses 256 bytes.
///
/// v2 layout (128 bytes):
/// ```text
/// Offset  Size  Field
/// 0       2     mode            0=unused, 1=file, 2=directory
/// 2       2     nlinks
/// 4       4     block_count     allocated data blocks
/// 8       8     size            file size in bytes
/// 16      8     created         Unix timestamp (seconds)
/// 24      8     modified        Unix timestamp (seconds)
/// 32      80    direct_blocks   10 × u64 block pointers
/// 112     8     indirect_block  single-indirect pointer
/// 120     8     double_indirect double-indirect pointer
/// ```
///
/// v3 layout (256 bytes): extends v2 with timestamps, permissions,
/// extent tree root, inline area, and checksum. See `serialize_v3()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inode {
    // --- Common fields (v2 + v3) ---
    pub mode: u16,
    pub nlinks: u16,
    pub block_count: u32,      // lower 32 bits
    pub size: u64,
    pub created: u64,          // v2: seconds. v3: nanoseconds since epoch
    pub modified: u64,         // v2: seconds. v3: nanoseconds since epoch

    // --- v2 block pointer fields ---
    pub direct_blocks: [u64; 10],
    pub indirect_block: u64,
    pub double_indirect: u64,

    // --- v3 fields ---
    pub accessed_ns: u64,      // atime in nanoseconds
    pub changed_ns: u64,       // ctime in nanoseconds
    pub owner_id: u32,
    pub group_id: u32,
    pub permissions: u32,      // lower 12 bits: rwxrwxrwx + setuid/setgid/sticky
    pub flags: u32,            // INODE_FLAG_* bitfield
    pub xattr_block: u64,      // pointer to external xattr block
    pub xattr_inline_size: u32,
    pub checksum: u32,         // CRC32 of bytes [0..172)
    pub block_count_hi: u32,   // upper 32 bits of block count
    pub inline_area: [u8; 76], // inline data / symlink target / xattr
}

impl Inode {
    /// Create a fresh directory inode (v2 compat).
    pub fn new_dir() -> Self {
        let now = unix_now();
        Self {
            mode: INODE_DIR,
            nlinks: 2,
            block_count: 0,
            size: 0,
            created: now,
            modified: now,
            direct_blocks: [0; 10],
            indirect_block: 0,
            double_indirect: 0,
            // v3 defaults
            accessed_ns: now,
            changed_ns: now,
            owner_id: 0,
            group_id: 0,
            permissions: 0o755,
            flags: 0,
            xattr_block: 0,
            xattr_inline_size: 0,
            checksum: 0,
            block_count_hi: 0,
            inline_area: [0u8; 76],
        }
    }

    /// Create a fresh file inode (v2 compat).
    pub fn new_file() -> Self {
        let now = unix_now();
        Self {
            mode: INODE_FILE,
            nlinks: 1,
            block_count: 0,
            size: 0,
            created: now,
            modified: now,
            direct_blocks: [0; 10],
            indirect_block: 0,
            double_indirect: 0,
            // v3 defaults
            accessed_ns: now,
            changed_ns: now,
            owner_id: 0,
            group_id: 0,
            permissions: 0o644,
            flags: 0,
            xattr_block: 0,
            xattr_inline_size: 0,
            checksum: 0,
            block_count_hi: 0,
            inline_area: [0u8; 76],
        }
    }

    /// Create a v3 file inode with extent tree and nanosecond timestamps.
    pub fn new_file_v3(permissions: u32) -> Self {
        let now_ns = unix_now_ns();
        let mut inode = Self {
            mode: INODE_FILE,
            nlinks: 1,
            block_count: 0,
            size: 0,
            created: now_ns,
            modified: now_ns,
            accessed_ns: now_ns,
            changed_ns: now_ns,
            owner_id: 0,
            group_id: 0,
            permissions,
            flags: 0,
            direct_blocks: [0; 10],
            indirect_block: 0,
            double_indirect: 0,
            xattr_block: 0,
            xattr_inline_size: 0,
            checksum: 0,
            block_count_hi: 0,
            inline_area: [0u8; 76],
        };
        inode.init_extent_root();
        inode
    }

    /// Create a v3 directory inode with extent tree and nanosecond timestamps.
    pub fn new_dir_v3(permissions: u32) -> Self {
        let now_ns = unix_now_ns();
        let mut inode = Self {
            mode: INODE_DIR,
            nlinks: 2,
            block_count: 0,
            size: 0,
            created: now_ns,
            modified: now_ns,
            accessed_ns: now_ns,
            changed_ns: now_ns,
            owner_id: 0,
            group_id: 0,
            permissions,
            flags: 0,
            direct_blocks: [0; 10],
            indirect_block: 0,
            double_indirect: 0,
            xattr_block: 0,
            xattr_inline_size: 0,
            checksum: 0,
            block_count_hi: 0,
            inline_area: [0u8; 76],
        };
        inode.init_extent_root();
        inode
    }

    /// Create a v3 symlink inode. Short targets (≤76 bytes) are stored inline.
    pub fn new_symlink(target: &str) -> Self {
        let now_ns = unix_now_ns();
        let target_bytes = target.as_bytes();
        let mut inline = [0u8; 76];
        let use_inline = target_bytes.len() <= 76;
        if use_inline {
            inline[..target_bytes.len()].copy_from_slice(target_bytes);
        }
        let mut inode = Self {
            mode: INODE_SYMLINK,
            nlinks: 1,
            block_count: 0,
            size: target_bytes.len() as u64,
            created: now_ns,
            modified: now_ns,
            accessed_ns: now_ns,
            changed_ns: now_ns,
            owner_id: 0,
            group_id: 0,
            permissions: 0o777,
            flags: if use_inline { INODE_FLAG_INLINE_DATA } else { 0 },
            direct_blocks: [0; 10],
            indirect_block: 0,
            double_indirect: 0,
            xattr_block: 0,
            xattr_inline_size: 0,
            checksum: 0,
            block_count_hi: 0,
            inline_area: inline,
        };
        if !use_inline {
            inode.init_extent_root();
        }
        inode
    }

    // -----------------------------------------------------------------------
    // Block count helpers (64-bit via hi+lo)
    // -----------------------------------------------------------------------

    /// Full 64-bit block count (combines lo and hi).
    pub fn block_count_u64(&self) -> u64 {
        (self.block_count_hi as u64) << 32 | self.block_count as u64
    }

    /// Set the full 64-bit block count.
    pub fn set_block_count_u64(&mut self, count: u64) {
        self.block_count = count as u32;
        self.block_count_hi = (count >> 32) as u32;
    }

    // -----------------------------------------------------------------------
    // Orphan list helpers
    // -----------------------------------------------------------------------

    /// Whether this inode is on the orphan list.
    pub fn is_orphan(&self) -> bool {
        self.flags & INODE_FLAG_ORPHAN != 0
    }

    /// Set the next-orphan inode index (stored in `double_indirect`).
    /// A value of 0 means end of the orphan chain.
    pub fn set_next_orphan(&mut self, next_idx: u32) {
        self.flags |= INODE_FLAG_ORPHAN;
        self.double_indirect = next_idx as u64;
    }

    /// Get the next-orphan inode index (0 = end of chain).
    pub fn get_next_orphan(&self) -> u32 {
        self.double_indirect as u32
    }

    /// Clear the orphan flag and reset the next-orphan field.
    pub fn clear_orphan(&mut self) {
        self.flags &= !INODE_FLAG_ORPHAN;
        self.double_indirect = 0;
    }

    // -----------------------------------------------------------------------
    // Extent tree root initialization
    // -----------------------------------------------------------------------

    /// Initialize the extent tree root in the dual-purpose area.
    /// Sets the EXTENTS flag and writes an empty extent header into
    /// the block pointer fields (bytes [64..160) of the v3 layout).
    pub fn init_extent_root(&mut self) {
        self.flags |= INODE_FLAG_EXTENTS;
        self.direct_blocks = [0u64; 10];
        self.indirect_block = 0;
        self.double_indirect = 0;

        // Write ExtentHeader { magic=0xF30A, entries=0, max=7, depth=0, gen=0 }
        // into the first 12 bytes of the dual-purpose area (direct_blocks[0]
        // holds bytes 0..8, and indirect_block holds bytes 8..12 in its low
        // bytes, but the layout is via raw byte reinterpretation).
        //
        // The 96-byte area is: direct_blocks[0..10] (80B) + indirect_block (8B)
        // + double_indirect (8B). We write the 12-byte header at offset 0.
        let magic: u16 = 0xF30A;
        let entries: u16 = 0;
        let max: u16 = 7; // EXTENT_ROOT_MAX_ENTRIES
        let depth: u16 = 0;
        let generation: u32 = 0;

        let mut hdr = [0u8; 12];
        hdr[0..2].copy_from_slice(&magic.to_le_bytes());
        hdr[2..4].copy_from_slice(&entries.to_le_bytes());
        hdr[4..6].copy_from_slice(&max.to_le_bytes());
        hdr[6..8].copy_from_slice(&depth.to_le_bytes());
        hdr[8..12].copy_from_slice(&generation.to_le_bytes());

        // Write first 8 bytes into direct_blocks[0]
        self.direct_blocks[0] = u64::from_le_bytes(hdr[0..8].try_into().unwrap());
        // Write next 4 bytes into direct_blocks[1] (low 4 bytes)
        let mut db1 = [0u8; 8];
        db1[0..4].copy_from_slice(&hdr[8..12]);
        self.direct_blocks[1] = u64::from_le_bytes(db1);
    }

    // -----------------------------------------------------------------------
    // Serialization — v2 (128 bytes)
    // -----------------------------------------------------------------------

    /// Serialize to exactly 128 bytes (v2 format), little-endian.
    pub fn serialize(&self) -> [u8; 128] {
        let mut buf = [0u8; 128];
        buf[0..2].copy_from_slice(&self.mode.to_le_bytes());
        buf[2..4].copy_from_slice(&self.nlinks.to_le_bytes());
        buf[4..8].copy_from_slice(&self.block_count.to_le_bytes());
        buf[8..16].copy_from_slice(&self.size.to_le_bytes());
        buf[16..24].copy_from_slice(&self.created.to_le_bytes());
        buf[24..32].copy_from_slice(&self.modified.to_le_bytes());
        for (i, &ptr) in self.direct_blocks.iter().enumerate() {
            let off = 32 + i * 8;
            buf[off..off + 8].copy_from_slice(&ptr.to_le_bytes());
        }
        buf[112..120].copy_from_slice(&self.indirect_block.to_le_bytes());
        buf[120..128].copy_from_slice(&self.double_indirect.to_le_bytes());
        buf
    }

    /// Deserialize from exactly 128 bytes (v2 format).
    /// v3 fields are populated with safe defaults.
    pub fn deserialize(buf: &[u8; 128]) -> Self {
        let mode = u16::from_le_bytes(buf[0..2].try_into().unwrap());
        let nlinks = u16::from_le_bytes(buf[2..4].try_into().unwrap());
        let block_count = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let size = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let created = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let modified = u64::from_le_bytes(buf[24..32].try_into().unwrap());

        let mut direct_blocks = [0u64; 10];
        for i in 0..10 {
            let off = 32 + i * 8;
            direct_blocks[i] =
                u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
        }

        let indirect_block =
            u64::from_le_bytes(buf[112..120].try_into().unwrap());
        let double_indirect =
            u64::from_le_bytes(buf[120..128].try_into().unwrap());

        Self {
            mode,
            nlinks,
            block_count,
            size,
            created,
            modified,
            direct_blocks,
            indirect_block,
            double_indirect,
            // v3 defaults
            accessed_ns: created,
            changed_ns: modified,
            owner_id: 0,
            group_id: 0,
            permissions: if mode == INODE_DIR { 0o755 } else { 0o644 },
            flags: 0,
            xattr_block: 0,
            xattr_inline_size: 0,
            checksum: 0,
            block_count_hi: 0,
            inline_area: [0u8; 76],
        }
    }

    // -----------------------------------------------------------------------
    // Serialization — v3 (256 bytes)
    // -----------------------------------------------------------------------

    /// Serialize to exactly 256 bytes (v3 format), little-endian.
    ///
    /// ```text
    /// Offset  Size  Field
    /// 0       2     mode
    /// 2       2     nlinks
    /// 4       4     block_count (lo)
    /// 8       8     size
    /// 16      8     created_ns
    /// 24      8     modified_ns
    /// 32      8     accessed_ns
    /// 40      8     changed_ns
    /// 48      4     owner_id
    /// 52      4     group_id
    /// 56      4     permissions
    /// 60      4     flags
    /// 64      96    dual-purpose: extents or block pointers
    /// 160     8     xattr_block
    /// 168     4     xattr_inline_size
    /// 172     4     checksum (CRC32 of [0..172))
    /// 176     4     block_count_hi
    /// 180     76    inline_area
    /// ```
    pub fn serialize_v3(&self) -> [u8; 256] {
        let mut buf = [0u8; 256];

        buf[0..2].copy_from_slice(&self.mode.to_le_bytes());
        buf[2..4].copy_from_slice(&self.nlinks.to_le_bytes());
        buf[4..8].copy_from_slice(&self.block_count.to_le_bytes());
        buf[8..16].copy_from_slice(&self.size.to_le_bytes());
        buf[16..24].copy_from_slice(&self.created.to_le_bytes());
        buf[24..32].copy_from_slice(&self.modified.to_le_bytes());

        buf[32..40].copy_from_slice(&self.accessed_ns.to_le_bytes());
        buf[40..48].copy_from_slice(&self.changed_ns.to_le_bytes());
        buf[48..52].copy_from_slice(&self.owner_id.to_le_bytes());
        buf[52..56].copy_from_slice(&self.group_id.to_le_bytes());
        buf[56..60].copy_from_slice(&self.permissions.to_le_bytes());
        buf[60..64].copy_from_slice(&self.flags.to_le_bytes());

        // Dual-purpose area [64..160): block pointers (always written the same way
        // — extent tree format is managed by the extent module at a higher level).
        for (i, &ptr) in self.direct_blocks.iter().enumerate() {
            let off = 64 + i * 8;
            buf[off..off + 8].copy_from_slice(&ptr.to_le_bytes());
        }
        buf[144..152].copy_from_slice(&self.indirect_block.to_le_bytes());
        buf[152..160].copy_from_slice(&self.double_indirect.to_le_bytes());

        buf[160..168].copy_from_slice(&self.xattr_block.to_le_bytes());
        buf[168..172].copy_from_slice(&self.xattr_inline_size.to_le_bytes());

        // Compute checksum over [0..172)
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&buf[..172]);
        let crc = hasher.finalize();
        buf[172..176].copy_from_slice(&crc.to_le_bytes());

        buf[176..180].copy_from_slice(&self.block_count_hi.to_le_bytes());
        buf[180..256].copy_from_slice(&self.inline_area);

        buf
    }

    /// Deserialize from exactly 256 bytes (v3 format).
    pub fn deserialize_v3(buf: &[u8; 256]) -> Result<Self> {
        // Verify CRC32 at offset 172 over [0..172)
        let stored_crc = u32::from_le_bytes(buf[172..176].try_into().unwrap());
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&buf[..172]);
        let computed = hasher.finalize();
        if stored_crc != computed {
            bail!(
                "inode v3 checksum mismatch: stored=0x{stored_crc:08x}, computed=0x{computed:08x}"
            );
        }

        let mode = u16::from_le_bytes(buf[0..2].try_into().unwrap());
        let nlinks = u16::from_le_bytes(buf[2..4].try_into().unwrap());
        let block_count = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let size = u64::from_le_bytes(buf[8..16].try_into().unwrap());
        let created = u64::from_le_bytes(buf[16..24].try_into().unwrap());
        let modified = u64::from_le_bytes(buf[24..32].try_into().unwrap());

        let accessed_ns = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        let changed_ns = u64::from_le_bytes(buf[40..48].try_into().unwrap());
        let owner_id = u32::from_le_bytes(buf[48..52].try_into().unwrap());
        let group_id = u32::from_le_bytes(buf[52..56].try_into().unwrap());
        let permissions = u32::from_le_bytes(buf[56..60].try_into().unwrap());
        let flags = u32::from_le_bytes(buf[60..64].try_into().unwrap());

        // Dual-purpose area [64..160)
        let mut direct_blocks = [0u64; 10];
        for i in 0..10 {
            let off = 64 + i * 8;
            direct_blocks[i] = u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
        }
        let indirect_block = u64::from_le_bytes(buf[144..152].try_into().unwrap());
        let double_indirect = u64::from_le_bytes(buf[152..160].try_into().unwrap());

        let xattr_block = u64::from_le_bytes(buf[160..168].try_into().unwrap());
        let xattr_inline_size = u32::from_le_bytes(buf[168..172].try_into().unwrap());
        let checksum = stored_crc;
        let block_count_hi = u32::from_le_bytes(buf[176..180].try_into().unwrap());

        let mut inline_area = [0u8; 76];
        inline_area.copy_from_slice(&buf[180..256]);

        Ok(Self {
            mode,
            nlinks,
            block_count,
            size,
            created,
            modified,
            direct_blocks,
            indirect_block,
            double_indirect,
            accessed_ns,
            changed_ns,
            owner_id,
            group_id,
            permissions,
            flags,
            xattr_block,
            xattr_inline_size,
            checksum,
            block_count_hi,
            inline_area,
        })
    }

    // -----------------------------------------------------------------------
    // Size-dispatched serialization
    // -----------------------------------------------------------------------

    /// Dispatch serialization based on inode_size (128 or 256).
    pub fn serialize_for_size(&self, inode_size: u32) -> Vec<u8> {
        match inode_size {
            128 => self.serialize().to_vec(),
            256 => self.serialize_v3().to_vec(),
            _ => panic!("unsupported inode_size: {}", inode_size),
        }
    }

    /// Dispatch deserialization based on inode_size (128 or 256).
    pub fn deserialize_for_size(buf: &[u8], inode_size: u32) -> Result<Self> {
        match inode_size {
            128 => {
                if buf.len() < 128 {
                    bail!("buffer too small for v2 inode: {} bytes", buf.len());
                }
                Ok(Self::deserialize(buf[..128].try_into().unwrap()))
            }
            256 => {
                if buf.len() < 256 {
                    bail!("buffer too small for v3 inode: {} bytes", buf.len());
                }
                Self::deserialize_v3(buf[..256].try_into().unwrap())
            }
            _ => bail!("unsupported inode_size: {}", inode_size),
        }
    }

    // -----------------------------------------------------------------------
    // Checksum helpers
    // -----------------------------------------------------------------------

    /// Compute CRC32 of bytes [0..172) of the v3 on-disk representation.
    pub fn compute_checksum(&self) -> u32 {
        let buf = self.serialize_v3();
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&buf[..172]);
        hasher.finalize()
    }

    /// Verify checksum. Returns Ok(()) or Err if mismatch.
    pub fn verify_checksum(&self) -> Result<()> {
        let expected = self.compute_checksum();
        if self.checksum != expected {
            bail!(
                "inode checksum mismatch: stored=0x{:08x}, computed=0x{expected:08x}",
                self.checksum
            );
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Timestamp helpers (10D.4)
    // -----------------------------------------------------------------------

    /// Check if atime should be updated given the current atime mode.
    pub fn should_update_atime(&self, mode: super::AtimeMode) -> bool {
        match mode {
            super::AtimeMode::Always => true,
            super::AtimeMode::Never => false,
            super::AtimeMode::Relatime => {
                let now = unix_now_ns();
                // Update if atime <= mtime
                if self.accessed_ns <= self.modified {
                    return true;
                }
                // Update if atime is more than 24 hours old
                if now.saturating_sub(self.accessed_ns) > RELATIME_THRESHOLD_NS {
                    return true;
                }
                false
            }
        }
    }

    /// Update the accessed timestamp to now.
    pub fn touch_atime(&mut self) {
        self.accessed_ns = unix_now_ns();
    }

    /// Update the changed timestamp to now (metadata change).
    pub fn touch_ctime(&mut self) {
        self.changed_ns = unix_now_ns();
    }

    /// Update the modified timestamp to now (data change).
    /// Also updates changed_ns (ctime always moves with mtime).
    pub fn touch_mtime(&mut self) {
        let now = unix_now_ns();
        self.modified = now;
        self.changed_ns = now;
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Get current time as nanoseconds since Unix epoch.
pub fn unix_now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Convert nanoseconds since Unix epoch to seconds.
pub fn ns_to_seconds(ns: u64) -> u64 {
    ns / 1_000_000_000
}

/// Convert seconds to nanoseconds.
pub fn seconds_to_ns(secs: u64) -> u64 {
    secs.saturating_mul(1_000_000_000)
}

/// 24 hours in nanoseconds (for relatime threshold).
const RELATIME_THRESHOLD_NS: u64 = 24 * 60 * 60 * 1_000_000_000;

// ---------------------------------------------------------------------------
// InodeTable — helper for reading/writing inodes on disk
// ---------------------------------------------------------------------------

/// Manages inode I/O on the block device using read-modify-write.
/// Supports both 128-byte (v2) and 256-byte (v3) inodes.
pub struct InodeTable {
    pub inode_table_start: u64, // block index on disk
    pub inode_count: u32,
    pub block_size: u32,
    pub inode_size: u32,        // 128 or 256
}

impl InodeTable {
    pub fn new(inode_table_start: u64, inode_count: u32, block_size: u32) -> Self {
        Self {
            inode_table_start,
            inode_count,
            block_size,
            inode_size: INODE_SIZE, // default: 128 for backward compat
        }
    }

    /// Create with explicit inode size (128 or 256).
    pub fn new_with_inode_size(
        inode_table_start: u64,
        inode_count: u32,
        block_size: u32,
        inode_size: u32,
    ) -> Self {
        Self {
            inode_table_start,
            inode_count,
            block_size,
            inode_size,
        }
    }

    /// Inodes that fit in one disk block.
    pub fn inodes_per_block(&self) -> u32 {
        self.block_size / self.inode_size
    }

    /// Read the inode at the given index from the device.
    pub fn read_inode(
        &self,
        dev: &mut dyn CFSBlockDevice,
        index: u32,
    ) -> Result<Inode> {
        if index >= self.inode_count {
            bail!(
                "inode index {index} out of range (max {})",
                self.inode_count - 1
            );
        }

        let bs = self.block_size as u64;
        let isz = self.inode_size as u64;
        let byte_offset = self.inode_table_start * bs + index as u64 * isz;
        let block_offset = (byte_offset / bs) * bs;
        let offset_within_block = (byte_offset % bs) as usize;

        let mut block_buf = vec![0u8; self.block_size as usize];
        dev.read(block_offset, &mut block_buf)?;

        let inode_bytes = &block_buf[offset_within_block..offset_within_block + self.inode_size as usize];
        Inode::deserialize_for_size(inode_bytes, self.inode_size)
    }

    /// Write an inode at the given index using read-modify-write.
    pub fn write_inode(
        &self,
        dev: &mut dyn CFSBlockDevice,
        index: u32,
        inode: &Inode,
    ) -> Result<()> {
        if index >= self.inode_count {
            bail!(
                "inode index {index} out of range (max {})",
                self.inode_count - 1
            );
        }

        let bs = self.block_size as u64;
        let isz = self.inode_size as u64;
        let byte_offset = self.inode_table_start * bs + index as u64 * isz;
        let block_offset = (byte_offset / bs) * bs;
        let offset_within_block = (byte_offset % bs) as usize;

        // Read-modify-write
        let mut block_buf = vec![0u8; self.block_size as usize];
        dev.read(block_offset, &mut block_buf)?;

        let serialized = inode.serialize_for_size(self.inode_size);
        block_buf[offset_within_block..offset_within_block + self.inode_size as usize]
            .copy_from_slice(&serialized);

        dev.write(block_offset, &block_buf)?;
        Ok(())
    }
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
    fn test_inode_serialize_roundtrip() {
        let mut inode = Inode::new_file();
        inode.size = 12345;
        inode.block_count = 3;
        inode.direct_blocks[0] = 100;
        inode.direct_blocks[1] = 200;
        inode.direct_blocks[2] = 300;
        inode.indirect_block = 999;
        inode.double_indirect = 1234;

        let buf = inode.serialize();
        let inode2 = Inode::deserialize(&buf);
        assert_eq!(inode, inode2);
    }

    #[test]
    fn test_inode_table_read_write() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        // Need enough space: superblock (1 block) + inode table (2 blocks for 64 inodes)
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        let table = InodeTable::new(1, 64, 4096); // start at block 1, 64 inodes

        // Zero the inode table blocks first
        let zero_block = [0u8; 4096];
        dev.write(4096, &zero_block).unwrap(); // block 1
        dev.write(8192, &zero_block).unwrap(); // block 2

        // Write inodes at various indices
        let mut inode0 = Inode::new_dir();
        inode0.size = 111;
        table.write_inode(&mut dev, 0, &inode0).unwrap();

        let mut inode5 = Inode::new_file();
        inode5.size = 555;
        table.write_inode(&mut dev, 5, &inode5).unwrap();

        let mut inode31 = Inode::new_file();
        inode31.size = 3131;
        table.write_inode(&mut dev, 31, &inode31).unwrap();

        // Read back and verify
        let r0 = table.read_inode(&mut dev, 0).unwrap();
        assert_eq!(r0.mode, INODE_DIR);
        assert_eq!(r0.size, 111);

        let r5 = table.read_inode(&mut dev, 5).unwrap();
        assert_eq!(r5.mode, INODE_FILE);
        assert_eq!(r5.size, 555);

        let r31 = table.read_inode(&mut dev, 31).unwrap();
        assert_eq!(r31.size, 3131);
    }

    #[test]
    fn test_inode_table_out_of_range() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        let table = InodeTable::new(1, 64, 4096);
        assert!(table.read_inode(&mut dev, 64).is_err());
        assert!(table.read_inode(&mut dev, 100).is_err());
    }

    #[test]
    fn test_multiple_inodes_same_block() {
        // Verify read-modify-write doesn't clobber neighboring inodes
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        let table = InodeTable::new(1, 64, 4096);

        // Zero the block first
        let zero_block = [0u8; 4096];
        dev.write(4096, &zero_block).unwrap();

        // Write inode 0
        let mut i0 = Inode::new_dir();
        i0.size = 1000;
        table.write_inode(&mut dev, 0, &i0).unwrap();

        // Write inode 1 (same block)
        let mut i1 = Inode::new_file();
        i1.size = 2000;
        table.write_inode(&mut dev, 1, &i1).unwrap();

        // Verify inode 0 survived the write of inode 1
        let r0 = table.read_inode(&mut dev, 0).unwrap();
        assert_eq!(r0.mode, INODE_DIR);
        assert_eq!(r0.size, 1000);

        let r1 = table.read_inode(&mut dev, 1).unwrap();
        assert_eq!(r1.mode, INODE_FILE);
        assert_eq!(r1.size, 2000);
    }

    // --- v3 tests ---

    #[test]
    fn test_inode_v3_serialize_roundtrip() {
        let mut inode = Inode::new_file_v3(0o644);
        inode.size = 99999;
        inode.block_count = 42;
        inode.direct_blocks[0] = 500;
        inode.direct_blocks[5] = 600;
        inode.indirect_block = 777;
        inode.double_indirect = 888;
        inode.owner_id = 1000;
        inode.group_id = 1001;
        inode.xattr_block = 12345;
        inode.xattr_inline_size = 16;
        inode.block_count_hi = 7;
        inode.inline_area[0] = 0xAA;
        inode.inline_area[75] = 0xBB;

        // Set checksum before serializing
        inode.checksum = inode.compute_checksum();

        let buf = inode.serialize_v3();
        assert_eq!(buf.len(), 256);
        let inode2 = Inode::deserialize_v3(&buf).unwrap();
        assert_eq!(inode, inode2);
    }

    #[test]
    fn test_inode_v3_dir_roundtrip() {
        let mut inode = Inode::new_dir_v3(0o755);
        inode.size = 4096;
        inode.block_count = 1;
        inode.direct_blocks[0] = 42;
        inode.checksum = inode.compute_checksum();

        let buf = inode.serialize_v3();
        let inode2 = Inode::deserialize_v3(&buf).unwrap();
        assert_eq!(inode.mode, inode2.mode);
        assert_eq!(inode.nlinks, inode2.nlinks);
        assert_eq!(inode.permissions, inode2.permissions);
        assert_eq!(inode.flags, inode2.flags);
    }

    #[test]
    fn test_inode_v3_symlink_inline() {
        let inode = Inode::new_symlink("hello");
        assert_eq!(inode.mode, INODE_SYMLINK);
        assert_eq!(inode.size, 5);
        assert!(inode.flags & INODE_FLAG_INLINE_DATA != 0);
        assert_eq!(&inode.inline_area[..5], b"hello");
        assert_eq!(inode.permissions, 0o777);
    }

    #[test]
    fn test_inode_v3_symlink_long() {
        let long_target = "a".repeat(80);
        let inode = Inode::new_symlink(&long_target);
        assert_eq!(inode.mode, INODE_SYMLINK);
        assert_eq!(inode.size, 80);
        // Too long for inline — should use extents
        assert!(inode.flags & INODE_FLAG_INLINE_DATA == 0);
        assert!(inode.flags & INODE_FLAG_EXTENTS != 0);
        // Verify extent header was properly initialized (magic=0xF30A)
        let expected_db0 = u64::from_le_bytes([0x0A, 0xF3, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00]);
        assert_eq!(inode.direct_blocks[0], expected_db0);
    }

    #[test]
    fn test_inode_v3_checksum_valid() {
        let mut inode = Inode::new_file_v3(0o600);
        inode.size = 1024;
        inode.block_count = 1;
        inode.checksum = inode.compute_checksum();
        assert!(inode.verify_checksum().is_ok());
    }

    #[test]
    fn test_inode_v3_checksum_corrupt() {
        let mut inode = Inode::new_file_v3(0o600);
        inode.size = 1024;
        inode.checksum = inode.compute_checksum();

        // Corrupt a field that affects the checksum region
        let mut buf = inode.serialize_v3();
        buf[10] ^= 0xFF; // flip a bit in the size field
        let result = Inode::deserialize_v3(&buf);
        assert!(result.is_err());
    }

    #[test]
    fn test_inode_v3_block_count_u64() {
        let mut inode = Inode::new_file_v3(0o644);
        inode.block_count = 0xFFFF_FFFF;
        inode.block_count_hi = 1;
        assert_eq!(inode.block_count_u64(), 0x1_FFFF_FFFF);

        inode.set_block_count_u64(0x2_0000_0001);
        assert_eq!(inode.block_count, 1);
        assert_eq!(inode.block_count_hi, 2);
        assert_eq!(inode.block_count_u64(), 0x2_0000_0001);
    }

    #[test]
    fn test_inode_v3_extents_flag() {
        let inode = Inode::new_file_v3(0o644);
        assert!(inode.flags & INODE_FLAG_EXTENTS != 0);

        // v2 files should NOT have extents flag
        let inode_v2 = Inode::new_file();
        assert_eq!(inode_v2.flags & INODE_FLAG_EXTENTS, 0);
    }

    #[test]
    fn test_inode_v2_compat() {
        // Serialize as v2, deserialize — v3 fields should get defaults
        let mut inode = Inode::new_file();
        inode.size = 42;
        inode.block_count = 1;
        inode.direct_blocks[0] = 100;

        let buf = inode.serialize();
        let inode2 = Inode::deserialize(&buf);

        assert_eq!(inode2.mode, INODE_FILE);
        assert_eq!(inode2.size, 42);
        assert_eq!(inode2.permissions, 0o644); // default for files
        assert_eq!(inode2.flags, 0);           // no extents in v2
        assert_eq!(inode2.block_count_hi, 0);
        assert_eq!(inode2.owner_id, 0);
    }

    #[test]
    fn test_inode_v2_no_extents() {
        // v2 deserialized inode should have flags=0
        let inode = Inode::new_dir();
        let buf = inode.serialize();
        let inode2 = Inode::deserialize(&buf);
        assert_eq!(inode2.flags, 0);
    }

    #[test]
    fn test_inode_serialize_for_size_128() {
        let inode = Inode::new_file();
        let buf = inode.serialize_for_size(128);
        assert_eq!(buf.len(), 128);
    }

    #[test]
    fn test_inode_serialize_for_size_256() {
        let mut inode = Inode::new_file_v3(0o644);
        inode.checksum = inode.compute_checksum();
        let buf = inode.serialize_for_size(256);
        assert_eq!(buf.len(), 256);
    }

    #[test]
    fn test_inode_deserialize_for_size_dispatch() {
        // v2 (128)
        let inode_v2 = Inode::new_file();
        let buf128 = inode_v2.serialize_for_size(128);
        let r = Inode::deserialize_for_size(&buf128, 128).unwrap();
        assert_eq!(r.mode, INODE_FILE);

        // v3 (256)
        let mut inode_v3 = Inode::new_file_v3(0o600);
        inode_v3.checksum = inode_v3.compute_checksum();
        let buf256 = inode_v3.serialize_for_size(256);
        let r = Inode::deserialize_for_size(&buf256, 256).unwrap();
        assert_eq!(r.mode, INODE_FILE);
        assert_eq!(r.permissions, 0o600);
    }

    #[test]
    fn test_inode_init_extent_root() {
        let mut inode = Inode::new_file();
        assert_eq!(inode.flags & INODE_FLAG_EXTENTS, 0);
        inode.init_extent_root();
        assert!(inode.flags & INODE_FLAG_EXTENTS != 0);

        // direct_blocks[0] holds the first 8 bytes of the extent header:
        // magic=0xF30A, entries=0, max=7, depth=0
        let expected_db0 = u64::from_le_bytes([0x0A, 0xF3, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00]);
        assert_eq!(inode.direct_blocks[0], expected_db0);
        // direct_blocks[1] holds generation=0 (low 4 bytes)
        assert_eq!(inode.direct_blocks[1], 0);
        // Remaining direct_blocks are zero
        for &db in &inode.direct_blocks[2..] {
            assert_eq!(db, 0);
        }
        assert_eq!(inode.indirect_block, 0);
        assert_eq!(inode.double_indirect, 0);
    }

    // --- 10A.4: InodeTable variable size tests ---

    #[test]
    fn test_inode_table_v3_read_write() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        // 256-byte inodes: 4096/256 = 16 per block
        let table = InodeTable::new_with_inode_size(1, 32, 4096, 256);
        assert_eq!(table.inodes_per_block(), 16);

        // Zero inode table blocks
        let zero_block = [0u8; 4096];
        dev.write(4096, &zero_block).unwrap();
        dev.write(8192, &zero_block).unwrap();

        let mut inode = Inode::new_file_v3(0o644);
        inode.size = 12345;
        inode.block_count = 3;
        inode.owner_id = 1000;
        inode.checksum = inode.compute_checksum();
        table.write_inode(&mut dev, 0, &inode).unwrap();

        let r = table.read_inode(&mut dev, 0).unwrap();
        assert_eq!(r.size, 12345);
        assert_eq!(r.block_count, 3);
        assert_eq!(r.owner_id, 1000);
        assert_eq!(r.permissions, 0o644);
    }

    #[test]
    fn test_inode_table_v3_16_per_block() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        let table = InodeTable::new_with_inode_size(1, 32, 4096, 256);

        let zero_block = [0u8; 4096];
        dev.write(4096, &zero_block).unwrap();
        dev.write(8192, &zero_block).unwrap();

        // Write inodes 0 and 15 (both in first block)
        let mut i0 = Inode::new_dir_v3(0o755);
        i0.size = 111;
        i0.checksum = i0.compute_checksum();
        table.write_inode(&mut dev, 0, &i0).unwrap();

        let mut i15 = Inode::new_file_v3(0o600);
        i15.size = 1515;
        i15.checksum = i15.compute_checksum();
        table.write_inode(&mut dev, 15, &i15).unwrap();

        // Both survive
        let r0 = table.read_inode(&mut dev, 0).unwrap();
        assert_eq!(r0.mode, INODE_DIR);
        assert_eq!(r0.size, 111);

        let r15 = table.read_inode(&mut dev, 15).unwrap();
        assert_eq!(r15.mode, INODE_FILE);
        assert_eq!(r15.size, 1515);
    }

    #[test]
    fn test_inode_table_v2_compat() {
        // InodeTable::new() defaults to inode_size=128
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        let table = InodeTable::new(1, 64, 4096);
        assert_eq!(table.inode_size, 128);
        assert_eq!(table.inodes_per_block(), 32);

        let zero_block = [0u8; 4096];
        dev.write(4096, &zero_block).unwrap();

        let mut inode = Inode::new_file();
        inode.size = 42;
        table.write_inode(&mut dev, 0, &inode).unwrap();

        let r = table.read_inode(&mut dev, 0).unwrap();
        assert_eq!(r.mode, INODE_FILE);
        assert_eq!(r.size, 42);
    }

    #[test]
    fn test_inode_table_cross_block() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        // 256-byte inodes: 16 per 4096-byte block
        let table = InodeTable::new_with_inode_size(1, 32, 4096, 256);

        let zero_block = [0u8; 4096];
        dev.write(4096, &zero_block).unwrap();
        dev.write(8192, &zero_block).unwrap();

        // Inode 16 is in the second block
        let mut inode = Inode::new_file_v3(0o755);
        inode.size = 9876;
        inode.checksum = inode.compute_checksum();
        table.write_inode(&mut dev, 16, &inode).unwrap();

        let r = table.read_inode(&mut dev, 16).unwrap();
        assert_eq!(r.size, 9876);
        assert_eq!(r.permissions, 0o755);
    }
}
