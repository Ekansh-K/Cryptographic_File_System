use aes::Aes256;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::NoPadding};
use anyhow::{bail, Result};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use crate::block_device::CFSBlockDevice;
use super::key::{generate_master_key, KdfAlgorithm, KdfParams};
use super::slot::{CFS_FEATURE_DATA_AEAD, KeySlot, KEY_SLOT_ACTIVE, KEY_SLOT_SIZE, MAX_KEY_SLOTS};

type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const CRYPTO_MAGIC: [u8; 4] = *b"CFSE";
pub const CRYPTO_VERSION_V3: u32 = 3;
pub const DEFAULT_HEADER_BLOCKS: u32 = 1;

const HEADER_SALT_LEN: usize = 60;
const PAYLOAD_LEN: usize = 4032; // 4096 - 64 salt bytes; must be multiple of 16

// KDF used only for the AES-CBC header-envelope key (fast, 10k iters)
const HEADER_KDF_ITERS: u32 = 10_000;

// Payload field offsets (inside the decrypted 4032-byte block)
const OFF_MAGIC: usize = 0;           // 4 bytes
const OFF_VERSION: usize = 4;         // 4 bytes
const OFF_HEADER_BLOCKS: usize = 8;   // 4 bytes
const OFF_EU: usize = 12;             // 4 bytes
const OFF_KDF_ALGO: usize = 16;       // 4 bytes
const OFF_NUM_SLOTS: usize = 20;      // 4 bytes
const OFF_DATA_SALT: usize = 24;      // 64 bytes -> ends at 88
const OFF_SLOTS: usize = 88;          // 4*144=576 bytes -> ends at 664
const OFF_FEATURE_FLAGS: usize = 664; // 4 bytes
const OFF_TAG_START: usize = 668;     // 8 bytes
const OFF_TAG_BLOCKS: usize = 676;    // 8 bytes
const OFF_CRC: usize = 684;           // 4 bytes; CRC32 of [0..684)
const PAYLOAD_MEANINGFUL: usize = 688;

// ---------------------------------------------------------------------------
// CryptoHeader v3
// ---------------------------------------------------------------------------

/// On-disk crypto header (v3 — fully encrypted, no plaintext magic).
///
/// Block 0 layout (4096 bytes):
/// ```text
/// [0..64]     header_salt  — random, plaintext (needed to derive AES-CBC key)
/// [64..4096]  AES-256-CBC encrypted payload (4032 bytes, NoPadding):
///   [0..4]    magic "CFSE" (confirms correct password after decryption)
///   [4..8]    version: 3
///   [8..12]   header_blocks: 1
///   [12..16]  encryption_unit: u32
///   [16..20]  kdf_algorithm: u32
///   [20..24]  num_active_slots: u32
///   [24..88]  data_salt: [u8; 64]
///   [88..664] KeySlot[0..3] (4 × 144 bytes)
///   [664..668] feature_flags: u32
///   [668..676] tag_region_start: u64
///   [676..684] tag_region_blocks: u64
///   [684..688] payload_crc: CRC32 of [0..684)
///   [688..4032] zero padding
/// ```
#[derive(Clone)]
pub struct CryptoHeader {
    pub version: u32,
    pub header_blocks: u32,
    pub encryption_unit: u32,
    pub kdf_algorithm: KdfAlgorithm,
    pub data_salt: [u8; 64],
    pub slots: [KeySlot; MAX_KEY_SLOTS],
    pub feature_flags: u32,
    pub tag_region_start: u64,
    pub tag_region_blocks: u64,
    /// Stored at buf[0..64] — needed when re-serialising the header.
    pub header_salt: [u8; HEADER_SALT_LEN],
}

impl Drop for CryptoHeader {
    fn drop(&mut self) {
        self.data_salt.zeroize();
        self.header_salt.zeroize();
    }
}

impl CryptoHeader {
    // -----------------------------------------------------------------------
    // Key derivation helpers
    // -----------------------------------------------------------------------

    /// Derive the 32-byte AES-256-CBC key + 16-byte IV for the header envelope.
    /// Fast PBKDF2-SHA256 (10 000 iters only — the slot KDF does the heavy lifting).
    fn derive_header_key(password: &[u8], header_salt: &[u8; HEADER_SALT_LEN]) -> ([u8; 32], [u8; 16]) {
        let mut derived = [0u8; 48];
        pbkdf2_hmac::<Sha256>(password, &header_salt[..32], HEADER_KDF_ITERS, &mut derived);
        let mut key = [0u8; 32];
        let mut iv = [0u8; 16];
        key.copy_from_slice(&derived[..32]);
        iv.copy_from_slice(&derived[32..48]);
        derived.zeroize();
        (key, iv)
    }

