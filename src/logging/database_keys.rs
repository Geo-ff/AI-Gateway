use chrono::Utc;
use rusqlite::Result;

use super::database::DatabaseLogger;
use crate::config::settings::KeyLogStrategy;

impl DatabaseLogger {
    pub async fn get_provider_keys(
        &self,
        provider: &str,
        strategy: &Option<KeyLogStrategy>,
    ) -> Result<Vec<String>> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare(
            "SELECT key_value, enc FROM provider_keys WHERE provider = ?1 AND active = 1 ORDER BY created_at"
        )?;
        let rows = stmt.query_map([provider], |row| {
            let value: String = row.get(0)?;
            let enc: i64 = row.get(1)?;
            let decrypted =
                crate::crypto::unprotect(strategy, provider, &value, enc != 0).unwrap_or_default();
            Ok(decrypted)
        })?;

        let mut out = Vec::new();
        for r in rows {
            let k = r?;
            if !k.is_empty() {
                out.push(k);
            }
        }
        Ok(out)
    }

    pub async fn add_provider_key(
        &self,
        provider: &str,
        key: &str,
        strategy: &Option<KeyLogStrategy>,
    ) -> Result<()> {
        let conn = self.connection.lock().await;
        let now = crate::logging::time::to_beijing_string(&Utc::now());
        let (stored, enc) = crate::crypto::protect(strategy, provider, key);
        conn.execute(
            "INSERT OR REPLACE INTO provider_keys (provider, key_value, enc, active, created_at)
             VALUES (?1, ?2, ?3, 1, ?4)",
            (provider, stored, if enc { 1 } else { 0 }, &now),
        )?;
        Ok(())
    }

    pub async fn remove_provider_key(
        &self,
        provider: &str,
        key: &str,
        strategy: &Option<KeyLogStrategy>,
    ) -> Result<bool> {
        let conn = self.connection.lock().await;
        // 删除明文或密文匹配
        // 优先删除密文匹配
        let (stored, enc) = crate::crypto::protect(strategy, provider, key);
        let mut affected = conn.execute(
            "DELETE FROM provider_keys WHERE provider = ?1 AND key_value = ?2",
            (provider, stored),
        )?;
        // 兼容已存明文的情况
        if enc {
            affected += conn.execute(
                "DELETE FROM provider_keys WHERE provider = ?1 AND key_value = ?2",
                (provider, key),
            )?;
        }
        Ok(affected > 0)
    }
}
