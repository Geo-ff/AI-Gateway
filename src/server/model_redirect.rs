use crate::config::{ModelRedirect, Settings};
use crate::providers::openai::ChatCompletionRequest;
use std::collections::HashMap;

// 应用可选的模型重定向（来自 redirect.toml）
pub fn apply_model_redirects(request: &mut ChatCompletionRequest) {
    let model_redirects = Settings::load_model_redirects().unwrap_or_else(|_| ModelRedirect {
        redirects: HashMap::new(),
    });

    if let Some(redirected_model) = model_redirects.redirects.get(&request.model) {
        request.model = redirected_model.clone();
    }
}
