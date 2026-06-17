#![allow(clippy::collapsible_if)]
pub mod key;
pub mod xts;
pub mod header;
pub mod secure_mem;
pub mod slot;
pub mod aead;

pub use header::CryptoHeader;
pub use key::{KdfAlgorithm, KdfParams, benchmark_kdf};
pub use secure_mem::LockedBuffer;
pub use xts::aes_ni_available;
pub use slot::{KeySlot, KeySlotInfo, MAX_KEY_SLOTS, CFS_FEATURE_DATA_AEAD, KEY_SLOT_SIZE};

use anyhow::Result;
use zeroize::Zeroize;

use crate::block_device::CFSBlockDevice;
use xts::XtsCipher;

// ---------------------------------------------------------------------------
// VolumeProbe
// ---------------------------------------------------------------------------

/// Result of probing a device for v3 CFS format.
/// In v3 there are no plaintext magic bytes, so any device >= 4096 bytes
/// is treated as a potential encrypted volume; password confirms it.
pub enum VolumeProbe {
    V3Encrypted,
    NotCfs,
}

pub fn probe_volume(dev: &mut dyn CFSBlockDevice) -> Result<VolumeProbe> {
    if dev.size() < 4096 {
        return Ok(VolumeProbe::NotCfs);
    }
    let mut probe = vec![0u8; dev.sector_size() as usize];
    dev.read(0, &mut probe)?;
    
    if &probe[0..4] == b"CFS1" {
        return Ok(VolumeProbe::NotCfs); // Plaintext v1 or v3 volume
    }
    Ok(VolumeProbe::V3Encrypted)
}

