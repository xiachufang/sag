use std::net::SocketAddr;
use std::str::FromStr;

use axum::http::{header, StatusCode};
use axum::response::Redirect;
use axum::routing::{any, delete, get, post};
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tower_http::trace::TraceLayer;

use crate::routes::{
    admin::{auth as admin_auth, budgets, cost, keys, logs, providers, routes_cfg},
    health, metrics, proxy,
};
use crate::state::AppState;

/// Bundled minimal admin UI (M2). Replaced by a Vite build in later
/// milestones — the file is shipped as-is so the gateway is fully
/// usable without a Node toolchain.
const UI_INDEX_HTML: &str = include_str!("../../gateway-ui/dist/index.html");

pub fn build_router(state: AppState) -> Router {
    let admin = Router::new()
        .route("/auth/login", post(admin_auth::login))
        .route("/auth/me", get(admin_auth::me))
        .route("/admins", post(admin_auth::create_admin))
        .route("/admins", get(admin_auth::list_admins))
        .route("/keys", post(keys::create_key))
        .route("/keys", get(keys::list_keys))
        .route("/keys/:id", delete(keys::revoke_key))
        .route(
            "/providers/credentials",
            post(providers::create_credential).get(providers::list_credentials),
        )
        .route(
            "/providers/credentials/:id",
            delete(providers::delete_credential),
        )
        .route("/logs", get(logs::list_logs))
        .route("/logs/:id", get(logs::get_log))
        .route("/routes", get(routes_cfg::get_routes))
        .route("/cost", get(cost::aggregate_cost))
        .route("/budgets", get(budgets::list_budgets));

    Router::new()
        .route("/", get(|| async { Redirect::permanent("/ui/") }))
        .route(
            "/ui/",
            get(|| async {
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    UI_INDEX_HTML,
                )
            }),
        )
        .route(
            "/ui/*rest",
            get(|| async {
                // Single-page app: every sub-path returns the same HTML.
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    UI_INDEX_HTML,
                )
            }),
        )
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/metrics", get(metrics::metrics_handler))
        .route("/v1/:provider/*tail", any(proxy::proxy_handler))
        .nest("/admin", admin)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub struct ServerHandle {
    pub addr: SocketAddr,
    pub shutdown: oneshot::Sender<()>,
    pub join: tokio::task::JoinHandle<std::io::Result<()>>,
}

pub async fn serve(state: AppState, bind: &str) -> anyhow::Result<ServerHandle> {
    let addr = SocketAddr::from_str(bind)
        .map_err(|e| anyhow::anyhow!("invalid bind address {bind}: {e}"))?;
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let app = build_router(state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let join = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
    });

    Ok(ServerHandle {
        addr: local_addr,
        shutdown: shutdown_tx,
        join,
    })
}
