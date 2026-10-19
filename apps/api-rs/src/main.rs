//! Serveur HTTP local de Vertex (axum) : interface web, recherche, visionneuse
//! assainie, signets, contact administrateur et proxys d'images/favicons.
//!
//! Les proxys `/api/img` et `/api/favicon` évitent tout contact direct du
//! navigateur avec des tiers (pas de trackers, pas de referer transmis).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    body::Body,
    extract::{Query, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Semaphore;

mod control;
mod monitor;
mod security;

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
    crawl_running: Arc<AtomicBool>,
    control_running: Arc<AtomicBool>,
    port: u16,
    access: security::Access,
    budget: Arc<security::Budget>,
    requests: Arc<Semaphore>,
    verification: monitor::Verification,
}

async fn local_request_guard(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if !state.access.accepts(request.headers(), state.port) {
        return (StatusCode::FORBIDDEN, "Requête externe refusée").into_response();
    }
    let cross_site = request
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site");
    let public_navigation = state.access.is_public()
        && request.method() == axum::http::Method::GET
        && !request.uri().path().starts_with("/api/");
    if cross_site && !public_navigation {
        return StatusCode::FORBIDDEN.into_response();
    }
    if state.access.is_public() {
        if security::requires_admin(request.method(), request.uri().path())
            && !state.access.is_admin(request.headers())
        {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
                Json(json!({"error": "Authentification administrateur requise"})),
            )
                .into_response();
        }
        if !state.budget.allow(
            request.method() == axum::http::Method::POST && request.uri().path() == "/api/contact",
        ) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "60")],
                "Trop de requetes",
            )
                .into_response();
        }
    }
    if request.uri().path().len() + request.uri().query().unwrap_or("").len() > 8192 {
        return StatusCode::URI_TOO_LONG.into_response();
    }
    let Ok(_permit) = state.requests.clone().try_acquire_owned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "Serveur occupe").into_response();
    };
    let mut response =
        match tokio::time::timeout(std::time::Duration::from_secs(25), next.run(request)).await {
            Ok(response) => response,
            Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
        };
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    response.headers_mut().insert("content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'".parse().unwrap());
    response.headers_mut().insert(
        "permissions-policy",
        "camera=(), microphone=(), geolocation=()".parse().unwrap(),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    if state.access.is_public() {
        response.headers_mut().insert(
            "strict-transport-security",
            "max-age=31536000".parse().unwrap(),
        );
    }
    response
}

async fn index() -> impl IntoResponse {
    Html(include_str!("../../web/index.html"))
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../web/app.js"),
    )
}

async fn app_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../web/app.css"),
    )
}

async fn icons_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("../../web/icons.svg"),
    )
}

async fn reader_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../web/reader.css"),
    )
}

async fn panel() -> impl IntoResponse {
    Html(include_str!("../../web/panel.html"))
}
async fn panel_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../../web/panel.js"),
    )
}
async fn panel_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../web/panel.css"),
    )
}

async fn config_handler(State(state): State<AppState>) -> Response {
    Json(json!({"public": state.access.is_public(), "discovery": discovery_enabled(), "shared": state.access.is_public()})).into_response()
}

fn discovery_enabled() -> bool {
    std::env::var("VERTEX_ALLOW_DISCOVERY").is_ok_and(|value| value == "1")
}

#[derive(Deserialize)]
struct DiscoveryPayload {
    url: String,
}

async fn discover_handler(
    State(state): State<AppState>,
    Json(payload): Json<DiscoveryPayload>,
) -> Response {
    if !discovery_enabled() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Some(url) = search_rs::runner::discovery_url(&payload.url) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"URL publique HTTP(S) requise"})),
        )
            .into_response();
    };
    if !state.budget.allow_discovery() {
        return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "60")]).into_response();
    }
    match db_call(&state, move |conn| crawler_request(conn, &url)).await {
        Ok(status) => {
            let code = if status == "full" {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            };
            (code, Json(json!({"status":status}))).into_response()
        }
        Err(error) => server_error(error),
    }
}

fn crawler_request(conn: &Connection, url: &str) -> rusqlite::Result<&'static str> {
    search_rs::database_request_indexing(conn, url)
}

async fn db_call<T: Send + 'static>(
    state: &AppState,
    query: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
) -> Result<T, String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|_| "Stockage indisponible".to_string())?;
        query(&conn).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    offset: Option<usize>,
}

