use anyhow::{bail, Result};

use crate::block_device::CFSBlockDevice;

// ---------------------------------------------------------------------------
// Bitmap — free-block allocation bitmap
// ---------------------------------------------------------------------------

/// Number of usable bitmap bits per block when a 4-byte CRC32 checksum is
/// appended at the end of each block (v3 layout).
pub fn usable_bits_per_block(block_size: u32) -> u32 {
    (block_size - 4) * 8
}

/// In-memory bitmap cache for tracking data-block allocation.
///
/// - Bit = 1 → block is allocated
/// - Bit = 0 → block is free
/// - Bit index `i` → data block at disk block `data_start + i`
pub struct Bitmap {
    data: Vec<u8>,
    total_bits: u64,
    bitmap_start: u64,  // starting block index on disk
    bitmap_blocks: u64, // number of blocks the bitmap occupies
    block_size: u32,
    next_free_hint: u64, // first potentially-free bit (avoids O(n) scan on every alloc)
}

impl Bitmap {
    /// Create a zeroed bitmap (all blocks free).
    pub fn new_empty(
        total_bits: u64,
        bitmap_start: u64,
        bitmap_blocks: u64,
        block_size: u32,
    ) -> Self {
        let byte_count = (bitmap_blocks * block_size as u64) as usize;
        Self {
            data: vec![0u8; byte_count],
            total_bits,
            bitmap_start,
            bitmap_blocks,
            block_size,
            next_free_hint: 0,
        }
    }

    /// Reset the hint to the beginning (call after loading from disk).
    pub fn reset_hint(&mut self) {
        self.next_free_hint = 0;
    }

    /// Load the bitmap from disk into the in-memory cache.
    pub fn load(&mut self, dev: &mut dyn CFSBlockDevice) -> Result<()> {
        let bs = self.block_size as u64;
        for b in 0..self.bitmap_blocks {
            let disk_offset = (self.bitmap_start + b) * bs;
            let buf_start = (b * bs) as usize;
            let buf_end = buf_start + self.block_size as usize;
            dev.read(disk_offset, &mut self.data[buf_start..buf_end])?;
        }
        self.reset_hint();
        Ok(())
    }

    /// Write the in-memory bitmap back to disk.
    pub fn save(&self, dev: &mut dyn CFSBlockDevice) -> Result<()> {
        let bs = self.block_size as u64;
        for b in 0..self.bitmap_blocks {
            let disk_offset = (self.bitmap_start + b) * bs;
            let buf_start = (b * bs) as usize;
            let buf_end = buf_start + self.block_size as usize;
            dev.write(disk_offset, &self.data[buf_start..buf_end])?;
        }
        Ok(())
    }

    /// Save bitmap data to disk with per-block CRC32 checksums (v3).
    ///
    /// Each on-disk block stores `block_size - 4` bytes of bitmap data
    /// followed by a 4-byte CRC32 over those data bytes.
    pub fn save_with_checksums(&self, dev: &mut dyn CFSBlockDevice) -> Result<()> {
        let bs = self.block_size as usize;
        let data_per_block = bs - 4;

        for b in 0..self.bitmap_blocks as usize {
            let data_start = b * data_per_block;
            let data_end = (data_start + data_per_block).min(self.data.len());

            let mut block_buf = vec![0u8; bs];
            let slice = &self.data[data_start..data_end];
            block_buf[..slice.len()].copy_from_slice(slice);

            let checksum = crc32fast::hash(&block_buf[..data_per_block]);
            block_buf[bs - 4..bs].copy_from_slice(&checksum.to_le_bytes());

            let disk_offset = (self.bitmap_start + b as u64) * self.block_size as u64;
            dev.write(disk_offset, &block_buf)?;
        }
        Ok(())
    }

    /// Load bitmap data from disk and verify per-block CRC32 checksums (v3).
    ///
    /// Returns an error if any block's checksum does not match.
    pub fn load_with_checksums(&mut self, dev: &mut dyn CFSBlockDevice) -> Result<()> {
        let bs = self.block_size as usize;
        let data_per_block = bs - 4;

        for b in 0..self.bitmap_blocks as usize {
            let disk_offset = (self.bitmap_start + b as u64) * self.block_size as u64;
            let mut block_buf = vec![0u8; bs];
            dev.read(disk_offset, &mut block_buf)?;

            let stored = u32::from_le_bytes(block_buf[bs - 4..bs].try_into().unwrap());
            let computed = crc32fast::hash(&block_buf[..data_per_block]);
            if stored != computed {
                bail!(
                    "bitmap block {} checksum mismatch: stored=0x{:08x}, computed=0x{:08x}",
                    b, stored, computed
                );
            }

            let data_start = b * data_per_block;
            let data_end = (data_start + data_per_block).min(self.data.len());
            let slice_len = data_end - data_start;
            self.data[data_start..data_start + slice_len]
                .copy_from_slice(&block_buf[..slice_len]);
        }
        self.reset_hint();
        Ok(())
    }

