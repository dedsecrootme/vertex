//! Recherche plein texte (FTS5) et accès aux données de Vertex.
//!
//! Ce crate ne fait que lire/écrire dans la base SQLite via les helpers de
//! `crawler-rs::database` : recherche BM25, contenu de page pour la visionneuse,
//! signets et messages de contact.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crawler_rs::database;

pub use crawler_rs::database::{
    Bookmark, ContactMessage, DbStats, blocklist_path, default_db_dir, init_in, optimize,
};
pub use crawler_rs::safety;
pub use crawler_rs::runner;
pub use crawler_rs::SEEDS;

/// Un résultat de recherche.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub domain: String,
    /// Verdict de sûreté de l'URL (`safe`, `suspicious`, `malicious`, `unknown`).
    pub safety: String,
    /// Vrai si l'URL est déjà en signet.
    pub bookmarked: bool,
}

/// Une image documentée d'une page.
#[derive(Debug, Serialize)]
pub struct ImageRef {
    pub url: String,
    pub alt: Option<String>,
}

/// Un lien complet sortant d'une page.
#[derive(Debug, Serialize)]
pub struct LinkRef {
    pub url: String,
    pub anchor: Option<String>,
    pub is_external: bool,
    pub safety: String,
}

/// Contenu complet d'une page pour la visionneuse.
#[derive(Debug, Serialize)]
pub struct PageView {
    pub url: String,
    pub domain: String,
    /// HTML assaini : sans script, iframe, pub, tracker ni affiliation.
    pub content_html: String,
    /// Texte brut (secours si le HTML assaini est absent).
    pub content: String,
    pub title: String,
    pub description: Option<String>,
    pub language: Option<String>,
    pub favicon: Option<String>,
    pub status_code: Option<i64>,
    pub crawled_at: Option<String>,
    pub bookmarked: bool,
    pub safety: String,
    pub images: Vec<ImageRef>,
    pub links: Vec<LinkRef>,
}

/// Recherche plein texte (FTS5) classée par pertinence (BM25), avec pagination.
pub fn search(
    conn: &Connection,
    query: &str,
    offset: usize,
) -> Result<Vec<SearchResult>, rusqlite::Error> {
    let match_query = fts_query(query);
    if match_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT p.url,
                p.title,
                snippet(pages_fts, 2, '', '', '…', 12) AS snip,
                COALESCE(p.domain, '') AS domain,
                COALESCE((SELECT u.safety FROM urls u WHERE u.url = p.url), 'unknown') AS safety,
                EXISTS(SELECT 1 FROM bookmarks b WHERE b.url = p.url) AS bookmarked
         FROM pages_fts
         JOIN pages p ON p.id = pages_fts.rowid
         WHERE pages_fts MATCH ?1
         ORDER BY rank
         LIMIT 30 OFFSET ?2",
    )?;

    let rows = stmt.query_map(rusqlite::params![&match_query, offset as i64], |row| {
        Ok(SearchResult {
            url: row.get(0)?,
            title: row.get(1)?,
            snippet: row.get(2)?,
            domain: row.get(3)?,
            safety: row.get(4)?,
            bookmarked: row.get::<_, i64>(5)? != 0,
        })
    })?;

    rows.collect()
}

/// Récupère le contenu complet (assaini) d'une page, ses images et ses liens.
pub fn get_page(conn: &Connection, url: &str) -> Result<Option<PageView>, rusqlite::Error> {
    let row = conn
        .query_row(
            "SELECT id, url, COALESCE(domain,''), COALESCE(title,''), description,
                    COALESCE(content,''), COALESCE(content_html,''), language, favicon,
                    status_code, crawled_at,
                    COALESCE((SELECT u.safety FROM urls u WHERE u.url = pages.url), 'unknown'),
                    EXISTS(SELECT 1 FROM bookmarks b WHERE b.url = pages.url)
             FROM pages WHERE url = ?1",
            [url],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    PageView {
                        url: row.get(1)?,
                        domain: row.get(2)?,
                        title: row.get(3)?,
                        description: row.get(4)?,
                        content: row.get(5)?,
                        content_html: row.get(6)?,
                        language: row.get(7)?,
                        favicon: row.get(8)?,
                        status_code: row.get(9)?,
                        crawled_at: row.get(10)?,
                        safety: row.get(11)?,
                        bookmarked: row.get::<_, i64>(12)? != 0,
                        images: Vec::new(),
                        links: Vec::new(),
                    },
                ))
            },
        )
        .optional()?;

    let Some((id, mut page)) = row else {
        return Ok(None);
    };

    page.images = database::images_of_page(conn, id)?
        .into_iter()
        .map(|i| ImageRef {
            url: i.url,
            alt: i.alt,
        })
        .collect();

    page.links = database::links_of_page(conn, id)?
        .into_iter()
        .map(|l| LinkRef {
            url: l.url,
            anchor: l.anchor,
            is_external: l.is_external,
            safety: l.safety,
        })
        .collect();

    Ok(Some(page))
}

// --- Signets -------------------------------------------------------------

/// Liste les signets.
pub fn bookmarks(conn: &Connection) -> Result<Vec<Bookmark>, rusqlite::Error> {
    database::bookmarks_list(conn)
}

/// Ajoute un signet.
pub fn add_bookmark(
    conn: &Connection,
    url: &str,
    title: Option<&str>,
    note: Option<&str>,
    tags: Option<&str>,
) -> Result<(), rusqlite::Error> {
    database::bookmark_add(conn, url, title, note, tags)
}

/// Supprime un signet.
pub fn remove_bookmark(conn: &Connection, url: &str) -> Result<bool, rusqlite::Error> {
    database::bookmark_remove(conn, url)
}

// --- Contact administrateur ---------------------------------------------

/// Enregistre un message destiné à l'administrateur.
pub fn add_contact(
    conn: &Connection,
    name: Option<&str>,
    email: Option<&str>,
    subject: Option<&str>,
    message: &str,
    page_url: Option<&str>,
) -> Result<i64, rusqlite::Error> {
    database::contact_add(conn, name, email, subject, message, page_url)
}

/// Liste les messages reçus.
pub fn messages(conn: &Connection) -> Result<Vec<ContactMessage>, rusqlite::Error> {
    database::contact_list(conn)
}

/// Marque un message comme lu.
pub fn mark_message_read(conn: &Connection, id: i64) -> Result<(), rusqlite::Error> {
    database::contact_mark_read(conn, id)
}

/// Statistiques globales de la base.
pub fn stats(conn: &Connection) -> Result<DbStats, rusqlite::Error> {
    database::stats(conn)
}

/// Vérifie la sûreté d'une URL (100 % local).
pub fn safety(url: &str) -> crawler_rs::safety::SafetyReport {
    crawler_rs::safety::check_url(url)
}

/// Construit une requête MATCH FTS5 sûre (chaque terme en phrase, combinés par OR).
fn fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        return String::new();
    }
    terms.join(" OR ")
}
