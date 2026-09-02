use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use search_rs::{get_page, search};

type SharedDb = Arc<Mutex<Connection>>;

/// Racine du workspace Vertex, rÃ©solue indÃ©pendamment du rÃ©pertoire courant.
fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent() // .../apps
        .and_then(|p| p.parent()) // .../Vertex
        .unwrap_or(manifest_dir)
        .to_path_buf()
}

async fn index() -> impl IntoResponse {
    Html(include_str!("../../web/index.html"))
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    offset: Option<usize>,
}

async fn search_handler(State(db): State<SharedDb>, Query(params): Query<SearchQuery>) -> Response {
    if params.q.trim().is_empty() {
        return Json(json!({ "query": params.q, "results": [] })).into_response();
    }

    let offset = params.offset.unwrap_or(0);
    let conn = db.lock().unwrap();
    match search(&conn, &params.q, offset) {
        Ok(results) => Json(json!({ "query": params.q, "results": results })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    url: String,
}

async fn page_handler(State(db): State<SharedDb>, Query(params): Query<PageQuery>) -> Response {
    let conn = db.lock().unwrap();
    match get_page(&conn, &params.url) {
        Ok(Some(page)) => Json(json!(page)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "page introuvable" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[tokio::main]
async fn main() {
    let db_path = workspace_root().join("database").join("vertex.db");
    let conn = Connection::open(&db_path).expect("âŒ Impossible d'ouvrir la base vertex.db");
    let state: SharedDb = Arc::new(Mutex::new(conn));

    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/api/search", get(search_handler))
        .route("/api/page", get(page_handler))
        .with_state(state);

    let addr = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("âŒ Impossible de binder le port 3000");

    println!("ðŸ•·ï¸ Vertex API sur http://{addr}");
    axum::serve(listener, app)
        .await
        .expect("âŒ Erreur serveur");
}