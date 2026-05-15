use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{GatewayError, Result};

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
    /// Name of the env var that holds the root admin token. The gateway
    /// will read it at startup; if empty, admin API is disabled.
    #[serde(default = "default_root_token_env")]
    pub root_token_env: String,
    #[serde(default)]
    pub password_login: bool,
}

fn default_root_token_env() -> String {
    "GATEWAY_ROOT_TOKEN".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    pub base_url: String,
    /// `env://VAR_NAME` to read directly from env, or `secret://<credential-id>`
    /// to look up an encrypted credential from the metadata store.
    pub credential_ref: String,
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
    #[serde(default)]
    pub path: Option<String>,
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
        for route in &self.routes {
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
        Ok(())
    }

    pub fn request_timeout(&self) -> Duration {
        Duration::from_millis(self.server.request_timeout_ms)
    }
}
