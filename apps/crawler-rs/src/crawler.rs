//! Récupération et parsing d'une page web.
//!
//! Le client HTTP applique un délai, un plafond de redirections, un
//! User-Agent identifiable et un plafond de taille. La sûreté de l'URL est
//! vérifiée avant tout accès réseau.

use crate::models::Page;
use crate::{network, parser, safety};

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};

use sha2::{Digest, Sha256};

use std::fmt;
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
    Deadline,
    Deferred {
        reason: String,
        after_secs: u64,
    },
}

impl fmt::Display for CrawlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CrawlError::Blocked(m) | CrawlError::Skipped(m) | CrawlError::Failed(m) => {
                f.write_str(m)
            }
            CrawlError::Deferred { reason, .. } => f.write_str(reason),
            CrawlError::Deadline => f.write_str("Duree de session atteinte"),
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
    network::crawler_client(
        Duration::from_secs(timeout_secs.clamp(1, 30)),
        &user_agent(),
    )
    .expect("Impossible de créer le client HTTP")
}

/// Télécharge, assainit et analyse une page HTML.
pub fn crawl(client: &Client, url: &str) -> Result<CrawlResult, CrawlError> {
    crawl_with(client, url, |_| Ok(()))
}

pub fn fetch_response(
    client: &Client,
    url: &str,
    mut before_request: impl FnMut(&str) -> Result<(), CrawlError>,
) -> Result<reqwest::blocking::Response, CrawlError> {
    let mut current = url.to_string();
    for hop in 0..=10 {
        network::validate_url(&current).map_err(CrawlError::Blocked)?;
        before_request(&current)?;
        let response = client
            .get(&current)
            .header(USER_AGENT, user_agent())
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,application/xml,text/plain",
            )
            .header(ACCEPT_LANGUAGE, "fr,en;q=0.8")
            .send()
            .map_err(|e| CrawlError::Failed(format!("Requete echouee : {e}")))?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if hop == 10 {
                return Err(CrawlError::Failed("Trop de redirections".into()));
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| CrawlError::Failed("Redirection sans Location valide".into()))?;
            current = response
                .url()
                .join(location)
                .map_err(|e| CrawlError::Failed(e.to_string()))?
                .to_string();
            continue;
        }
        return Ok(response);
    }
    unreachable!()
}

fn retry_delay(value: Option<&str>, now: std::time::SystemTime) -> u64 {
    value
        .and_then(|s| {
            s.trim().parse::<u64>().ok().or_else(|| {
                httpdate::parse_http_date(s)
                    .ok()
                    .map(|date| date.duration_since(now).unwrap_or_default().as_secs())
            })
        })
        .unwrap_or(30)
        .max(1)
}

pub fn crawl_with(
    client: &Client,
    url: &str,
    before_request: impl FnMut(&str) -> Result<(), CrawlError>,
) -> Result<CrawlResult, CrawlError> {
    let report = safety::check_url(url);
    if safety::is_malicious(&report) {
        return Err(CrawlError::Blocked(format!(
            "URL malveillante bloquée ({})",
            report.reasons.join(" ; ")
        )));
    }

    if !is_crawlable_url(url) {
        return Err(CrawlError::Skipped(
            "URL non autorisée pour le crawl".into(),
        ));
    }

    let start = Instant::now();

    let response = fetch_response(client, url, before_request)?;

    let status = response.status();
    let http_code = status.as_u16();
    let base = response.url().clone();

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    if matches!(http_code, 429 | 503) {
        return Err(CrawlError::Deferred {
            reason: format!("HTTP {status}"),
            after_secs: retry_delay(
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok()),
                std::time::SystemTime::now(),
            ),
        });
    }
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

    let bytes = network::read_limited(response, MAX_CONTENT_SIZE).map_err(|e| {
        if e.kind() == std::io::ErrorKind::FileTooLarge {
            CrawlError::Skipped("Page trop volumineuse".into())
        } else {
            CrawlError::Failed(format!("Lecture du corps échouée : {e}"))
        }
    })?;

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
    if IGNORED.iter().any(|ext| path.ends_with(ext)) || exclusion_reason(url.as_str()).is_some() {
        return None;
    }

    Some(url.to_string())
}

/// Vrai si l'URL est un point d'entrée web public autorisé au crawl.
pub fn is_crawlable_url(value: &str) -> bool {
    network::validate_url(value).is_ok() && exclusion_reason(value).is_none()
}

pub fn exclusion_reason(value: &str) -> Option<&'static str> {
    let url = Url::parse(value).ok()?;
    let host = url.host_str()?;
    if [
        "wikipedia.org",
        "wikimedia.org",
        "wiktionary.org",
        "wikisource.org",
        "wikibooks.org",
        "wikiquote.org",
        "wikivoyage.org",
        "wikidata.org",
    ]
    .iter()
    .any(|root| host == *root || host.ends_with(&format!(".{root}")))
        && url.query_pairs().any(|(key, value)| {
            (key == "redlink" && value == "1")
                || ((key == "action" || key == "veaction")
                    && matches!(
                        value.as_ref(),
                        "edit" | "submit" | "history" | "delete" | "protect" | "rollback" | "purge"
                    ))
        })
    {
        return Some("Interface Wikimedia hors contenu : edition, historique ou lien rouge");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{is_crawlable_url, normalize_url};

    #[test]
    fn excludes_wikimedia_edit_variants_without_dropping_article_parameters() {
        assert!(!is_crawlable_url(
            "https://en.wikipedia.org/w/index.php?title=Rust&action=edit"
        ));
        assert!(!is_crawlable_url(
            "https://en.wikipedia.org/w/index.php?title=Missing&redlink=1"
        ));
        assert!(is_crawlable_url(
            "https://en.wikipedia.org/w/index.php?title=Rust&action=view"
        ));
        assert!(is_crawlable_url("https://en.wikipedia.org/wiki/Rust"));
        assert!(is_crawlable_url("https://example.com/?action=edit"));
        assert!(is_crawlable_url(
            "https://wikipedia.org.example.com/?action=edit"
        ));
        assert!(
            normalize_url(
                "https://en.wikipedia.org/",
                "/w/index.php?title=Rust&action=edit"
            )
            .is_none()
        );
    }

    #[test]
    fn retry_after_accepts_seconds_dates_and_invalid_values() {
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(super::retry_delay(Some("120"), now), 120);
        let date = httpdate::fmt_http_date(now + std::time::Duration::from_secs(90));
        assert_eq!(super::retry_delay(Some(&date), now), 90);
        assert_eq!(super::retry_delay(Some("invalid"), now), 30);
        assert_eq!(super::retry_delay(Some("0"), now), 1);
    }

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

    #[test]
    fn preserves_directory_and_query_slashes() {
        assert_eq!(
            normalize_url("https://example.com/", "/docs/"),
            Some("https://example.com/docs/".into())
        );
        assert_eq!(
            normalize_url("https://example.com/", "/?next=/"),
            Some("https://example.com/?next=%2F".into())
        );
    }
}
