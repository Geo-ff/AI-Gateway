use crate::config::settings::KeyLogStrategy;
use crate::error::{GatewayError, Result as AppResult};

// 轻量可逆混淆：按 provider+固定盐 作为 key 做异或，再十六进制编码
// 注意：非强加密，仅为“masked”场景提供基础保护
fn xor_bytes(data: &[u8], key: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[allow(clippy::manual_is_multiple_of)]
fn from_hex(s: &str) -> AppResult<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(GatewayError::Config("Invalid hex length".into()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for i in (0..s.len()).step_by(2) {
        let hi = (bytes[i] as char)
            .to_digit(16)
            .ok_or_else(|| GatewayError::Config("Invalid hex".into()))?;
        let lo = (bytes[i + 1] as char)
            .to_digit(16)
            .ok_or_else(|| GatewayError::Config("Invalid hex".into()))?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

fn key_material(provider: &str) -> Vec<u8> {
    let mut v = Vec::from(provider.as_bytes());
    v.extend_from_slice(b"::ai-gateway");
    v
}

pub fn protect(strategy: &Option<KeyLogStrategy>, provider: &str, plain: &str) -> (String, bool) {
    match strategy.clone().unwrap_or(KeyLogStrategy::Masked) {
        KeyLogStrategy::Plain => (plain.to_string(), false),
        KeyLogStrategy::None | KeyLogStrategy::Masked => {
            let km = key_material(provider);
            let xored = xor_bytes(plain.as_bytes(), &km);
            (to_hex(&xored), true)
        }
    }
}

pub fn unprotect(
    strategy: &Option<KeyLogStrategy>,
    provider: &str,
    data: &str,
    encrypted: bool,
) -> AppResult<String> {
    if !encrypted {
        return Ok(data.to_string());
    }
    match strategy.clone().unwrap_or(KeyLogStrategy::Masked) {
        KeyLogStrategy::Plain => Ok(data.to_string()),
        KeyLogStrategy::None | KeyLogStrategy::Masked => {
            let bytes = from_hex(data)?;
            let km = key_material(provider);
            let plain = xor_bytes(&bytes, &km);
            Ok(String::from_utf8(plain)
                .map_err(|e| GatewayError::Config(format!("Invalid UTF-8 after decrypt: {}", e)))?)
        }
    }
}
