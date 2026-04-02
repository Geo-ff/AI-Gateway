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
use crate::providers::openai::{
    ChatCompletionRequest, ChatCompletionResponse, ModelListResponse, RawAndTypedChatCompletion,
};

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

pub struct ChatCompletionsRequest<'a> {
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub provider_config: &'a ProviderConfig,
    pub request: &'a ChatCompletionRequest,
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

    async fn chat_completions(
        &self,
        request: ChatCompletionsRequest<'_>,
    ) -> Result<RawAndTypedChatCompletion, GatewayError> {
        let _ = request;
        Err(GatewayError::Config(
            "当前 provider adapter 未实现真实聊天请求链路。".into(),
        ))
    }

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

pub async fn runtime_chat_completions(
    provider_type: ProviderType,
    request: ChatCompletionsRequest<'_>,
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    let adapter = adapter_for(provider_type)
        .ok_or_else(|| GatewayError::Config(unsupported_provider_message(provider_type)))?;
    adapter.chat_completions(request).await
}

pub fn runtime_streaming_unsupported_message(provider_type: ProviderType) -> Option<String> {
    match provider_type {
        ProviderType::AzureOpenAI => {
            Some("Azure OpenAI 当前仅支持非流式真实请求，stream=true 暂未实现。".into())
        }
        ProviderType::GoogleGemini => {
            Some("Google Gemini 当前仅支持非流式真实请求，stream=true 暂未实现。".into())
        }
        ProviderType::Cohere => {
            Some("Cohere 当前仅支持非流式真实请求，stream=true 暂未实现。".into())
        }
        _ => None,
    }
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

    if status == StatusCode::TOO_MANY_REQUESTS {
        return ("rate_limited".into(), Some(snippet.to_string()));
    }

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
        "rate_limited" => GatewayError::RateLimited(fallback_message),
        _ => GatewayError::Config(fallback_message),
    }
}

fn request_value(request: &ChatCompletionRequest) -> Result<serde_json::Value, GatewayError> {
    serde_json::to_value(request).map_err(GatewayError::from)
}

fn request_messages(value: &serde_json::Value) -> Vec<serde_json::Value> {
    value
        .get("messages")
        .and_then(|messages| messages.as_array())
        .cloned()
        .unwrap_or_default()
}

