use anyhow::{bail, Result};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use zeroize::Zeroize;

/// Minimum PBKDF2 iteration count.
/// 300,000 balances security with acceptable unlock latency on consumer hardware.
pub const MIN_PBKDF2_ITERS: u32 = 300_000;

/// PBKDF2 output length: 64-byte KEK + 32-byte HMAC key = 96 bytes.
const DERIVED_KEY_LEN: usize = 96;

// ---------------------------------------------------------------------------
// KDF Algorithm & Params (Phase 7F)
// ---------------------------------------------------------------------------

/// Supported key derivation function algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum KdfAlgorithm {
    /// PBKDF2-HMAC-SHA256 (v1 default, FIPS-compliant)
    Pbkdf2HmacSha256 = 0,
    /// Argon2id (v2 default, memory-hard)
    Argon2id = 1,
}

impl KdfAlgorithm {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::Pbkdf2HmacSha256),
            1 => Ok(Self::Argon2id),
            _ => bail!("unknown KDF algorithm: {v}"),
        }
    }
}

/// Parameters for key derivation. Encapsulates algorithm choice and
/// algorithm-specific tuning knobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KdfParams {
    pub algorithm: KdfAlgorithm,
    /// PBKDF2 iterations (only used when algorithm == Pbkdf2HmacSha256)
    pub pbkdf2_iterations: u32,
    /// Argon2id memory cost in KiB (only used when algorithm == Argon2id)
    pub argon2_memory_kib: u32,
    /// Argon2id time cost (iterations) (only used when algorithm == Argon2id)
    pub argon2_time_cost: u32,
    /// Argon2id parallelism (lanes) (only used when algorithm == Argon2id)
    pub argon2_parallelism: u32,
}

impl KdfParams {
    /// Default PBKDF2 parameters (balanced preset: 600,000 iterations).
    pub fn default_pbkdf2() -> Self {
        Self {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 600_000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        }
    }

    /// Default Argon2id parameters (balanced preset: 32 MiB, t=2, p=2).
    pub fn default_argon2id() -> Self {
        Self {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 32 * 1024, // 32 MiB
            argon2_time_cost: 2,
            argon2_parallelism: 2,
        }
    }

    /// Validate that the parameters are within acceptable ranges.
    pub fn validate(&self) -> Result<()> {
        match self.algorithm {
            KdfAlgorithm::Pbkdf2HmacSha256 => {
                if self.pbkdf2_iterations < 100_000 {
                    bail!("PBKDF2 iterations ({}) below minimum (100,000)", self.pbkdf2_iterations);
                }
                if self.pbkdf2_iterations > 10_000_000 {
                    bail!("PBKDF2 iterations ({}) above maximum (10,000,000)", self.pbkdf2_iterations);
                }
            }
            KdfAlgorithm::Argon2id => {
                if self.argon2_memory_kib < 16 * 1024 {
                    bail!(
                        "Argon2id memory ({} KiB) below minimum (16 MiB = 16384 KiB)",
                        self.argon2_memory_kib
                    );
                }
                if self.argon2_memory_kib > 256 * 1024 {
                    bail!(
                        "Argon2id memory ({} KiB) above maximum (256 MiB = 262144 KiB)",
                        self.argon2_memory_kib
                    );
                }
                if self.argon2_time_cost < 1 || self.argon2_time_cost > 6 {
                    bail!(
                        "Argon2id time_cost ({}) out of range [1, 6]",
                        self.argon2_time_cost
                    );
                }
                if self.argon2_parallelism < 1 || self.argon2_parallelism > 4 {
                    bail!(
                        "Argon2id parallelism ({}) out of range [1, 4]",
                        self.argon2_parallelism
                    );
                }
            }
        }
        Ok(())
    }
}

