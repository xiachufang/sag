use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use gateway_api::budget::BudgetManager;
use gateway_api::server::serve;
use gateway_api::state::AppState;
use gateway_core::config::{AppConfig, StorageConfig};
use gateway_core::pricing::PricingCatalog;
use gateway_core::proxy::ProxyEngine;
use gateway_core::security::{AdminTokenSigner, MasterKey};
use gateway_storage::memory::{
    MemoryCounterStore, MemoryKvStore, MemoryLogStore, MemoryMetadataStore,
};
use gateway_storage::models::NewProject;
use gateway_storage::postgres::{PostgresBackend, PostgresConfig};
use gateway_storage::redis::{connect as redis_connect, RedisCounterStore, RedisKvStore};
use gateway_storage::sqlite::{SqliteBackend, SqlitePoolConfig};
use gateway_storage::traits::{CounterStore, KvStore, LogStore, MetadataStore, StoreBundle};

const MASTER_KEY_ENV: &str = "GATEWAY_MASTER_KEY";

#[derive(Debug, Parser)]
#[command(name = "gateway", version, about = "Simple AI Gateway")]
struct Cli {
    /// Path to the YAML config file.
    #[arg(
        short,
        long,
        env = "GATEWAY_CONFIG",
        default_value = "config/example.lite.yaml"
    )]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing();

    let config = AppConfig::load_from_path(&cli.config)
        .map_err(|e| anyhow!("failed to load config {}: {e}", cli.config.display()))?;

    validate_runtime_environment(&config)?;

    gateway_api::metrics::install_prometheus();

    let master_key = load_master_key()?;
    let admin_signer = AdminTokenSigner::new(&master_key);
    let admin_root_token = read_admin_token(&config);

    let stores = build_stores(&config).await?;
    seed_default_project(&stores, &config.server.default_project_id).await?;

    let proxy = Arc::new(
        ProxyEngine::new(
            &config,
            Some(stores.metadata.clone()),
            Some(master_key.clone()),
            &config.server.default_project_id,
        )
        .await
        .context("failed to construct proxy engine")?,
    );

    let config_arc = Arc::new(ArcSwap::from_pointee(config.clone()));
    let pricing = Arc::new(PricingCatalog::embedded());
    let budgets = Arc::new(BudgetManager::new(
        stores.counter.clone(),
        stores.metadata.clone(),
        config_arc.clone(),
    ));

    let state = AppState {
        config: config_arc,
        proxy,
        stores,
        default_project_id: config.server.default_project_id.clone(),
        admin_root_token,
        master_key: Arc::new(master_key),
        admin_signer: Arc::new(admin_signer),
        pricing,
        budgets,
    };

    gateway_api::reload::spawn(cli.config.clone(), state.config.clone());

    let handle = serve(state, &config.server.bind).await?;
    tracing::info!(addr = %handle.addr, profile = %config.storage.profile_name(), "gateway listening");

    tokio::signal::ctrl_c()
        .await
        .context("failed to install Ctrl-C handler")?;
    tracing::info!("shutdown signal received");
    let _ = handle.shutdown.send(());
    let _ = handle.join.await;
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx::query=warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .json()
        .init();
}

fn read_admin_token(config: &AppConfig) -> Option<String> {
    let var = config.admin.root_token_env.as_str();
    if var.is_empty() {
        None
    } else {
        std::env::var(var).ok().filter(|v| !v.is_empty())
    }
}

fn load_master_key() -> Result<MasterKey> {
    match std::env::var(MASTER_KEY_ENV) {
        Ok(v) => MasterKey::from_base64(&v).map_err(|e| anyhow!("{MASTER_KEY_ENV} invalid: {e}")),
        Err(_) => Err(anyhow!(
            "{MASTER_KEY_ENV} not set; generate one with `openssl rand -base64 32` and export it"
        )),
    }
}

