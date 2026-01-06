use async_trait::async_trait;
use chrono::Utc;

use crate::admin::{
    AdminToken, CreateTokenPayload, TokenStore, UpdateTokenPayload, admin_token_id_for_token,
    normalize_admin_token_name,
};
use crate::error::GatewayError;
use crate::logging::database::DatabaseLogger;
use crate::logging::time::{parse_beijing_string, to_beijing_string};

fn join_allowed_models(v: &Option<Vec<String>>) -> Option<String> {
    v.as_ref().map(|list| list.join(","))
}

fn parse_allowed_models(s: Option<String>) -> Option<Vec<String>> {
    s.map(|v| {
        v.split(',')
            .filter(|x| !x.trim().is_empty())
            .map(|x| x.trim().to_string())
            .collect::<Vec<_>>()
    })
    .and_then(|v| if v.is_empty() { None } else { Some(v) })
}

#[async_trait]
impl TokenStore for DatabaseLogger {
    async fn create_token(&self, payload: CreateTokenPayload) -> Result<AdminToken, GatewayError> {
        // 始终生成随机令牌，忽略传入 token 字段
        let token = {
            use rand::Rng;
            let rng = rand::rng();
            use rand::distr::Alphanumeric;
            let s: String = rng
                .sample_iter(&Alphanumeric)
                .take(40)
                .map(char::from)
                .collect();
            s
        };
        let id = admin_token_id_for_token(&token);
        let name = normalize_admin_token_name(payload.name.clone(), &id);
        let now = Utc::now();
        let allowed_models_s = join_allowed_models(&payload.allowed_models);
        let expires_at_s = payload.expires_at.clone();
        let conn = self.connection.lock().await;
        conn.execute(
            "INSERT INTO admin_tokens (id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, 0, 0)",
            (
                &id,
                &name,
                &token,
                &allowed_models_s,
                payload.max_tokens,
                if payload.enabled { 1 } else { 0 },
                &expires_at_s,
                to_beijing_string(&now),
                payload.max_amount,
            ),
        )?;

        Ok(AdminToken {
            id,
            name,
            token,
            allowed_models: payload.allowed_models,
            max_tokens: payload.max_tokens,
            max_amount: payload.max_amount,
            enabled: payload.enabled,
            expires_at: match expires_at_s {
                Some(s) => Some(parse_beijing_string(&s)?),
                None => None,
            },
            created_at: now,
            amount_spent: 0.0,
            prompt_tokens_spent: 0,
            completion_tokens_spent: 0,
            total_tokens_spent: 0,
        })
    }