async fn search_handler(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Response {
    if params.q.trim().is_empty() {
        return Json(json!({ "query": params.q, "results": [] })).into_response();
    }
    let offset = params.offset.unwrap_or(0);
    if params.q.len() > 256 || offset > 30_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Recherche trop longue ou pagination excessive"})),
        )
            .into_response();
    }
    let query = params.q.clone();
    match db_call(&state, move |conn| search(conn, &query, offset)).await {
        Ok(mut results) => {
            if state.access.is_public() {
                for result in &mut results {
                    result.bookmarked = false;
                }
            }
            Json(json!({ "query": params.q, "results": results })).into_response()
        }
        Err(e) => server_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    url: String,
}

async fn page_handler(State(state): State<AppState>, Query(params): Query<PageQuery>) -> Response {
    match db_call(&state, move |conn| get_page(conn, &params.url)).await {
        Ok(Some(mut page)) => {
            if state.access.is_public() {
                page.bookmarked = false;
            }
            Json(json!(page)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "page introuvable" })),
        )
            .into_response(),
        Err(e) => server_error(e),
    }
}

fn server_error(e: impl std::fmt::Display) -> Response {
    eprintln!("Erreur interne Vertex : {e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "Erreur interne du stockage" })),
    )
        .into_response()
}

/// Proxy d'images/favicons : le navigateur ne contacte jamais le site tiers.
#[derive(Debug, Deserialize)]
struct MediaQuery {
    url: String,
}

async fn proxy_media(state: &AppState, url: &str, max_bytes: usize) -> Response {
    let parsed = match search_rs::network::validate_url(url) {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "Destination non autorisée").into_response(),
    };
    if state.access.is_public() {
        let value = url.to_string();
        let host = parsed.host_str().unwrap_or("").to_string();
        let favicon = parsed.path() == "/favicon.ico" && parsed.query().is_none();
        let allowed = db_call(state, move |conn| conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM images WHERE url=?1) OR EXISTS(SELECT 1 FROM pages WHERE favicon=?1) OR (?3 AND EXISTS(SELECT 1 FROM pages WHERE domain=?2))",
            rusqlite::params![value, host, favicon], |r| r.get::<_, bool>(0))).await;
        match allowed {
            Ok(true) => {}
            Ok(false) => return StatusCode::FORBIDDEN.into_response(),
            Err(e) => return server_error(e),
        }
    }

    let resp = match state.http.get(parsed).send().await {
        Ok(r) => r,
        Err(_) => {
            return (StatusCode::BAD_GATEWAY, "Ressource distante indisponible").into_response();
        }
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

    let Some(content_type) = search_rs::network::image_content_type(&content_type) else {
        return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Type non image").into_response();
    };
    let content_type = content_type.to_string();

    match search_rs::network::read_limited_async(resp, max_bytes).await {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CACHE_CONTROL, "public, max-age=86400")
            .header("X-Content-Type-Options", "nosniff")
            .body(Body::from(bytes))
            .unwrap(),
        Err(_) => (StatusCode::BAD_GATEWAY, "Lecture distante impossible").into_response(),
    }
}

async fn img_handler(State(state): State<AppState>, Query(q): Query<MediaQuery>) -> Response {
    proxy_media(&state, &q.url, 8_000_000).await
}

async fn favicon_handler(State(state): State<AppState>, Query(q): Query<MediaQuery>) -> Response {
    proxy_media(&state, &q.url, 512_000).await
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
    match db_call(&state, bookmarks).await {
        Ok(list) => Json(json!({ "bookmarks": list })).into_response(),
        Err(e) => server_error(e),
    }
}

