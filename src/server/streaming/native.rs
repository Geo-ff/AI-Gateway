use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

use async_openai::types::ChatCompletionStreamOptions;
use axum::response::{IntoResponse, Response, Sse};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::{Url, header::HeaderValue};
use serde_json::{Value, json};

use crate::config::ProviderType;
use crate::config::settings::ProviderConfig;
use crate::error::GatewayError;
use crate::providers::{
    adapters::{
        aws_claude_error_message, aws_claude_finish_reason, aws_sigv4_headers,
        azure_error_message, azure_openai_chat_completions_url, baidu_access_token,
        baidu_ernie_chat_url, baidu_error_response, baidu_requires_error,
        build_aws_claude_payload, build_baidu_ernie_payload, build_cohere_payload,
        build_gemini_payload, classify_aws_claude_error, classify_azure_error,
        classify_cohere_error, classify_gemini_error, classify_vertex_error,
        cohere_error_message, cohere_finish_reason, gemini_error_message,
        gemini_finish_reason, gemini_generate_content_url, gateway_error_from_normalized,
        vertex_access_token, vertex_error_message, vertex_stream_generate_content_url,
    },
    openai::{ChatCompletionRequest, Usage},
};
use crate::server::{AppState, util::mask_key};

fn join_openai_compat_endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let normalized_path = path.trim_start_matches('/');
    let base_path = match reqwest::Url::parse(base) {
        Ok(u) => u.path().trim_end_matches('/').to_string(),
        Err(_) => String::new(),
    };

    if base_path.ends_with("/v1") || base_path.ends_with("/api/v3") {
        format!("{}/{}", base, normalized_path)
    } else {
        format!("{}/v1/{}", base, normalized_path)
    }
}

#[derive(Debug)]
enum NormalizedStreamEvent {
    RoleStart,
    TextDelta(String),
    Finish(String),
    Usage(Usage),
}

struct OpenAiSseEmitter {
    tx: tokio::sync::mpsc::UnboundedSender<axum::response::sse::Event>,
    id: String,
    created: u64,
    model: String,
    sent_role: bool,
    sent_finish: bool,
    sent_done: bool,
}

impl OpenAiSseEmitter {
    fn new(
        tx: tokio::sync::mpsc::UnboundedSender<axum::response::sse::Event>,
        model: String,
    ) -> Self {
        Self {
            tx,
            id: format!("chatcmpl-{}", Utc::now().timestamp_millis()),
            created: Utc::now().timestamp().max(0) as u64,
            model,
            sent_role: false,
            sent_finish: false,
            sent_done: false,
        }
    }

    fn send_json(&self, value: Value) -> bool {
        self.tx
            .send(axum::response::sse::Event::default().data(value.to_string()))
            .is_ok()
    }

    fn ensure_role_start(&mut self) -> bool {
        if self.sent_role {
            return true;
        }
        self.sent_role = true;
        self.send_json(json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": {"role": "assistant"},
                "finish_reason": Value::Null,
            }],
        }))
    }

    fn emit(&mut self, event: NormalizedStreamEvent) -> bool {
        match event {
            NormalizedStreamEvent::RoleStart => self.ensure_role_start(),
            NormalizedStreamEvent::TextDelta(text) => {
                if text.is_empty() {
                    return true;
                }
                self.ensure_role_start()
                    && self.send_json(json!({
                        "id": self.id,
                        "object": "chat.completion.chunk",
                        "created": self.created,
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": {"content": text},
                            "finish_reason": Value::Null,
                        }],
                    }))
            }
            NormalizedStreamEvent::Finish(reason) => {
                if self.sent_finish {
                    return true;
                }
                self.sent_finish = true;
                self.ensure_role_start()
                    && self.send_json(json!({
                        "id": self.id,
                        "object": "chat.completion.chunk",
                        "created": self.created,
                        "model": self.model,
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": reason,
                        }],
                    }))
            }
            NormalizedStreamEvent::Usage(usage) => self.send_json(json!({
                "id": self.id,
                "object": "chat.completion.chunk",
                "created": self.created,
                "model": self.model,
                "choices": [],
                "usage": usage,
            })),
        }
    }

    fn emit_error(&self, message: String) -> bool {
        self.tx
            .send(axum::response::sse::Event::default().data(format!("error: {message}")))
            .is_ok()
    }

    fn emit_done(&mut self) -> bool {
        if self.sent_done {
            return true;
        }
        self.sent_done = true;
        self.tx
            .send(axum::response::sse::Event::default().data("[DONE]"))
            .is_ok()
    }
}

