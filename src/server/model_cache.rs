use crate::providers::openai::Model;
use crate::server::AppState;

// 读取所有供应商的缓存模型，并在 id 前加上 "{provider}/{id}"
pub async fn get_cached_models_all(app_state: &AppState) -> rusqlite::Result<Vec<Model>> {
    let cached = app_state.model_cache.get_cached_models(None).await?;
    let models = cached
        .into_iter()
        .map(|m| Model {
            id: format!("{}/{}", m.provider, m.id),
            object: m.object,
            created: m.created,
            owned_by: m.owned_by,
        })
        .collect();
    Ok(models)
}

// 读取指定供应商的缓存模型（不加前缀）
pub async fn get_cached_models_for_provider(
    app_state: &AppState,
    provider_name: &str,
) -> rusqlite::Result<Vec<Model>> {
    let cached = app_state
        .model_cache
        .get_cached_models(Some(provider_name))
        .await?;
    let models = cached
        .into_iter()
        .map(|m| Model {
            id: m.id,
            object: m.object,
            created: m.created,
            owned_by: m.owned_by,
        })
        .collect();
    Ok(models)
}

// 检查所有供应商的缓存是否都在有效期内（任一失败或错误均视为不新鲜）
pub async fn is_cache_fresh_for_all(app_state: &AppState, max_age_minutes: i64) -> bool {
    for provider_name in app_state.config.providers.keys() {
        match app_state
            .model_cache
            .is_cache_fresh(provider_name, max_age_minutes)
            .await
        {
            Ok(true) => {}
            _ => return false,
        }
    }
    true
}

// 检查某个供应商的缓存是否在有效期内（错误记为不新鲜）
pub async fn is_cache_fresh_for_provider(
    app_state: &AppState,
    provider_name: &str,
    max_age_minutes: i64,
) -> bool {
    app_state
        .model_cache
        .is_cache_fresh(provider_name, max_age_minutes)
        .await
        .unwrap_or(false)
}

// 写入（覆盖）某供应商的模型缓存
pub async fn cache_models_for_provider(
    app_state: &AppState,
    provider_name: &str,
    models: &[Model],
) -> rusqlite::Result<()> {
    app_state
        .model_cache
        .cache_models(provider_name, models)
        .await
}
