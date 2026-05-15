pub mod logs;
pub mod metadata;

use sqlx::PgPool;

use crate::error::Result;

pub use logs::PostgresLogStore;
pub use metadata::PostgresMetadataStore;

#[derive(Debug, Clone)]
pub struct PostgresConfig {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Clone)]
pub struct PostgresBackend {
    pub pool: PgPool,
}

impl PostgresBackend {
    pub async fn open(cfg: &PostgresConfig) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(cfg.max_connections)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect(&cfg.url)
            .await?;
        sqlx::migrate!("../../migrations/postgres")
            .run(&pool)
            .await?;
        Ok(Self { pool })
    }

    pub fn metadata_store(&self) -> PostgresMetadataStore {
        PostgresMetadataStore::new(self.pool.clone())
    }

    pub fn log_store(&self) -> PostgresLogStore {
        PostgresLogStore::new(self.pool.clone())
    }
}
