use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::balance::{BalanceStore, BalanceTransaction, BalanceTransactionKind};
use crate::error::GatewayError;
use crate::logging::postgres_store::PgLogStore;

#[async_trait]
impl BalanceStore for PgLogStore {
    async fn create_transaction(
        &self,
        user_id: &str,
        kind: BalanceTransactionKind,
        amount: f64,
        meta: Option<String>,
    ) -> Result<BalanceTransaction, GatewayError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let client = self.pool.pick();
        client
            .execute(
                "INSERT INTO balance_transactions (id, user_id, kind, amount, created_at, meta) VALUES ($1,$2,$3,$4,$5,$6)",
                &[&id, &user_id, &kind.as_str(), &amount, &now, &meta],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        Ok(BalanceTransaction {
            id,
            user_id: user_id.to_string(),
            kind,
            amount,
            created_at: now,
            meta,
        })
    }

    async fn list_transactions(
        &self,
        user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BalanceTransaction>, GatewayError> {
        let client = self.pool.pick();
        let rows = client
            .query(
                "SELECT id, user_id, kind, amount, created_at, meta
                 FROM balance_transactions
                 WHERE user_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2 OFFSET $3",
                &[&user_id, &limit, &offset],
            )
            .await
            .map_err(|e| GatewayError::Config(format!("DB error: {}", e)))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let kind_s: String = row.get(2);
            let kind = BalanceTransactionKind::parse(&kind_s)
                .ok_or_else(|| GatewayError::Config("invalid balance transaction kind".into()))?;
            let created_at: DateTime<Utc> = row.get(4);
            out.push(BalanceTransaction {
                id: row.get(0),
                user_id: row.get(1),
                kind,
                amount: row.get(3),
                created_at,
                meta: row.get(5),
            });
        }
        Ok(out)
    }
}
