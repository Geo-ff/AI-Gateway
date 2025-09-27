use rusqlite::{Result, OptionalExtension};

use crate::config::settings::{Provider, ProviderType, KeyLogStrategy};

use super::database::DatabaseLogger;

impl DatabaseLogger {
    pub async fn insert_provider(&self, provider: &Provider) -> Result<bool> {
        let conn = self.connection.lock().await;
        let res = conn.execute(
            "INSERT OR IGNORE INTO providers (name, api_type, base_url, models_endpoint)
             VALUES (?1, ?2, ?3, ?4)",
            (
                &provider.name,
                provider_type_to_str(&provider.api_type),
                &provider.base_url,
                &provider.models_endpoint,
            ),
        )?;
        Ok(res > 0)
    }

    pub async fn upsert_provider(&self, provider: &Provider) -> Result<()> {
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO providers (name, api_type, base_url, models_endpoint)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET api_type = excluded.api_type,
                                         base_url = excluded.base_url,
                                         models_endpoint = excluded.models_endpoint",
            (
                &provider.name,
                provider_type_to_str(&provider.api_type),
                &provider.base_url,
                &provider.models_endpoint,
            ),
        )?;
        Ok(())
    }

    pub async fn provider_exists(&self, name: &str) -> Result<bool> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare("SELECT 1 FROM providers WHERE name = ?1 LIMIT 1")?;
        let exists = stmt.exists([name])?;
        Ok(exists)
    }

    

    pub async fn get_provider(&self, name: &str) -> Result<Option<Provider>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT name, api_type, base_url, models_endpoint FROM providers WHERE name = ?1 LIMIT 1",
        )?;
        let provider = stmt.query_row([name], |row| {
            let name: String = row.get(0)?;
            let api_type: String = row.get(1)?;
            let base_url: String = row.get(2)?;
            let models_endpoint: Option<String> = row.get(3)?;
            Ok(Provider {
                name,
                api_type: provider_type_from_str(&api_type),
                base_url,
                api_keys: Vec::new(),
                models_endpoint,
            })
        }).optional()?;
        Ok(provider)
    }

    

    pub async fn list_providers(&self) -> Result<Vec<Provider>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare("SELECT name, api_type, base_url, models_endpoint FROM providers ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let api_type: String = row.get(1)?;
            let base_url: String = row.get(2)?;
            let models_endpoint: Option<String> = row.get(3)?;
            Ok(Provider {
                name,
                api_type: provider_type_from_str(&api_type),
                base_url,
                api_keys: Vec::new(),
                models_endpoint,
            })
        })?;
        let mut out = Vec::new();
        for r in rows { out.push(r?); }
        Ok(out)
    }

    pub async fn list_providers_with_keys(&self, strategy: &Option<KeyLogStrategy>) -> Result<Vec<Provider>> {
        let mut out = self.list_providers().await?;
        for p in &mut out {
            p.api_keys = self.get_provider_keys(&p.name, strategy).await?;
        }
        Ok(out)
    }

    pub async fn delete_provider(&self, name: &str) -> Result<bool> {
        let conn = self.connection.lock().await;
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM provider_keys WHERE provider = ?1", [name])?;
        tx.execute("DELETE FROM cached_models WHERE provider = ?1", [name])?;
        let affected = tx.execute("DELETE FROM providers WHERE name = ?1", [name])?;
        tx.commit()?;
        Ok(affected > 0)
    }
}

fn provider_type_from_str(s: &str) -> ProviderType {
    match s.to_ascii_lowercase().as_str() {
        "openai" => ProviderType::OpenAI,
        "anthropic" => ProviderType::Anthropic,
        "zhipu" => ProviderType::Zhipu,
        _ => ProviderType::OpenAI,
    }
}

fn provider_type_to_str(t: &ProviderType) -> &'static str {
    match t {
        ProviderType::OpenAI => "openai",
        ProviderType::Anthropic => "anthropic",
        ProviderType::Zhipu => "zhipu",
    }
}
