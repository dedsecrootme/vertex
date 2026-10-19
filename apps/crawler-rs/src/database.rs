//! Système de base de données de Vertex (SQLite + FTS5).
//!
//! Un seul endroit gère : la file d'URL, les pages, les liens complets, les
//! images, la sûreté des liens, les signets, les messages de contact et les
//! exécutions de crawl. Le schéma est versionné et migré automatiquement.

use rusqlite::{Connection, OptionalExtension, Result};

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::models::Page;

/// Version courante du schéma (stockée dans la table `meta`).
pub const SCHEMA_VERSION: i64 = 6;

/// Racine du workspace Vertex.
pub fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

/// Dossier SQLite utilisé par défaut (`<workspace>/database`).
pub fn default_db_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VERTEX_DB_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    workspace_root().join("database")
}

/// Chemin du fichier de blocklist utilisateur.
pub fn blocklist_path(db_dir: &Path) -> PathBuf {
    db_dir.join("blocklist.txt")
}

/// Schéma principal : métadonnées, file d'URL et pages.
const SCHEMA_MAIN: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS urls (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    url         TEXT UNIQUE NOT NULL,
    domain      TEXT,
    status      TEXT NOT NULL DEFAULT 'pending',
    http_code   INTEGER,
    depth       INTEGER NOT NULL DEFAULT 0,
    priority    INTEGER NOT NULL DEFAULT 0,
    attempts    INTEGER NOT NULL DEFAULT 0,
    next_attempt INTEGER NOT NULL DEFAULT 0,
    error       TEXT,
    safety      TEXT NOT NULL DEFAULT 'unknown',
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS pages (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    url           TEXT UNIQUE NOT NULL,
    domain        TEXT,
    title         TEXT,
    description   TEXT,
    content       TEXT DEFAULT '',
    content_html  TEXT DEFAULT '',
    content_hash  TEXT,
    status_code   INTEGER,
    content_type  TEXT,
    language      TEXT,
    size          INTEGER DEFAULT 0,
    links_count   INTEGER DEFAULT 0,
    images_count  INTEGER DEFAULT 0,
    favicon       TEXT,
    crawled_at    DATETIME DEFAULT CURRENT_TIMESTAMP
);
"#;

/// Schéma secondaire : liens, images, signets, contact, sûreté, exécutions.
const SCHEMA_AUX: &str = r#"
CREATE TABLE IF NOT EXISTS links (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id     INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    url         TEXT NOT NULL,
    anchor      TEXT,
    is_external INTEGER NOT NULL DEFAULT 0,
    safety      TEXT NOT NULL DEFAULT 'unknown'
);
CREATE TABLE IF NOT EXISTS images (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id  INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    url      TEXT NOT NULL,
    alt      TEXT,
    UNIQUE(page_id, url)
);
CREATE TABLE IF NOT EXISTS bookmarks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    url        TEXT UNIQUE NOT NULL,
    title      TEXT,
    note       TEXT,
    tags       TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS contact_messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT,
    email      TEXT,
    subject    TEXT,
    message    TEXT NOT NULL,
    page_url   TEXT,
    is_read    INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS url_safety (
    url        TEXT PRIMARY KEY,
    verdict    TEXT NOT NULL,
    score      INTEGER NOT NULL DEFAULT 0,
    reasons    TEXT,
    checked_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS crawl_runs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at    DATETIME DEFAULT CURRENT_TIMESTAMP,
    finished_at   DATETIME,
    duration_secs INTEGER,
    pages_ok      INTEGER DEFAULT 0,
    pages_failed  INTEGER DEFAULT 0,
    pages_total   INTEGER DEFAULT 0,
    status        TEXT DEFAULT 'running'
);

CREATE TABLE IF NOT EXISTS errors (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    url        TEXT,
    error      TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
"#;

/// Index créés **après** les migrations (les colonnes existent alors forcément).
const SCHEMA_INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_urls_status   ON urls(status);
CREATE INDEX IF NOT EXISTS idx_urls_domain   ON urls(domain);
CREATE INDEX IF NOT EXISTS idx_urls_safety   ON urls(safety);
CREATE INDEX IF NOT EXISTS idx_urls_priority ON urls(priority, id);
CREATE INDEX IF NOT EXISTS idx_pages_url     ON pages(url);
CREATE INDEX IF NOT EXISTS idx_pages_domain  ON pages(domain);
CREATE INDEX IF NOT EXISTS idx_pages_hash    ON pages(content_hash);
CREATE INDEX IF NOT EXISTS idx_links_page    ON links(page_id);
CREATE INDEX IF NOT EXISTS idx_links_url     ON links(url);
CREATE INDEX IF NOT EXISTS idx_images_page   ON images(page_id);
CREATE INDEX IF NOT EXISTS idx_images_url    ON images(url);
CREATE INDEX IF NOT EXISTS idx_pages_favicon ON pages(favicon);
CREATE INDEX IF NOT EXISTS idx_urls_schedule ON urls(status, next_attempt, depth, priority);
CREATE INDEX IF NOT EXISTS idx_urls_claim ON urls(priority DESC, id ASC)
WHERE status='pending' AND safety!='malicious' AND attempts<3;
CREATE INDEX IF NOT EXISTS idx_urls_retry_exhausted ON urls(id)
WHERE status='pending' AND attempts>=3;
"#;

/// Initialise la base par défaut.
pub fn init() -> Result<Connection> {
    init_in(&default_db_dir())
}

/// Ouvre/crée la base et applique le schéma + les migrations.
pub fn init_in(db_dir: &Path) -> Result<Connection> {
    fs::create_dir_all(db_dir).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    let conn = Connection::open(db_dir.join("vertex.db"))?;
    conn.busy_timeout(Duration::from_secs(30))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         PRAGMA cache_size = -32768;
         PRAGMA journal_size_limit = 67108864;
         PRAGMA analysis_limit = 1000;",
    )?;
    conn.execute_batch(SCHEMA_MAIN)?;
    conn.execute_batch(SCHEMA_AUX)?;

    migrate(&conn)?;
    conn.execute_batch(SCHEMA_INDEXES)?;
    let previously_verified: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='schema_version' AND CAST(value AS INTEGER)>=5) AND EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='pages_fts')",
        [], |r| r.get(0))?;
    if previously_verified {
        install_fts(&conn)?;
    } else {
        ensure_fts(&conn)?;
    }
    crate::monitor::install(&conn)?;

    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [SCHEMA_VERSION.to_string()],
    )?;

    Ok(conn)
}