    /// Allocate one block. Returns the bit index, or `None` if disk is full.
    ///
    /// Starts scanning from `next_free_hint` for O(1) average cost.
    pub fn alloc(&mut self) -> Option<u64> {
        let start_byte = (self.next_free_hint / 8) as usize;
        let total_bytes = self.data.len();
        // Scan from hint to end, then wrap from 0 to hint
        for pass in 0..2usize {
            let (lo, hi) = if pass == 0 {
                (start_byte, total_bytes)
            } else {
                (0, start_byte)
            };
            for byte_idx in lo..hi {
                if self.data[byte_idx] != 0xFF {
                    for bit in 0..8u64 {
                        let global_bit = byte_idx as u64 * 8 + bit;
                        if global_bit >= self.total_bits {
                            return None;
                        }
                        if (self.data[byte_idx] >> bit) & 1 == 0 {
                            self.data[byte_idx] |= 1 << bit;
                            self.next_free_hint = global_bit + 1;
                            if self.next_free_hint >= self.total_bits {
                                self.next_free_hint = 0;
                            }
                            return Some(global_bit);
                        }
                    }
                }
            }
        }
        None
    }

    /// Free a previously allocated block. Errors on double-free.
    pub fn free(&mut self, index: u64) -> Result<()> {
        if index >= self.total_bits {
            bail!("bitmap index {index} out of range (max {})", self.total_bits);
        }
        let byte_idx = (index / 8) as usize;
        let bit = index % 8;
        if (self.data[byte_idx] >> bit) & 1 == 0 {
            bail!("double-free: block {index} is already free");
        }
        self.data[byte_idx] &= !(1 << bit);
        // Pull the hint back if this freed block comes before the current hint
        if index < self.next_free_hint {
            self.next_free_hint = index;
        }
        Ok(())
    }

    /// Check whether a block is free.
    pub fn is_free(&self, index: u64) -> bool {
        if index >= self.total_bits {
            return false;
        }
        let byte_idx = (index / 8) as usize;
        let bit = index % 8;
        (self.data[byte_idx] >> bit) & 1 == 0
    }

    /// Count the number of free blocks.
    pub fn free_count(&self) -> u64 {
        let mut count = 0u64;
        for i in 0..self.total_bits {
            if self.is_free(i) {
                count += 1;
            }
        }
        count
    }

    // -- Group bitmap helpers (10B.4) --

    /// Create a bitmap with `total_bits` capacity, all blocks free.
    /// Does not have disk location — used for per-group in-memory bitmaps.
    pub fn new_all_free(total_bits: u32) -> Self {
        let byte_count = ((total_bits as u64 + 7) / 8) as usize;
        Self {
            data: vec![0u8; byte_count],
            total_bits: total_bits as u64,
            bitmap_start: 0,
            bitmap_blocks: 0,
            block_size: 0,
            next_free_hint: 0,
        }
    }

    /// Load a bitmap from raw bytes. Bits beyond `total_bits` are ignored.
    pub fn from_bytes(bytes: &[u8], total_bits: u32) -> Self {
        let byte_count = ((total_bits as u64 + 7) / 8) as usize;
        let mut data = vec![0u8; byte_count];
        let copy_len = byte_count.min(bytes.len());
        data[..copy_len].copy_from_slice(&bytes[..copy_len]);
        Self {
            data,
            total_bits: total_bits as u64,
            bitmap_start: 0,
            bitmap_blocks: 0,
            block_size: 0,
            next_free_hint: 0,
        }
    }

    /// Return a read-only view of the backing bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Mark a bit as allocated (set to 1). No double-alloc check.
    /// Used internally by contiguous allocation.
    pub fn set_allocated(&mut self, index: u64) {
        assert!(
            index < self.total_bits,
            "set_allocated: index {} out of bounds (total_bits {})",
            index,
            self.total_bits
        );
        let byte_idx = (index / 8) as usize;
        let bit = index % 8;
        self.data[byte_idx] |= 1 << bit;
    }

