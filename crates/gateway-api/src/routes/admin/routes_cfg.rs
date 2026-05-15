use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use gateway_core::config::AppConfig;

use crate::auth::AdminPrincipal;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn get_routes(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
) -> Result<Json<Arc<AppConfig>>, ApiError> {
    Ok(Json(state.config_snapshot()))
}
