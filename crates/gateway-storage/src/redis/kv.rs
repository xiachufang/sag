use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use crate::error::{Result, StorageError};
use crate::traits::KvStore;

pub struct RedisKvStore {
    conn: ConnectionManager,
}

impl RedisKvStore {
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl KvStore for RedisKvStore {
    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        let mut conn = self.conn.clone();
        let v: Option<Vec<u8>> = conn
            .get(key)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis get: {e}")))?;
        Ok(v.map(Bytes::from))
    }

    async fn put(&self, key: &str, value: Bytes, ttl: Duration) -> Result<()> {
        let mut conn = self.conn.clone();
        let secs = ttl.as_secs().max(1);
        let _: () = conn
            .set_ex(key, value.as_ref(), secs)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis set: {e}")))?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let mut conn = self.conn.clone();
        let _: () = conn
            .del(key)
            .await
            .map_err(|e| StorageError::Unavailable(format!("redis del: {e}")))?;
        Ok(())
    }
}