#[derive(Default)]
struct SseMessageDecoder {
    buffer: String,
}

struct SseMessage {
    event: String,
    data: String,
}

impl SseMessageDecoder {
    fn push(&mut self, chunk: &str) -> Vec<SseMessage> {
        self.buffer.push_str(chunk);
        self.buffer = self.buffer.replace("\r\n", "\n").replace('\r', "\n");

        let mut out = Vec::new();
        while let Some(idx) = self.buffer.find("\n\n") {
            let raw = self.buffer[..idx].to_string();
            self.buffer.drain(..idx + 2);
            if let Some(message) = parse_sse_message(&raw) {
                out.push(message);
            }
        }
        out
    }
}

fn parse_sse_message(raw: &str) -> Option<SseMessage> {
    let mut event = String::new();
    let mut data_lines = Vec::new();

    for line in raw.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event = rest.trim().to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }

    if data_lines.is_empty() {
        None
    } else {
        Some(SseMessage {
            event,
            data: data_lines.join("\n"),
        })
    }
}

#[derive(Default)]
struct JsonObjectStreamDecoder {
    buffer: String,
}

impl JsonObjectStreamDecoder {
    fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        extract_complete_json_objects(&mut self.buffer)
    }
}

fn extract_complete_json_objects(buffer: &mut String) -> Vec<String> {
    let bytes = buffer.as_bytes();
    let mut out = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut last_consumed = 0usize;

    for (idx, ch) in bytes.iter().copied().enumerate() {
        if let Some(begin) = start {
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == b'\\' {
                    escaped = true;
                } else if ch == b'"' {
                    in_string = false;
                }
                continue;
            }

            match ch {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if let Some(fragment) = buffer.get(begin..=idx) {
                            out.push(fragment.to_string());
                        }
                        last_consumed = idx + 1;
                        start = None;
                    }
                }
                _ => {}
            }
        } else if ch == b'{' {
            start = Some(idx);
            depth = 1;
            in_string = false;
            escaped = false;
        }
    }

    if last_consumed > 0 {
        buffer.drain(..last_consumed);
    }

    out
}

#[derive(Default)]
struct AwsEventStreamDecoder {
    buffer: Vec<u8>,
}

impl AwsEventStreamDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();

        loop {
            if self.buffer.len() < 12 {
                break;
            }

            let total_len =
                u32::from_be_bytes(self.buffer[0..4].try_into().unwrap_or([0, 0, 0, 0])) as usize;
            let headers_len =
                u32::from_be_bytes(self.buffer[4..8].try_into().unwrap_or([0, 0, 0, 0])) as usize;

            if total_len < 16 {
                return Err("AWS Bedrock 返回了无效的 EventStream 帧长度。".into());
            }
            if self.buffer.len() < total_len {
                break;
            }

            let payload_start = 12usize.saturating_add(headers_len);
            let payload_end = total_len.saturating_sub(4);
            if payload_start > payload_end || payload_end > total_len {
                return Err("AWS Bedrock EventStream 帧头解析失败。".into());
            }

            frames.push(self.buffer[payload_start..payload_end].to_vec());
            self.buffer.drain(..total_len);
        }

        Ok(frames)
    }
}

fn usage_from_counts(prompt_tokens: u64, completion_tokens: u64, total_tokens: Option<u64>) -> Usage {
    Usage {
        prompt_tokens: prompt_tokens as u32,
        completion_tokens: completion_tokens as u32,
        total_tokens: total_tokens.unwrap_or(prompt_tokens + completion_tokens) as u32,
        prompt_tokens_details: None,
        completion_tokens_details: None,
    }
}