/// Derive a Key Encryption Key (KEK) and an HMAC key from a password
/// using the specified KDF parameters.
///
/// Returns `(kek, hmac_key)` where `kek` is 64 bytes and `hmac_key` is 32 bytes.
pub fn derive_keys_with_params(
    password: &[u8],
    salt: &[u8; 32],
    params: &KdfParams,
) -> Result<([u8; 64], [u8; 32])> {
    let mut derived = [0u8; DERIVED_KEY_LEN];

    match params.algorithm {
        KdfAlgorithm::Pbkdf2HmacSha256 => {
            pbkdf2_hmac::<Sha256>(password, salt, params.pbkdf2_iterations, &mut derived);
        }
        KdfAlgorithm::Argon2id => {
            use argon2::{Algorithm, Argon2, Params, Version};

            let argon_params = Params::new(
                params.argon2_memory_kib,
                params.argon2_time_cost,
                params.argon2_parallelism,
                Some(DERIVED_KEY_LEN),
            )
            .map_err(|e| anyhow::anyhow!("Argon2id params error: {e}"))?;

            let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

            argon2
                .hash_password_into(password, salt, &mut derived)
                .map_err(|e| anyhow::anyhow!("Argon2id derivation failed: {e}"))?;
        }
    }

    let mut kek = [0u8; 64];
    let mut hmac_key = [0u8; 32];
    kek.copy_from_slice(&derived[..64]);
    hmac_key.copy_from_slice(&derived[64..96]);
    derived.zeroize();

    Ok((kek, hmac_key))
}

/// Derive a Key Encryption Key (KEK) and an HMAC key from a password
/// using PBKDF2-HMAC-SHA256 (backward-compatibility wrapper).
///
/// Returns `(kek, hmac_key)` where `kek` is 64 bytes and `hmac_key` is 32 bytes.
pub fn derive_keys(password: &[u8], salt: &[u8; 32], iters: u32) -> ([u8; 64], [u8; 32]) {
    let params = KdfParams {
        algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
        pbkdf2_iterations: iters,
        argon2_memory_kib: 0,
        argon2_time_cost: 0,
        argon2_parallelism: 0,
    };
    // PBKDF2 branch can't fail, so unwrap is safe here
    derive_keys_with_params(password, salt, &params).unwrap()
}

/// Generate a cryptographically random 32-byte salt.
pub fn generate_salt() -> [u8; 32] {
    let mut salt = [0u8; 32];
    OsRng.fill_bytes(&mut salt);
    salt
}

/// Generate a cryptographically random 64-byte master key.
pub fn generate_master_key() -> [u8; 64] {
    let mut key = [0u8; 64];
    OsRng.fill_bytes(&mut key);
    key
}

/// Compute HMAC-SHA256 over `master_key` using `hmac_key`.
pub fn compute_hmac(hmac_key: &[u8], master_key: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(hmac_key)
        .expect("HMAC can take key of any size");
    mac.update(master_key);
    let result = mac.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result.into_bytes());
    out
}

/// Verify an HMAC-SHA256 tag (constant-time comparison).
pub fn verify_hmac(hmac_key: &[u8], master_key: &[u8], expected: &[u8; 32]) -> Result<()> {
    let mut mac = Hmac::<Sha256>::new_from_slice(hmac_key)
        .expect("HMAC can take key of any size");
    mac.update(master_key);
    mac.verify_slice(expected)
        .map_err(|_| anyhow::anyhow!("HMAC verification failed — wrong password or corrupted header"))?;
    Ok(())
}

/// XOR key wrapping: encrypt or decrypt a 64-byte key with a 64-byte KEK.
/// This is symmetric — calling it twice returns the original.
pub fn xor_key_wrap(key: &[u8; 64], kek: &[u8; 64]) -> [u8; 64] {
    let mut out = [0u8; 64];
    for i in 0..64 {
        out[i] = key[i] ^ kek[i];
    }
    out
}

