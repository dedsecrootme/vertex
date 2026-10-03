//! Serveur HTTP local de Vertex (axum) : interface web, recherche, visionneuse
//! assainie, signets, contact administrateur et proxys d'images/favicons.
//!
//! Les proxys `/api/img` et `/api/favicon` évitent tout contact direct du
//! navigateur avec des tiers (pas de trackers, pas de referer transmis).

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    body::Body,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use search_rs::runner::{CrawlConfig, run_crawl_with};
use search_rs::{
    add_bookmark, add_contact, bookmarks, get_page, mark_message_read, messages, remove_bookmark,
    search, stats,
};

/// État partagé du serveur.
#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    http: reqwest::Client,
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

async fn search_handler(State(state): State<AppState>, Query(params): Query<SearchQuery>) -> Response {
    if params.q.trim().is_empty() {
        return Json(json!({ "query": params.q, "results": [] })).into_response();
    }
    let offset = params.offset.unwrap_or(0);
    let conn = state.db.lock().unwrap();
    match search(&conn, &params.q, offset) {
        Ok(results) => Json(json!({ "query": params.q, "results": results })).into_response(),
        Err(e) => server_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    url: String,
}

async fn page_handler(State(state): State<AppState>, Query(params): Query<PageQuery>) -> Response {
    let conn = state.db.lock().unwrap();
    match get_page(&conn, &params.url) {
        Ok(Some(page)) => Json(json!(page)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "page introuvable" })),
        )
            .into_response(),
        Err(e) => server_error(e),
    }
}

fn server_error(e: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": e.to_string() })),
    )
        .into_response()
}

/// Proxy d'images/favicons : le navigateur ne contacte jamais le site tiers.
#[derive(Debug, Deserialize)]
struct MediaQuery {
    url: String,
}

async fn proxy_media(state: &AppState, url: &str, max_bytes: usize, expect_image: bool) -> Response {
    let parsed = match url::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => u,
        _ => return (StatusCode::BAD_REQUEST, "URL invalide").into_response(),
    };

    let resp = match state.http.get(parsed).send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("Échec du proxy : {e}")).into_response(),
    };

    if !resp.status().is_success() {
        return (StatusCode::BAD_GATEWAY, "Ressource indisponible").into_response();
    }

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if expect_image && !content_type.starts_with("image/") {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Type non image").into_response();
    }

    match resp.bytes().await {
        Ok(bytes) if bytes.len() <= max_bytes => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "public, max-age=86400")
            .body(Body::from(bytes))
            .unwrap(),
        Ok(_) => (StatusCode::PAYLOAD_TOO_LARGE, "Fichier trop volumineux").into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("Lecture échouée : {e}")).into_response(),
    }
}

async fn img_handler(State(state): State<AppState>, Query(q): Query<MediaQuery>) -> Response {
    proxy_media(&state, &q.url, 8_000_000, true).await
}

async fn favicon_handler(State(state): State<AppState>, Query(q): Query<MediaQuery>) -> Response {
    proxy_media(&state, &q.url, 512_000, false).await
}

// --- Signets --------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BookmarkPayload {
    url: String,
    title: Option<String>,
    note: Option<String>,
    tags: Option<String>,
}

async fn bookmarks_list_handler(State(state): State<AppState>) -> Response {
    let conn = state.db.lock().unwrap();
    match bookmarks(&conn) {
        Ok(list) => Json(json!({ "bookmarks": list })).into_response(),
        Err(e) => server_error(e),
    }
}

