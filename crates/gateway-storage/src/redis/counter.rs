use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use crate::error::{Result, StorageError};
use crate::models::Timestamp;
use crate::traits::{ConcurrencyPermit, CounterStore};

pub struct RedisCounterStore {
    conn: ConnectionManager,
}

impl RedisCounterStore {
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }
}

fn bucket_key(key: &str, window_ms: u64) -> String {
    let bucket = (chrono::Utc::now().timestamp_millis() as u64) / window_ms.max(1);
    format!("ctr:{key}:{window_ms}:{bucket}")
}

#[async_trait]
impl CounterStore for RedisCounterStore {
    async fn incr_window(&self, key: &str, window_ms: u64, by: i64) -> Result<i64> {
        let mut conn = self.conn.clone();
        let k = bucket_key(key, window_ms);
        let v: i64 = conn
            .incr(&k, by)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis incr: {e}")))?;
        // Set TTL on first increment so the key doesn't accumulate forever.
        let _: bool = conn
            .expire(&k, ((window_ms / 1000).max(1) * 2) as i64)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis expire: {e}")))?;
        Ok(v)
    }

    async fn current(&self, key: &str, window_ms: u64) -> Result<i64> {
        let mut conn = self.conn.clone();
        let v: Option<i64> = conn
            .get(bucket_key(key, window_ms))
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis get: {e}")))?;
        Ok(v.unwrap_or(0))
    }

    async fn try_acquire_concurrency(
        &self,
        key: &str,
        max: u32,
    ) -> Result<Option<ConcurrencyPermit>> {
        let mut conn = self.conn.clone();
        let conn_for_permit = self.conn.clone();
        let k = format!("conc:{key}");
        let v: i64 = conn
            .incr(&k, 1)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis incr conc: {e}")))?;
        // Safety: refresh expire so a crashed client doesn't leak forever.
        let _: bool = conn
            .expire(&k, 60)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis expire: {e}")))?;
        if v as u32 > max {
            // Roll back, deny.
            let _: i64 = conn
                .decr(&k, 1)
                .await
                .map_err(|e| StorageError::Unavailable(format!("redis decr: {e}")))?;
            return Ok(None);
        }
        let key_owned = k;
        let release = move || {
            let mut conn = conn_for_permit.clone();
            tokio::spawn(async move {
                let _: std::result::Result<i64, _> = conn.decr(&key_owned, 1).await;
            });
        };
        Ok(Some(ConcurrencyPermit::new(release)))
    }

    async fn incr_budget(
        &self,
        budget_id: &str,
        period_start: Timestamp,
        delta: f64,
    ) -> Result<f64> {
        let mut conn = self.conn.clone();
        let k = format!("budget:{budget_id}:{period_start}");
        // INCRBYFLOAT returns the new value as a string.
        let v: f64 = redis::cmd("INCRBYFLOAT")
            .arg(&k)
            .arg(delta)
            .query_async(&mut conn)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis incrbyfloat: {e}")))?;
        let _: bool = conn
            .expire(&k, 60 * 60 * 24 * 35)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis expire: {e}")))?;
        Ok(v)
    }

    async fn read_budget(&self, budget_id: &str, period_start: Timestamp) -> Result<f64> {
        let mut conn = self.conn.clone();
        let v: Option<String> = conn
            .get(format!("budget:{budget_id}:{period_start}"))
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis get: {e}")))?;
        Ok(v.and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0))
    }
}
