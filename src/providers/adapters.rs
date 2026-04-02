use async_trait::async_trait;
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

use crate::config::settings::{
    ProviderAuthMode, ProviderConfig, ProviderProtocolFamily, ProviderType,
};
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
    pub provider_config: &'a ProviderConfig,
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

#[derive(Debug)]
struct AzureOpenAIAdapter;

#[derive(Debug)]
struct GoogleGeminiAdapter;

#[derive(Debug)]
struct CohereAdapter;

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

static AZURE_OPENAI_ADAPTER: AzureOpenAIAdapter = AzureOpenAIAdapter;
static GOOGLE_GEMINI_ADAPTER: GoogleGeminiAdapter = GoogleGeminiAdapter;
static COHERE_ADAPTER: CohereAdapter = CohereAdapter;

pub fn adapter_for(provider_type: ProviderType) -> Option<&'static dyn ProviderAdapter> {
    match provider_type {
        ProviderType::OpenAI
        | ProviderType::Cloudflare
        | ProviderType::Perplexity
        | ProviderType::Mistral
        | ProviderType::DeepSeek
        | ProviderType::SiliconCloud
        | ProviderType::Moonshot
        | ProviderType::AlibabaQwen
        | ProviderType::Custom
        | ProviderType::XAI
        | ProviderType::Doubao => Some(&OPENAI_COMPAT_ADAPTER),
        ProviderType::Anthropic => Some(&ANTHROPIC_ADAPTER),
        ProviderType::Zhipu => Some(&ZHIPU_ADAPTER),
        ProviderType::AzureOpenAI => Some(&AZURE_OPENAI_ADAPTER),
        ProviderType::GoogleGemini => Some(&GOOGLE_GEMINI_ADAPTER),
        ProviderType::Cohere => Some(&COHERE_ADAPTER),
        ProviderType::AwsClaude | ProviderType::VertexAI => None,
    }
}

pub fn unsupported_provider_message(provider_type: ProviderType) -> String {
    format!(
        "provider type '{}' 仅完成类型注册骨架，当前版本暂未实现对应适配逻辑",
        provider_type.as_str()
    )
}

fn client_for_url(url: &str, timeout_secs: u64) -> Result<reqwest::Client, GatewayError> {
    let builder = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(timeout_secs));
    Ok(crate::http_client::maybe_disable_proxy(builder, url).build()?)
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

fn azure_openai_chat_completions_url(
    base_url: &Url,
    provider_config: &ProviderConfig,
) -> Result<String, (String, Option<String>)> {
    let deployment = provider_config.azure_deployment().ok_or_else(|| {
        (
            "configuration_required".into(),
            Some("Azure OpenAI 需要填写 deployment。".into()),
        )
    })?;
    let api_version = provider_config.azure_api_version().ok_or_else(|| {
        (
            "configuration_required".into(),
            Some("Azure OpenAI 需要填写 apiVersion。".into()),
        )
    })?;

    let base = base_url.as_str().trim_end_matches('/');
    let path = base_url.path().trim_end_matches('/');
    let prefix = if path.ends_with("/openai") {
        base.to_string()
    } else {
        format!("{}/openai", base)
    };
    Ok(format!(
        "{}/deployments/{}/chat/completions?api-version={}",
        prefix, deployment, api_version
    ))
}

fn append_query_api_key(url: &Url, api_key: &str) -> Result<String, (String, Option<String>)> {
    let mut out = url.clone();
    out.query_pairs_mut().append_pair("key", api_key);
    Ok(out.to_string())
}

fn gemini_base_url(base_url: &Url, provider_config: &ProviderConfig) -> String {
    let base = base_url.as_str().trim_end_matches('/');
    let path = base_url.path().trim_end_matches('/');
    let api_version = provider_config.google_api_version().unwrap_or("v1beta");
    if path.ends_with("/v1beta") || path.ends_with("/v1") || path.ends_with(api_version) {
        base.to_string()
    } else {
        format!("{}/{}", base, api_version)
    }
}