    /// Derive the 32-byte AEAD tag key from the master key via HMAC.
    pub fn derive_tag_key(master_key: &[u8; 64]) -> [u8; 32] {
        use hmac::{Hmac, Mac};
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(master_key)
            .expect("HMAC key always valid");
        mac.update(b"cfs-aead-tag-key-v3");
        let result = mac.finalize().into_bytes();
        let mut tag_key = [0u8; 32];
        tag_key.copy_from_slice(&result[..32]);
        tag_key
    }

    // -----------------------------------------------------------------------
    // Create
    // -----------------------------------------------------------------------

    /// Create a new v3 header with slot 0 active. Returns (header, master_key).
    ///
    /// `enable_aead`: if true, sets `CFS_FEATURE_DATA_AEAD` in feature_flags.
    /// `tag_region_start` / `tag_region_blocks`: AEAD tag region layout (0 if disabled).
    pub fn create(
        password: &[u8],
        kdf_params: &KdfParams,
        encryption_unit: u32,
        enable_aead: bool,
        tag_region_start: u64,
        tag_region_blocks: u64,
    ) -> Result<(Self, [u8; 64])> {
        kdf_params.validate()?;
        if encryption_unit < 512 || !encryption_unit.is_power_of_two() {
            bail!("encryption_unit must be power of 2 and >= 512, got {encryption_unit}");
        }
        use rand::RngCore;
        let mut header_salt = [0u8; HEADER_SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut header_salt);
        let mut data_salt = [0u8; 64];
        rand::rngs::OsRng.fill_bytes(&mut data_salt);

        let master_key = generate_master_key();
        let slot0 = KeySlot::create(&master_key, password, kdf_params)?;
        let slots = [slot0, KeySlot::empty(), KeySlot::empty(), KeySlot::empty()];
        let feature_flags = if enable_aead { CFS_FEATURE_DATA_AEAD } else { 0 };

        let hdr = Self {
            version: CRYPTO_VERSION_V3,
            header_blocks: DEFAULT_HEADER_BLOCKS,
            encryption_unit,
            kdf_algorithm: kdf_params.algorithm,
            data_salt,
            slots,
            feature_flags,
            tag_region_start,
            tag_region_blocks,
            header_salt,
        };
        Ok((hdr, master_key))
    }

    /// Create without KDF validation — for tests with low iteration counts.
    #[cfg(test)]
    pub fn create_for_testing(
        password: &[u8],
        kdf_params: KdfParams,
        encryption_unit: u32,
    ) -> Result<(Self, [u8; 64])> {
        use rand::RngCore;
        if encryption_unit < 512 || !encryption_unit.is_power_of_two() {
            bail!("bad eu");
        }
        let mut header_salt = [0u8; HEADER_SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut header_salt);
        let mut data_salt = [0u8; 64];
        rand::rngs::OsRng.fill_bytes(&mut data_salt);
        let master_key = generate_master_key();
        let slot0 = KeySlot::create(&master_key, password, &kdf_params)?;
        let slots = [slot0, KeySlot::empty(), KeySlot::empty(), KeySlot::empty()];
        let hdr = Self { version: CRYPTO_VERSION_V3, header_blocks: DEFAULT_HEADER_BLOCKS, encryption_unit, kdf_algorithm: kdf_params.algorithm, data_salt, slots, feature_flags: 0, tag_region_start: 0, tag_region_blocks: 0, header_salt };
        Ok((hdr, master_key))
    }

    // -----------------------------------------------------------------------
    // Serialization
    // -----------------------------------------------------------------------

