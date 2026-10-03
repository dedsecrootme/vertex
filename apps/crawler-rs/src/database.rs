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
pub const SCHEMA_VERSION: i64 = 2;

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
"#;

/// Initialise la base par défaut.
pub fn init() -> Result<Connection> {
    init_in(&default_db_dir())
}

/// Ouvre/crée la base et applique le schéma + les migrations.
pub fn init_in(db_dir: &Path) -> Result<Connection> {
    fs::create_dir_all(db_dir).expect("Impossible de créer le dossier de base de données");

    let conn = Connection::open(db_dir.join("vertex.db"))?;
    conn.busy_timeout(Duration::from_secs(30))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;",
    )?;
    conn.execute_batch(SCHEMA_MAIN)?;
    conn.execute_batch(SCHEMA_AUX)?;

    migrate(&conn)?;
    conn.execute_batch(SCHEMA_INDEXES)?;
    ensure_fts(&conn)?;

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
        ("error", "TEXT"),
        ("safety", "TEXT NOT NULL DEFAULT 'unknown'"),
    ] {
        ensure_column(conn, "urls", col, decl)?;
    }

    for (col, decl) in [
        ("domain", "TEXT"),
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
fn ensure_fts(conn: &Connection) -> Result<()> {
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

    let pages_count: i64 = conn.query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))?;
    let fts_count: i64 = conn.query_row("SELECT COUNT(*) FROM pages_fts", [], |r| r.get(0))?;
    if pages_count != fts_count {
        println!("Synchronisation FTS5 : {pages_count} pages / {fts_count} documents.");
        conn.execute("INSERT INTO pages_fts(pages_fts) VALUES ('rebuild')", [])?;
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
        "UPDATE urls SET status = 'pending', updated_at = CURRENT_TIMESTAMP
         WHERE status = 'failed'",
        [],
    )?;
    Ok(n)
}

/// Enregistre le verdict de sûreté d'une URL (cache).
pub fn set_url_safety(conn: &Connection, url: &str, verdict: &str, score: i64, reasons: &str) -> Result<()> {
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
    conn.query_row(
        "SELECT COUNT(*) FROM urls WHERE status = ?1",
        [status],
        |row| row.get(0),
    )
}

/// Nombre total d'URL connues.
pub fn count_urls(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM urls", [], |row| row.get(0))
}

/// Remplit la colonne `domain` des URL créées avant la migration v2.
/// Traite au plus `limit` lignes par appel pour ne pas bloquer indéfiniment.
pub fn backfill_domains(conn: &Connection, limit: usize) -> Result<usize> {
    let rows: Vec<(i64, String)> = {
        let mut stmt =
            conn.prepare("SELECT id, url FROM urls WHERE domain IS NULL LIMIT ?1")?;
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
        "SELECT COUNT(DISTINCT domain) FROM urls WHERE domain IS NOT NULL AND domain != ''",
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
    conn.query_row("SELECT id FROM pages WHERE url = ?1", [url], |row| row.get(0))
        .optional()
}

/// Enregistre (ou met à jour) une page, ses liens complets et ses images.
/// Renvoie l'identifiant interne de la page.
pub fn save_page(conn: &Connection, page: &Page) -> Result<i64> {
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

    let id: i64 = conn.query_row("SELECT id FROM pages WHERE url = ?1", [&page.url], |r| r.get(0))?;
    conn.execute("DELETE FROM links WHERE page_id = ?1", [id])?;
    conn.execute("DELETE FROM images WHERE page_id = ?1", [id])?;

    for link in &page.links {
        conn.execute(
            "INSERT OR IGNORE INTO links(page_id, url, anchor, is_external, safety)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                id,
                link.url,
                link.anchor,
                link.is_external as i64,
                link.safety
            ],
        )?;
    }

    for image in &page.images {
        conn.execute(
            "INSERT OR IGNORE INTO images(page_id, url, alt) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, image.url, image.alt],
        )?;
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
    conn.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
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
    conn.execute("UPDATE contact_messages SET is_read = 1 WHERE id = ?1", [id])?;
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
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize; ANALYZE;")
}
