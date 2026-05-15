use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct SqlitePoolConfig {
    pub path: PathBuf,
    pub max_connections: u32,
    pub busy_timeout: Duration,
    pub cache_size_kb: i64,
    pub mmap_size: i64,
}

impl SqlitePoolConfig {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            max_connections: 8,
            busy_timeout: Duration::from_secs(5),
            cache_size_kb: 64 * 1024, // 64 MB
            mmap_size: 256 * 1024 * 1024,
        }
    }
}

/// Open a SQLite pool configured with WAL + reasonable pragmas. The pool is
/// used for reads; writes should still be serialized at the application layer
/// to minimize `database is locked` errors under load.
pub async fn open_sqlite_pool(cfg: &SqlitePoolConfig) -> Result<SqlitePool> {
    if let Some(parent) = cfg.path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
    }
    ensure_writable(&cfg.path)?;

    let url = format!("sqlite://{}", cfg.path.display());
    let options = SqliteConnectOptions::from_str(&url)
        .map_err(sqlx::Error::from)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(cfg.busy_timeout)
        .foreign_keys(true)
        .pragma("cache_size", format!("-{}", cfg.cache_size_kb))
        .pragma("temp_store", "MEMORY")
        .pragma("mmap_size", cfg.mmap_size.to_string());

    let pool = SqlitePoolOptions::new()
        .max_connections(cfg.max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await?;

    Ok(pool)
}

fn ensure_writable(path: &Path) -> Result<()> {
    // basic sanity: can't be NFS path on macOS we don't really detect, leave to
    // the runtime check at startup. Here we just ensure the parent dir is okay.
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && parent.exists() {
            let md = std::fs::metadata(parent)?;
            if md.permissions().readonly() {
                return Err(crate::error::StorageError::Invalid(format!(
                    "sqlite parent directory is read-only: {}",
                    parent.display()
                )));
            }
        }
    }
    Ok(())
}

/// Run the embedded migrations. Migrations live under `migrations/sqlite/`
/// in the workspace root and are compiled in via `sqlx::migrate!`.
pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    sqlx::migrate!("../../migrations/sqlite").run(pool).await?;
    Ok(())
}
