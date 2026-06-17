use anyhow::Result;
use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Derive a deterministic 12-byte GCM nonce for `block_idx`.
/// HMAC-SHA256(tag_key, block_idx_le_u64)[0..12]
pub fn block_nonce(tag_key: &[u8; 32], block_idx: u64) -> [u8; 12] {
    let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(tag_key).expect("HMAC key always valid");
    mac.update(&block_idx.to_le_bytes());
    let digest = mac.finalize().into_bytes();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&digest[..12]);
    nonce
}

/// Compute 16-byte AES-256-GCM authentication tag over the XTS-encrypted
/// `ciphertext` at `block_idx`. Uses empty msg + (block_idx || ciphertext) as AAD.
/// The resulting 16-byte value is the GCM tag only (no re-encrypted data).
pub fn compute_block_tag(tag_key: &[u8; 32], block_idx: u64, ciphertext: &[u8]) -> [u8; 16] {
    let cipher = Aes256Gcm::new_from_slice(tag_key).expect("32-byte key always valid");
    let nonce_bytes = block_nonce(tag_key, block_idx);
    let nonce = Nonce::from_slice(&nonce_bytes);
    // Build AAD: block_idx_le (8 bytes) || ciphertext
    let mut aad = Vec::with_capacity(8 + ciphertext.len());
    aad.extend_from_slice(&block_idx.to_le_bytes());
    aad.extend_from_slice(ciphertext);
    // Encrypt empty msg -> output is just the 16-byte GCM tag
    let tag_bytes = cipher
        .encrypt(nonce, Payload { msg: &[], aad: &aad })
        .expect("AES-GCM encrypt with empty msg must succeed");
    debug_assert_eq!(tag_bytes.len(), 16);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&tag_bytes);
    tag
}

/// Verify that `expected_tag` matches the GCM tag for `ciphertext` at `block_idx`.
/// Returns Err on mismatch (constant-time comparison via AES-GCM).
pub fn verify_block_tag(
    tag_key: &[u8; 32],
    block_idx: u64,
    ciphertext: &[u8],
    expected_tag: &[u8; 16],
) -> Result<()> {
    let computed = compute_block_tag(tag_key, block_idx, ciphertext);
    if computed.ct_eq(expected_tag).unwrap_u8() == 0 {
        anyhow::bail!(
            "data integrity error: block {} authentication tag mismatch — tampering detected",
            block_idx
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_key() -> [u8; 32] { [0xDE; 32] }

    #[test]
    fn test_tag_roundtrip() {
        let key = tag_key();
        let ct = vec![0xABu8; 4096];
        let tag = compute_block_tag(&key, 0, &ct);
        assert!(verify_block_tag(&key, 0, &ct, &tag).is_ok());
    }
    #[test]
    fn test_tamper_detection() {
        let key = tag_key();
        let mut ct = vec![0x42u8; 4096];
        let tag = compute_block_tag(&key, 5, &ct);
        ct[100] ^= 0x01;
        assert!(verify_block_tag(&key, 5, &ct, &tag).is_err());
    }
    #[test]
    fn test_block_index_bound() {
        let key = tag_key();
        let ct = vec![0x99u8; 4096];
        let tag0 = compute_block_tag(&key, 0, &ct);
        assert!(verify_block_tag(&key, 1, &ct, &tag0).is_err());
    }
    #[test]
    fn test_different_keys_different_tags() {
        let ct = vec![0xFF; 4096];
        let t1 = compute_block_tag(&[0xAA; 32], 0, &ct);
        let t2 = compute_block_tag(&[0xBB; 32], 0, &ct);
        assert_ne!(t1, t2);
    }
    #[test]
    fn test_deterministic() {
        let key = tag_key();
        let ct = vec![0x00; 4096];
        assert_eq!(compute_block_tag(&key, 42, &ct), compute_block_tag(&key, 42, &ct));
    }
}
