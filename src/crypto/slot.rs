use anyhow::Result;
use zeroize::Zeroize;

use super::key::{
    compute_hmac, derive_keys_with_params, verify_hmac, xor_key_wrap, KdfAlgorithm, KdfParams,
};

pub const MAX_KEY_SLOTS: usize = 4;
pub const KEY_SLOT_DISABLED: u8 = 0;
pub const KEY_SLOT_ACTIVE: u8 = 1;
pub const CFS_FEATURE_DATA_AEAD: u32 = 0x0008;

/// Serialized size of one KeySlot on disk (144 bytes).
/// [0]       state
/// [1..33]   slot_salt: [u8;32]
/// [33]      kdf_algorithm: u8
/// [34..38]  pbkdf2_iterations: u32 LE
/// [38..42]  argon2_memory_kib: u32 LE
/// [42..44]  argon2_time_cost: u16 LE
/// [44..46]  argon2_parallelism: u16 LE
/// [46..110] encrypted_master_key: [u8;64]
/// [110..142] slot_hmac: [u8;32]
/// [142..144] zero padding
pub const KEY_SLOT_SIZE: usize = 144;

#[derive(Clone)]
pub struct KeySlot {
    pub state: u8,
    pub slot_salt: [u8; 32],
    pub kdf_params: KdfParams,
    pub encrypted_master_key: [u8; 64],
    pub slot_hmac: [u8; 32],
}

impl Drop for KeySlot {
    fn drop(&mut self) {
        self.slot_salt.zeroize();
        self.encrypted_master_key.zeroize();
        self.slot_hmac.zeroize();
    }
}

impl KeySlot {
    pub fn create(master_key: &[u8; 64], password: &[u8], kdf_params: &KdfParams) -> Result<Self> {
        let slot_salt = generate_slot_salt();
        let (mut kek, mut hmac_key) = derive_keys_with_params(password, &slot_salt, kdf_params)?;
        let encrypted_master_key = xor_key_wrap(master_key, &kek);
        let slot_hmac = compute_hmac(&hmac_key, master_key);
        kek.zeroize();
        hmac_key.zeroize();
        Ok(Self { state: KEY_SLOT_ACTIVE, slot_salt, kdf_params: kdf_params.clone(), encrypted_master_key, slot_hmac })
    }

    pub fn try_unlock(&self, password: &[u8]) -> Result<Option<[u8; 64]>> {
        if self.state != KEY_SLOT_ACTIVE { return Ok(None); }
        let (mut kek, mut hmac_key) = derive_keys_with_params(password, &self.slot_salt, &self.kdf_params)?;
        let mut candidate = xor_key_wrap(&self.encrypted_master_key, &kek);
        kek.zeroize();
        let result = verify_hmac(&hmac_key, &candidate, &self.slot_hmac);
        hmac_key.zeroize();
        match result {
            Ok(_) => Ok(Some(candidate)),
            Err(_) => { candidate.zeroize(); Err(anyhow::anyhow!("wrong password for this slot")) }
        }
    }

    pub fn revoke(&mut self) {
        self.encrypted_master_key.zeroize();
        self.slot_hmac.zeroize();
        self.slot_salt.zeroize();
        self.state = KEY_SLOT_DISABLED;
    }

    pub fn is_active(&self) -> bool { self.state == KEY_SLOT_ACTIVE }

    pub fn serialize(&self) -> [u8; KEY_SLOT_SIZE] {
        let mut buf = [0u8; KEY_SLOT_SIZE];
        buf[0] = self.state;
        buf[1..33].copy_from_slice(&self.slot_salt);
        buf[33] = self.kdf_params.algorithm as u8;
        buf[34..38].copy_from_slice(&self.kdf_params.pbkdf2_iterations.to_le_bytes());
        buf[38..42].copy_from_slice(&self.kdf_params.argon2_memory_kib.to_le_bytes());
        buf[42..44].copy_from_slice(&(self.kdf_params.argon2_time_cost as u16).to_le_bytes());
        buf[44..46].copy_from_slice(&(self.kdf_params.argon2_parallelism as u16).to_le_bytes());
        buf[46..110].copy_from_slice(&self.encrypted_master_key);
        buf[110..142].copy_from_slice(&self.slot_hmac);
        buf
    }