/// Legacy helper kept for callers that still do a bool check.
/// In v3 we can't peek at magic — always returns true for non-empty devices.
pub fn is_encrypted_device(dev: &mut dyn CFSBlockDevice) -> Result<bool> {
    match probe_volume(dev)? {
        VolumeProbe::V3Encrypted => Ok(true),
        VolumeProbe::NotCfs => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// EncryptedBlockDevice
// ---------------------------------------------------------------------------

pub struct EncryptedBlockDevice {
    inner: Box<dyn CFSBlockDevice>,
    cipher: XtsCipher,
    header_blocks: u64,
    encryption_unit: u32,
    aead_enabled: bool,
    tag_key: Option<[u8; 32]>,
    /// Byte offset in the INNER device where the tag region begins (0 if disabled).
    tag_region_start: u64,
    /// Usable data size exposed to callers (pre-computed at construction).
    usable_size: u64,
    /// Password cache for re-serialising the header (slot management).
    header_password: Vec<u8>,
}

impl Drop for EncryptedBlockDevice {
    fn drop(&mut self) {
        self.header_password.zeroize();
        if let Some(ref mut tk) = self.tag_key { tk.zeroize(); }
    }
}

impl EncryptedBlockDevice {
    pub fn format_encrypted(
        mut inner: Box<dyn CFSBlockDevice>,
        password: &[u8],
        kdf_params: &KdfParams,
        encryption_unit: u32,
        enable_aead: bool,
    ) -> Result<Self> {
        let total_size = inner.size();
        let header_size = encryption_unit as u64;
        let data_size = total_size.saturating_sub(header_size);
        let n_data_blocks = data_size / encryption_unit as u64;

        let (tag_region_start, tag_region_blocks) = if enable_aead {
            let tag_bytes = n_data_blocks * 16;
            let start = total_size - tag_bytes;
            (start, n_data_blocks)
        } else {
            (0, 0)
        };

        let (hdr, mut master_key) = CryptoHeader::create(
            password, kdf_params, encryption_unit, enable_aead, tag_region_start, tag_region_blocks,
        )?;
        hdr.write_to(&mut *inner, password, encryption_unit)?;

        let tag_key = if enable_aead {
            Some(CryptoHeader::derive_tag_key(&master_key))
        } else { None };

        let cipher = XtsCipher::new(&master_key, encryption_unit);
        master_key.zeroize();

        let usable_size = if enable_aead {
            tag_region_start.saturating_sub(header_size)
        } else {
            total_size.saturating_sub(header_size)
        };

        Ok(Self {
            inner,
            cipher,
            header_blocks: 1,
            encryption_unit,
            aead_enabled: enable_aead,
            tag_key,
            tag_region_start,
            usable_size,
            header_password: password.to_vec(),
        })
    }

    pub fn open_encrypted(mut inner: Box<dyn CFSBlockDevice>, password: &[u8]) -> Result<Self> {
        let probe_size = std::cmp::max(4096, inner.sector_size() as usize);
        let mut probe = vec![0u8; probe_size];
        inner.read(0, &mut probe)?;
        let hdr = CryptoHeader::deserialize(&probe, password)?;

        let mut master_key = hdr.unlock(password)?;
        let aead_enabled = hdr.aead_enabled();
        let tag_region_start = hdr.tag_region_start;
        let total_size = inner.size();
        let header_size = hdr.header_blocks as u64 * hdr.encryption_unit as u64;

        let tag_key = if aead_enabled {
            Some(CryptoHeader::derive_tag_key(&master_key))
        } else { None };

        let cipher = XtsCipher::new(&master_key, hdr.encryption_unit);
        master_key.zeroize();

        let usable_size = if aead_enabled {
            tag_region_start.saturating_sub(header_size)
        } else {
            total_size.saturating_sub(header_size)
        };

        Ok(Self {
            inner,
            cipher,
            header_blocks: hdr.header_blocks as u64,
            encryption_unit: hdr.encryption_unit,
            aead_enabled,
            tag_key,
            tag_region_start,
            usable_size,
            header_password: password.to_vec(),
        })
    }

    /// Read the stored 16-byte GCM tag for `data_block_idx` from the tag region.
    fn read_tag(&mut self, data_block_idx: u64) -> Result<[u8; 16]> {
        let tag_offset = self.tag_region_start + data_block_idx * 16;
        let sector_size = self.inner.sector_size() as u64;
        let sector_offset = (tag_offset / sector_size) * sector_size;
        let mut sector_buf = vec![0u8; sector_size as usize];
        self.inner.read(sector_offset, &mut sector_buf)?;
        let within = (tag_offset - sector_offset) as usize;
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&sector_buf[within..within + 16]);
        Ok(tag)
    }

    /// Write a 16-byte GCM tag for `data_block_idx` to the tag region.
    fn write_tag(&mut self, data_block_idx: u64, tag: &[u8; 16]) -> Result<()> {
        let tag_offset = self.tag_region_start + data_block_idx * 16;
        let sector_size = self.inner.sector_size() as u64;
        let sector_offset = (tag_offset / sector_size) * sector_size;
        let mut sector_buf = vec![0u8; sector_size as usize];
        self.inner.read(sector_offset, &mut sector_buf)?;
        let within = (tag_offset - sector_offset) as usize;
        sector_buf[within..within + 16].copy_from_slice(tag);
        self.inner.write(sector_offset, &sector_buf)?;
        Ok(())
    }

    /// Read the crypto header back from disk with the cached password.
    pub fn read_header(&mut self) -> Result<CryptoHeader> {
        CryptoHeader::read_from(&mut *self.inner, &self.header_password, self.encryption_unit)
    }

    /// Write an updated crypto header back to disk.
    pub fn write_header(&mut self, hdr: &CryptoHeader) -> Result<()> {
        hdr.write_to(&mut *self.inner, &self.header_password, self.encryption_unit)
    }
}

impl CFSBlockDevice for EncryptedBlockDevice {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let eu = self.encryption_unit as usize;
        if offset % eu as u64 != 0 {
            anyhow::bail!("EncryptedBlockDevice::read offset ({offset}) not aligned to eu ({eu})");
        }
        if buf.len() % eu != 0 {
            anyhow::bail!("EncryptedBlockDevice::read length ({}) not aligned to eu ({eu})", buf.len());
        }
        let header_offset = self.header_blocks * self.encryption_unit as u64;
        let inner_offset = offset + header_offset;
        let n = self.inner.read(inner_offset, buf)?;

        let first_block = offset / self.encryption_unit as u64;
        if self.aead_enabled {
            if let Some(tag_key) = self.tag_key {
                let num_blocks = n / eu;
                for i in 0..num_blocks {
                    let blk_idx = first_block + i as u64;
                    let chunk = &buf[i * eu..(i + 1) * eu];
                    let stored_tag = self.read_tag(blk_idx)?;
                    aead::verify_block_tag(&tag_key, blk_idx, chunk, &stored_tag)?;
                }
            }
        }

        self.cipher.decrypt_blocks_parallel(first_block, &mut buf[..n]);
        Ok(n)
    }

    fn write(&mut self, offset: u64, buf: &[u8]) -> Result<usize> {
        let eu = self.encryption_unit as usize;
        if offset % eu as u64 != 0 {
            anyhow::bail!("EncryptedBlockDevice::write offset ({offset}) not aligned to eu ({eu})");
        }
        if buf.len() % eu != 0 {
            anyhow::bail!("EncryptedBlockDevice::write length ({}) not aligned to eu ({eu})", buf.len());
        }
        let header_offset = self.header_blocks * self.encryption_unit as u64;
        let inner_offset = offset + header_offset;
        let mut encrypted = buf.to_vec();
        let first_block = offset / self.encryption_unit as u64;
        self.cipher.encrypt_blocks_parallel(first_block, &mut encrypted);

        if self.aead_enabled {
            if let Some(tag_key) = self.tag_key {
                let num_blocks = encrypted.len() / eu;
                for i in 0..num_blocks {
                    let blk_idx = first_block + i as u64;
                    let chunk = &encrypted[i * eu..(i + 1) * eu];
                    let tag = aead::compute_block_tag(&tag_key, blk_idx, chunk);
                    self.write_tag(blk_idx, &tag)?;
                }
            }
        }
        self.inner.write(inner_offset, &encrypted)
    }

    fn size(&self) -> u64 { self.usable_size }

    fn sector_size(&self) -> u32 { self.inner.sector_size() }

    fn flush(&mut self) -> Result<()> { self.inner.flush() }
}

