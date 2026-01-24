use serde::Deserialize;

use crate::providers::openai::ChatCompletionRequest;

/// Gateway chat completion request envelope.
///
/// Notes:
/// - The public `/v1/chat/completions` endpoint is OpenAI-compatible, but the gateway supports a
///   few extra fields (e.g. `top_k`) that don't belong to the upstream OpenAI schema.
/// - We keep this shared between non-stream and stream paths so clients can send one shape.
#[derive(Debug, Clone, Deserialize)]
pub struct GatewayChatCompletionRequest {
    #[serde(flatten)]
    pub request: ChatCompletionRequest,
    /// Top-k sampling parameter (best-effort; currently only Anthropic path uses it).
    pub top_k: Option<u32>,
}
