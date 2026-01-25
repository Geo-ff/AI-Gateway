use crate::error::GatewayError;

pub const ALLOWED_MODEL_TYPES: [&str; 6] =
    ["chat", "completion", "embedding", "image", "audio", "video"];

fn is_allowed_model_type(v: &str) -> bool {
    ALLOWED_MODEL_TYPES.contains(&v)
}

pub fn parse_model_types(raw: Option<&str>) -> Option<Vec<String>> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;

    if raw.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Vec<String>>(raw) {
            let mut out = Vec::new();
            for item in v {
                let t = item.trim();
                if t.is_empty() {
                    continue;
                }
                out.push(t.to_string());
            }
            if out.is_empty() {
                return None;
            }
            return Some(dedup_preserve_order(out));
        }
    }

    if raw.contains(',') {
        let mut out = Vec::new();
        for part in raw.split(',') {
            let t = part.trim();
            if t.is_empty() {
                continue;
            }
            out.push(t.to_string());
        }
        if out.is_empty() {
            return None;
        }
        return Some(dedup_preserve_order(out));
    }

    Some(vec![raw.to_string()])
}

pub fn normalize_model_types(
    model_type: Option<&str>,
    model_types: Option<&[String]>,
) -> Result<Option<Vec<String>>, GatewayError> {
    let mut out: Vec<String> = Vec::new();

    if let Some(list) = model_types {
        for item in list {
            let t = item.trim();
            if t.is_empty() {
                continue;
            }
            out.push(t.to_string());
        }
    } else if let Some(v) = model_type {
        let v = v.trim();
        if !v.is_empty() {
            out.push(v.to_string());
        }
    }

    out = dedup_preserve_order(out);
    for t in &out {
        if !is_allowed_model_type(t.as_str()) {
            return Err(GatewayError::Config("invalid model_type".into()));
        }
    }

    if out.is_empty() {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

pub fn model_types_to_storage(types: Option<&[String]>) -> Option<String> {
    let types = types?;
    if types.is_empty() {
        return None;
    }
    if types.len() == 1 {
        return Some(types[0].clone());
    }
    serde_json::to_string(types).ok()
}

pub fn model_types_for_response(raw: Option<&str>) -> (Option<String>, Option<Vec<String>>) {
    let types = parse_model_types(raw);
    let first = types.as_ref().and_then(|v| v.first().cloned());
    (first, types)
}

fn dedup_preserve_order(items: Vec<String>) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::new();
    for v in items {
        if seen.insert(v.clone()) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_type() {
        assert_eq!(parse_model_types(Some("chat")).unwrap(), vec!["chat"]);
    }

    #[test]
    fn parse_csv_types() {
        assert_eq!(
            parse_model_types(Some("chat, image ,chat")).unwrap(),
            vec!["chat", "image"]
        );
    }

    #[test]
    fn parse_json_array_types() {
        assert_eq!(
            parse_model_types(Some("[\"chat\",\"image\",\"chat\"]")).unwrap(),
            vec!["chat", "image"]
        );
    }

    #[test]
    fn normalize_accepts_list_and_validates() {
        let v = vec!["chat".to_string(), "image".to_string(), "chat".to_string()];
        assert_eq!(
            normalize_model_types(None, Some(&v)).unwrap(),
            Some(vec!["chat".to_string(), "image".to_string()])
        );
    }

    #[test]
    fn model_types_to_storage_single_is_plain() {
        let v = vec!["chat".to_string()];
        assert_eq!(model_types_to_storage(Some(&v)), Some("chat".to_string()));
    }

    #[test]
    fn model_types_to_storage_multi_is_json() {
        let v = vec!["chat".to_string(), "image".to_string()];
        assert_eq!(
            model_types_to_storage(Some(&v)),
            Some("[\"chat\",\"image\"]".to_string())
        );
    }
}
