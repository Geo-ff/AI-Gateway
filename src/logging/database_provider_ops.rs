use rusqlite::Result;

use crate::logging::time::to_beijing_string;
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
}
