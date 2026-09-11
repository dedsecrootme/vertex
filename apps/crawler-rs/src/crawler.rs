use crate::models::Page;
use crate::parser;

use reqwest::blocking::Client;
use reqwest::header::USER_AGENT;

use sha2::{Digest, Sha256};

use std::error::Error;
use std::net::Ipv4Addr;
use std::time::Instant;

use url::Url;

const MAX_CONTENT_SIZE: usize = 5_000_000;

pub struct CrawlResult {
    pub page: Page,
    pub http_code: u16,
}

pub fn crawl(client: &Client, url: &str) -> Result<CrawlResult, Box<dyn Error>> {
    if !is_crawlable_url(url) {
        return Err("URL non autorisée pour le crawl".into());
    }

    println!("📡 Téléchargement : {url}");

    let start = Instant::now();

    let response = client
        .get(url)
        .header(
            USER_AGENT,
            format!("VertexCrawler/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()?;

    let status = response.status();
    let http_code = status.as_u16();

    if !status.is_success() {
        return Err(format!("HTTP erreur {status}").into());
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());

    if let Some(ref ct) = content_type {
        let content_type = ct.to_ascii_lowercase();
        if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
            return Err(format!("Type non supporté : {ct}").into());
        }
    }

    if let Some(content_length) = response.content_length()
        && content_length > MAX_CONTENT_SIZE as u64
    {
        return Err("Page trop volumineuse".into());
    }

    let bytes = response.bytes()?;

    if bytes.len() > MAX_CONTENT_SIZE {
        return Err("Page trop volumineuse".into());
    }

    let size = bytes.len();

    let html = String::from_utf8_lossy(&bytes).to_string();

    let title = parser::extract_title(&html);
    let links = parser::extract_links(&html);
    let content = parser::extract_text(&html);

    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    let elapsed = start.elapsed().as_millis();

    let page = Page {
        url: url.to_string(),
        status_code: http_code,
        title,
        description: None,
        content,
        links: links.clone(),
        depth: 0,
        hash,
        size,
        content_type,
        language: None,
        crawl_time_ms: elapsed,
    };

    Ok(CrawlResult { page, http_code })
}

/// Extrait le nom de domaine d'une URL (utilisé pour les statistiques).
pub fn extract_domain(url: &str) -> String {
    match Url::parse(url) {
        Ok(parsed) => parsed.domain().unwrap_or("").to_string(),
        Err(_) => String::new(),
    }
}

/// Normalise un lien détecté dans une page en URL absolue propre.
pub fn normalize_url(base: &str, link: &str) -> Option<String> {
    let link = link.trim();

    if link.is_empty() {
        return None;
    }

    if link.starts_with('#')
        || link.starts_with("javascript:")
        || link.starts_with("mailto:")
        || link.starts_with("tel:")
    {
        return None;
    }

    let base = Url::parse(base).ok()?;
    let mut url = base.join(link).ok()?;

    if !is_crawlable(&url) {
        return None;
    }

    url.set_fragment(None);
    let retained_query_pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            !key.starts_with("utm_") && key != "fbclid" && key != "gclid"
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !retained_query_pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(retained_query_pairs);
    }

    let path = url.path().to_lowercase();

    let ignored = [
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp", ".zip", ".rar", ".7z", ".exe",
        ".pdf", ".mp4", ".mp3",
    ];

    for ext in ignored {
        if path.ends_with(ext) {
            return None;
        }
    }

    Some(url.to_string().trim_end_matches('/').to_string())
}

/// Accepte uniquement les URL web publiques afin qu'une page distante ne
/// puisse pas faire explorer les services locaux de la machine.
pub fn is_crawlable_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| is_crawlable(&url))
}

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
                || ip.is_multicast())
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
}
