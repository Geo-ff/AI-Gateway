use rusqlite::Result;

use super::database::DatabaseLogger;
use crate::logging::time::{parse_datetime_string, to_iso8601_utc_string};
use crate::logging::{ModelPriceRecord, ModelPriceSource, ModelPriceStatus, ModelPriceUpsert};

fn parse_price_source(raw: &str) -> ModelPriceSource {
    match raw {
        "auto" => ModelPriceSource::Auto,
        _ => ModelPriceSource::Manual,
    }
}

fn parse_price_status(raw: &str) -> ModelPriceStatus {
    match raw {
        "missing" => ModelPriceStatus::Missing,
        "stale" => ModelPriceStatus::Stale,
        _ => ModelPriceStatus::Active,
    }
}

fn price_source_str(source: ModelPriceSource) -> &'static str {
    match source {
        ModelPriceSource::Manual => "manual",
        ModelPriceSource::Auto => "auto",
    }
}

fn price_status_str(status: ModelPriceStatus) -> &'static str {
    match status {
        ModelPriceStatus::Active => "active",
        ModelPriceStatus::Missing => "missing",
        ModelPriceStatus::Stale => "stale",
    }
}

impl DatabaseLogger {
    pub async fn upsert_model_price(&self, price: ModelPriceUpsert) -> Result<()> {
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO model_prices (
                provider,
                model,
                prompt_price_per_million,
                completion_price_per_million,
                currency,
                model_type,
                source,
                status,
                synced_at,
                expires_at
            )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(provider, model) DO UPDATE SET
                prompt_price_per_million = excluded.prompt_price_per_million,
                completion_price_per_million = excluded.completion_price_per_million,
                currency = excluded.currency,
                model_type = excluded.model_type,
                source = excluded.source,
                status = excluded.status,
                synced_at = excluded.synced_at,
                expires_at = excluded.expires_at",
            (
                &price.provider,
                &price.model,
                price.prompt_price_per_million,
                price.completion_price_per_million,
                price.currency.as_deref(),
                price.model_type.as_deref(),
                price_source_str(price.source),
                price_status_str(price.status),
                price.synced_at.as_ref().map(to_iso8601_utc_string),
                price.expires_at.as_ref().map(to_iso8601_utc_string),
            ),
        )?;
        Ok(())
    }

    pub async fn get_model_price(
        &self,
        provider: &str,
        model: &str,
    ) -> Result<Option<ModelPriceRecord>> {
        let conn = self.connection.lock().await;
        use rusqlite::OptionalExtension;
        let mut stmt = conn.prepare(
            "SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency, model_type, source, status, synced_at, expires_at
             FROM model_prices WHERE provider = ?1 AND model = ?2",
        )?;
        let row = stmt
            .query_row((provider, model), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            })
            .optional()?;
        Ok(row.map(
            |(
                provider,
                model,
                prompt_price_per_million,
                completion_price_per_million,
                currency,
                model_type,
                source,
                status,
                synced_at,
                expires_at,
            )| ModelPriceRecord {
                provider,
                model,
                prompt_price_per_million,
                completion_price_per_million,
                currency,
                model_type,
                source: parse_price_source(&source),
                status: parse_price_status(&status),
                synced_at: synced_at.and_then(|raw| parse_datetime_string(&raw).ok()),
                expires_at: expires_at.and_then(|raw| parse_datetime_string(&raw).ok()),
            },
        ))
    }

    pub async fn list_model_prices(&self, provider: Option<&str>) -> Result<Vec<ModelPriceRecord>> {
        let conn = self.connection.lock().await;
        if let Some(p) = provider {
            let mut stmt = conn.prepare(
                "SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency, model_type, source, status, synced_at, expires_at
                 FROM model_prices WHERE provider = ?1 ORDER BY model",
            )?;
            let rows = stmt.query_map([p], |row| {
                Ok(ModelPriceRecord {
                    provider: row.get(0)?,
                    model: row.get(1)?,
                    prompt_price_per_million: row.get(2)?,
                    completion_price_per_million: row.get(3)?,
                    currency: row.get(4)?,
                    model_type: row.get(5)?,
                    source: parse_price_source(&row.get::<_, String>(6)?),
                    status: parse_price_status(&row.get::<_, String>(7)?),
                    synced_at: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|raw| parse_datetime_string(&raw).ok()),
                    expires_at: row
                        .get::<_, Option<String>>(9)?
                        .and_then(|raw| parse_datetime_string(&raw).ok()),
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        } else {
            let mut stmt = conn.prepare(
                "SELECT provider, model, prompt_price_per_million, completion_price_per_million, currency, model_type, source, status, synced_at, expires_at
                 FROM model_prices ORDER BY provider, model",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(ModelPriceRecord {
                    provider: row.get(0)?,
                    model: row.get(1)?,
                    prompt_price_per_million: row.get(2)?,
                    completion_price_per_million: row.get(3)?,
                    currency: row.get(4)?,
                    model_type: row.get(5)?,
                    source: parse_price_source(&row.get::<_, String>(6)?),
                    status: parse_price_status(&row.get::<_, String>(7)?),
                    synced_at: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|raw| parse_datetime_string(&raw).ok()),
                    expires_at: row
                        .get::<_, Option<String>>(9)?
                        .and_then(|raw| parse_datetime_string(&raw).ok()),
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Timelike, Utc};
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[tokio::test]
    async fn sqlite_model_price_migration_backfills_metadata_defaults() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute(
                "CREATE TABLE model_prices (
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    prompt_price_per_million REAL NOT NULL,
                    completion_price_per_million REAL NOT NULL,
                    currency TEXT,
                    model_type TEXT,
                    PRIMARY KEY (provider, model)
                )",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO model_prices (provider, model, prompt_price_per_million, completion_price_per_million, currency, model_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                ("p1", "m1", 1.0_f64, 2.0_f64, Some("USD"), Some("chat")),
            )
            .unwrap();
        }

        let db = DatabaseLogger::new(db_path.to_str().unwrap())
            .await
            .unwrap();
        let record = db.get_model_price("p1", "m1").await.unwrap().unwrap();

        assert_eq!(record.source, ModelPriceSource::Manual);
        assert_eq!(record.status, ModelPriceStatus::Active);
        assert_eq!(record.synced_at, None);
        assert_eq!(record.expires_at, None);
    }

    #[tokio::test]
    async fn sqlite_model_price_roundtrip_preserves_metadata() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = DatabaseLogger::new(db_path.to_str().unwrap())
            .await
            .unwrap();
        let synced_at = Utc::now().with_nanosecond(0).unwrap();
        let expires_at = synced_at + Duration::hours(2);

        db.upsert_model_price(ModelPriceUpsert {
            provider: "p1".into(),
            model: "m1".into(),
            prompt_price_per_million: 1.5,
            completion_price_per_million: 2.5,
            currency: Some("USD".into()),
            model_type: Some("chat,image".into()),
            source: ModelPriceSource::Auto,
            status: ModelPriceStatus::Stale,
            synced_at: Some(synced_at),
            expires_at: Some(expires_at),
        })
        .await
        .unwrap();

        let record = db.get_model_price("p1", "m1").await.unwrap().unwrap();
        assert_eq!(record.source, ModelPriceSource::Auto);
        assert_eq!(record.status, ModelPriceStatus::Stale);
        assert_eq!(record.currency.as_deref(), Some("USD"));
        assert_eq!(record.model_type.as_deref(), Some("chat,image"));
        assert_eq!(record.synced_at, Some(synced_at));
        assert_eq!(record.expires_at, Some(expires_at));

        let listed = db.list_model_prices(None).await.unwrap();
        assert_eq!(listed, vec![record]);
    }
}
