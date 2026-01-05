use async_trait::async_trait;
use chrono::Utc;
use rusqlite::OptionalExtension;
use uuid::Uuid;

use crate::error::GatewayError;
use crate::logging::database::DatabaseLogger;
use crate::logging::time::{parse_beijing_string, to_beijing_string};
use crate::users::{CreateUserPayload, UpdateUserPayload, User, UserRole, UserStatus, UserStore};

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

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let status_s: String = row.get(6)?;
    let role_s: String = row.get(7)?;
    let created_at_s: String = row.get(8)?;
    let updated_at_s: String = row.get(9)?;
    Ok(User {
        id: row.get(0)?,
        first_name: row.get(1)?,
        last_name: row.get(2)?,
        username: row.get(3)?,
        email: row.get(4)?,
        phone_number: row.get(5)?,
        status: UserStatus::parse(&status_s).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(6, "status".into(), rusqlite::types::Type::Text)
        })?,
        role: UserRole::parse(&role_s).ok_or_else(|| {
            rusqlite::Error::InvalidColumnType(7, "role".into(), rusqlite::types::Type::Text)
        })?,
        created_at: parse_beijing_string(&created_at_s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?,
        updated_at: parse_beijing_string(&updated_at_s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        })?,
    })
}

#[async_trait]
impl UserStore for DatabaseLogger {
    async fn create_user(&self, payload: CreateUserPayload) -> Result<User, GatewayError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let first_name = payload.first_name.unwrap_or_default();
        let last_name = payload.last_name.unwrap_or_default();
        let phone_number = payload.phone_number.unwrap_or_default();

        let mut username = payload
            .username
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| default_username_from_email(&payload.email));

        let conn = self.connection.lock().await;
        // best-effort: avoid username collision
        for _ in 0..5 {
            let exists: Option<String> = conn
                .query_row(
                    "SELECT id FROM users WHERE username = ?1",
                    [&username],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                break;
            }
            username = format!("{}-{}", username, &id[..8]);
        }

        conn.execute(
            "INSERT INTO users (id, first_name, last_name, username, email, phone_number, status, role, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                &id,
                &first_name,
                &last_name,
                &username,
                &payload.email,
                &phone_number,
                payload.status.as_str(),
                payload.role.as_str(),
                to_beijing_string(&now),
                to_beijing_string(&now),
            ],
        )?;

        Ok(User {
            id,
            first_name,
            last_name,
            username,
            email: payload.email,
            phone_number,
            status: payload.status,
            role: payload.role,
            created_at: now,
            updated_at: now,
        })
    }

    async fn update_user(
        &self,
        id: &str,
        payload: UpdateUserPayload,
    ) -> Result<Option<User>, GatewayError> {
        let conn = self.connection.lock().await;

        let mut stmt = conn.prepare(
            "SELECT id, first_name, last_name, username, email, phone_number, status, role, created_at, updated_at FROM users WHERE id = ?1",
        )?;
        let row = stmt
            .query_row([id], |row| row_to_user(row))
            .optional()?;
        let Some(mut user) = row else {
            return Ok(None);
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

        conn.execute(
            "UPDATE users SET first_name = ?2, last_name = ?3, username = ?4, email = ?5, phone_number = ?6, status = ?7, role = ?8, updated_at = ?9 WHERE id = ?1",
            rusqlite::params![
                &user.id,
                &user.first_name,
                &user.last_name,
                &user.username,
                &user.email,
                &user.phone_number,
                user.status.as_str(),
                user.role.as_str(),
                to_beijing_string(&user.updated_at),
            ],
        )?;

        Ok(Some(user))
    }

    async fn get_user(&self, id: &str) -> Result<Option<User>, GatewayError> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, first_name, last_name, username, email, phone_number, status, role, created_at, updated_at FROM users WHERE id = ?1",
        )?;
        let row = stmt
            .query_row([id], |row| row_to_user(row))
            .optional()?;
        Ok(row)
    }

    async fn list_users(&self) -> Result<Vec<User>, GatewayError> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, first_name, last_name, username, email, phone_number, status, role, created_at, updated_at FROM users ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| row_to_user(row))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    async fn delete_user(&self, id: &str) -> Result<bool, GatewayError> {
        let conn = self.connection.lock().await;
        let rows = conn.execute("DELETE FROM users WHERE id = ?1", [id])?;
        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::{UserRole, UserStatus};
    use tempfile::tempdir;

    #[tokio::test]
    async fn sqlite_user_crud_works() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db_path = db_path.to_str().unwrap();
        let db = DatabaseLogger::new(db_path).await.unwrap();

        let created = db
            .create_user(CreateUserPayload {
                first_name: Some("Alice".into()),
                last_name: Some("Liddell".into()),
                username: None,
                email: "alice@example.com".into(),
                phone_number: Some("+1-555-0000".into()),
                status: UserStatus::Active,
                role: UserRole::Manager,
            })
            .await
            .unwrap();
        assert_eq!(created.email, "alice@example.com");
        assert_eq!(created.username, "alice");

        let fetched = db.get_user(&created.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.status.as_str(), "active");
        assert_eq!(fetched.role.as_str(), "manager");

        let updated = db
            .update_user(
                &created.id,
                UpdateUserPayload {
                    first_name: Some("Alicia".into()),
                    last_name: None,
                    username: None,
                    email: None,
                    phone_number: None,
                    status: Some(UserStatus::Suspended),
                    role: None,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.first_name, "Alicia");
        assert_eq!(updated.status.as_str(), "suspended");

        let users = db.list_users().await.unwrap();
        assert_eq!(users.len(), 1);

        let deleted = db.delete_user(&created.id).await.unwrap();
        assert!(deleted);
        let missing = db.get_user(&created.id).await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn sqlite_username_collision_is_avoided() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db_path = db_path.to_str().unwrap();
        let db = DatabaseLogger::new(db_path).await.unwrap();

        let u1 = db
            .create_user(CreateUserPayload {
                first_name: None,
                last_name: None,
                username: Some("dup".into()),
                email: "dup1@example.com".into(),
                phone_number: None,
                status: UserStatus::Invited,
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        assert_eq!(u1.username, "dup");

        let u2 = db
            .create_user(CreateUserPayload {
                first_name: None,
                last_name: None,
                username: Some("dup".into()),
                email: "dup2@example.com".into(),
                phone_number: None,
                status: UserStatus::Invited,
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        assert_ne!(u2.username, "dup");
    }
}