/// Ajoute une colonne si elle n'existe pas encore (migration douce).
fn ensure_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        (table, column),
        |row| row.get(0),
    )?;
    if exists == 0 {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            [],
        )?;
    }
    Ok(())
}

/// Migrations : aligne une base existante (v1) sur le schéma courant.
fn migrate(conn: &Connection) -> Result<()> {
    for (col, decl) in [
        ("domain", "TEXT"),
        ("depth", "INTEGER NOT NULL DEFAULT 0"),
        ("priority", "INTEGER NOT NULL DEFAULT 0"),
        ("attempts", "INTEGER NOT NULL DEFAULT 0"),
        ("next_attempt", "INTEGER NOT NULL DEFAULT 0"),
        ("error", "TEXT"),
        ("safety", "TEXT NOT NULL DEFAULT 'unknown'"),
    ] {
        ensure_column(conn, "urls", col, decl)?;
    }

    for (col, decl) in [
        ("domain", "TEXT"),
        ("content", "TEXT DEFAULT ''"),
        ("content_html", "TEXT DEFAULT ''"),
        ("content_type", "TEXT"),
        ("language", "TEXT"),
        ("size", "INTEGER DEFAULT 0"),
        ("images_count", "INTEGER DEFAULT 0"),
        ("favicon", "TEXT"),
    ] {
        ensure_column(conn, "pages", col, decl)?;
    }

    Ok(())
}

/// Crée et maintient l'index FTS5 (reconstruit s'il est désynchronisé).
fn install_fts(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
            url UNINDEXED,
            title,
            content,
            content='pages',
            content_rowid='id'
         );

         CREATE TRIGGER IF NOT EXISTS pages_fts_ai AFTER INSERT ON pages BEGIN
            INSERT INTO pages_fts(rowid, url, title, content)
            VALUES (new.id, new.url, new.title, new.content);
         END;

         CREATE TRIGGER IF NOT EXISTS pages_fts_ad AFTER DELETE ON pages BEGIN
            INSERT INTO pages_fts(pages_fts, rowid, url, title, content)
            VALUES ('delete', old.id, old.url, old.title, old.content);
         END;

         CREATE TRIGGER IF NOT EXISTS pages_fts_au AFTER UPDATE ON pages BEGIN
            INSERT INTO pages_fts(pages_fts, rowid, url, title, content)
            VALUES ('delete', old.id, old.url, old.title, old.content);
            INSERT INTO pages_fts(rowid, url, title, content)
            VALUES (new.id, new.url, new.title, new.content);
         END;",
    )?;

    Ok(())
}