fn handle_normalized_events(
    emitter: &mut OpenAiSseEmitter,
    usage_cell: &Arc<Mutex<Option<Usage>>>,
    events: Vec<NormalizedStreamEvent>,
) -> Result<bool, String> {
    for event in events {
        if let NormalizedStreamEvent::Usage(usage) = &event {
            *usage_cell.lock().unwrap() = Some(usage.clone());
        }
        if !emitter.emit(event) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parse_azure_chunk(data: &str) -> Result<Vec<NormalizedStreamEvent>, String> {
    let value: Value = serde_json::from_str(data)
        .map_err(|err| format!("Azure OpenAI 流式响应解析失败：{err}"))?;
    let mut events = Vec::new();

    if let Some(usage) = value.get("usage")
        && !usage.is_null()
    {
        let prompt = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total = usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .or(Some(prompt + completion));
        events.push(NormalizedStreamEvent::Usage(usage_from_counts(
            prompt, completion, total,
        )));
    }

    if let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    {
        if choice
            .get("delta")
            .and_then(|delta| delta.get("role"))
            .and_then(Value::as_str)
            == Some("assistant")
        {
            events.push(NormalizedStreamEvent::RoleStart);
        }
        if let Some(text) = choice
            .get("delta")
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            events.push(NormalizedStreamEvent::TextDelta(text.to_string()));
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            events.push(NormalizedStreamEvent::Finish(reason.to_string()));
        }
    }

    Ok(events)
}

fn parse_generate_content_response(
    data: &str,
    provider_label: &str,
) -> Result<Vec<NormalizedStreamEvent>, String> {
    let value: Value = serde_json::from_str(data)
        .map_err(|err| format!("{provider_label} 流式响应解析失败：{err}"))?;
    let mut events = Vec::new();

    if let Some(prompt_feedback) = value.get("promptFeedback")
        && value
            .get("candidates")
            .and_then(Value::as_array)
            .is_none_or(|items| items.is_empty())
    {
        let block_reason = prompt_feedback
            .get("blockReason")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN");
        return Err(format!(
            "{provider_label} 因 promptFeedback.blockReason={block_reason} 拒绝了本次流式请求。"
        ));
    }

    if let Some(candidate) = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
    {
        if candidate
            .get("content")
            .and_then(|content| content.get("role"))
            .and_then(Value::as_str)
            == Some("model")
        {
            events.push(NormalizedStreamEvent::RoleStart);
        }

        if let Some(parts) = candidate
            .get("content")
            .and_then(|content| content.get("parts"))
            .and_then(Value::as_array)
        {
            for text in parts.iter().filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
            }) {
                events.push(NormalizedStreamEvent::TextDelta(text.to_string()));
            }
        }

        if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
            events.push(NormalizedStreamEvent::Finish(
                gemini_finish_reason(Some(reason)).to_string(),
            ));
        }
    }

    if let Some(usage) = value.get("usageMetadata") {
        let prompt = usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion = usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total = usage
            .get("totalTokenCount")
            .and_then(Value::as_u64)
            .or(Some(prompt + completion));
        events.push(NormalizedStreamEvent::Usage(usage_from_counts(
            prompt, completion, total,
        )));
    }

    Ok(events)
}

fn parse_cohere_event(event_name: &str, data: &str) -> Result<Vec<NormalizedStreamEvent>, String> {
    let value: Value = serde_json::from_str(data)
        .map_err(|err| format!("Cohere 流式事件解析失败：{err}"))?;
    let event_name = if event_name.is_empty() {
        value.get("type").and_then(Value::as_str).unwrap_or_default()
    } else {
        event_name
    };
    let mut events = Vec::new();

    match event_name {
        "message-start" => events.push(NormalizedStreamEvent::RoleStart),
        "content-delta" => {
            if let Some(text) = value
                .get("delta")
                .and_then(|delta| delta.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(|content| content.get("text"))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                events.push(NormalizedStreamEvent::TextDelta(text.to_string()));
            }
        }
        "message-end" => {
            if let Some(reason) = value
                .get("delta")
                .and_then(|delta| delta.get("finish_reason"))
                .and_then(Value::as_str)
            {
                events.push(NormalizedStreamEvent::Finish(
                    cohere_finish_reason(Some(reason)).to_string(),
                ));
            }

            if let Some(usage) = value.get("delta" ).and_then(|delta| delta.get("usage")) {
                let tokens = usage.get("tokens");
                let billed_units = usage.get("billed_units");
                let prompt = usage
                    .get("input_tokens")
                    .and_then(Value::as_u64)
                    .or_else(|| tokens.and_then(|value| value.get("input_tokens")).and_then(Value::as_u64))
                    .or_else(|| {
                        billed_units
                            .and_then(|value| value.get("input_tokens"))
                            .and_then(Value::as_u64)
                    })
                    .unwrap_or(0);
                let completion = usage
                    .get("output_tokens")
                    .and_then(Value::as_u64)
                    .or_else(|| tokens.and_then(|value| value.get("output_tokens")).and_then(Value::as_u64))
                    .or_else(|| {
                        billed_units
                            .and_then(|value| value.get("output_tokens"))
                            .and_then(Value::as_u64)
                    })
                    .unwrap_or(0);
                events.push(NormalizedStreamEvent::Usage(usage_from_counts(
                    prompt,
                    completion,
                    Some(prompt + completion),
                )));
            }
        }
        "error" => {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| value.get("error").and_then(Value::as_str))
                .unwrap_or("Cohere 流式返回错误事件。")
                .to_string();
            return Err(message);
        }
        _ => {}
    }

    Ok(events)
}

