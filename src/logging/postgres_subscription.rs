use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::GatewayError;
use crate::logging::postgres_store::PgLogStore;
use crate::subscription::{SubscriptionPlan, SubscriptionPlansRecord, SubscriptionStore};

async fn ensure_scope_row(
    store: &PgLogStore,
    scope: &str,
) -> Result<SubscriptionPlansRecord, GatewayError> {
    let now = Utc::now();
    let client = store.pool.pick();
    let _ = client
        .execute(
            "INSERT INTO subscription_plans (scope, content, updated_at, updated_by)
             VALUES ($1, $2, $3, NULL)
             ON CONFLICT (scope) DO NOTHING",
            &[&scope, &"[]", &now],
        )
        .await;

    let row_opt = client
        .query_opt(
            "SELECT scope, content, updated_at, updated_by FROM subscription_plans WHERE scope = $1",
            &[&scope],
        )
        .await
        .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;

    let Some(row) = row_opt else {
        return Ok(SubscriptionPlansRecord {
            scope: scope.to_string(),
            plans: Vec::new(),
            updated_at: now,
            updated_by: None,
        });
    };

    let scope: String = row.get(0);
    let content: String = row.get(1);
    let updated_at: DateTime<Utc> = row.get(2);
    let updated_by: Option<String> = row.get(3);
    let plans = serde_json::from_str::<Vec<SubscriptionPlan>>(content.as_str())
        .unwrap_or_else(|_| Vec::new());
    Ok(SubscriptionPlansRecord {
        scope,
        plans,
        updated_at,
        updated_by,
    })
}

#[async_trait]
impl SubscriptionStore for PgLogStore {
    async fn get_published_plans(&self) -> Result<SubscriptionPlansRecord, GatewayError> {
        ensure_scope_row(self, "published").await
    }

    async fn get_draft_plans(&self) -> Result<SubscriptionPlansRecord, GatewayError> {
        ensure_scope_row(self, "draft").await
    }

    async fn put_draft_plans(
        &self,
        plans: Vec<SubscriptionPlan>,
        updated_by: Option<String>,
    ) -> Result<SubscriptionPlansRecord, GatewayError> {
        let now = Utc::now();
        let content = serde_json::to_string(&plans)?;
        let client = self.pool.pick();
        client
            .execute(
                "INSERT INTO subscription_plans (scope, content, updated_at, updated_by)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (scope) DO UPDATE SET content = EXCLUDED.content, updated_at = EXCLUDED.updated_at, updated_by = EXCLUDED.updated_by",
                &[&"draft", &content, &now, &updated_by],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(SubscriptionPlansRecord {
            scope: "draft".into(),
            plans,
            updated_at: now,
            updated_by,
        })
    }

    async fn publish_draft(
        &self,
        updated_by: Option<String>,
    ) -> Result<SubscriptionPlansRecord, GatewayError> {
        let draft = self.get_draft_plans().await?;
        let now = Utc::now();
        let content = serde_json::to_string(&draft.plans)?;
        let client = self.pool.pick();
        client
            .execute(
                "INSERT INTO subscription_plans (scope, content, updated_at, updated_by)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (scope) DO UPDATE SET content = EXCLUDED.content, updated_at = EXCLUDED.updated_at, updated_by = EXCLUDED.updated_by",
                &[&"published", &content, &now, &updated_by],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(SubscriptionPlansRecord {
            scope: "published".into(),
            plans: draft.plans,
            updated_at: now,
            updated_by,
        })
    }
}