// ---------------------------------------------------------------------------
// Password / slot management helpers
// ---------------------------------------------------------------------------

/// Change password for a slot on an encrypted device.
pub fn change_password(
    dev: &mut dyn CFSBlockDevice,
    old_password: &[u8],
    new_password: &[u8],
    new_kdf: Option<KdfParams>,
    block_size: u32,
) -> Result<()> {
    let mut hdr = CryptoHeader::read_from(dev, old_password, block_size)?;
    // Find the slot that old_password unlocks and re-wrap it
    let mut master_key = hdr.unlock(old_password)?;
    // Find the first active slot that old_password unlocks
    for i in 0..MAX_KEY_SLOTS {
        if let Ok(Some(mut mk)) = hdr.slots[i].try_unlock(old_password) {
            mk.zeroize();
            let kdf = new_kdf.clone().unwrap_or_else(|| hdr.slots[i].kdf_params.clone());
            hdr.slots[i] = slot::KeySlot::create(&master_key, new_password, &kdf)?;
            break;
        }
    }
    hdr.write_to(dev, new_password, block_size)?;
    dev.flush()?;
    master_key.zeroize();
    Ok(())
}

/// Add a new key slot on a device. Returns the new slot index.
pub fn add_key_slot(
    dev: &mut dyn CFSBlockDevice,
    auth_password: &[u8],
    new_password: &[u8],
    kdf: KdfParams,
    block_size: u32,
) -> Result<usize> {
    let mut hdr = CryptoHeader::read_from(dev, auth_password, block_size)?;
    let mut master_key = hdr.unlock(auth_password)?;
    let idx = hdr.add_slot(&master_key, new_password, &kdf)?;
    hdr.write_to(dev, auth_password, block_size)?;
    dev.flush()?;
    master_key.zeroize();
    Ok(idx)
}

/// Remove a key slot on a device.
pub fn remove_key_slot(
    dev: &mut dyn CFSBlockDevice,
    auth_password: &[u8],
    slot_idx: usize,
    block_size: u32,
) -> Result<()> {
    let mut hdr = CryptoHeader::read_from(dev, auth_password, block_size)?;
    hdr.remove_slot(slot_idx, auth_password)?;
    hdr.write_to(dev, auth_password, block_size)?;
    dev.flush()?;
    Ok(())
}