fn parse_baidu_ernie_chunk(data: &str) -> Result<Vec<NormalizedStreamEvent>, String> {
    let value: Value = serde_json::from_str(data)
        .map_err(|err| format!("百度文心旧版流式响应解析失败：{err}"))?;
    if value.get("error_code").is_some() || value.get("error").is_some() {
        let bytes = serde_json::to_vec(&value)
            .map_err(|err| format!("百度文心旧版错误响应编码失败：{err}"))?;
        let (_, detail) = baidu_error_response(reqwest::StatusCode::OK, &bytes);
        return Err(detail.unwrap_or_else(|| "百度文心旧版流式请求失败。".into()));
    }

    let mut events = Vec::new();
    if let Some(text) = value
        .get("result")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        events.push(NormalizedStreamEvent::TextDelta(text.to_string()));
    }

    if let Some(usage) = value.get("usage") {
        let prompt = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total = usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .or(Some(prompt + completion));
        events.push(NormalizedStreamEvent::Usage(usage_from_counts(
            prompt, completion, total,
        )));
    }

    if value
        .get("is_end")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let reason = if let Some(reason) = value.get("finish_reason").and_then(Value::as_str) {
            reason.to_string()
        } else if value
            .get("is_truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            "length".into()
        } else {
            "stop".into()
        };
        events.push(NormalizedStreamEvent::Finish(reason));
    }

    Ok(events)
}

fn parse_xf_spark_chunk(data: &str) -> Result<Vec<NormalizedStreamEvent>, String> {
    let value: Value = serde_json::from_str(data)
        .map_err(|err| format!("讯飞星火流式响应解析失败：{err}"))?;

    if let Some(code) = value.get("code").and_then(Value::as_i64)
        && code != 0
    {
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("讯飞星火流式请求失败");
        return Err(format!("讯飞星火流式请求失败：code={code}, message={message}"));
    }

    let mut events = Vec::new();
    if let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    {
        if choice
            .get("delta")
            .and_then(|delta| delta.get("role"))
            .and_then(Value::as_str)
            == Some("assistant")
        {
            events.push(NormalizedStreamEvent::RoleStart);
        }
        if let Some(text) = choice
            .get("delta")
            .and_then(|delta| delta.get("content"))
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        {
            events.push(NormalizedStreamEvent::TextDelta(text.to_string()));
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            events.push(NormalizedStreamEvent::Finish(reason.to_string()));
        }
    }

    if let Some(usage) = value.get("usage") {
        let prompt = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total = usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .or(Some(prompt + completion));
        events.push(NormalizedStreamEvent::Usage(usage_from_counts(
            prompt, completion, total,
        )));
        events.push(NormalizedStreamEvent::Finish("stop".into()));
    }

    Ok(events)
}

