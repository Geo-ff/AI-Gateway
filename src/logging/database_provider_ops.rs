use rusqlite::Result;

use chrono::Utc;

use crate::logging::time::{parse_beijing_string, to_beijing_string};
use crate::logging::types::ProviderOpLog;

use super::database::DatabaseLogger;

impl DatabaseLogger {
    pub async fn log_provider_op(&self, op: ProviderOpLog) -> Result<i64> {
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO provider_ops_logs (timestamp, operation, provider, details)
             VALUES (?1, ?2, ?3, ?4)",
            (
                to_beijing_string(&op.timestamp),
                &op.operation,
                &op.provider,
                &op.details,
            ),
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub async fn get_provider_ops_logs(
        &self,
        limit: i32,
        cursor: Option<i64>,
    ) -> Result<Vec<ProviderOpLog>> {
        let conn = self.connection.lock().await;
        let mut stmt = if cursor.is_some() {
            conn.prepare(
                "SELECT id, timestamp, operation, provider, details
                 FROM provider_ops_logs
                 WHERE id < ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )?
        } else {
            conn.prepare(
                "SELECT id, timestamp, operation, provider, details
                 FROM provider_ops_logs
                 ORDER BY id DESC
                 LIMIT ?1",
            )?
        };

        let rows = if let Some(cursor_id) = cursor {
            stmt.query_map(rusqlite::params![cursor_id, limit], map_provider_op_row)?
        } else {
            stmt.query_map([limit], map_provider_op_row)?
        };

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

fn map_provider_op_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderOpLog> {
    let ts: String = row.get(1)?;
    Ok(ProviderOpLog {
        id: Some(row.get(0)?),
        timestamp: parse_beijing_string(&ts).unwrap_or_else(|_| Utc::now()),
        operation: row.get(2)?,
        provider: row.get(3)?,
        details: row.get(4)?,
    })
}
