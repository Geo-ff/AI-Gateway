use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

/// 应用程序配置结构
///
/// 包含网关API基础URL、管理员私钥以及登录凭证的默认参数
/// 支持序列化和反序列化用于持久化存储
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_base_url: String,
    #[serde(default)]
    pub private_key_b64: Option<String>,
    #[serde(default = "default_ttl")]
    pub ttl_secs: u64,
    #[serde(default = "default_max_uses")]
    pub max_uses: u32,
    #[serde(default = "default_length")]
    pub length: usize,
}

/// 为配置结构提供默认值
impl Default for Config {
    fn default() -> Self {
        Self {
            api_base_url: "http://127.0.0.1:8080".into(),
            private_key_b64: None,
            ttl_secs: default_ttl(),
            max_uses: default_max_uses(),
            length: default_length(),
        }
    }
}

/// 返回默认的TTL秒数（60秒）
///
/// # Returns
///
/// 默认的登录凭证过期时间（60秒）
pub fn default_ttl() -> u64 {
    60
}
/// 返回默认的最大使用次数（1次）
///
/// # Returns
///
/// 默认的登录凭证最大使用次数
pub fn default_max_uses() -> u32 {
    1
}
/// 返回默认的登录凭证长度（32位）
///
/// # Returns
///
/// 默认的登录凭证字符串长度
pub fn default_length() -> usize {
    32
}

/// 获取配置文件路径
///
/// 按以下优先级查找配置文件：
/// 1. `tui/config.toml` (如果存在)
/// 2. `config.toml` (如果存在)
/// 3. 如果当前目录名为"tui"，使用 `./config.toml`
/// 4. 默认使用 `tui/config.toml`
///
/// # Returns
///
/// 配置文件的完整路径
fn config_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let tui_cfg = cwd.join("tui").join("config.toml");
    let root_cfg = cwd.join("config.toml");
    if tui_cfg.exists() {
        return tui_cfg;
    }
    if root_cfg.exists() {
        return root_cfg;
    }
    match cwd.file_name().and_then(|s| s.to_str()) {
        Some("tui") => root_cfg,
        _ => {
            if let Some(p) = tui_cfg.parent() {
                let _ = fs::create_dir_all(p);
            }
            tui_cfg
        }
    }
}

/// 从文件加载配置
///
/// 读取并解析TOML格式的配置文件，加载后会自动标准化配置参数
///
/// # Returns
///
/// * `Some(Config)` - 成功加载的配置
/// * `None` - 文件不存在或解析失败
pub fn load_config() -> Option<Config> {
    let p = config_path();
    let s = fs::read_to_string(p).ok()?;
    toml::from_str(&s).ok().map(normalize_config)
}

/// 保存配置到文件
///
/// 将配置序列化为TOML格式并写入文件，保存前会自动标准化配置参数
///
/// # Arguments
///
/// * `cfg` - 要保存的配置数据
///
/// # Returns
///
/// * `Ok(())` - 保存成功
/// * `Err(String)` - 保存失败，包含错误信息
pub fn save_config(cfg: &Config) -> Result<(), String> {
    let cfg = normalize_config(cfg.clone());
    let s = toml::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    let path = config_path();
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    fs::write(path, s).map_err(|e| e.to_string())
}

