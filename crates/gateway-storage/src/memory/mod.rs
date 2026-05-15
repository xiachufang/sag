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
    creds: HashMap<String, ProviderCredential>,
    routes: HashMap<String, (RoutesConfig, i64)>,
    budgets: HashMap<String, Budget>,
    admins: HashMap<String, AdminUser>,
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

    async fn put_provider_credential(&self, c: ProviderCredential) -> Result<()> {
        self.state.lock().unwrap().creds.insert(c.id.clone(), c);
        Ok(())
    }

    async fn list_provider_credentials(&self, project_id: &str) -> Result<Vec<ProviderCredential>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .creds
            .values()
            .filter(|c| c.project_id == project_id)
            .cloned()
            .collect())
    }

    async fn delete_provider_credential(&self, id: &str) -> Result<()> {
        if self.state.lock().unwrap().creds.remove(id).is_none() {
            return Err(StorageError::NotFound);
        }
        Ok(())
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
                    && q.provider
                        .as_deref()
                        .map_or(true, |p| r.provider.as_deref() == Some(p))
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
                provider: r.provider.clone(),
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