pub fn ensure_fts(conn: &Connection) -> Result<()> {
    install_fts(conn)?;
    // COUNT(*) on an external-content FTS table reads pages, not the index.
    if let Err(error) = conn.execute(
        "INSERT INTO pages_fts(pages_fts, rank) VALUES ('integrity-check', 1)",
        [],
    ) {
        if matches!(&error, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::DatabaseCorrupt)
        {
            conn.execute("INSERT INTO pages_fts(pages_fts) VALUES ('rebuild')", [])?;
            conn.execute(
                "INSERT INTO pages_fts(pages_fts, rank) VALUES ('integrity-check', 1)",
                [],
            )?;
        } else {
            return Err(error);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// File d'URL
// ---------------------------------------------------------------------------

/// Extrait le domaine d'une URL (ou chaîne vide).
fn domain_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
}

/// Ajoute une URL à la file (domaine et sûreté calculés automatiquement).
pub fn enqueue(conn: &Connection, url: &str, depth: i64, priority: i64) -> Result<bool> {
    let safety = crate::safety::check_url(url).verdict.as_str();
    let domain = domain_of(url);
    let changed = conn.execute(
        "INSERT OR IGNORE INTO urls(url, domain, status, depth, priority, safety)
         VALUES (?1, ?2, 'pending', ?3, ?4, ?5)",
        rusqlite::params![url, domain, depth, priority, safety],
    )?;
    Ok(changed > 0)
}

/// Compatibilité : ajoute une URL racine (profondeur 0, priorité normale).
pub fn add_url(conn: &Connection, url: &str) -> Result<()> {
    enqueue(conn, url, 0, 0).map(|_| ())
}

/// Shared discovery never overwrites content or exposes private client tables.
pub fn request_indexing(conn: &Connection, url: &str) -> Result<&'static str> {
    let tx = conn.unchecked_transaction()?;
    let fresh: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pages WHERE url=?1 AND crawled_at >= datetime('now','-1 day'))",
        [url], |r| r.get(0),
    )?;
    if fresh {
        return Ok("ready");
    }
    let existing: Option<(String, bool)> = tx
        .query_row(
            "SELECT status, updated_at > datetime('now','-1 day') FROM urls WHERE url=?1",
            [url],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match existing
        .as_ref()
        .map(|(status, recent)| (status.as_str(), *recent))
    {
        Some(("crawling", _)) => return Ok("processing"),
        Some(("pending", _)) => return Ok("queued"),
        Some(("blocked" | "skipped", _)) => return Ok("blocked"),
        Some(("failed", true)) => return Ok("deferred"),
        _ => {}
    }
    let pending: i64 = tx.query_row(
        "SELECT count(*) FROM urls WHERE status IN ('pending','crawling')",
        [],
        |r| r.get(0),
    )?;
    if pending >= 100_000 {
        return Ok("full");
    }
    if existing.is_some() {
        tx.execute("UPDATE urls SET status='pending', attempts=0, next_attempt=0, depth=0, priority=20, error=NULL, updated_at=CURRENT_TIMESTAMP WHERE url=?1", [url])?;
    } else {
        enqueue(&tx, url, 0, 20)?;
    }
    tx.commit()?;
    Ok("queued")
}

/// Only successful, stale pages are scheduled again; failed sites keep a cooldown.
pub fn schedule_stale(conn: &Connection, age_secs: u64, limit: usize) -> Result<usize> {
    conn.execute(
        "UPDATE urls SET status='pending', attempts=0, next_attempt=0, depth=0, updated_at=CURRENT_TIMESTAMP
         WHERE id IN (SELECT u.id FROM urls u JOIN pages p ON p.url=u.url
         WHERE u.status='done' AND p.crawled_at < datetime('now', ?1)
         ORDER BY p.crawled_at LIMIT ?2)",
        rusqlite::params![format!("-{} seconds", age_secs.max(3600)), limit.min(1000) as i64],
    )
}