async fn bookmark_add_handler(
    State(state): State<AppState>,
    Json(p): Json<BookmarkPayload>,
) -> Response {
    if p.url.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "url requise" }))).into_response();
    }
    let conn = state.db.lock().unwrap();
    match add_bookmark(
        &conn,
        &p.url,
        p.title.as_deref(),
        p.note.as_deref(),
        p.tags.as_deref(),
    ) {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => server_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct UrlQuery {
    url: String,
}

async fn bookmark_remove_handler(
    State(state): State<AppState>,
    Query(q): Query<UrlQuery>,
) -> Response {
    let conn = state.db.lock().unwrap();
    match remove_bookmark(&conn, &q.url) {
        Ok(removed) => Json(json!({ "ok": true, "removed": removed })).into_response(),
        Err(e) => server_error(e),
    }
}

// --- Contact administrateur ----------------------------------------------

#[derive(Debug, Deserialize)]
struct ContactPayload {
    name: Option<String>,
    email: Option<String>,
    subject: Option<String>,
    message: String,
    page_url: Option<String>,
}

async fn contact_add_handler(
    State(state): State<AppState>,
    Json(p): Json<ContactPayload>,
) -> Response {
    if p.message.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "message vide" })),
        )
            .into_response();
    }
    let conn = state.db.lock().unwrap();
    match add_contact(
        &conn,
        p.name.as_deref(),
        p.email.as_deref(),
        p.subject.as_deref(),
        &p.message,
        p.page_url.as_deref(),
    ) {
        Ok(id) => Json(json!({ "ok": true, "id": id })).into_response(),
        Err(e) => server_error(e),
    }
}

async fn contact_list_handler(State(state): State<AppState>) -> Response {
    let conn = state.db.lock().unwrap();
    match messages(&conn) {
        Ok(list) => Json(json!({ "messages": list })).into_response(),
        Err(e) => server_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct IdPayload {
    id: i64,
}

async fn contact_read_handler(
    State(state): State<AppState>,
    Json(p): Json<IdPayload>,
) -> Response {
    let conn = state.db.lock().unwrap();
    match mark_message_read(&conn, p.id) {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => server_error(e),
    }
}

async fn stats_handler(State(state): State<AppState>) -> Response {
    let conn = state.db.lock().unwrap();
    match stats(&conn) {
        Ok(s) => Json(json!(s)).into_response(),
        Err(e) => server_error(e),
    }
}

// --- Crawl et vérification de sûreté --------------------------------------

#[derive(Debug, Deserialize)]
struct CrawlPayload {
    duration: Option<u64>,
}

/// Démarre un crawl en arrière-plan (le serveur reste disponible).
async fn crawl_handler(Json(p): Json<CrawlPayload>) -> Response {
    let duration = p.duration.unwrap_or(3600).clamp(30, 86_400);
    let db_dir = search_rs::default_db_dir();
    std::thread::spawn(move || {
        let cfg = CrawlConfig {
            duration_secs: duration,
            ..CrawlConfig::default()
        };
        let summary = run_crawl_with(&db_dir, search_rs::SEEDS, cfg);
        println!(
            "[api] crawl terminé : {} OK, {} échecs, {} bloqués, {} domaines",
            summary.pages_ok, summary.pages_failed, summary.pages_blocked, summary.sites
        );
    });
    Json(json!({ "started": true, "duration": duration })).into_response()
}

#[derive(Debug, Deserialize)]
struct SafetyQuery {
    url: String,
}

/// Vérifie localement la sûreté d'un lien (aucune requête tierce).
async fn safety_handler(Query(q): Query<SafetyQuery>) -> Response {
    Json(search_rs::safety::check_url(&q.url)).into_response()
}

#[tokio::main]
async fn main() {
    let db_dir = search_rs::default_db_dir();
    let _ = search_rs::safety::load_user_blocklist(&search_rs::blocklist_path(&db_dir));

    let conn = search_rs::init_in(&db_dir).expect("Impossible d'ouvrir la base Vertex");
    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        http: reqwest::Client::builder()
            .user_agent("VertexProxy/0.1")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("Client HTTP du proxy"),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/api/search", get(search_handler))
        .route("/api/page", get(page_handler))
        .route(
            "/api/bookmarks",
            get(bookmarks_list_handler)
                .post(bookmark_add_handler)
                .delete(bookmark_remove_handler),
        )
        .route(
            "/api/contact",
            get(contact_list_handler).post(contact_add_handler),
        )
        .route("/api/contact/read", axum::routing::post(contact_read_handler))
        .route("/api/stats", get(stats_handler))
        .route("/api/safety", get(safety_handler))
        .route("/api/crawl", axum::routing::post(crawl_handler))
        .route("/api/img", get(img_handler))
        .route("/api/favicon", get(favicon_handler))
        .with_state(state);

    let addr = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Impossible de binder le port 3000");

    println!("Vertex API sur http://{addr}");
    axum::serve(listener, app).await.expect("Erreur serveur");
}
