use aes::cipher::generic_array::GenericArray;
use aes::cipher::KeyInit;
use aes::Aes256;
use xts_mode::{get_tweak_default, Xts128};

/// AES-256-XTS cipher wrapper.
///
/// Encrypts/decrypts data in `encryption_unit`-sized blocks (typically 4096).
/// The XTS tweak is the block index encoded as LE u128 (dm-crypt XTS-plain64).
pub struct XtsCipher {
    xts: Xts128<Aes256>,
    encryption_unit: u32,
}

impl Drop for XtsCipher {
    fn drop(&mut self) {
        // Zero the memory that holds the AES key schedule so that key material
        // doesn't linger in heap/stack after the cipher is no longer needed.
        // We use volatile writes to prevent the compiler from optimizing away
        // this zeroing as a dead store (the standard zeroize concern).
        // SAFETY: `self.xts` is being dropped immediately after this; we will
        // never read from it again, so overwriting its bytes is sound.
        let ptr = &raw mut self.xts as *mut u8;
        let len = std::mem::size_of::<Xts128<Aes256>>();
        for i in 0..len {
            // SAFETY: ptr+i is within the Xts128 allocation.
            unsafe { std::ptr::write_volatile(ptr.add(i), 0u8); }
        }
        // Compiler fence to ensure the volatile writes are not reordered past drop.
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

impl XtsCipher {
    /// Construct from a 64-byte master key (first 32 = cipher key, last 32 = tweak key).
    pub fn new(master_key: &[u8; 64], encryption_unit: u32) -> Self {
        let cipher_1 = Aes256::new(GenericArray::from_slice(&master_key[..32]));
        let cipher_2 = Aes256::new(GenericArray::from_slice(&master_key[32..]));
        let xts = Xts128::new(cipher_1, cipher_2);
        Self { xts, encryption_unit }
    }

    /// Encrypt a single encryption-unit-sized block in-place.
    pub fn encrypt_block(&self, block_index: u64, data: &mut [u8]) {
        debug_assert_eq!(data.len(), self.encryption_unit as usize);
        let tweak = get_tweak_default(block_index as u128);
        self.xts.encrypt_sector(data, tweak);
    }

    /// Decrypt a single encryption-unit-sized block in-place.
    pub fn decrypt_block(&self, block_index: u64, data: &mut [u8]) {
        debug_assert_eq!(data.len(), self.encryption_unit as usize);
        let tweak = get_tweak_default(block_index as u128);
        self.xts.decrypt_sector(data, tweak);
    }

    /// Encrypt multiple contiguous encryption-unit-sized blocks in-place.
    pub fn encrypt_blocks(&self, first_block_index: u64, data: &mut [u8]) {
        let eu = self.encryption_unit as usize;
        self.xts
            .encrypt_area(data, eu, first_block_index as u128, get_tweak_default);
    }

    /// Decrypt multiple contiguous encryption-unit-sized blocks in-place.
    pub fn decrypt_blocks(&self, first_block_index: u64, data: &mut [u8]) {
        let eu = self.encryption_unit as usize;
        self.xts
            .decrypt_area(data, eu, first_block_index as u128, get_tweak_default);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;
    use rand::RngCore;

    fn random_key() -> [u8; 64] {
        let mut key = [0u8; 64];
        OsRng.fill_bytes(&mut key);
        key
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        let original = vec![0xABu8; 4096];
        let mut buf = original.clone();

        cipher.encrypt_block(0, &mut buf);
        cipher.decrypt_block(0, &mut buf);

        assert_eq!(buf, original);
    }

    #[test]
    fn test_different_tweaks_different_ciphertext() {
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        let plaintext = vec![0xCDu8; 4096];

        let mut ct_a = plaintext.clone();
        cipher.encrypt_block(0, &mut ct_a);

        let mut ct_b = plaintext.clone();
        cipher.encrypt_block(1, &mut ct_b);

        assert_ne!(ct_a, ct_b, "same plaintext with different tweaks must produce different ciphertext");
    }

    #[test]
    fn test_same_tweak_deterministic() {
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        let plaintext = vec![0xEFu8; 4096];

        let mut ct1 = plaintext.clone();
        cipher.encrypt_block(42, &mut ct1);

        let mut ct2 = plaintext.clone();
        cipher.encrypt_block(42, &mut ct2);

        assert_eq!(ct1, ct2, "same key + tweak + plaintext must be deterministic");
    }

    #[test]
    fn test_ciphertext_differs_from_plaintext() {
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        let plaintext = vec![0x42u8; 4096];
        let mut ct = plaintext.clone();
        cipher.encrypt_block(0, &mut ct);

        assert_ne!(ct, plaintext, "ciphertext must differ from plaintext");
    }

    #[test]
    fn test_multi_block_roundtrip() {
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        let mut data = vec![0u8; 4096 * 4];
        OsRng.fill_bytes(&mut data);
        let original = data.clone();

        cipher.encrypt_blocks(0, &mut data);
        assert_ne!(data, original);

        cipher.decrypt_blocks(0, &mut data);
        assert_eq!(data, original);
    }

    #[test]
    fn test_wrong_key_produces_garbage() {
        let key_a = random_key();
        let key_b = random_key();
        let cipher_a = XtsCipher::new(&key_a, 4096);
        let cipher_b = XtsCipher::new(&key_b, 4096);

        let original = vec![0x99u8; 4096];
        let mut buf = original.clone();

        cipher_a.encrypt_block(0, &mut buf);
        cipher_b.decrypt_block(0, &mut buf);

        assert_ne!(buf, original, "decrypting with wrong key must not produce original data");
    }
}
