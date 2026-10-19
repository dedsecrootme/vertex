use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, Method, header};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

#[derive(Clone, Default)]
pub struct Access {
    pub origin: Option<String>,
    pub authority: Option<String>,
    admin_hash: Option<[u8; 32]>,
}

impl Access {
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("VERTEX_PUBLIC_ORIGIN") {
            Ok(origin) => Self::public(
                &origin,
                &std::env::var("VERTEX_ADMIN_TOKEN").unwrap_or_default(),
            ),
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(_) => Err("VERTEX_PUBLIC_ORIGIN invalide".into()),
        }
    }

    pub fn public(origin: &str, token: &str) -> Result<Self, String> {
        let url = search_rs::network::validate_url(origin)?;
        if url.scheme() != "https"
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("VERTEX_PUBLIC_ORIGIN doit etre une origine HTTPS sans chemin".into());
        }
        if !(32..=256).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(
                "VERTEX_ADMIN_TOKEN doit contenir 32 a 256 caracteres ASCII sans espace".into(),
            );
        }
        let origin = url.origin().ascii_serialization();
        Ok(Self {
            authority: Some(origin.trim_start_matches("https://").to_string()),
            origin: Some(origin),
            admin_hash: Some(Sha256::digest(token.as_bytes()).into()),
        })
    }

    pub fn is_public(&self) -> bool {
        self.origin.is_some()
    }

    pub fn is_admin(&self, headers: &HeaderMap) -> bool {
        if !self.is_public() {
            return true;
        }
        let Some(token) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            return false;
        };
        let actual: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        self.admin_hash
            .as_ref()
            .is_some_and(|expected| bool::from(expected.ct_eq(&actual)))
    }

    pub fn accepts(&self, headers: &HeaderMap, port: u16) -> bool {
        let hosts = if let Some(host) = &self.authority {
            vec![host.clone()]
        } else {
            vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")]
        };
        let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
            return false;
        };
        if !hosts
            .iter()
            .any(|allowed| host.eq_ignore_ascii_case(allowed))
        {
            return false;
        }
        if let Some(origin) = headers.get(header::ORIGIN) {
            let Ok(origin) = origin.to_str() else {
                return false;
            };
            let valid = if let Some(expected) = &self.origin {
                origin == expected
            } else {
                hosts.iter().any(|h| origin == format!("http://{h}"))
            };
            if !valid {
                return false;
            }
        }
        true
    }
}

pub fn requires_admin(method: &Method, path: &str) -> bool {
    path.starts_with("/api/admin/")
        || path == "/api/bookmarks"
        || path == "/api/contact/read"
        || path == "/api/crawl"
        || (path == "/api/contact" && method != Method::POST)
}

// Global budgets deliberately ignore spoofable forwarded IP headers. A reverse
// proxy should add fair per-client limits in addition to this bounded safeguard.
pub struct Budget(Mutex<(Instant, u32, u32, u32)>);
impl Default for Budget {
    fn default() -> Self {
        Self(Mutex::new((Instant::now(), 0, 0, 0)))
    }
}
impl Budget {
    pub fn allow(&self, contact: bool) -> bool {
        let mut window = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if window.0.elapsed() >= Duration::from_secs(60) {
            *window = (Instant::now(), 0, 0, 0);
        }
        if window.1 >= 600 || (contact && window.2 >= 5) {
            return false;
        }
        window.1 += 1;
        if contact {
            window.2 += 1;
        }
        true
    }

    pub fn allow_discovery(&self) -> bool {
        let mut window = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if window.0.elapsed() >= Duration::from_secs(60) {
            *window = (Instant::now(), 0, 0, 0);
        }
        if window.3 >= 30 {
            return false;
        }
        window.3 += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TOKEN: &str = "a-long-random-test-token-with-32-characters";
    #[test]
    fn public_configuration_fails_closed() {
        for origin in [
            "http://vertex.example",
            "https://vertex.example/path",
            "https://vertex.example/?q=1",
            "https://127.0.0.1",
        ] {
            assert!(Access::public(origin, TOKEN).is_err());
        }
        assert!(Access::public("https://vertex.example", "short").is_err());
    }
    #[test]
    fn public_host_origin_and_token_are_independent() {
        let access = Access::public("https://vertex.example", TOKEN).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "vertex.example".parse().unwrap());
        assert!(access.accepts(&headers, 3000));
        assert!(!access.is_admin(&headers));
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        assert!(access.is_admin(&headers));
        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!access.accepts(&headers, 3000));
        headers.remove(header::ORIGIN);
        headers.insert(header::HOST, "127.0.0.1:3000".parse().unwrap());
        assert!(!access.accepts(&headers, 3000));
    }
    #[test]
    fn personal_routes_require_admin() {
        assert!(requires_admin(&Method::GET, "/api/admin/monitor"));
        assert!(requires_admin(&Method::POST, "/api/admin/verify"));
        assert!(requires_admin(&Method::GET, "/api/contact"));
        assert!(requires_admin(&Method::POST, "/api/crawl"));
        assert!(requires_admin(&Method::GET, "/api/bookmarks"));
        assert!(!requires_admin(&Method::POST, "/api/contact"));
        assert!(!requires_admin(&Method::GET, "/api/search"));
    }
    #[test]
    fn budgets_bound_abuse() {
        let budget = Budget::default();
        for _ in 0..5 {
            assert!(budget.allow(true));
        }
        assert!(!budget.allow(true));
        for _ in 5..600 {
            assert!(budget.allow(false));
        }
        assert!(!budget.allow(false));
    }
}
