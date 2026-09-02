use rusqlite::{Connection, Result};

use std::fs;
use std::path::{Path, PathBuf};

use crate::models::Page;

/// Racine du workspace Vertex, résolue indépendamment du répertoire courant.
/// Le crate vit dans `<racine>/apps/crawler-rs`, on remonte donc de 2 niveaux.
fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent() // .../apps
        .and_then(|p| p.parent()) // .../Vertex
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

pub fn init() -> Result<Connection> {
    let db_dir = workspace_root().join("database");
    init_in(&db_dir)
}

/// Ouvre (en créant si besoin) la base SQLite dans `db_dir`.
/// Les tables et l'index FTS5 sont créés s'ils n'existent pas.
pub fn init_in(db_dir: &std::path::Path) -> Result<Connection> {
    fs::create_dir_all(db_dir).unwrap();

    let conn = Connection::open(db_dir.join("vertex.db"))?;

    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;

        CREATE TABLE IF NOT EXISTS urls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT UNIQUE NOT NULL,
            status TEXT DEFAULT 'pending',
            http_code INTEGER,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS pages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT UNIQUE NOT NULL,
            title TEXT,
            description TEXT,
            content TEXT DEFAULT '',
            content_hash TEXT,
            status_code INTEGER,
            links_count INTEGER DEFAULT 0,
            crawled_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS errors (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            url TEXT,
            error TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_urls_status
        ON urls(status);

        CREATE INDEX IF NOT EXISTS idx_pages_url
        ON pages(url);
        "
    )?;

    ensure_content_column(&conn)?;
    ensure_fts(&conn)?;

    Ok(conn)
}

/// Ajoute la colonne `content` aux tables `pages` existantes (migration).
fn ensure_content_column(conn: &Connection) -> Result<()> {
    let exists = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('pages') WHERE name='content'",
        [],
        |row| row.get::<_, i64>(0),
    )?;

    if exists == 0 {
        conn.execute(
            "ALTER TABLE pages ADD COLUMN content TEXT DEFAULT ''",
            [],
        )?;
    }

    Ok(())
}

pub fn add_url(conn: &Connection, url: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO urls(url) VALUES(?1)",
        [url],
    )?;
    Ok(())
}

pub fn url_exists(conn: &Connection, url: &str) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT EXISTS(SELECT 1 FROM urls WHERE url=?1)",
    )?;
    let exists = stmt.query_row([url], |row| row.get(0))?;
    Ok(exists)
}

pub fn save_page(conn: &Connection, page: &Page) -> Result<()> {
    conn.execute(
        "
        INSERT OR REPLACE INTO pages
        (url, title, description, content, content_hash, status_code, links_count)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
        (
            &page.url,
            &page.title,
            &page.description,
            &page.content,
            &page.hash,
            page.status_code,
            page.links.len(),
        ),
    )?;
    Ok(())
}

pub fn mark_crawled(conn: &Connection, url: &str, http_code: u16) -> Result<()> {
    conn.execute(
        "
        UPDATE urls
        SET status='done', http_code=?2, updated_at=CURRENT_TIMESTAMP
        WHERE url=?1
        ",
        (url, http_code),
    )?;
    Ok(())
}

pub fn mark_crawling(conn: &Connection, url: &str) -> Result<()> {
    conn.execute(
        "
        UPDATE urls
        SET status='crawling', updated_at=CURRENT_TIMESTAMP
        WHERE url=?1
        ",
        [url],
    )?;
    Ok(())
}

pub fn reset_crawling(conn: &Connection) -> Result<()> {
    conn.execute(
        "
        UPDATE urls
        SET status='pending'
        WHERE status='crawling'
        ",
        [],
    )?;
    Ok(())
}

pub fn get_next_url(conn: &Connection) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "
        SELECT url
        FROM urls
        WHERE status='pending'
        ORDER BY id ASC
        LIMIT 1
        ",
    )?;
    let mut rows = stmt.query([])?;

    if let Some(row) = rows.next()? {
        let url: String = row.get(0)?;
        Ok(Some(url))
    } else {
        Ok(None)
    }
}

pub fn mark_failed(conn: &Connection, url: &str, error: &str) -> Result<()> {
    conn.execute(
        "
        UPDATE urls
        SET status='failed', updated_at=CURRENT_TIMESTAMP
        WHERE url=?1
        ",
        [url],
    )?;

    save_error(conn, url, error)?;
    Ok(())
}

pub fn save_error(conn: &Connection, url: &str, error: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO errors (url, error) VALUES (?1, ?2)",
        (url, error),
    )?;
    Ok(())
}
pub fn count_urls_by_status(conn: &Connection, status: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM urls WHERE status=?1",
        [status],
        |row| row.get(0),
    )
}

pub fn all_urls(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT url FROM urls")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut urls = Vec::new();
    for row in rows {
        urls.push(row?);
    }
    Ok(urls)
}
/// Crée la table virtuelle FTS5 (index plein texte) et la garde synchronisée
/// avec `pages` via des triggers. Reconstruit l'index si la base est vide.
fn ensure_fts(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
            url UNINDEXED,
            title,
            content,
            content='pages',
            content_rowid='id'
        );

        CREATE TRIGGER IF NOT EXISTS pages_fts_ai
        AFTER INSERT ON pages BEGIN
            INSERT INTO pages_fts(rowid, url, title, content)
            VALUES (new.id, new.url, new.title, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS pages_fts_ad
        AFTER DELETE ON pages BEGIN
            INSERT INTO pages_fts(pages_fts, rowid, url, title, content)
            VALUES ('delete', old.id, old.url, old.title, old.content);
        END;

        CREATE TRIGGER IF NOT EXISTS pages_fts_au
        AFTER UPDATE ON pages BEGIN
            INSERT INTO pages_fts(pages_fts, rowid, url, title, content)
            VALUES ('delete', old.id, old.url, old.title, old.content);
            INSERT INTO pages_fts(rowid, url, title, content)
            VALUES (new.id, new.url, new.title, new.content);
        END;
        "
    )?;

    let count: i64 = conn.query_row("SELECT count(*) FROM pages_fts", [], |r| r.get(0))?;
    if count == 0 {
        conn.execute("INSERT INTO pages_fts(pages_fts) VALUES('rebuild')", [])?;
    }

    Ok(())
}