fn extract_text_fragments(content: &serde_json::Value) -> Vec<String> {
    match content {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
        serde_json::Value::Array(parts) => parts.iter().flat_map(extract_text_fragments).collect(),
        serde_json::Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(|value| value.as_str()) {
                let trimmed = text.trim();
                return if trimmed.is_empty() {
                    Vec::new()
                } else {
                    vec![trimmed.to_string()]
                };
            }

            if let Some(refusal) = object.get("refusal").and_then(|value| value.as_str()) {
                let trimmed = refusal.trim();
                return if trimmed.is_empty() {
                    Vec::new()
                } else {
                    vec![trimmed.to_string()]
                };
            }

            if let Some(url) = object
                .get("image_url")
                .and_then(|value| value.get("url"))
                .and_then(|value| value.as_str())
            {
                let trimmed = url.trim();
                return if trimmed.is_empty() {
                    Vec::new()
                } else {
                    vec![format!("[image] {trimmed}")]
                };
            }

            if object.get("input_audio").is_some() {
                return vec!["[audio input omitted]".into()];
            }

            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn extract_message_text(message: &serde_json::Value) -> String {
    let mut segments = Vec::new();

    if let Some(content) = message.get("content") {
        segments.extend(extract_text_fragments(content));
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(|value| value.as_array()) {
        for tool_call in tool_calls {
            let name = tool_call
                .get("function")
                .and_then(|value| value.get("name"))
                .and_then(|value| value.as_str())
                .unwrap_or("tool");
            let arguments = tool_call
                .get("function")
                .and_then(|value| value.get("arguments"))
                .and_then(|value| value.as_str())
                .unwrap_or("{}");
            segments.push(format!("[tool_call:{name}] {arguments}"));
        }
    }

    if message.get("role").and_then(|value| value.as_str()) == Some("tool") {
        let tool_id = message
            .get("tool_call_id")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown_tool");
        let joined = segments.join("\n");
        return if joined.trim().is_empty() {
            format!("Tool result ({tool_id})")
        } else {
            format!("Tool result ({tool_id}):\n{joined}")
        };
    }

    segments.join("\n")
}

fn combined_system_prompt(messages: &[serde_json::Value]) -> Option<String> {
    let mut segments = Vec::new();

    for message in messages {
        let role = message
            .get("role")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if matches!(role, "system" | "developer") {
            let text = extract_message_text(message);
            if !text.trim().is_empty() {
                segments.push(text);
            }
        }
    }

    if segments.is_empty() {
        None
    } else {
        Some(segments.join("\n\n"))
    }
}

fn string_stop_sequences(value: &serde_json::Value) -> Vec<String> {
    match value.get("stop") {
        Some(serde_json::Value::String(stop)) => vec![stop.to_string()],
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .filter(|item| !item.trim().is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn build_openai_style_response(
    id: Option<String>,
    model: &str,
    content: String,
    finish_reason: Option<&str>,
    usage: Option<serde_json::Value>,
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    let mut raw = json!({
        "id": id.unwrap_or_else(|| format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis())),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp().max(0) as u64,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content,
            },
            "finish_reason": finish_reason.unwrap_or("stop"),
        }],
    });

    if let Some(usage) = usage
        && let Some(object) = raw.as_object_mut()
    {
        object.insert("usage".into(), usage);
    }

    let typed: ChatCompletionResponse = serde_json::from_value(raw.clone())?;
    Ok(RawAndTypedChatCompletion { typed, raw })
}

fn parse_openai_compatible_response(
    bytes: &[u8],
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    let raw: serde_json::Value = serde_json::from_slice(bytes)?;
    let typed: ChatCompletionResponse = serde_json::from_value(raw.clone())?;
    Ok(RawAndTypedChatCompletion { typed, raw })
}

fn gemini_finish_reason(reason: Option<&str>) -> &'static str {
    match reason.unwrap_or_default() {
        "MAX_TOKENS" => "length",
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" => "content_filter",
        _ => "stop",
    }
}

fn cohere_finish_reason(reason: Option<&str>) -> &'static str {
    match reason.unwrap_or_default() {
        "MAX_TOKENS" => "length",
        _ => "stop",
    }
}

fn build_gemini_payload(
    request: &ChatCompletionRequest,
) -> Result<serde_json::Value, GatewayError> {
    let value = request_value(request)?;
    let messages = request_messages(&value);
    let mut contents = Vec::new();

    for message in &messages {
        let role = message
            .get("role")
            .and_then(|role| role.as_str())
            .unwrap_or("user");

        if matches!(role, "system" | "developer") {
            continue;
        }

        let text = extract_message_text(message);
        if text.trim().is_empty() {
            continue;
        }

        let gemini_role = if role == "assistant" { "model" } else { "user" };
        contents.push(json!({
            "role": gemini_role,
            "parts": [{ "text": text }],
        }));
    }

    if contents.is_empty() {
        return Err(GatewayError::Config(
            "Google Gemini 请求缺少可转换的消息内容。".into(),
        ));
    }

    let mut payload = json!({ "contents": contents });
    if let Some(system) = combined_system_prompt(&messages)
        && let Some(object) = payload.as_object_mut()
    {
        object.insert(
            "systemInstruction".into(),
            json!({ "parts": [{ "text": system }] }),
        );
    }

    let mut generation_config = serde_json::Map::new();
    if let Some(value) = value.get("temperature").cloned() {
        generation_config.insert("temperature".into(), value);
    }
    if let Some(value) = value.get("top_p").cloned() {
        generation_config.insert("topP".into(), value);
    }
    if let Some(value) = value
        .get("max_completion_tokens")
        .cloned()
        .or_else(|| value.get("max_tokens").cloned())
    {
        generation_config.insert("maxOutputTokens".into(), value);
    }
    let stop_sequences = string_stop_sequences(&value);
    if !stop_sequences.is_empty() {
        generation_config.insert("stopSequences".into(), json!(stop_sequences));
    }

    if !generation_config.is_empty()
        && let Some(object) = payload.as_object_mut()
    {
        object.insert(
            "generationConfig".into(),
            serde_json::Value::Object(generation_config),
        );
    }

    Ok(payload)
}

