//! Moteur de crawl complet : concurrence, politesse par domaine, respect de
//! `robots.txt`, découverte des `sitemap.xml`, limite de profondeur, reprise
//! après arrêt et journalisation des exécutions.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use rusqlite::{Connection, OptionalExtension};

use crate::{crawler, database, safety};

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
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            duration_secs: 300,
            workers: 12,
            max_depth: 6,
            per_domain_delay_ms: 250,
            request_timeout_secs: 8,
            max_pages: None,
            follow_external: true,
            respect_robots: true,
        }
    }
}

/// Résumé d'un crawl.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CrawlSummary {
    pub run_id: i64,
    pub pages_ok: i64,
    pub pages_failed: i64,
    pub pages_blocked: i64,
    pub pages_skipped: i64,
    pub pending: i64,
    pub sites: i64,
    pub elapsed_secs: u64,
}

/// Règles `robots.txt` d'un domaine.
#[derive(Debug, Clone, Default)]
struct RobotsRules {
    crawl_delay_ms: Option<u64>,
    disallow: Vec<String>,
    allow: Vec<String>,
    sitemaps: Vec<String>,
}

impl RobotsRules {
    fn allows(&self, path_query: &str) -> bool {
        let mut best_len = -1isize;
        let mut allowed = true;
        for rule in &self.allow {
            if !rule.is_empty() && path_query.starts_with(rule.as_str()) && rule.len() as isize > best_len
            {
                best_len = rule.len() as isize;
                allowed = true;
            }
        }
        for rule in &self.disallow {
            if !rule.is_empty() && path_query.starts_with(rule.as_str()) && rule.len() as isize > best_len
            {
                best_len = rule.len() as isize;
                allowed = false;
            }
        }
        allowed
    }
}

/// Analyse un `robots.txt` en retenant les règles du groupe le plus spécifique
/// (`VertexCrawler`) puis, à défaut, du groupe `*`.
fn parse_robots(body: &str) -> RobotsRules {
    let mut specific = RobotsRules::default();
    let mut wildcard = RobotsRules::default();
    let mut sitemaps = Vec::new();
    let mut current: Option<bool> = None; // Some(true) = spécifique, Some(false) = wildcard

    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        match key.as_str() {
            "user-agent" => {
                let v = value.to_ascii_lowercase();
                current = if v == "*" {
                    Some(false)
                } else if v.contains("vertex") {
                    Some(true)
                } else {
                    None
                };
            }
            "disallow" | "allow" if current.is_some() => {
                let target = if current == Some(true) {
                    &mut specific
                } else {
                    &mut wildcard
                };
                if key == "disallow" {
                    target.disallow.push(value.to_string());
                } else {
                    target.allow.push(value.to_string());
                }
            }
            "crawl-delay" if current.is_some() => {
                if let Ok(secs) = value.parse::<f64>() {
                    let ms = (secs * 1000.0).round() as u64;
                    let target = if current == Some(true) {
                        &mut specific
                    } else {
                        &mut wildcard
                    };
                    target.crawl_delay_ms = Some(ms);
                }
            }
            "sitemap" => sitemaps.push(value.to_string()),
            _ => {}
        }
    }

    let mut rules = if specific.disallow.is_empty() && specific.allow.is_empty() && specific.crawl_delay_ms.is_none()
    {
        wildcard
    } else {
        if specific.crawl_delay_ms.is_none() {
            specific.crawl_delay_ms = wildcard.crawl_delay_ms;
        }
        specific
    };
    rules.sitemaps = sitemaps;
    rules
}

/// État partagé par tous les workers.
struct Shared {
    robots: Mutex<HashMap<String, RobotsRules>>,
    domain_next: Mutex<HashMap<String, Instant>>,
    sitemap_done: Mutex<HashSet<String>>,
    pages_done: Mutex<i64>,
    /// Client dédié aux fichiers annexes (robots.txt, sitemaps) : délai court
    /// pour ne jamais bloquer un worker sur un domaine lent.
    aux_client: Client,
}

impl Shared {
    fn new() -> Self {
        Self {
            robots: Mutex::new(HashMap::new()),
            domain_next: Mutex::new(HashMap::new()),
            sitemap_done: Mutex::new(HashSet::new()),
            pages_done: Mutex::new(0),
            aux_client: crawler::build_client(3),
        }
    }

