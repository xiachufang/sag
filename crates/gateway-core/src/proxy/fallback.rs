use serde::Serialize;

use crate::config::{RouteConfig, RouteRetryConfig, RouteTarget};

/// Resolved chain of providers to try in order — primary first, then
/// each configured fallback. Each element keeps the original trigger so
/// the executor knows whether the previous attempt's outcome qualifies.
#[derive(Debug, Clone)]
pub struct ProviderChain {
    pub entries: Vec<ProviderChainEntry>,
    pub retry: RouteRetryConfig,
}

#[derive(Debug, Clone)]
pub struct ProviderChainEntry {
    pub provider: String,
    pub model_override: Option<String>,
    pub trigger: Vec<String>,
}

impl ProviderChainEntry {
    fn from_target(t: &RouteTarget) -> Self {
        Self {
            provider: t.provider.clone(),
            model_override: t.model.clone(),
            trigger: t.trigger.clone(),
        }
    }
}

impl ProviderChain {
    pub fn primary_only(provider: &str) -> Self {
        Self {
            entries: vec![ProviderChainEntry {
                provider: provider.to_string(),
                model_override: None,
                trigger: vec![],
            }],
            retry: RouteRetryConfig::default(),
        }
    }

    pub fn from_route(route: &RouteConfig) -> Self {
        let mut entries = Vec::with_capacity(route.fallbacks.len() + 1);
        entries.push(ProviderChainEntry::from_target(&route.primary));
        for fb in &route.fallbacks {
            entries.push(ProviderChainEntry::from_target(fb));
        }
        Self {
            entries,
            retry: route.retry.clone(),
        }
    }
}

/// Trace entry for a single provider attempt, surfaced in
/// `request_logs.metadata.attempts`.
#[derive(Debug, Clone, Serialize)]
pub struct AttemptTrace {
    pub provider: String,
    pub model: Option<String>,
    pub status: Option<u16>,
    pub outcome: &'static str,
    pub duration_ms: u64,
}
