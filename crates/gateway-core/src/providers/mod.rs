use std::collections::HashMap;

use crate::config::ProviderConfig;
use crate::error::{GatewayError, Result};

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
        "openai" => Ok(Box::new(openai::OpenAiAuth)),
        "anthropic" => Ok(Box::new(anthropic::AnthropicAuth)),
        other => Err(GatewayError::ProviderUnknown(other.into())),
    }
}

/// Used by `AppConfig::validate` to fail-fast on typos in the YAML
/// `providers.<x>.kind` field. Must stay in sync with the match in
/// `build_auth_injector`.
pub fn is_known_provider_kind(kind: &str) -> bool {
    matches!(kind, "openai" | "anthropic")
}

/// Resolve a provider's API key. Accepts `env://VAR_NAME` (env lookup,
/// with optional in-process overrides for tests) or any other literal
/// string, which is used as the token verbatim.
pub fn resolve_credential(
    cfg: &ProviderConfig,
    env_overrides: &HashMap<String, String>,
) -> Result<String> {
    let r = &cfg.credential;
    if let Some(rest) = r.strip_prefix("env://") {
        if rest.is_empty() {
            return Err(GatewayError::Internal(
                "provider credential env:// reference is missing a var name".into(),
            ));
        }
        env_overrides
            .get(rest)
            .cloned()
            .or_else(|| std::env::var(rest).ok())
            .ok_or_else(|| {
                GatewayError::Internal(format!("env var {rest} not set for provider credential"))
            })
    } else if r.is_empty() {
        Err(GatewayError::Internal(
            "provider credential is empty".into(),
        ))
    } else {
        Ok(r.clone())
    }
}
