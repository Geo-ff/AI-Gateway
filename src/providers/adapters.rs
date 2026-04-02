use async_trait::async_trait;
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde_json::json;
use std::time::Duration;

use crate::config::settings::{ProviderAuthMode, ProviderProtocolFamily, ProviderType};
use crate::error::GatewayError;
use crate::providers::openai::ModelListResponse;

pub struct ListModelsRequest<'a> {
    pub models_url: &'a Url,
    pub api_key: &'a str,
}

pub struct ConnectionTestRequest<'a> {
    pub base_url: &'a Url,
    pub api_key: &'a str,
    pub model: &'a str,
    pub stream: bool,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn build_auth_headers(
        &self,
        api_key: &str,
    ) -> Result<reqwest::header::HeaderMap, (String, Option<String>)>;

    fn normalize_error(
        &self,
        status: StatusCode,
        content_type: Option<&str>,
        bytes: &[u8],
    ) -> (String, Option<String>);

    async fn list_models(
        &self,
        request: ListModelsRequest<'_>,
    ) -> Result<Vec<String>, GatewayError>;

    async fn test_connection(
        &self,
        request: ConnectionTestRequest<'_>,
    ) -> Result<(), (String, Option<String>)>;

    fn supports_stream_retry(&self) -> bool {
        false
    }
}

#[derive(Debug)]
struct ProtocolAdapter {
    family: ProviderProtocolFamily,
    auth_mode: ProviderAuthMode,
    supports_stream_retry: bool,
}

static OPENAI_COMPAT_ADAPTER: ProtocolAdapter = ProtocolAdapter {
    family: ProviderProtocolFamily::OpenAI,
    auth_mode: ProviderAuthMode::Bearer,
    supports_stream_retry: true,
};

static ANTHROPIC_ADAPTER: ProtocolAdapter = ProtocolAdapter {
    family: ProviderProtocolFamily::Anthropic,
    auth_mode: ProviderAuthMode::XApiKey,
    supports_stream_retry: false,
};

static ZHIPU_ADAPTER: ProtocolAdapter = ProtocolAdapter {
    family: ProviderProtocolFamily::Zhipu,
    auth_mode: ProviderAuthMode::Bearer,
    supports_stream_retry: false,
};

pub fn adapter_for(provider_type: ProviderType) -> Option<&'static dyn ProviderAdapter> {
    match provider_type.protocol_family() {
        ProviderProtocolFamily::OpenAI => Some(&OPENAI_COMPAT_ADAPTER),
        ProviderProtocolFamily::Anthropic if matches!(provider_type, ProviderType::Anthropic) => {
            Some(&ANTHROPIC_ADAPTER)
        }
        ProviderProtocolFamily::Zhipu => Some(&ZHIPU_ADAPTER),
        ProviderProtocolFamily::Unsupported | ProviderProtocolFamily::Anthropic => None,
    }
}

pub fn unsupported_provider_message(provider_type: ProviderType) -> String {
    format!(
        "provider type '{}' 仅完成类型注册骨架，当前版本暂未实现对应适配逻辑",
        provider_type.as_str()
    )
}

fn openai_compat_chat_completions_url(base_url: &Url) -> String {
    let base = base_url.as_str().trim_end_matches('/');
    let path = base_url.path().trim_end_matches('/');
    if path.ends_with("/v1") || path.ends_with("/api/v3") {
        format!("{}/chat/completions", base)
    } else {
        format!("{}/v1/chat/completions", base)
    }
}

fn classify_http_failure(status: StatusCode, body_snippet: &str) -> (String, Option<String>) {
    let snippet = body_snippet.trim();
    let lower = snippet.to_lowercase();

    if status == StatusCode::NOT_FOUND {
        if lower.contains("model") && (lower.contains("not found") || lower.contains("not_found")) {
            return ("model_not_found".into(), Some(snippet.to_string()));
        }
        return ("invalid_path".into(), Some(snippet.to_string()));
    }

    if status == StatusCode::REQUEST_TIMEOUT {
        return ("timeout".into(), Some(snippet.to_string()));
    }

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return ("authentication_failed".into(), Some(snippet.to_string()));
    }

    if status == StatusCode::PAYMENT_REQUIRED
        || lower.contains("insufficient")
        || lower.contains("balance")
    {
        return ("insufficient_balance".into(), Some(snippet.to_string()));
    }

    (
        "other".into(),
        Some(snippet.to_string()).filter(|s| !s.is_empty()),
    )
}

