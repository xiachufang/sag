use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::error::{GatewayError, Result};
use crate::security::master_key::MasterKey;

/// JWT-like session token issued after admin password login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTokenClaims {
    pub sub: String, // admin user id
    pub username: String,
    pub exp: usize,  // unix seconds
    pub iat: usize,
}

#[derive(Clone)]
pub struct AdminTokenSigner {
    enc: EncodingKey,
    dec: DecodingKey,
}

impl AdminTokenSigner {
    pub fn new(master: &MasterKey) -> Self {
        Self {
            enc: EncodingKey::from_secret(master.as_bytes()),
            dec: DecodingKey::from_secret(master.as_bytes()),
        }
    }

    pub fn issue(&self, user_id: &str, username: &str, ttl_secs: i64) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        let claims = AdminTokenClaims {
            sub: user_id.into(),
            username: username.into(),
            iat: now as usize,
            exp: (now + ttl_secs) as usize,
        };
        encode(&Header::default(), &claims, &self.enc)
            .map_err(|e| GatewayError::Internal(format!("jwt encode: {e}")))
    }

    pub fn verify(&self, token: &str) -> Result<AdminTokenClaims> {
        let data = decode::<AdminTokenClaims>(token, &self.dec, &Validation::default())
            .map_err(|_| GatewayError::Unauthorized)?;
        Ok(data.claims)
    }
}

pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let phc = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| GatewayError::Internal(format!("argon2 hash: {e}")))?
        .to_string();
    Ok(phc)
}

pub fn verify_password(presented: &str, stored_phc: &str) -> Result<bool> {
    let parsed = PasswordHash::new(stored_phc)
        .map_err(|e| GatewayError::Internal(format!("parse argon2: {e}")))?;
    match Argon2::default().verify_password(presented.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(GatewayError::Internal(format!("argon2 verify: {e}"))),
    }
}
