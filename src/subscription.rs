use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionPlan {
    pub plan_id: String,
    pub name: String,
    #[serde(default)]
    pub price_cny: Option<f64>,
    pub credits: f64,
    #[serde(default)]
    pub tagline: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionPlansRecord {
    pub scope: String, // "draft" | "published"
    pub plans: Vec<SubscriptionPlan>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

#[async_trait]
pub trait SubscriptionStore: Send + Sync {
    async fn get_published_plans(&self) -> Result<SubscriptionPlansRecord, GatewayError>;
    async fn get_draft_plans(&self) -> Result<SubscriptionPlansRecord, GatewayError>;
    async fn put_draft_plans(
        &self,
        plans: Vec<SubscriptionPlan>,
        updated_by: Option<String>,
    ) -> Result<SubscriptionPlansRecord, GatewayError>;
    async fn publish_draft(
        &self,
        updated_by: Option<String>,
    ) -> Result<SubscriptionPlansRecord, GatewayError>;
}
