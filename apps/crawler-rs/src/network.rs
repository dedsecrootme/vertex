//! Shared outbound HTTP policy: public destinations, checked DNS and redirects,
//! and bounded reads. Validated DNS answers are used directly by the connector.

use std::io::{self, Read};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || a == 0
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && (c == 0 || c == 2))
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] & 0xf000 == 0)
        }
    }
}

pub fn validate_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "Invalid URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Only public HTTP(S) URLs without credentials are allowed".into());
    }
    let allowed = match url.host() {
        Some(Host::Ipv4(ip)) => is_public_ip(ip.into()),
        Some(Host::Ipv6(ip)) => is_public_ip(ip.into()),
        Some(Host::Domain(host)) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            host.contains('.')
                && ![
                    "localhost",
                    "local",
                    "internal",
                    "lan",
                    "home",
                    "test",
                    "invalid",
                ]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        }
        None => false,
    };
    if !allowed {
        return Err("Local or reserved network destination blocked".into());
    }
    if crate::safety::is_malicious(&crate::safety::check_url(value)) {
        return Err("Destination blocked by the URL safety policy".into());
    }
    Ok(url)
}

fn checked_addresses(addresses: Vec<SocketAddr>) -> io::Result<Addrs> {
    if addresses.is_empty() || addresses.iter().any(|addr| !is_public_ip(addr.ip())) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Non-public DNS answer blocked",
        ));
    }
    Ok(Box::new(addresses.into_iter()))
}

struct PublicDns;

impl Resolve for PublicDns {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addresses = tokio::task::spawn_blocking(move || {
                (host.as_str(), 0).to_socket_addrs().map(|a| a.collect())
            })
            .await??;
            Ok(checked_addresses(addresses)?)
        })
    }
}

fn redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("Too many redirects")
        } else if let Err(error) = validate_url(attempt.url().as_str()) {
            attempt.error(error)
        } else {
            attempt.follow()
        }
    })
}

pub fn async_client(
    timeout: Duration,
    user_agent: &str,
) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(PublicDns))
        .redirect(redirects())
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(8)))
        .user_agent(user_agent)
        .build()
}

pub fn blocking_client(
    timeout: Duration,
    user_agent: &str,
) -> Result<reqwest::blocking::Client, reqwest::Error> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(PublicDns))
        .redirect(redirects())
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(8)))
        .user_agent(user_agent)
        .build()
}

/// Index requests stay on the explicitly configured origin, including POST bodies.
pub fn api_client(timeout: Duration, user_agent: &str) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(PublicDns))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(8)))
        .user_agent(user_agent)
        .build()
}

/// Crawler redirects are followed manually so every hop checks robots/politeness.
pub fn crawler_client(
    timeout: Duration,
    user_agent: &str,
) -> Result<reqwest::blocking::Client, reqwest::Error> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(PublicDns))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(timeout.min(Duration::from_secs(8)))
        .user_agent(user_agent)
        .build()
}

/// Reads at most limit + 1 bytes, including responses without Content-Length.
pub fn read_limited(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut body)?;
    if body.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "Response exceeds size limit",
        ));
    }
    Ok(body)
}

pub async fn read_limited_async(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|len| len > limit as u64)
    {
        return Err("Response exceeds size limit".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err("Response exceeds size limit".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub fn image_content_type(value: &str) -> Option<&str> {
    let mime = value.split(';').next()?.trim();
    [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/avif",
        "image/bmp",
        "image/x-icon",
        "image/vnd.microsoft.icon",
    ]
    .into_iter()
    .find(|allowed| mime.eq_ignore_ascii_case(allowed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_private_reserved_and_mapped_addresses() {
        for ip in [
            "0.1.2.3",
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "224.0.0.1",
            "240.1.2.3",
            "::1",
            "::",
            "fc00::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "2002:7f00:1::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn rejects_unsafe_urls_and_credentials() {
        for url in [
            "file:///secret",
            "http://localhost./",
            "http://a.localhost/",
            "http://router/",
            "http://a.local/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://user:pass@example.com/",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
        assert!(validate_url("https://www.rust-lang.org/").is_ok());
    }

    #[test]
    fn rejects_mixed_public_and_private_dns_answers() {
        let addresses = vec!["1.1.1.1:0".parse().unwrap(), "127.0.0.1:0".parse().unwrap()];
        assert!(checked_addresses(addresses).is_err());
        assert!(checked_addresses(Vec::new()).is_err());
    }

    #[test]
    fn enforces_body_limits_before_reading_everything() {
        let mut input = io::Cursor::new(vec![0; 100]);
        assert_eq!(
            read_limited(&mut input, 10).unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
        assert_eq!(input.position(), 11);
        assert_eq!(read_limited(&b"123"[..], 3).unwrap(), b"123");
    }

    #[test]
    fn rejects_active_media_types() {
        assert_eq!(
            image_content_type("image/png; charset=binary"),
            Some("image/png")
        );
        assert_eq!(image_content_type("image/svg+xml"), None);
        assert_eq!(image_content_type("text/html"), None);
    }

    #[test]
    fn redirect_policy_blocks_private_destinations_before_following() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0; 2048];
            assert!(stream.read(&mut buffer).unwrap() > 0);
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        // Initial loopback access is used only to serve this test fixture.
        let result = blocking_client(Duration::from_secs(2), "test")
            .unwrap()
            .get(format!("http://{address}/"))
            .send();
        server.join().unwrap();
        assert!(result.unwrap_err().is_redirect());
    }

    #[test]
    fn resolver_blocks_loopback_dns_answers() {
        let result = blocking_client(Duration::from_secs(2), "test")
            .unwrap()
            .get("http://localhost:9/")
            .send();
        assert!(format!("{:?}", result.unwrap_err()).contains("Non-public DNS answer blocked"));
    }

    #[tokio::test]
    async fn index_client_does_not_forward_requests_through_redirects() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            assert!(stream.read(&mut request).unwrap() > 0);
            write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{address}/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        // Loopback is only the isolated test fixture, never a SharedIndex origin.
        let response = api_client(Duration::from_secs(2), "test")
            .unwrap()
            .post(format!("http://{address}/proposal"))
            .body("fixture-only")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
        server.join().unwrap();
    }
}