    /// Serialize to a 4096-byte block, AES-256-CBC encrypting the payload.
    pub fn serialize(&self, password: &[u8]) -> Result<Vec<u8>> {
        let mut payload = [0u8; PAYLOAD_LEN];
        payload[OFF_MAGIC..OFF_MAGIC + 4].copy_from_slice(&CRYPTO_MAGIC);
        payload[OFF_VERSION..OFF_VERSION + 4].copy_from_slice(&CRYPTO_VERSION_V3.to_le_bytes());
        payload[OFF_HEADER_BLOCKS..OFF_HEADER_BLOCKS + 4].copy_from_slice(&self.header_blocks.to_le_bytes());
        payload[OFF_EU..OFF_EU + 4].copy_from_slice(&self.encryption_unit.to_le_bytes());
        payload[OFF_KDF_ALGO..OFF_KDF_ALGO + 4].copy_from_slice(&(self.kdf_algorithm as u32).to_le_bytes());
        let num_active = self.slots.iter().filter(|s| s.state == KEY_SLOT_ACTIVE).count() as u32;
        payload[OFF_NUM_SLOTS..OFF_NUM_SLOTS + 4].copy_from_slice(&num_active.to_le_bytes());
        payload[OFF_DATA_SALT..OFF_DATA_SALT + 64].copy_from_slice(&self.data_salt);
        for (i, slot) in self.slots.iter().enumerate() {
            let s = OFF_SLOTS + i * KEY_SLOT_SIZE;
            payload[s..s + KEY_SLOT_SIZE].copy_from_slice(&slot.serialize());
        }
        payload[OFF_FEATURE_FLAGS..OFF_FEATURE_FLAGS + 4].copy_from_slice(&self.feature_flags.to_le_bytes());
        payload[OFF_TAG_START..OFF_TAG_START + 8].copy_from_slice(&self.tag_region_start.to_le_bytes());
        payload[OFF_TAG_BLOCKS..OFF_TAG_BLOCKS + 8].copy_from_slice(&self.tag_region_blocks.to_le_bytes());
        let crc = crc32fast::hash(&payload[..OFF_CRC]);
        payload[OFF_CRC..OFF_CRC + 4].copy_from_slice(&crc.to_le_bytes());

        let (mut hkey, hiv) = Self::derive_header_key(password, &self.header_salt);
        let mut enc = payload.to_vec();
        Aes256CbcEnc::new_from_slices(&hkey, &hiv)
            .map_err(|e| anyhow::anyhow!("CBC init: {e}"))?
            .encrypt_padded_mut::<NoPadding>(&mut enc, PAYLOAD_LEN)
            .map_err(|e| anyhow::anyhow!("CBC encrypt: {e}"))?;
        hkey.zeroize();

        let mut block = vec![0u8; 4096];
        block[..HEADER_SALT_LEN].copy_from_slice(&self.header_salt);
        block[HEADER_SALT_LEN..HEADER_SALT_LEN + PAYLOAD_LEN].copy_from_slice(&enc);
        Ok(block)
    }

