//! Moteur de crawl complet : concurrence, politesse par domaine, respect de
//! `robots.txt`, découverte des `sitemap.xml`, limite de profondeur, reprise
//! après arrêt et journalisation des exécutions.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use rusqlite::Connection;
#[cfg(test)]
use rusqlite::OptionalExtension;

use crate::{crawler, database, network, robots, safety};

/// Configuration d'une session de crawl.
#[derive(Debug, Clone)]
pub struct CrawlConfig {
    /// Durée maximale en secondes.
    pub duration_secs: u64,
    /// Nombre de threads de travail.
    pub workers: usize,
    /// Profondeur maximale des liens suivis.
    pub max_depth: i64,
    /// Délai minimal entre deux requêtes vers un même domaine.
    pub per_domain_delay_ms: u64,
    /// Délai d'expiration d'une requête HTTP.
    pub request_timeout_secs: u64,
    /// Plafond facultatif du nombre de pages traitées sur la session.
    pub max_pages: Option<i64>,
    /// Suivre les liens sortant du domaine de départ.
    pub follow_external: bool,
    /// Respecter `robots.txt`.
    pub respect_robots: bool,
    /// Discover more URLs, or only process the queue already present.
    pub discover_links: bool,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            duration_secs: 300,
            workers: 4,
            max_depth: 6,
            per_domain_delay_ms: 1000,
            request_timeout_secs: 8,
            max_pages: Some(10_000),
            follow_external: true,
            respect_robots: true,
            discover_links: true,
        }
    }
}

/// Résumé d'un crawl.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CrawlSummary {
    pub run_id: i64,
    pub pages_ok: i64,
    pub pages_failed: i64,
    pub pages_blocked: i64,
    pub pages_skipped: i64,
    pub pending: i64,
    pub sites: i64,
    pub elapsed_secs: u64,
    pub errors: Vec<String>,
}

/// Règles `robots.txt` d'un domaine.
/// L'implémentation complète et conforme (RFC 9309 : jokers `*`/`$`, règle du
/// plus long match, groupe d'agent le plus spécifique) vit dans [`crate::robots`].
type RobotsRules = robots::Rules;

/// Analyse un `robots.txt` pour l'agent Vertex (RFC 9309).
fn parse_robots(body: &str) -> RobotsRules {
    robots::parse(body, "VertexCrawler")
}

#[derive(Default)]
struct RunCounts {
    claimed: i64,
    active: i64,
    ok: i64,
    failed: i64,
    blocked: i64,
    skipped: i64,
}

struct WorkPermit(Arc<Shared>, String);
impl Drop for WorkPermit {
    fn drop(&mut self) {
        self.0.active_hosts.lock().unwrap().remove(&self.1);
        self.0.counts.lock().unwrap().active -= 1;
    }
}

/// État partagé par tous les workers.
struct Shared {
    stop: Arc<AtomicBool>,
    storage: Option<StorageBudget>,
    storage_error: Mutex<Option<String>>,
    robots: Mutex<HashMap<String, Arc<OnceLock<RobotsRules>>>>,
    domain_next: Mutex<HashMap<String, Instant>>,
    sitemap_done: Mutex<HashSet<String>>,
    counts: Mutex<RunCounts>,
    workers: Mutex<Vec<crate::monitor::Worker>>,
    frontier: Mutex<crate::frontier::Frontier>,
    active_hosts: Mutex<HashSet<String>>,
    /// Client dédié aux fichiers annexes (robots.txt, sitemaps) : délai court
    /// pour ne jamais bloquer un worker sur un domaine lent.
    aux_client: Client,
}

impl Shared {
    fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            storage: None,
            storage_error: Mutex::new(None),
            robots: Mutex::new(HashMap::new()),
            domain_next: Mutex::new(HashMap::new()),
            sitemap_done: Mutex::new(HashSet::new()),
            counts: Mutex::new(RunCounts::default()),
            workers: Mutex::new(Vec::new()),
            frontier: Mutex::new(crate::frontier::Frontier::default()),
            active_hosts: Mutex::new(HashSet::new()),
            aux_client: crawler::build_client(3),
        }
    }

    /// Attend si nécessaire pour respecter le délai par domaine.
    fn respect_delay(&self, domain: &str, delay_ms: u64, deadline: Instant) -> bool {
        if self.stop.load(Ordering::Acquire) {
            return false;
        }
        if delay_ms == 0 {
            return Instant::now() < deadline;
        }
        let wait_until = {
            let mut map = self.domain_next.lock().unwrap();
            let now = Instant::now();
            let slot = map.entry(domain.to_string()).or_insert(now);
            let wait = if *slot > now { *slot } else { now };
            if wait >= deadline {
                return false;
            }
            *slot = wait
                .checked_add(Duration::from_millis(delay_ms))
                .unwrap_or(deadline);
            wait
        };
        while Instant::now() < wait_until {
            if self.stop.load(Ordering::Acquire) {
                return false;
            }
            thread::sleep(
                wait_until
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(200)),
            );
        }
        Instant::now() < deadline
    }
}

struct StorageBudget {
    dir: std::path::PathBuf,
    max_bytes: u64,
    min_free_bytes: u64,
}

