//! Pure-in-memory stores. Useful for unit tests, ephemeral demos, and CI
//! environments where you don't want any filesystem state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;

use crate::error::{Result, StorageError};
use crate::models::*;
use crate::traits::{ConcurrencyPermit, CounterStore, KvStore, LogStore, MetadataStore};

pub struct MemoryMetadataStore {
    state: Mutex<MemMeta>,
}

#[derive(Default)]
struct MemMeta {
    projects: HashMap<String, Project>,
    keys: HashMap<String, GatewayKeyRow>,
    routes: HashMap<String, (RoutesConfig, i64)>,
    budgets: HashMap<String, Budget>,
    admins: HashMap<String, AdminUser>,
    pricing: HashMap<(String, String), PricingRow>,
}

impl Default for MemoryMetadataStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryMetadataStore {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemMeta::default()),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[async_trait]
impl MetadataStore for MemoryMetadataStore {
    async fn create_project(&self, p: NewProject) -> Result<Project> {
        let project = Project {
            id: p.id.clone(),
            name: p.name,
            created_at: now_ms(),
        };
        self.state
            .lock()
            .unwrap()
            .projects
            .insert(p.id, project.clone());
        Ok(project)
    }

    async fn get_project(&self, id: &str) -> Result<Option<Project>> {
        Ok(self.state.lock().unwrap().projects.get(id).cloned())
    }