fn parse_bedrock_event(value: &Value) -> Result<Vec<NormalizedStreamEvent>, String> {
    let mut events = Vec::new();

    if value.get("messageStart").is_some() {
        events.push(NormalizedStreamEvent::RoleStart);
    }

    if let Some(delta) = value
        .get("contentBlockDelta")
        .and_then(|event| event.get("delta"))
    {
        if let Some(text) = delta.get("text").and_then(Value::as_str).filter(|text| !text.is_empty())
        {
            events.push(NormalizedStreamEvent::TextDelta(text.to_string()));
        }
    }

    if let Some(message_stop) = value.get("messageStop") {
        if let Some(reason) = message_stop.get("stopReason").and_then(Value::as_str) {
            events.push(NormalizedStreamEvent::Finish(
                aws_claude_finish_reason(Some(reason)).to_string(),
            ));
        }
    }

    if let Some(metadata) = value.get("metadata") {
        if let Some(usage) = metadata.get("usage") {
            let prompt = usage
                .get("inputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let completion = usage
                .get("outputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let total = usage
                .get("totalTokens")
                .and_then(Value::as_u64)
                .or(Some(prompt + completion));
            events.push(NormalizedStreamEvent::Usage(usage_from_counts(
                prompt, completion, total,
            )));
        }

        if let Some(latency_ms) = metadata
            .get("metrics")
            .and_then(|metrics| metrics.get("latencyMs"))
            .and_then(Value::as_u64)
        {
            tracing::debug!(latency_ms, "bedrock converse stream metadata latency");
        }
    }

    if let Some((error_key, error_value)) = value.as_object().and_then(|object| {
        object.iter().find(|(key, _)| {
            key.ends_with("Exception") || key.eq_ignore_ascii_case("error")
        })
    }) {
        let message = error_value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(error_key)
            .to_string();
        return Err(format!("AWS Claude 流式返回错误事件：{message}"));
    }

    Ok(events)
}

#[allow(clippy::too_many_arguments)]
pub async fn stream_native_chat(
    app_state: Arc<AppState>,
    start_time: DateTime<Utc>,
    model_with_prefix: String,
    requested_model: String,
    effective_model: String,
    provider_type: ProviderType,
    base_url: String,
    provider_name: String,
    api_key: String,
    client_token: Option<String>,
    mut upstream_req: ChatCompletionRequest,
    provider_config: ProviderConfig,
) -> Result<Response, GatewayError> {
    let api_key_ref = Some(mask_key(&api_key));
    let usage_cell: Arc<Mutex<Option<Usage>>> = Arc::new(Mutex::new(None));
    let logged_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<axum::response::sse::Event>();
    let mut emitter = OpenAiSseEmitter::new(tx.clone(), effective_model.clone());

    upstream_req.stream = Some(true);
    upstream_req.stream_options = Some(ChatCompletionStreamOptions {
        include_usage: true,
    });

    let client = crate::http_client::client_for_url(&base_url)?;
    let response = match provider_type {
        ProviderType::AzureOpenAI => {
            let base = Url::parse(&base_url)
                .map_err(|err| GatewayError::Config(format!("Azure OpenAI base_url 无效：{err}")))?;
            let url = azure_openai_chat_completions_url(&base, &provider_config).map_err(
                |(_, detail)| {
                    GatewayError::Config(
                        detail.unwrap_or_else(|| "Azure OpenAI 配置不完整。".into()),
                    )
                },
            )?;
            let mut payload = serde_json::to_value(&upstream_req)?;
            if let Some(object) = payload.as_object_mut() {
                object.remove("model");
            }

            let response = client
                .post(&url)
                .header("api-key", api_key)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&payload)
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let bytes = response.bytes().await?;
                let (error_type, detail) = classify_azure_error(status, &bytes);
                return Err(gateway_error_from_normalized(
                    &error_type,
                    detail.unwrap_or_else(|| azure_error_message(status, &bytes)),
                ));
            }
            response
        }
        ProviderType::GoogleGemini => {
            let base = Url::parse(&base_url)
                .map_err(|err| GatewayError::Config(format!("Google Gemini base_url 无效：{err}")))?;
            let url = gemini_generate_content_url(
                &base,
                &provider_config,
                &upstream_req.model,
                true,
                &api_key,
            )
            .map_err(|(_, detail)| {
                GatewayError::Config(
                    detail.unwrap_or_else(|| "Google Gemini 配置不完整。".into()),
                )
            })?;
            let payload = build_gemini_payload(&upstream_req)?;
            let response = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&payload)
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let bytes = response.bytes().await?;
                let (error_type, detail) = classify_gemini_error(status, &bytes);
                return Err(gateway_error_from_normalized(
                    &error_type,
                    detail.unwrap_or_else(|| gemini_error_message(status, &bytes)),
                ));
            }
            response
        }
        ProviderType::VertexAI => {
            let base = Url::parse(&base_url)
                .map_err(|err| GatewayError::Config(format!("Vertex AI base_url 无效：{err}")))?;
            let url = vertex_stream_generate_content_url(&base, &provider_config, &upstream_req.model)
                .map_err(|(_, detail)| {
                    GatewayError::Config(
                        detail.unwrap_or_else(|| "Vertex AI 配置不完整。".into()),
                    )
                })?;
            let access_token = vertex_access_token(&provider_config).map_err(|(_, detail)| {
                GatewayError::Config(
                    detail.unwrap_or_else(|| "Vertex AI Access Token 配置无效。".into()),
                )
            })?;
            let payload = build_gemini_payload(&upstream_req)?;
            let response = client
                .post(&url)
                .header(
                    reqwest::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {access_token}"))
                        .map_err(|err| GatewayError::Config(format!("Vertex AI Token 无效：{err}")))?,
                )
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let bytes = response.bytes().await?;
                let (error_type, detail) = classify_vertex_error(status, &bytes);
                return Err(gateway_error_from_normalized(
                    &error_type,
                    detail.unwrap_or_else(|| vertex_error_message(status, &bytes)),
                ));
            }
            response
        }
        ProviderType::Cohere => {
            let url = format!("{}/v2/chat", base_url.trim_end_matches('/'));
            let mut payload = build_cohere_payload(&upstream_req)?;
            if let Some(object) = payload.as_object_mut() {
                object.insert("stream".into(), Value::Bool(true));
            }
            let response = client
                .post(&url)
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&payload)
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let bytes = response.bytes().await?;
                let (error_type, detail) = classify_cohere_error(status, &bytes);
                return Err(gateway_error_from_normalized(
                    &error_type,
                    detail.unwrap_or_else(|| cohere_error_message(status, &bytes)),
                ));
            }
            response
        }
        ProviderType::BaiduErnie => {
            let base = Url::parse(&base_url)
                .map_err(|err| GatewayError::Config(format!("百度文心旧版 base_url 无效：{err}")))?;
            let access_token = baidu_access_token(Some(&base), &provider_config)
                .await
                .map_err(|(_, detail)| {
                    GatewayError::Config(
                        detail.unwrap_or_else(|| "百度文心旧版鉴权配置无效。".into()),
                    )
                })?;
            let url = baidu_ernie_chat_url(&base, &upstream_req.model, &access_token).map_err(
                |(_, detail)| {
                    GatewayError::Config(
                        detail.unwrap_or_else(|| "百度文心旧版模型或路径配置无效。".into()),
                    )
                },
            )?;
            let payload = build_baidu_ernie_payload(&upstream_req, true)?;
            let response = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream, application/json")
                .json(&payload)
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let bytes = response.bytes().await?;
                let (error_type, detail) = baidu_error_response(status, &bytes);
                return Err(gateway_error_from_normalized(
                    &error_type,
                    detail.unwrap_or_else(|| "百度文心旧版流式请求失败。".into()),
                ));
            }
            response
        }
        ProviderType::XfSpark => {
            let url = join_openai_compat_endpoint(&base_url, "chat/completions");
            let mut payload = serde_json::to_value(&upstream_req)?;
            if let Some(object) = payload.as_object_mut() {
                object.remove("stream_options");
            }
            let response = client
                .post(&url)
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&payload)
                .send()
                .await?;
            if !response.status().is_success() {
                let bytes = response.bytes().await?;
                return Err(gateway_error_from_normalized(
                    "other",
                    String::from_utf8_lossy(&bytes).trim().to_string(),
                ));
            }
            response
        }
        ProviderType::AwsClaude => {
            let url = Url::parse(&format!(
                "{}/model/{}/converse-stream",
                base_url.trim_end_matches('/'),
                upstream_req.model
            ))
            .map_err(|err| GatewayError::Config(format!("AWS Claude 请求地址无效：{err}")))?;
            let payload = build_aws_claude_payload(&upstream_req)?;
            let payload_bytes = serde_json::to_vec(&payload)?;
            let headers = aws_sigv4_headers("POST", &url, &payload_bytes, &provider_config)
                .map_err(|(_, detail)| {
                    GatewayError::Config(
                        detail.unwrap_or_else(|| "AWS Claude SigV4 配置无效。".into()),
                    )
                })?;
            let mut request = client
                .post(url.clone())
                .header("Accept", "application/vnd.amazon.eventstream")
                .body(payload_bytes);
            for (name, value) in &headers {
                request = request.header(name, value);
            }
            let response = request.send().await?;
            if !response.status().is_success() {
                let status = response.status();
                let bytes = response.bytes().await?;
                let (error_type, detail) = classify_aws_claude_error(status, &bytes);
                return Err(gateway_error_from_normalized(
                    &error_type,
                    detail.unwrap_or_else(|| aws_claude_error_message(status, &bytes)),
                ));
            }
            response
        }
        _ => {
            return Err(GatewayError::Config(format!(
                "provider '{}' 未进入原生流式分发分支",
                provider_type.as_str()
            )));
        }
    };

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let usage_cell_for_task = usage_cell.clone();
    let app_state_clone = app_state.clone();
    let client_token_for_task = client_token.clone();
    let response_model = effective_model.clone();

    tokio::spawn(async move {
        let outcome: Result<(), String> = match provider_type {
            ProviderType::AzureOpenAI => {
                let mut decoder = SseMessageDecoder::default();
                let mut stream = response.bytes_stream();
                while let Some(item) = stream.next().await {
                    let chunk = item.map_err(|err| format!("Azure OpenAI 流式读取失败：{err}"))?;
                    let text = String::from_utf8_lossy(&chunk).to_string();
                    for message in decoder.push(&text) {
                        let data = message.data.trim();
                        if data == "[DONE]" {
                            if !emitter.emit_done() {
                                return Ok(());
                            }
                            return Ok(());
                        }
                        let events = parse_azure_chunk(data)?;
                        if !handle_normalized_events(&mut emitter, &usage_cell_for_task, events)? {
                            return Ok(());
                        }
                    }
                }
                let _ = emitter.emit_done();
                Ok(())
            }
            ProviderType::GoogleGemini | ProviderType::VertexAI => {
                let is_sse = content_type.contains("text/event-stream");
                let provider_label = if provider_type == ProviderType::GoogleGemini {
                    "Google Gemini"
                } else {
                    "Vertex AI"
                };
                let mut sse_decoder = SseMessageDecoder::default();
                let mut json_decoder = JsonObjectStreamDecoder::default();
                let mut stream = response.bytes_stream();
                while let Some(item) = stream.next().await {
                    let chunk = item.map_err(|err| format!("{provider_label} 流式读取失败：{err}"))?;
                    let text = String::from_utf8_lossy(&chunk).to_string();

                    if is_sse {
                        for message in sse_decoder.push(&text) {
                            let data = message.data.trim();
                            if data == "[DONE]" {
                                if !emitter.emit_done() {
                                    return Ok(());
                                }
                                return Ok(());
                            }
                            let events = parse_generate_content_response(data, provider_label)?;
                            if !handle_normalized_events(&mut emitter, &usage_cell_for_task, events)? {
                                return Ok(());
                            }
                        }
                    } else {
                        for fragment in json_decoder.push(&text) {
                            let events = parse_generate_content_response(&fragment, provider_label)?;
                            if !handle_normalized_events(&mut emitter, &usage_cell_for_task, events)? {
                                return Ok(());
                            }
                        }
                    }
                }
                let _ = emitter.emit_done();
                Ok(())
            }
            ProviderType::Cohere => {
                let mut decoder = SseMessageDecoder::default();
                let mut stream = response.bytes_stream();
                while let Some(item) = stream.next().await {
                    let chunk = item.map_err(|err| format!("Cohere 流式读取失败：{err}"))?;
                    let text = String::from_utf8_lossy(&chunk).to_string();
                    for message in decoder.push(&text) {
                        let events = parse_cohere_event(&message.event, &message.data)?;
                        if !handle_normalized_events(&mut emitter, &usage_cell_for_task, events)? {
                            return Ok(());
                        }
                    }
                }
                let _ = emitter.emit_done();
                Ok(())
            }
            ProviderType::BaiduErnie => {
                let is_sse = content_type.contains("text/event-stream");
                let mut sse_decoder = SseMessageDecoder::default();
                let mut json_decoder = JsonObjectStreamDecoder::default();
                let mut stream = response.bytes_stream();
                while let Some(item) = stream.next().await {
                    let chunk = item.map_err(|err| format!("百度文心旧版流式读取失败：{err}"))?;
                    if baidu_requires_error(&chunk) {
                        let (error_type, detail) = baidu_error_response(reqwest::StatusCode::OK, &chunk);
                        return Err(detail.unwrap_or_else(|| error_type));
                    }
                    let text = String::from_utf8_lossy(&chunk).to_string();
                    if is_sse || text.contains("data:") {
                        for message in sse_decoder.push(&text) {
                            let data = message.data.trim();
                            if data == "[DONE]" {
                                let _ = emitter.emit(NormalizedStreamEvent::Finish("stop".into()));
                                let _ = emitter.emit_done();
                                return Ok(());
                            }
                            let events = parse_baidu_ernie_chunk(data)?;
                            if !handle_normalized_events(&mut emitter, &usage_cell_for_task, events)? {
                                return Ok(());
                            }
                        }
                    } else {
                        for fragment in json_decoder.push(&text) {
                            let events = parse_baidu_ernie_chunk(&fragment)?;
                            if !handle_normalized_events(&mut emitter, &usage_cell_for_task, events)? {
                                return Ok(());
                            }
                        }
                    }
                }
                let _ = emitter.emit(NormalizedStreamEvent::Finish("stop".into()));
                let _ = emitter.emit_done();
                Ok(())
            }
            ProviderType::XfSpark => {
                let mut decoder = SseMessageDecoder::default();
                let mut stream = response.bytes_stream();
                while let Some(item) = stream.next().await {
                    let chunk = item.map_err(|err| format!("讯飞星火流式读取失败：{err}"))?;
                    let text = String::from_utf8_lossy(&chunk).to_string();
                    for message in decoder.push(&text) {
                        let data = message.data.trim();
                        if data == "[DONE]" {
                            let _ = emitter.emit(NormalizedStreamEvent::Finish("stop".into()));
                            let _ = emitter.emit_done();
                            return Ok(());
                        }
                        let events = parse_xf_spark_chunk(data)?;
                        if !handle_normalized_events(&mut emitter, &usage_cell_for_task, events)? {
                            return Ok(());
                        }
                    }
                }
                let _ = emitter.emit(NormalizedStreamEvent::Finish("stop".into()));
                let _ = emitter.emit_done();
                Ok(())
            }
            ProviderType::AwsClaude => {
                let mut decoder = AwsEventStreamDecoder::default();
                let mut stream = response.bytes_stream();
                while let Some(item) = stream.next().await {
                    let chunk = item.map_err(|err| format!("AWS Claude 流式读取失败：{err}"))?;
                    for frame in decoder.push(&chunk)? {
                        if frame.is_empty() {
                            continue;
                        }
                        let value: Value = serde_json::from_slice(&frame).map_err(|err| {
                            format!("AWS Claude EventStream 载荷解析失败：{err}")
                        })?;
                        let events = parse_bedrock_event(&value)?;
                        if !handle_normalized_events(&mut emitter, &usage_cell_for_task, events)? {
                            return Ok(());
                        }
                    }
                }
                let _ = emitter.emit_done();
                Ok(())
            }
            _ => Ok(()),
        };

        match &outcome {
            Ok(()) => {
                if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    let usage_snapshot = usage_cell_for_task.lock().unwrap().clone();
                    super::common::log_stream_success(
                        app_state_clone,
                        start_time,
                        model_with_prefix,
                        requested_model,
                        response_model,
                        provider_name,
                        api_key_ref,
                        client_token_for_task,
                        usage_snapshot,
                    )
                    .await;
                }
            }
            Err(message) => {
                let _ = emitter.emit_error(message.clone());
                if !logged_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    super::common::log_stream_error(
                        app_state_clone,
                        start_time,
                        model_with_prefix,
                        requested_model,
                        effective_model,
                        provider_name,
                        api_key_ref,
                        client_token_for_task,
                        message.clone(),
                    )
                    .await;
                }
            }
        };

        outcome
    });

    let out_stream = tokio_stream::StreamExt::map(
        tokio_stream::wrappers::UnboundedReceiverStream::new(rx),
        Ok::<_, Infallible>,
    );
    Ok(Sse::new(out_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}
