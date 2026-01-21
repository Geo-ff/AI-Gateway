use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BalanceTransactionKind {
    Topup,
    Spend,
}

impl BalanceTransactionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BalanceTransactionKind::Topup => "topup",
            BalanceTransactionKind::Spend => "spend",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "topup" => Some(BalanceTransactionKind::Topup),
            "spend" => Some(BalanceTransactionKind::Spend),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BalanceTransaction {
    pub id: String,
    pub user_id: String,
    pub kind: BalanceTransactionKind,
    pub amount: f64,
    pub created_at: DateTime<Utc>,
    pub meta: Option<String>,
}

#[async_trait]
pub trait BalanceStore: Send + Sync {
    async fn create_transaction(
        &self,
        user_id: &str,
        kind: BalanceTransactionKind,
        amount: f64,
        meta: Option<String>,
    ) -> Result<BalanceTransaction, GatewayError>;

    async fn list_transactions(
        &self,
        user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BalanceTransaction>, GatewayError>;
}