/// Benchmark KDF derivation: runs the KDF once with fixed test data
/// and returns the wall-clock time.
pub fn benchmark_kdf(params: &KdfParams) -> Result<std::time::Duration> {
    let test_password = b"benchmark_test_password_1234567890";
    let test_salt = [0xAAu8; 32];
    let start = std::time::Instant::now();
    derive_keys_with_params(test_password, &test_salt, params)?;
    Ok(start.elapsed())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pbkdf2_deterministic() {
        let password = b"test_password";
        let salt = [0x42u8; 32];
        let iters = 1000; // Low for test speed

        let (kek1, hmac1) = derive_keys(password, &salt, iters);
        let (kek2, hmac2) = derive_keys(password, &salt, iters);

        assert_eq!(kek1, kek2);
        assert_eq!(hmac1, hmac2);
    }

    #[test]
    fn test_pbkdf2_different_passwords() {
        let salt = [0x42u8; 32];
        let iters = 1000;

        let (kek1, hmac1) = derive_keys(b"password_a", &salt, iters);
        let (kek2, hmac2) = derive_keys(b"password_b", &salt, iters);

        assert_ne!(kek1, kek2);
        assert_ne!(hmac1, hmac2);
    }

    #[test]
    fn test_pbkdf2_different_salts() {
        let password = b"same_password";
        let iters = 1000;

        let salt_a = [0xAAu8; 32];
        let salt_b = [0xBBu8; 32];

        let (kek1, _) = derive_keys(password, &salt_a, iters);
        let (kek2, _) = derive_keys(password, &salt_b, iters);

        assert_ne!(kek1, kek2);
    }

    #[test]
    fn test_derive_key_length() {
        let (kek, hmac_key) = derive_keys(b"pw", &[0u8; 32], 1000);
        assert_eq!(kek.len(), 64);
        assert_eq!(hmac_key.len(), 32);
    }

    #[test]
    fn test_hmac_verify_correct_and_wrong() {
        let hmac_key = [0xABu8; 32];
        let master_key = [0xCDu8; 64];

        let tag = compute_hmac(&hmac_key, &master_key);
        // Correct case
        assert!(verify_hmac(&hmac_key, &master_key, &tag).is_ok());

        // Wrong master key
        let wrong_key = [0x00u8; 64];
        assert!(verify_hmac(&hmac_key, &wrong_key, &tag).is_err());

        // Tampered tag
        let mut bad_tag = tag;
        bad_tag[0] ^= 0xFF;
        assert!(verify_hmac(&hmac_key, &master_key, &bad_tag).is_err());
    }

    #[test]
    fn test_xor_key_wrap_roundtrip() {
        let key = generate_master_key();
        let kek: [u8; 64] = {
            let mut k = [0u8; 64];
            OsRng.fill_bytes(&mut k);
            k
        };

        let wrapped = xor_key_wrap(&key, &kek);
        assert_ne!(wrapped, key); // wrapped should differ from original
        let unwrapped = xor_key_wrap(&wrapped, &kek);
        assert_eq!(unwrapped, key); // roundtrip
    }

    // -----------------------------------------------------------------------
    // Phase 7F — KdfAlgorithm / KdfParams / Argon2id tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_argon2id_deterministic() {
        let password = b"argon2_test_password";
        let salt = [0x42u8; 32];
        // Use small params for test speed
        let params = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024, // 16 MiB (minimum)
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        };

        let (kek1, hmac1) = derive_keys_with_params(password, &salt, &params).unwrap();
        let (kek2, hmac2) = derive_keys_with_params(password, &salt, &params).unwrap();

        assert_eq!(kek1, kek2);
        assert_eq!(hmac1, hmac2);
    }

    #[test]
    fn test_argon2id_output_size() {
        let params = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        };
        let (kek, hmac_key) = derive_keys_with_params(b"pw", &[0u8; 32], &params).unwrap();
        assert_eq!(kek.len(), 64);
        assert_eq!(hmac_key.len(), 32);
    }

    #[test]
    fn test_argon2id_different_memory_sizes_diverge() {
        let password = b"same_password";
        let salt = [0xAA; 32];

        let params_16 = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        };
        let params_32 = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 32 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        };

        let (kek1, _) = derive_keys_with_params(password, &salt, &params_16).unwrap();
        let (kek2, _) = derive_keys_with_params(password, &salt, &params_32).unwrap();

        assert_ne!(kek1, kek2, "different memory sizes must produce different output");
    }

    #[test]
    fn test_argon2id_differs_from_pbkdf2() {
        let password = b"cross_algo_test";
        let salt = [0xBB; 32];

        let pbkdf2_params = KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: 1000,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        };
        let argon2_params = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        };

        let (kek_pb, _) = derive_keys_with_params(password, &salt, &pbkdf2_params).unwrap();
        let (kek_ar, _) = derive_keys_with_params(password, &salt, &argon2_params).unwrap();

        assert_ne!(kek_pb, kek_ar, "different algorithms must produce different output");
    }

    #[test]
    fn test_kdf_params_validate_pbkdf2() {
        // Valid
        let mut p = KdfParams::default_pbkdf2();
        assert!(p.validate().is_ok());

        // Too low
        p.pbkdf2_iterations = 50_000;
        assert!(p.validate().is_err());

        // Too high
        p.pbkdf2_iterations = 20_000_000;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_kdf_params_validate_argon2id() {
        // Valid
        let mut p = KdfParams::default_argon2id();
        assert!(p.validate().is_ok());

        // Memory too low
        p.argon2_memory_kib = 8 * 1024;
        assert!(p.validate().is_err());

        // Memory too high
        p = KdfParams::default_argon2id();
        p.argon2_memory_kib = 512 * 1024;
        assert!(p.validate().is_err());

        // Time too high
        p = KdfParams::default_argon2id();
        p.argon2_time_cost = 7;
        assert!(p.validate().is_err());

        // Time zero
        p = KdfParams::default_argon2id();
        p.argon2_time_cost = 0;
        assert!(p.validate().is_err());

        // Parallelism too high
        p = KdfParams::default_argon2id();
        p.argon2_parallelism = 5;
        assert!(p.validate().is_err());

        // Parallelism zero
        p = KdfParams::default_argon2id();
        p.argon2_parallelism = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn test_kdf_algorithm_from_u8() {
        assert_eq!(KdfAlgorithm::from_u8(0).unwrap(), KdfAlgorithm::Pbkdf2HmacSha256);
        assert_eq!(KdfAlgorithm::from_u8(1).unwrap(), KdfAlgorithm::Argon2id);
        assert!(KdfAlgorithm::from_u8(2).is_err());
        assert!(KdfAlgorithm::from_u8(255).is_err());
    }

    #[test]
    fn test_pbkdf2_via_derive_keys_with_params_matches_legacy() {
        let password = b"compat_test";
        let salt = [0xCC; 32];
        let iters = 1000;

        let (kek_legacy, hmac_legacy) = derive_keys(password, &salt, iters);
        let params = KdfParams {
            algorithm: KdfAlgorithm::Pbkdf2HmacSha256,
            pbkdf2_iterations: iters,
            argon2_memory_kib: 0,
            argon2_time_cost: 0,
            argon2_parallelism: 0,
        };
        let (kek_new, hmac_new) = derive_keys_with_params(password, &salt, &params).unwrap();

        assert_eq!(kek_legacy, kek_new, "new wrapper must match legacy derive_keys");
        assert_eq!(hmac_legacy, hmac_new);
    }

    #[test]
    fn test_argon2id_parallelism_deterministic() {
        let password = b"parallel_test";
        let salt = [0x42u8; 32];
        let params = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 4,
        };
        let (kek1, hmac1) = derive_keys_with_params(password, &salt, &params).unwrap();
        let (kek2, hmac2) = derive_keys_with_params(password, &salt, &params).unwrap();
        assert_eq!(kek1, kek2, "parallel argon2 must be deterministic");
        assert_eq!(hmac1, hmac2);
    }

    #[test]
    fn test_argon2id_different_parallelism_diverges() {
        let password = b"parallel_diverge";
        let salt = [0xAA; 32];
        let params_p1 = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 1,
        };
        let params_p2 = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 16 * 1024,
            argon2_time_cost: 1,
            argon2_parallelism: 2,
        };
        let (kek1, _) = derive_keys_with_params(password, &salt, &params_p1).unwrap();
        let (kek2, _) = derive_keys_with_params(password, &salt, &params_p2).unwrap();
        assert_ne!(kek1, kek2, "different parallelism must produce different output");
    }

    #[test]
    #[ignore]
    fn test_argon2id_parallelism_speedup() {
        let password = b"speedup_test";
        let salt = [0xBB; 32];
        let params_p1 = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 64 * 1024, // 64 MiB
            argon2_time_cost: 3,
            argon2_parallelism: 1,
        };
        let params_p4 = KdfParams {
            algorithm: KdfAlgorithm::Argon2id,
            pbkdf2_iterations: 0,
            argon2_memory_kib: 64 * 1024,
            argon2_time_cost: 3,
            argon2_parallelism: 4,
        };
        let start1 = std::time::Instant::now();
        let _ = derive_keys_with_params(password, &salt, &params_p1).unwrap();
        let d1 = start1.elapsed();
        let start4 = std::time::Instant::now();
        let _ = derive_keys_with_params(password, &salt, &params_p4).unwrap();
        let d4 = start4.elapsed();
        assert!(
            d4 < d1,
            "p=4 ({d4:?}) should be faster than p=1 ({d1:?})"
        );
    }
}