    /// Find the first run of `count` contiguous free bits starting at or after `hint`.
    /// Returns the starting bit index, or `None` if no such run exists.
    ///
    /// Scans byte-by-byte, skipping fully-allocated bytes (0xFF) for efficiency.
    /// Wraps around once from the end back to bit 0 if needed.
    pub fn find_contiguous_run(&self, count: u64, hint: u64) -> Option<u64> {
        if count == 0 || count > self.total_bits {
            return None;
        }
        let total_bytes = self.data.len();
        let hint_byte = (hint / 8) as usize;
        let mut run_start: Option<u64> = None;
        let mut run_len: u64 = 0;

        // Two passes: [hint_byte..end), then [0..hint_byte)
        for pass in 0..2 {
            let (start_byte, end_byte) = if pass == 0 {
                (hint_byte, total_bytes)
            } else {
                (0, hint_byte)
            };

            for byte_idx in start_byte..end_byte {
                let b = self.data[byte_idx];
                if b == 0xFF {
                    // All bits allocated — reset run
                    run_start = None;
                    run_len = 0;
                    continue;
                }
                for bit_in_byte in 0..8u64 {
                    let global_bit = byte_idx as u64 * 8 + bit_in_byte;
                    if global_bit >= self.total_bits {
                        // Past valid bits — reset and stop this byte
                        run_start = None;
                        run_len = 0;
                        break;
                    }
                    if b & (1 << bit_in_byte) == 0 {
                        if run_start.is_none() {
                            run_start = Some(global_bit);
                            run_len = 0;
                        }
                        run_len += 1;
                        if run_len >= count {
                            return run_start;
                        }
                    } else {
                        run_start = None;
                        run_len = 0;
                    }
                }
            }
            // Reset run tracking at wrap boundary
            run_start = None;
            run_len = 0;
        }
        None
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
    fn test_alloc_sequential() {
        let mut bm = Bitmap::new_empty(100, 0, 1, 4096);
        let mut indices = Vec::new();
        for _ in 0..20 {
            let idx = bm.alloc().expect("alloc should succeed");
            assert!(idx < 100);
            assert!(!indices.contains(&idx));
            indices.push(idx);
        }
        assert_eq!(indices.len(), 20);
        // Should be sequential 0..20
        for (i, &idx) in indices.iter().enumerate() {
            assert_eq!(idx, i as u64);
        }
    }

    #[test]
    fn test_free_and_realloc() {
        let mut bm = Bitmap::new_empty(100, 0, 1, 4096);
        // Alloc 10
        for _ in 0..10 {
            bm.alloc().unwrap();
        }
        // Free indices 3 and 7
        bm.free(3).unwrap();
        bm.free(7).unwrap();
        // Next two allocs should return 3, then 7 (first-fit)
        assert_eq!(bm.alloc(), Some(3));
        assert_eq!(bm.alloc(), Some(7));
    }

    #[test]
    fn test_exhaustion() {
        let mut bm = Bitmap::new_empty(8, 0, 1, 4096);
        for i in 0..8 {
            assert_eq!(bm.alloc(), Some(i));
        }
        assert_eq!(bm.alloc(), None);
    }

    #[test]
    fn test_double_free() {
        let mut bm = Bitmap::new_empty(16, 0, 1, 4096);
        let idx = bm.alloc().unwrap();
        bm.free(idx).unwrap();
        assert!(bm.free(idx).is_err());
    }

    #[test]
    fn test_save_load_roundtrip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let mut dev = FileBlockDevice::open(&path, Some(1_048_576)).unwrap();

        // Create bitmap at block 3 (offset 12288), 1 block
        let mut bm = Bitmap::new_empty(100, 3, 1, 4096);
        // Alloc some blocks
        for _ in 0..15 {
            bm.alloc().unwrap();
        }
        bm.free(5).unwrap();

        bm.save(&mut dev).unwrap();
        dev.flush().unwrap();

        // Load into a fresh bitmap
        let mut bm2 = Bitmap::new_empty(100, 3, 1, 4096);
        bm2.load(&mut dev).unwrap();

        assert_eq!(bm2.free_count(), bm.free_count());
        assert!(bm2.is_free(5));
        assert!(!bm2.is_free(0));
        assert!(!bm2.is_free(14));
    }

    #[test]
    fn test_free_count() {
        let mut bm = Bitmap::new_empty(32, 0, 1, 4096);
        assert_eq!(bm.free_count(), 32);
        bm.alloc().unwrap();
        bm.alloc().unwrap();
        assert_eq!(bm.free_count(), 30);
        bm.free(0).unwrap();
        assert_eq!(bm.free_count(), 31);
    }
}