fn gemini_model_name(model: &str) -> Result<String, (String, Option<String>)> {
    let model = model.trim();
    if model.is_empty() {
        return Err((
            "model_not_found".into(),
            Some("Google Gemini 测试时需要填写模型名称。".into()),
        ));
    }

    if model.starts_with("models/") {
        Ok(model.to_string())
    } else {
        Ok(format!("models/{}", model))
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

fn extract_error_message(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;

    value
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .and_then(|message| message.as_str())
                .map(str::to_string)
                .or_else(|| error.as_str().map(str::to_string))
        })
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .get("detail")
                .and_then(|detail| detail.as_str())
                .map(str::to_string)
        })
}

fn gateway_error_from_normalized(error_type: &str, fallback_message: String) -> GatewayError {
    match error_type {
        "authentication_failed" => GatewayError::Unauthorized(fallback_message),
        _ => GatewayError::Config(fallback_message),
    }
}

fn azure_error_message(status: StatusCode, bytes: &[u8]) -> String {
    let upstream = extract_error_message(bytes)
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).trim().to_string());
    let lower = upstream.to_lowercase();

    if status == StatusCode::NOT_FOUND && lower.contains("deployment") {
        return "Azure OpenAI deployment 不存在，请检查 deployment 与资源地址。".into();
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return "Azure OpenAI 鉴权失败，请检查 API Key。".into();
    }
    if status == StatusCode::NOT_FOUND {
        return "Azure OpenAI 接口路径无效，请检查上游地址是否为资源根地址。".into();
    }
    if upstream.is_empty() {
        format!("Azure OpenAI 返回错误（{}）。", status.as_u16())
    } else {
        format!("Azure OpenAI 返回错误（{}）：{}", status.as_u16(), upstream)
    }
}

fn classify_azure_error(status: StatusCode, bytes: &[u8]) -> (String, Option<String>) {
    let message = azure_error_message(status, bytes);
    let lower = message.to_lowercase();

    if lower.contains("deployment") && lower.contains("不存在") {
        return ("configuration_required".into(), Some(message));
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return ("authentication_failed".into(), Some(message));
    }
    if status == StatusCode::NOT_FOUND {
        return ("invalid_path".into(), Some(message));
    }
    if status == StatusCode::BAD_REQUEST && lower.contains("api-version") {
        return ("configuration_required".into(), Some(message));
    }
    classify_http_failure(status, &message)
}

fn gemini_error_message(status: StatusCode, bytes: &[u8]) -> String {
    let upstream = extract_error_message(bytes)
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).trim().to_string());
    let lower = upstream.to_lowercase();

    if lower.contains("api key not valid") || lower.contains("permission denied") {
        return "Google Gemini 鉴权失败，请检查 API Key。".into();
    }
    if lower.contains("requested entity was not found")
        || lower.contains("model") && lower.contains("not found")
    {
        return "Google Gemini 模型不存在，请检查模型名称。".into();
    }
    if lower.contains("method not found") || lower.contains("url not found") {
        return "Google Gemini 接口路径无效，请检查上游地址或 API 版本。".into();
    }
    if upstream.is_empty() {
        format!("Google Gemini 返回错误（{}）。", status.as_u16())
    } else {
        format!(
            "Google Gemini 返回错误（{}）：{}",
            status.as_u16(),
            upstream
        )
    }
}

fn classify_gemini_error(status: StatusCode, bytes: &[u8]) -> (String, Option<String>) {
    let message = gemini_error_message(status, bytes);
    let lower = message.to_lowercase();

    if lower.contains("鉴权失败") {
        return ("authentication_failed".into(), Some(message));
    }
    if lower.contains("模型不存在") {
        return ("model_not_found".into(), Some(message));
    }
    if lower.contains("接口路径无效") {
        return ("invalid_path".into(), Some(message));
    }
    classify_http_failure(status, &message)
}

