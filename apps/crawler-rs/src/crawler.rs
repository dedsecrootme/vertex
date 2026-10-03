//! Récupération et parsing d'une page web.
//!
//! Le client HTTP applique un délai, un plafond de redirections, un
//! User-Agent identifiable et un plafond de taille. La sûreté de l'URL est
//! vérifiée avant tout accès réseau.

use crate::models::Page;
use crate::{parser, safety};

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};

use sha2::{Digest, Sha256};

use std::fmt;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use url::Url;

/// Taille maximale du corps téléchargé.
pub const MAX_CONTENT_SIZE: usize = 5_000_000;

/// User-Agent identifiable transmis aux sites (politesse / traçabilité).
pub fn user_agent() -> String {
    format!(
        "VertexCrawler/{} (+https://github.com/dedsecrootme/vertex)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Erreur de crawl classée (permet au moteur de choisir le statut correct).
#[derive(Debug)]
pub enum CrawlError {
    /// URL jugée malveillante : aucun accès réseau effectué.
    Blocked(String),
    /// Type de contenu non exploitable ou taille excessive.
    Skipped(String),
    /// Erreur réseau / HTTP / protocole.
    Failed(String),
}

impl fmt::Display for CrawlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CrawlError::Blocked(m) | CrawlError::Skipped(m) | CrawlError::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CrawlError {}

/// Résultat de récupération d'une page.
pub struct CrawlResult {
    pub page: Page,
    pub http_code: u16,
}

/// Construit le client HTTP partagé par les workers.
pub fn build_client(timeout_secs: u64) -> Client {
    Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(user_agent())
        .build()
        .expect("Impossible de créer le client HTTP")
}

/// Télécharge, assainit et analyse une page HTML.
pub fn crawl(client: &Client, url: &str) -> Result<CrawlResult, CrawlError> {
    let report = safety::check_url(url);
    if safety::is_malicious(&report) {
        return Err(CrawlError::Blocked(format!(
            "URL malveillante bloquée ({})",
            report.reasons.join(" ; ")
        )));
    }

    if !is_crawlable_url(url) {
        return Err(CrawlError::Skipped("URL non autorisée pour le crawl".into()));
    }

    let base = Url::parse(url).map_err(|e| CrawlError::Failed(format!("URL invalide : {e}")))?;
    let start = Instant::now();

    let response = client
        .get(url)
        .header(USER_AGENT, user_agent())
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .header(ACCEPT_LANGUAGE, "fr,en;q=0.8")
        .send()
        .map_err(|e| CrawlError::Failed(format!("Requête échouée : {e}")))?;

    let status = response.status();
    let http_code = status.as_u16();

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    if !status.is_success() {
        return Err(CrawlError::Failed(format!("HTTP {status}")));
    }

    if let Some(ref ct) = content_type {
        let ct_lower = ct.to_ascii_lowercase();
        if !ct_lower.contains("text/html") && !ct_lower.contains("application/xhtml") {
            return Err(CrawlError::Skipped(format!("Type non supporté : {ct}")));
        }
    }

    if let Some(len) = response.content_length()
        && len > MAX_CONTENT_SIZE as u64
    {
        return Err(CrawlError::Skipped("Page trop volumineuse".into()));
    }

    let bytes = response
        .bytes()
        .map_err(|e| CrawlError::Failed(format!("Lecture du corps échouée : {e}")))?;

    if bytes.len() > MAX_CONTENT_SIZE {
        return Err(CrawlError::Skipped("Page trop volumineuse".into()));
    }

    let size = bytes.len();
    let html = String::from_utf8_lossy(&bytes).to_string();

    // Un seul parse du DOM pour toute l'extraction (gain de performance).
    let parsed = parser::analyze(&html, &base);

    let mut hasher = Sha256::new();
    hasher.update(parsed.text.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());

    let page = Page {
        url: url.to_string(),
        domain: base.host_str().unwrap_or("").to_ascii_lowercase(),
        status_code: http_code,
        title: parsed.title,
        description: parsed.description,
        content: parsed.text,
        content_html: parsed.sanitized_html,
        links: parsed.links,
        images: parsed.images,
        favicon: parsed.favicon,
        content_hash,
        size,
        content_type,
        language: parsed.language,
        depth: 0,
        crawl_time_ms: start.elapsed().as_millis(),
    };

    Ok(CrawlResult { page, http_code })
}

/// Extrait le nom de domaine d'une URL.
pub fn extract_domain(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.domain().map(str::to_string))
        .unwrap_or_default()
}

/// Normalise un lien détecté dans une page en URL absolue propre
/// (fragments et paramètres de suivi retirés, ressources binaires ignorées).
pub fn normalize_url(base: &str, link: &str) -> Option<String> {
    let base = Url::parse(base).ok()?;
    let normalized = parser::absolutize(&base, link)?;
    let url = Url::parse(&normalized).ok()?;

    let path = url.path().to_ascii_lowercase();
    const IGNORED: &[&str] = &[
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp", ".bmp", ".tiff", ".css", ".js",
        ".json", ".xml", ".zip", ".rar", ".7z", ".tar", ".gz", ".exe", ".msi", ".dmg", ".apk",
        ".pdf", ".mp4", ".mp3", ".avi", ".mov", ".mkv", ".webm", ".woff", ".woff2", ".ttf", ".eot",
        ".rss", ".atom",
    ];
    if IGNORED.iter().any(|ext| path.ends_with(ext)) {
        return None;
    }

    Some(url.to_string().trim_end_matches('/').to_string())
}

/// Vrai si l'URL est un point d'entrée web public autorisé au crawl.
pub fn is_crawlable_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| is_crawlable(&url))
}

/// Refuse les schémas non web, les identifiants dans l'URL et les adresses
/// privées/loopback (protège les services locaux de la machine).
fn is_crawlable(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }

    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']);
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return false;
    }

    match host.parse::<Ipv4Addr>() {
        Ok(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast())
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_crawlable_url, normalize_url};

    #[test]
    fn normalizes_tracking_parameters_and_fragments() {
        assert_eq!(
            normalize_url(
                "https://example.com/a/",
                "../guide?utm_source=newsletter&id=7#intro"
            ),
            Some("https://example.com/guide?id=7".to_string())
        );
    }

    #[test]
    fn rejects_local_and_non_web_urls() {
        assert!(!is_crawlable_url("file:///C:/secret.txt"));
        assert!(!is_crawlable_url("http://127.0.0.1:8080"));
        assert!(!is_crawlable_url("http://localhost:3000"));
        assert!(is_crawlable_url("https://www.rust-lang.org/"));
    }

    #[test]
    fn ignores_binary_resources() {
        assert_eq!(normalize_url("https://x.tld/", "/img/a.png"), None);
        assert_eq!(normalize_url("https://x.tld/", "/doc.pdf"), None);
    }
}
