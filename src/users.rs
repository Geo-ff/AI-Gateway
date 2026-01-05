use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    Active,
    Inactive,
    Invited,
    Suspended,
}

impl UserStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            UserStatus::Active => "active",
            UserStatus::Inactive => "inactive",
            UserStatus::Invited => "invited",
            UserStatus::Suspended => "suspended",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(UserStatus::Active),
            "inactive" => Some(UserStatus::Inactive),
            "invited" => Some(UserStatus::Invited),
            "suspended" => Some(UserStatus::Suspended),
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
    #[serde(default = "default_status_invited")]
    pub status: UserStatus,
    #[serde(default = "default_role_admin")]
    pub role: UserRole,
}

fn default_status_invited() -> UserStatus {
    UserStatus::Invited
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
    pub status: Option<UserStatus>,
    #[serde(default)]
    pub role: Option<UserRole>,
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
            ("invited", UserStatus::Invited),
            ("suspended", UserStatus::Suspended),
        ] {
            assert_eq!(UserStatus::parse(s).unwrap().as_str(), expected.as_str());
        }
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