    /// Deserialize a v3 header. Returns Err if wrong password or not v3.
    pub fn deserialize(buf: &[u8], password: &[u8]) -> Result<Self> {
        if buf.len() < 4096 {
            bail!("buffer too small for v3 header ({} < 4096)", buf.len());
        }
        let mut header_salt = [0u8; HEADER_SALT_LEN];
        header_salt.copy_from_slice(&buf[..HEADER_SALT_LEN]);

        let (mut hkey, hiv) = Self::derive_header_key(password, &header_salt);
        let mut plain = buf[HEADER_SALT_LEN..HEADER_SALT_LEN + PAYLOAD_LEN].to_vec();
        Aes256CbcDec::new_from_slices(&hkey, &hiv)
            .map_err(|e| anyhow::anyhow!("CBC init: {e}"))?
            .decrypt_padded_mut::<NoPadding>(&mut plain)
            .map_err(|_| anyhow::anyhow!("wrong password or not a v3 CFS volume"))?;
        hkey.zeroize();

        if plain.len() < PAYLOAD_MEANINGFUL {
            bail!("decrypted payload too short");
        }
        if &plain[OFF_MAGIC..OFF_MAGIC + 4] != &CRYPTO_MAGIC {
            bail!("wrong password or not a v3 CFS volume");
        }
        let version = u32::from_le_bytes(plain[OFF_VERSION..OFF_VERSION + 4].try_into().unwrap());
        if version != CRYPTO_VERSION_V3 {
            bail!("unsupported crypto version: {version}");
        }
        let stored_crc = u32::from_le_bytes(plain[OFF_CRC..OFF_CRC + 4].try_into().unwrap());
        let computed_crc = crc32fast::hash(&plain[..OFF_CRC]);
        if stored_crc.to_le_bytes().ct_eq(&computed_crc.to_le_bytes()).unwrap_u8() == 0 {
            bail!("v3 header CRC mismatch — possible corruption");
        }

        let header_blocks = u32::from_le_bytes(plain[OFF_HEADER_BLOCKS..OFF_HEADER_BLOCKS + 4].try_into().unwrap());
        let encryption_unit = u32::from_le_bytes(plain[OFF_EU..OFF_EU + 4].try_into().unwrap());
        if encryption_unit < 512 || !encryption_unit.is_power_of_two() {
            bail!("invalid encryption_unit: {encryption_unit}");
        }
        let kdf_algo_u32 = u32::from_le_bytes(plain[OFF_KDF_ALGO..OFF_KDF_ALGO + 4].try_into().unwrap());
        let kdf_algorithm = KdfAlgorithm::from_u8(kdf_algo_u32 as u8)?;
        let mut data_salt = [0u8; 64];
        data_salt.copy_from_slice(&plain[OFF_DATA_SALT..OFF_DATA_SALT + 64]);

        let mut slots_arr = [KeySlot::empty(), KeySlot::empty(), KeySlot::empty(), KeySlot::empty()];
        for i in 0..MAX_KEY_SLOTS {
            let s = OFF_SLOTS + i * KEY_SLOT_SIZE;
            let mut sb = [0u8; KEY_SLOT_SIZE];
            sb.copy_from_slice(&plain[s..s + KEY_SLOT_SIZE]);
            slots_arr[i] = KeySlot::deserialize(&sb)?;
        }

        let feature_flags = u32::from_le_bytes(plain[OFF_FEATURE_FLAGS..OFF_FEATURE_FLAGS + 4].try_into().unwrap());
        let tag_region_start = u64::from_le_bytes(plain[OFF_TAG_START..OFF_TAG_START + 8].try_into().unwrap());
        let tag_region_blocks = u64::from_le_bytes(plain[OFF_TAG_BLOCKS..OFF_TAG_BLOCKS + 8].try_into().unwrap());

        Ok(Self { version, header_blocks, encryption_unit, kdf_algorithm, data_salt, slots: slots_arr, feature_flags, tag_region_start, tag_region_blocks, header_salt })
    }

    // -----------------------------------------------------------------------
    // Unlock
    // -----------------------------------------------------------------------

    /// Try all active slots with `password`, return master key on first match.
    pub fn unlock(&self, password: &[u8]) -> Result<[u8; 64]> {
        for slot in &self.slots {
            match slot.try_unlock(password) {
                Ok(Some(mk)) => return Ok(mk),
                Ok(None) => continue,
                Err(_) => continue, // wrong pw for this slot, try next
            }
        }
        bail!("wrong password — no active slot could be unlocked")
    }

    // -----------------------------------------------------------------------
    // Slot management
    // -----------------------------------------------------------------------