struct Heartbeat {
    quit: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Heartbeat {
    fn start(dir: &std::path::Path, shared: Arc<Shared>, configured: usize, mode: &str) -> Self {
        let path = dir.join("vertex.db");
        let mode = mode.to_owned();
        let quit = Arc::new(AtomicBool::new(false));
        let done = quit.clone();
        let thread = thread::spawn(move || {
            let Ok(conn) = Connection::open(path) else {
                return;
            };
            let _ = conn.busy_timeout(Duration::from_millis(250));
            let started_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            loop {
                let final_tick = done.load(Ordering::Acquire);
                let counts = shared.counts.lock().unwrap();
                let mut workers = shared.workers.lock().unwrap().clone();
                if final_tick {
                    for worker in &mut workers {
                        worker.online = false;
                        worker.phase = "offline".into();
                        worker.domain.clear();
                    }
                }
                let runtime = crate::monitor::Runtime {
                    pid: std::process::id(),
                    started_at,
                    configured,
                    mode: mode.clone(),
                    max_storage_bytes: shared.storage.as_ref().map_or(0, |s| s.max_bytes),
                    min_free_bytes: shared.storage.as_ref().map_or(0, |s| s.min_free_bytes),
                    session_ok: counts.ok,
                    session_failed: counts.failed,
                    workers,
                };
                drop(counts);
                if let Ok(payload) = serde_json::to_string(&runtime) {
                    let _=conn.execute("INSERT INTO crawler_runtime(id,heartbeat,payload) VALUES(1,unixepoch(),?1) ON CONFLICT(id) DO UPDATE SET heartbeat=excluded.heartbeat,payload=excluded.payload",[payload]);
                }
                if final_tick {
                    break;
                }
                for _ in 0..5 {
                    if done.load(Ordering::Acquire) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(200));
                }
            }
        });
        Self {
            quit,
            thread: Some(thread),
        }
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.quit.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct WorkerPresence {
    shared: Arc<Shared>,
    id: usize,
}
impl WorkerPresence {
    fn set(&self, phase: &str, domain: &str) {
        if let Some(worker) = self.shared.workers.lock().unwrap().get_mut(self.id) {
            worker.online = true;
            worker.phase = phase.into();
            worker.domain = domain.into();
        }
    }
}
impl Drop for WorkerPresence {
    fn drop(&mut self) {
        if let Some(worker) = self.shared.workers.lock().unwrap().get_mut(self.id) {
            worker.online = false;
            worker.phase = "offline".into();
            worker.domain.clear();
        }
    }
}

impl StorageBudget {
    fn check(&self) -> Result<(), String> {
        let bytes: u64 = ["vertex.db", "vertex.db-wal"]
            .iter()
            .filter_map(|name| std::fs::metadata(self.dir.join(name)).ok())
            .map(|m| m.len())
            .sum();
        if bytes >= self.max_bytes {
            return Err("Quota de stockage atteint; arret sans supprimer les URL restantes".into());
        }
        let free = fs2::available_space(&self.dir).map_err(|e| e.to_string())?;
        if free < self.min_free_bytes {
            return Err(
                "Reserve d'espace disque atteinte; arret sans supprimer les URL restantes".into(),
            );
        }
        Ok(())
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
}

pub fn discovery_url(value: &str) -> Option<String> {
    if value.len() > 4096 {
        return None;
    }
    let clean = crawler::normalize_url(value, value)?;
    network::validate_url(&clean).ok()?;
    if !crawler::is_crawlable_url(&clean)
        || safety::check_url(&clean).verdict.as_str() == "malicious"
    {
        return None;
    }
    Some(clean)
}

fn path_query_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => {
            let mut p = u.path().to_string();
            if let Some(q) = u.query() {
                p.push('?');
                p.push_str(q);
            }
            p
        }
        Err(_) => "/".to_string(),
    }
}

/// Récupère un petit fichier texte (robots.txt, sitemap) avec plafond de taille.
fn fetch_text(shared: &Shared, cfg: &CrawlConfig, url: &str, deadline: Instant) -> Option<String> {
    let resp = crawler::fetch_response(&shared.aux_client, url, |hop| {
        prepare_request(shared, cfg, hop, url, deadline)
    })
    .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = network::read_limited(resp, 2_000_000).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// Charge (et met en cache) les règles `robots.txt` d'un domaine, et découvre
/// les sitemaps déclarés à la première visite du domaine.
/// Charge (et met en cache) les règles `robots.txt`. Renvoie aussi les sitemaps à
/// explorer lors de la première visite d'un domaine joignable.
///
/// **Aucun verrou de base n'est pris ici** : les appels réseau ne bloquent donc
/// pas les autres workers.
fn load_robots(
    shared: &Shared,
    base_url: &str,
    cfg: &CrawlConfig,
    deadline: Instant,
) -> (RobotsRules, Vec<String>) {
    let base = url::Url::parse(base_url).expect("validated URL");
    let domain = base.origin().ascii_serialization();
    let entry = shared
        .robots
        .lock()
        .unwrap()
        .entry(domain.clone())
        .or_default()
        .clone();
    let rules = entry
        .get_or_init(|| {
            let robots_url = format!("{domain}/robots.txt");

            // Délai court dédié : un domaine lent ne doit pas immobiliser un worker.
            let response = crawler::fetch_response(&shared.aux_client, &robots_url, |hop| {
                if shared.respect_delay(&host_of(hop), cfg.per_domain_delay_ms, deadline) {
                    Ok(())
                } else {
                    Err(crawler::CrawlError::Skipped(
                        "Duree de session atteinte".into(),
                    ))
                }
            })
            .ok();
            let was_success = response.as_ref().is_some_and(|r| r.status().is_success());
            let unavailable = response
                .as_ref()
                .is_none_or(|r| r.status().is_server_error() || r.status().as_u16() == 429);
            let fetched = response
                .filter(|r| r.status().is_success())
                .and_then(|r| network::read_limited(r, 2_000_000).ok())
                .map(|body| String::from_utf8_lossy(&body).into_owned());
            if unavailable || (was_success && fetched.is_none()) {
                parse_robots("User-agent: *\nDisallow: /\n")
            } else {
                fetched.map(|body| parse_robots(&body)).unwrap_or_default()
            }
        })
        .clone();

    // Sitemaps : une seule fois par domaine, et uniquement si le site répond.
    let first_visit = shared.sitemap_done.lock().unwrap().insert(domain.clone());
    let sitemaps = if first_visit {
        let mut list = rules.sitemaps.clone();
        if list.is_empty() {
            list.push(format!("{domain}/sitemap.xml"));
        }
        list.into_iter().take(2).collect()
    } else {
        Vec::new()
    };

    (rules, sitemaps)
}

fn prepare_request(
    shared: &Shared,
    cfg: &CrawlConfig,
    url: &str,
    source: &str,
    deadline: Instant,
) -> Result<(), crawler::CrawlError> {
    if Instant::now() >= deadline || shared.stop.load(Ordering::Acquire) {
        return Err(crawler::CrawlError::Deadline);
    }
    if !cfg.follow_external && host_of(url) != host_of(source) {
        return Err(crawler::CrawlError::Skipped(
            "Redirection hors du domaine autorise".into(),
        ));
    }
    let mut delay = cfg.per_domain_delay_ms;
    if cfg.respect_robots {
        let (rules, _) = load_robots(shared, url, cfg, deadline);
        if Instant::now() >= deadline {
            return Err(crawler::CrawlError::Deadline);
        }
        if !rules.allows(&path_query_of(url)) {
            return Err(crawler::CrawlError::Skipped(
                "Interdit par robots.txt".into(),
            ));
        }
        delay = delay.max(rules.crawl_delay_ms.unwrap_or(0));
    }
    if !shared.respect_delay(&host_of(url), delay, deadline) {
        return Err(crawler::CrawlError::Deadline);
    }
    Ok(())
}

// The OS releases this lock even after a crash. Never delete the lock file:
// replacing its inode would allow a second live process to acquire a new lock.
struct CrawlLease(std::fs::File);
impl CrawlLease {
    fn acquire(dir: &std::path::Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("crawl.lock"))
            .map_err(|e| e.to_string())?;
        file.try_lock().map_err(|e| {
            format!("Une session de crawl utilise deja cette base (ou verrou indisponible) : {e}")
        })?;
        Ok(Self(file))
    }
}
impl Drop for CrawlLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub fn crawl_is_running(dir: &std::path::Path) -> bool {
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join("crawl.lock"))
    else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(_) => true,
    }
}

