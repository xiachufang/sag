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
    async fn get_key(&self, id: &str) -> Result<Option<GatewayKeyRow>>;
    async fn find_key_by_hash(&self, hash: &[u8]) -> Result<Option<GatewayKeyRow>>;
    async fn revoke_key(&self, id: &str) -> Result<()>;
    async fn touch_key_last_used(&self, id: &str, ts: Timestamp) -> Result<()>;

    /// Insert or update a config-seeded key. Returns an error if a row with
    /// the same id already exists with a non-`config` origin.
    async fn upsert_seeded_key(&self, k: NewGatewayKey) -> Result<GatewayKeyRow>;

    /// Delete config-seeded keys in the given project whose id is not in
    /// `keep_ids`. Returns the number of pruned rows.
    async fn prune_seeded_keys_not_in(&self, project_id: &str, keep_ids: &[String]) -> Result<u64>;

    async fn upsert_routes(&self, project_id: &str, cfg: RoutesConfig, version: i64) -> Result<()>;
    async fn load_routes(&self, project_id: &str) -> Result<Option<(RoutesConfig, i64)>>;

    async fn upsert_budget(&self, b: Budget) -> Result<()>;
    async fn list_budgets(&self) -> Result<Vec<Budget>>;
    async fn get_budget(&self, id: &str) -> Result<Option<Budget>>;

    /// Admin-set model price overrides (take precedence over the catalog file).
    async fn upsert_pricing(&self, p: PricingRow) -> Result<()>;
    async fn list_pricing(&self) -> Result<Vec<PricingRow>>;
    async fn delete_pricing(&self, provider: &str, model: &str) -> Result<()>;

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

    /// Distinct (provider, model) pairs among logs that carry token counts,
    /// where provider is `COALESCE(fallback_used, namespace)` — the same
    /// resolution used when the cost was originally computed. Supports the
    /// admin cost-recompute flow.
    async fn distinct_cost_keys(
        &self,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
    ) -> Result<Vec<(String, String)>>;

    /// Re-derive `cost_usd` / `would_have_cost_usd` from stored token counts
    /// for all rows matching (provider, model) using the given per-1K rates.
    /// Cached rows keep `cost_usd = 0`. Returns the number of rows updated.
    #[allow(clippy::too_many_arguments)]
    async fn recompute_costs(
        &self,
        provider: &str,
        model: &str,
        input_per_1k: f64,
        cached_input_per_1k: f64,
        output_per_1k: f64,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
    ) -> Result<u64>;

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
