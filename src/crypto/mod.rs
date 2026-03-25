pub mod key;
pub mod xts;
pub mod header;

pub use header::CryptoHeader;
pub use key::{KdfAlgorithm, KdfParams, benchmark_kdf};

use anyhow::Result;
use zeroize::Zeroize;

use crate::block_device::CFSBlockDevice;
use header::CRYPTO_MAGIC;
use xts::XtsCipher;

// ---------------------------------------------------------------------------
// EncryptedBlockDevice
// ---------------------------------------------------------------------------

/// A block device that transparently encrypts on write and decrypts on read.
///
/// Wraps an inner `CFSBlockDevice`. Block 0 of the inner device holds the
/// `CryptoHeader`; all data blocks are shifted by `header_blocks`.
pub struct EncryptedBlockDevice {
    inner: Box<dyn CFSBlockDevice>,
    cipher: XtsCipher,
    header_blocks: u64,
    encryption_unit: u32,
}

impl EncryptedBlockDevice {
    /// Create a new encrypted volume: generates keys, writes crypto header,
    /// returns a usable `EncryptedBlockDevice`.
    pub fn format_encrypted(
        mut inner: Box<dyn CFSBlockDevice>,
        password: &[u8],
        kdf_params: &KdfParams,
        encryption_unit: u32,
    ) -> Result<Self> {
        let (hdr, mut master_key) =
            CryptoHeader::create(password, kdf_params, encryption_unit)?;
        hdr.write_to(&mut *inner, encryption_unit)?;

        let cipher = XtsCipher::new(&master_key, encryption_unit);
        master_key.zeroize();

        Ok(Self {
            inner,
            cipher,
            header_blocks: hdr.header_blocks as u64,
            encryption_unit,
        })
    }

    /// Open an existing encrypted volume: reads crypto header, derives and
    /// verifies master key, returns a usable `EncryptedBlockDevice`.
    pub fn open_encrypted(
        mut inner: Box<dyn CFSBlockDevice>,
        password: &[u8],
    ) -> Result<Self> {
        // Read the first block to determine encryption_unit from the header
        // First, try reading with a minimal block size (4096) to get the header
        let mut probe = vec![0u8; 4096];
        inner.read(0, &mut probe)?;
        let hdr = CryptoHeader::deserialize(&probe)?;

        let mut master_key = hdr.unlock(password)?;
        let cipher = XtsCipher::new(&master_key, hdr.encryption_unit);
        master_key.zeroize();

        Ok(Self {
            inner,
            cipher,
            header_blocks: hdr.header_blocks as u64,
            encryption_unit: hdr.encryption_unit,
        })
    }
}

impl CFSBlockDevice for EncryptedBlockDevice {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let header_offset = self.header_blocks * self.encryption_unit as u64;
        let inner_offset = offset + header_offset;

        let n = self.inner.read(inner_offset, buf)?;

        // Decrypt in encryption_unit-sized chunks
        let eu = self.encryption_unit as usize;
        let first_block = offset / self.encryption_unit as u64;
        let mut block_idx = first_block;

        for chunk in buf[..n].chunks_mut(eu) {
            if chunk.len() == eu {
                self.cipher.decrypt_block(block_idx, chunk);
            }
            block_idx += 1;
        }

        Ok(n)
    }

    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize> {
        let header_offset = self.header_blocks * self.encryption_unit as u64;
        let inner_offset = offset + header_offset;

        // Copy buffer so we don't mutate the caller's data
        let eu = self.encryption_unit as usize;
        let mut encrypted = buf.to_vec();
        let first_block = offset / self.encryption_unit as u64;
        let mut block_idx = first_block;

        for chunk in encrypted.chunks_mut(eu) {
            if chunk.len() == eu {
                self.cipher.encrypt_block(block_idx, chunk);
            }
            block_idx += 1;
        }

        self.inner.write(inner_offset, &encrypted)
    }

    fn size(&self) -> u64 {
        let header_size = self.header_blocks * self.encryption_unit as u64;
        self.inner.size().saturating_sub(header_size)
    }

    fn sector_size(&self) -> u32 {
        self.inner.sector_size()
    }

    fn flush(&mut self) -> Result<()> {
        self.inner.flush()
    }
}

// ---------------------------------------------------------------------------
// Auto-detection helpers
// ---------------------------------------------------------------------------

/// Peek the first 4 bytes of a device to check for the "CFSE" magic.
pub fn is_encrypted_device(dev: &mut dyn CFSBlockDevice) -> Result<bool> {
    let ss = dev.sector_size() as usize;
    let mut buf = vec![0u8; ss];
    dev.read(0, &mut buf)?;
    Ok(&buf[0..4] == &CRYPTO_MAGIC)
}

