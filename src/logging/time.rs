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
/// - 兼容 Postgres 常见字符串格式：`YYYY-MM-DD HH:mm:ss(.f)?(+/-offset)`
/// - 回退兼容旧格式：`YYYY-MM-DD HH:mm:ss`（按北京时间解释）
pub fn parse_datetime_string(s: &str) -> crate::error::Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    fn normalize_trailing_offset(raw: &str) -> Option<String> {
        let pos = raw.rfind(|c| c == '+' || c == '-')?;
        let (prefix, offset) = raw.split_at(pos);
        if offset.contains(':') {
            return None;
        }
        match offset.len() {
            // +HH / -HH
            3 => Some(format!("{prefix}{offset}:00")),
            // +HHMM / -HHMM
            5 => Some(format!("{prefix}{}:{}", &offset[..3], &offset[3..])),
            _ => None,
        }
    }

    // Some deployments might have stored timestamps like "YYYY-MM-DD HH:mm:ss UTC".
    if let Some(stripped) = s.strip_suffix(" UTC") {
        use chrono::NaiveDateTime;
        let naive = NaiveDateTime::parse_from_str(stripped, DATETIME_FORMAT)
            .map_err(|e| GatewayError::TimeParse(e.to_string()))?;
        return Ok(Utc.from_utc_datetime(&naive));
    }

    // Postgres can surface "YYYY-MM-DD HH:mm:ss+00" / "+0000" / "+00:00" (optionally with .f)
    // depending on column type / legacy values.
    let candidates = [Some(s.to_string()), normalize_trailing_offset(s)];
    for cand in candidates.into_iter().flatten() {
        for fmt in [
            "%Y-%m-%d %H:%M:%S%:z",
            "%Y-%m-%d %H:%M:%S%.f%:z",
            "%Y-%m-%d %H:%M:%S%z",
            "%Y-%m-%d %H:%M:%S%.f%z",
        ] {
            if let Ok(dt) = DateTime::parse_from_str(&cand, fmt) {
                return Ok(dt.with_timezone(&Utc));
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_datetime_string_accepts_rfc3339() {
        let dt = parse_datetime_string("2026-01-20T10:20:30Z").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 1, 20, 10, 20, 30).unwrap());
    }

    #[test]
    fn parse_datetime_string_accepts_beijing_legacy() {
        // 18:20:30 Beijing == 10:20:30 UTC
        let dt = parse_datetime_string("2026-01-20 18:20:30").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 1, 20, 10, 20, 30).unwrap());
    }

    #[test]
    fn parse_datetime_string_accepts_pg_offset_short() {
        let dt = parse_datetime_string("2026-01-20 10:20:30+00").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 1, 20, 10, 20, 30).unwrap());
    }

    #[test]
    fn parse_datetime_string_accepts_pg_offset_colon() {
        let dt = parse_datetime_string("2026-01-20 10:20:30+00:00").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 1, 20, 10, 20, 30).unwrap());
    }

    #[test]
    fn parse_datetime_string_accepts_pg_offset_hhmm() {
        let dt = parse_datetime_string("2026-01-20 10:20:30+0000").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 1, 20, 10, 20, 30).unwrap());
    }

    #[test]
    fn parse_datetime_string_accepts_pg_utc_suffix() {
        let dt = parse_datetime_string("2026-01-20 10:20:30 UTC").unwrap();
        assert_eq!(dt, Utc.with_ymd_and_hms(2026, 1, 20, 10, 20, 30).unwrap());
    }
}
