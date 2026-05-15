use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;

use crate::error::{GatewayError, Result};
use crate::security::master_key::MasterKey;

const NONCE_LEN: usize = 12;

/// Encrypt an API key with AES-256-GCM. The output layout is
/// `nonce(12) || ciphertext || tag(16)`.
pub fn encrypt_credential(master: &MasterKey, plaintext: &str) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(master.as_bytes())
        .map_err(|e| GatewayError::Internal(format!("aes init: {e}")))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| GatewayError::Internal(format!("aes encrypt: {e}")))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt_credential(master: &MasterKey, blob: &[u8]) -> Result<String> {
    if blob.len() < NONCE_LEN {
        return Err(GatewayError::Internal(
            "encrypted credential too short".into(),
        ));
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(master.as_bytes())
        .map_err(|e| GatewayError::Internal(format!("aes init: {e}")))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let pt = cipher
        .decrypt(nonce, ct)
        .map_err(|e| GatewayError::Internal(format!("aes decrypt: {e}")))?;
    String::from_utf8(pt).map_err(|e| GatewayError::Internal(format!("credential utf-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = MasterKey([7u8; 32]);
        let blob = encrypt_credential(&key, "sk-test-secret").unwrap();
        assert_eq!(decrypt_credential(&key, &blob).unwrap(), "sk-test-secret");
    }
}
