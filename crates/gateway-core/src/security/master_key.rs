use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::error::{GatewayError, Result};

/// 32-byte master key used for AES-256-GCM credential encryption and as
/// the HMAC secret for admin session tokens. Loaded from a single env var
/// (base64-encoded) at startup.
#[derive(Clone)]
pub struct MasterKey(pub [u8; 32]);

impl MasterKey {
    pub fn from_base64(s: &str) -> Result<Self> {
        let raw = STANDARD
            .decode(s.trim())
            .map_err(|e| GatewayError::Internal(format!("master key base64 decode failed: {e}")))?;
        if raw.len() != 32 {
            return Err(GatewayError::Internal(format!(
                "master key must decode to exactly 32 bytes (got {})",
                raw.len()
            )));
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(&raw);
        Ok(Self(buf))
    }

    pub fn from_env(var: &str) -> Result<Self> {
        let v = std::env::var(var)
            .map_err(|_| GatewayError::Internal(format!("env var {var} not set")))?;
        Self::from_base64(&v)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