fn cohere_error_message(status: StatusCode, bytes: &[u8]) -> String {
    let upstream = extract_error_message(bytes)
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).trim().to_string());
    let lower = upstream.to_lowercase();

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return "Cohere 鉴权失败，请检查 API Key。".into();
    }
    if lower.contains("model") && lower.contains("not found") {
        return "Cohere 模型不存在，请检查模型名称。".into();
    }
    if status == StatusCode::NOT_FOUND {
        return "Cohere 接口路径无效，请检查上游地址。".into();
    }
    if upstream.is_empty() {
        format!("Cohere 返回错误（{}）。", status.as_u16())
    } else {
        format!("Cohere 返回错误（{}）：{}", status.as_u16(), upstream)
    }
}

fn classify_cohere_error(status: StatusCode, bytes: &[u8]) -> (String, Option<String>) {
    let message = cohere_error_message(status, bytes);
    let lower = message.to_lowercase();

    if lower.contains("鉴权失败") {
        return ("authentication_failed".into(), Some(message));
    }
    if lower.contains("模型不存在") {
        return ("model_not_found".into(), Some(message));
    }
    if lower.contains("接口路径无效") {
        return ("invalid_path".into(), Some(message));
    }
    classify_http_failure(status, &message)
}

#[derive(Debug, Deserialize)]
struct GeminiModelsResponse {
    #[serde(default)]
    models: Vec<GeminiModel>,
}

#[derive(Debug, Deserialize)]
struct GeminiModel {
    name: String,
    #[serde(default, rename = "supportedGenerationMethods")]
    supported_generation_methods: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CohereModelsResponse {
    #[serde(default)]
    models: Vec<CohereModel>,
}

#[derive(Debug, Deserialize)]
struct CohereModel {
    name: String,
    #[serde(default)]
    endpoints: Vec<String>,
}

impl ProtocolAdapter {
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
            ProviderProtocolFamily::AzureOpenAI
            | ProviderProtocolFamily::GoogleGemini
            | ProviderProtocolFamily::Cohere
            | ProviderProtocolFamily::Unsupported => base_url.as_str().to_string(),
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
            ProviderProtocolFamily::AzureOpenAI
            | ProviderProtocolFamily::GoogleGemini
            | ProviderProtocolFamily::Cohere
            | ProviderProtocolFamily::Unsupported => json!({}),
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
        let client = client_for_url(request.models_url.as_str(), 12)?;
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
        let client = client_for_url(&url, 30).map_err(|e| ("other".into(), Some(e.to_string())))?;
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

#[async_trait]
impl ProviderAdapter for AzureOpenAIAdapter {
    fn build_auth_headers(
        &self,
        api_key: &str,
    ) -> Result<reqwest::header::HeaderMap, (String, Option<String>)> {
        use reqwest::header::{HeaderMap, HeaderValue};

        let mut headers = HeaderMap::new();
        let api_key_value =
            HeaderValue::from_str(api_key).map_err(|e| ("other".into(), Some(e.to_string())))?;
        headers.insert("api-key", api_key_value);
        Ok(headers)
    }

    fn normalize_error(
        &self,
        status: StatusCode,
        _content_type: Option<&str>,
        bytes: &[u8],
    ) -> (String, Option<String>) {
        classify_azure_error(status, bytes)
    }

    async fn list_models(
        &self,
        _request: ListModelsRequest<'_>,
    ) -> Result<Vec<String>, GatewayError> {
        Err(GatewayError::Config(
            "Azure OpenAI 当前以 deployment + 手动模型为主，本轮暂不支持自动拉取模型列表。".into(),
        ))
    }