/// Vrai si l'URL est déjà connue.
pub fn url_exists(conn: &Connection, url: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM urls WHERE url = ?1)",
        [url],
        |row| row.get(0),
    )
}

/// Prochaine URL à traiter (priorité décroissante, puis ordre d'insertion).
/// Les URL marquées « malveillantes » sont ignorées.
pub fn get_next_url(conn: &Connection) -> Result<Option<String>> {
    conn.query_row(
        "SELECT url FROM urls
         WHERE status = 'pending' AND safety != 'malicious'
         ORDER BY priority DESC, id ASC
         LIMIT 1",
        [],
        |row| row.get(0),
    )
    .optional()
}

/// Marque une URL comme en cours de traitement.
pub fn mark_crawling(conn: &Connection, url: &str) -> Result<()> {
    conn.execute(
        "UPDATE urls SET status = 'crawling', attempts = attempts + 1,
         updated_at = CURRENT_TIMESTAMP WHERE url = ?1",
        [url],
    )?;
    Ok(())
}

/// Marque une URL comme traitée avec succès.
pub fn mark_crawled(conn: &Connection, url: &str, http_code: u16) -> Result<()> {
    conn.execute(
        "UPDATE urls SET status = 'done', http_code = ?2, error = NULL,
         updated_at = CURRENT_TIMESTAMP WHERE url = ?1",
        rusqlite::params![url, http_code],
    )?;
    Ok(())
}

/// Marque une URL comme échouée (et journalise l'erreur).
pub fn mark_failed(conn: &Connection, url: &str, error: &str) -> Result<()> {
    conn.execute(
        "UPDATE urls SET status = 'failed', error = ?2,
         updated_at = CURRENT_TIMESTAMP WHERE url = ?1",
        rusqlite::params![url, error],
    )?;
    save_error(conn, url, error)?;
    Ok(())
}

/// Marque une URL comme bloquée (malveillante : jamais visitée).
pub fn mark_blocked(conn: &Connection, url: &str, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE urls SET status = 'blocked', safety = 'malicious', error = ?2,
         updated_at = CURRENT_TIMESTAMP WHERE url = ?1",
        rusqlite::params![url, reason],
    )?;
    Ok(())
}

/// Marque une URL comme ignorée (type non supporté, hors profondeur…).
pub fn mark_skipped(conn: &Connection, url: &str, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE urls SET status = 'skipped', error = ?2,
         updated_at = CURRENT_TIMESTAMP WHERE url = ?1",
        rusqlite::params![url, reason],
    )?;
    Ok(())
}

/// Remet les URL bloquées en « crawling » dans la file (reprise après arrêt).
pub fn reset_crawling(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE urls SET status = 'pending', updated_at = CURRENT_TIMESTAMP
         WHERE status = 'crawling'",
        [],
    )?;
    Ok(())
}

/// (Re)programme des URL échouées pour une nouvelle tentative.
pub fn requeue_failed(conn: &Connection) -> Result<usize> {
    let n = conn.execute(
        "UPDATE urls SET status = 'pending', attempts = 0, next_attempt = 0, updated_at = CURRENT_TIMESTAMP
         WHERE status = 'failed'",
        [],
    )?;
    Ok(n)
}

/// Enregistre le verdict de sûreté d'une URL (cache).
pub fn set_url_safety(
    conn: &Connection,
    url: &str,
    verdict: &str,
    score: i64,
    reasons: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO url_safety(url, verdict, score, reasons) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(url) DO UPDATE SET
             verdict = excluded.verdict,
             score = excluded.score,
             reasons = excluded.reasons,
             checked_at = CURRENT_TIMESTAMP",
        rusqlite::params![url, verdict, score, reasons],
    )?;
    Ok(())
}

/// Nombre d'URL selon leur statut.
pub fn count_urls_by_status(conn: &Connection, status: &str) -> Result<i64> {
    crate::monitor::count(conn, &format!("urls:{status}"))
}

/// Nombre total d'URL connues.
pub fn count_urls(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT coalesce(sum(value),0) FROM monitor_counts WHERE key LIKE 'urls:%'",
        [],
        |r| r.get(0),
    )
}