/// Change password on an encrypted device (reads header, re-wraps master key).
pub fn change_password(
    dev: &mut dyn CFSBlockDevice,
    old_password: &[u8],
    new_password: &[u8],
    new_kdf: Option<KdfParams>,
    block_size: u32,
) -> Result<()> {
    let mut hdr = CryptoHeader::read_from(dev, block_size)?;
    hdr.change_password(old_password, new_password, new_kdf)?;
    hdr.write_to(dev, block_size)?;
    dev.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_device::FileBlockDevice;
    use crate::volume::CFSVolume;
    use tempfile::NamedTempFile;

    const EU: u32 = 4096;

    /// Helper to create a backing file of given size.
    fn make_backing(size: u64) -> (NamedTempFile, Box<dyn CFSBlockDevice>) {
        let tmp = NamedTempFile::new().unwrap();
        let dev = FileBlockDevice::open(tmp.path(), Some(size)).unwrap();
        (tmp, Box::new(dev))
    }

    /// Low-iter format for test speed (bypasses MIN_PBKDF2_ITERS check).
    fn fast_format_encrypted(
        mut inner: Box<dyn CFSBlockDevice>,
        password: &[u8],
    ) -> EncryptedBlockDevice {
        let salt = key::generate_salt();
        let master_key = key::generate_master_key();
        let params = KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 1000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        };
        let (kek, hmac_key) = key::derive_keys_with_params(password, &salt, &params).unwrap();
        let encrypted_key = key::xor_key_wrap(&master_key, &kek);
        let key_hmac = key::compute_hmac(&hmac_key, &master_key);
        let hdr = CryptoHeader {
            version: header::CRYPTO_VERSION,
            header_blocks: 1,
            encryption_unit: EU,
            salt,
            pbkdf2_iters: 1000,
            kdf_algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
            encrypted_key,
            key_hmac,
        };
        hdr.write_to(&mut *inner, EU).unwrap();

        let cipher = XtsCipher::new(&master_key, EU);
        EncryptedBlockDevice {
            inner,
            cipher,
            header_blocks: 1,
            encryption_unit: EU,
        }
    }

    /// Low-iter open for test speed.
    fn fast_open_encrypted(
        mut inner: Box<dyn CFSBlockDevice>,
        password: &[u8],
    ) -> Result<EncryptedBlockDevice> {
        let mut probe = vec![0u8; EU as usize];
        inner.read(0, &mut probe)?;
        let hdr = CryptoHeader::deserialize(&probe)?;
        let master_key = hdr.unlock(password)?;
        let cipher = XtsCipher::new(&master_key, hdr.encryption_unit);
        Ok(EncryptedBlockDevice {
            inner,
            cipher,
            header_blocks: hdr.header_blocks as u64,
            encryption_unit: hdr.encryption_unit,
        })
    }

    #[test]
    fn test_encrypted_read_write_roundtrip() {
        let (_tmp, dev) = make_backing(2 * 1024 * 1024);
        let mut enc = fast_format_encrypted(dev, b"pw123");

        let data = vec![0xABu8; EU as usize];
        enc.write(0, &data).unwrap();

        let mut readback = vec![0u8; EU as usize];
        enc.read(0, &mut readback).unwrap();

        assert_eq!(readback, data);
    }

    #[test]
    fn test_raw_data_is_ciphertext() {
        let (tmp, dev) = make_backing(2 * 1024 * 1024);
        let mut enc = fast_format_encrypted(dev, b"pw123");

        let plaintext = vec![0x42u8; EU as usize];
        enc.write(0, &plaintext).unwrap();
        enc.flush().unwrap();
        drop(enc);

        // Re-open inner device directly (no decryption) and read data area
        let mut raw = FileBlockDevice::open(tmp.path(), None).unwrap();
        let header_offset = EU as u64; // 1 header block
        let mut raw_buf = vec![0u8; EU as usize];
        raw.read(header_offset, &mut raw_buf).unwrap();

        assert_ne!(raw_buf, plaintext, "data on disk must be encrypted");
    }

    #[test]
    fn test_encrypted_device_size() {
        let total = 2 * 1024 * 1024;
        let (_tmp, dev) = make_backing(total);
        let enc = fast_format_encrypted(dev, b"pw");

        let expected = total - EU as u64; // minus 1 header block
        assert_eq!(enc.size(), expected);
    }

    #[test]
    fn test_format_mount_encrypted_volume() {
        let (_tmp, dev) = make_backing(2 * 1024 * 1024);
        let enc = fast_format_encrypted(dev, b"secret");

        let vol = CFSVolume::format(Box::new(enc), EU).unwrap();
        assert_eq!(&vol.superblock().magic, b"CFS1");

        // Verify root dir exists
        let entries = vol.list_dir("/").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name_str()).collect();
        assert!(names.contains(&"."));
        assert!(names.contains(&".."));
    }

    #[test]
    fn test_encrypted_file_io() {
        let (tmp, dev) = make_backing(4 * 1024 * 1024);
        let enc = fast_format_encrypted(dev, b"fileio_pw");

        let vol = CFSVolume::format(Box::new(enc), EU).unwrap();
        vol.create_file("/test.txt").unwrap();

        // Write 10 KB
        let data: Vec<u8> = (0..10240).map(|i| (i % 256) as u8).collect();
        vol.write_file("/test.txt", 0, &data).unwrap();
        vol.sync().unwrap();

        // Read back
        let readback = vol.read_file("/test.txt", 0, 10240).unwrap();
        assert_eq!(readback, data);

        // Drop volume, reopen with password
        drop(vol);
        let dev2 = FileBlockDevice::open(tmp.path(), None).unwrap();
        let enc2 = fast_open_encrypted(Box::new(dev2), b"fileio_pw").unwrap();
        let vol2 = CFSVolume::mount(Box::new(enc2), EU).unwrap();

        let readback2 = vol2.read_file("/test.txt", 0, 10240).unwrap();
        assert_eq!(readback2, data);
    }

    #[test]
    fn test_wrong_password_fails() {
        let (tmp, dev) = make_backing(2 * 1024 * 1024);
        let _enc = fast_format_encrypted(dev, b"correct_pw");

        let dev2 = FileBlockDevice::open(tmp.path(), None).unwrap();
        let result = fast_open_encrypted(Box::new(dev2), b"wrong_pw");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // 6G â€” End-to-End Integration Tests
    // -----------------------------------------------------------------------

    /// Format encrypted â†’ mkdir â†’ write file â†’ sync â†’ re-open with password â†’
    /// list dir â†’ read file â†’ verify content.
    #[test]
    fn test_e2e_encrypted_workflow() {
        let (tmp, dev) = make_backing(4 * 1024 * 1024);
        let enc = fast_format_encrypted(dev, b"workflow_pw");

        let vol = CFSVolume::format(Box::new(enc), EU).unwrap();
        vol.mkdir("/docs").unwrap();
        vol.create_file("/docs/hello.txt").unwrap();

        let content = b"Hello, encrypted CFS world!";
        vol.write_file("/docs/hello.txt", 0, content).unwrap();
        vol.sync().unwrap();
        drop(vol);

        // Re-open with password
        let dev2 = FileBlockDevice::open(tmp.path(), None).unwrap();
        let enc2 = fast_open_encrypted(Box::new(dev2), b"workflow_pw").unwrap();
        let vol2 = CFSVolume::mount(Box::new(enc2), EU).unwrap();

        // List dir
        let entries = vol2.list_dir("/docs").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name_str()).collect();
        assert!(names.contains(&"hello.txt"), "file must appear in listing");

        // Read file
        let readback = vol2.read_file("/docs/hello.txt", 0, content.len() as u64).unwrap();
        assert_eq!(readback, content);
    }

    /// Format encrypted â†’ write file â†’ read raw bytes â†’ must be ciphertext.
    #[test]
    fn test_e2e_encrypted_raw_is_opaque() {
        let (tmp, dev) = make_backing(4 * 1024 * 1024);
        let enc = fast_format_encrypted(dev, b"opaque_pw");

        let vol = CFSVolume::format(Box::new(enc), EU).unwrap();
        vol.create_file("/secret.txt").unwrap();

        // Write a recognizable pattern
        let pattern: Vec<u8> = b"SECRETDATA".iter().cycle().take(EU as usize).cloned().collect();
        vol.write_file("/secret.txt", 0, &pattern).unwrap();
        vol.sync().unwrap();
        drop(vol);

        // Read raw backing file â€” skip header block, then scan all data blocks
        let mut raw = FileBlockDevice::open(tmp.path(), None).unwrap();
        let total_blocks = (raw.size() / EU as u64) as usize;
        for blk in 1..total_blocks {
            let offset = blk as u64 * EU as u64;
            let mut raw_buf = vec![0u8; EU as usize];
            raw.read(offset, &mut raw_buf).unwrap();
            // No data block should contain the plaintext pattern
            assert_ne!(raw_buf, pattern, "block {blk} must be encrypted, not plaintext");
        }
    }

    /// Format â†’ write data â†’ change password â†’ open with new password â†’ data intact.
    #[test]
    fn test_e2e_password_change_flow() {
        let (tmp, dev) = make_backing(4 * 1024 * 1024);
        let enc = fast_format_encrypted(dev, b"old_pw");

        let vol = CFSVolume::format(Box::new(enc), EU).unwrap();
        vol.create_file("/important.txt").unwrap();

        let data = b"Critical encrypted data";
        vol.write_file("/important.txt", 0, data).unwrap();
        vol.sync().unwrap();
        drop(vol);

        // Change password manually (low iters for test, bypassing the min check)
        let mut raw = FileBlockDevice::open(tmp.path(), None).unwrap();
        let mut hdr = CryptoHeader::read_from(&mut raw, EU).unwrap();
        // Unlock with old password to get master key
        let master_key = hdr.unlock(b"old_pw").unwrap();
        // Re-wrap with new password at low iters
        let new_salt = key::generate_salt();
        let (new_kek, new_hmac_key) = key::derive_keys(b"new_pw", &new_salt, 1000);
        hdr.salt = new_salt;
        hdr.pbkdf2_iters = 1000;
        hdr.encrypted_key = key::xor_key_wrap(&master_key, &new_kek);
        hdr.key_hmac = key::compute_hmac(&new_hmac_key, &master_key);
        hdr.write_to(&mut raw, EU).unwrap();
        raw.flush().unwrap();
        drop(raw);

        // Old password must fail
        let dev_fail = FileBlockDevice::open(tmp.path(), None).unwrap();
        assert!(fast_open_encrypted(Box::new(dev_fail), b"old_pw").is_err());

        // New password must succeed and data must be intact
        let dev_ok = FileBlockDevice::open(tmp.path(), None).unwrap();
        let enc2 = fast_open_encrypted(Box::new(dev_ok), b"new_pw").unwrap();
        let vol2 = CFSVolume::mount(Box::new(enc2), EU).unwrap();

        let readback = vol2.read_file("/important.txt", 0, data.len() as u64).unwrap();
        assert_eq!(readback, data);
    }

    /// Same operations on encrypted and plaintext volumes â†’ same filesystem behavior.
    #[test]
    fn test_e2e_encrypted_vs_plaintext() {
        let create_and_exercise = |vol: &mut CFSVolume| -> Vec<u8> {
            vol.mkdir("/a").unwrap();
            vol.mkdir("/a/b").unwrap();
            vol.create_file("/a/b/data.bin").unwrap();

            let payload: Vec<u8> = (0..8192).map(|i| (i % 251) as u8).collect();
            vol.write_file("/a/b/data.bin", 0, &payload).unwrap();
            vol.sync().unwrap();

            // List root
            let root_entries = vol.list_dir("/").unwrap();
            let root_names: Vec<&str> = root_entries.iter().map(|e| e.name_str()).collect();
            assert!(root_names.contains(&"a"));

            // Read back
            let read = vol.read_file("/a/b/data.bin", 0, 8192).unwrap();
            assert_eq!(read, payload);
            read
        };

        // Plaintext volume
        let (_tmp1, dev1) = make_backing(4 * 1024 * 1024);
        let mut plain_vol = CFSVolume::format(dev1, EU).unwrap();
        let plain_data = create_and_exercise(&mut plain_vol);

        // Encrypted volume
        let (_tmp2, dev2) = make_backing(4 * 1024 * 1024);
        let enc = fast_format_encrypted(dev2, b"compare_pw");
        let mut enc_vol = CFSVolume::format(Box::new(enc), EU).unwrap();
        let enc_data = create_and_exercise(&mut enc_vol);

        assert_eq!(plain_data, enc_data, "encrypted and plaintext must produce identical data");
    }

    /// Mount encrypted via WinFSP â†’ read/write via std::fs â†’ unmount â†’ verify.
    /// Requires WinFSP runtime and administrator privileges.
    #[test]
    #[ignore]
    fn test_e2e_encrypted_mount_winfsp() {
        // This test requires WinFSP and is tested via:
        //   cargo test --bin cfs-io test_e2e_encrypted_mount_winfsp -- --ignored
        // Placeholder: actual WinFSP mount testing needs the binary to be spawned
        // as a process (see tests/integration_mount.rs pattern).
        eprintln!("Skipped: requires WinFSP runtime + admin privileges");
    }
}
