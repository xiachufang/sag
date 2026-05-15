use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::Utc;
use subtle::ConstantTimeEq;

use gateway_core::security::{derive_hash, parse_gateway_key};

use crate::error::ApiError;
use crate::state::AppState;

/// Identity attached to admin-scoped requests.
#[derive(Debug, Clone)]
pub enum AdminPrincipal {
    /// Authenticated via the bootstrap root token from env.
    Root,
    /// Authenticated via a session token issued after password login.
    User { id: String, username: String },
}

/// Identity attached to `/v1/*` proxy requests after the gateway key
/// middleware authenticates the caller.
#[derive(Debug, Clone)]
pub struct GatewayKeyPrincipal {
    pub key_id: String,
    pub project_id: String,
}

fn extract_bearer(parts: &Parts) -> Option<String> {
    let h = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let s = h.to_str().ok()?;
    let rest = s.strip_prefix("Bearer ")?.trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

#[async_trait]
impl FromRequestParts<AppState> for AdminPrincipal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_bearer(parts).ok_or(ApiError::Unauthorized)?;

        if let Some(root) = state.admin_root_token.as_deref() {
            if token.as_bytes().ct_eq(root.as_bytes()).into() {
                return Ok(AdminPrincipal::Root);
            }
        }

        match state.admin_signer.verify(&token) {
            Ok(claims) => Ok(AdminPrincipal::User {
                id: claims.sub,
                username: claims.username,
            }),
            Err(_) => Err(ApiError::Unauthorized),
        }
    }
}

#[async_trait]
impl FromRequestParts<AppState> for GatewayKeyPrincipal {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_bearer(parts).ok_or(ApiError::Unauthorized)?;
        let _prefix = parse_gateway_key(&token).ok_or(ApiError::Unauthorized)?;

        let hash = derive_hash(&state.master_key, &token);
        let row = state
            .stores
            .metadata
            .find_key_by_hash(&hash)
            .await
            .map_err(ApiError::Storage)?
            .ok_or(ApiError::Unauthorized)?;

        if row.status != "active" {
            return Err(ApiError::Unauthorized);
        }
        let now = Utc::now().timestamp_millis();
        if let Some(exp) = row.expires_at {
            if exp <= now {
                return Err(ApiError::Unauthorized);
            }
        }

        // Fire-and-forget last_used touch. We swallow errors so a busy
        // SQLite writer never breaks the proxy hot path.
        let stores = state.stores.metadata.clone();
        let id = row.id.clone();
        tokio::spawn(async move {
            let _ = stores.touch_key_last_used(&id, now).await;
        });

        Ok(GatewayKeyPrincipal {
            key_id: row.id,
            project_id: row.project_id,
        })
    }
}