    /// Add a new slot. Requires master key (obtained by first unlocking with an existing slot).
    pub fn add_slot(&mut self, master_key: &[u8; 64], new_password: &[u8], kdf_params: &KdfParams) -> Result<usize> {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if !slot.is_active() {
                *slot = KeySlot::create(master_key, new_password, kdf_params)?;
                return Ok(i);
            }
        }
        bail!("all {MAX_KEY_SLOTS} key slots are already active")
    }

    /// Revoke slot at `slot_idx`. `auth_password` must unlock any active slot.
    /// Refuses if it would remove the last active slot.
    pub fn remove_slot(&mut self, slot_idx: usize, auth_password: &[u8]) -> Result<()> {
        if slot_idx >= MAX_KEY_SLOTS { bail!("slot index {slot_idx} out of range"); }
        let mut mk = self.unlock(auth_password)?; // authenticate
        mk.zeroize();
        let active_count = self.slots.iter().filter(|s| s.is_active()).count();
        if active_count <= 1 { bail!("cannot revoke the last active key slot"); }
        self.slots[slot_idx].revoke();
        Ok(())
    }

    pub fn aead_enabled(&self) -> bool { self.feature_flags & CFS_FEATURE_DATA_AEAD != 0 }

    // -----------------------------------------------------------------------
    // Block I/O helpers
    // -----------------------------------------------------------------------

    pub fn write_to(&self, dev: &mut dyn CFSBlockDevice, password: &[u8], _block_size: u32) -> Result<()> {
        let buf = self.serialize(password)?;
        dev.write(0, &buf)?;
        Ok(())
    }

    pub fn read_from(dev: &mut dyn CFSBlockDevice, password: &[u8], block_size: u32) -> Result<Self> {
        let sz = (block_size as usize).max(4096);
        let mut buf = vec![0u8; sz];
        dev.read(0, &mut buf)?;
        Self::deserialize(&buf, password)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    const EU: u32 = 4096;

    fn fast_kdf() -> KdfParams {
        KdfParams { algorithm: KdfAlgorithm::Pbkdf2HmacSha256, pbkdf2_iterations: 100_000, argon2_memory_kib: 0, argon2_time_cost: 0, argon2_parallelism: 0 }
    }

    #[test]
    fn test_v3_roundtrip() {
        let (hdr, mk) = CryptoHeader::create_for_testing(b"pw", fast_kdf(), EU).unwrap();
        let block = hdr.serialize(b"pw").unwrap();
        let hdr2 = CryptoHeader::deserialize(&block, b"pw").unwrap();
        assert_eq!(hdr2.unlock(b"pw").unwrap(), mk);
    }

    #[test]
    fn test_v3_no_plaintext_magic() {
        let (hdr, _) = CryptoHeader::create_for_testing(b"secret", fast_kdf(), EU).unwrap();
        let block = hdr.serialize(b"secret").unwrap();
        // Bytes 0..4 (salt) must not be "CFSE"
        assert_ne!(&block[0..4], b"CFSE");
        // Bytes 64..68 (start of encrypted payload) must not be "CFSE"
        assert_ne!(&block[64..68], b"CFSE");
    }

    #[test]
    fn test_v3_wrong_password_fails() {
        let (hdr, _) = CryptoHeader::create_for_testing(b"correct", fast_kdf(), EU).unwrap();
        let block = hdr.serialize(b"correct").unwrap();
        assert!(CryptoHeader::deserialize(&block, b"wrong").is_err());
    }

    #[test]
    fn test_v3_multi_slot_unlock() {
        let (mut hdr, mk) = CryptoHeader::create_for_testing(b"slot0", fast_kdf(), EU).unwrap();
        hdr.add_slot(&mk, b"slot1", &fast_kdf()).unwrap();
        let block = hdr.serialize(b"slot0").unwrap();
        let hdr2 = CryptoHeader::deserialize(&block, b"slot0").unwrap();
        assert_eq!(hdr2.unlock(b"slot0").unwrap(), mk);
        assert_eq!(hdr2.unlock(b"slot1").unwrap(), mk);
    }

    #[test]
    fn test_v3_slot_revocation() {
        let (mut hdr, mk) = CryptoHeader::create_for_testing(b"s0", fast_kdf(), EU).unwrap();
        hdr.add_slot(&mk, b"s1", &fast_kdf()).unwrap();
        hdr.remove_slot(0, b"s0").unwrap();
        assert!(hdr.unlock(b"s0").is_err());
        assert_eq!(hdr.unlock(b"s1").unwrap(), mk);
    }

    #[test]
    fn test_v3_cannot_revoke_last_slot() {
        let (mut hdr, _) = CryptoHeader::create_for_testing(b"only", fast_kdf(), EU).unwrap();
        assert!(hdr.remove_slot(0, b"only").is_err());
    }

    #[test]
    fn test_v3_aead_flag_persisted() {
        let kdf = fast_kdf();
        let (hdr, _) = CryptoHeader::create(b"pw", &kdf, EU, true, 1024, 500).unwrap();
        assert!(hdr.aead_enabled());
        let block = hdr.serialize(b"pw").unwrap();
        let hdr2 = CryptoHeader::deserialize(&block, b"pw").unwrap();
        assert!(hdr2.aead_enabled());
        assert_eq!(hdr2.tag_region_start, 1024);
        assert_eq!(hdr2.tag_region_blocks, 500);
    }

    #[test]
    fn test_derive_tag_key_stable() {
        let mk = [0xABu8; 64];
        let tk1 = CryptoHeader::derive_tag_key(&mk);
        let tk2 = CryptoHeader::derive_tag_key(&mk);
        assert_eq!(tk1, tk2);
        // Different master keys -> different tag keys
        let mk2 = [0xCDu8; 64];
        let tk3 = CryptoHeader::derive_tag_key(&mk2);
        assert_ne!(tk1, tk3);
    }
}
