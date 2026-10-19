//! Application desktop Vertex (Tauri).
//!
//! Expose au front-end : recherche, visionneuse assainie, statistiques, crawl,
//! signets, contact administrateur et un proxy média local (images/favicons)
//! pour eviter les requetes tierces directes depuis la visionneuse.
//! Le proxy contacte les serveurs externes ; l'adresse IP reste visible.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use tauri::Manager;
use tauri::State;
use tauri_plugin_opener::OpenerExt;
mod contributor;
pub use contributor::run_if_requested as run_contributor_if_requested;

use crawler_rs::runner::{run_crawl_with, CrawlConfig};
use crawler_rs::{database, SEEDS};
use search_rs::{
    add_bookmark, add_contact, bookmarks, get_page, mark_message_read, messages, remove_bookmark,
    safety as link_safety, search as fts_search, stats as db_stats, Bookmark, ContactMessage,
    DbStats, PageView, SearchResult,
};

/// Base de données partagée par les commandes.
struct Db {
    conn: Mutex<Connection>,
    dir: PathBuf,
    crawl_running: std::sync::Arc<AtomicBool>,
    shared: Option<search_rs::shared::SharedIndex>,
}

// --- Recherche et lecture -------------------------------------------------

#[tauri::command]
async fn search(
    query: String,
    offset: usize,
    db: State<'_, Db>,
) -> Result<Vec<SearchResult>, String> {
    if let Some(shared) = &db.shared {
        let mut rows = shared.search(&query, offset).await?;
        let conn = db.conn.lock().map_err(|e| e.to_string())?;
        if rows.is_empty() {
            return fts_search(&conn, &query, offset).map_err(|e| e.to_string());
        }
        let saved = bookmarks(&conn).map_err(|e| e.to_string())?;
        for row in &mut rows {
            row.bookmarked = saved.iter().any(|b| b.url == row.url);
        }
        return Ok(rows);
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    fts_search(&conn, &query, offset).map_err(|e| e.to_string())
}

#[tauri::command]
async fn page(url: String, db: State<'_, Db>) -> Result<Option<PageView>, String> {
    if let Some(shared) = &db.shared {
        if let Some(page) = shared.page(&url).await? {
            return Ok(Some(page));
        }
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    get_page(&conn, &url).map_err(|e| e.to_string())
}

#[tauri::command]
async fn stats(db: State<'_, Db>) -> Result<DbStats, String> {
    let shared = if let Some(shared) = &db.shared {
        Some(shared.stats().await?)
    } else {
        None
    };
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut result = db_stats(&conn).map_err(|e| e.to_string())?;
    if let Some(shared) = shared {
        result.pages = shared.pages;
        result.domains = shared.domains;
    }
    Ok(result)
}

#[derive(Serialize)]
struct ClientConfig {
    public: bool,
    shared: bool,
    discovery: bool,
    origin: Option<String>,
}

#[tauri::command]
fn client_config(db: State<Db>) -> ClientConfig {
    ClientConfig {
        public: false,
        shared: db.shared.is_some(),
        discovery: db.shared.is_some(),
        origin: db.shared.as_ref().map(|s| s.origin().to_string()),
    }
}

#[tauri::command]
fn contribution_status(db: State<Db>) -> ContributionStatus {
    ContributionStatus {
        settings: contributor::settings(&db.dir),
        running: crawler_rs::runner::crawl_is_running(&db.dir),
        error: std::fs::read_to_string(db.dir.join("contributor-error.txt"))
            .ok()
            .map(|error| error.chars().take(1000).collect()),
    }
}

#[derive(Serialize)]
struct ContributionStatus {
    #[serde(flatten)]
    settings: contributor::Settings,
    running: bool,
    error: Option<String>,
}

#[tauri::command]
fn contribution_set(
    enabled: bool,
    background: bool,
    share_discoveries: bool,
    db: State<Db>,
) -> Result<(), String> {
    let origin = if share_discoveries {
        Some(
            db.shared
                .as_ref()
                .ok_or("Index partage non configure")?
                .origin(),
        )
    } else {
        None
    };
    contributor::configure(&db.dir, enabled, background, origin)
}

#[tauri::command]
async fn discover(url: String, db: State<'_, Db>) -> Result<String, String> {
    let url = crawler_rs::runner::discovery_url(&url).ok_or("URL publique requise")?;
    let shared = db.shared.as_ref().ok_or("Index partage non configure")?;
    shared.discover(&url).await
}

/// Vérifie localement la sûreté d'un lien.
#[tauri::command]
fn check_link(url: String) -> link_safety::SafetyReport {
    link_safety::check_url(&url)
}

// --- Crawl ----------------------------------------------------------------

#[tauri::command]
fn crawl_status(db: State<Db>) -> bool {
    db.crawl_running.load(Ordering::Acquire) || crawler_rs::runner::crawl_is_running(&db.dir)
}

#[tauri::command]
fn crawl(duration: u64, db: State<Db>) -> Result<(), String> {
    if crawler_rs::runner::crawl_is_running(&db.dir) {
        return Err("Un autre crawler utilise deja cette base".into());
    }
    if db
        .crawl_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("Un crawl est déjà en cours".into());
    }
    let dir = db.dir.clone();
    let running = std::sync::Arc::clone(&db.crawl_running);
    let duration = duration.clamp(30, 86_400);

    thread::spawn(move || {
        struct Reset(std::sync::Arc<AtomicBool>);
        impl Drop for Reset {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _reset = Reset(running);
        let cfg = CrawlConfig {
            duration_secs: duration,
            ..CrawlConfig::default()
        };
        let summary = run_crawl_with(&dir, SEEDS, cfg);
        for error in &summary.errors {
            eprintln!("Echec du crawl : {error}");
        }
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
    crawler_rs::network::validate_url(&url)?;
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
/// directement le site tiers (pas de script ni de referer de la visionneuse).
#[tauri::command]
async fn fetch_media(url: String) -> Result<MediaData, String> {
    let url = crawler_rs::network::validate_url(&url)?;
    let client = crawler_rs::network::async_client(Duration::from_secs(20), "Vertex/0.1")
        .map_err(|e| e.to_string())?;

    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let content_type = crawler_rs::network::image_content_type(&content_type)
        .ok_or_else(|| "Type non image".to_string())?
        .to_string();
    let bytes = crawler_rs::network::read_limited_async(resp, 8_000_000).await?;

    Ok(MediaData {
        content_type,
        data: bytes.to_vec(),
    })
}

/// Ouvre une URL dans le navigateur du système.
#[tauri::command]
fn open(url: String, app: tauri::AppHandle) -> Result<(), String> {
    let url = crawler_rs::network::validate_url(&url)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let db_dir = match std::env::var_os("VERTEX_DB_DIR").filter(|dir| !dir.is_empty()) {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            let conn = database::init_in(&db_dir)?;
            let _ = search_rs::safety::load_user_blocklist(&search_rs::blocklist_path(&db_dir));
            contributor::start(&db_dir)?;
            app.manage(Db {
                conn: Mutex::new(conn),
                dir: db_dir,
                crawl_running: std::sync::Arc::new(AtomicBool::new(false)),
                shared: std::env::var("VERTEX_SHARED_API")
                    .ok()
                    .or_else(|| option_env!("VERTEX_SHARED_API").map(str::to_string))
                    .filter(|value| !value.is_empty())
                    .map(|origin| search_rs::shared::SharedIndex::new(&origin))
                    .transpose()?,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            search,
            client_config,
            discover,
            contribution_status,
            contribution_set,
            page,
            crawl,
            crawl_status,
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
        .build(tauri::generate_context!())
        .expect("Erreur au lancement de Vertex Desktop")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let db = app.state::<Db>();
                if !contributor::settings(&db.dir).background {
                    let _ = contributor::stop(&db.dir);
                }
            }
        });
}