/// Enregistre en base les URL d'un sitemap **déjà téléchargé** (aucun réseau ici).
fn store_sitemap(
    conn: &Connection,
    body: &str,
    depth: i64,
    source: &str,
    follow_external: bool,
) -> rusqlite::Result<()> {
    const MAX_URLS: usize = 2000;
    let Ok(document) = roxmltree::Document::parse(body) else {
        return Ok(());
    };
    for loc in document
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "loc")
        .filter_map(|n| n.text())
        .take(MAX_URLS)
    {
        if database::count_urls_by_status(conn, "pending")? >= 100_000 {
            break;
        }
        let loc = loc.trim();
        if loc.is_empty() || loc.ends_with(".xml") || loc.ends_with(".xml.gz") {
            // Index de sitemaps : ignoré (pas de descente récursive).
            continue;
        }
        if let Some(clean) = crawler::normalize_url(loc, loc)
            && network::validate_url(&clean).is_ok()
            && (follow_external || host_of(&clean) == host_of(source))
        {
            database::enqueue(conn, &clean, depth, 5)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Boucle de crawl
// ---------------------------------------------------------------------------

/// Sélectionne et réserve (atomiquement) la prochaine URL à traiter.
#[cfg(test)]
fn take_next(conn: &Connection, max_depth: i64) -> rusqlite::Result<Option<(String, i64)>> {
    conn.query_row(
        "UPDATE urls SET status = 'crawling', attempts = attempts + 1,
         updated_at = CURRENT_TIMESTAMP WHERE id = (
             SELECT id FROM urls INDEXED BY idx_urls_claim WHERE status = 'pending' AND safety != 'malicious'
             AND attempts < 3 AND next_attempt <= unixepoch()
             AND depth <= ?1 ORDER BY priority DESC, id ASC LIMIT 1
         ) RETURNING url, depth",
        [max_depth],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    )
    .optional()
}

/// Restitue une reservation non executee sans consommer une tentative.
fn release_claim(conn: &Connection, url: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE urls SET status='pending', attempts=max(0,attempts-1) WHERE url=?1 AND status='crawling'", [url])?;
    Ok(())
}

/// Lance un crawl sur la base par défaut (configuration standard).
pub fn run_crawl(duration_secs: u64, seeds: &[&str]) -> CrawlSummary {
    run_crawl_in(&database::default_db_dir(), duration_secs, seeds)
}

/// Lance un crawl dans un dossier de base explicite (configuration standard).
pub fn run_crawl_in(db_dir: &std::path::Path, duration_secs: u64, seeds: &[&str]) -> CrawlSummary {
    let cfg = CrawlConfig {
        duration_secs,
        ..CrawlConfig::default()
    };
    run_crawl_with(db_dir, seeds, cfg)
}

/// Cœur du moteur : file partagée, workers, journalisation.
pub fn run_crawl_with(db_dir: &std::path::Path, seeds: &[&str], cfg: CrawlConfig) -> CrawlSummary {
    let _lease = match CrawlLease::acquire(db_dir) {
        Ok(lease) => lease,
        Err(error) => {
            return CrawlSummary {
                errors: vec![error],
                ..CrawlSummary::default()
            };
        }
    };
    let conn = match database::init_in(db_dir) {
        Ok(conn) => conn,
        Err(error) => {
            return CrawlSummary {
                errors: vec![error.to_string()],
                ..CrawlSummary::default()
            };
        }
    };

    // Charge la blocklist utilisateur si elle existe.
    let _ = safety::load_user_blocklist(&database::blocklist_path(db_dir));

    let shared = Arc::new(Shared::new());
    let _heartbeat = Heartbeat::start(db_dir, shared.clone(), cfg.workers.clamp(1, 64), "session");
    run_session(Arc::new(Mutex::new(conn)), seeds, cfg, shared, true)
}

#[derive(Clone, Debug)]
pub struct ContinuousConfig {
    pub crawl: CrawlConfig,
    pub idle_secs: u64,
    pub refresh_secs: u64,
    pub max_storage_bytes: u64,
    pub min_free_bytes: u64,
    pub drain_existing: bool,
}

impl Default for ContinuousConfig {
    fn default() -> Self {
        Self {
            crawl: CrawlConfig::default(),
            idle_secs: 30,
            refresh_secs: 86_400,
            max_storage_bytes: 8 * 1024 * 1024 * 1024,
            min_free_bytes: 2 * 1024 * 1024 * 1024,
            drain_existing: false,
        }
    }
}

/// Keeps the OS lease and politeness state across sessions. The caller owns shutdown.
pub fn run_continuous(
    db_dir: &std::path::Path,
    seeds: &[&str],
    config: ContinuousConfig,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let _lease = CrawlLease::acquire(db_dir)?;
    println!(
        "Vertex : ouverture et verification de l'index {}",
        db_dir.display()
    );
    let conn = database::init_in(db_dir).map_err(|e| e.to_string())?;
    database::reset_crawling(&conn).map_err(|e| e.to_string())?;
    for seed in seeds.iter().filter(|_| !config.drain_existing) {
        if network::validate_url(seed).is_ok() {
            database::enqueue(&conn, seed, 0, 10).map_err(|e| e.to_string())?;
        }
    }
    if config.drain_existing {
        conn.execute("UPDATE urls INDEXED BY idx_urls_safety SET status='blocked', error='URL deja signalee malveillante', updated_at=CURRENT_TIMESTAMP WHERE status='pending' AND safety='malicious'", []).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE urls INDEXED BY idx_urls_retry_exhausted SET attempts=0, next_attempt=0 WHERE status='pending' AND attempts>=3",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    let _ = safety::load_user_blocklist(&database::blocklist_path(db_dir));
    let conn = Arc::new(Mutex::new(conn));
    let mut shared = Shared::new();
    shared.stop = stop.clone();
    shared.storage = Some(StorageBudget {
        dir: db_dir.to_path_buf(),
        max_bytes: config.max_storage_bytes,
        min_free_bytes: config.min_free_bytes,
    });
    let shared = Arc::new(shared);
    let _heartbeat = Heartbeat::start(
        db_dir,
        shared.clone(),
        config.crawl.workers.clamp(1, 64),
        if config.drain_existing {
            "drain"
        } else {
            "continuous"
        },
    );
    let mut cache_started = Instant::now();
    let mut maintenance = Instant::now();
    while !stop.load(Ordering::Acquire) {
        shared.storage.as_ref().unwrap().check()?;
        if !config.drain_existing {
            let guard = conn.lock().map_err(|e| e.to_string())?;
            database::schedule_stale(&guard, config.refresh_secs, 500)
                .map_err(|e| e.to_string())?;
            guard.execute("UPDATE urls SET status='pending', attempts=0, next_attempt=0, updated_at=CURRENT_TIMESTAMP WHERE id IN (SELECT id FROM urls WHERE status='failed' AND safety!='malicious' AND updated_at < datetime('now','-1 day') LIMIT 100)", []).map_err(|e| e.to_string())?;
        }
        if cache_started.elapsed() >= Duration::from_secs(3600) {
            shared.robots.lock().unwrap().clear();
            shared.sitemap_done.lock().unwrap().clear();
            shared
                .domain_next
                .lock()
                .unwrap()
                .retain(|_, next| *next > Instant::now());
            cache_started = Instant::now();
        }
        *shared.counts.lock().unwrap() = RunCounts::default();
        let mut cfg = config.crawl.clone();
        cfg.duration_secs = cfg.duration_secs.clamp(1, 300);
        if config.drain_existing {
            cfg.discover_links = false;
            cfg.max_depth = i64::MAX;
            cfg.max_pages = None;
        }
        let summary = run_session(conn.clone(), &[], cfg, shared.clone(), false);
        if let Some(error) = shared.storage_error.lock().unwrap().take() {
            return Err(error);
        }
        if !summary.errors.is_empty() {
            return Err(summary.errors.join("; "));
        }
        println!(
            "Vertex session {} : {} indexees, {} echecs, {} bloquees, {} ignorees, {} en attente ({} s)",
            summary.run_id,
            summary.pages_ok,
            summary.pages_failed,
            summary.pages_blocked,
            summary.pages_skipped,
            summary.pending,
            summary.elapsed_secs
        );
        if config.drain_existing && summary.pending == 0 {
            println!(
                "Traitement de la file termine; les echecs et exclusions restent consultables."
            );
            break;
        }
        if maintenance.elapsed() >= Duration::from_secs(3600) {
            let guard = conn.lock().map_err(|e| e.to_string())?;
            guard.execute_batch("PRAGMA wal_checkpoint(PASSIVE); DELETE FROM errors WHERE created_at < datetime('now','-30 days'); DELETE FROM crawl_runs WHERE finished_at < datetime('now','-30 days');")
                .map_err(|e| e.to_string())?;
            maintenance = Instant::now();
        }
        let until = Instant::now() + Duration::from_secs(config.idle_secs.clamp(1, 3600));
        while Instant::now() < until && !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(200));
        }
    }
    Ok(())
}

fn run_session(
    conn: Arc<Mutex<Connection>>,
    seeds: &[&str],
    mut cfg: CrawlConfig,
    shared: Arc<Shared>,
    full_maintenance: bool,
) -> CrawlSummary {
    cfg.workers = cfg.workers.clamp(1, 64);
    cfg.duration_secs = cfg.duration_secs.min(86_400);
    cfg.max_depth = if cfg.discover_links {
        cfg.max_depth.clamp(0, 20)
    } else {
        cfg.max_depth.max(0)
    };
    cfg.request_timeout_secs = cfg.request_timeout_secs.clamp(1, 30);

    let setup = (|| -> rusqlite::Result<i64> {
        let conn = conn.lock().unwrap();
        if full_maintenance {
            database::reset_crawling(&conn)?;
        }
        for seed in seeds {
            database::enqueue(&conn, seed, 0, 10)?;
        }
        database::run_start(&conn, cfg.duration_secs as i64)
    })();
    let run_id = match setup {
        Ok(id) => id,
        Err(error) => {
            return CrawlSummary {
                errors: vec![error.to_string()],
                ..CrawlSummary::default()
            };
        }
    };

    let client = crawler::build_client(cfg.request_timeout_secs);
    let cfg = Arc::new(cfg);
    let start = Instant::now();

    let mut handles = Vec::new();
    *shared.workers.lock().unwrap() = (0..cfg.workers)
        .map(|id| crate::monitor::Worker {
            id: id + 1,
            online: false,
            phase: "starting".into(),
            domain: String::new(),
        })
        .collect();
    for id in 0..cfg.workers.max(1) {
        let conn = Arc::clone(&conn);
        let client = client.clone();
        let shared = Arc::clone(&shared);
        let cfg = Arc::clone(&cfg);
        handles.push(thread::spawn(move || {
            let presence = WorkerPresence {
                shared: shared.clone(),
                id,
            };
            presence.set("idle", "");
            worker(conn, client, shared, cfg, start, &presence)
        }));
    }
    let mut errors = Vec::new();
    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(error),
            Err(_) => errors.push("Crawl worker panicked".into()),
        }
    }

    let conn = conn.lock().unwrap();
    // Alimente la colonne `domain` des URL héritées de la v1, une fois le crawl
    // terminé (n'affecte donc pas le débit de crawl).
    if full_maintenance {
        let _ = database::backfill_domains(&conn, 200_000);
    }
    let counts = shared.counts.lock().unwrap();
    let pages_ok = counts.ok;
    let pages_failed = counts.failed;
    let pages_blocked = counts.blocked;
    let pages_skipped = counts.skipped;
    let pending = database::count_urls_by_status(&conn, "pending").unwrap_or(0);
    let sites = database::count_domains(&conn).unwrap_or(0);
    if let Err(error) = database::run_finish(
        &conn,
        run_id,
        pages_ok,
        pages_failed,
        pages_ok + pages_failed + pages_blocked + pages_skipped,
    ) {
        errors.push(error.to_string());
    }
    if full_maintenance && let Err(error) = database::optimize(&conn) {
        errors.push(error.to_string());
    }
    if !errors.is_empty() {
        let _ = conn.execute(
            "UPDATE crawl_runs SET status = 'failed' WHERE id = ?1",
            [run_id],
        );
    }

    CrawlSummary {
        run_id,
        pages_ok,
        pages_failed,
        pages_blocked,
        pages_skipped,
        pending,
        sites,
        elapsed_secs: start.elapsed().as_secs(),
        errors,
    }
}