    async fn update_token(
        &self,
        token: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<AdminToken>, GatewayError> {
        let conn = self.connection.lock().await;
        use rusqlite::OptionalExtension;
        let mut stmt = conn.prepare("SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens WHERE token = ?1")?;
        let row_opt = stmt
            .query_row([token], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<f64>>(8)?,
                    row.get::<_, Option<f64>>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                ))
            })
            .optional()?;
        let Some((
            id0,
            name0,
            tok,
            allowed,
            max,
            enabled_i,
            expires,
            created_at_s,
            max_amount0,
            amount_spent0,
            prompt0,
            completion0,
            total0,
        )) = row_opt
        else {
            return Ok(None);
        };

        let needs_id_backfill = id0.as_deref().filter(|s| !s.is_empty()).is_none();
        let id = id0
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| admin_token_id_for_token(&tok));
        let mut name = normalize_admin_token_name(name0.clone(), &id);
        if needs_id_backfill {
            let _ = conn.execute(
                "UPDATE admin_tokens SET id = ?2 WHERE token = ?1 AND (id IS NULL OR id = '')",
                (&tok, &id),
            );
        }
        if name0.as_deref().filter(|s| !s.trim().is_empty()).is_none() {
            let _ = conn.execute(
                "UPDATE admin_tokens SET name = ?2 WHERE token = ?1 AND (name IS NULL OR name = '')",
                (&tok, &name),
            );
        }

        let mut allowed_models = parse_allowed_models(allowed);
        let mut max_tokens = max;
        let mut enabled = enabled_i != 0;
        let mut expires_at = expires;
        let mut max_amount = max_amount0;
        let amount_spent = amount_spent0.unwrap_or(0.0);
        let prompt_tokens_spent = prompt0.unwrap_or(0);
        let completion_tokens_spent = completion0.unwrap_or(0);
        let total_tokens_spent = total0.unwrap_or(0);

        if let Some(v) = payload.name {
            name = normalize_admin_token_name(Some(v), &id);
        }
        if let Some(v) = payload.allowed_models {
            allowed_models = Some(v);
        }
        if let Some(v) = payload.max_tokens {
            max_tokens = v;
        }
        if let Some(v) = payload.max_amount {
            max_amount = v;
        }
        if let Some(v) = payload.enabled {
            enabled = v;
        }
        if let Some(v) = payload.expires_at {
            expires_at = v;
        }

        conn.execute(
            "UPDATE admin_tokens SET name = ?2, allowed_models = ?3, max_tokens = ?4, enabled = ?5, expires_at = ?6, max_amount = ?7 WHERE token = ?1",
            (
                &tok,
                &name,
                join_allowed_models(&allowed_models),
                max_tokens,
                if enabled { 1 } else { 0 },
                expires_at.clone(),
                max_amount,
            ),
        )?;

        Ok(Some(AdminToken {
            id,
            name,
            token: tok,
            allowed_models,
            max_tokens,
            max_amount,
            enabled,
            expires_at: match expires_at {
                Some(s) => Some(parse_beijing_string(&s)?),
                None => None,
            },
            created_at: parse_beijing_string(&created_at_s)?,
            amount_spent,
            prompt_tokens_spent,
            completion_tokens_spent,
            total_tokens_spent,
        }))
    }

    async fn set_enabled(&self, token: &str, enabled: bool) -> Result<bool, GatewayError> {
        let conn = self.connection.lock().await;
        let affected = conn.execute(
            "UPDATE admin_tokens SET enabled = ?2 WHERE token = ?1",
            (token, if enabled { 1 } else { 0 }),
        )?;
        Ok(affected > 0)
    }

    async fn get_token(&self, token: &str) -> Result<Option<AdminToken>, GatewayError> {
        let conn = self.connection.lock().await;
        use rusqlite::OptionalExtension;
        let mut stmt = conn.prepare("SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens WHERE token = ?1")?;
        let row = stmt
            .query_row([token], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<f64>>(8)?,
                    row.get::<_, Option<f64>>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                ))
            })
            .optional()?;
        if let Some((
            id0,
            name0,
            token,
            allowed,
            max_tokens,
            enabled_i,
            expires,
            created_at_s,
            max_amount,
            amount_spent,
            prompt_tokens_spent,
            completion_tokens_spent,
            total_tokens_spent,
        )) = row
        {
        let needs_id_backfill = id0.as_deref().filter(|s| !s.is_empty()).is_none();
        let needs_name_backfill = name0.as_deref().filter(|s| !s.trim().is_empty()).is_none();
        let id = id0
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| admin_token_id_for_token(&token));
        let name = normalize_admin_token_name(name0.clone(), &id);
        if needs_id_backfill {
            let _ = conn.execute(
                "UPDATE admin_tokens SET id = ?2 WHERE token = ?1 AND (id IS NULL OR id = '')",
                (&token, &id),
            );
        }
        if needs_name_backfill {
            let _ = conn.execute(
                "UPDATE admin_tokens SET name = ?2 WHERE token = ?1 AND (name IS NULL OR name = '')",
                (&token, &name),
            );
        }
            Ok(Some(AdminToken {
                id,
                name,
                token,
                allowed_models: parse_allowed_models(allowed),
                max_tokens,
                max_amount,
                enabled: enabled_i != 0,
                expires_at: match expires {
                    Some(s) => Some(parse_beijing_string(&s)?),
                    None => None,
                },
                created_at: parse_beijing_string(&created_at_s)?,
                amount_spent: amount_spent.unwrap_or(0.0),
                prompt_tokens_spent: prompt_tokens_spent.unwrap_or(0),
                completion_tokens_spent: completion_tokens_spent.unwrap_or(0),
                total_tokens_spent: total_tokens_spent.unwrap_or(0),
            }))
        } else {
            Ok(None)
        }
    }

    async fn get_token_by_id(&self, id: &str) -> Result<Option<AdminToken>, GatewayError> {
        let conn = self.connection.lock().await;
        use rusqlite::OptionalExtension;
        let mut stmt = conn.prepare("SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens WHERE id = ?1")?;
        let row = stmt
            .query_row([id], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<f64>>(8)?,
                    row.get::<_, Option<f64>>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                    row.get::<_, Option<i64>>(12)?,
                ))
            })
            .optional()?;
        let Some((
            id0,
            name0,
            token,
            allowed,
            max_tokens,
            enabled_i,
            expires,
            created_at_s,
            max_amount,
            amount_spent,
            prompt_tokens_spent,
            completion_tokens_spent,
            total_tokens_spent,
        )) = row
        else {
            return Ok(None);
        };
        let needs_id_backfill = id0.as_deref().filter(|s| !s.is_empty()).is_none();
        let needs_name_backfill = name0.as_deref().filter(|s| !s.trim().is_empty()).is_none();
        let id = id0
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| admin_token_id_for_token(&token));
        let name = normalize_admin_token_name(name0.clone(), &id);
        if needs_id_backfill {
            let _ = conn.execute(
                "UPDATE admin_tokens SET id = ?2 WHERE token = ?1 AND (id IS NULL OR id = '')",
                (&token, &id),
            );
        }
        if needs_name_backfill {
            let _ = conn.execute(
                "UPDATE admin_tokens SET name = ?2 WHERE token = ?1 AND (name IS NULL OR name = '')",
                (&token, &name),
            );
        }
        Ok(Some(AdminToken {
            id,
            name,
            token,
            allowed_models: parse_allowed_models(allowed),
            max_tokens,
            max_amount,
            enabled: enabled_i != 0,
            expires_at: match expires {
                Some(s) => Some(parse_beijing_string(&s)?),
                None => None,
            },
            created_at: parse_beijing_string(&created_at_s)?,
            amount_spent: amount_spent.unwrap_or(0.0),
            prompt_tokens_spent: prompt_tokens_spent.unwrap_or(0),
            completion_tokens_spent: completion_tokens_spent.unwrap_or(0),
            total_tokens_spent: total_tokens_spent.unwrap_or(0),
        }))
    }

    async fn list_tokens(&self) -> Result<Vec<AdminToken>, GatewayError> {
        let conn = self.connection.lock().await;
        let mut stmt = conn.prepare("SELECT id, name, token, allowed_models, max_tokens, enabled, expires_at, created_at, max_amount, amount_spent, prompt_tokens_spent, completion_tokens_spent, total_tokens_spent FROM admin_tokens ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<f64>>(8)?,
                row.get::<_, Option<f64>>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, Option<i64>>(11)?,
                row.get::<_, Option<i64>>(12)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (
                id0,
                name0,
                token,
                allowed,
                max_tokens,
                enabled_i,
                expires,
                created_at_s,
                max_amount,
                amount_spent,
                prompt_tokens_spent,
                completion_tokens_spent,
                total_tokens_spent,
            ) = r?;
            let needs_id_backfill = id0.as_deref().filter(|s| !s.is_empty()).is_none();
            let needs_name_backfill = name0.as_deref().filter(|s| !s.trim().is_empty()).is_none();
            let id = id0
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| admin_token_id_for_token(&token));
            let name = normalize_admin_token_name(name0.clone(), &id);
            if needs_id_backfill {
                let _ = conn.execute(
                    "UPDATE admin_tokens SET id = ?2 WHERE token = ?1 AND (id IS NULL OR id = '')",
                    (&token, &id),
                );
            }
            if needs_name_backfill {
                let _ = conn.execute(
                    "UPDATE admin_tokens SET name = ?2 WHERE token = ?1 AND (name IS NULL OR name = '')",
                    (&token, &name),
                );
            }
            out.push(AdminToken {
                id,
                name,
                token,
                allowed_models: parse_allowed_models(allowed),
                max_tokens,
                max_amount,
                enabled: enabled_i != 0,
                expires_at: match expires {
                    Some(s) => parse_beijing_string(&s).ok(),
                    None => None,
                },
                created_at: parse_beijing_string(&created_at_s).unwrap_or(Utc::now()),
                amount_spent: amount_spent.unwrap_or(0.0),
                prompt_tokens_spent: prompt_tokens_spent.unwrap_or(0),
                completion_tokens_spent: completion_tokens_spent.unwrap_or(0),
                total_tokens_spent: total_tokens_spent.unwrap_or(0),
            });
        }
        Ok(out)
    }

    async fn add_amount_spent(&self, token: &str, delta: f64) -> Result<(), GatewayError> {
        let conn = self.connection.lock().await;
        conn.execute(
            "UPDATE admin_tokens SET amount_spent = COALESCE(amount_spent, 0) + ?2 WHERE token = ?1",
            (token, delta),
        )?;
        Ok(())
    }

    async fn add_usage_spent(
        &self,
        token: &str,
        prompt: i64,
        completion: i64,
        total: i64,
    ) -> Result<(), GatewayError> {
        let conn = self.connection.lock().await;
        conn.execute(
            "UPDATE admin_tokens SET prompt_tokens_spent = COALESCE(prompt_tokens_spent,0) + ?2, completion_tokens_spent = COALESCE(completion_tokens_spent,0) + ?3, total_tokens_spent = COALESCE(total_tokens_spent,0) + ?4 WHERE token = ?1",
            (token, prompt, completion, total),
        )?;
        Ok(())
    }

    async fn delete_token(&self, token: &str) -> Result<bool, GatewayError> {
        let conn = self.connection.lock().await;
        let affected = conn.execute("DELETE FROM admin_tokens WHERE token = ?1", (token,))?;
        Ok(affected > 0)
    }

    async fn delete_token_by_id(&self, id: &str) -> Result<bool, GatewayError> {
        let conn = self.connection.lock().await;
        let affected = conn.execute("DELETE FROM admin_tokens WHERE id = ?1", (id,))?;
        Ok(affected > 0)
    }

    async fn update_token_by_id(
        &self,
        id: &str,
        payload: UpdateTokenPayload,
    ) -> Result<Option<AdminToken>, GatewayError> {
        let tok: Option<String> = {
            let conn = self.connection.lock().await;
            use rusqlite::OptionalExtension;
            let mut stmt = conn.prepare("SELECT token FROM admin_tokens WHERE id = ?1")?;
            stmt.query_row([id], |row| row.get(0)).optional()?
        };
        let Some(tok) = tok else {
            return Ok(None);
        };
        self.update_token(&tok, payload).await
    }

    async fn set_enabled_by_id(&self, id: &str, enabled: bool) -> Result<bool, GatewayError> {
        let conn = self.connection.lock().await;
        let affected = conn.execute(
            "UPDATE admin_tokens SET enabled = ?2 WHERE id = ?1",
            (id, if enabled { 1 } else { 0 }),
        )?;
        Ok(affected > 0)
    }
}
