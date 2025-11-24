use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// 创建登录凭证的请求参数
#[derive(Debug, Serialize)]
struct CreateCodePayload {
    ttl_secs: u64,
    max_uses: u32,
    length: usize,
    magic_url: bool,
}

/// 创建登录凭证的响应数据
#[derive(Debug, Deserialize)]
pub struct CreateCodeResp {
    pub code: String,
    pub expires_at: String,
    #[serde(default)]
    pub max_uses: u32,
    #[serde(default)]
    pub uses: u32,
    #[serde(default)]
    pub remaining_uses: u32,
    #[serde(default)]
    pub login_url: Option<String>,
}

/// 请求挑战的参数
#[derive(Debug, Serialize)]
struct ChallengePayload<'a> {
    fingerprint: &'a str,
}

/// 挑战请求的响应数据
#[derive(Debug, Deserialize)]
pub struct ChallengeResp {
    pub challenge_id: String,
    pub nonce: String,
    #[serde(rename = "expires_at")]
    pub _expires_at: String,
    #[serde(rename = "alg")]
    pub _alg: String,
}

/// 验证挑战的请求参数
#[derive(Debug, Serialize)]
struct VerifyPayload<'a> {
    challenge_id: &'a str,
    fingerprint: &'a str,
    signature: &'a str,
}

/// 验证挑战的响应数据
#[derive(Debug, Deserialize)]
pub struct VerifyResp {
    pub token: String,
    pub expires_at: String,
    pub fingerprint: String,
}

/// 登录凭证状态信息
#[derive(Debug, Deserialize)]
pub struct CodeStatusInfo {
    #[serde(rename = "created_at")]
    pub _created_at: String,
    #[serde(rename = "expires_at")]
    pub _expires_at: String,
    #[serde(rename = "max_uses")]
    pub _max_uses: u32,
    pub uses: u32,
    pub remaining_uses: u32,
    pub disabled: bool,
}

/// 登录凭证状态查询响应
#[derive(Debug, Deserialize)]
pub struct CodeStatusResp {
    #[serde(rename = "exists")]
    pub _exists: bool,
    #[serde(default)]
    pub info: Option<CodeStatusInfo>,
}

/// 管理员密钥信息
#[derive(Debug, Deserialize)]
pub struct AdminKeyOut {
    pub fingerprint: String,
    pub comment: Option<String>,
    #[serde(rename = "enabled")]
    pub _enabled: bool,
    pub created_at: String,
    #[serde(rename = "last_used_at")]
    pub _last_used_at: Option<String>,
}

/// 管理员令牌信息
#[derive(Debug, Deserialize, Clone)]
pub struct AdminTokenOut {
    pub token: String,
    #[serde(default, rename = "allowed_models")]
    pub _allowed_models: Option<Vec<String>>,
    #[serde(default, rename = "max_tokens")]
    pub _max_tokens: Option<i64>,
    #[serde(default)]
    pub max_amount: Option<f64>,
    pub amount_spent: f64,
    pub prompt_tokens_spent: i64,
    pub completion_tokens_spent: i64,
    pub total_tokens_spent: i64,
    pub enabled: bool,
    #[serde(default)]
    pub expires_at: Option<String>,
    pub created_at: String,
}

/// 添加管理员密钥的请求参数
#[derive(Debug, Serialize)]
struct AddKeyPayload<'a> {
    public_key_b64: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")] comment: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")] enabled: Option<bool>,
}


/// API错误响应的主体结构
#[derive(Debug, Deserialize)]
struct ErrorBody {
    message: Option<String>,
}

/// 创建 HTTP 客户端
///
/// 使用默认配置创建 reqwest HTTP 客户端
///
/// # Returns
///
/// * `Ok(Client)` - 成功创建的 HTTP 客户端
/// * `Err(String)` - 创建失败的错误信息
fn client() -> Result<Client, String> {
    Client::builder().build().map_err(|e| e.to_string())
}