/// Boucle d'un worker : réserve une URL, respecte la politesse, crawle,
/// enregistre la page puis met ses liens complets en file.
fn worker(
    conn: Arc<Mutex<Connection>>,
    client: Client,
    shared: Arc<Shared>,
    cfg: Arc<CrawlConfig>,
    start: Instant,
    presence: &WorkerPresence,
) -> Result<(), String> {
    worker_inner(conn, client, shared, cfg, start, presence).map_err(|e| {
        eprintln!("Crawl worker database error: {e:?}");
        e.to_string()
    })
}

fn worker_inner(
    conn: Arc<Mutex<Connection>>,
    client: Client,
    shared: Arc<Shared>,
    cfg: Arc<CrawlConfig>,
    start: Instant,
    presence: &WorkerPresence,
) -> rusqlite::Result<()> {
    let deadline = start
        .checked_add(Duration::from_secs(cfg.duration_secs))
        .unwrap_or(start);
    loop {
        presence.set("idle", "");
        if start.elapsed().as_secs() >= cfg.duration_secs || shared.stop.load(Ordering::Acquire) {
            break;
        }
        if let Some(budget) = &shared.storage
            && let Err(error) = budget.check()
        {
            *shared.storage_error.lock().unwrap() = Some(error);
            shared.stop.store(true, Ordering::Release);
            break;
        }
        let next = {
            let guard = conn.lock().unwrap();
            if cfg
                .max_pages
                .is_some_and(|max| shared.counts.lock().unwrap().claimed >= max)
            {
                break;
            }
            let claim = shared.frontier.lock().unwrap().claim(
                &guard,
                cfg.max_depth,
                &shared.active_hosts.lock().unwrap(),
                &shared.domain_next.lock().unwrap(),
            )?;
            match claim {
                Some(claim) => {
                    shared
                        .active_hosts
                        .lock()
                        .unwrap()
                        .insert(claim.host.clone());
                    let mut counts = shared.counts.lock().unwrap();
                    counts.claimed += 1;
                    counts.active += 1;
                    Some(claim)
                }
                None if shared.counts.lock().unwrap().active == 0 => {
                    if database::count_urls_by_status(&guard, "pending")? == 0 {
                        break;
                    }
                    None
                }
                None => None,
            }
        };
        let Some(claim) = next else {
            thread::sleep(Duration::from_millis(200));
            continue;
        };
        let _permit = WorkPermit(Arc::clone(&shared), claim.host);
        let (url, depth) = (claim.url, claim.depth);
        if !crawler::is_crawlable_url(&url) {
            let guard = conn.lock().unwrap();
            database::mark_skipped(
                &guard,
                &url,
                crawler::exclusion_reason(&url).unwrap_or("Destination non autorisee"),
            )?;
            shared.counts.lock().unwrap().skipped += 1;
            continue;
        }

        let domain = host_of(&url);
        presence.set("robots", &domain);

        if cfg.respect_robots {
            // Tout le réseau se fait HORS verrou de base (workers réellement parallèles).
            let (_, sitemaps) = load_robots(&shared, &url, &cfg, deadline);

            if cfg.discover_links && !sitemaps.is_empty() && depth < cfg.max_depth {
                let bodies: Vec<String> = sitemaps
                    .iter()
                    .filter(|s| cfg.follow_external || host_of(s) == domain)
                    .filter_map(|s| fetch_text(&shared, &cfg, s, deadline))
                    .collect();
                if !bodies.is_empty() {
                    let guard = conn.lock().unwrap();
                    for body in &bodies {
                        store_sitemap(&guard, body, depth + 1, &url, cfg.follow_external)?;
                    }
                }
            }
        }
        if Instant::now() >= deadline || shared.stop.load(Ordering::Acquire) {
            release_claim(&conn.lock().unwrap(), &url)?;
            break;
        }

        presence.set("fetching", &domain);
        match crawler::crawl_with(&client, &url, |hop| {
            prepare_request(&shared, &cfg, hop, &url, deadline)
        }) {
            Ok(mut result) => {
                presence.set("saving", &domain);
                result.page.depth = depth as i32;
                let child_depth = depth.saturating_add(1);
                let links = result.page.links.clone();
                {
                    let guard = conn.lock().unwrap();
                    database::save_crawled_page(&guard, &result.page, result.http_code)?;
                    shared.counts.lock().unwrap().ok += 1;
                }

                let capacity = if cfg.discover_links {
                    (100_000 - database::count_urls_by_status(&conn.lock().unwrap(), "pending")?)
                        .clamp(0, 500) as usize
                } else {
                    0
                };
                for link in links.into_iter().take(capacity) {
                    if link.is_external && !cfg.follow_external {
                        continue;
                    }
                    if child_depth > cfg.max_depth {
                        continue;
                    }
                    if let Some(clean) = crawler::normalize_url(&url, &link.url) {
                        let guard = conn.lock().unwrap();
                        database::enqueue(&guard, &clean, child_depth, 0)?;
                        if link.safety != "safe" {
                            database::set_url_safety(&guard, &clean, &link.safety, 0, "")?;
                        }
                    }
                }
            }
            Err(crawler::CrawlError::Blocked(reason)) => {
                let guard = conn.lock().unwrap();
                database::mark_blocked(&guard, &url, &reason)?;
                shared.counts.lock().unwrap().blocked += 1;
            }
            Err(crawler::CrawlError::Skipped(reason)) => {
                let guard = conn.lock().unwrap();
                database::mark_skipped(&guard, &url, &reason)?;
                shared.counts.lock().unwrap().skipped += 1;
            }
            Err(crawler::CrawlError::Failed(reason)) => {
                let guard = conn.lock().unwrap();
                database::mark_failed(&guard, &url, &reason)?;
                guard.execute("UPDATE urls SET status='pending', next_attempt=unixepoch()+CASE attempts WHEN 1 THEN 30 ELSE 120 END WHERE url=?1 AND attempts<3", [&url])?;
                shared.counts.lock().unwrap().failed += 1;
            }
            Err(crawler::CrawlError::Deadline) => {
                if Instant::now() < deadline && !shared.stop.load(Ordering::Acquire) {
                    let wait = shared
                        .domain_next
                        .lock()
                        .unwrap()
                        .get(&domain)
                        .copied()
                        .map(|at| {
                            at.saturating_duration_since(Instant::now())
                                .as_secs()
                                .saturating_add(1)
                        })
                        .unwrap_or(1)
                        .clamp(1, 86400);
                    conn.lock().unwrap().execute(
                        "UPDATE urls SET status='pending',attempts=max(0,attempts-1),next_attempt=max(next_attempt,unixepoch()+?2) WHERE url=?1 AND status='crawling'",
                        rusqlite::params![url, wait as i64],
                    )?;
                    continue;
                }
                release_claim(&conn.lock().unwrap(), &url)?;
                break;
            }
            Err(crawler::CrawlError::Deferred { reason, after_secs }) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let next = i64::try_from(now.saturating_add(after_secs)).unwrap_or(i64::MAX);
                let guard = conn.lock().unwrap();
                guard.execute("UPDATE urls SET status=CASE WHEN attempts < 3 THEN 'pending' ELSE 'failed' END, next_attempt=?2, error=?3, updated_at=CURRENT_TIMESTAMP WHERE url=?1", rusqlite::params![url, next, reason])?;
                let mut domains = shared.domain_next.lock().unwrap();
                let until = Instant::now()
                    .checked_add(Duration::from_secs(after_secs))
                    .unwrap_or(deadline);
                let slot = domains.entry(domain).or_insert(until);
                *slot = (*slot).max(until);
                shared.counts.lock().unwrap().failed += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_test_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "vertex-{name}-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ))
    }

    #[test]
    fn draining_ignores_seeds_and_does_not_recrawl_old_pages() {
        let dir = drain_test_dir("drain-empty");
        let conn = database::init_in(&dir).unwrap();
        database::enqueue(&conn, "https://example.com/old", 0, 0).unwrap();
        database::mark_crawled(&conn, "https://example.com/old", 200).unwrap();
        conn.execute("INSERT INTO pages(url,title,crawled_at) VALUES('https://example.com/old','old',datetime('now','-3 days'))",[]).unwrap();
        drop(conn);
        run_continuous(
            &dir,
            &["https://example.com/new"],
            ContinuousConfig {
                drain_existing: true,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let conn = database::init_in(&dir).unwrap();
        assert_eq!(database::count_urls_by_status(&conn, "pending").unwrap(), 0);
        assert!(!database::url_exists(&conn, "https://example.com/new").unwrap());
        assert_eq!(database::count_pages(&conn).unwrap(), 1);
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn draining_processes_deep_existing_urls_without_networking_to_private_hosts() {
        let dir = drain_test_dir("drain-deep");
        let conn = database::init_in(&dir).unwrap();
        database::enqueue(&conn, "http://127.0.0.1/private", 999, 0).unwrap();
        drop(conn);
        run_continuous(
            &dir,
            &[],
            ContinuousConfig {
                drain_existing: true,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let conn = database::init_in(&dir).unwrap();
        assert_eq!(database::count_urls_by_status(&conn, "pending").unwrap(), 0);
        assert_eq!(database::count_urls_by_status(&conn, "skipped").unwrap(), 1);
        assert_eq!(database::count_pages(&conn).unwrap(), 0);
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn storage_limit_preserves_unprocessed_urls() {
        let dir = drain_test_dir("drain-quota");
        let conn = database::init_in(&dir).unwrap();
        database::enqueue(&conn, "https://example.com/pending", 0, 0).unwrap();
        drop(conn);
        let result = run_continuous(
            &dir,
            &[],
            ContinuousConfig {
                drain_existing: true,
                max_storage_bytes: 1,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
        );
        assert!(result.unwrap_err().contains("Quota"));
        let conn = database::init_in(&dir).unwrap();
        assert_eq!(database::count_urls_by_status(&conn, "pending").unwrap(), 1);
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn claim_uses_an_ordered_partial_index_without_sorting_the_whole_frontier() {
        let dir = drain_test_dir("claim-plan");
        let conn = database::init_in(&dir).unwrap();
        let mut statement=conn.prepare("EXPLAIN QUERY PLAN SELECT id FROM urls INDEXED BY idx_urls_claim WHERE status='pending' AND safety!='malicious' AND attempts<3 AND next_attempt<=unixepoch() AND depth<=20 ORDER BY priority DESC,id ASC LIMIT 1").unwrap();
        let plans: Vec<String> = statement
            .query_map([], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(plans.iter().any(|line| line.contains("idx_urls_claim")));
        assert!(!plans.iter().any(|line| line.contains("TEMP B-TREE")));
        drop(statement);
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn continuous_stop_releases_lease_without_network() {
        let dir = std::env::temp_dir().join(format!(
            "vertex-stop-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        let stop = Arc::new(AtomicBool::new(true));
        run_continuous(&dir, &[], ContinuousConfig::default(), stop).unwrap();
        assert!(!crawl_is_running(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_daemon_can_be_interrupted_while_idle() {
        let dir = std::env::temp_dir().join(format!(
            "vertex-idle-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_dir = dir.clone();
        let worker = thread::spawn(move || {
            run_continuous(&worker_dir, &[], ContinuousConfig::default(), worker_stop)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !crawl_is_running(&dir) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(crawl_is_running(&dir));
        let stopped = Instant::now();
        stop.store(true, Ordering::Release);
        worker.join().unwrap().unwrap();
        assert!(stopped.elapsed() < Duration::from_secs(3));
        assert!(!crawl_is_running(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn live_session_cannot_be_reset_by_another_crawler() {
        let dir = std::env::temp_dir().join(format!(
            "vertex-lock-test-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        let conn = database::init_in(&dir).unwrap();
        database::enqueue(&conn, "https://example.com/live", 0, 0).unwrap();
        database::mark_crawling(&conn, "https://example.com/live").unwrap();
        let lease = CrawlLease::acquire(&dir).unwrap();
        assert!(crawl_is_running(&dir));
        let summary = run_crawl_with(&dir, &[], CrawlConfig::default());
        assert!(!summary.errors.is_empty());
        assert_eq!(
            database::count_urls_by_status(&conn, "crawling").unwrap(),
            1
        );
        drop(lease);
        assert!(!crawl_is_running(&dir));
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deferred_or_exhausted_urls_are_not_claimed() {
        let dir = std::env::temp_dir().join(format!(
            "vertex-retry-test-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        let conn = database::init_in(&dir).unwrap();
        database::enqueue(&conn, "https://example.com/later", 0, 0).unwrap();
        conn.execute("UPDATE urls SET next_attempt = unixepoch()+600", [])
            .unwrap();
        assert!(take_next(&conn, 2).unwrap().is_none());
        conn.execute("UPDATE urls SET next_attempt=0, attempts=3", [])
            .unwrap();
        assert!(take_next(&conn, 2).unwrap().is_none());
        conn.execute("UPDATE urls SET attempts=2", []).unwrap();
        assert!(take_next(&conn, 2).unwrap().is_some());
        release_claim(&conn, "https://example.com/later").unwrap();
        assert!(take_next(&conn, 2).unwrap().is_some());
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn redirected_destination_is_checked_against_its_own_robots() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut paths = Vec::new();
            while paths.len() < 3 && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    request.push_str(&line);
                }
                let response = if request.starts_with("GET /start ") {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://redirect.example:{}/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        address.port()
                    )
                } else {
                    let body = if request.contains("redirect.example") {
                        "User-agent: *\nDisallow: /secret\n"
                    } else {
                        "User-agent: *\nAllow: /\n"
                    };
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                paths.push(request);
                stream.write_all(response.as_bytes()).unwrap();
            }
            paths
        });
        // Only this isolated fixture client maps public-looking names to loopback.
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve("fixture.example", address)
            .resolve("redirect.example", address)
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let mut shared = Shared::new();
        shared.aux_client = client.clone();
        let cfg = CrawlConfig {
            per_domain_delay_ms: 0,
            ..CrawlConfig::default()
        };
        let url = format!("http://fixture.example:{}/start", address.port());
        let result = crawler::crawl_with(&client, &url, |hop| {
            prepare_request(
                &shared,
                &cfg,
                hop,
                &url,
                Instant::now() + Duration::from_secs(5),
            )
        });
        assert!(matches!(result, Err(crawler::CrawlError::Skipped(_))));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(
            !requests
                .iter()
                .any(|request| request.starts_with("GET /secret "))
        );
    }

    #[test]
    fn claims_each_url_once_and_respects_depth() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT, depth INTEGER, status TEXT, safety TEXT, priority INTEGER, attempts INTEGER, updated_at TEXT, next_attempt INTEGER DEFAULT 0); CREATE INDEX idx_urls_claim ON urls(priority DESC,id ASC) WHERE status='pending' AND safety!='malicious' AND attempts<3;").unwrap();
        conn.execute_batch("INSERT INTO urls VALUES (1, 'https://example.com/', 0, 'pending', 'safe', 0, 0, NULL, 0), (2, 'https://example.com/deep', 9, 'pending', 'safe', 0, 0, NULL, 0);").unwrap();
        assert!(take_next(&conn, 2).unwrap().is_some());
        assert!(take_next(&conn, 2).unwrap().is_none());
        assert!(take_next(&conn, 9).unwrap().is_some());
    }

    #[test]
    fn excessive_crawl_delay_does_not_overrun_deadline() {
        let shared = Shared::new();
        shared.domain_next.lock().unwrap().insert(
            "example.com".into(),
            Instant::now() + Duration::from_secs(3600),
        );
        assert!(!shared.respect_delay(
            "example.com",
            1000,
            Instant::now() + Duration::from_millis(100)
        ));
    }

    #[test]
    fn sitemap_decodes_entities_and_keeps_domain_scope() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT UNIQUE, domain TEXT, status TEXT, depth INTEGER, priority INTEGER, safety TEXT);").unwrap();
        conn.execute_batch("CREATE TABLE pages(id INTEGER PRIMARY KEY);")
            .unwrap();
        crate::monitor::install(&conn).unwrap();
        store_sitemap(&conn, "<urlset><url><loc>https://example.com/?a=1&amp;b=2</loc></url><url><loc>https://other.com/</loc></url></urlset>", 1, "https://example.com/", false).unwrap();
        let urls = database::all_urls(&conn).unwrap();
        assert_eq!(urls, ["https://example.com/?a=1&b=2"]);
    }

    #[test]
    fn heartbeat_tracks_real_worker_presence_and_goes_offline_on_drop() {
        let dir = std::env::temp_dir().join(format!("vertex-heartbeat-{}", std::process::id()));
        let conn = database::init_in(&dir).unwrap();
        let shared = Arc::new(Shared::new());
        *shared.workers.lock().unwrap() = vec![crate::monitor::Worker {
            id: 1,
            ..Default::default()
        }];
        let presence = WorkerPresence {
            shared: shared.clone(),
            id: 0,
        };
        presence.set("robots", "example.com");
        let heartbeat = Heartbeat::start(&dir, shared, 1, "drain");
        let mut observed = false;
        for _ in 0..40 {
            if let Ok(payload) =
                conn.query_row("SELECT payload FROM crawler_runtime WHERE id=1", [], |r| {
                    r.get::<_, String>(0)
                })
            {
                let runtime: crate::monitor::Runtime = serde_json::from_str(&payload).unwrap();
                observed = runtime.workers[0].online && runtime.workers[0].domain == "example.com";
                if observed {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(observed);
        drop(presence);
        drop(heartbeat);
        let payload: String = conn
            .query_row("SELECT payload FROM crawler_runtime WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        let runtime: crate::monitor::Runtime = serde_json::from_str(&payload).unwrap();
        assert!(!runtime.workers[0].online);
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn page_budget_is_exact_and_summary_is_per_run() {
        let dir = std::env::temp_dir().join(format!(
            "vertex-run-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let conn = database::init_in(&dir).unwrap();
        database::enqueue(&conn, "https://example.com/old", 0, 0).unwrap();
        database::mark_crawled(&conn, "https://example.com/old", 200).unwrap();
        drop(conn);
        let cfg = CrawlConfig {
            duration_secs: 5,
            workers: 12,
            max_pages: Some(2),
            respect_robots: false,
            ..CrawlConfig::default()
        };
        let summary = run_crawl_with(
            &dir,
            &[
                "http://127.0.0.1/one",
                "http://127.0.0.1/two",
                "http://127.0.0.1/three",
            ],
            cfg,
        );
        assert_eq!(summary.pages_ok, 0);
        assert_eq!(summary.pages_skipped, 2);
        assert_eq!(summary.pending, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
