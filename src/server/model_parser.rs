/// 解析模型名称，提取供应商前缀和实际模型名称
#[derive(Debug, Clone)]
pub struct ParsedModel {
    pub provider_name: Option<String>,
    pub model_name: String,
}

impl ParsedModel {
    /// 从完整的模型名称中解析出供应商前缀和实际模型名称
    ///
    /// 示例：
    /// - "openai/Qwen3-Coder-Instruct-MD" -> ParsedModel { provider_name: Some("openai"), model_name: "Qwen3-Coder-Instruct-MD" }
    /// - "Qwen3-Coder-Instruct-MD" -> ParsedModel { provider_name: None, model_name: "Qwen3-Coder-Instruct-MD" }
    pub fn parse(model: &str) -> Self {
        if let Some(slash_pos) = model.find('/') {
            let provider_name = model[..slash_pos].to_string();
            let model_name = model[slash_pos + 1..].to_string();
            Self {
                provider_name: Some(provider_name),
                model_name,
            }
        } else {
            Self {
                provider_name: None,
                model_name: model.to_string(),
            }
        }
    }

    /// 获取实际应该传递给上游 API 的模型名称
    pub fn get_upstream_model_name(&self) -> &str {
        &self.model_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_with_prefix() {
        let parsed = ParsedModel::parse("openai/Qwen3-Coder-Instruct-MD");
        assert_eq!(parsed.provider_name, Some("openai".to_string()));
        assert_eq!(parsed.model_name, "Qwen3-Coder-Instruct-MD");
        assert_eq!(parsed.get_upstream_model_name(), "Qwen3-Coder-Instruct-MD");
    }

    #[test]
    fn test_parse_without_prefix() {
        let parsed = ParsedModel::parse("Qwen3-Coder-Instruct-MD");
        assert_eq!(parsed.provider_name, None);
        assert_eq!(parsed.model_name, "Qwen3-Coder-Instruct-MD");
        assert_eq!(parsed.get_upstream_model_name(), "Qwen3-Coder-Instruct-MD");
    }
}
