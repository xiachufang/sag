use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{GatewayError, Result};
use crate::providers::is_known_provider_kind;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    pub storage: StorageConfig,
    #[serde(default)]
    pub admin: AdminConfig,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub limits: Vec<LimitConfig>,
    #[serde(default)]
    pub budgets: Vec<BudgetConfig>,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    /// Gateway keys to seed into the metadata store at boot and on every
    /// successful hot reload. These are managed declaratively — the Admin
    /// API refuses to revoke or modify them.
    #[serde(default)]
    pub gateway_keys: Vec<GatewayKeyConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_project_id")]
    pub default_project_id: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            request_timeout_ms: default_request_timeout_ms(),
            default_project_id: default_project_id(),
        }
    }
}

fn default_bind() -> String {
    "0.0.0.0:8080".into()
}
fn default_request_timeout_ms() -> u64 {
    600_000
}
fn default_project_id() -> String {
    "default".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "profile", rename_all = "lowercase")]
pub enum StorageConfig {
    Lite {
        #[serde(default)]
        sqlite: SqliteConfig,
        #[serde(default)]
        cache: CacheConfig,
    },
    Standard {
        postgres: PostgresConfig,
        redis: RedisConfig,
        #[serde(default)]
        cache: CacheConfig,
    },
    Memory {
        #[serde(default)]
        cache: CacheConfig,
    },
}

impl StorageConfig {
    pub fn profile_name(&self) -> &'static str {
        match self {
            StorageConfig::Lite { .. } => "lite",
            StorageConfig::Standard { .. } => "standard",
            StorageConfig::Memory { .. } => "memory",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SqliteConfig {
    #[serde(default = "default_sqlite_path")]
    pub path: PathBuf,
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u64,
    #[serde(default = "default_log_retention_days")]
    pub log_retention_days: u32,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            path: default_sqlite_path(),
            max_size_mb: default_max_size_mb(),
            log_retention_days: default_log_retention_days(),
        }
    }
}

fn default_sqlite_path() -> PathBuf {
    PathBuf::from("./data/gateway.db")
}
fn default_max_size_mb() -> u64 {
    10240
}
fn default_log_retention_days() -> u32 {
    30
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostgresConfig {
    pub url: String,
    #[serde(default = "default_pg_max_connections")]
    pub max_connections: u32,
}

fn default_pg_max_connections() -> u32 {
    32
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RedisConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheConfig {
    #[serde(default = "default_l1_memory_mb")]
    pub l1_memory_mb: u64,
    #[serde(default = "default_l2_max_size_mb")]
    pub l2_max_size_mb: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            l1_memory_mb: default_l1_memory_mb(),
            l2_max_size_mb: default_l2_max_size_mb(),
        }
    }
}

