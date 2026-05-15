use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use gateway_core::security::{generate_gateway_key, KeyEnv};
use gateway_storage::models::NewGatewayKey;

use crate::auth::AdminPrincipal;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateKeyRequest {
    pub name: String,
    #[serde(default = "default_env")]
    pub env: String, // live | test
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub expires_at: Option<i64>, // unix ms
}

fn default_env() -> String {
    "live".into()
}

#[derive(Debug, Serialize)]
pub struct CreateKeyResponse {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub last4: String,
    pub secret: String, // returned exactly once
    pub created_at: i64,
}

pub async fn create_key(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Json(req): Json<CreateKeyRequest>,
) -> Result<Json<CreateKeyResponse>, ApiError> {
    let env = KeyEnv::parse(&req.env).ok_or_else(|| {
        ApiError::BadRequest(format!("invalid env '{}', expected live or test", req.env))
    })?;
    let secret = generate_gateway_key(env, &state.master_key)?;
    let project_id = req
        .project_id
        .unwrap_or_else(|| state.default_project_id.clone());

    let row = state
        .stores
        .metadata
        .create_key(NewGatewayKey {
            id: Uuid::now_v7().to_string(),
            project_id: project_id.clone(),
            name: req.name.clone(),
            prefix: secret.prefix.clone(),
            hash: secret.hash.clone(),
            last4: secret.last4.clone(),
            scopes: req.scopes,
            expires_at: req.expires_at,
        })
        .await?;

    Ok(Json(CreateKeyResponse {
        id: row.id,
        name: row.name,
        prefix: row.prefix,
        last4: row.last4,
        secret: secret.plaintext,
        created_at: row.created_at,
    }))
}

#[derive(Debug, Serialize)]
pub struct KeySummary {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub prefix: String,
    pub last4: String,
    pub status: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ListKeysQuery {
    #[serde(default)]
    pub project_id: Option<String>,
}

pub async fn list_keys(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    axum::extract::Query(q): axum::extract::Query<ListKeysQuery>,
) -> Result<Json<Vec<KeySummary>>, ApiError> {
    let project_id = q.project_id.unwrap_or_else(|| state.default_project_id.clone());
    let rows = state.stores.metadata.list_keys(&project_id).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| KeySummary {
                id: r.id,
                project_id: r.project_id,
                name: r.name,
                prefix: r.prefix,
                last4: r.last4,
                status: r.status,
                created_at: r.created_at,
                last_used_at: r.last_used_at,
                expires_at: r.expires_at,
            })
            .collect(),
    ))
}

pub async fn revoke_key(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state.stores.metadata.revoke_key(&id).await?;
    Ok(Json(serde_json::json!({ "id": id, "status": "revoked" })))
}
