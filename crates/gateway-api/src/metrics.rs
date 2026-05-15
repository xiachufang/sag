use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use once_cell::sync::OnceCell;

static HANDLE: OnceCell<PrometheusHandle> = OnceCell::new();

/// Install the global Prometheus recorder and return a handle that can be
/// used by the `/metrics` endpoint. Safe to call multiple times — the
/// recorder is installed only on the first call.
pub fn install_prometheus() -> PrometheusHandle {
    if let Some(h) = HANDLE.get() {
        return h.clone();
    }
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("install prometheus recorder");
    let _ = HANDLE.set(handle.clone());
    handle
}

/// Render the current metrics snapshot as Prometheus text format.
pub fn render(handle: &PrometheusHandle) -> String {
    handle.render()
}
