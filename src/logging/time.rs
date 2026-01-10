use crate::error::GatewayError;
use chrono::{DateTime, FixedOffset, SecondsFormat, TimeZone, Utc};

// 北京时间时区 (UTC+8)
pub const BEIJING_OFFSET: FixedOffset = FixedOffset::east_opt(8 * 3600).unwrap();
pub const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// 将 UTC 时间转换为北京时间的人类友好格式
pub fn to_beijing_string(dt: &DateTime<Utc>) -> String {
    dt.with_timezone(&BEIJING_OFFSET)
        .format(DATETIME_FORMAT)
        .to_string()
}

/// 将 UTC 时间转换为 ISO-8601 / RFC3339（UTC, `Z`）
pub fn to_iso8601_utc_string(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 从北京时间字符串解析为 UTC 时间
pub fn parse_beijing_string(s: &str) -> crate::error::Result<DateTime<Utc>> {
    use chrono::NaiveDateTime;
    let naive_dt = NaiveDateTime::parse_from_str(s, DATETIME_FORMAT)
        .map_err(|e| GatewayError::TimeParse(e.to_string()))?;
    let beijing_dt = BEIJING_OFFSET
        .from_local_datetime(&naive_dt)
        .single()
        .ok_or_else(|| GatewayError::TimeParse("Invalid local datetime".into()))?;
    Ok(beijing_dt.with_timezone(&Utc))
}

/// 解析时间字符串为 UTC：
/// - 优先 RFC3339 / ISO-8601（带时区偏移或 `Z`）
/// - 回退兼容旧格式：`YYYY-MM-DD HH:mm:ss`（按北京时间解释）
pub fn parse_datetime_string(s: &str) -> crate::error::Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    parse_beijing_string(s)
}

// tracing_subscriber 自定义时间格式：输出北京时间，与数据库一致
pub struct BeijingTimer;

impl tracing_subscriber::fmt::time::FormatTime for BeijingTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        let now = Utc::now();
        let s = to_beijing_string(&now);
        write!(w, "{}", s)
    }
}