fn format_upstream_error_detail(
    status: StatusCode,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Option<String> {
    let ct = content_type.unwrap_or("").trim();

    if ct.contains("application/json") {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
            let out = json!({
                "status": status.as_u16(),
                "content_type": if ct.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(ct.to_string()) },
                "body": v,
            });
            return serde_json::to_string_pretty(&out).ok();
        }
    }

    let snippet = String::from_utf8_lossy(bytes);
    let snippet = snippet.trim();
    if snippet.is_empty() {
        let out = json!({
            "status": status.as_u16(),
            "content_type": if ct.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(ct.to_string()) },
        });
        return serde_json::to_string_pretty(&out).ok();
    }

    let out = json!({
        "status": status.as_u16(),
        "content_type": if ct.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(ct.to_string()) },
        "body_text": snippet,
    });
    serde_json::to_string_pretty(&out).ok()
}

fn classify_reqwest_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "timeout".into();
    }
    if let Some(status) = err.status()
        && status == StatusCode::NOT_FOUND
    {
        return "invalid_path".into();
    }
    "other".into()
}

fn build_models_list_error(status: StatusCode, bytes: &[u8]) -> GatewayError {
    let snippet = String::from_utf8_lossy(bytes);
    let snippet = snippet.trim();
    let snippet = if snippet.len() > 240 {
        &snippet[..240]
    } else {
        snippet
    };
    let msg = match status.as_u16() {
        401 | 403 => "上游鉴权失败，请检查 Key（或先添加/启用 Key）".to_string(),
        404 => "上游未找到模型列表接口（404），该上游可能不支持自动获取模型列表；可配置 models_endpoint 或手动输入模型".to_string(),
        _ => {
            if snippet.is_empty() {
                format!("上游返回错误（{}）", status.as_u16())
            } else {
                format!("上游返回错误（{}）：{}", status.as_u16(), snippet)
            }
        }
    };
    if matches!(status.as_u16(), 401 | 403) {
        GatewayError::Unauthorized(msg)
    } else {
        GatewayError::Config(msg)
    }
}

impl ProtocolAdapter {
    fn client_for_url(
        &self,
        url: &str,
        timeout_secs: u64,
    ) -> Result<reqwest::Client, GatewayError> {
        let builder = reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(timeout_secs));
        Ok(crate::http_client::maybe_disable_proxy(builder, url).build()?)
    }

    fn connection_test_url(&self, base_url: &Url) -> String {
        match self.family {
            ProviderProtocolFamily::OpenAI => openai_compat_chat_completions_url(base_url),
            ProviderProtocolFamily::Anthropic => {
                format!("{}/v1/messages", base_url.as_str().trim_end_matches('/'))
            }
            ProviderProtocolFamily::Zhipu => format!(
                "{}/api/paas/v4/chat/completions",
                base_url.as_str().trim_end_matches('/'),
            ),
            ProviderProtocolFamily::Unsupported => base_url.as_str().to_string(),
        }
    }

    fn connection_test_payload(&self, model: &str, stream: bool) -> serde_json::Value {
        match self.family {
            ProviderProtocolFamily::Anthropic => json!({
                "model": model,
                "stream": stream,
                "max_tokens": 1,
                "messages": [{"role":"user","content":[{"type":"text","text":"ping"}]}]
            }),
            ProviderProtocolFamily::OpenAI | ProviderProtocolFamily::Zhipu => json!({
                "model": model,
                "messages": [{"role":"user","content":"ping"}],
                "stream": stream,
                "max_tokens": 1,
                "temperature": 0
            }),
            ProviderProtocolFamily::Unsupported => json!({}),
        }
    }
}

