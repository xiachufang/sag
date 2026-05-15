use rand::distributions::Alphanumeric;
use rand::Rng;
use subtle::ConstantTimeEq;

use crate::error::Result;
use crate::security::master_key::MasterKey;

const PREFIX_LIVE: &str = "sk-gw-live-";
const PREFIX_TEST: &str = "sk-gw-test-";
const SECRET_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEnv {
    Live,
    Test,
}

impl KeyEnv {
    pub fn prefix(self) -> &'static str {
        match self {
            KeyEnv::Live => PREFIX_LIVE,
            KeyEnv::Test => PREFIX_TEST,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "live" => Some(KeyEnv::Live),
            "test" => Some(KeyEnv::Test),
            _ => None,
        }
    }
}

pub struct GatewayKeySecret {
    /// Full plaintext, returned to caller exactly once at creation time.
    pub plaintext: String,
    pub prefix: String,
    pub last4: String,
    /// 32-byte keyed BLAKE3 hash. Deterministic for a given plaintext +
    /// master key, so it can be indexed and looked up.
    pub hash: Vec<u8>,
}

/// Mint a new gateway key. Plaintext is returned to the caller exactly
/// once; only the keyed hash should be persisted.
pub fn generate_gateway_key(env: KeyEnv, master: &MasterKey) -> Result<GatewayKeySecret> {
    let secret: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(SECRET_LEN)
        .map(char::from)
        .collect();
    let plaintext = format!("{}{}", env.prefix(), secret);
    let last4: String = plaintext
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let hash = derive_hash(master, &plaintext);
    Ok(GatewayKeySecret {
        plaintext,
        prefix: env.prefix().to_string(),
        last4,
        hash,
    })
}

pub fn parse_gateway_key(secret: &str) -> Option<&'static str> {
    if secret.starts_with(PREFIX_LIVE) {
        Some(PREFIX_LIVE)
    } else if secret.starts_with(PREFIX_TEST) {
        Some(PREFIX_TEST)
    } else {
        None
    }
}

/// Compute the deterministic 32-byte hash used to index a key in storage.
pub fn derive_hash(master: &MasterKey, plaintext: &str) -> Vec<u8> {
    blake3::keyed_hash(master.as_bytes(), plaintext.as_bytes())
        .as_bytes()
        .to_vec()
}

/// Constant-time comparison against a stored hash blob (in case the caller
/// already pulled a candidate row and wants to double-check).
pub fn verify_gateway_key(presented: &str, master: &MasterKey, stored_hash: &[u8]) -> bool {
    if stored_hash.len() != 32 {
        return false;
    }
    let h = derive_hash(master, presented);
    h.ct_eq(stored_hash).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_live_key() {
        let mk = MasterKey([7u8; 32]);
        let k = generate_gateway_key(KeyEnv::Live, &mk).unwrap();
        assert!(k.plaintext.starts_with(PREFIX_LIVE));
        assert_eq!(parse_gateway_key(&k.plaintext), Some(PREFIX_LIVE));
        assert!(verify_gateway_key(&k.plaintext, &mk, &k.hash));
        assert!(!verify_gateway_key("sk-gw-live-wrong", &mk, &k.hash));
        assert_eq!(derive_hash(&mk, &k.plaintext), k.hash);
    }
}