    async fn list_projects(&self) -> Result<Vec<Project>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .projects
            .values()
            .cloned()
            .collect())
    }

    async fn create_key(&self, k: NewGatewayKey) -> Result<GatewayKeyRow> {
        let row = GatewayKeyRow {
            id: k.id.clone(),
            project_id: k.project_id,
            name: k.name,
            prefix: k.prefix,
            hash: k.hash,
            last4: k.last4,
            scopes: k.scopes,
            status: "active".into(),
            expires_at: k.expires_at,
            last_used_at: None,
            created_at: now_ms(),
            revoked_at: None,
            origin: k.origin,
        };
        self.state.lock().unwrap().keys.insert(k.id, row.clone());
        Ok(row)
    }

    async fn list_keys(&self, project_id: &str) -> Result<Vec<GatewayKeyRow>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .keys
            .values()
            .filter(|k| k.project_id == project_id)
            .cloned()
            .collect())
    }

    async fn get_key(&self, id: &str) -> Result<Option<GatewayKeyRow>> {
        Ok(self.state.lock().unwrap().keys.get(id).cloned())
    }

    async fn find_key_by_hash(&self, hash: &[u8]) -> Result<Option<GatewayKeyRow>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .keys
            .values()
            .find(|k| k.hash == hash)
            .cloned())
    }

    async fn revoke_key(&self, id: &str) -> Result<()> {
        let mut g = self.state.lock().unwrap();
        match g.keys.get_mut(id) {
            Some(k) => {
                k.status = "revoked".into();
                k.revoked_at = Some(now_ms());
                Ok(())
            }
            None => Err(StorageError::NotFound),
        }
    }

    async fn touch_key_last_used(&self, id: &str, ts: Timestamp) -> Result<()> {
        if let Some(k) = self.state.lock().unwrap().keys.get_mut(id) {
            k.last_used_at = Some(ts);
        }
        Ok(())
    }

    async fn upsert_seeded_key(&self, k: NewGatewayKey) -> Result<GatewayKeyRow> {
        let mut g = self.state.lock().unwrap();
        if let Some(existing) = g.keys.get(&k.id) {
            if existing.origin != KEY_ORIGIN_CONFIG {
                return Err(StorageError::Conflict(format!(
                    "gateway key '{}' is admin-managed; rename the config-seeded key",
                    k.id
                )));
            }
        }
        let created_at = g
            .keys
            .get(&k.id)
            .map(|r| r.created_at)
            .unwrap_or_else(now_ms);
        let last_used_at = g.keys.get(&k.id).and_then(|r| r.last_used_at);
        let row = GatewayKeyRow {
            id: k.id.clone(),
            project_id: k.project_id,
            name: k.name,
            prefix: k.prefix,
            hash: k.hash,
            last4: k.last4,
            scopes: k.scopes,
            status: "active".into(),
            expires_at: k.expires_at,
            last_used_at,
            created_at,
            revoked_at: None,
            origin: KEY_ORIGIN_CONFIG.into(),
        };
        g.keys.insert(k.id, row.clone());
        Ok(row)
    }

    async fn prune_seeded_keys_not_in(&self, project_id: &str, keep_ids: &[String]) -> Result<u64> {
        let mut g = self.state.lock().unwrap();
        let to_remove: Vec<String> = g
            .keys
            .values()
            .filter(|k| {
                k.project_id == project_id
                    && k.origin == KEY_ORIGIN_CONFIG
                    && !keep_ids.iter().any(|id| id == &k.id)
            })
            .map(|k| k.id.clone())
            .collect();
        let removed = to_remove.len() as u64;
        for id in to_remove {
            g.keys.remove(&id);
        }
        Ok(removed)
    }

    async fn upsert_routes(&self, project_id: &str, cfg: RoutesConfig, version: i64) -> Result<()> {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(project_id.to_string(), (cfg, version));
        Ok(())
    }

    async fn load_routes(&self, project_id: &str) -> Result<Option<(RoutesConfig, i64)>> {
        Ok(self.state.lock().unwrap().routes.get(project_id).cloned())
    }

    async fn upsert_budget(&self, b: Budget) -> Result<()> {
        self.state.lock().unwrap().budgets.insert(b.id.clone(), b);
        Ok(())
    }

    async fn upsert_pricing(&self, p: PricingRow) -> Result<()> {
        self.state
            .lock()
            .unwrap()
            .pricing
            .insert((p.provider.clone(), p.model.clone()), p);
        Ok(())
    }

    async fn list_pricing(&self) -> Result<Vec<PricingRow>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .pricing
            .values()
            .cloned()
            .collect())
    }

    async fn delete_pricing(&self, provider: &str, model: &str) -> Result<()> {
        self.state
            .lock()
            .unwrap()
            .pricing
            .remove(&(provider.to_string(), model.to_string()));
        Ok(())
    }

    async fn list_budgets(&self) -> Result<Vec<Budget>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .budgets
            .values()
            .cloned()
            .collect())
    }

    async fn get_budget(&self, id: &str) -> Result<Option<Budget>> {
        Ok(self.state.lock().unwrap().budgets.get(id).cloned())
    }

    async fn create_admin_user(&self, u: NewAdminUser) -> Result<AdminUser> {
        let admin = AdminUser {
            id: u.id.clone(),
            username: u.username,
            password_hash: u.password_hash,
            created_at: now_ms(),
            last_login_at: None,
        };
        let mut g = self.state.lock().unwrap();
        if g.admins.values().any(|a| a.username == admin.username) {
            return Err(StorageError::Conflict(format!(
                "admin '{}' exists",
                admin.username
            )));
        }
        g.admins.insert(u.id, admin.clone());
        Ok(admin)
    }

    async fn find_admin_user(&self, username: &str) -> Result<Option<AdminUser>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .admins
            .values()
            .find(|a| a.username == username)
            .cloned())
    }

    async fn list_admin_users(&self) -> Result<Vec<AdminUser>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .admins
            .values()
            .cloned()
            .collect())
    }

    async fn touch_admin_last_login(&self, id: &str, ts: Timestamp) -> Result<()> {
        if let Some(a) = self.state.lock().unwrap().admins.get_mut(id) {
            a.last_login_at = Some(ts);
        }
        Ok(())
    }
}

/// Ring-buffer log store. Drops oldest record when capacity is reached.
pub struct MemoryLogStore {
    state: Mutex<Vec<RequestLogRecord>>,
    capacity: usize,
}

/// Provider used for pricing, mirroring the request-time resolution: the
/// fallback provider if one fired, else the provider of the last attempt in
/// the metadata trace (the real upstream — namespaces can be decoupled from
/// providers), else the namespace.
fn resolve_provider(r: &RequestLogRecord) -> Option<String> {
    r.fallback_used
        .clone()
        .or_else(|| {
            r.metadata
                .as_ref()
                .and_then(|m| m.get("attempts"))
                .and_then(|a| a.as_array())
                .and_then(|a| a.last())
                .and_then(|x| x.get("provider"))
                .and_then(|p| p.as_str())
                .map(str::to_string)
        })
        .or_else(|| r.namespace.clone())
}

impl MemoryLogStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(Vec::with_capacity(capacity)),
            capacity,
        }
    }
}

