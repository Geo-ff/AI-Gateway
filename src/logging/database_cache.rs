use chrono::Utc;
use rusqlite::Result;

use crate::logging::time::{parse_beijing_string, to_beijing_string};
use crate::logging::types::CachedModel;
use crate::providers::openai::Model;

use super::database::DatabaseLogger;

impl DatabaseLogger {
    pub async fn cache_models(&self, provider: &str, models: &[Model]) -> Result<()> {
        let conn = self.connection.lock().await;
        let now = Utc::now();

        conn.execute("DELETE FROM cached_models WHERE provider = ?1", [provider])?;

        for model in models {
            conn.execute(
                "INSERT INTO cached_models (id, provider, object, created, owned_by, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    &model.id,
                    provider,
                    &model.object,
                    model.created,
                    &model.owned_by,
                    to_beijing_string(&now),
                ),
            )?;
        }

        tracing::info!("Cached {} models for provider: {}", models.len(), provider);
        Ok(())
    }

    pub async fn get_cached_models(&self, provider: Option<&str>) -> Result<Vec<CachedModel>> {
        let conn = self.connection.lock().await;

        if let Some(provider) = provider {
            let mut stmt = conn.prepare(
                "SELECT id, provider, object, created, owned_by, cached_at
                 FROM cached_models WHERE provider = ?1
                 ORDER BY id",
            )?;

            let model_iter = stmt.query_map([provider], |row| {
                Ok(CachedModel {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    object: row.get(2)?,
                    created: row.get(3)?,
                    owned_by: row.get(4)?,
                    cached_at: parse_beijing_string(&row.get::<_, String>(5)?).unwrap(),
                })
            })?;

            let mut models = Vec::new();
            for model in model_iter {
                models.push(model?);
            }
            Ok(models)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, provider, object, created, owned_by, cached_at
                 FROM cached_models
                 ORDER BY provider, id",
            )?;

            let model_iter = stmt.query_map([], |row| {
                Ok(CachedModel {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    object: row.get(2)?,
                    created: row.get(3)?,
                    owned_by: row.get(4)?,
                    cached_at: parse_beijing_string(&row.get::<_, String>(5)?).unwrap(),
                })
            })?;

            let mut models = Vec::new();
            for model in model_iter {
                models.push(model?);
            }
            Ok(models)
        }
    }

    // 追加或更新模型（不清空该供应商原有缓存）
    pub async fn cache_models_append(&self, provider: &str, models: &[Model]) -> Result<()> {
        let conn = self.connection.lock().await;
        let now = chrono::Utc::now();
        for model in models {
            conn.execute(
                "INSERT OR REPLACE INTO cached_models (id, provider, object, created, owned_by, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    &model.id,
                    provider,
                    &model.object,
                    model.created,
                    &model.owned_by,
                    crate::logging::time::to_beijing_string(&now),
                ),
            )?;
        }
        Ok(())
    }

    pub async fn remove_cached_models(&self, provider: &str, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.connection.lock().await;
        let tx = conn.unchecked_transaction()?;
        for id in ids {
            tx.execute(
                "DELETE FROM cached_models WHERE provider = ?1 AND id = ?2",
                (provider, id),
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
