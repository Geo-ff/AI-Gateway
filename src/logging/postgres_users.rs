use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::error::GatewayError;
use crate::logging::postgres_store::PgLogStore;
use crate::users::{CreateUserPayload, UpdateUserPayload, User, UserAuthRecord, UserRole, UserStore, hash_password};

fn default_username_from_email(email: &str) -> String {
    let base = email
        .split('@')
        .next()
        .unwrap_or("user")
        .trim()
        .to_lowercase();
    if base.is_empty() {
        "user".to_string()
    } else {
        base
    }
}

#[async_trait]
impl UserStore for PgLogStore {
    async fn create_user(&self, payload: CreateUserPayload) -> Result<User, GatewayError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let first_name = payload.first_name.unwrap_or_default();
        let last_name = payload.last_name.unwrap_or_default();
        let phone_number = payload.phone_number.unwrap_or_default();
        let password_hash = payload
            .password
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(hash_password)
            .transpose()?;
        let username = payload
            .username
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| default_username_from_email(&payload.email));

        let client = self.pool.pick();
        let is_first_user = client
            .query_opt("SELECT 1 FROM users LIMIT 1", &[])
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?
            .is_none();
        let role = if is_first_user {
            UserRole::Superadmin
        } else if matches!(payload.role, UserRole::Superadmin) {
            UserRole::Admin
        } else {
            payload.role
        };

        client
            .execute(
                "INSERT INTO users (id, first_name, last_name, username, email, phone_number, password_hash, status, role, created_at, updated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
                &[
                    &id,
                    &first_name,
                    &last_name,
                    &username,
                    &payload.email,
                    &phone_number,
                    &password_hash,
                    &payload.status.as_str(),
                    &role.as_str(),
                    &now,
                    &now,
                ],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;

        Ok(User {
            id,
            first_name,
            last_name,
            username,
            email: payload.email,
            phone_number,
            status: payload.status,
            role,
            created_at: now,
            updated_at: now,
        })
    }

    async fn update_user(
        &self,
        id: &str,
        payload: UpdateUserPayload,
    ) -> Result<Option<User>, GatewayError> {
        let client = self.pool.pick();

        let row_opt = client
            .query_opt(
                "SELECT id, first_name, last_name, username, email, phone_number, status, role, created_at, updated_at FROM users WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(row) = row_opt else {
            return Ok(None);
        };

        let mut user = User {
            id: row.get(0),
            first_name: row.get(1),
            last_name: row.get(2),
            username: row.get(3),
            email: row.get(4),
            phone_number: row.get(5),
            status: crate::users::UserStatus::parse(row.get::<usize, String>(6).as_str())
                .ok_or_else(|| GatewayError::Config("invalid user status".into()))?,
            role: crate::users::UserRole::parse(row.get::<usize, String>(7).as_str())
                .ok_or_else(|| GatewayError::Config("invalid user role".into()))?,
            created_at: row.get(8),
            updated_at: row.get(9),
        };

        if let Some(v) = payload.first_name {
            user.first_name = v;
        }
        if let Some(v) = payload.last_name {
            user.last_name = v;
        }
        if let Some(v) = payload.username {
            user.username = v;
        }
        if let Some(v) = payload.email {
            user.email = v;
        }
        if let Some(v) = payload.phone_number {
            user.phone_number = v;
        }
        if let Some(v) = payload.status {
            user.status = v;
        }
        if let Some(v) = payload.role {
            user.role = v;
        }
        user.updated_at = Utc::now();
        let password_hash_update = payload
            .password
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(hash_password)
            .transpose()?;

        client
            .execute(
                "UPDATE users SET first_name = $2, last_name = $3, username = $4, email = $5, phone_number = $6, status = $7, role = $8, password_hash = COALESCE($9, password_hash), updated_at = $10 WHERE id = $1",
                &[
                    &user.id,
                    &user.first_name,
                    &user.last_name,
                    &user.username,
                    &user.email,
                    &user.phone_number,
                    &user.status.as_str(),
                    &user.role.as_str(),
                    &password_hash_update,
                    &user.updated_at,
                ],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;

        Ok(Some(user))
    }

    async fn get_user(&self, id: &str) -> Result<Option<User>, GatewayError> {
        let client = self.pool.pick();
        let row_opt = client
            .query_opt(
                "SELECT id, first_name, last_name, username, email, phone_number, status, role, created_at, updated_at FROM users WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(row) = row_opt else {
            return Ok(None);
        };
        Ok(Some(User {
            id: row.get(0),
            first_name: row.get(1),
            last_name: row.get(2),
            username: row.get(3),
            email: row.get(4),
            phone_number: row.get(5),
            status: crate::users::UserStatus::parse(row.get::<usize, String>(6).as_str())
                .ok_or_else(|| GatewayError::Config("invalid user status".into()))?,
            role: crate::users::UserRole::parse(row.get::<usize, String>(7).as_str())
                .ok_or_else(|| GatewayError::Config("invalid user role".into()))?,
            created_at: row.get(8),
            updated_at: row.get(9),
        }))
    }

    async fn get_auth_by_email(&self, email: &str) -> Result<Option<UserAuthRecord>, GatewayError> {
        let client = self.pool.pick();
        let row_opt = client
            .query_opt(
                "SELECT id, email, role, password_hash FROM users WHERE email = $1 LIMIT 1",
                &[&email],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let Some(row) = row_opt else {
            return Ok(None);
        };
        let role = UserRole::parse(row.get::<usize, String>(2).as_str())
            .ok_or_else(|| GatewayError::Config("invalid user role".into()))?;
        Ok(Some(UserAuthRecord {
            id: row.get(0),
            email: row.get(1),
            role,
            password_hash: row.get(3),
        }))
    }

    async fn any_users(&self) -> Result<bool, GatewayError> {
        let client = self.pool.pick();
        Ok(client
            .query_opt("SELECT 1 FROM users LIMIT 1", &[])
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?
            .is_some())
    }

    async fn list_users(&self) -> Result<Vec<User>, GatewayError> {
        let client = self.pool.pick();
        let rows = client
            .query(
                "SELECT id, first_name, last_name, username, email, phone_number, status, role, created_at, updated_at FROM users ORDER BY created_at DESC",
                &[],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(User {
                id: row.get(0),
                first_name: row.get(1),
                last_name: row.get(2),
                username: row.get(3),
                email: row.get(4),
                phone_number: row.get(5),
                status: crate::users::UserStatus::parse(row.get::<usize, String>(6).as_str())
                    .ok_or_else(|| GatewayError::Config("invalid user status".into()))?,
                role: crate::users::UserRole::parse(row.get::<usize, String>(7).as_str())
                    .ok_or_else(|| GatewayError::Config("invalid user role".into()))?,
                created_at: row.get(8),
                updated_at: row.get(9),
            });
        }
        Ok(out)
    }

    async fn delete_user(&self, id: &str) -> Result<bool, GatewayError> {
        let client = self.pool.pick();
        let affected = client
            .execute("DELETE FROM users WHERE id = $1", &[&id])
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(affected > 0)
    }
}