#[async_trait]
impl LogStore for MemoryLogStore {
    async fn append(&self, rec: RequestLogRecord) -> Result<()> {
        let mut g = self.state.lock().unwrap();
        if g.len() >= self.capacity {
            g.remove(0);
        }
        g.push(rec);
        Ok(())
    }

    async fn query(&self, q: LogQuery) -> Result<Page<RequestLogRow>> {
        let limit = if q.limit == 0 { 50 } else { q.limit.min(500) } as usize;
        let g = self.state.lock().unwrap();
        let mut rows: Vec<_> = g
            .iter()
            .filter(|r| {
                q.project_id.as_deref().map_or(true, |p| p == r.project_id)
                    && q.namespace
                        .as_deref()
                        .map_or(true, |p| r.namespace.as_deref() == Some(p))
                    && q.model
                        .as_deref()
                        .map_or(true, |m| r.model.as_deref() == Some(m))
                    && q.status.as_deref().map_or(true, |s| r.status == s)
                    && q.from_ts.map_or(true, |f| r.request_ts >= f)
                    && q.to_ts.map_or(true, |t| r.request_ts <= t)
            })
            .map(|r| RequestLogRow {
                id: r.id.clone(),
                project_id: r.project_id.clone(),
                gateway_key_id: r.gateway_key_id.clone(),
                namespace: r.namespace.clone(),
                model: r.model.clone(),
                endpoint: r.endpoint.clone(),
                request_ts: r.request_ts,
                duration_ms: r.duration_ms,
                status: r.status.clone(),
                http_status: r.http_status,
                cached: r.cached,
                retry_count: r.retry_count,
                prompt_tokens: r.prompt_tokens,
                completion_tokens: r.completion_tokens,
                cost_usd: r.cost_usd,
                request_body: r.request_body.clone(),
            })
            .collect();
        rows.sort_by(|a, b| b.request_ts.cmp(&a.request_ts));
        rows.truncate(limit);
        Ok(Page {
            items: rows,
            next_cursor: None,
        })
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<RequestLogDetail>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .map(|record| RequestLogDetail { record }))
    }

    async fn aggregate(&self, q: AggregateQuery) -> Result<AggregateResult> {
        let g = self.state.lock().unwrap();
        let mut total = 0.0;
        let mut requests = 0i64;
        let mut prompt = 0i64;
        let mut completion = 0i64;
        for r in g.iter() {
            if !q.project_id.as_deref().map_or(true, |p| p == r.project_id) {
                continue;
            }
            if !q.from_ts.map_or(true, |f| r.request_ts >= f) {
                continue;
            }
            if !q.to_ts.map_or(true, |t| r.request_ts <= t) {
                continue;
            }
            requests += 1;
            prompt += r.prompt_tokens.unwrap_or(0);
            completion += r.completion_tokens.unwrap_or(0);
            total += r.cost_usd.unwrap_or(0.0);
        }
        Ok(AggregateResult {
            total_cost_usd: total,
            groups: vec![AggregateGroup {
                key: serde_json::Value::Null,
                requests,
                prompt_tokens: prompt,
                completion_tokens: completion,
                cost_usd: total,
                cached_savings_usd: 0.0,
            }],
        })
    }

    async fn purge_older_than(&self, ts: Timestamp) -> Result<u64> {
        let mut g = self.state.lock().unwrap();
        let before = g.len();
        g.retain(|r| r.request_ts >= ts);
        Ok((before - g.len()) as u64)
    }

    async fn distinct_cost_keys(
        &self,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
    ) -> Result<Vec<(String, String)>> {
        let g = self.state.lock().unwrap();
        let mut keys: Vec<(String, String)> = g
            .iter()
            .filter(|r| {
                (r.prompt_tokens.is_some() || r.completion_tokens.is_some())
                    && from.is_none_or(|f| r.request_ts >= f)
                    && to.is_none_or(|t| r.request_ts <= t)
            })
            .filter_map(|r| Some((resolve_provider(r)?, r.model.clone()?)))
            .collect();
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    async fn recompute_costs(
        &self,
        provider: &str,
        model: &str,
        input_per_1k: f64,
        cached_input_per_1k: f64,
        output_per_1k: f64,
        from: Option<Timestamp>,
        to: Option<Timestamp>,
    ) -> Result<u64> {
        let round6 = |v: f64| (v * 1_000_000.0).round() / 1_000_000.0;
        let mut g = self.state.lock().unwrap();
        let mut updated = 0u64;
        for r in g.iter_mut() {
            let r_provider = resolve_provider(r);
            if r_provider.as_deref() != Some(provider)
                || r.model.as_deref() != Some(model)
                || (r.prompt_tokens.is_none() && r.completion_tokens.is_none())
                || from.is_some_and(|f| r.request_ts < f)
                || to.is_some_and(|t| r.request_ts > t)
            {
                continue;
            }
            let prompt = r.prompt_tokens.unwrap_or(0);
            let cached = r.cached_tokens.unwrap_or(0);
            let completion = r.completion_tokens.unwrap_or(0);
            let cost = round6(
                (prompt - cached).max(0) as f64 * input_per_1k / 1000.0
                    + cached.max(0) as f64 * cached_input_per_1k / 1000.0
                    + completion.max(0) as f64 * output_per_1k / 1000.0,
            );
            r.would_have_cost_usd = Some(cost);
            r.cost_usd = Some(if r.cached { 0.0 } else { cost });
            updated += 1;
        }
        Ok(updated)
    }

    async fn flush(&self) -> Result<()> {
        Ok(())
    }
}