/// List all key slots on a device (no password needed — reads header with any active password).
pub fn list_key_slots(
    dev: &mut dyn CFSBlockDevice,
    auth_password: &[u8],
    block_size: u32,
) -> Result<Vec<KeySlotInfo>> {
    let hdr = CryptoHeader::read_from(dev, auth_password, block_size)?;
    Ok(hdr.slots.iter().enumerate().map(|(i, s)| KeySlotInfo::from((i, s))).collect())
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

    fn make_backing(size: u64) -> (NamedTempFile, Box<dyn CFSBlockDevice>) {
        let tmp = NamedTempFile::new().unwrap();
        let dev = FileBlockDevice::open(tmp.path(), Some(size)).unwrap();
        (tmp, Box::new(dev))
    }

    fn fast_kdf() -> KdfParams {
        KdfParams { algorithm: KdfAlgorithm::Pbkdf2HmacSha256, pbkdf2_iterations: 100_000, argon2_memory_kib: 0, argon2_time_cost: 0, argon2_parallelism: 0 }
    }

    fn fast_format(inner: Box<dyn CFSBlockDevice>, password: &[u8]) -> EncryptedBlockDevice {
        EncryptedBlockDevice::format_encrypted(inner, password, &fast_kdf(), EU, false).unwrap()
    }

    fn fast_open(inner: Box<dyn CFSBlockDevice>, password: &[u8]) -> Result<EncryptedBlockDevice> {
        EncryptedBlockDevice::open_encrypted(inner, password)
    }

    #[test]
    fn test_encrypted_read_write_roundtrip() {
        let (_tmp, dev) = make_backing(2 * 1024 * 1024);
        let mut enc = fast_format(dev, b"pw123");
        let data = vec![0xABu8; EU as usize];
        enc.write(0, &data).unwrap();
        let mut rb = vec![0u8; EU as usize];
        enc.read(0, &mut rb).unwrap();
        assert_eq!(rb, data);
    }

    #[test]
    fn test_raw_data_is_ciphertext() {
        let (tmp, dev) = make_backing(2 * 1024 * 1024);
        let mut enc = fast_format(dev, b"pw123");
        let plaintext = vec![0x42u8; EU as usize];
        enc.write(0, &plaintext).unwrap();
        enc.flush().unwrap();
        drop(enc);
        let mut raw = FileBlockDevice::open(tmp.path(), None).unwrap();
        let mut raw_buf = vec![0u8; EU as usize];
        raw.read(EU as u64, &mut raw_buf).unwrap(); // skip header block
        assert_ne!(raw_buf, plaintext, "data must be encrypted on disk");
    }

    #[test]
    fn test_wrong_password_fails() {
        let (tmp, dev) = make_backing(2 * 1024 * 1024);
        let _ = fast_format(dev, b"correct_pw");
        let dev2 = FileBlockDevice::open(tmp.path(), None).unwrap();
        assert!(fast_open(Box::new(dev2), b"wrong_pw").is_err());
    }

    #[test]
    fn test_e2e_encrypted_workflow() {
        let (tmp, dev) = make_backing(4 * 1024 * 1024);
        let enc = fast_format(dev, b"workflow_pw");
        let vol = CFSVolume::format(Box::new(enc), EU).unwrap();
        vol.mkdir("/docs").unwrap();
        vol.create_file("/docs/hello.txt").unwrap();
        let content = b"Hello, encrypted CFS world!";
        vol.write_file("/docs/hello.txt", 0, content).unwrap();
        vol.sync().unwrap();
        drop(vol);

        let dev2 = FileBlockDevice::open(tmp.path(), None).unwrap();
        let enc2 = fast_open(Box::new(dev2), b"workflow_pw").unwrap();
        let vol2 = CFSVolume::mount(Box::new(enc2), EU).unwrap();
        let entries = vol2.list_dir("/docs").unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name_str()).collect();
        assert!(names.contains(&"hello.txt"));
        let rb = vol2.read_file("/docs/hello.txt", 0, content.len() as u64).unwrap();
        assert_eq!(rb, content);
    }

    #[test]
    fn test_aead_write_read_roundtrip() {
        let (_tmp, dev) = make_backing(4 * 1024 * 1024);
        let mut enc = EncryptedBlockDevice::format_encrypted(
            dev, b"aead_pw", &fast_kdf(), EU, true,
        ).unwrap();
        assert!(enc.aead_enabled);
        let data = vec![0xCCu8; EU as usize];
        enc.write(0, &data).unwrap();
        let mut rb = vec![0u8; EU as usize];
        enc.read(0, &mut rb).unwrap();
        assert_eq!(rb, data);
    }
}
