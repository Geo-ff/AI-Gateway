use async_openai::types as oai;

use crate::providers::openai::ChatCompletionRequest;

pub fn image_source_from_url(url: &str) -> (String, String, String) {
    if url.starts_with("http://") || url.starts_with("https://") {
        ("url".to_string(), String::new(), url.to_string())
    } else if let Some(rest) = url.strip_prefix("data:") {
        // format: data:<mime>;base64,<data>
        let mut parts = rest.splitn(2, ',');
        let meta = parts.next().unwrap_or("");
        let data = parts.next().unwrap_or("");
        let mime = meta.split(';').next().unwrap_or("application/octet-stream");
        ("base64".to_string(), mime.to_string(), data.to_string())
    } else {
        ("url".to_string(), String::new(), url.to_string())
    }
}

pub fn extract_system_prompt(openai_req: &ChatCompletionRequest) -> Option<String> {
    for msg in &openai_req.messages {
        match msg {
            oai::ChatCompletionRequestMessage::Developer(dev) => {
                return match &dev.content {
                    oai::ChatCompletionRequestDeveloperMessageContent::Text(s) => Some(s.clone()),
                    oai::ChatCompletionRequestDeveloperMessageContent::Array(parts) => Some(parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n")),
                }
            }
            oai::ChatCompletionRequestMessage::System(sys) => {
                return match &sys.content {
                    oai::ChatCompletionRequestSystemMessageContent::Text(s) => Some(s.clone()),
                    oai::ChatCompletionRequestSystemMessageContent::Array(parts) => Some(parts.iter().map(|p| match p { oai::ChatCompletionRequestSystemMessageContentPart::Text(t) => t.text.as_str() }).collect::<Vec<_>>().join("\n")),
                }
            }
            _ => {}
        }
    }
    None
}

