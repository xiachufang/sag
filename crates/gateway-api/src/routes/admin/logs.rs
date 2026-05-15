use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use gateway_storage::models::LogQuery;

use crate::auth::AdminPrincipal;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListLogsQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub gateway_key_id: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub items: Vec<serde_json::Value>,
    pub next_cursor: Option<String>,
}

pub async fn list_logs(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Query(q): Query<ListLogsQuery>,
) -> Result<Json<LogsResponse>, ApiError> {
    let project_id = q.project_id.or(Some(state.default_project_id.clone()));
    let lq = LogQuery {
        project_id,
        gateway_key_id: q.gateway_key_id,
        provider: q.provider,
        model: q.model,
        status: q.status,
        from_ts: q.from,
        to_ts: q.to,
        limit: q.limit.unwrap_or(50),
        cursor: q.cursor,
    };
    let page = state.stores.logs.query(lq).await?;
    let items = page
        .items
        .into_iter()
        .map(|r| serde_json::to_value(&r).unwrap_or(serde_json::Value::Null))
        .collect();
    Ok(Json(LogsResponse {
        items,
        next_cursor: page.next_cursor,
    }))
}

pub async fn get_log(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let detail = state
        .stores
        .logs
        .get_by_id(&id)
        .await?
        .ok_or(ApiError::Gateway(gateway_core::GatewayError::NotFound))?;
    Ok(Json(serde_json::to_value(&detail.record).unwrap()))
}
