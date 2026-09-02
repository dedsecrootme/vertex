//! Exécution d'un crawl complet (multi-thread, budget de temps).
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use rusqlite::Connection;

use crate::{crawler, database};

const NUM_WORKERS: usize = 6;
const REQUEST_DELAY_MS: u64 = 250;

/// Résumé d'un crawl.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CrawlSummary {
    pub pages_ok: i64,
    pub pages_failed: i64,
    pub pending: i64,
    pub sites: usize,
    pub elapsed_secs: u64,
}

/// Lance un crawl multi-thread pendant `duration_secs` secondes et renvoie
/// un résumé une fois terminé.
pub fn run_crawl(duration_secs: u64, seeds: &[&str]) -> CrawlSummary {
    let db = database::init().expect("❌ Impossible d'ouvrir SQLite");
    let db = Arc::new(Mutex::new(db));

    database::reset_crawling(&db.lock().unwrap()).expect("❌ reset");
    for seed in seeds {
        database::add_url(&db.lock().unwrap(), seed).expect("❌ ajout seed");
    }

    let start = Instant::now();
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("❌ client HTTP");

    let mut handles = Vec::new();
    for _ in 0..NUM_WORKERS {
        let db = Arc::clone(&db);
        let client = client.clone();
        handles.push(thread::spawn(move || worker(db, client, start, duration_secs)));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let conn = db.lock().unwrap();
    let pages_ok = database::count_urls_by_status(&conn, "done").unwrap_or(0);
    let pages_failed = database::count_urls_by_status(&conn, "failed").unwrap_or(0);
    let pending = database::count_urls_by_status(&conn, "pending").unwrap_or(0);

    let mut domains: HashSet<String> = HashSet::new();
    for u in database::all_urls(&conn).unwrap_or_default() {
        let d = crawler::extract_domain(&u);
        if !d.is_empty() {
            domains.insert(d);
        }
    }

    CrawlSummary {
        pages_ok,
        pages_failed,
        pending,
        sites: domains.len(),
        elapsed_secs: start.elapsed().as_secs(),
    }
}

fn worker(db: Arc<Mutex<Connection>>, client: Client, start: Instant, duration_secs: u64) {
    loop {
        if start.elapsed().as_secs() >= duration_secs {
            break;
        }

        let url = {
            let conn = db.lock().unwrap();
            match database::get_next_url(&conn) {
                Ok(Some(url)) => url,
                Ok(None) => {
                    drop(conn);
                    thread::sleep(Duration::from_millis(300));
                    continue;
                }
                Err(_) => {
                    drop(conn);
                    thread::sleep(Duration::from_millis(300));
                    continue;
                }
            }
        };

        {
            let conn = db.lock().unwrap();
            database::mark_crawling(&conn, &url).unwrap();
        }

        match crawler::crawl(&client, &url) {
            Ok(result) => {
                let page = result.page;
                let http_code = result.http_code;

                {
                    let conn = db.lock().unwrap();
                    database::save_page(&conn, &page).unwrap();
                    database::mark_crawled(&conn, &url, http_code).unwrap();
                }

                for link in page.links {
                    if let Some(clean) = crawler::normalize_url(&url, &link) {
                        let conn = db.lock().unwrap();
                        if !database::url_exists(&conn, &clean).unwrap_or(false) {
                            database::add_url(&conn, &clean).unwrap();
                        }
                    }
                }
            }
            Err(error) => {
                let conn = db.lock().unwrap();
                database::mark_failed(&conn, &url, &error.to_string()).unwrap();
            }
        }

        thread::sleep(Duration::from_millis(REQUEST_DELAY_MS));
    }
}