/// 从标准输入读取用户输入
///
/// 显示提示信息并等待用户输入，返回去除首尾空白的字符串
///
/// # Arguments
///
/// * `s` - 要显示的提示信息
///
/// # Returns
///
/// 用户输入的字符串（去除首尾空白）
fn prompt(s: &str) -> io::Result<String> {
    print!("{}", s);
    let _ = io::stdout().flush();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// 交互式配置初始化
///
/// 检查是否存在有效的配置文件，如果不存在或配置不完整，
/// 则启动交互式配置流程，引导用户配置必要的参数
///
/// # Returns
///
/// 完整的配置对象，包含API地址和管理员私钥
pub fn ensure_config_interactive() -> Config {
    if let Some(cfg) = load_config() {
        if !cfg.api_base_url.trim().is_empty() && cfg.private_key_b64.is_some() {
            return cfg;
        }
    }

    println!("首次运行：请配置网关地址与管理员私钥（可稍后在 tui/config.toml 修改）");
    let mut cfg = load_config().unwrap_or_default();
    let default_url = if cfg.api_base_url.trim().is_empty() {
        "http://127.0.0.1:8080".to_string()
    } else {
        cfg.api_base_url.clone()
    };
    let url_input =
        prompt(&format!("Gateway API Base URL [{}]: ", default_url)).unwrap_or_default();
    cfg.api_base_url = if url_input.trim().is_empty() {
        default_url
    } else {
        url_input.trim().to_string()
    };

    loop {
        let pk_prompt = if cfg.private_key_b64.is_some() {
            "管理员私钥（base64，留空沿用现有）: "
        } else {
            "管理员私钥（base64）: "
        };
        let input = prompt(pk_prompt).unwrap_or_default();
        if input.trim().is_empty() {
            if cfg.private_key_b64.is_some() {
                break;
            }
            println!("管理员私钥不能为空");
            continue;
        }
        match sanitize_private_key(&input) {
            Ok(clean) => match decode_private_key_bytes(&clean) {
                Ok(bytes) => {
                    let signing = SigningKey::from_bytes(&bytes);
                    let fp = compute_fingerprint(&signing.verifying_key().to_bytes());
                    println!("检测到管理员指纹：{}", fp);
                    let confirm = prompt("确认保存该私钥？(Y/n): ").unwrap_or_default();
                    let yes = confirm.trim().is_empty()
                        || matches!(confirm.trim().to_ascii_lowercase().as_str(), "y" | "yes");
                    if yes {
                        cfg.private_key_b64 = Some(clean);
                        break;
                    } else {
                        println!("已取消，请重新输入管理员私钥。");
                    }
                }
                Err(e) => {
                    println!("私钥格式无效：{}", e);
                }
            },
            Err(e) => {
                println!("私钥格式无效：{}", e);
            }
        }
    }

    if let Err(e) = save_config(&cfg) {
        println!("保存配置失败：{}", e);
    } else {
        if let Ok(fp) = fingerprint(&cfg) {
            println!("配置已保存。管理员指纹：{}", fp);
        } else {
            println!("配置已保存。");
        }
        let _ = prompt("按 Enter 键继续...");
    }
    cfg
}

/// 标准化配置参数
///
/// 对配置参数进行验证和规范化，确保参数在合理范围内
/// 包括：
/// - 移除URL末尾的斜杠
/// - 限制长度在 25-64 之间
/// - 限制TTL在 1-86400 秒之间
/// - 限制使用次数在 1-1000 之间
/// - 清理私钥中的空白字符
///
/// # Arguments
///
/// * `cfg` - 要标准化的配置
///
/// # Returns
///
/// 标准化后的配置
fn normalize_config(mut cfg: Config) -> Config {
    cfg.api_base_url = cfg.api_base_url.trim().trim_end_matches('/').to_string();
    if cfg.length < 25 {
        cfg.length = 25;
    }
    if cfg.length > 64 {
        cfg.length = 64;
    }
    if cfg.ttl_secs < 1 {
        cfg.ttl_secs = 1;
    }
    if cfg.ttl_secs > 86400 {
        cfg.ttl_secs = 86400;
    }
    if cfg.max_uses < 1 {
        cfg.max_uses = 1;
    }
    if cfg.max_uses > 1000 {
        cfg.max_uses = 1000;
    }
    if let Some(pk) = cfg.private_key_b64.clone() {
        cfg.private_key_b64 = Some(pk.chars().filter(|c| !c.is_whitespace()).collect());
    }
    cfg
}

/// 清理和验证私钥格式
///
/// 移除私钥字符串中的所有空白字符，并验证其是否为有效的Base64编码的Ed25519私钥
///
/// # Arguments
///
/// * `input` - 原始私钥字符串
///
/// # Returns
///
/// * `Ok(String)` - 清理后的有效私钥
/// * `Err(String)` - 验证失败的错误信息
fn sanitize_private_key(input: &str) -> Result<String, String> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let _ = decode_private_key_bytes(&cleaned)?;
    Ok(cleaned)
}

/// 解码Base64私钥为字节数组
///
/// 将Base64编码的私钥字符串解码为32字节的Ed25519私钥
///
/// # Arguments
///
/// * `raw` - Base64编码的私钥字符串
///
/// # Returns
///
/// * `Ok([u8; 32])` - 32字节的私钥数据
/// * `Err(String)` - 解码失败的错误信息
fn decode_private_key_bytes(raw: &str) -> Result<[u8; 32], String> {
    let bytes = B64_STANDARD.decode(raw).map_err(|e| e.to_string())?;
    bytes
        .try_into()
        .map_err(|_| "管理员私钥需为 32 字节的 Ed25519 秘钥".into())
}

/// 从配置加载签名私钥
///
/// 从配置中读取Base64编码的私钥并创建签名密钥对象
///
/// # Arguments
///
/// * `cfg` - 包含私钥的配置对象
///
/// # Returns
///
/// * `Ok(SigningKey)` - Ed25519签名密钥
/// * `Err(String)` - 加载失败的错误信息
pub fn load_signing_key(cfg: &Config) -> Result<SigningKey, String> {
    let pk = cfg
        .private_key_b64
        .as_ref()
        .ok_or_else(|| "未配置管理员私钥".to_string())?;
    let bytes = decode_private_key_bytes(pk)?;
    Ok(SigningKey::from_bytes(&bytes))
}

/// 从配置生成公钥指纹
///
/// 从配置中的私钥提取公钥并计算其SHA256指纹
///
/// # Arguments
///
/// * `cfg` - 包含私钥的配置对象
///
/// # Returns
///
/// * `Ok(String)` - 公钥的十六进制指纹
/// * `Err(String)` - 生成失败的错误信息
pub fn fingerprint(cfg: &Config) -> Result<String, String> {
    let key = load_signing_key(cfg)?;
    Ok(compute_fingerprint(&key.verifying_key().to_bytes()))
}

/// 计算公钥的SHA256指纹
///
/// 对公钥字节数组计算SHA256哈希值并返回十六进制字符串
///
/// # Arguments
///
/// * `public_key` - 32字节的Ed25519公钥数据
///
/// # Returns
///
/// 公钥的十六进制SHA256指纹
pub fn compute_fingerprint(public_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key);
    hex::encode(hasher.finalize())
}