pub struct MemoryKvStore {
    state: Mutex<HashMap<String, (Bytes, Instant)>>,
}

impl Default for MemoryKvStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryKvStore {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl KvStore for MemoryKvStore {
    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        let mut g = self.state.lock().unwrap();
        if let Some((v, exp)) = g.get(key) {
            if *exp > Instant::now() {
                return Ok(Some(v.clone()));
            }
            g.remove(key);
        }
        Ok(None)
    }

    async fn put(&self, key: &str, value: Bytes, ttl: Duration) -> Result<()> {
        let exp = Instant::now() + ttl;
        self.state
            .lock()
            .unwrap()
            .insert(key.to_string(), (value, exp));
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.state.lock().unwrap().remove(key);
        Ok(())
    }
}

pub struct MemoryCounterStore {
    windows: Arc<Mutex<HashMap<(String, u64, u64), i64>>>,
    concurrency: Arc<Mutex<HashMap<String, u32>>>,
    budgets: Arc<Mutex<HashMap<(String, Timestamp), f64>>>,
}

impl Default for MemoryCounterStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryCounterStore {
    pub fn new() -> Self {
        Self {
            windows: Arc::new(Mutex::new(HashMap::new())),
            concurrency: Arc::new(Mutex::new(HashMap::new())),
            budgets: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl CounterStore for MemoryCounterStore {
    async fn incr_window(&self, key: &str, window_ms: u64, by: i64) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis() as u64;
        let bucket = now / window_ms.max(1);
        let mut g = self.windows.lock().unwrap();
        let entry = g.entry((key.to_string(), window_ms, bucket)).or_insert(0);
        *entry += by;
        Ok(*entry)
    }

    async fn current(&self, key: &str, window_ms: u64) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis() as u64;
        let bucket = now / window_ms.max(1);
        Ok(*self
            .windows
            .lock()
            .unwrap()
            .get(&(key.to_string(), window_ms, bucket))
            .unwrap_or(&0))
    }

    async fn try_acquire_concurrency(
        &self,
        key: &str,
        max: u32,
    ) -> Result<Option<ConcurrencyPermit>> {
        let key_owned = key.to_string();
        let acquired = {
            let mut g = self.concurrency.lock().unwrap();
            let cur = g.entry(key_owned.clone()).or_insert(0);
            if *cur >= max {
                false
            } else {
                *cur += 1;
                true
            }
        };
        if !acquired {
            return Ok(None);
        }
        let concurrency = self.concurrency.clone();
        let release = move || {
            let mut g = concurrency.lock().unwrap();
            if let Some(c) = g.get_mut(&key_owned) {
                *c = c.saturating_sub(1);
            }
        };
        Ok(Some(ConcurrencyPermit::new(release)))
    }

    async fn incr_budget(
        &self,
        budget_id: &str,
        period_start: Timestamp,
        delta: f64,
    ) -> Result<f64> {
        let mut g = self.budgets.lock().unwrap();
        let entry = g
            .entry((budget_id.to_string(), period_start))
            .or_insert(0.0);
        *entry += delta;
        Ok(*entry)
    }

    async fn read_budget(&self, budget_id: &str, period_start: Timestamp) -> Result<f64> {
        Ok(*self
            .budgets
            .lock()
            .unwrap()
            .get(&(budget_id.to_string(), period_start))
            .unwrap_or(&0.0))
    }
}
