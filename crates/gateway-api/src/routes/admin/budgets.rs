use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::auth::AdminPrincipal;
use crate::budget::BudgetUsage;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct BudgetsQuery {
    #[serde(default)]
    pub project_id: Option<String>,
}

pub async fn list_budgets(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Query(q): Query<BudgetsQuery>,
) -> Json<Vec<BudgetUsage>> {
    Json(state.budgets.current_usage(q.project_id.as_deref()).await)
}
