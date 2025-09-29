use rusqlite::Result;

use super::database::DatabaseLogger;

impl DatabaseLogger {
    pub async fn upsert_model_price(
        &self,
        provider: &str,
        model: &str,
        prompt_price_per_million: f64,
        completion_price_per_million: f64,
        currency: Option<&str>,
    ) -> Result<()> {
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO model_prices (provider, model, prompt_price_per_million, completion_price_per_million, currency)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(provider, model) DO UPDATE SET
                prompt_price_per_million = excluded.prompt_price_per_million,
                completion_price_per_million = excluded.completion_price_per_million,
                currency = excluded.currency",
            (
                provider,
                model,
                prompt_price_per_million,
                completion_price_per_million,
                currency,
            ),
        )?;
        Ok(())
    }

    pub async fn get_model_price(
        &self,
        provider: &str,
        model: &str,
    ) -> Result<Option<(f64, f64, Option<String>)>> {
        let conn = self.connection.lock().await;
        use rusqlite::OptionalExtension;
        let mut stmt = conn.prepare(
            "SELECT prompt_price_per_million, completion_price_per_million, currency FROM model_prices WHERE provider = ?1 AND model = ?2",
        )?;
        let row = stmt
            .query_row((provider, model), |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .optional()?;
        Ok(row)
    }

    pub async fn list_model_prices(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<(String, String, f64, f64, Option<String>)>> {
        let conn = self.connection.lock().await;
        if let Some(p) = provider {
            let mut stmt = conn.prepare("SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency FROM model_prices WHERE provider = ?1 ORDER BY model")?;
            let rows = stmt.query_map([p], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        } else {
            let mut stmt = conn.prepare("SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency FROM model_prices ORDER BY provider, model")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        }
    }
}
