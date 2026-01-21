use async_trait::async_trait;
use chrono::Utc;
use rusqlite::OptionalExtension;

use crate::error::GatewayError;
use crate::logging::database::DatabaseLogger;
use crate::logging::time::{parse_beijing_string, to_beijing_string};
use crate::subscription::{SubscriptionPlan, SubscriptionPlansRecord, SubscriptionStore};

async fn ensure_scope_row(
    logger: &DatabaseLogger,
    scope: &str,
) -> Result<SubscriptionPlansRecord, GatewayError> {
    let now = Utc::now();
    let now_s = to_beijing_string(&now);
    let conn = logger.connection.lock().await;
    let _ = conn.execute(
        "INSERT OR IGNORE INTO subscription_plans (scope, content, updated_at, updated_by) VALUES (?1, ?2, ?3, NULL)",
        rusqlite::params![scope, "[]", &now_s],
    );
    drop(conn);
    // Fetch after ensure
    let conn = logger.connection.lock().await;
    let mut stmt = conn.prepare(
        "SELECT scope, content, updated_at, updated_by FROM subscription_plans WHERE scope = ?1",
    )?;
    let row_opt: Option<(String, String, String, Option<String>)> = stmt
        .query_row([scope], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .optional()?;
    let (scope, content, updated_at_s, updated_by) =
        row_opt.unwrap_or((scope.to_string(), "[]".to_string(), now_s, None::<String>));
    let updated_at = parse_beijing_string(&updated_at_s).unwrap_or(now);
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
impl SubscriptionStore for DatabaseLogger {
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
        let now_s = to_beijing_string(&now);
        let content = serde_json::to_string(&plans)?;
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO subscription_plans (scope, content, updated_at, updated_by) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["draft", &content, &now_s, updated_by.clone()],
        )?;
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
        let now_s = to_beijing_string(&now);
        let content = serde_json::to_string(&draft.plans)?;
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT OR REPLACE INTO subscription_plans (scope, content, updated_at, updated_by) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["published", &content, &now_s, updated_by.clone()],
        )?;
        Ok(SubscriptionPlansRecord {
            scope: "published".into(),
            plans: draft.plans,
            updated_at: now,
            updated_by,
        })
    }
}
