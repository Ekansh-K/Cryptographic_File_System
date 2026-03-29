use anyhow::{bail, Result};
use zeroize::Zeroize;

use crate::block_device::CFSBlockDevice;
use super::key::{
    compute_hmac, derive_keys_with_params, generate_master_key, generate_salt,
    verify_hmac, xor_key_wrap, KdfAlgorithm, KdfParams,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const CRYPTO_MAGIC: [u8; 4] = *b"CFSE";
/// Current header version (always written).
pub const CRYPTO_VERSION: u32 = 2;
/// Legacy header version (read-only backward compat).
pub const CRYPTO_VERSION_V1: u32 = 1;
pub const DEFAULT_HEADER_BLOCKS: u32 = 1;
/// v1 meaningful bytes (CRC at offset 152).
const CRYPTO_HEADER_MEANINGFUL_V1: usize = 156;
/// v2 meaningful bytes (CRC at offset 160).
const CRYPTO_HEADER_MEANINGFUL_V2: usize = 164;

// ---------------------------------------------------------------------------
// CryptoHeader
// ---------------------------------------------------------------------------

/// On-disk crypto header — occupies block 0 (4096 bytes).
///
/// v1 layout (LE, read-only):
/// ```text
/// 0..4     magic          b"CFSE"
/// 4..8     version        1
/// 8..12    header_blocks  1
/// 12..16   encryption_unit 4096
/// 16..48   salt           [u8; 32]
/// 48..52   pbkdf2_iters   ≥300,000
/// 52..56   _reserved      0
/// 56..120  encrypted_key  [u8; 64]
/// 120..152 key_hmac       [u8; 32]
/// 152..156 header_crc     CRC32 of [0..152)
/// 156..4096 zero padding
/// ```
///
/// v2 layout (LE, current):
/// ```text
/// 0..4     magic              b"CFSE"
/// 4..8     version            2
/// 8..12    header_blocks      1
/// 12..16   encryption_unit    4096
/// 16..48   salt               [u8; 32]
/// 48..52   pbkdf2_iters       u32 (0 for Argon2id)
/// 52..53   kdf_algorithm      u8 (0=PBKDF2, 1=Argon2id)
/// 53..56   _reserved          [0; 3]
/// 56..120  encrypted_key      [u8; 64]
/// 120..152 key_hmac           [u8; 32]
/// 152..156 argon2_memory_kib  u32 (0 for PBKDF2)
/// 156..158 argon2_time_cost   u16 (0 for PBKDF2)
/// 158..160 argon2_parallelism u16 (0 for PBKDF2)
/// 160..164 header_crc         CRC32 of [0..160)
/// 164..4096 zero padding
/// ```
#[derive(Clone)]
pub struct CryptoHeader {
    pub version: u32,
    pub header_blocks: u32,
    pub encryption_unit: u32,
    pub salt: [u8; 32],
    pub pbkdf2_iters: u32,
    pub kdf_algorithm: KdfAlgorithm,
    pub argon2_memory_kib: u32,
    pub argon2_time_cost: u16,
    pub argon2_parallelism: u16,
    pub encrypted_key: [u8; 64],
    pub key_hmac: [u8; 32],
}

impl Drop for CryptoHeader {
    fn drop(&mut self) {
        self.salt.zeroize();
        self.encrypted_key.zeroize();
        self.key_hmac.zeroize();
    }
}

impl CryptoHeader {
    /// Generate a new crypto header (v2) and return `(header, master_key)`.
    ///
    /// The master key is needed by the caller to construct the XTS cipher.
    pub fn create(
        password: &[u8],
        kdf_params: &KdfParams,
        encryption_unit: u32,
    ) -> Result<(Self, [u8; 64])> {
        kdf_params.validate()?;

        if encryption_unit < 512 || !encryption_unit.is_power_of_two() {
            bail!(
                "encryption_unit must be a power of 2 and ≥ 512, got {encryption_unit}"
            );
        }

        let salt = generate_salt();
        let master_key = generate_master_key();
        let (mut kek, mut hmac_key) = derive_keys_with_params(password, &salt, kdf_params)?;
        let encrypted_key = xor_key_wrap(&master_key, &kek);
        let key_hmac = compute_hmac(&hmac_key, &master_key);
        kek.zeroize();
        hmac_key.zeroize();

        let hdr = Self {
            version: CRYPTO_VERSION,
            header_blocks: DEFAULT_HEADER_BLOCKS,
            encryption_unit,
            salt,
            pbkdf2_iters: kdf_params.pbkdf2_iterations,
            kdf_algorithm: kdf_params.algorithm,
            argon2_memory_kib: kdf_params.argon2_memory_kib,
            argon2_time_cost: kdf_params.argon2_time_cost as u16,
            argon2_parallelism: kdf_params.argon2_parallelism as u16,
            encrypted_key,
            key_hmac,
        };

        Ok((hdr, master_key))
    }

    /// Reconstruct `KdfParams` from header fields.
    pub fn kdf_params(&self) -> KdfParams {
        KdfParams {
            algorithm: self.kdf_algorithm,
            pbkdf2_iterations: self.pbkdf2_iters,
            argon2_memory_kib: self.argon2_memory_kib,
            argon2_time_cost: self.argon2_time_cost as u32,
            argon2_parallelism: self.argon2_parallelism as u32,
        }
    }

    /// Unlock the header with a password, returning the master key.
    pub fn unlock(&self, password: &[u8]) -> Result<[u8; 64]> {
        let params = self.kdf_params();
        let (mut kek, mut hmac_key) = derive_keys_with_params(password, &self.salt, &params)?;
        let mut candidate = xor_key_wrap(&self.encrypted_key, &kek);
        let result = verify_hmac(&hmac_key, &candidate, &self.key_hmac);
        kek.zeroize();
        hmac_key.zeroize();
        if result.is_err() {
            candidate.zeroize();
            return Err(result.unwrap_err());
        }
        Ok(candidate)
    }

    /// Change the password protecting the master key.
    ///
    /// The old password is verified, then the master key is re-wrapped with
    /// the new password. A fresh salt is generated. Optionally switches KDF.
    pub fn change_password(
        &mut self,
        old_password: &[u8],
        new_password: &[u8],
        new_kdf: Option<KdfParams>,
    ) -> Result<()> {
        let mut master_key = self.unlock(old_password)?;

        let params = match new_kdf {
            Some(p) => { p.validate()?; p }
            None => self.kdf_params(),
        };

        let new_salt = generate_salt();
        let (mut new_kek, mut new_hmac_key) =
            derive_keys_with_params(new_password, &new_salt, &params)?;
        self.version = CRYPTO_VERSION;
        self.salt = new_salt;
        self.kdf_algorithm = params.algorithm;
        self.pbkdf2_iters = params.pbkdf2_iterations;
        self.argon2_memory_kib = params.argon2_memory_kib;
        self.argon2_time_cost = params.argon2_time_cost as u16;
        self.argon2_parallelism = params.argon2_parallelism as u16;
        self.encrypted_key = xor_key_wrap(&master_key, &new_kek);
        self.key_hmac = compute_hmac(&new_hmac_key, &master_key);
        master_key.zeroize();
        new_kek.zeroize();
        new_hmac_key.zeroize();

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Serialization
    // -----------------------------------------------------------------------

    /// Serialize header into a full block (block_size bytes, default 4096).
    /// Always writes v2 format.
    pub fn serialize(&self, block_size: u32) -> Vec<u8> {
        let mut buf = vec![0u8; block_size as usize];

        buf[0..4].copy_from_slice(&CRYPTO_MAGIC);
        buf[4..8].copy_from_slice(&CRYPTO_VERSION.to_le_bytes());
        buf[8..12].copy_from_slice(&self.header_blocks.to_le_bytes());
        buf[12..16].copy_from_slice(&self.encryption_unit.to_le_bytes());
        buf[16..48].copy_from_slice(&self.salt);
        buf[48..52].copy_from_slice(&self.pbkdf2_iters.to_le_bytes());
        buf[52] = self.kdf_algorithm as u8;
        // 53..56 reserved (zeros)
        buf[56..120].copy_from_slice(&self.encrypted_key);
        buf[120..152].copy_from_slice(&self.key_hmac);
        buf[152..156].copy_from_slice(&self.argon2_memory_kib.to_le_bytes());
        buf[156..158].copy_from_slice(&self.argon2_time_cost.to_le_bytes());
        buf[158..160].copy_from_slice(&self.argon2_parallelism.to_le_bytes());

        let crc = crc32fast::hash(&buf[..160]);
        buf[160..164].copy_from_slice(&crc.to_le_bytes());

        buf
    }

    /// Deserialize a CryptoHeader from a block-sized buffer.
    /// Supports both v1 (read-only) and v2 formats.
    pub fn deserialize(buf: &[u8]) -> Result<Self> {
        if buf.len() < CRYPTO_HEADER_MEANINGFUL_V1 {
            bail!("buffer too small for CryptoHeader ({} < {CRYPTO_HEADER_MEANINGFUL_V1})", buf.len());
        }
        if &buf[0..4] != &CRYPTO_MAGIC {
            bail!("bad crypto magic: expected CFSE");
        }
        let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        match version {
            CRYPTO_VERSION_V1 => Self::deserialize_v1(buf),
            CRYPTO_VERSION => Self::deserialize_v2(buf),
            _ => bail!("unsupported crypto version: {version}"),
        }
    }

    fn deserialize_v1(buf: &[u8]) -> Result<Self> {
        let stored_crc = u32::from_le_bytes(buf[152..156].try_into().unwrap());
        let computed_crc = crc32fast::hash(&buf[..152]);
        if stored_crc != computed_crc {
            bail!("CryptoHeader CRC mismatch (stored={stored_crc:#010x}, computed={computed_crc:#010x})");
        }

        let header_blocks = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let encryption_unit = u32::from_le_bytes(buf[12..16].try_into().unwrap());

        if encryption_unit < 512 || !encryption_unit.is_power_of_two() {
            bail!(
                "invalid encryption_unit in header: must be power of 2 and ≥ 512, got {encryption_unit}"
            );
        }

        let mut salt = [0u8; 32];
        salt.copy_from_slice(&buf[16..48]);
        let pbkdf2_iters = u32::from_le_bytes(buf[48..52].try_into().unwrap());
        let mut encrypted_key = [0u8; 64];
        encrypted_key.copy_from_slice(&buf[56..120]);
        let mut key_hmac = [0u8; 32];
        key_hmac.copy_from_slice(&buf[120..152]);

        Ok(Self {
            version: CRYPTO_VERSION_V1,
            header_blocks,
            encryption_unit,
            salt,
            pbkdf2_iters,
            kdf_algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
            encrypted_key,
            key_hmac,
        })
    }

    fn deserialize_v2(buf: &[u8]) -> Result<Self> {
        if buf.len() < CRYPTO_HEADER_MEANINGFUL_V2 {
            bail!("buffer too small for v2 CryptoHeader ({} < {CRYPTO_HEADER_MEANINGFUL_V2})", buf.len());
        }

        let stored_crc = u32::from_le_bytes(buf[160..164].try_into().unwrap());
        let computed_crc = crc32fast::hash(&buf[..160]);
        if stored_crc != computed_crc {
            bail!("CryptoHeader CRC mismatch (stored={stored_crc:#010x}, computed={computed_crc:#010x})");
        }

        let header_blocks = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let encryption_unit = u32::from_le_bytes(buf[12..16].try_into().unwrap());

        if encryption_unit < 512 || !encryption_unit.is_power_of_two() {
            bail!(
                "invalid encryption_unit in header: must be power of 2 and ≥ 512, got {encryption_unit}"
            );
        }

        let mut salt = [0u8; 32];
        salt.copy_from_slice(&buf[16..48]);
        let pbkdf2_iters = u32::from_le_bytes(buf[48..52].try_into().unwrap());
        let kdf_algorithm = KdfAlgorithm::from_u8(buf[52])?;
        let mut encrypted_key = [0u8; 64];
        encrypted_key.copy_from_slice(&buf[56..120]);
        let mut key_hmac = [0u8; 32];
        key_hmac.copy_from_slice(&buf[120..152]);
        let argon2_memory_kib = u32::from_le_bytes(buf[152..156].try_into().unwrap());
        let argon2_time_cost = u16::from_le_bytes(buf[156..158].try_into().unwrap());
        let argon2_parallelism = u16::from_le_bytes(buf[158..160].try_into().unwrap());

        Ok(Self {
            version: CRYPTO_VERSION,
            header_blocks,
            encryption_unit,
            salt,
            pbkdf2_iters,
            kdf_algorithm,
            argon2_memory_kib,
            argon2_time_cost,
            argon2_parallelism,
            encrypted_key,
            key_hmac,
        })
    }

    /// Write header to block 0 of a device.
    pub fn write_to(&self, dev: &mut dyn CFSBlockDevice, block_size: u32) -> Result<()> {
        let buf = self.serialize(block_size);
        dev.write(0, &buf)?;
        Ok(())
    }

    /// Read header from block 0 of a device.
    pub fn read_from(dev: &mut dyn CFSBlockDevice, block_size: u32) -> Result<Self> {
        let mut buf = vec![0u8; block_size as usize];
        dev.read(0, &mut buf)?;
        Self::deserialize(&buf)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::key::{KdfAlgorithm, KdfParams};

    // Use fewer PBKDF2 iterations in tests for speed.
    // Production code enforces minimums via KdfParams::validate().
    const EU: u32 = 4096;

    /// Helper: create test header with low-iter PBKDF2 for speed.
    fn test_create(password: &[u8]) -> (CryptoHeader, [u8; 64]) {
        // Bypass the validate() minimum by constructing manually.
        let salt = super::super::key::generate_salt();
        let master_key = super::super::key::generate_master_key();
        let params = KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 1000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        };
        let (kek, hmac_key) = super::super::key::derive_keys_with_params(password, &salt, &params).unwrap();
        let encrypted_key = super::super::key::xor_key_wrap(&master_key, &kek);
        let key_hmac = super::super::key::compute_hmac(&hmac_key, &master_key);
        let hdr = CryptoHeader {
            version: CRYPTO_VERSION,
            header_blocks: DEFAULT_HEADER_BLOCKS,
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
        (hdr, master_key)
    }

    /// Helper: create test header with Argon2id (minimal params for speed).
    fn test_create_argon2id(password: &[u8]) -> (CryptoHeader, [u8; 64]) {
        let salt = super::super::key::generate_salt();
        let master_key = super::super::key::generate_master_key();
        let params = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        };
        let (kek, hmac_key) = super::super::key::derive_keys_with_params(password, &salt, &params).unwrap();
        let encrypted_key = super::super::key::xor_key_wrap(&master_key, &kek);
        let key_hmac = super::super::key::compute_hmac(&hmac_key, &master_key);
        let hdr = CryptoHeader {
            version: CRYPTO_VERSION,
            header_blocks: DEFAULT_HEADER_BLOCKS,
            encryption_unit: EU,
            salt,
            pbkdf2_iters: 0,
            kdf_algorithm: KdfAlgorithm::Argon2id,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
            encrypted_key,
            key_hmac,
        };
        (hdr, master_key)
    }

    #[test]
    fn test_header_roundtrip() {
        let password = b"roundtrip_pw";
        let (hdr, master_key) = test_create(password);

        let buf = hdr.serialize(EU);
        let hdr2 = CryptoHeader::deserialize(&buf).unwrap();

        let recovered = hdr2.unlock(password).unwrap();
        assert_eq!(recovered, master_key);
    }

    #[test]
    fn test_wrong_password_rejected() {
        let (hdr, _) = test_create(b"correct");
        let result = hdr.unlock(b"incorrect");
        assert!(result.is_err());
    }

    #[test]
    fn test_correct_password_accepted() {
        let password = b"my_secret";
        let (hdr, mk) = test_create(password);
        let recovered = hdr.unlock(password).unwrap();
        assert_eq!(recovered, mk);
    }

    #[test]
    fn test_tampered_encrypted_key() {
        let password = b"tamper_test";
        let (mut hdr, _) = test_create(password);
        hdr.encrypted_key[0] ^= 0xFF; // corrupt one byte
        let result = hdr.unlock(password);
        assert!(result.is_err(), "tampered encrypted_key should fail HMAC");
    }

    #[test]
    fn test_tampered_hmac() {
        let password = b"hmac_test";
        let (mut hdr, _) = test_create(password);
        hdr.key_hmac[0] ^= 0xFF;
        let result = hdr.unlock(password);
        assert!(result.is_err(), "tampered HMAC should fail verification");
    }

    #[test]
    fn test_password_change() {
        let old_pw = b"old_password";
        let new_pw = b"new_password";
        let (mut hdr, master_key) = test_create(old_pw);

        // Manually perform the change_password logic with low iters for speed
        let mk = hdr.unlock(old_pw).unwrap();
        assert_eq!(mk, master_key);

        let new_salt = super::super::key::generate_salt();
        let params = KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 1000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        };
        let (new_kek, new_hmac_key) =
            super::super::key::derive_keys_with_params(new_pw, &new_salt, &params).unwrap();
        hdr.salt = new_salt;
        hdr.pbkdf2_iters = 1000;
        hdr.encrypted_key = super::super::key::xor_key_wrap(&mk, &new_kek);
        hdr.key_hmac = super::super::key::compute_hmac(&new_hmac_key, &mk);

        // Old password should fail
        assert!(hdr.unlock(old_pw).is_err());
        // New password should recover the same master key
        let recovered = hdr.unlock(new_pw).unwrap();
        assert_eq!(recovered, master_key);
    }

    #[test]
    fn test_create_rejects_bad_encryption_unit() {
        let params = KdfParams::default_pbkdf2();

        // Not a power of 2
        let result = CryptoHeader::create(b"pw", &params, 3000);
        assert!(result.is_err(), "should reject non-power-of-2");

        // Too small
        let result = CryptoHeader::create(b"pw", &params, 256);
        assert!(result.is_err(), "should reject < 512");

        // Zero
        let result = CryptoHeader::create(b"pw", &params, 0);
        assert!(result.is_err(), "should reject zero");
    }

    #[test]
    fn test_deserialize_rejects_bad_encryption_unit() {
        // Create a valid header then tamper with the encryption_unit field
        let (hdr, _) = test_create(b"pw");
        let mut buf = hdr.serialize(EU);

        // Set encryption_unit to 300 (not power of 2)
        buf[12..16].copy_from_slice(&300u32.to_le_bytes());
        // Recompute CRC for the tampered buffer (v2: CRC at 160)
        let new_crc = crc32fast::hash(&buf[..160]);
        buf[160..164].copy_from_slice(&new_crc.to_le_bytes());

        let result = CryptoHeader::deserialize(&buf);
        assert!(result.is_err(), "should reject invalid encryption_unit on deserialize");
    }

    // -----------------------------------------------------------------------
    // Phase 7F — v2 / Argon2id tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_v2_roundtrip() {
        let password = b"v2_roundtrip_pw";
        let (hdr, master_key) = test_create(password);

        let buf = hdr.serialize(EU);
        assert_eq!(&buf[4..8], &2u32.to_le_bytes()); // version == 2

        let hdr2 = CryptoHeader::deserialize(&buf).unwrap();
        assert_eq!(hdr2.version, CRYPTO_VERSION);
        assert_eq!(hdr2.kdf_algorithm, KdfAlgorithm::Pbkdf2HmacSha256);

        let recovered = hdr2.unlock(password).unwrap();
        assert_eq!(recovered, master_key);
    }

    #[test]
    fn test_v1_backward_compat() {
        // Manually build a v1-format buffer and verify it deserializes.
        let password = b"v1_test";
        let salt = super::super::key::generate_salt();
        let master_key = super::super::key::generate_master_key();
        let params = KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 1000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        };
        let (kek, hmac_key) = super::super::key::derive_keys_with_params(password, &salt, &params).unwrap();
        let encrypted_key = super::super::key::xor_key_wrap(&master_key, &kek);
        let key_hmac = super::super::key::compute_hmac(&hmac_key, &master_key);

        let mut buf = vec![0u8; EU as usize];
        buf[0..4].copy_from_slice(b"CFSE");
        buf[4..8].copy_from_slice(&1u32.to_le_bytes()); // version 1
        buf[8..12].copy_from_slice(&1u32.to_le_bytes());
        buf[12..16].copy_from_slice(&EU.to_le_bytes());
        buf[16..48].copy_from_slice(&salt);
        buf[48..52].copy_from_slice(&1000u32.to_le_bytes());
        buf[56..120].copy_from_slice(&encrypted_key);
        buf[120..152].copy_from_slice(&key_hmac);
        let crc = crc32fast::hash(&buf[..152]);
        buf[152..156].copy_from_slice(&crc.to_le_bytes());

        let hdr = CryptoHeader::deserialize(&buf).unwrap();
        assert_eq!(hdr.version, CRYPTO_VERSION_V1);
        assert_eq!(hdr.kdf_algorithm, KdfAlgorithm::Pbkdf2HmacSha256);
        assert_eq!(hdr.argon2_memory_kib, 0);

        let recovered = hdr.unlock(password).unwrap();
        assert_eq!(recovered, master_key);
    }

    #[test]
    fn test_argon2id_create_unlock() {
        let password = b"argon2id_header_test";
        let (hdr, master_key) = test_create_argon2id(password);

        assert_eq!(hdr.kdf_algorithm, KdfAlgorithm::Argon2id);
        assert_eq!(hdr.argon2_memory_kib, 16 * 1024);
        assert_eq!(hdr.argon2_time_cost, 1);
        assert_eq!(hdr.argon2_parallelism, 1);

        // Serialize and deserialize
        let buf = hdr.serialize(EU);
        let hdr2 = CryptoHeader::deserialize(&buf).unwrap();
        assert_eq!(hdr2.kdf_algorithm, KdfAlgorithm::Argon2id);

        let recovered = hdr2.unlock(password).unwrap();
        assert_eq!(recovered, master_key);

        // Wrong password
        assert!(hdr2.unlock(b"wrong").is_err());
    }

    #[test]
    fn test_change_password_pbkdf2_to_argon2id() {
        let old_pw = b"old_pass";
        let new_pw = b"new_pass";
        let (mut hdr, master_key) = test_create(old_pw);
        assert_eq!(hdr.kdf_algorithm, KdfAlgorithm::Pbkdf2HmacSha256);

        // Switch to Argon2id
        let argon_params = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        };
        hdr.change_password(old_pw, new_pw, Some(argon_params)).unwrap();

        assert_eq!(hdr.kdf_algorithm, KdfAlgorithm::Argon2id);
        assert_eq!(hdr.version, CRYPTO_VERSION);

        // Old password should fail
        assert!(hdr.unlock(old_pw).is_err());

        // New password should recover master key
        let recovered = hdr.unlock(new_pw).unwrap();
        assert_eq!(recovered, master_key);

        // Roundtrip through serialize/deserialize
        let buf = hdr.serialize(EU);
        let hdr2 = CryptoHeader::deserialize(&buf).unwrap();
        let recovered2 = hdr2.unlock(new_pw).unwrap();
        assert_eq!(recovered2, master_key);
    }

    #[test]
    fn test_tampered_v2_header_detected() {
        let (hdr, _) = test_create(b"tamper_v2");
        let mut buf = hdr.serialize(EU);

        // Tamper a byte in the KDF fields area
        buf[153] ^= 0xFF;
        let result = CryptoHeader::deserialize(&buf);
        assert!(result.is_err(), "tampered v2 header should fail CRC");
    }

    #[test]
    fn test_kdf_params_roundtrip() {
        let (hdr, _) = test_create_argon2id(b"params_test");
        let params = hdr.kdf_params();
        assert_eq!(params.algorithm, KdfAlgorithm::Argon2id);
        assert_eq!(params.argon2_memory_kib, 16 * 1024);
        assert_eq!(params.argon2_time_cost, 1);
        assert_eq!(params.argon2_parallelism, 1);
    }
}
