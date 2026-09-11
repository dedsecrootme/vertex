use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;

use rusqlite::Connection;
use tauri::State;

use crawler_rs::{database, runner::run_crawl_in, SEEDS};
use search_rs::{get_page as fetch_page, search as fts_search, PageContent, SearchResult};

struct Db {
    conn: Mutex<Connection>,
    dir: PathBuf,
}

#[tauri::command]
fn search(query: String, offset: usize, db: State<Db>) -> Result<Vec<SearchResult>, String> {
    let conn = db.conn.lock().unwrap();
    fts_search(&conn, &query, offset).map_err(|e| e.to_string())
}

#[tauri::command]
fn page(url: String, db: State<Db>) -> Result<Option<PageContent>, String> {
    let conn = db.conn.lock().unwrap();
    fetch_page(&conn, &url).map_err(|e| e.to_string())
}

#[tauri::command]
fn crawl(duration: u64, db: State<Db>) -> Result<(), String> {
    let dir = db.dir.clone();
    thread::spawn(move || {
        let _ = run_crawl_in(&dir, duration, SEEDS);
    });
    Ok(())
}

#[tauri::command]
fn stats(db: State<Db>) -> Result<serde_json::Value, String> {
    let conn = db.conn.lock().unwrap();
    let done = database::count_urls_by_status(&conn, "done").unwrap_or(0);
    let failed = database::count_urls_by_status(&conn, "failed").unwrap_or(0);
    let pending = database::count_urls_by_status(&conn, "pending").unwrap_or(0);
    Ok(serde_json::json!({ "done": done, "failed": failed, "pending": pending }))
}

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
    // Dossier writable et stable pour la base SQLite (l'app fonctionne
    // même installée dans Program Files). Le crawl écrit dans le même dossier.
    let app_data = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    let db_dir = std::path::Path::new(&app_data).join("Vertex");
    let conn = database::init_in(&db_dir).expect("❌ Impossible d'ouvrir la base Vertex");

    tauri::Builder::default()
        .manage(Db { conn: Mutex::new(conn), dir: db_dir })
        .invoke_handler(tauri::generate_handler![search, page, crawl, stats, open])
        .run(tauri::generate_context!())
        .expect("❌ Erreur au lancement de l'app");
}