async fn bookmark_add_handler(
    State(state): State<AppState>,
    Json(p): Json<BookmarkPayload>,
) -> Response {
    if search_rs::network::validate_url(&p.url).is_err()
        || p.url.len() > 4096
        || p.title.as_ref().is_some_and(|v| v.len() > 500)
        || p.note.as_ref().is_some_and(|v| v.len() > 4000)
        || p.tags.as_ref().is_some_and(|v| v.len() > 500)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "url requise" })),
        )
            .into_response();
    }
    match db_call(&state, move |conn| {
        add_bookmark(
            conn,
            &p.url,
            p.title.as_deref(),
            p.note.as_deref(),
            p.tags.as_deref(),
        )
    })
    .await
    {
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
    match db_call(&state, move |conn| remove_bookmark(conn, &q.url)).await {
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
    if p.message.trim().is_empty()
        || p.message.len() > 8000
        || p.name.as_ref().is_some_and(|v| v.len() > 200)
        || p.email
            .as_ref()
            .is_some_and(|v| v.len() > 320 || (!v.is_empty() && !v.contains('@')))
        || p.subject.as_ref().is_some_and(|v| v.len() > 300)
        || p.page_url.as_ref().is_some_and(|v| v.len() > 4096)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Message vide ou champs invalides/trop longs" })),
        )
            .into_response();
    }
    let public = state.access.is_public();
    match db_call(&state, move |conn| {
        if public
            && conn.query_row("SELECT count(*) FROM contact_messages", [], |row| {
                row.get::<_, i64>(0)
            })? >= 10_000
        {
            return Ok(None);
        }
        add_contact(
            conn,
            p.name.as_deref(),
            p.email.as_deref(),
            p.subject.as_deref(),
            &p.message,
            p.page_url.as_deref(),
        )
        .map(Some)
    })
    .await
    {
        Ok(Some(id)) => Json(json!({ "ok": true, "id": id })).into_response(),
        Ok(None) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "La boite de contact est pleine"})),
        )
            .into_response(),
        Err(e) => server_error(e),
    }
}