fn adapt_gemini_response(
    model: &str,
    bytes: &[u8],
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let candidate = value
        .get("candidates")
        .and_then(|candidates| candidates.as_array())
        .and_then(|candidates| candidates.first())
        .cloned();

    let Some(candidate) = candidate else {
        let block_reason = value
            .get("promptFeedback")
            .and_then(|feedback| feedback.get("blockReason"))
            .and_then(|reason| reason.as_str())
            .unwrap_or("unknown");
        return Err(GatewayError::Config(format!(
            "Google Gemini 未返回可用候选内容（block reason: {block_reason}）。"
        )));
    };

    let content = candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(|parts| parts.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(|text| text.as_str())
                        .map(str::to_string)
                        .or_else(|| {
                            part.get("functionCall").map(|call| {
                                let name = call
                                    .get("name")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("tool");
                                let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
                                format!("[function_call:{name}] {args}")
                            })
                        })
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let usage = value.get("usageMetadata").map(|usage| {
        json!({
            "prompt_tokens": usage
                .get("promptTokenCount")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            "completion_tokens": usage
                .get("candidatesTokenCount")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            "total_tokens": usage
                .get("totalTokenCount")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
        })
    });

    build_openai_style_response(
        value
            .get("responseId")
            .and_then(|response_id| response_id.as_str())
            .map(str::to_string),
        model,
        content,
        candidate
            .get("finishReason")
            .and_then(|reason| reason.as_str())
            .map(|reason| gemini_finish_reason(Some(reason))),
        usage,
    )
}

fn build_cohere_payload(
    request: &ChatCompletionRequest,
) -> Result<serde_json::Value, GatewayError> {
    let value = request_value(request)?;
    let messages = request_messages(&value);
    let model = value
        .get("model")
        .and_then(|model| model.as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .ok_or_else(|| GatewayError::Config("Cohere 请求缺少模型名称。".into()))?;

    let mut payload_messages = Vec::new();
    for message in &messages {
        let role = message
            .get("role")
            .and_then(|role| role.as_str())
            .unwrap_or("user");
        if matches!(role, "system" | "developer") {
            continue;
        }

        let text = extract_message_text(message);
        if text.trim().is_empty() {
            continue;
        }

        payload_messages.push(json!({
            "role": if role == "assistant" { "assistant" } else if role == "tool" { "tool" } else { "user" },
            "content": text,
        }));
    }

    if payload_messages.is_empty() {
        return Err(GatewayError::Config(
            "Cohere 请求缺少可转换的消息内容。".into(),
        ));
    }

    let mut payload = json!({
        "model": model,
        "messages": payload_messages,
    });
    if let Some(system) = combined_system_prompt(&messages)
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("preamble".into(), json!(system));
    }
    if let Some(value) = value.get("temperature").cloned()
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("temperature".into(), value);
    }
    if let Some(value) = value.get("top_p").cloned()
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("p".into(), value);
    }
    if let Some(value) = value
        .get("max_completion_tokens")
        .cloned()
        .or_else(|| value.get("max_tokens").cloned())
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("max_tokens".into(), value);
    }
    let stop_sequences = string_stop_sequences(&value);
    if !stop_sequences.is_empty()
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("stop_sequences".into(), json!(stop_sequences));
    }

    Ok(payload)
}