    async fn test_connection(
        &self,
        request: ConnectionTestRequest<'_>,
    ) -> Result<(), (String, Option<String>)> {
        let url = azure_openai_chat_completions_url(request.base_url, request.provider_config)?;
        let client = client_for_url(&url, 30).map_err(|e| ("other".into(), Some(e.to_string())))?;
        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({
                "messages": [{"role":"user","content":"ping"}],
                "max_tokens": 1,
                "temperature": 0
            }));

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
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ("other".into(), Some(e.to_string())))?;

        if !status.is_success() {
            return Err(self.normalize_error(status, None, &bytes));
        }

        Ok(())
    }
}

#[async_trait]
impl ProviderAdapter for GoogleGeminiAdapter {
    fn build_auth_headers(
        &self,
        _api_key: &str,
    ) -> Result<reqwest::header::HeaderMap, (String, Option<String>)> {
        Ok(reqwest::header::HeaderMap::new())
    }

    fn normalize_error(
        &self,
        status: StatusCode,
        _content_type: Option<&str>,
        bytes: &[u8],
    ) -> (String, Option<String>) {
        classify_gemini_error(status, bytes)
    }

    async fn list_models(
        &self,
        request: ListModelsRequest<'_>,
    ) -> Result<Vec<String>, GatewayError> {
        let url =
            append_query_api_key(request.models_url, request.api_key).map_err(|(_, detail)| {
                GatewayError::Config(
                    detail.unwrap_or_else(|| "Google Gemini API Key 配置无效。".into()),
                )
            })?;
        let client = client_for_url(&url, 12)?;
        let resp = client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let (error_type, detail) = self.normalize_error(status, None, &bytes);
            return Err(gateway_error_from_normalized(
                &error_type,
                detail.unwrap_or_else(|| gemini_error_message(status, &bytes)),
            ));
        }

        let parsed: GeminiModelsResponse = serde_json::from_slice(&bytes)
            .map_err(|_| GatewayError::Config("解析 Google Gemini 模型列表失败。".into()))?;
        let mut models: Vec<String> = parsed
            .models
            .into_iter()
            .filter(|model| {
                model.supported_generation_methods.is_empty()
                    || model
                        .supported_generation_methods
                        .iter()
                        .any(|method| method == "generateContent")
            })
            .map(|model| {
                model
                    .name
                    .strip_prefix("models/")
                    .unwrap_or(model.name.as_str())
                    .to_string()
            })
            .collect();
        models.sort();
        models.dedup();
        Ok(models)
    }

    async fn test_connection(
        &self,
        request: ConnectionTestRequest<'_>,
    ) -> Result<(), (String, Option<String>)> {
        let model = gemini_model_name(request.model)?;
        let base = gemini_base_url(request.base_url, request.provider_config);
        let url = Url::parse(&format!("{}/{}:generateContent", base, model))
            .map_err(|e| ("invalid_path".into(), Some(e.to_string())))?;
        let url = append_query_api_key(&url, request.api_key)?;
        let client = client_for_url(&url, 30).map_err(|e| ("other".into(), Some(e.to_string())))?;
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({
                "contents": [{"role": "user", "parts": [{"text": "ping"}]}],
                "generationConfig": {"maxOutputTokens": 1, "temperature": 0}
            }))
            .send()
            .await
            .map_err(|e| (classify_reqwest_error(&e), Some(e.to_string())))?;
        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ("other".into(), Some(e.to_string())))?;

        if !status.is_success() {
            return Err(self.normalize_error(status, None, &bytes));
        }

        Ok(())
    }
}