async fn contact_list_handler(State(state): State<AppState>) -> Response {
    match db_call(&state, messages).await {
        Ok(list) => Json(json!({ "messages": list })).into_response(),
        Err(e) => server_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct IdPayload {
    id: i64,
}

async fn contact_read_handler(State(state): State<AppState>, Json(p): Json<IdPayload>) -> Response {
    match db_call(&state, move |conn| mark_message_read(conn, p.id)).await {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => server_error(e),
    }
}

async fn stats_handler(State(state): State<AppState>) -> Response {
    if state.access.is_public() {
        return match db_call(&state, |conn| {
            conn.query_row(
                "SELECT (SELECT count(*) FROM pages), (SELECT count(DISTINCT domain) FROM pages)",
                [],
                |r| Ok(json!({"pages": r.get::<_, i64>(0)?, "domains": r.get::<_, i64>(1)?})),
            )
        })
        .await
        {
            Ok(value) => Json(value).into_response(),
            Err(e) => server_error(e),
        };
    }
    match db_call(&state, stats).await {
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
async fn crawl_handler(State(state): State<AppState>, Json(p): Json<CrawlPayload>) -> Response {
    if search_rs::runner::crawl_is_running(&search_rs::default_db_dir()) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "Cette base est deja utilisee par un crawler"})),
        )
            .into_response();
    }
    if state
        .crawl_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "Un crawl est déjà en cours" })),
        )
            .into_response();
    }
    let duration = p.duration.unwrap_or(3600).clamp(30, 86_400);
    let db_dir = search_rs::default_db_dir();
    std::thread::spawn(move || {
        struct Reset(Arc<AtomicBool>);
        impl Drop for Reset {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _reset = Reset(state.crawl_running);
        let cfg = CrawlConfig {
            duration_secs: duration,
            ..CrawlConfig::default()
        };
        let summary = run_crawl_with(&db_dir, search_rs::SEEDS, cfg);
        for error in &summary.errors {
            eprintln!("[api] echec du crawl : {error}");
        }
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

async fn crawl_status_handler(State(state): State<AppState>) -> Response {
    let running = state.crawl_running.load(Ordering::Acquire)
        || search_rs::runner::crawl_is_running(&search_rs::default_db_dir());
    Json(json!({ "running": running })).into_response()
}

#[tokio::main]
async fn main() {
    let access = security::Access::from_env().unwrap_or_else(|error| {
        eprintln!("Configuration refusee : {error}");
        std::process::exit(1);
    });
    if access.is_public() && std::env::var_os("VERTEX_DB_DIR").is_none_or(|dir| dir.is_empty()) {
        eprintln!("Mode public refuse : choisir explicitement un index dedie avec VERTEX_DB_DIR");
        std::process::exit(1);
    }
    let port = std::env::var("VERTEX_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3000);
    let bind_host = std::env::var("VERTEX_BIND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    if bind_host != "127.0.0.1" && !(bind_host == "0.0.0.0" && access.is_public()) {
        eprintln!("VERTEX_BIND_HOST refuse : 0.0.0.0 exige le mode public authentifie");
        std::process::exit(1);
    }
    let db_dir = search_rs::default_db_dir();
    let _ = search_rs::safety::load_user_blocklist(&search_rs::blocklist_path(&db_dir));

    let conn = search_rs::init_in(&db_dir).expect("Impossible d'ouvrir la base Vertex");
    conn.busy_timeout(std::time::Duration::from_secs(3))
        .expect("SQLite timeout");
    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        http: search_rs::network::async_client(
            std::time::Duration::from_secs(20),
            "VertexProxy/0.1",
        )
        .expect("Client HTTP du proxy"),
        crawl_running: Arc::new(AtomicBool::new(false)),
        control_running: Arc::new(AtomicBool::new(false)),
        port,
        access,
        budget: Arc::new(security::Budget::default()),
        requests: Arc::new(Semaphore::new(32)),
        verification: monitor::Verification::default(),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/panel", get(panel))
        .route("/panel.js", get(panel_js))
        .route("/panel.css", get(panel_css))
        .route("/api/admin/monitor", get(monitor::snapshot))
        .route("/api/admin/crawler", axum::routing::post(control::command))
        .route("/api/admin/verify", axum::routing::post(monitor::verify))
        .route("/app.js", get(app_js))
        .route("/app.css", get(app_css))
        .route("/reader.css", get(reader_css))
        .route("/icons.svg", get(icons_svg))
        .route("/api/config", get(config_handler))
        .route("/api/discover", axum::routing::post(discover_handler))
        .route(
            "/robots.txt",
            get(|| async { "User-agent: *\nDisallow: /api/\n" }),
        )
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
        .route(
            "/api/contact/read",
            axum::routing::post(contact_read_handler),
        )
        .route("/api/stats", get(stats_handler))
        .route("/api/safety", get(safety_handler))
        .route("/api/crawl", axum::routing::post(crawl_handler))
        .route("/api/crawl/status", get(crawl_status_handler))
        .route("/api/img", get(img_handler))
        .route("/api/favicon", get(favicon_handler))
        .layer(axum::extract::DefaultBodyLimit::max(16_384))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            local_request_guard,
        ))
        .with_state(state);

    let addr = format!("{bind_host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Impossible de binder le port 3000");

    println!("Vertex API sur http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("Erreur serveur");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn state() -> AppState {
        AppState {
            db: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
            http: search_rs::network::async_client(std::time::Duration::from_secs(1), "test")
                .unwrap(),
            crawl_running: Arc::new(AtomicBool::new(false)),
            control_running: Arc::new(AtomicBool::new(false)),
            port: 3000,
            access: security::Access::default(),
            budget: Arc::new(security::Budget::default()),
            requests: Arc::new(Semaphore::new(32)),
            verification: monitor::Verification::default(),
        }
    }

    #[tokio::test]
    async fn public_control_cannot_execute_local_commands() {
        let mut app = state();
        app.access = security::Access::public(
            "https://vertex.example",
            "test-token-123456789012345678901234567890",
        )
        .unwrap();
        let command = serde_json::from_value(json!({"action":"start","workers":64})).unwrap();
        let response = control::command(State(app), Json(command)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn local_control_rejects_unbounded_or_injected_commands() {
        let command = serde_json::from_value(json!({"action":"start","workers":65})).unwrap();
        let response = control::command(State(state()), Json(command)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(
            serde_json::from_value::<control::Command>(json!({"action":"start; calc.exe"}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<control::Command>(
                json!({"action":"start","script":"other.ps1"})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn refuses_cross_site_requests_and_dns_rebinding_hosts() {
        let app = Router::new()
            .route("/health", get(health))
            .layer(middleware::from_fn_with_state(state(), local_request_guard));
        for (host, origin, expected) in [
            ("127.0.0.1:3000", "http://127.0.0.1:3000", StatusCode::OK),
            (
                "attacker.example:3000",
                "http://attacker.example:3000",
                StatusCode::FORBIDDEN,
            ),
            (
                "127.0.0.1:3000",
                "https://attacker.example",
                StatusCode::FORBIDDEN,
            ),
        ] {
            let request = Request::builder()
                .uri("/health")
                .header(header::HOST, host)
                .header(header::ORIGIN, origin)
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                app.clone().oneshot(request).await.unwrap().status(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn proxy_blocks_local_services_without_connecting() {
        let state = state();
        for url in [
            "http://127.0.0.1:3000/health",
            "http://[::1]:3000/",
            "http://169.254.169.254/",
        ] {
            assert_eq!(
                proxy_media(&state, url, 100).await.status(),
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[tokio::test]
    async fn duplicate_crawl_returns_conflict() {
        let state = state();
        state.crawl_running.store(true, Ordering::Release);
        assert_eq!(
            crawl_handler(State(state), Json(CrawlPayload { duration: Some(30) }))
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
}
