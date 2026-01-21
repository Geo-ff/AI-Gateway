use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::balance::{BalanceStore, BalanceTransaction, BalanceTransactionKind};
use crate::error::GatewayError;
use crate::logging::database::DatabaseLogger;
use crate::logging::time::{parse_beijing_string, to_beijing_string};

fn row_to_transaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<BalanceTransaction> {
    let kind_s: String = row.get(2)?;
    let created_at_s: String = row.get(4)?;
    let kind = BalanceTransactionKind::parse(&kind_s).ok_or_else(|| {
        rusqlite::Error::InvalidColumnType(2, "kind".into(), rusqlite::types::Type::Text)
    })?;
    let created_at = parse_beijing_string(&created_at_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;
    Ok(BalanceTransaction {
        id: row.get(0)?,
        user_id: row.get(1)?,
        kind,
        amount: row.get(3)?,
        created_at,
        meta: row.get(5)?,
    })
}

#[async_trait]
impl BalanceStore for DatabaseLogger {
    async fn create_transaction(
        &self,
        user_id: &str,
        kind: BalanceTransactionKind,
        amount: f64,
        meta: Option<String>,
    ) -> Result<BalanceTransaction, GatewayError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO balance_transactions (id, user_id, kind, amount, created_at, meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                &id,
                user_id,
                kind.as_str(),
                amount,
                to_beijing_string(&now),
                meta.clone(),
            ],
        )?;
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
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, user_id, kind, amount, created_at, meta
             FROM balance_transactions
             WHERE user_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![user_id, limit, offset],
            row_to_transaction,
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}
