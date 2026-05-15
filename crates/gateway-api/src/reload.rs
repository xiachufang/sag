use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use notify::{Event, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use gateway_core::config::AppConfig;

/// Spawn a background task that watches the config file and atomically
/// swaps the [`AppConfig`] in `target` whenever the file changes (with a
/// short debounce). Only fields safe to live-reload are honored —
/// providers, listen address, storage backends, etc. require a restart.
pub fn spawn(path: PathBuf, target: Arc<ArcSwap<AppConfig>>) {
    tokio::spawn(async move {
        if let Err(e) = run(path, target).await {
            tracing::warn!(error = %e, "config watcher exited");
        }
    });
}

async fn run(path: PathBuf, target: Arc<ArcSwap<AppConfig>>) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::channel::<()>(8);
    let watch_path = path.clone();
    let _watcher = std::thread::spawn(move || {
        let tx_inner = tx.clone();
        let res = notify::recommended_watcher(move |res: Result<Event, _>| {
            if res.is_ok() {
                let _ = tx_inner.try_send(());
            }
        });
        if let Ok(mut w) = res {
            let _ = w.watch(&watch_path, RecursiveMode::NonRecursive);
            // Park the thread; the watcher must outlive the channel.
            std::thread::park();
        }
    });

    // Debounce loop.
    while rx.recv().await.is_some() {
        // Drain any extra events queued in the next 200ms.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                Some(_) = rx.recv() => {}
                else => break,
            }
        }
        try_reload(&path, &target).await;
    }
    Ok(())
}

async fn try_reload(path: &Path, target: &Arc<ArcSwap<AppConfig>>) {
    match AppConfig::load_from_path(path) {
        Ok(new_cfg) => {
            let old = target.load_full();
            let warn = warn_unsafe_changes(&old, &new_cfg);
            target.store(Arc::new(new_cfg));
            metrics::counter!("gateway_config_reload_total").increment(1);
            tracing::info!(path = %path.display(), warn = %warn, "config hot-reloaded");
        }
        Err(e) => {
            metrics::counter!("gateway_config_reload_error_total").increment(1);
            tracing::warn!(error = %e, "config reload failed; keeping previous config");
        }
    }
}

fn warn_unsafe_changes(old: &AppConfig, new: &AppConfig) -> String {
    let mut warnings = Vec::new();
    if old.server.bind != new.server.bind {
        warnings.push("server.bind changed (requires restart)");
    }
    if old.providers.len() != new.providers.len()
        || old.providers.iter().any(|(k, v)| {
            new.providers
                .get(k)
                .map(|n| n.base_url != v.base_url)
                .unwrap_or(true)
        })
    {
        warnings.push("providers changed (proxy engine still uses old credentials until restart)");
    }
    if old.storage.profile_name() != new.storage.profile_name() {
        warnings.push("storage.profile changed (requires restart)");
    }
    warnings.join(", ")
}
