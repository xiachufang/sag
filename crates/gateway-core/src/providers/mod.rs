use std::collections::HashMap;
use std::sync::Arc;

use gateway_storage::traits::MetadataStore;

use crate::config::ProviderConfig;
use crate::error::{GatewayError, Result};
use crate::security::{decrypt_credential, MasterKey};

pub mod anthropic;
pub mod openai;

/// Adapter that knows how to authenticate against a specific upstream
/// provider. The pass-through code path uses [`AuthInjector::inject`] to
/// rewrite the request before forwarding.
pub trait AuthInjector: Send + Sync {
    fn inject(&self, headers: &mut http::HeaderMap, api_key: &str);
}

pub fn build_auth_injector(kind: &str) -> Result<Box<dyn AuthInjector>> {
    match kind {
        "openai" | "deepseek" | "openai-compatible" => Ok(Box::new(openai::OpenAiAuth)),
        "anthropic" => Ok(Box::new(anthropic::AnthropicAuth)),
        other => Err(GatewayError::ProviderUnknown(other.into())),
    }
}

/// Used by `AppConfig::validate` to fail-fast on typos in the YAML
/// `providers.<x>.kind` field. Must stay in sync with the match in
/// `build_auth_injector`.
pub fn is_known_provider_kind(kind: &str) -> bool {
    matches!(
        kind,
        "openai" | "deepseek" | "openai-compatible" | "anthropic"
    )
}

/// Resolve a provider's API key. Supports `env://VAR` (env lookup) and
/// `secret://<credential-id>` (encrypted DB lookup, decrypted with the
/// master key).
pub async fn resolve_credential(
    cfg: &ProviderConfig,
    env_overrides: &HashMap<String, String>,
    metadata: Option<&Arc<dyn MetadataStore>>,
    master: Option<&MasterKey>,
    project_id: &str,
) -> Result<String> {
    let r = &cfg.credential_ref;
    if let Some(rest) = r.strip_prefix("env://") {
        env_overrides
            .get(rest)
            .cloned()
            .or_else(|| std::env::var(rest).ok())
            .ok_or_else(|| {
                GatewayError::Internal(format!("env var {rest} not set for provider credential"))
            })
    } else if let Some(id) = r.strip_prefix("secret://") {
        let metadata = metadata
            .ok_or_else(|| GatewayError::Internal("secret:// requires metadata store".into()))?;
        let master =
            master.ok_or_else(|| GatewayError::Internal("secret:// requires master key".into()))?;
        let creds = metadata
            .list_provider_credentials(project_id)
            .await
            .map_err(|e| GatewayError::Storage(e))?;
        let row = creds
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| GatewayError::Internal(format!("credential {id} not found")))?;
        decrypt_credential(master, &row.encrypted_key)
    } else {
        Err(GatewayError::Internal(format!(
            "unsupported credential_ref: {r}"
        )))
    }
}