fn adapt_cohere_response(
    model: &str,
    bytes: &[u8],
) -> Result<RawAndTypedChatCompletion, GatewayError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let content = value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_array())
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(|text| text.as_str())
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|content| !content.trim().is_empty())
        .or_else(|| {
            value
                .get("text")
                .and_then(|text| text.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();

    let usage = value.get("usage").map(|usage| {
        let tokens = usage.get("tokens");
        let billed_units = usage.get("billed_units");
        let prompt_tokens = tokens
            .and_then(|tokens| tokens.get("input_tokens"))
            .and_then(|value| value.as_u64())
            .or_else(|| {
                billed_units
                    .and_then(|units| units.get("input_tokens"))
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(0);
        let completion_tokens = tokens
            .and_then(|tokens| tokens.get("output_tokens"))
            .and_then(|value| value.as_u64())
            .or_else(|| {
                billed_units
                    .and_then(|units| units.get("output_tokens"))
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(0);
        json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens,
        })
    });

    build_openai_style_response(
        value
            .get("id")
            .and_then(|id| id.as_str())
            .map(str::to_string),
        model,
        content,
        value
            .get("finish_reason")
            .and_then(|reason| reason.as_str())
            .map(|reason| cohere_finish_reason(Some(reason))),
        usage,
    )
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

    async fn chat_completions(
        &self,
        request: ChatCompletionsRequest<'_>,
    ) -> Result<RawAndTypedChatCompletion, GatewayError> {
        let base_url = Url::parse(request.base_url)
            .map_err(|err| GatewayError::Config(format!("Azure OpenAI base_url 无效：{err}")))?;
        let url = azure_openai_chat_completions_url(&base_url, request.provider_config).map_err(
            |(_, detail)| {
                GatewayError::Config(detail.unwrap_or_else(|| "Azure OpenAI 配置不完整。".into()))
            },
        )?;
        let client = client_for_url(&url, 60)?;
        let mut payload = request_value(request.request)?;
        if let Some(object) = payload.as_object_mut() {
            object.remove("model");
        }

        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&payload);
        for (name, value) in self
            .build_auth_headers(request.api_key)
            .map_err(|(_, detail)| {
                GatewayError::Config(
                    detail.unwrap_or_else(|| "Azure OpenAI API Key 配置无效。".into()),
                )
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
            let (error_type, detail) = classify_azure_error(status, &bytes);
            return Err(gateway_error_from_normalized(
                &error_type,
                detail.unwrap_or_else(|| azure_error_message(status, &bytes)),
            ));
        }

        parse_openai_compatible_response(&bytes)
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

    async fn chat_completions(
        &self,
        request: ChatCompletionsRequest<'_>,
    ) -> Result<RawAndTypedChatCompletion, GatewayError> {
        let base_url = Url::parse(request.base_url)
            .map_err(|err| GatewayError::Config(format!("Google Gemini base_url 无效：{err}")))?;
        let model = gemini_model_name(&request.request.model).map_err(|(_, detail)| {
            GatewayError::Config(
                detail.unwrap_or_else(|| "Google Gemini 需要填写模型名称。".into()),
            )
        })?;
        let base = gemini_base_url(&base_url, request.provider_config);
        let url = Url::parse(&format!("{}/{}:generateContent", base, model))
            .map_err(|err| GatewayError::Config(format!("Google Gemini 请求地址无效：{err}")))?;
        let url = append_query_api_key(&url, request.api_key).map_err(|(_, detail)| {
            GatewayError::Config(
                detail.unwrap_or_else(|| "Google Gemini API Key 配置无效。".into()),
            )
        })?;
        let client = client_for_url(&url, 60)?;
        let payload = build_gemini_payload(request.request)?;
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&payload)
            .send()
            .await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            let (error_type, detail) = classify_gemini_error(status, &bytes);
            return Err(gateway_error_from_normalized(
                &error_type,
                detail.unwrap_or_else(|| gemini_error_message(status, &bytes)),
            ));
        }

        adapt_gemini_response(&request.request.model, &bytes)
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

    async fn chat_completions(
        &self,
        request: ChatCompletionsRequest<'_>,
    ) -> Result<RawAndTypedChatCompletion, GatewayError> {
        let base_url = request.base_url.trim_end_matches('/');
        let url = format!("{}/v2/chat", base_url);
        let client = client_for_url(&url, 60)?;
        let payload = build_cohere_payload(request.request)?;
        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&payload);
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
            let (error_type, detail) = classify_cohere_error(status, &bytes);
            return Err(gateway_error_from_normalized(
                &error_type,
                detail.unwrap_or_else(|| cohere_error_message(status, &bytes)),
            ));
        }

        adapt_cohere_response(&request.request.model, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn gemini_payload_uses_system_instruction_and_contents() {
        let request: ChatCompletionRequest = serde_json::from_value(json!({
            "model": "gemini-2.0-flash",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"}
            ],
            "temperature": 0.2,
            "top_p": 0.9,
            "max_tokens": 64
        }))
        .unwrap();

        let payload = build_gemini_payload(&request).unwrap();
        assert_eq!(
            payload["systemInstruction"]["parts"][0]["text"],
            json!("You are helpful")
        );
        assert_eq!(payload["contents"][0]["role"], json!("user"));
        assert_eq!(payload["contents"][1]["role"], json!("model"));
        assert_eq!(payload["generationConfig"]["maxOutputTokens"], json!(64));
    }

    #[test]
    fn gemini_response_is_adapted_to_openai_shape() {
        let adapted = adapt_gemini_response(
            "gemini-2.0-flash",
            serde_json::to_vec(&json!({
                "responseId": "gemini-resp-1",
                "candidates": [{
                    "finishReason": "STOP",
                    "content": {
                        "parts": [{"text": "hello from gemini"}]
                    }
                }],
                "usageMetadata": {
                    "promptTokenCount": 12,
                    "candidatesTokenCount": 7,
                    "totalTokenCount": 19
                }
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();

        assert_eq!(adapted.raw["model"], json!("gemini-2.0-flash"));
        assert_eq!(
            adapted.raw["choices"][0]["message"]["content"],
            json!("hello from gemini")
        );
        assert_eq!(adapted.raw["usage"]["total_tokens"], json!(19));
    }

    #[test]
    fn cohere_payload_uses_native_message_shape() {
        let request: ChatCompletionRequest = serde_json::from_value(json!({
            "model": "command-r-plus",
            "messages": [
                {"role": "system", "content": "Be concise"},
                {"role": "user", "content": "Hello Cohere"}
            ],
            "temperature": 0,
            "max_tokens": 32
        }))
        .unwrap();

        let payload = build_cohere_payload(&request).unwrap();
        assert_eq!(payload["model"], json!("command-r-plus"));
        assert_eq!(payload["preamble"], json!("Be concise"));
        assert_eq!(payload["messages"][0]["role"], json!("user"));
        assert_eq!(payload["messages"][0]["content"], json!("Hello Cohere"));
    }

    #[test]
    fn cohere_response_is_adapted_to_openai_shape() {
        let adapted = adapt_cohere_response(
            "command-r-plus",
            serde_json::to_vec(&json!({
                "id": "cohere-chat-1",
                "finish_reason": "COMPLETE",
                "message": {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "hello from cohere"}]
                },
                "usage": {
                    "tokens": {
                        "input_tokens": 9,
                        "output_tokens": 4
                    }
                }
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();

        assert_eq!(adapted.raw["id"], json!("cohere-chat-1"));
        assert_eq!(
            adapted.raw["choices"][0]["message"]["content"],
            json!("hello from cohere")
        );
        assert_eq!(adapted.raw["usage"]["prompt_tokens"], json!(9));
        assert_eq!(adapted.raw["usage"]["completion_tokens"], json!(4));
    }

    #[test]
    fn runtime_streaming_boundary_is_explicit_for_new_native_providers() {
        assert!(runtime_streaming_unsupported_message(ProviderType::AzureOpenAI).is_some());
        assert!(runtime_streaming_unsupported_message(ProviderType::GoogleGemini).is_some());
        assert!(runtime_streaming_unsupported_message(ProviderType::Cohere).is_some());
        assert!(runtime_streaming_unsupported_message(ProviderType::OpenAI).is_none());
    }
}
