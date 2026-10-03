//! Application desktop Vertex (Tauri).
//!
//! Expose au front-end : recherche, visionneuse assainie, statistiques, crawl,
//! signets, contact administrateur et un proxy média local (images/favicons)
//! afin qu'aucune requête ne parte vers un tiers.

use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use crawler_rs::runner::{run_crawl_with, CrawlConfig};
use crawler_rs::{database, SEEDS};
use search_rs::{
    Bookmark, ContactMessage, DbStats, PageView, SearchResult, add_bookmark, add_contact, bookmarks,
    get_page, mark_message_read, messages, remove_bookmark, safety as link_safety, search as fts_search,
    stats as db_stats,
};

/// Base de données partagée par les commandes.
struct Db {
    conn: Mutex<Connection>,
    dir: PathBuf,
}

/// Racine du workspace Vertex (…/Vertex).
fn workspace_root() -> PathBuf {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent() // desktop
        .and_then(|p| p.parent()) // apps
        .and_then(|p| p.parent()) // Vertex
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

// --- Recherche et lecture -------------------------------------------------

#[tauri::command]
fn search(query: String, offset: usize, db: State<Db>) -> Result<Vec<SearchResult>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    fts_search(&conn, &query, offset).map_err(|e| e.to_string())
}

#[tauri::command]
fn page(url: String, db: State<Db>) -> Result<Option<PageView>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    get_page(&conn, &url).map_err(|e| e.to_string())
}

#[tauri::command]
fn stats(db: State<Db>) -> Result<DbStats, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    db_stats(&conn).map_err(|e| e.to_string())
}

/// Vérifie localement la sûreté d'un lien.
#[tauri::command]
fn check_link(url: String) -> link_safety::SafetyReport {
    link_safety::check_url(&url)
}

// --- Crawl ----------------------------------------------------------------

#[tauri::command]
fn crawl(duration: u64, db: State<Db>) -> Result<(), String> {
    let dir = db.dir.clone();

    thread::spawn(move || {
        let cfg = CrawlConfig {
            duration_secs: duration,
            ..CrawlConfig::default()
        };
        let summary = run_crawl_with(&dir, SEEDS, cfg);
        println!(
            "Vertex crawl terminé : {} OK, {} échecs, {} bloqués (malveillants), {} ignorés, {} domaines",
            summary.pages_ok,
            summary.pages_failed,
            summary.pages_blocked,
            summary.pages_skipped,
            summary.sites
        );
    });

    Ok(())
}

// --- Signets --------------------------------------------------------------

#[tauri::command]
fn bookmarks_list(db: State<Db>) -> Result<Vec<Bookmark>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    bookmarks(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn bookmark_add(
    url: String,
    title: Option<String>,
    note: Option<String>,
    tags: Option<String>,
    db: State<Db>,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    add_bookmark(
        &conn,
        &url,
        title.as_deref(),
        note.as_deref(),
        tags.as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn bookmark_remove(url: String, db: State<Db>) -> Result<bool, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    remove_bookmark(&conn, &url).map_err(|e| e.to_string())
}

// --- Contact administrateur ----------------------------------------------

#[tauri::command]
fn contact_send(
    name: Option<String>,
    email: Option<String>,
    subject: Option<String>,
    message: String,
    db: State<Db>,
) -> Result<i64, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    add_contact(
        &conn,
        name.as_deref(),
        email.as_deref(),
        subject.as_deref(),
        &message,
        None,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn contact_list(db: State<Db>) -> Result<Vec<ContactMessage>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    messages(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn contact_mark_read(id: i64, db: State<Db>) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    mark_message_read(&conn, id).map_err(|e| e.to_string())
}

// --- Proxy média (images / favicons) --------------------------------------

/// Réponse binaire renvoyée au front-end (converti en `Blob` côté JS).
#[derive(Serialize)]
struct MediaData {
    content_type: String,
    data: Vec<u8>,
}

/// Récupère une ressource distante côté Rust : le WebView ne contacte jamais
/// directement le site tiers (pas de tracker, pas de referer).
#[tauri::command]
async fn fetch_media(url: String) -> Result<MediaData, String> {
    let client = reqwest::Client::builder()
        .user_agent("Vertex/0.1")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > 8_000_000 {
        return Err("Fichier trop volumineux".into());
    }

    Ok(MediaData {
        content_type,
        data: bytes.to_vec(),
    })
}

/// Ouvre une URL dans le navigateur du système.
#[tauri::command]
fn open(url: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .arg("/c")
            .arg("start")
            .arg("")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let root = workspace_root();
    let db_dir = root.join("database");

    let _ = search_rs::safety::load_user_blocklist(&search_rs::blocklist_path(&db_dir));

    println!("============================================");
    println!("Vertex Desktop");
    println!("============================================");
    println!("Workspace : {}", root.display());
    println!("Database  : {}", db_dir.display());

    let conn = database::init_in(&db_dir).expect("Impossible d'ouvrir la base Vertex");
    let pages = database::count_pages(&conn).unwrap_or(0);
    println!("Pages disponibles : {}", pages);

    let db = Db {
        conn: Mutex::new(conn),
        dir: db_dir,
    };

    tauri::Builder::default()
        .manage(db)
        .invoke_handler(tauri::generate_handler![
            search,
            page,
            crawl,
            stats,
            check_link,
            bookmarks_list,
            bookmark_add,
            bookmark_remove,
            contact_send,
            contact_list,
            contact_mark_read,
            fetch_media,
            open
        ])
        .run(tauri::generate_context!())
        .expect("Erreur au lancement de Vertex Desktop");
}
