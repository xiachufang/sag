use std::time::Duration;

use http::HeaderMap;
use serde_json::Value;

/// Per-request cache directive parsed from the `X-Gateway-Cache` and
/// related headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheDirective {
    /// Normal flow: read from cache, write on miss.
    Default,
    /// Skip both read and write.
    Bypass,
    /// Skip read; still write on success.
    Refresh,
    /// Read only; on miss return 504 to the caller without contacting upstream.
    OnlyIfCached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    Hit,
    Miss,
    Bypass,
    Refresh,
    Disabled,
}

impl CacheStatus {
    pub fn as_header(self) -> &'static str {
        match self {
            CacheStatus::Hit => "hit",
            CacheStatus::Miss => "miss",
            CacheStatus::Bypass => "bypass",
            CacheStatus::Refresh => "refresh",
            CacheStatus::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CachePolicy {
    pub directive: CacheDirective,
    pub ttl: Duration,
    /// Optional user-supplied cache-key bumper (header `X-Gateway-Cache-Scope`),
    /// distinct from the URL namespace used to scope the route itself.
    pub cache_scope: Option<String>,
    pub allow_nondeterministic: bool, // route config OR X-Gateway-Cache-Force
}

impl CachePolicy {
    /// Build a per-request policy by merging route defaults with client-supplied
    /// headers. Returns `None` when caching is disabled for the route entirely.
    pub fn from_headers(
        headers: &HeaderMap,
        route_enabled: bool,
        default_ttl: Duration,
        route_allow_nondeterministic: bool,
    ) -> Option<Self> {
        if !route_enabled {
            return None;
        }
        let directive = match headers
            .get("x-gateway-cache")
            .and_then(|v| v.to_str().ok())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("bypass") => CacheDirective::Bypass,
            Some("refresh") => CacheDirective::Refresh,
            Some("only") | Some("only-if-cached") => CacheDirective::OnlyIfCached,
            _ => CacheDirective::Default,
        };
        let ttl = headers
            .get("x-gateway-cache-ttl")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(default_ttl);
        let cache_scope = headers
            .get("x-gateway-cache-scope")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let header_force = headers
            .get("x-gateway-cache-force")
            .and_then(|v| v.to_str().ok())
            .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        let allow_nondeterministic = route_allow_nondeterministic || header_force;
        Some(Self {
            directive,
            ttl,
            cache_scope,
            allow_nondeterministic,
        })
    }

    /// Reject caching for non-deterministic bodies unless the caller
    /// explicitly opted in via `X-Gateway-Cache-Force`.
    pub fn body_is_cacheable(&self, body: Option<&Value>) -> bool {
        let Some(body) = body else {
            return false;
        };
        if self.allow_nondeterministic {
            return true;
        }
        let temperature = body
            .get("temperature")
            .and_then(|t| t.as_f64())
            .unwrap_or(0.0);
        let top_p = body.get("top_p").and_then(|t| t.as_f64()).unwrap_or(1.0);
        temperature == 0.0 && top_p >= 0.999
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allow_nondeterministic: bool) -> CachePolicy {
        CachePolicy {
            directive: CacheDirective::Default,
            ttl: Duration::from_secs(60),
            cache_scope: None,
            allow_nondeterministic,
        }
    }

    #[test]
    fn preserves_existing_temperature_defaults() {
        let p = policy(false);
        assert!(p.body_is_cacheable(Some(&serde_json::json!({"input": "hello"}))));
        assert!(p.body_is_cacheable(Some(
            &serde_json::json!({"input": "hello", "temperature": 0})
        )));
        assert!(!p.body_is_cacheable(Some(
            &serde_json::json!({"input": "hello", "temperature": 0.5})
        )));
    }

    #[test]
    fn override_still_requires_valid_json() {
        let p = policy(true);
        assert!(p.body_is_cacheable(Some(&serde_json::json!({"input": "hello"}))));
        assert!(!p.body_is_cacheable(None));
    }
}
