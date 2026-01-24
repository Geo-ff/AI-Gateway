use std::time::Duration;

use reqwest::ClientBuilder;

fn has_proxy_env() -> bool {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .any(|k| std::env::var(k).is_ok_and(|v| !v.trim().is_empty()))
}

fn should_bypass_proxy_impl(url: &str, proxy_env_present: bool) -> bool {
    if !proxy_env_present {
        return false;
    }

    // Escape hatch: allow users to keep proxy behavior even for Volcengine (Ark) domains.
    if std::env::var("GATEWAY_ALLOW_PROXY_FOR_VOLCES")
        .is_ok_and(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
    {
        return false;
    }

    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    let Some(host) = u.host_str() else {
        return false;
    };

    host == "ark.cn-beijing.volces.com" || host.ends_with(".volces.com")
}

pub fn should_bypass_proxy_for_url(url: &str) -> bool {
    should_bypass_proxy_impl(url, has_proxy_env())
}

pub fn maybe_disable_proxy(builder: ClientBuilder, url: &str) -> ClientBuilder {
    if should_bypass_proxy_for_url(url) {
        builder.no_proxy()
    } else {
        builder
    }
}

pub fn client_for_url(url: &str) -> Result<reqwest::Client, reqwest::Error> {
    let builder = reqwest::Client::builder();
    maybe_disable_proxy(builder, url).build()
}

pub fn client_for_url_with_timeout(
    url: &str,
    timeout: Duration,
) -> Result<reqwest::Client, reqwest::Error> {
    let builder = reqwest::Client::builder().timeout(timeout);
    maybe_disable_proxy(builder, url).build()
}

#[cfg(test)]
mod tests {
    use super::should_bypass_proxy_impl;

    #[test]
    fn bypass_proxy_for_volces_when_proxy_env_present() {
        assert!(should_bypass_proxy_impl(
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
            true
        ));
        assert!(should_bypass_proxy_impl(
            "https://ark.cn-shanghai.volces.com/api/v3/chat/completions",
            true
        ));
        assert!(should_bypass_proxy_impl("https://example.volces.com", true));
    }

    #[test]
    fn do_not_bypass_without_proxy_env() {
        assert!(!should_bypass_proxy_impl(
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
            false
        ));
    }
}
