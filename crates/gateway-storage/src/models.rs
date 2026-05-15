use serde::{Deserialize, Serialize};

pub type Timestamp = i64; // unix ms

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone)]
pub struct NewProject {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayKeyRow {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub prefix: String,
    pub hash: Vec<u8>,
    pub last4: String,
    pub scopes: Vec<String>,
    pub status: String, // active|revoked|expired
    pub expires_at: Option<Timestamp>,
    pub last_used_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub revoked_at: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct NewGatewayKey {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub prefix: String,
    pub hash: Vec<u8>,
    pub last4: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCredential {
    pub id: String,
    pub project_id: String,
    pub provider: String,
    pub name: String,
    pub encrypted_key: Vec<u8>,
    pub status: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutesConfig {
    /// Raw routes/limits/budgets/providers payload. We keep it as a JSON blob
    /// so the storage layer doesn't need to evolve with feature additions.
    #[serde(flatten)]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogRecord {
    pub id: String,
    pub project_id: String,
    pub gateway_key_id: Option<String>,
    pub namespace: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub request_ts: Timestamp,
    pub duration_ms: Option<i64>,
    pub upstream_ms: Option<i64>,
    pub ttfb_ms: Option<i64>,
    pub status: String, // success | upstream_error | gateway_error | timeout | cancelled
    pub http_status: Option<i32>,
    pub cached: bool,
    pub retry_count: i32,
    pub fallback_used: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub would_have_cost_usd: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    pub error_message: Option<String>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogRow {
    pub id: String,
    pub project_id: String,
    pub gateway_key_id: Option<String>,
    pub namespace: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub request_ts: Timestamp,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub http_status: Option<i32>,
    pub cached: bool,
    pub retry_count: i32,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogDetail {
    #[serde(flatten)]
    pub record: RequestLogRecord,
}

#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub project_id: Option<String>,
    pub gateway_key_id: Option<String>,
    pub namespace: Option<String>,
    pub model: Option<String>,
    pub status: Option<String>,
    pub from_ts: Option<Timestamp>,
    pub to_ts: Option<Timestamp>,
    pub limit: u32,
    pub cursor: Option<String>, // request_ts-id encoded cursor
}

impl LogQuery {
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = limit.clamp(1, 500);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AggregateQuery {
    pub project_id: Option<String>,
    pub from_ts: Option<Timestamp>,
    pub to_ts: Option<Timestamp>,
    pub group_by: Vec<AggregateDimension>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateDimension {
    Namespace,
    Model,
    Day,
    Hour,
    GatewayKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateResult {
    pub groups: Vec<AggregateGroup>,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateGroup {
    pub key: serde_json::Value, // map of dim -> value
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cost_usd: f64,
    pub cached_savings_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUser {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub created_at: Timestamp,
    pub last_login_at: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct NewAdminUser {
    pub id: String,
    pub username: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub id: String,
    pub name: String,
    pub target_type: String,
    pub target_id: String,
    pub period: String, // daily|weekly|monthly|custom
    pub amount_usd: f64,
    pub thresholds: serde_json::Value, // [{at, action, webhook}]
    pub status: String,
}