/// Remplit la colonne `domain` des URL créées avant la migration v2.
/// Traite au plus `limit` lignes par appel pour ne pas bloquer indéfiniment.
pub fn backfill_domains(conn: &Connection, limit: usize) -> Result<usize> {
    let rows: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT id, url FROM urls WHERE domain IS NULL LIMIT ?1")?;
        stmt.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(std::result::Result::ok)
            .collect()
    };

    if rows.is_empty() {
        return Ok(0);
    }

    // Transaction unique : indispensable pour rester rapide sur de gros volumes.
    let tx = conn.unchecked_transaction()?;
    let mut updated = 0usize;
    for (id, url) in rows {
        let domain = domain_of(&url);
        tx.execute(
            "UPDATE urls SET domain = ?2 WHERE id = ?1",
            rusqlite::params![id, domain],
        )?;
        if !domain.is_empty() {
            updated += 1;
        }
    }
    tx.commit()?;
    Ok(updated)
}

/// Nombre de domaines distincts connus.
pub fn count_domains(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM monitor_domains WHERE refs>0",
        [],
        |row| row.get(0),
    )
}

/// Toutes les URL connues.
pub fn all_urls(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT url FROM urls")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut urls = Vec::new();
    for row in rows {
        urls.push(row?);
    }
    Ok(urls)
}

/// Enregistre une erreur de crawl.
pub fn save_error(conn: &Connection, url: &str, error: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO errors(url, error) VALUES (?1, ?2)",
        rusqlite::params![url, error],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pages, liens complets et images
// ---------------------------------------------------------------------------

/// Identifiant d'une page à partir de son URL.
pub fn page_id(conn: &Connection, url: &str) -> Result<Option<i64>> {
    conn.query_row("SELECT id FROM pages WHERE url = ?1", [url], |row| {
        row.get(0)
    })
    .optional()
}

/// Enregistre (ou met à jour) une page, ses liens complets et ses images.
/// Renvoie l'identifiant interne de la page.
pub fn save_page(conn: &Connection, page: &Page) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let id = save_page_inner(&tx, page)?;
    tx.commit()?;
    Ok(id)
}

/// Commits the page, FTS update and completed queue state together.
pub fn save_crawled_page(conn: &Connection, page: &Page, http_code: u16) -> Result<i64> {
    retry_busy(|| {
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
        let id = save_page_inner(&tx, page)?;
        mark_crawled(&tx, &page.url, http_code)?;
        tx.commit()?;
        Ok(id)
    })
}

// Retry only complete atomic writes, never an arbitrary partially completed operation.
pub(crate) fn retry_busy<T>(mut operation: impl FnMut() -> Result<T>) -> Result<T> {
    for attempt in 0..4 {
        match operation() {
            Err(rusqlite::Error::SqliteFailure(code, _))
                if attempt < 3
                    && matches!(
                        code.code,
                        rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                    ) =>
            {
                std::thread::sleep(Duration::from_millis(25 * (attempt + 1)));
            }
            result => return result,
        }
    }
    unreachable!()
}

fn save_page_inner(conn: &Connection, page: &Page) -> Result<i64> {
    conn.execute(
        "INSERT INTO pages (
            url, domain, title, description, content, content_html, content_hash,
            status_code, content_type, language, size, links_count, images_count,
            favicon, crawled_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14, CURRENT_TIMESTAMP)
         ON CONFLICT(url) DO UPDATE SET
            domain = excluded.domain,
            title = excluded.title,
            description = excluded.description,
            content = excluded.content,
            content_html = excluded.content_html,
            content_hash = excluded.content_hash,
            status_code = excluded.status_code,
            content_type = excluded.content_type,
            language = excluded.language,
            size = excluded.size,
            links_count = excluded.links_count,
            images_count = excluded.images_count,
            favicon = excluded.favicon,
            crawled_at = CURRENT_TIMESTAMP",
        rusqlite::params![
            page.url,
            page.domain,
            page.title,
            page.description,
            page.content,
            page.content_html,
            page.content_hash,
            page.status_code,
            page.content_type,
            page.language,
            page.size as i64,
            page.links.len() as i64,
            page.images.len() as i64,
            page.favicon,
        ],
    )?;

    let id: i64 = conn.query_row("SELECT id FROM pages WHERE url = ?1", [&page.url], |r| {
        r.get(0)
    })?;
    conn.execute("DELETE FROM links WHERE page_id = ?1", [id])?;
    conn.execute("DELETE FROM images WHERE page_id = ?1", [id])?;

    let mut insert_link = conn.prepare_cached(
        "INSERT OR IGNORE INTO links(page_id, url, anchor, is_external, safety) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for link in &page.links {
        insert_link.execute(rusqlite::params![
            id,
            link.url,
            link.anchor,
            link.is_external as i64,
            link.safety
        ])?;
    }

    let mut insert_image =
        conn.prepare_cached("INSERT OR IGNORE INTO images(page_id, url, alt) VALUES (?1, ?2, ?3)")?;
    for image in &page.images {
        insert_image.execute(rusqlite::params![id, image.url, image.alt])?;
    }

    Ok(id)
}