    pub fn deserialize(buf: &[u8; KEY_SLOT_SIZE]) -> Result<Self> {
        let state = buf[0];
        let mut slot_salt = [0u8; 32];
        slot_salt.copy_from_slice(&buf[1..33]);
        let kdf_algorithm = KdfAlgorithm::from_u8(buf[33])?;
        let pbkdf2_iterations = u32::from_le_bytes(buf[34..38].try_into().unwrap());
        let argon2_memory_kib = u32::from_le_bytes(buf[38..42].try_into().unwrap());
        let argon2_time_cost = u16::from_le_bytes(buf[42..44].try_into().unwrap()) as u32;
        let argon2_parallelism = u16::from_le_bytes(buf[44..46].try_into().unwrap()) as u32;
        let mut encrypted_master_key = [0u8; 64];
        encrypted_master_key.copy_from_slice(&buf[46..110]);
        let mut slot_hmac = [0u8; 32];
        slot_hmac.copy_from_slice(&buf[110..142]);
        Ok(Self { state, slot_salt, kdf_params: KdfParams { algorithm: kdf_algorithm, pbkdf2_iterations, argon2_memory_kib, argon2_time_cost, argon2_parallelism }, encrypted_master_key, slot_hmac })
    }

    pub fn empty() -> Self {
        Self { state: KEY_SLOT_DISABLED, slot_salt: [0u8; 32], kdf_params: KdfParams { algorithm: KdfAlgorithm::Argon2id, pbkdf2_iterations: 0, argon2_memory_kib: 0, argon2_time_cost: 0, argon2_parallelism: 0 }, encrypted_master_key: [0u8; 64], slot_hmac: [0u8; 32] }
    }
}

#[derive(Debug, Clone)]
pub struct KeySlotInfo {
    pub index: usize,
    pub is_active: bool,
    pub kdf_algorithm: String,
    pub argon2_memory_mib: Option<u32>,
    pub argon2_time_cost: Option<u32>,
    pub argon2_parallelism: Option<u32>,
    pub pbkdf2_iterations: Option<u32>,
}

impl From<(usize, &KeySlot)> for KeySlotInfo {
    fn from((index, slot): (usize, &KeySlot)) -> Self {
        let is_active = slot.is_active();
        let (algo_str, mem, time, para, iters) = if is_active {
            match slot.kdf_params.algorithm {
                KdfAlgorithm::Argon2id => ("argon2id".to_string(), Some(slot.kdf_params.argon2_memory_kib / 1024), Some(slot.kdf_params.argon2_time_cost), Some(slot.kdf_params.argon2_parallelism), None),
                KdfAlgorithm::Pbkdf2HmacSha256 => ("pbkdf2".to_string(), None, None, None, Some(slot.kdf_params.pbkdf2_iterations)),
                KdfAlgorithm::Pbkdf2HmacSha512 => ("pbkdf2-sha512".to_string(), None, None, None, Some(slot.kdf_params.pbkdf2_iterations)),
            }
        } else { ("disabled".to_string(), None, None, None, None) };
        Self { index, is_active, kdf_algorithm: algo_str, argon2_memory_mib: mem, argon2_time_cost: time, argon2_parallelism: para, pbkdf2_iterations: iters }
    }
}

fn generate_slot_salt() -> [u8; 32] {
    use rand::RngCore;
    let mut salt = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    salt
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fast_kdf() -> KdfParams { KdfParams { algorithm: KdfAlgorithm::Pbkdf2HmacSha256, pbkdf2_iterations: 100_000, argon2_memory_kib: 0, argon2_time_cost: 0, argon2_parallelism: 0 } }
    fn make_mk() -> [u8; 64] { use rand::RngCore; let mut k = [0u8; 64]; rand::rngs::OsRng.fill_bytes(&mut k); k }

    #[test]
    fn test_slot_create_unlock_roundtrip() {
        let mk = make_mk();
        let slot = KeySlot::create(&mk, b"pw", &fast_kdf()).unwrap();
        assert!(slot.is_active());
        assert_eq!(slot.try_unlock(b"pw").unwrap().unwrap(), mk);
    }
    #[test]
    fn test_slot_wrong_password() {
        let mk = make_mk();
        let slot = KeySlot::create(&mk, b"correct", &fast_kdf()).unwrap();
        assert!(slot.try_unlock(b"wrong").is_err());
    }
    #[test]
    fn test_slot_disabled_returns_none() {
        let slot = KeySlot::empty();
        assert!(slot.try_unlock(b"any").unwrap().is_none());
    }
    #[test]
    fn test_slot_revoke() {
        let mk = make_mk();
        let mut slot = KeySlot::create(&mk, b"pw", &fast_kdf()).unwrap();
        slot.revoke();
        assert!(!slot.is_active());
        assert_eq!(slot.encrypted_master_key, [0u8; 64]);
    }
    #[test]
    fn test_slot_serialize_deserialize() {
        let mk = make_mk();
        let slot = KeySlot::create(&mk, b"rt", &fast_kdf()).unwrap();
        let buf = slot.serialize();
        assert_eq!(buf.len(), KEY_SLOT_SIZE);
        let slot2 = KeySlot::deserialize(&buf).unwrap();
        assert_eq!(slot2.try_unlock(b"rt").unwrap().unwrap(), mk);
    }
}
