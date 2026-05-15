use std::sync::Arc;

use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use tokio::sync::Mutex;

use crate::error::{Result, StorageError};
use crate::models::*;
use crate::traits::MetadataStore;

/// SQLite implementation. All writes funnel through a single mutex (shared
/// with the log store) to keep `database is locked` errors away under load.
pub struct SqliteMetadataStore {
    pool: SqlitePool,
    write_lock: Arc<Mutex<()>>,
}

impl SqliteMetadataStore {
    pub fn new(pool: SqlitePool, write_lock: Arc<Mutex<()>>) -> Self {
        Self { pool, write_lock }
    }
}

#[async_trait]
impl MetadataStore for SqliteMetadataStore {
    async fn create_project(&self, p: NewProject) -> Result<Project> {
        let created_at = now_ms();
        let _w = self.write_lock.lock().await;
        sqlx::query("INSERT INTO projects (id, name, created_at) VALUES (?, ?, ?)")
            .bind(&p.id)
            .bind(&p.name)
            .bind(created_at)
            .execute(&self.pool)
            .await?;
        Ok(Project {
            id: p.id,
            name: p.name,
            created_at,
        })
    }

    async fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let row = sqlx::query("SELECT id, name, created_at FROM projects WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| Project {
            id: r.get("id"),
            name: r.get("name"),
            created_at: r.get("created_at"),
        }))
    }

    async fn list_projects(&self) -> Result<Vec<Project>> {
        let rows = sqlx::query("SELECT id, name, created_at FROM projects ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| Project {
                id: r.get("id"),
                name: r.get("name"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn create_key(&self, k: NewGatewayKey) -> Result<GatewayKeyRow> {
        let created_at = now_ms();
        let scopes_json = serde_json::to_string(&k.scopes)?;
        let _w = self.write_lock.lock().await;
        sqlx::query(
            r#"
            INSERT INTO gateway_keys
                (id, project_id, name, prefix, hash, last4, scopes, status, expires_at, last_used_at, created_at, revoked_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, NULL, ?, NULL)
            "#,
        )
        .bind(&k.id)
        .bind(&k.project_id)
        .bind(&k.name)
        .bind(&k.prefix)
        .bind(&k.hash)
        .bind(&k.last4)
        .bind(&scopes_json)
        .bind(k.expires_at)
        .bind(created_at)
        .execute(&self.pool)
        .await?;

        Ok(GatewayKeyRow {
            id: k.id,
            project_id: k.project_id,
            name: k.name,
            prefix: k.prefix,
            hash: k.hash,
            last4: k.last4,
            scopes: k.scopes,
            status: "active".to_string(),
            expires_at: k.expires_at,
            last_used_at: None,
            created_at,
            revoked_at: None,
        })
    }

    async fn list_keys(&self, project_id: &str) -> Result<Vec<GatewayKeyRow>> {
        let rows = sqlx::query(
            r#"
            SELECT id, project_id, name, prefix, hash, last4, scopes, status,
                   expires_at, last_used_at, created_at, revoked_at
              FROM gateway_keys
             WHERE project_id = ?
             ORDER BY created_at DESC
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_gateway_key).collect()
    }

    async fn find_key_by_hash(&self, hash: &[u8]) -> Result<Option<GatewayKeyRow>> {
        let row = sqlx::query(
            r#"
            SELECT id, project_id, name, prefix, hash, last4, scopes, status,
                   expires_at, last_used_at, created_at, revoked_at
              FROM gateway_keys
             WHERE hash = ?
            "#,
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_gateway_key).transpose()
    }

    async fn revoke_key(&self, id: &str) -> Result<()> {
        let ts = now_ms();
        let _w = self.write_lock.lock().await;
        let res = sqlx::query(
            "UPDATE gateway_keys SET status = 'revoked', revoked_at = ? WHERE id = ?",
        )
        .bind(ts)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    async fn touch_key_last_used(&self, id: &str, ts: Timestamp) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("UPDATE gateway_keys SET last_used_at = ? WHERE id = ?")
            .bind(ts)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn put_provider_credential(&self, c: ProviderCredential) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query(
            r#"
            INSERT INTO provider_credentials (id, project_id, provider, name, encrypted_key, status, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                name = excluded.name,
                encrypted_key = excluded.encrypted_key,
                status = excluded.status
            "#,
        )
        .bind(&c.id)
        .bind(&c.project_id)
        .bind(&c.provider)
        .bind(&c.name)
        .bind(&c.encrypted_key)
        .bind(&c.status)
        .bind(c.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_provider_credentials(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProviderCredential>> {
        let rows = sqlx::query(
            r#"
            SELECT id, project_id, provider, name, encrypted_key, status, created_at
              FROM provider_credentials
             WHERE project_id = ?
             ORDER BY created_at DESC
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ProviderCredential {
                id: r.get("id"),
                project_id: r.get("project_id"),
                provider: r.get("provider"),
                name: r.get("name"),
                encrypted_key: r.get("encrypted_key"),
                status: r.get("status"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn delete_provider_credential(&self, id: &str) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let res = sqlx::query("DELETE FROM provider_credentials WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    async fn upsert_routes(
        &self,
        project_id: &str,
        cfg: RoutesConfig,
        version: i64,
    ) -> Result<()> {
        let json = serde_json::to_string(&cfg.raw)?;
        let updated_at = now_ms();
        let _w = self.write_lock.lock().await;
        sqlx::query(
            r#"
            INSERT INTO routes (project_id, config, version, updated_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(project_id) DO UPDATE SET
                config = excluded.config,
                version = excluded.version,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(project_id)
        .bind(&json)
        .bind(version)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_routes(&self, project_id: &str) -> Result<Option<(RoutesConfig, i64)>> {
        let row = sqlx::query("SELECT config, version FROM routes WHERE project_id = ?")
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let cfg_str: String = row.get("config");
        let version: i64 = row.get("version");
        let raw: serde_json::Value = serde_json::from_str(&cfg_str)?;
        Ok(Some((RoutesConfig { raw }, version)))
    }

    async fn upsert_budget(&self, b: Budget) -> Result<()> {
        let thresholds = serde_json::to_string(&b.thresholds)?;
        let _w = self.write_lock.lock().await;
        sqlx::query(
            r#"
            INSERT INTO budgets (id, name, target_type, target_id, period, amount_usd, thresholds, status)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                target_type = excluded.target_type,
                target_id = excluded.target_id,
                period = excluded.period,
                amount_usd = excluded.amount_usd,
                thresholds = excluded.thresholds,
                status = excluded.status
            "#,
        )
        .bind(&b.id)
        .bind(&b.name)
        .bind(&b.target_type)
        .bind(&b.target_id)
        .bind(&b.period)
        .bind(b.amount_usd)
        .bind(&thresholds)
        .bind(&b.status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_budgets(&self) -> Result<Vec<Budget>> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, target_type, target_id, period, amount_usd, thresholds, status
              FROM budgets
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_budget).collect()
    }

    async fn get_budget(&self, id: &str) -> Result<Option<Budget>> {
        let row = sqlx::query(
            r#"
            SELECT id, name, target_type, target_id, period, amount_usd, thresholds, status
              FROM budgets WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_budget).transpose()
    }

    async fn create_admin_user(&self, u: NewAdminUser) -> Result<AdminUser> {
        let created_at = now_ms();
        let _w = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO admin_users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&u.id)
        .bind(&u.username)
        .bind(&u.password_hash)
        .bind(created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                StorageError::Conflict(format!("admin user '{}' already exists", u.username))
            }
            _ => StorageError::Database(e),
        })?;
        Ok(AdminUser {
            id: u.id,
            username: u.username,
            password_hash: u.password_hash,
            created_at,
            last_login_at: None,
        })
    }

    async fn find_admin_user(&self, username: &str) -> Result<Option<AdminUser>> {
        let row = sqlx::query(
            r#"
            SELECT id, username, password_hash, created_at, last_login_at
              FROM admin_users WHERE username = ?
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| AdminUser {
            id: r.get("id"),
            username: r.get("username"),
            password_hash: r.get("password_hash"),
            created_at: r.get("created_at"),
            last_login_at: r.get("last_login_at"),
        }))
    }

    async fn list_admin_users(&self) -> Result<Vec<AdminUser>> {
        let rows = sqlx::query(
            r#"
            SELECT id, username, password_hash, created_at, last_login_at
              FROM admin_users ORDER BY created_at ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| AdminUser {
                id: r.get("id"),
                username: r.get("username"),
                password_hash: r.get("password_hash"),
                created_at: r.get("created_at"),
                last_login_at: r.get("last_login_at"),
            })
            .collect())
    }

    async fn touch_admin_last_login(&self, id: &str, ts: Timestamp) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("UPDATE admin_users SET last_login_at = ? WHERE id = ?")
            .bind(ts)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn row_to_gateway_key(r: sqlx::sqlite::SqliteRow) -> Result<GatewayKeyRow> {
    let scopes_str: String = r.get("scopes");
    let scopes: Vec<String> = serde_json::from_str(&scopes_str)?;
    Ok(GatewayKeyRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        name: r.get("name"),
        prefix: r.get("prefix"),
        hash: r.get("hash"),
        last4: r.get("last4"),
        scopes,
        status: r.get("status"),
        expires_at: r.get("expires_at"),
        last_used_at: r.get("last_used_at"),
        created_at: r.get("created_at"),
        revoked_at: r.get("revoked_at"),
    })
}

fn row_to_budget(r: sqlx::sqlite::SqliteRow) -> Result<Budget> {
    let thresholds_str: String = r.get("thresholds");
    let thresholds: serde_json::Value = serde_json::from_str(&thresholds_str)?;
    Ok(Budget {
        id: r.get("id"),
        name: r.get("name"),
        target_type: r.get("target_type"),
        target_id: r.get("target_id"),
        period: r.get("period"),
        amount_usd: r.get("amount_usd"),
        thresholds,
        status: r.get("status"),
    })
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
