use crate::models::Page;
use crate::parser;

use reqwest::blocking::Client;
use reqwest::header::USER_AGENT;

use sha2::{Digest, Sha256};

use std::error::Error;
use std::time::Instant;

use url::Url;

const MAX_CONTENT_SIZE: usize = 5_000_000;

pub struct CrawlResult {
    pub page: Page,
    pub http_code: u16,
}

pub fn crawl(client: &Client, url: &str) -> Result<CrawlResult, Box<dyn Error>> {
    println!("📡 Téléchargement : {url}");

    let start = Instant::now();

    let response = client
        .get(url)
        .header(USER_AGENT, format!("VertexCrawler/{}", env!("CARGO_PKG_VERSION")))
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
        if !ct.contains("text/html") && !ct.contains("application/xhtml") {
            return Err(format!("Type non supporté : {ct}").into());
        }
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

    url.set_fragment(None);

    let path = url.path().to_lowercase();

    let ignored = [
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp",
        ".zip", ".rar", ".7z",
        ".exe", ".pdf",
        ".mp4", ".mp3",
    ];

    for ext in ignored {
        if path.ends_with(ext) {
            return None;
        }
    }

    Some(url.to_string().trim_end_matches('/').to_string())
}