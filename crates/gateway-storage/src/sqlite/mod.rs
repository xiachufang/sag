pub mod kv;
pub mod logs;
pub mod metadata;
pub mod pool;

use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::Mutex;

pub use kv::SqliteKvStore;
pub use logs::SqliteLogStore;
pub use metadata::SqliteMetadataStore;
pub use pool::{open_sqlite_pool, run_migrations, SqlitePoolConfig};

use crate::error::Result;

/// Holds a SQLite connection pool plus the single write mutex that all
/// stores share. Construct stores via the helper methods so they end up
/// using the same lock.
#[derive(Clone)]
pub struct SqliteBackend {
    pub pool: SqlitePool,
    pub write_lock: Arc<Mutex<()>>,
}

impl SqliteBackend {
    pub async fn open(cfg: &SqlitePoolConfig) -> Result<Self> {
        let pool = open_sqlite_pool(cfg).await?;
        run_migrations(&pool).await?;
        Ok(Self {
            pool,
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn metadata_store(&self) -> SqliteMetadataStore {
        SqliteMetadataStore::new(self.pool.clone(), self.write_lock.clone())
    }

    pub fn log_store(&self) -> SqliteLogStore {
        SqliteLogStore::new(self.pool.clone(), self.write_lock.clone())
    }

    pub fn kv_store(&self, l1_capacity_mb: u64) -> SqliteKvStore {
        SqliteKvStore::new(self.pool.clone(), self.write_lock.clone(), l1_capacity_mb)
    }
}