fn default_l1_memory_mb() -> u64 {
    256
}
fn default_l2_max_size_mb() -> u64 {
    1024
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AdminConfig {
    /// Source for the root admin token. Accepts:
    /// - `""` (empty) → Admin API disabled.
    /// - `env://VAR_NAME` → read from the named env var; empty/unset
    ///   disables Admin API.
    /// - anything else → literal token. Same threat model as inline
    ///   `gateway_keys[].secret`: convenient for local dev, but the YAML
    ///   file becomes a secret-bearing artifact.
    #[serde(default = "default_root_token")]
    pub root_token: String,
    #[serde(default)]
    pub password_login: bool,
}

fn default_root_token() -> String {
    "env://GATEWAY_ROOT_TOKEN".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    /// Auth adapter to use. Currently recognised: `openai`, `anthropic`.
    /// Use `openai` for any upstream that follows the OpenAI wire
    /// protocol (the real api.openai.com, doubao, DeepSeek, Groq,
    /// Together, vLLM, Ollama, …). When unset, the gateway falls back
    /// to the providers-map key, which preserves backwards compatibility
    /// with the convention `providers.openai: { ... }`.
    #[serde(default)]
    pub kind: Option<String>,
    pub base_url: String,
    /// Upstream API key. Either `env://VAR_NAME` (read from env at boot)
    /// or a literal token. Inline literals share the same threat model as
    /// inline `gateway_keys[].secret` — fine for local dev, prefer env
    /// references in production where the YAML may be reviewed broadly.
    pub credential: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteConfig {
    #[serde(rename = "match", default)]
    pub match_: RouteMatch,
    pub primary: RouteTarget,
    #[serde(default)]
    pub cache: RouteCacheConfig,
    #[serde(default)]
    pub retry: RouteRetryConfig,
    #[serde(default)]
    pub fallbacks: Vec<RouteTarget>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RouteMatch {
    /// URL namespace — the `{namespace}` segment in `/v1/{namespace}/...`.
    /// Conceptually distinct from `primary.provider` (which names an entry
    /// in the `providers` map). If unset, defaults to `primary.provider`
    /// so existing configs that didn't decouple the two keep working.
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub model_prefix: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteTarget {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub trigger: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RouteCacheConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cache_ttl")]
    pub ttl: u64,
    /// When true, cache requests even when sampling parameters indicate
    /// non-determinism (temperature != 0 or top_p < 0.999). The per-request
    /// `X-Gateway-Cache-Force` header is ORed on top of this.
    #[serde(default)]
    pub allow_nondeterministic: bool,
}

fn default_cache_ttl() -> u64 {
    3600
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RouteRetryConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
}

impl Default for RouteRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            initial_backoff_ms: default_initial_backoff_ms(),
        }
    }
}

fn default_max_attempts() -> u32 {
    3
}
fn default_initial_backoff_ms() -> u64 {
    500
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LimitConfig {
    pub target: LimitTarget,
    #[serde(default)]
    pub rpm: Option<u64>,
    #[serde(default)]
    pub tpm: Option<u64>,
    #[serde(default)]
    pub concurrency: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LimitTarget {
    #[serde(rename = "type")]
    pub kind: String, // key | project | metadata | global
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub key: Option<String>, // for metadata
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BudgetConfig {
    pub name: String,
    pub target: BudgetTarget,
    pub period: String,
    pub amount_usd: f64,
    #[serde(default)]
    pub thresholds: Vec<BudgetThreshold>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BudgetTarget {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub gateway_key_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BudgetThreshold {
    pub at: f64,
    pub action: String, // notify | block
    #[serde(default)]
    pub webhook: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ObservabilityConfig {
    #[serde(default = "default_true")]
    pub metrics: bool,
    #[serde(default)]
    pub tracing: TracingConfig,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            metrics: true,
            tracing: TracingConfig::default(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// A gateway API key declared in the config file. The secret can be a
/// literal `sk-gw-{live,test}-...` string or `env://VAR_NAME` to read it
/// from the environment at apply time.
#[derive(Clone, Deserialize, Serialize)]
pub struct GatewayKeyConfig {
    /// Stable identifier — survives across reloads. Used as the row id
    /// in `gateway_keys`, so logs/limits/budgets stay attached if the
    /// secret rotates.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub project_id: Option<String>,
    pub secret: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl GatewayKeyConfig {
    /// Resolve `secret` to the actual plaintext. `env://VAR` looks up the
    /// env var; anything else is returned verbatim.
    pub fn resolve_secret(&self) -> std::result::Result<String, String> {
        if let Some(var) = self.secret.strip_prefix("env://") {
            if var.is_empty() {
                return Err("env:// reference is missing a var name".into());
            }
            std::env::var(var).map_err(|_| format!("env var '{var}' not set or empty"))
        } else if self.secret.is_empty() {
            Err("secret is empty".into())
        } else {
            Ok(self.secret.clone())
        }
    }
}

impl fmt::Debug for GatewayKeyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Keep plaintext secrets out of debug output (and therefore out of
        // any structured log that happens to render the full AppConfig).
        let secret_repr = if self.secret.starts_with("env://") {
            self.secret.as_str()
        } else {
            "<redacted>"
        };
        f.debug_struct("GatewayKeyConfig")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("project_id", &self.project_id)
            .field("secret", &secret_repr)
            .field("scopes", &self.scopes)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TracingConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_tracing_format")]
    pub format: String, // json | text
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            format: default_tracing_format(),
            otlp_endpoint: None,
        }
    }
}

fn default_tracing_format() -> String {
    "json".into()
}

impl AppConfig {
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            GatewayError::Internal(format!("failed to read config {}: {e}", path.display()))
        })?;
        Self::load_from_str(&text)
    }

    pub fn load_from_str(text: &str) -> Result<Self> {
        let cfg: AppConfig = serde_yaml::from_str(text)
            .map_err(|e| GatewayError::Internal(format!("failed to parse config yaml: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        for (name, p) in &self.providers {
            let kind = p.kind.as_deref().unwrap_or(name);
            if !is_known_provider_kind(kind) {
                return Err(GatewayError::BadRequest(format!(
                    "provider '{name}' has unsupported kind '{kind}'; supported: openai, anthropic"
                )));
            }
        }
        for route in &self.routes {
            if route
                .match_
                .namespace
                .as_deref()
                .map(str::is_empty)
                .unwrap_or(true)
            {
                return Err(GatewayError::BadRequest(format!(
                    "route for primary.provider '{}' must declare match.namespace",
                    route.primary.provider
                )));
            }
            if !self.providers.contains_key(&route.primary.provider) {
                return Err(GatewayError::BadRequest(format!(
                    "route references unknown provider: {}",
                    route.primary.provider
                )));
            }
            for fb in &route.fallbacks {
                if !self.providers.contains_key(&fb.provider) {
                    return Err(GatewayError::BadRequest(format!(
                        "fallback references unknown provider: {}",
                        fb.provider
                    )));
                }
            }
        }
        let mut seen_ids: HashSet<&str> = HashSet::new();
        for k in &self.gateway_keys {
            if k.id.trim().is_empty() {
                return Err(GatewayError::BadRequest(
                    "gateway_keys[].id must be non-empty".into(),
                ));
            }
            if !seen_ids.insert(k.id.as_str()) {
                return Err(GatewayError::BadRequest(format!(
                    "gateway_keys[].id '{}' is declared more than once",
                    k.id
                )));
            }
            if k.name.trim().is_empty() {
                return Err(GatewayError::BadRequest(format!(
                    "gateway_keys[id={}].name must be non-empty",
                    k.id
                )));
            }
            if k.secret.is_empty() {
                return Err(GatewayError::BadRequest(format!(
                    "gateway_keys[id={}].secret must be non-empty",
                    k.id
                )));
            }
            // For inline plaintext we can check the wire format up front.
            // env:// references are checked when the var is resolved (at
            // apply time), so a temporarily-unset var doesn't break load.
            if !k.secret.starts_with("env://") {
                let prefix_ok =
                    k.secret.starts_with("sk-gw-live-") || k.secret.starts_with("sk-gw-test-");
                if !prefix_ok {
                    return Err(GatewayError::BadRequest(format!(
                        "gateway_keys[id={}].secret must start with 'sk-gw-live-' or 'sk-gw-test-' (or use env://VAR)",
                        k.id
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_millis(self.server.request_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_kind_defaults_to_map_key() {
        let yaml = r#"
storage:
  profile: memory
providers:
  openai:
    base_url: https://api.openai.com
    credential: env://X
"#;
        let cfg = AppConfig::load_from_str(yaml).expect("valid");
        // No explicit kind — falls back to the providers-map key.
        assert!(cfg.providers["openai"].kind.is_none());
    }

    #[test]
    fn explicit_kind_decouples_name_from_adapter() {
        let yaml = r#"
storage:
  profile: memory
providers:
  doubao:
    kind: openai
    base_url: https://ark.example
    credential: env://X
"#;
        let cfg = AppConfig::load_from_str(yaml).expect("valid");
        assert_eq!(cfg.providers["doubao"].kind.as_deref(), Some("openai"));
    }

    #[test]
    fn unknown_kind_is_rejected_at_load() {
        let yaml = r#"
storage:
  profile: memory
providers:
  weird:
    kind: gemini-but-typo
    base_url: https://example
    credential: env://X
"#;
        let err = AppConfig::load_from_str(yaml).expect_err("should fail");
        let msg = format!("{err}");
        assert!(msg.contains("unsupported kind"), "got: {msg}");
    }

    #[test]
    fn unknown_kind_via_implicit_name_also_rejected() {
        // No explicit kind, and the map key isn't a recognised adapter.
        let yaml = r#"
storage:
  profile: memory
providers:
  doubao:
    base_url: https://example
    credential: env://X
"#;
        let err = AppConfig::load_from_str(yaml).expect_err("should fail");
        let msg = format!("{err}");
        assert!(msg.contains("unsupported kind"), "got: {msg}");
    }
}
