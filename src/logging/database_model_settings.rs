use rusqlite::Result;

use super::database::DatabaseLogger;

impl DatabaseLogger {
    pub async fn upsert_model_enabled(
        &self,
        provider: &str,
        model: &str,
        enabled: bool,
    ) -> Result<()> {
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO model_settings (provider, model, enabled)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(provider, model) DO UPDATE SET
                enabled = excluded.enabled",
            (provider, model, if enabled { 1 } else { 0 }),
        )?;
        Ok(())
    }

    pub async fn get_model_enabled(&self, provider: &str, model: &str) -> Result<Option<bool>> {
        let conn = self.connection.lock().await;
        use rusqlite::OptionalExtension;
        let mut stmt =
            conn.prepare("SELECT enabled FROM model_settings WHERE provider = ?1 AND model = ?2")?;
        let row = stmt
            .query_row((provider, model), |row| row.get::<_, i64>(0))
            .optional()?;
        Ok(row.map(|v| v != 0))
    }

    pub async fn list_model_enabled(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<(String, String, bool)>> {
        let conn = self.connection.lock().await;
        if let Some(p) = provider {
            let mut stmt = conn.prepare(
                "SELECT provider, model, enabled FROM model_settings WHERE provider = ?1 ORDER BY model",
            )?;
            let rows = stmt.query_map([p], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        } else {
            let mut stmt = conn.prepare(
                "SELECT provider, model, enabled FROM model_settings ORDER BY provider, model",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        }
    }
}
