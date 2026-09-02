use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct PageContent {
    pub url: String,
    pub title: String,
    pub content: String,
    pub status_code: Option<i64>,
    pub crawled_at: Option<String>,
}

/// Recherche plein texte (FTS5) avec classement par pertinence (BM25).
/// `offset` permet la pagination.
pub fn search(conn: &Connection, query: &str, offset: usize) -> Result<Vec<SearchResult>, rusqlite::Error> {
    let match_query = fts_query(query);
    if match_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT p.url, p.title,
                snippet(pages_fts, 2, '', '', '…', 12) AS snip
         FROM pages_fts
         JOIN pages p ON p.id = pages_fts.rowid
         WHERE pages_fts MATCH ?1
         ORDER BY rank
         LIMIT 30 OFFSET ?2",
    )?;

    let rows = stmt.query_map(params![&match_query, offset as i64], |row| {
        Ok(SearchResult {
            url: row.get(0)?,
            title: row.get(1)?,
            snippet: row.get(2)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Récupère le contenu complet d'une page pour l'afficher dans le navigateur.
pub fn get_page(conn: &Connection, url: &str) -> Result<Option<PageContent>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT url, title, content, status_code, crawled_at
         FROM pages WHERE url = ?1",
    )?;
    let mut rows = stmt.query([url])?;

    if let Some(row) = rows.next()? {
        Ok(Some(PageContent {
            url: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2).unwrap_or_default(),
            status_code: row.get(3)?,
            crawled_at: row.get(4)?,
        }))
    } else {
        Ok(None)
    }
}

/// Construit une requête MATCH FTS5 sûre : chaque terme est passé en phrase
/// (entre guillemets) et combiné par OR, puis classé par pertinence.
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