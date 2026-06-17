use aes::cipher::generic_array::GenericArray;
use aes::cipher::KeyInit;
use aes::Aes256;
use rayon::prelude::*;
use xts_mode::{get_tweak_default, Xts128};

/// AES-256-XTS cipher wrapper.
///
/// Encrypts/decrypts data in `encryption_unit`-sized blocks (typically 4096).
/// The XTS tweak is the block index encoded as LE u128 (dm-crypt XTS-plain64).
pub struct XtsCipher {
    xts: Xts128<Aes256>,
    encryption_unit: u32,
}

// SAFETY: `Xts128<Aes256>` holds only immutable AES key schedules computed at
// construction time. `encrypt_sector` / `decrypt_sector` take `&self` and
// perform no interior mutation, so sharing `XtsCipher` across Rayon threads is
// safe. There is no interior mutability (no Cell, Mutex, etc.).
unsafe impl Sync for XtsCipher {}

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

    /// Encrypt multiple contiguous blocks in parallel (Rayon).
    ///
    /// Falls back to serial path for fewer than 4 blocks, avoiding thread
    /// spawn overhead for small I/O operations.
    pub fn encrypt_blocks_parallel(&self, first_block_index: u64, data: &mut [u8]) {
        let eu = self.encryption_unit as usize;
        let num_blocks = data.len() / eu;
        if num_blocks < 4 {
            return self.encrypt_blocks(first_block_index, data);
        }
        data.par_chunks_mut(eu)
            .enumerate()
            .for_each(|(i, chunk)| {
                let tweak = get_tweak_default(first_block_index as u128 + i as u128);
                self.xts.encrypt_sector(chunk, tweak);
            });
    }

    /// Decrypt multiple contiguous blocks in parallel (Rayon).
    ///
    /// Falls back to serial path for fewer than 4 blocks.
    pub fn decrypt_blocks_parallel(&self, first_block_index: u64, data: &mut [u8]) {
        let eu = self.encryption_unit as usize;
        let num_blocks = data.len() / eu;
        if num_blocks < 4 {
            return self.decrypt_blocks(first_block_index, data);
        }
        data.par_chunks_mut(eu)
            .enumerate()
            .for_each(|(i, chunk)| {
                let tweak = get_tweak_default(first_block_index as u128 + i as u128);
                self.xts.decrypt_sector(chunk, tweak);
            });
    }
}

/// Returns `true` if the CPU supports AES-NI hardware acceleration.
///
/// On x86-64 this checks CPUID at runtime using the standard library's
/// `is_x86_feature_detected!` macro. On other targets it always returns `false`.
pub fn aes_ni_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::is_x86_feature_detected!("aes")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
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

    #[test]
    fn test_parallel_encrypt_matches_serial() {
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        let mut original = vec![0u8; 4096 * 8];
        OsRng.fill_bytes(&mut original);

        let mut serial = original.clone();
        let mut parallel = original.clone();

        cipher.encrypt_blocks(0, &mut serial);
        cipher.encrypt_blocks_parallel(0, &mut parallel);

        assert_eq!(
            serial, parallel,
            "parallel encrypt must produce identical ciphertext to serial"
        );
    }

    #[test]
    fn test_parallel_decrypt_matches_serial() {
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        // Encrypt first (serially), then compare decryption methods
        let mut ciphertext = vec![0u8; 4096 * 8];
        OsRng.fill_bytes(&mut ciphertext);

        let mut serial_dec = ciphertext.clone();
        let mut parallel_dec = ciphertext.clone();

        cipher.decrypt_blocks(0, &mut serial_dec);
        cipher.decrypt_blocks_parallel(0, &mut parallel_dec);

        assert_eq!(
            serial_dec, parallel_dec,
            "parallel decrypt must produce identical output to serial"
        );
    }

    #[test]
    fn test_parallel_roundtrip() {
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        let mut data = vec![0u8; 4096 * 8];
        OsRng.fill_bytes(&mut data);
        let original = data.clone();

        cipher.encrypt_blocks_parallel(0, &mut data);
        assert_ne!(data, original, "encrypted data must differ from plaintext");

        cipher.decrypt_blocks_parallel(0, &mut data);
        assert_eq!(data, original, "roundtrip must recover original plaintext");
    }

    #[test]
    fn test_parallel_small_falls_back_to_serial() {
        // For < 4 blocks the parallel methods fall back to serial.
        // Verify correctness is maintained.
        let key = random_key();
        let cipher = XtsCipher::new(&key, 4096);

        let mut data = vec![0u8; 4096 * 2]; // only 2 blocks
        OsRng.fill_bytes(&mut data);
        let original = data.clone();

        cipher.encrypt_blocks_parallel(0, &mut data);
        cipher.decrypt_blocks_parallel(0, &mut data);
        assert_eq!(data, original);
    }

    #[test]
    fn test_aes_ni_available_smoke() {
        // Just ensure the call doesn't panic and returns a bool.
        let result = aes_ni_available();
        // Result depends on hardware — we can't assert true/false.
        let _ = result;
    }
}