    /// Attend si nécessaire pour respecter le délai par domaine.
    fn respect_delay(&self, domain: &str, delay_ms: u64) {
        if delay_ms == 0 {
            return;
        }
        let wait_until = {
            let mut map = self.domain_next.lock().unwrap();
            let now = Instant::now();
            let slot = map.entry(domain.to_string()).or_insert(now);
            let wait = if *slot > now { *slot } else { now };
            *slot = wait + Duration::from_millis(delay_ms);
            wait
        };
        let now = Instant::now();
        if wait_until > now {
            thread::sleep(wait_until - now);
        }
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
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
fn fetch_text(client: &Client, url: &str) -> Option<String> {
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().ok()?;
    if body.len() > 2_000_000 {
        return None;
    }
    Some(body)
}

/// Charge (et met en cache) les règles `robots.txt` d'un domaine, et découvre
/// les sitemaps déclarés à la première visite du domaine.
/// Charge (et met en cache) les règles `robots.txt`. Renvoie aussi les sitemaps à
/// explorer lors de la première visite d'un domaine joignable.
///
/// **Aucun verrou de base n'est pris ici** : les appels réseau ne bloquent donc
/// pas les autres workers.
fn load_robots(shared: &Shared, base_url: &str) -> (RobotsRules, Vec<String>) {
    let domain = host_of(base_url);
    if let Some(rules) = shared.robots.lock().unwrap().get(&domain) {
        return (rules.clone(), Vec::new());
    }

    let scheme = url::Url::parse(base_url)
        .map(|u| u.scheme().to_string())
        .unwrap_or_else(|_| "https".to_string());
    let robots_url = format!("{scheme}://{domain}/robots.txt");

    // Délai court dédié : un domaine lent ne doit pas immobiliser un worker.
    let fetched = fetch_text(&shared.aux_client, &robots_url);
    let reachable = fetched.is_some();
    let rules = fetched.map(|body| parse_robots(&body)).unwrap_or_default();

    // Sitemaps : une seule fois par domaine, et uniquement si le site répond.
    let first_visit = shared.sitemap_done.lock().unwrap().insert(domain.clone());
    let sitemaps = if first_visit && reachable {
        let mut list = rules.sitemaps.clone();
        if list.is_empty() {
            list.push(format!("{scheme}://{domain}/sitemap.xml"));
        }
        list.into_iter().take(2).collect()
    } else {
        Vec::new()
    };

    shared.robots.lock().unwrap().insert(domain, rules.clone());
    (rules, sitemaps)
}

/// Enregistre en base les URL d'un sitemap **déjà téléchargé** (aucun réseau ici).
fn store_sitemap(conn: &Connection, body: &str, depth: i64) {
    const MAX_URLS: usize = 2000;
    let mut added = 0usize;
    for chunk in body.split("<loc>").skip(1) {
        let Some(rest) = chunk.split("</loc>").next() else {
            continue;
        };
        let loc = rest.trim();
        if loc.is_empty() || loc.ends_with(".xml") || loc.ends_with(".xml.gz") {
            // Index de sitemaps : ignoré (pas de descente récursive).
            continue;
        }
        if let Some(clean) = crawler::normalize_url(loc, loc)
            && safety::check_url(&clean).verdict != safety::Verdict::Malicious
        {
            let _ = database::enqueue(conn, &clean, depth, 5);
            added += 1;
        }
        if added >= MAX_URLS {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Boucle de crawl
// ---------------------------------------------------------------------------

/// Sélectionne et réserve (atomiquement) la prochaine URL à traiter.
fn take_next(conn: &Connection) -> Option<(String, i64)> {
    conn.query_row(
        "SELECT url, depth FROM urls
         WHERE status = 'pending' AND safety != 'malicious'
         ORDER BY priority DESC, id ASC
         LIMIT 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    )
    .optional()
    .ok()
    .flatten()
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
    let conn = database::init_in(db_dir).expect("Impossible d'ouvrir SQLite");

    // Charge la blocklist utilisateur si elle existe.
    let _ = safety::load_user_blocklist(&database::blocklist_path(db_dir));

    database::reset_crawling(&conn).ok();
    for seed in seeds {
        database::enqueue(&conn, seed, 0, 10).ok();
    }

    let run_id = database::run_start(&conn, cfg.duration_secs as i64).unwrap_or(0);

    let conn = Arc::new(Mutex::new(conn));
    let client = crawler::build_client(cfg.request_timeout_secs);
    let shared = Arc::new(Shared::new());
    let cfg = Arc::new(cfg);
    let start = Instant::now();

    let mut handles = Vec::new();
    for _ in 0..cfg.workers.max(1) {
        let conn = Arc::clone(&conn);
        let client = client.clone();
        let shared = Arc::clone(&shared);
        let cfg = Arc::clone(&cfg);
        handles.push(thread::spawn(move || worker(conn, client, shared, cfg, start)));
    }
    for handle in handles {
        let _ = handle.join();
    }

    let conn = conn.lock().unwrap();
    // Alimente la colonne `domain` des URL héritées de la v1, une fois le crawl
    // terminé (n'affecte donc pas le débit de crawl).
    let _ = database::backfill_domains(&conn, 200_000);
    let pages_ok = database::count_urls_by_status(&conn, "done").unwrap_or(0);
    let pages_failed = database::count_urls_by_status(&conn, "failed").unwrap_or(0);
    let pages_blocked = database::count_urls_by_status(&conn, "blocked").unwrap_or(0);
    let pages_skipped = database::count_urls_by_status(&conn, "skipped").unwrap_or(0);
    let pending = database::count_urls_by_status(&conn, "pending").unwrap_or(0);
    let sites = database::count_domains(&conn).unwrap_or(0);
    let _ = database::run_finish(&conn, run_id, pages_ok, pages_failed, pages_ok + pages_failed);
    let _ = database::optimize(&conn);

    CrawlSummary {
        run_id,
        pages_ok,
        pages_failed,
        pages_blocked,
        pages_skipped,
        pending,
        sites,
        elapsed_secs: start.elapsed().as_secs(),
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
) {
    loop {
        if start.elapsed().as_secs() >= cfg.duration_secs {
            break;
        }
        if let Some(max) = cfg.max_pages
            && *shared.pages_done.lock().unwrap() >= max
        {
            break;
        }

        let next = {
            let guard = conn.lock().unwrap();
            match take_next(&guard) {
                Some((url, depth)) => {
                    database::mark_crawling(&guard, &url).ok();
                    Some((url, depth))
                }
                None => None,
            }
        };

        let Some((url, depth)) = next else {
            thread::sleep(Duration::from_millis(400));
            continue;
        };

        let domain = host_of(&url);

        let mut delay_ms = cfg.per_domain_delay_ms;
        if cfg.respect_robots {
            // Tout le réseau se fait HORS verrou de base (workers réellement parallèles).
            let (rules, sitemaps) = load_robots(&shared, &url);

            if !sitemaps.is_empty() && cfg.follow_external {
                let bodies: Vec<String> = sitemaps
                    .iter()
                    .filter_map(|s| fetch_text(&shared.aux_client, s))
                    .collect();
                if !bodies.is_empty() {
                    let guard = conn.lock().unwrap();
                    for body in &bodies {
                        store_sitemap(&guard, body, depth + 1);
                    }
                }
            }

            if !rules.allows(&path_query_of(&url)) {
                let guard = conn.lock().unwrap();
                database::mark_skipped(&guard, &url, "Interdit par robots.txt").ok();
                continue;
            }
            if let Some(cd) = rules.crawl_delay_ms {
                delay_ms = delay_ms.max(cd);
            }
        }
        shared.respect_delay(&domain, delay_ms);

        match crawler::crawl(&client, &url) {
            Ok(result) => {
                let child_depth = depth + 1;
                let links = result.page.links.clone();
                {
                    let guard = conn.lock().unwrap();
                    database::save_page(&guard, &result.page).ok();
                    database::mark_crawled(&guard, &url, result.http_code).ok();
                    *shared.pages_done.lock().unwrap() += 1;
                }

                for link in links {
                    if link.is_external && !cfg.follow_external {
                        continue;
                    }
                    if child_depth > cfg.max_depth {
                        continue;
                    }
                    if let Some(clean) = crawler::normalize_url(&url, &link.url) {
                        let guard = conn.lock().unwrap();
                        database::enqueue(&guard, &clean, child_depth, 0).ok();
                        if link.safety != "safe" {
                            database::set_url_safety(&guard, &clean, &link.safety, 0, "").ok();
                        }
                    }
                }
            }
            Err(crawler::CrawlError::Blocked(reason)) => {
                let guard = conn.lock().unwrap();
                database::mark_blocked(&guard, &url, &reason).ok();
            }
            Err(crawler::CrawlError::Skipped(reason)) => {
                let guard = conn.lock().unwrap();
                database::mark_skipped(&guard, &url, &reason).ok();
            }
            Err(crawler::CrawlError::Failed(reason)) => {
                let guard = conn.lock().unwrap();
                database::mark_failed(&guard, &url, &reason).ok();
            }
        }
    }
}