/// Liens complets sortants d'une page.
pub fn links_of_page(conn: &Connection, id: i64) -> Result<Vec<crate::models::PageLink>> {
    let mut stmt = conn.prepare(
        "SELECT url, anchor, is_external, safety FROM links WHERE page_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([id], |row| {
        Ok(crate::models::PageLink {
            url: row.get(0)?,
            anchor: row.get(1)?,
            is_external: row.get::<_, i64>(2)? != 0,
            safety: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Images d'une page.
pub fn images_of_page(conn: &Connection, id: i64) -> Result<Vec<crate::models::PageImage>> {
    let mut stmt = conn.prepare("SELECT url, alt FROM images WHERE page_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([id], |row| {
        Ok(crate::models::PageImage {
            url: row.get(0)?,
            alt: row.get(1)?,
        })
    })?;
    rows.collect()
}

/// Nombre de pages indexées.
pub fn count_pages(conn: &Connection) -> Result<i64> {
    crate::monitor::count(conn, "pages")
}

// ---------------------------------------------------------------------------
// Signets
// ---------------------------------------------------------------------------

/// Un signet enregistré par l'utilisateur.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Bookmark {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub note: Option<String>,
    pub tags: Option<String>,
    pub created_at: Option<String>,
}

/// Ajoute (ou met à jour) un signet.
pub fn bookmark_add(
    conn: &Connection,
    url: &str,
    title: Option<&str>,
    note: Option<&str>,
    tags: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO bookmarks(url, title, note, tags) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(url) DO UPDATE SET
            title = excluded.title,
            note = excluded.note,
            tags = excluded.tags",
        rusqlite::params![url, title, note, tags],
    )?;
    Ok(())
}

/// Supprime un signet. Renvoie vrai si une ligne a été supprimée.
pub fn bookmark_remove(conn: &Connection, url: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM bookmarks WHERE url = ?1", [url])? > 0)
}

/// Vrai si l'URL est déjà en signet.
pub fn is_bookmarked(conn: &Connection, url: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM bookmarks WHERE url = ?1)",
        [url],
        |row| row.get(0),
    )
}