/// Refuse to start in obviously broken setups (Lite + multiple workers,
/// Lite on a networked filesystem, etc.).
fn validate_runtime_environment(config: &AppConfig) -> Result<()> {
    if let StorageConfig::Lite { sqlite, .. } = &config.storage {
        // Multi-worker check: we don't actually spawn workers ourselves, but
        // we refuse to run if the operator set the documented env knob.
        if std::env::var("GATEWAY_WORKERS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
            > 1
        {
            return Err(anyhow!(
                "GATEWAY_WORKERS>1 is incompatible with storage.profile=lite; switch to profile=standard or run a single worker"
            ));
        }
        // NFS check: SQLite locking on NFS is unreliable. Inspect filesystem type best-effort.
        if is_likely_networked(&sqlite.path) {
            tracing::warn!(
                path = %sqlite.path.display(),
                "SQLite database appears to be on a networked filesystem; this is unsafe"
            );
        }
    }
    Ok(())
}

fn is_likely_networked(path: &std::path::Path) -> bool {
    // Cheap heuristic on macOS/Linux: nothing super reliable from userspace.
    // Look at the parent for an `nfs`/`smb` substring in canonical path.
    if let Ok(canon) = path.canonicalize() {
        let s = canon.to_string_lossy().to_ascii_lowercase();
        return s.contains("/nfs/") || s.contains("/smb/");
    }
    false
}

async fn build_stores(config: &AppConfig) -> Result<StoreBundle> {
    match &config.storage {
        StorageConfig::Lite { sqlite, cache } => {
            let cfg = SqlitePoolConfig::new(sqlite.path.clone());
            let backend = SqliteBackend::open(&cfg)
                .await
                .with_context(|| format!("opening sqlite db at {}", sqlite.path.display()))?;
            let metadata: Arc<dyn MetadataStore> = Arc::new(backend.metadata_store());
            let logs: Arc<dyn LogStore> = Arc::new(backend.log_store());
            let kv: Arc<dyn KvStore> = Arc::new(backend.kv_store(cache.l1_memory_mb));
            let counter: Arc<dyn CounterStore> = Arc::new(MemoryCounterStore::new());
            Ok(StoreBundle {
                metadata,
                logs,
                kv,
                counter,
            })
        }
        StorageConfig::Standard {
            postgres, redis, ..
        } => {
            let pg = PostgresBackend::open(&PostgresConfig {
                url: postgres.url.clone(),
                max_connections: postgres.max_connections,
            })
            .await
            .context("opening postgres pool")?;
            let conn = redis_connect(&redis.url)
                .await
                .context("connecting to redis")?;
            let metadata: Arc<dyn MetadataStore> = Arc::new(pg.metadata_store());
            let logs: Arc<dyn LogStore> = Arc::new(pg.log_store());
            let kv: Arc<dyn KvStore> = Arc::new(RedisKvStore::new(conn.clone()));
            let counter: Arc<dyn CounterStore> = Arc::new(RedisCounterStore::new(conn));
            Ok(StoreBundle {
                metadata,
                logs,
                kv,
                counter,
            })
        }
        StorageConfig::Memory { .. } => {
            let metadata: Arc<dyn MetadataStore> = Arc::new(MemoryMetadataStore::new());
            let logs: Arc<dyn LogStore> = Arc::new(MemoryLogStore::new(10_000));
            let kv: Arc<dyn KvStore> = Arc::new(MemoryKvStore::new());
            let counter: Arc<dyn CounterStore> = Arc::new(MemoryCounterStore::new());
            Ok(StoreBundle {
                metadata,
                logs,
                kv,
                counter,
            })
        }
    }
}

async fn seed_default_project(stores: &StoreBundle, project_id: &str) -> Result<()> {
    if stores.metadata.get_project(project_id).await?.is_none() {
        stores
            .metadata
            .create_project(NewProject {
                id: project_id.to_string(),
                name: project_id.to_string(),
            })
            .await?;
    }
    Ok(())
}