#[async_trait]
impl ProviderAdapter for CohereAdapter {
    fn build_auth_headers(
        &self,
        api_key: &str,
    ) -> Result<reqwest::header::HeaderMap, (String, Option<String>)> {
        use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

        let mut headers = HeaderMap::new();
        let value = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|e| ("other".into(), Some(e.to_string())))?;
        headers.insert(AUTHORIZATION, value);
        Ok(headers)
    }

    fn normalize_error(
        &self,
        status: StatusCode,
        _content_type: Option<&str>,
        bytes: &[u8],
    ) -> (String, Option<String>) {
        classify_cohere_error(status, bytes)
    }

    async fn list_models(
        &self,
        request: ListModelsRequest<'_>,
    ) -> Result<Vec<String>, GatewayError> {
        let client = client_for_url(request.models_url.as_str(), 12)?;
        let mut req = client
            .get(request.models_url.as_str())
            .header("Accept", "application/json");
        for (name, value) in self
            .build_auth_headers(request.api_key)
            .map_err(|(_, detail)| {
                GatewayError::Config(detail.unwrap_or_else(|| "Cohere API Key 配置无效。".into()))
            })?
        {
            if let Some(name) = name {
                req = req.header(name, value);
            }
        }

        let resp = req.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let (error_type, detail) = self.normalize_error(status, None, &bytes);
            return Err(gateway_error_from_normalized(
                &error_type,
                detail.unwrap_or_else(|| cohere_error_message(status, &bytes)),
            ));
        }

        let parsed: CohereModelsResponse = serde_json::from_slice(&bytes)
            .map_err(|_| GatewayError::Config("解析 Cohere 模型列表失败。".into()))?;
        let mut models: Vec<String> = parsed
            .models
            .into_iter()
            .filter(|model| {
                model.endpoints.is_empty()
                    || model.endpoints.iter().any(|endpoint| endpoint == "chat")
            })
            .map(|model| model.name)
            .collect();
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
                Some("Cohere 测试时需要填写模型名称。".into()),
            ));
        }

        let base = request.base_url.as_str().trim_end_matches('/');
        let url = format!("{}/v2/chat", base);
        let client = client_for_url(&url, 30).map_err(|e| ("other".into(), Some(e.to_string())))?;
        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({
                "model": model,
                "messages": [{"role": "user", "content": "ping"}],
                "max_tokens": 1,
                "temperature": 0
            }));

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
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ("other".into(), Some(e.to_string())))?;

        if !status.is_success() {
            return Err(self.normalize_error(status, None, &bytes));
        }

        Ok(())
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
        assert!(adapter_for(ProviderType::AzureOpenAI).is_some());
        assert!(adapter_for(ProviderType::GoogleGemini).is_some());
        assert!(adapter_for(ProviderType::Cohere).is_some());
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

        let azure_headers = adapter_for(ProviderType::AzureOpenAI)
            .unwrap()
            .build_auth_headers("sk-test")
            .unwrap();
        assert!(azure_headers.contains_key("api-key"));
    }

    #[test]
    fn azure_url_uses_openai_deployment_path() {
        let url = azure_openai_chat_completions_url(
            &Url::parse("https://demo.openai.azure.com").unwrap(),
            &ProviderConfig {
                azure_deployment: Some("gpt-4o-prod".into()),
                azure_api_version: Some("2024-06-01".into()),
                google_api_version: None,
            },
        )
        .unwrap();
        assert_eq!(
            url,
            "https://demo.openai.azure.com/openai/deployments/gpt-4o-prod/chat/completions?api-version=2024-06-01"
        );
    }

    #[test]
    fn gemini_base_url_respects_explicit_version() {
        let url = gemini_base_url(
            &Url::parse("https://generativelanguage.googleapis.com").unwrap(),
            &ProviderConfig {
                azure_deployment: None,
                azure_api_version: None,
                google_api_version: Some("v1".into()),
            },
        );
        assert_eq!(url, "https://generativelanguage.googleapis.com/v1");
    }

    #[test]
    fn gemini_model_name_accepts_prefixed_and_plain_values() {
        assert_eq!(
            gemini_model_name("gemini-2.0-flash").unwrap(),
            "models/gemini-2.0-flash"
        );
        assert_eq!(
            gemini_model_name("models/gemini-2.0-flash").unwrap(),
            "models/gemini-2.0-flash"
        );
    }
}
