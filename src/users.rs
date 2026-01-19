use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;

pub(crate) fn hash_password(password: &str) -> Result<String, GatewayError> {
    use argon2::{
        Argon2, PasswordHasher,
        password_hash::{SaltString, rand_core::OsRng},
    };

    let salt = SaltString::generate(&mut OsRng);
    let hashed = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| {
            GatewayError::Io(std::io::Error::other(format!(
                "password hash failed: {}",
                e
            )))
        })?
        .to_string();
    Ok(hashed)
}

pub(crate) fn verify_password(password: &str, password_hash: &str) -> Result<bool, GatewayError> {
    use argon2::{Argon2, PasswordVerifier, password_hash::PasswordHash};

    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return Ok(false);
    };
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    Active,
    Inactive,
    Disabled,
}

impl<'de> serde::Deserialize<'de> for UserStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).ok_or_else(|| {
            serde::de::Error::unknown_variant(
                &s,
                &["active", "inactive", "disabled", "invited", "suspended"],
            )
        })
    }
}

impl UserStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            UserStatus::Active => "active",
            UserStatus::Inactive => "inactive",
            UserStatus::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(UserStatus::Active),
            "inactive" => Some(UserStatus::Inactive),
            "disabled" => Some(UserStatus::Disabled),
            // 向后兼容：将旧的 suspended 和 invited 映射到新状态
            "suspended" => Some(UserStatus::Disabled),
            "invited" => Some(UserStatus::Inactive),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Superadmin,
    Admin,
    Cashier,
    Manager,
}

impl UserRole {
    pub fn as_str(self) -> &'static str {
        match self {
            UserRole::Superadmin => "superadmin",
            UserRole::Admin => "admin",
            UserRole::Cashier => "cashier",
            UserRole::Manager => "manager",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "superadmin" => Some(UserRole::Superadmin),
            "admin" => Some(UserRole::Admin),
            "cashier" => Some(UserRole::Cashier),
            "manager" => Some(UserRole::Manager),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    pub username: String,
    pub email: String,
    pub phone_number: String,
    pub status: UserStatus,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateUserPayload {
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    pub email: String,
    #[serde(default)]
    pub phone_number: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_status_invited")]
    pub status: UserStatus,
    #[serde(default = "default_role_admin")]
    pub role: UserRole,
    #[serde(default)]
    pub is_anonymous: bool,
}

fn default_status_invited() -> UserStatus {
    UserStatus::Inactive
}

fn default_role_admin() -> UserRole {
    UserRole::Admin
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserPayload {
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone_number: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub status: Option<UserStatus>,
    #[serde(default)]
    pub role: Option<UserRole>,
}

#[derive(Debug, Clone)]
pub struct UserAuthRecord {
    pub id: String,
    pub email: String,
    pub role: UserRole,
    pub password_hash: Option<String>,
}

#[async_trait]
pub trait UserStore: Send + Sync {
    async fn create_user(&self, payload: CreateUserPayload) -> Result<User, GatewayError>;
    async fn update_user(
        &self,
        id: &str,
        payload: UpdateUserPayload,
    ) -> Result<Option<User>, GatewayError>;
    async fn get_user(&self, id: &str) -> Result<Option<User>, GatewayError>;
    async fn get_auth_by_email(&self, email: &str) -> Result<Option<UserAuthRecord>, GatewayError>;
    async fn any_users(&self) -> Result<bool, GatewayError>;
    async fn list_users(&self) -> Result<Vec<User>, GatewayError>;
    async fn delete_user(&self, id: &str) -> Result<bool, GatewayError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_status_roundtrip() {
        for (s, expected) in [
            ("active", UserStatus::Active),
            ("inactive", UserStatus::Inactive),
            ("disabled", UserStatus::Disabled),
        ] {
            assert_eq!(UserStatus::parse(s).unwrap().as_str(), expected.as_str());
        }
        // 测试向后兼容
        assert_eq!(UserStatus::parse("suspended").unwrap().as_str(), "disabled");
        assert_eq!(UserStatus::parse("invited").unwrap().as_str(), "inactive");
        assert!(UserStatus::parse("nope").is_none());
    }

    #[test]
    fn user_role_roundtrip() {
        for (s, expected) in [
            ("superadmin", UserRole::Superadmin),
            ("admin", UserRole::Admin),
            ("cashier", UserRole::Cashier),
            ("manager", UserRole::Manager),
        ] {
            assert_eq!(UserRole::parse(s).unwrap().as_str(), expected.as_str());
        }
        assert!(UserRole::parse("nope").is_none());
    }
}
