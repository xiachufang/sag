use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use gateway_storage::models::{AggregateDimension, AggregateQuery, AggregateResult};

use crate::auth::AdminPrincipal;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CostQuery {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
    /// Comma-separated dimension list: provider, model, day, hour, gateway_key.
    #[serde(default)]
    pub group_by: Option<String>,
}

pub async fn aggregate_cost(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Query(q): Query<CostQuery>,
) -> Result<Json<AggregateResult>, ApiError> {
    let group_by = q
        .group_by
        .unwrap_or_else(|| "provider,model".into())
        .split(',')
        .filter_map(|s| match s.trim().to_ascii_lowercase().as_str() {
            "provider" => Some(AggregateDimension::Provider),
            "model" => Some(AggregateDimension::Model),
            "day" => Some(AggregateDimension::Day),
            "hour" => Some(AggregateDimension::Hour),
            "gateway_key" | "key" => Some(AggregateDimension::GatewayKey),
            _ => None,
        })
        .collect::<Vec<_>>();
    let result = state
        .stores
        .logs
        .aggregate(AggregateQuery {
            project_id: q.project_id.or(Some(state.default_project_id.clone())),
            from_ts: q.from,
            to_ts: q.to,
            group_by,
        })
        .await?;
    Ok(Json(result))
}