#[async_trait]
impl ProviderAdapter for ProtocolAdapter {
    fn build_auth_headers(
        &self,
        api_key: &str,
    ) -> Result<reqwest::header::HeaderMap, (String, Option<String>)> {
        use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

        let mut headers = HeaderMap::new();
        match self.auth_mode {
            ProviderAuthMode::Bearer => {
                let value = HeaderValue::from_str(&format!("Bearer {api_key}"))
                    .map_err(|e| ("other".into(), Some(e.to_string())))?;
                headers.insert(AUTHORIZATION, value);
            }
            ProviderAuthMode::ApiKey => {
                let api_key_value = HeaderValue::from_str(api_key)
                    .map_err(|e| ("other".into(), Some(e.to_string())))?;
                headers.insert("api-key", api_key_value);
            }
            ProviderAuthMode::XApiKey => {
                let api_key_value = HeaderValue::from_str(api_key)
                    .map_err(|e| ("other".into(), Some(e.to_string())))?;
                headers.insert("x-api-key", api_key_value);
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            }
            ProviderAuthMode::Unsupported | ProviderAuthMode::SigV4 | ProviderAuthMode::OAuth => {}
        }
        Ok(headers)
    }

    fn normalize_error(
        &self,
        status: StatusCode,
        content_type: Option<&str>,
        bytes: &[u8],
    ) -> (String, Option<String>) {
        let detail = format_upstream_error_detail(status, content_type, bytes);
        let snippet = String::from_utf8_lossy(bytes);
        let (ty, _) = classify_http_failure(status, &snippet);
        (ty, detail)
    }

    async fn list_models(
        &self,
        request: ListModelsRequest<'_>,
    ) -> Result<Vec<String>, GatewayError> {
        let client = self.client_for_url(request.models_url.as_str(), 12)?;
        let mut req = client
            .get(request.models_url.as_str())
            .header("Accept", "application/json");

        if !request.api_key.trim().is_empty() {
            for (name, value) in
                self.build_auth_headers(request.api_key)
                    .map_err(|(_, detail)| {
                        GatewayError::Config(detail.unwrap_or_else(|| "invalid auth header".into()))
                    })?
            {
                if let Some(name) = name {
                    req = req.header(name, value);
                }
            }
        }

        let resp = req.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;

        if !status.is_success() {
            return Err(build_models_list_error(status, &bytes));
        }

        let parsed: ModelListResponse = serde_json::from_slice(&bytes).map_err(|_| {
            GatewayError::Config("解析上游模型列表失败（非 OpenAI 兼容响应）".into())
        })?;
        let mut models: Vec<String> = parsed.data.into_iter().map(|m| m.id).collect();
        models.sort();
        models.dedup();
        Ok(models)
    }

    async fn test_connection(
        &self,
        request: ConnectionTestRequest<'_>,
    ) -> Result<(), (String, Option<String>)> {
        let model = request.model.trim();
        if model.is_empty() {
            return Err((
                "model_not_found".into(),
                Some("model cannot be empty".into()),
            ));
        }

        let url = self.connection_test_url(request.base_url);
        let client = self
            .client_for_url(&url, 30)
            .map_err(|e| ("other".into(), Some(e.to_string())))?;
        let payload = self.connection_test_payload(model, request.stream);
        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&payload);

        for (name, value) in self.build_auth_headers(request.api_key)? {
            if let Some(name) = name {
                req = req.header(name, value);
            }
        }

        let resp = req
            .send()
            .await
            .map_err(|e| (classify_reqwest_error(&e), Some(e.to_string())))?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ("other".into(), Some(e.to_string())))?;

        if !status.is_success() {
            return Err(self.normalize_error(status, content_type.as_deref(), &bytes));
        }

        Ok(())
    }

    fn supports_stream_retry(&self) -> bool {
        self.supports_stream_retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_provider_types_resolve_to_adapters() {
        assert!(adapter_for(ProviderType::OpenAI).is_some());
        assert!(adapter_for(ProviderType::Doubao).is_some());
        assert!(adapter_for(ProviderType::Anthropic).is_some());
        assert!(adapter_for(ProviderType::Zhipu).is_some());
        assert!(adapter_for(ProviderType::AzureOpenAI).is_none());
        assert!(adapter_for(ProviderType::AwsClaude).is_none());
    }

    #[test]
    fn auth_headers_follow_protocol_family() {
        let openai_headers = adapter_for(ProviderType::DeepSeek)
            .unwrap()
            .build_auth_headers("sk-test")
            .unwrap();
        assert!(openai_headers.contains_key(reqwest::header::AUTHORIZATION));

        let anthropic_headers = adapter_for(ProviderType::Anthropic)
            .unwrap()
            .build_auth_headers("sk-test")
            .unwrap();
        assert!(anthropic_headers.contains_key("x-api-key"));
        assert!(anthropic_headers.contains_key("anthropic-version"));
    }
}
