use std::collections::HashSet;

use std::sync::Arc;

use crate::admin::ClientToken;
use crate::error::GatewayError;
use crate::server::AppState;

const MODEL_LIST_MAX_LEN: usize = 200;
const MODEL_ITEM_MAX_LEN: usize = 128;

pub fn normalize_model_list(
    field: &str,
    v: Option<Vec<String>>,
) -> Result<Option<Vec<String>>, GatewayError> {
    let Some(list) = v else { return Ok(None) };
    if list.is_empty() {
        return Ok(None);
    }
    if list.len() > MODEL_LIST_MAX_LEN {
        return Err(GatewayError::Config(format!(
            "{} 数量不能超过 {}",
            field, MODEL_LIST_MAX_LEN
        )));
    }
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::new();
    for raw in list {
        let s = raw.trim().to_string();
        if s.is_empty() {
            return Err(GatewayError::Config(format!("{} 不能包含空字符串", field)));
        }
        if s.chars().any(|c| c.is_control()) {
            return Err(GatewayError::Config(format!("{} 不能包含控制字符", field)));
        }
        if s.chars().count() > MODEL_ITEM_MAX_LEN {
            return Err(GatewayError::Config(format!(
                "{} 单条长度不能超过 {}",
                field, MODEL_ITEM_MAX_LEN
            )));
        }
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    if out.is_empty() {
        return Ok(None);
    }
    Ok(Some(out))
}

pub fn normalize_model_list_patch(
    field: &str,
    v: Option<Option<Vec<String>>>,
) -> Result<Option<Option<Vec<String>>>, GatewayError> {
    match v {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(list)) => Ok(Some(normalize_model_list(field, Some(list))?)),
    }
}

pub fn ensure_model_lists_mutually_exclusive(
    allowed_models: &Option<Vec<String>>,
    model_blacklist: &Option<Vec<String>>,
) -> Result<(), GatewayError> {
    if allowed_models.as_ref().is_some_and(|v| !v.is_empty())
        && model_blacklist.as_ref().is_some_and(|v| !v.is_empty())
    {
        return Err(GatewayError::Config(
            "allowed_models 与 model_blacklist 不可同时设置（白名单/黑名单互斥）".into(),
        ));
    }
    Ok(())
}

pub async fn validate_models_exist_in_cache(
    app_state: &Arc<AppState>,
    field: &str,
    list: &Option<Vec<String>>,
) -> Result<(), GatewayError> {
    let Some(list) = list.as_ref() else {
        return Ok(());
    };
    if list.is_empty() {
        return Ok(());
    }
    let cached = crate::server::model_cache::get_cached_models_all(app_state)
        .await
        .map_err(GatewayError::Db)?;
    let set: HashSet<String> = cached.into_iter().map(|m| m.id).collect();
    for m in list {
        if !set.contains(m) {
            return Err(GatewayError::NotFound(format!(
                "{} 中包含不存在的模型: {}",
                field, m
            )));
        }
    }
    Ok(())
}

pub fn enforce_model_allowed_for_token(
    token: &ClientToken,
    model: &str,
) -> Result<(), GatewayError> {
    if let Some(deny) = token.model_blacklist.as_ref()
        && deny.iter().any(|m| m == model)
    {
        return Err(GatewayError::Forbidden(format!(
            "model '{}' is blocked by token",
            model
        )));
    }
    if let Some(allow) = token.allowed_models.as_ref()
        && !allow.iter().any(|m| m == model)
    {
        return Err(GatewayError::Forbidden(format!(
            "model '{}' is not allowed for token",
            model
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn dummy_token() -> ClientToken {
        ClientToken {
            id: "atk_test".into(),
            user_id: None,
            name: "t".into(),
            token: "tok".into(),
            allowed_models: None,
            model_blacklist: None,
            max_tokens: None,
            max_amount: None,
            enabled: true,
            expires_at: None,
            created_at: Utc::now(),
            amount_spent: 0.0,
            prompt_tokens_spent: 0,
            completion_tokens_spent: 0,
            total_tokens_spent: 0,
            remark: None,
            organization_id: None,
            ip_whitelist: None,
            ip_blacklist: None,
        }
    }

    #[test]
    fn normalize_model_list_trims_dedups_and_rejects_empty() {
        let out = normalize_model_list(
            "allowed_models",
            Some(vec!["  a  ".into(), "a".into(), "b".into()]),
        )
        .unwrap();
        assert_eq!(out, Some(vec!["a".into(), "b".into()]));

        let err = normalize_model_list("allowed_models", Some(vec!["".into()])).unwrap_err();
        assert!(matches!(err, GatewayError::Config(_)));
    }

    #[test]
    fn enforce_whitelist_works() {
        let mut t = dummy_token();
        t.allowed_models = Some(vec!["a".into(), "b".into()]);
        enforce_model_allowed_for_token(&t, "a").unwrap();
        let err = enforce_model_allowed_for_token(&t, "c").unwrap_err();
        assert!(matches!(err, GatewayError::Forbidden(_)));
    }

    #[test]
    fn enforce_blacklist_works() {
        let mut t = dummy_token();
        t.model_blacklist = Some(vec!["a".into()]);
        enforce_model_allowed_for_token(&t, "b").unwrap();
        let err = enforce_model_allowed_for_token(&t, "a").unwrap_err();
        assert!(matches!(err, GatewayError::Forbidden(_)));
    }
}
