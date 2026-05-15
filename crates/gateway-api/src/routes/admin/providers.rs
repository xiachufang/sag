use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use gateway_core::security::encrypt_credential;
use gateway_storage::models::ProviderCredential;

use crate::auth::AdminPrincipal;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateCredentialRequest {
    pub provider: String,
    pub name: String,
    pub api_key: String,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CredentialSummary {
    pub id: String,
    pub project_id: String,
    pub provider: String,
    pub name: String,
    pub status: String,
    pub created_at: i64,
}

pub async fn create_credential(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Json(req): Json<CreateCredentialRequest>,
) -> Result<Json<CredentialSummary>, ApiError> {
    if req.api_key.is_empty() {
        return Err(ApiError::BadRequest("api_key required".into()));
    }
    let encrypted = encrypt_credential(&state.master_key, &req.api_key)?;
    let project_id = req
        .project_id
        .unwrap_or_else(|| state.default_project_id.clone());
    let row = ProviderCredential {
        id: Uuid::now_v7().to_string(),
        project_id: project_id.clone(),
        provider: req.provider,
        name: req.name,
        encrypted_key: encrypted,
        status: "active".into(),
        created_at: Utc::now().timestamp_millis(),
    };
    state
        .stores
        .metadata
        .put_provider_credential(row.clone())
        .await?;
    Ok(Json(CredentialSummary {
        id: row.id,
        project_id: row.project_id,
        provider: row.provider,
        name: row.name,
        status: row.status,
        created_at: row.created_at,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ListCredentialsQuery {
    #[serde(default)]
    pub project_id: Option<String>,
}

pub async fn list_credentials(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    axum::extract::Query(q): axum::extract::Query<ListCredentialsQuery>,
) -> Result<Json<Vec<CredentialSummary>>, ApiError> {
    let project_id = q
        .project_id
        .unwrap_or_else(|| state.default_project_id.clone());
    let rows = state
        .stores
        .metadata
        .list_provider_credentials(&project_id)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| CredentialSummary {
                id: r.id,
                project_id: r.project_id,
                provider: r.provider,
                name: r.name,
                status: r.status,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

pub async fn delete_credential(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .stores
        .metadata
        .delete_provider_credential(&id)
        .await?;
    Ok(Json(serde_json::json!({ "id": id, "status": "deleted" })))
}
