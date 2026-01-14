use chrono::{DateTime, Utc};
use rusqlite::Result;

use crate::logging::time::to_beijing_string;

use super::database::DatabaseLogger;

impl DatabaseLogger {
    pub async fn list_model_redirects(
        &self,
        provider: &str,
    ) -> Result<Vec<(String, String)>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT source_model, target_model FROM model_redirects WHERE provider = ?1 ORDER BY source_model",
        )?;
        let rows = stmt.query_map([provider], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub async fn replace_model_redirects(
        &self,
        provider: &str,
        redirects: &[(String, String)],
        now: DateTime<Utc>,
    ) -> Result<()> {
        let conn = self.connection.lock().await;
        let now_s = to_beijing_string(&now);
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM model_redirects WHERE provider = ?1", [provider])?;
        for (source, target) in redirects {
            tx.execute(
                "INSERT INTO model_redirects (provider, source_model, target_model, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (provider, source, target, &now_s, &now_s),
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub async fn delete_model_redirect(&self, provider: &str, source_model: &str) -> Result<bool> {
        let conn = self.connection.lock().await;
        let affected = conn.execute(
            "DELETE FROM model_redirects WHERE provider = ?1 AND source_model = ?2",
            (provider, source_model),
        )?;
        Ok(affected > 0)
    }
}
