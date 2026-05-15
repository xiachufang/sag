use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use crate::error::Result;
use crate::models::*;

#[async_trait]
pub trait MetadataStore: Send + Sync {
    async fn create_project(&self, p: NewProject) -> Result<Project>;
    async fn get_project(&self, id: &str) -> Result<Option<Project>>;
    async fn list_projects(&self) -> Result<Vec<Project>>;

    async fn create_key(&self, k: NewGatewayKey) -> Result<GatewayKeyRow>;
    async fn list_keys(&self, project_id: &str) -> Result<Vec<GatewayKeyRow>>;
    async fn find_key_by_hash(&self, hash: &[u8]) -> Result<Option<GatewayKeyRow>>;
    async fn revoke_key(&self, id: &str) -> Result<()>;
    async fn touch_key_last_used(&self, id: &str, ts: Timestamp) -> Result<()>;

    async fn put_provider_credential(&self, c: ProviderCredential) -> Result<()>;
    async fn list_provider_credentials(&self, project_id: &str) -> Result<Vec<ProviderCredential>>;
    async fn delete_provider_credential(&self, id: &str) -> Result<()>;

    async fn upsert_routes(&self, project_id: &str, cfg: RoutesConfig, version: i64) -> Result<()>;
    async fn load_routes(&self, project_id: &str) -> Result<Option<(RoutesConfig, i64)>>;

    async fn upsert_budget(&self, b: Budget) -> Result<()>;
    async fn list_budgets(&self) -> Result<Vec<Budget>>;
    async fn get_budget(&self, id: &str) -> Result<Option<Budget>>;

    async fn create_admin_user(&self, u: NewAdminUser) -> Result<AdminUser>;
    async fn find_admin_user(&self, username: &str) -> Result<Option<AdminUser>>;
    async fn list_admin_users(&self) -> Result<Vec<AdminUser>>;
    async fn touch_admin_last_login(&self, id: &str, ts: Timestamp) -> Result<()>;
}

#[async_trait]
pub trait LogStore: Send + Sync {
    /// Append a single record. Implementations are expected to internally batch.
    async fn append(&self, rec: RequestLogRecord) -> Result<()>;

    async fn query(&self, q: LogQuery) -> Result<Page<RequestLogRow>>;
    async fn get_by_id(&self, id: &str) -> Result<Option<RequestLogDetail>>;
    async fn aggregate(&self, q: AggregateQuery) -> Result<AggregateResult>;
    async fn purge_older_than(&self, ts: Timestamp) -> Result<u64>;

    /// Flush any buffered records. Called on shutdown.
    async fn flush(&self) -> Result<()>;
}

#[async_trait]
pub trait KvStore: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<bytes::Bytes>>;
    async fn put(&self, key: &str, value: bytes::Bytes, ttl: Duration) -> Result<()>;
    async fn delete(&self, key: &str) -> Result<()>;
}

/// Permit for a concurrency slot. Dropping the permit releases the slot.
pub struct ConcurrencyPermit {
    release: Option<Box<dyn FnOnce() + Send + Sync>>,
}

impl ConcurrencyPermit {
    pub fn new(release: impl FnOnce() + Send + Sync + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        if let Some(r) = self.release.take() {
            r();
        }
    }
}

#[async_trait]
pub trait CounterStore: Send + Sync {
    async fn incr_window(&self, key: &str, window_ms: u64, by: i64) -> Result<i64>;
    async fn current(&self, key: &str, window_ms: u64) -> Result<i64>;

    /// Attempt to grab a concurrency slot. Returns None if at max.
    async fn try_acquire_concurrency(
        &self,
        key: &str,
        max: u32,
    ) -> Result<Option<ConcurrencyPermit>>;

    async fn incr_budget(
        &self,
        budget_id: &str,
        period_start: Timestamp,
        delta: f64,
    ) -> Result<f64>;
    async fn read_budget(&self, budget_id: &str, period_start: Timestamp) -> Result<f64>;
}

/// Bundle of all stores used by the gateway. Different profiles construct
/// this differently (SQLite + in-memory for Lite, Postgres + Redis for Standard,
/// pure memory for Memory).
#[derive(Clone)]
pub struct StoreBundle {
    pub metadata: Arc<dyn MetadataStore>,
    pub logs: Arc<dyn LogStore>,
    pub kv: Arc<dyn KvStore>,
    pub counter: Arc<dyn CounterStore>,
}
