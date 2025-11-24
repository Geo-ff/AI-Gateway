use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 会话缓存数据结构，用于存储管理员会话信息
///
/// 包含会话令牌和过期时间，支持序列化和反序列化用于持久化存储
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionCache {
    pub token: String,
    pub expires_at: String,
}

impl SessionCache {
    /// 解析过期时间字符串为UTC时间
    ///
    /// # Returns
    ///
    /// * `Some(DateTime<Utc>)` - 成功解析的UTC时间
    /// * `None` - 解析失败时返回None
    ///
    /// # Examples
    ///
    /// ```
    /// use gateway_tui::session::SessionCache;
    ///
    /// let cache = SessionCache {
    ///     token: "token123".to_string(),
    ///     expires_at: "2024-12-31T23:59:59Z".to_string(),
    /// };
    ///
    /// if let Some(expiry) = cache.expires_at() {
    ///     println!("Expires at: {}", expiry);
    /// }
    /// ```
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.expires_at)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

}

/// 获取会话缓存文件的路径
///
/// 优先使用当前工作目录，如果获取失败则使用当前目录
/// 缓存文件名固定为 "session_cache.json"
///
/// # Returns
///
/// 会话缓存文件的完整路径
fn cache_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("session_cache.json")
}

/// 从磁盘加载会话缓存
///
/// 读取并解析会话缓存文件，如果文件不存在或格式错误则返回None
///
/// # Returns
///
/// * `Some(SessionCache)` - 成功加载的会话缓存
/// * `None` - 文件不存在或解析失败
///
/// # Examples
///
/// ```
/// use gateway_tui::session::load_cache;
///
/// if let Some(cache) = load_cache() {
///     println!("Loaded session: {}", cache.token);
/// } else {
///     println!("No cached session found");
/// }
/// ```
pub fn load_cache() -> Option<SessionCache> {
    let path = cache_path();
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// 保存会话缓存到磁盘
///
/// 将会话缓存序列化为JSON格式并写入文件
///
/// # Arguments
///
/// * `cache` - 要保存的会话缓存数据
///
/// # Returns
///
/// * `Ok(())` - 保存成功
/// * `Err(String)` - 保存失败，包含错误信息
///
/// # Examples
///
/// ```
/// use gateway_tui::session::{SessionCache, save_cache};
///
/// let cache = SessionCache {
///     token: "token123".to_string(),
///     expires_at: "2024-12-31T23:59:59Z".to_string(),
/// };
///
/// match save_cache(&cache) {
///     Ok(()) => println!("Cache saved successfully"),
///     Err(e) => eprintln!("Failed to save cache: {}", e),
/// }
/// ```
pub fn save_cache(cache: &SessionCache) -> Result<(), String> {
    let path = cache_path();
    let json = serde_json::to_string_pretty(cache).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// 清除磁盘上的会话缓存文件
///
/// 删除会话缓存文件，如果文件不存在或删除失败则忽略错误
/// 这通常在会话过期或用户登出时调用
///
/// # Examples
///
/// ```
/// use gateway_tui::session::clear_cache;
///
/// clear_cache();
/// println!("Session cache cleared");
/// ```
pub fn clear_cache() {
    let path = cache_path();
    let _ = fs::remove_file(path);
}