/// 构建完整的 API URL
///
/// 将配置中的基础 URL 和路径组合成完整的 API 地址
///
/// # Arguments
///
/// * `cfg` - 包含基础 URL 的配置
/// * `path` - API 路径
///
/// # Returns
///
/// 完整的 API URL
fn api_url(cfg: &Config, path: &str) -> String {
    format!(
        "{}/{}",
        cfg.api_base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// 提取和格式化 API 错误信息
///
/// 从 HTTP 响应中提取错误信息，优先尝试解析 JSON 格式的错误消息
///
/// # Arguments
///
/// * `status` - HTTP 状态码
/// * `text` - 响应主体内容
///
/// # Returns
///
/// 格式化后的错误信息
fn extract_error(status: StatusCode, text: String) -> String {
    if let Ok(body) = serde_json::from_str::<ErrorBody>(&text) {
        if let Some(msg) = body.message {
            return msg;
        }
    }
    format!("HTTP {}: {}", status, text)
}

/// 请求身份验证挑战
///
/// 发起挑战-应答身份验证的第一步，获取服务器端生成的挑战信息
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
/// * `fingerprint` - 管理员公钥指纹
///
/// # Returns
///
/// * `Ok(ChallengeResp)` - 成功获取的挑战信息
/// * `Err(String)` - 请求失败的错误信息
pub fn request_challenge(cfg: &Config, fingerprint: &str) -> Result<ChallengeResp, String> {
    let client = client()?;
    let url = api_url(cfg, "/auth/tui/challenge");
    let resp = client
        .post(url)
        .json(&ChallengePayload { fingerprint })
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(extract_error(status, text));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {}; body={}", e, text))
}

/// 验证身份验证挑战
///
/// 发起挑战-应答身份验证的第二步，提交签名后的响应进行验证
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
/// * `challenge_id` - 挑战 ID
/// * `fingerprint` - 管理员公钥指纹
/// * `signature` - Base64 编码的数字签名
///
/// # Returns
///
/// * `Ok(VerifyResp)` - 成功验证后的会话信息
/// * `Err(String)` - 验证失败的错误信息
pub fn verify_challenge(
    cfg: &Config,
    challenge_id: &str,
    fingerprint: &str,
    signature: &str,
) -> Result<VerifyResp, String> {
    let client = client()?;
    let url = api_url(cfg, "/auth/tui/verify");
    let resp = client
        .post(url)
        .json(&VerifyPayload {
            challenge_id,
            fingerprint,
            signature,
        })
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(extract_error(status, text));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {}; body={}", e, text))
}

/// 添加管理员公钥
///
/// 向服务器添加一个新的管理员公钥，用于后续的身份验证
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
/// * `token` - 管理员会话令牌
/// * `public_key_b64` - Base64 编码的公钥
/// * `comment` - 可选的密钥备注
///
/// # Returns
///
/// * `Ok(AdminKeyOut)` - 成功添加的密钥信息
/// * `Err(String)` - 添加失败的错误信息
pub fn add_admin_key(cfg: &Config, token: &str, public_key_b64: &str, comment: Option<&str>) -> Result<AdminKeyOut, String> {
    let client = client()?;
    let url = api_url(cfg, "/auth/keys");
    let payload = AddKeyPayload { public_key_b64, comment, enabled: Some(true) };
    let resp = client.post(url).bearer_auth(token).json(&payload).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() { return Err(extract_error(status, text)); }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {}; body={}", e, text))
}

/// 删除管理员公钥
///
/// 根据公钥指纹删除管理员公钥
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
/// * `token` - 管理员会话令牌
/// * `fingerprint` - 要删除的公钥指纹
///
/// # Returns
///
/// * `Ok(true)` - 删除成功
/// * `Ok(false)` - 删除失败
/// * `Err(String)` - 网络或其他错误
pub fn delete_admin_key(cfg: &Config, token: &str, fingerprint: &str) -> Result<bool, String> {
    let client = client()?;
    let url = api_url(cfg, &format!("/auth/keys/{}", fingerprint));
    let resp = client.delete(url).bearer_auth(token).send().map_err(|e| e.to_string())?;
    Ok(resp.status().is_success())
}

/// 获取管理员公钥列表
///
/// 查询所有管理员公钥的信息
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
/// * `token` - 管理员会话令牌
///
/// # Returns
///
/// * `Ok(Vec<AdminKeyOut>)` - 管理员公钥列表
/// * `Err(String)` - 查询失败的错误信息
pub fn list_admin_keys(cfg: &Config, token: &str) -> Result<Vec<AdminKeyOut>, String> {
    let client = client()?;
    let url = api_url(cfg, "/auth/keys");
    let resp = client.get(url).bearer_auth(token).send().map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() { return Err(extract_error(status, text)); }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {}; body={}", e, text))
}

/// 获取管理员令牌列表
///
/// 查询所有管理员令牌的信息和使用统计
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
/// * `token` - 管理员会话令牌
///
/// # Returns
///
/// * `Ok(Vec<AdminTokenOut>)` - 管理员令牌列表
/// * `Err(String)` - 查询失败的错误信息
pub fn list_tokens(cfg: &Config, token: &str) -> Result<Vec<AdminTokenOut>, String> {
    let client = client()?;
    let url = api_url(cfg, "/admin/tokens");
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(extract_error(status, text));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {}; body={}", e, text))
}


/// 创建登录凭证
///
/// 创建一个新的登录凭证，可以是普通的 Code 或 Magic URL
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
/// * `token` - 管理员会话令牌
/// * `ttl` - 凭证有效期（秒）
/// * `max_uses` - 最大使用次数
/// * `length` - 凭证字符串长度
/// * `magic` - 是否生成 Magic URL
///
/// # Returns
///
/// * `Ok(CreateCodeResp)` - 成功创建的凭证信息
/// * `Err(String)` - 创建失败的错误信息
pub fn create_code(
    cfg: &Config,
    token: &str,
    ttl: u64,
    max_uses: u32,
    length: usize,
    magic: bool,
) -> Result<CreateCodeResp, String> {
    let client = client()?;
    let url = api_url(cfg, "/auth/login-codes");
    let payload = CreateCodePayload {
        ttl_secs: ttl,
        max_uses,
        length,
        magic_url: magic,
    };
    let resp = client
        .post(url)
        .bearer_auth(token)
        .json(&payload)
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(extract_error(status, text));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {}; body={}", e, text))
}

/// 查询登录凭证状态
///
/// 获取当前管理员会话的登录凭证状态信息
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
/// * `token` - 管理员会话令牌
///
/// # Returns
///
/// * `Ok(CodeStatusResp)` - 凭证状态信息
/// * `Err(String)` - 查询失败的错误信息
pub fn fetch_code_status(cfg: &Config, token: &str) -> Result<CodeStatusResp, String> {
    let client = client()?;
    let url = api_url(cfg, "/auth/login-codes/status");
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(extract_error(status, text));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {}; body={}", e, text))
}

/// 测试网络连接
///
/// 通过发起一个简单的 HTTP 请求来检查网络连接状态
///
/// # Arguments
///
/// * `cfg` - 包含 API 地址的配置
///
/// # Returns
///
/// * `Ok(())` - 连接正常
/// * `Err(String)` - 连接失败的错误信息
pub fn connectivity_check(cfg: &Config) -> Result<(), String> {
    let url = api_url(cfg, "/auth/session");
    let resp = client()?.get(url).send().map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}