/// Liste des signets (du plus récent au plus ancien).
pub fn bookmarks_list(conn: &Connection) -> Result<Vec<Bookmark>> {
    let mut stmt = conn
        .prepare("SELECT id, url, title, note, tags, created_at FROM bookmarks ORDER BY id DESC")?;
    let rows = stmt.query_map([], |row| {
        Ok(Bookmark {
            id: row.get(0)?,
            url: row.get(1)?,
            title: row.get(2)?,
            note: row.get(3)?,
            tags: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// Nombre de signets.
pub fn count_bookmarks(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM bookmarks", [], |row| row.get(0))
}

// ---------------------------------------------------------------------------
// Contact administrateur
// ---------------------------------------------------------------------------

/// Message envoyé à l'administrateur.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContactMessage {
    pub id: i64,
    pub name: Option<String>,
    pub email: Option<String>,
    pub subject: Option<String>,
    pub message: String,
    pub page_url: Option<String>,
    pub is_read: bool,
    pub created_at: Option<String>,
}

/// Enregistre un message de contact.
pub fn contact_add(
    conn: &Connection,
    name: Option<&str>,
    email: Option<&str>,
    subject: Option<&str>,
    message: &str,
    page_url: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO contact_messages(name, email, subject, message, page_url)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![name, email, subject, message, page_url],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Liste des messages reçus.
pub fn contact_list(conn: &Connection) -> Result<Vec<ContactMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, email, subject, message, page_url, is_read, created_at
         FROM contact_messages ORDER BY id DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ContactMessage {
            id: row.get(0)?,
            name: row.get(1)?,
            email: row.get(2)?,
            subject: row.get(3)?,
            message: row.get(4)?,
            page_url: row.get(5)?,
            is_read: row.get::<_, i64>(6)? != 0,
            created_at: row.get(7)?,
        })
    })?;
    rows.collect()
}

/// Marque un message comme lu.
pub fn contact_mark_read(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE contact_messages SET is_read = 1 WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

/// Nombre de messages non lus.
pub fn count_unread_messages(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM contact_messages WHERE is_read = 0",
        [],
        |row| row.get(0),
    )
}

// ---------------------------------------------------------------------------
// Exécutions de crawl et statistiques
// ---------------------------------------------------------------------------

/// Démarre une exécution de crawl et renvoie son identifiant.
pub fn run_start(conn: &Connection, duration_secs: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO crawl_runs(duration_secs, status) VALUES (?1, 'running')",
        [duration_secs],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Clôture une exécution de crawl.
pub fn run_finish(
    conn: &Connection,
    run_id: i64,
    pages_ok: i64,
    pages_failed: i64,
    pages_total: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE crawl_runs SET finished_at = CURRENT_TIMESTAMP, status = 'done',
         pages_ok = ?2, pages_failed = ?3, pages_total = ?4 WHERE id = ?1",
        rusqlite::params![run_id, pages_ok, pages_failed, pages_total],
    )?;
    Ok(())
}

/// Vue d'ensemble de la base.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DbStats {
    pub pages: i64,
    pub done: i64,
    pub failed: i64,
    pub pending: i64,
    pub crawling: i64,
    pub blocked: i64,
    pub domains: i64,
    pub bookmarks: i64,
    pub messages: i64,
    pub unread: i64,
}

/// Statistiques globales.
pub fn stats(conn: &Connection) -> Result<DbStats> {
    Ok(DbStats {
        pages: count_pages(conn)?,
        done: count_urls_by_status(conn, "done")?,
        failed: count_urls_by_status(conn, "failed")?,
        pending: count_urls_by_status(conn, "pending")?,
        crawling: count_urls_by_status(conn, "crawling")?,
        blocked: count_urls_by_status(conn, "blocked")?,
        domains: count_domains(conn)?,
        bookmarks: count_bookmarks(conn)?,
        messages: conn.query_row("SELECT COUNT(*) FROM contact_messages", [], |r| r.get(0))?,
        unread: count_unread_messages(conn)?,
    })
}

