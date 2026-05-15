use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use moka::future::Cache;
use sqlx::{Row, SqlitePool};
use tokio::sync::Mutex;

use crate::error::Result;
use crate::traits::KvStore;

/// L1 (moka) + L2 (SQLite) cache with TTL-based eviction. Reads hit L1
/// first, falling back to L2 on miss and refilling L1.
pub struct SqliteKvStore {
    pool: SqlitePool,
    write_lock: Arc<Mutex<()>>,
    l1: Cache<String, Bytes>,
}

impl SqliteKvStore {
    pub fn new(pool: SqlitePool, write_lock: Arc<Mutex<()>>, l1_capacity_mb: u64) -> Self {
        // moka counts weight in arbitrary units; map ~1 byte per unit.
        let l1 = Cache::builder()
            .weigher(|_k: &String, v: &Bytes| v.len().min(u32::MAX as usize) as u32)
            .max_capacity(l1_capacity_mb.saturating_mul(1024 * 1024))
            .time_to_live(Duration::from_secs(3600 * 24))
            .build();

        let me = Self {
            pool,
            write_lock,
            l1,
        };

        // Spawn the eviction worker. It owns clones of pool + write_lock.
        me.spawn_evictor();
        me
    }

    fn spawn_evictor(&self) {
        let pool = self.pool.clone();
        let write_lock = self.write_lock.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(60));
            loop {
                ticker.tick().await;
                let now = chrono::Utc::now().timestamp_millis();
                let _w = write_lock.lock().await;
                if let Err(e) = sqlx::query(
                    "DELETE FROM kv_cache WHERE rowid IN (SELECT rowid FROM kv_cache WHERE expires_at < ? LIMIT 5000)",
                )
                .bind(now)
                .execute(&pool)
                .await
                {
                    tracing::warn!(error = %e, "kv eviction failed");
                }
            }
        });
    }
}

#[async_trait]
impl KvStore for SqliteKvStore {
    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        if let Some(v) = self.l1.get(&key.to_string()).await {
            metrics::counter!("gateway_cache_hit_total", "tier" => "l1").increment(1);
            return Ok(Some(v));
        }
        let row = sqlx::query("SELECT value, expires_at FROM kv_cache WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => {
                let exp: i64 = r.get("expires_at");
                let now = chrono::Utc::now().timestamp_millis();
                if exp < now {
                    metrics::counter!("gateway_cache_miss_total").increment(1);
                    let _w = self.write_lock.lock().await;
                    let _ = sqlx::query("DELETE FROM kv_cache WHERE key = ?")
                        .bind(key)
                        .execute(&self.pool)
                        .await;
                    return Ok(None);
                }
                let bytes: Vec<u8> = r.get("value");
                let value = Bytes::from(bytes);
                self.l1.insert(key.to_string(), value.clone()).await;
                metrics::counter!("gateway_cache_hit_total", "tier" => "l2").increment(1);
                Ok(Some(value))
            }
            None => {
                metrics::counter!("gateway_cache_miss_total").increment(1);
                Ok(None)
            }
        }
    }

    async fn put(&self, key: &str, value: Bytes, ttl: Duration) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let expires_at = now + ttl.as_millis().min(i64::MAX as u128) as i64;
        let size = value.len() as i64;
        self.l1.insert(key.to_string(), value.clone()).await;
        let _w = self.write_lock.lock().await;
        sqlx::query(
            r#"
            INSERT INTO kv_cache (key, value, expires_at, size_bytes, hit_count, last_accessed_at, created_at)
            VALUES (?, ?, ?, ?, 0, ?, ?)
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                expires_at = excluded.expires_at,
                size_bytes = excluded.size_bytes,
                last_accessed_at = excluded.last_accessed_at
            "#,
        )
        .bind(key)
        .bind(&value[..])
        .bind(expires_at)
        .bind(size)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        metrics::counter!("gateway_cache_write_total").increment(1);
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.l1.invalidate(&key.to_string()).await;
        let _w = self.write_lock.lock().await;
        sqlx::query("DELETE FROM kv_cache WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
