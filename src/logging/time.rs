use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use crate::error::GatewayError;

// 北京时间时区 (UTC+8)
pub const BEIJING_OFFSET: FixedOffset = FixedOffset::east_opt(8 * 3600).unwrap();
pub const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// 将 UTC 时间转换为北京时间的人类友好格式
pub fn to_beijing_string(dt: &DateTime<Utc>) -> String {
    dt.with_timezone(&BEIJING_OFFSET).format(DATETIME_FORMAT).to_string()
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