/// Optimise la base (à appeler après un gros crawl).
pub fn optimize(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA wal_checkpoint(PASSIVE); PRAGMA analysis_limit=1000; PRAGMA optimize;",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Page, PageImage};

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(SCHEMA_MAIN).unwrap();
        conn.execute_batch(SCHEMA_AUX).unwrap();
        ensure_fts(&conn).unwrap();
        crate::monitor::install(&conn).unwrap();
        conn
    }

    fn page() -> Page {
        Page {
            url: "https://example.com/".into(),
            domain: "example.com".into(),
            status_code: 200,
            title: "Guide".into(),
            description: None,
            content: "searchableword".into(),
            content_html: "<p>Guide</p>".into(),
            links: vec![],
            images: vec![],
            favicon: None,
            content_hash: "hash".into(),
            size: 10,
            content_type: None,
            language: None,
            depth: 0,
            crawl_time_ms: 1,
        }
    }

    #[test]
    fn discovery_is_deduplicated_and_fresh_pages_are_not_recrawled() {
        let conn = db();
        assert_eq!(
            request_indexing(&conn, "https://example.com/").unwrap(),
            "queued"
        );
        assert_eq!(
            request_indexing(&conn, "https://example.com/").unwrap(),
            "queued"
        );
        assert_eq!(count_urls_by_status(&conn, "pending").unwrap(), 1);
        save_page(&conn, &page()).unwrap();
        mark_crawled(&conn, "https://example.com/", 200).unwrap();
        assert_eq!(
            request_indexing(&conn, "https://example.com/").unwrap(),
            "ready"
        );
        assert_eq!(count_urls_by_status(&conn, "pending").unwrap(), 0);
        conn.execute("UPDATE pages SET crawled_at=datetime('now','-2 days')", [])
            .unwrap();
        assert_eq!(schedule_stale(&conn, 86400, 500).unwrap(), 1);
        assert_eq!(schedule_stale(&conn, 86400, 500).unwrap(), 0);
    }

    #[test]
    fn discovery_respects_blocked_and_recent_failure_states() {
        let conn = db();
        enqueue(&conn, "https://example.com/", 0, 0).unwrap();
        mark_failed(&conn, "https://example.com/", "temporary").unwrap();
        assert_eq!(
            request_indexing(&conn, "https://example.com/").unwrap(),
            "deferred"
        );
        conn.execute("UPDATE urls SET updated_at=datetime('now','-2 days')", [])
            .unwrap();
        assert_eq!(
            request_indexing(&conn, "https://example.com/").unwrap(),
            "queued"
        );
        mark_blocked(&conn, "https://example.com/", "policy").unwrap();
        assert_eq!(
            request_indexing(&conn, "https://example.com/").unwrap(),
            "blocked"
        );
    }

    #[test]
    fn detects_and_repairs_an_empty_external_content_index() {
        let conn = db();
        conn.execute_batch("DROP TRIGGER pages_fts_ai;").unwrap();
        save_page(&conn, &page()).unwrap();
        let missing: i64 = conn
            .query_row(
                "SELECT count(*) FROM pages_fts WHERE pages_fts MATCH 'searchableword'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(missing, 0);
        ensure_fts(&conn).unwrap();
        let found: i64 = conn
            .query_row(
                "SELECT count(*) FROM pages_fts WHERE pages_fts MATCH 'searchableword'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, 1);
    }

    #[test]
    fn failed_media_insert_rolls_back_page_and_index() {
        let conn = db();
        save_page(&conn, &page()).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_image BEFORE INSERT ON images BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        let mut update = page();
        update.content = "replacementword".into();
        update.images.push(PageImage {
            url: "https://example.com/pic.png".into(),
            alt: None,
        });
        assert!(save_page(&conn, &update).is_err());
        let content: String = conn
            .query_row("SELECT content FROM pages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(content, "searchableword");
        conn.execute(
            "INSERT INTO pages_fts(pages_fts, rank) VALUES ('integrity-check', 1)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn crawled_page_and_queue_status_commit_or_rollback_together() {
        let conn = db();
        let page = page();
        enqueue(&conn, &page.url, 0, 0).unwrap();
        mark_crawling(&conn, &page.url).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_done BEFORE UPDATE OF status ON urls WHEN NEW.status='done' BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(save_crawled_page(&conn, &page, 200).is_err());
        assert_eq!(count_pages(&conn).unwrap(), 0);
        assert_eq!(count_urls_by_status(&conn, "crawling").unwrap(), 1);
        conn.execute_batch("DROP TRIGGER reject_done;").unwrap();
        save_crawled_page(&conn, &page, 200).unwrap();
        assert_eq!(count_pages(&conn).unwrap(), 1);
        assert_eq!(count_urls_by_status(&conn, "done").unwrap(), 1);
        conn.execute(
            "INSERT INTO pages_fts(pages_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn crawled_page_retries_a_temporary_external_writer_without_partial_results() {
        let dir = std::env::temp_dir().join(format!("vertex-writer-retry-{}", std::process::id()));
        let conn = init_in(&dir).unwrap();
        let page = page();
        enqueue(&conn, &page.url, 0, 0).unwrap();
        mark_crawling(&conn, &page.url).unwrap();
        conn.busy_timeout(Duration::from_millis(20)).unwrap();
        let path = dir.join("vertex.db");
        let (ready, waiting) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            let other = Connection::open(path).unwrap();
            other.execute_batch("BEGIN IMMEDIATE;").unwrap();
            ready.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            other.execute_batch("COMMIT;").unwrap();
        });
        waiting.recv().unwrap();
        save_crawled_page(&conn, &page, 200).unwrap();
        writer.join().unwrap();
        assert_eq!(count_pages(&conn).unwrap(), 1);
        assert_eq!(count_urls_by_status(&conn, "done").unwrap(), 1);
        conn.execute(
            "INSERT INTO pages_fts(pages_fts,rank) VALUES('integrity-check',1)",
            [],
        )
        .unwrap();
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
