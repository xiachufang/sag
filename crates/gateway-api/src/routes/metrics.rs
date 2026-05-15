use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use metrics_exporter_prometheus::PrometheusHandle;

use crate::state::AppState;

#[derive(Clone)]
pub struct MetricsState {
    pub handle: Option<PrometheusHandle>,
}

pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let _ = state;
    let body = crate::metrics::install_prometheus().render();
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}
