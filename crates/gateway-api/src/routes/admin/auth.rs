use axum::extract::State;
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use gateway_core::security::{hash_password, verify_password};
use gateway_storage::models::NewAdminUser;

use crate::auth::AdminPrincipal;
use crate::error::ApiError;
use crate::state::AppState;

const SESSION_TTL_SECS: i64 = 12 * 60 * 60;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub expires_at: i64,
}

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let user = state
        .stores
        .metadata
        .find_admin_user(&req.username)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let ok = verify_password(&req.password, &user.password_hash)?;
    if !ok {
        return Err(ApiError::Unauthorized);
    }
    let now = Utc::now().timestamp_millis();
    let _ = state
        .stores
        .metadata
        .touch_admin_last_login(&user.id, now)
        .await;
    let token = state
        .admin_signer
        .issue(&user.id, &user.username, SESSION_TTL_SECS)?;
    let expires_at = Utc::now().timestamp() + SESSION_TTL_SECS;
    Ok(Json(LoginResponse {
        token,
        username: user.username,
        expires_at,
    }))
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub principal: &'static str,
    pub username: Option<String>,
    pub id: Option<String>,
}

pub async fn me(principal: AdminPrincipal) -> Json<MeResponse> {
    match principal {
        AdminPrincipal::Root => Json(MeResponse {
            principal: "root",
            username: None,
            id: None,
        }),
        AdminPrincipal::User { id, username } => Json(MeResponse {
            principal: "user",
            username: Some(username),
            id: Some(id),
        }),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateAdminRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct CreateAdminResponse {
    pub id: String,
    pub username: String,
}

/// Create a new admin user. Requires an authenticated admin (root or
/// existing user). Used for bootstrap — first admin must be created with
/// the root token.
pub async fn create_admin(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
    Json(req): Json<CreateAdminRequest>,
) -> Result<Json<CreateAdminResponse>, ApiError> {
    if req.username.is_empty() || req.password.len() < 6 {
        return Err(ApiError::BadRequest(
            "username required, password >= 6 chars".into(),
        ));
    }
    let phc = hash_password(&req.password)?;
    let user = state
        .stores
        .metadata
        .create_admin_user(NewAdminUser {
            id: Uuid::now_v7().to_string(),
            username: req.username,
            password_hash: phc,
        })
        .await?;
    Ok(Json(CreateAdminResponse {
        id: user.id,
        username: user.username,
    }))
}

#[derive(Debug, Serialize)]
pub struct AdminUserSummary {
    pub id: String,
    pub username: String,
    pub created_at: i64,
    pub last_login_at: Option<i64>,
}

pub async fn list_admins(
    State(state): State<AppState>,
    _principal: AdminPrincipal,
) -> Result<Json<Vec<AdminUserSummary>>, ApiError> {
    let users = state.stores.metadata.list_admin_users().await?;
    Ok(Json(
        users
            .into_iter()
            .map(|u| AdminUserSummary {
                id: u.id,
                username: u.username,
                created_at: u.created_at,
                last_login_at: u.last_login_at,
            })
            .collect(),
    ))
}
