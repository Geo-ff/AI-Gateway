use crate::error::GatewayError;
use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr};

fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || is_v4_shared(v4)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    }
}

// 100.64.0.0/10（CGNAT 共享地址）
fn is_v4_shared(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (64..=127).contains(&b)
}

fn is_disallowed_host(domain: &str) -> bool {
    let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    d == "localhost" || d.ends_with(".localhost") || d.ends_with(".local")
}

/// SSRF 基础防护：
/// - 仅允许 http/https
/// - 禁止 userinfo
/// - 禁止本机/内网/链路本地/ULA 等地址（域名会进行 DNS 解析校验）
pub async fn validate_outbound_base_url(raw: &str) -> Result<Url, GatewayError> {
    let url =
        Url::parse(raw).map_err(|_| GatewayError::Config("base_url 不是合法的 URL".into()))?;

    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(GatewayError::Config("base_url 仅允许 http/https".into())),
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(GatewayError::Config(
            "base_url 不允许包含用户名/密码".into(),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| GatewayError::Config("base_url 缺少 host".into()))?;

    let port = url.port_or_known_default().unwrap_or(443);

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_disallowed_ip(ip) {
            return Err(GatewayError::Config("base_url 不允许指向本机/内网".into()));
        }
    } else {
        if is_disallowed_host(host) {
            return Err(GatewayError::Config("base_url 不允许指向本机/内网".into()));
        }
        let addrs = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| GatewayError::Config("base_url 域名解析失败".into()))?;
        for addr in addrs {
            if is_disallowed_ip(addr.ip()) {
                return Err(GatewayError::Config("base_url 不允许指向本机/内网".into()));
            }
        }
    }

    Ok(url)
}

pub fn join_models_url(base_url: &Url, models_endpoint: Option<&str>) -> Result<Url, GatewayError> {
    if let Some(ep) = models_endpoint {
        let ep = ep.trim();
        if ep.is_empty() {
            return Err(GatewayError::Config(
                "models_endpoint 不能为空字符串".into(),
            ));
        }
        if ep.starts_with("http://") || ep.starts_with("https://") {
            return Url::parse(ep)
                .map_err(|_| GatewayError::Config("models_endpoint 不是合法的 URL".into()));
        }
        if !ep.starts_with('/') {
            return Err(GatewayError::Config(
                "models_endpoint 需要以 '/' 开头（或提供完整 URL）".into(),
            ));
        }
        let full = format!("{}{}", base_url.as_str().trim_end_matches('/'), ep);
        return Url::parse(&full)
            .map_err(|_| GatewayError::Config("models_endpoint 拼接失败".into()));
    }

    // OpenAI 兼容：既支持 base_url=.../v1（追加 /models），也支持 base_url=...（追加 /v1/models）
    // 兼容火山引擎 Ark：base_url=.../api/v3（追加 /models）
    let path = base_url.path().trim_end_matches('/');
    let base = base_url.as_str().trim_end_matches('/');
    let full = if path.ends_with("/v1") || path.ends_with("/api/v3") {
        format!("{}/models", base)
    } else {
        format!("{}/v1/models", base)
    };
    Url::parse(&full).map_err(|_| GatewayError::Config("models URL 拼接失败".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ssrf_rejects_loopback() {
        let err = validate_outbound_base_url("http://127.0.0.1:8000")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("不允许"));
    }

    #[tokio::test]
    async fn ssrf_rejects_private_v4() {
        assert!(validate_outbound_base_url("http://10.0.0.1").await.is_err());
        assert!(
            validate_outbound_base_url("http://192.168.1.1")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn ssrf_rejects_link_local_v4() {
        assert!(
            validate_outbound_base_url("http://169.254.0.1")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn ssrf_allows_public_ip() {
        assert!(validate_outbound_base_url("https://1.1.1.1").await.is_ok());
    }

    #[tokio::test]
    async fn ssrf_requires_http_scheme() {
        assert!(
            validate_outbound_base_url("file:///etc/passwd")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn ssrf_rejects_userinfo() {
        assert!(
            validate_outbound_base_url("https://user:pass@example.com")
                .await
                .is_err()
        );
    }

    #[test]
    fn join_models_url_default_openai() {
        let base = Url::parse("https://api.openai.com").unwrap();
        let u = join_models_url(&base, None).unwrap();
        assert_eq!(u.as_str(), "https://api.openai.com/v1/models");

        let base = Url::parse("https://api.openai.com/v1").unwrap();
        let u = join_models_url(&base, None).unwrap();
        assert_eq!(u.as_str(), "https://api.openai.com/v1/models");
    }

    #[test]
    fn join_models_url_custom_endpoint() {
        let base = Url::parse("https://example.com/v1").unwrap();
        let u = join_models_url(&base, Some("/models")).unwrap();
        assert_eq!(u.as_str(), "https://example.com/v1/models");

        let u = join_models_url(&base, Some("https://foo.bar/v1/models")).unwrap();
        assert_eq!(u.as_str(), "https://foo.bar/v1/models");
    }
}
