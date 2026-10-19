use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use serde::{Deserialize, Serialize};
static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub background: bool,
    pub share_origin: Option<String>,
}

pub fn settings(dir: &Path) -> Settings {
    std::fs::read(dir.join("contributor.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn stop(dir: &Path) -> Result<(), String> {
    std::fs::write(dir.join("contributor.stop"), b"stop").map_err(|e| e.to_string())
}

pub fn configure(
    dir: &Path,
    enabled: bool,
    background: bool,
    share_origin: Option<&str>,
) -> Result<(), String> {
    let _guard = SETTINGS_LOCK.lock().map_err(|e| e.to_string())?;
    if enabled && !settings(dir).enabled && crawler_rs::runner::crawl_is_running(dir) {
        return Err(
            "Attendre l'arret du crawl en cours avant d'activer l'exploration continue".into(),
        );
    }
    if let Some(origin) = share_origin {
        search_rs::shared::SharedIndex::new(origin)?;
    }
    save_settings(
        dir,
        &Settings {
            enabled,
            background,
            share_origin: if enabled {
                share_origin.map(str::to_string)
            } else {
                None
            },
        },
    )?;
    if !enabled {
        return stop(dir);
    }
    start(dir)
}

fn save_settings(dir: &Path, value: &Settings) -> Result<(), String> {
    if value.share_origin.is_some() && settings(dir).share_origin != value.share_origin {
        // Start at consent time: older pages must not become a shared history.
        let conn = rusqlite::Connection::open(dir.join("vertex.db")).map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        conn.execute("INSERT INTO meta(key,value) VALUES('shared_discovery_cursor',(SELECT coalesce(max(id),0) FROM pages)) ON CONFLICT(key) DO UPDATE SET value=excluded.value", []).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let temp = dir.join(format!("contributor-{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(temp, dir.join("contributor.json")).map_err(|e| e.to_string())
}

pub fn start(dir: &Path) -> Result<(), String> {
    if !settings(dir).enabled || crawler_rs::runner::crawl_is_running(dir) {
        return Ok(());
    }
    match std::fs::remove_file(dir.join("contributor.stop")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let _ = std::fs::remove_file(dir.join("contributor-error.txt"));
    let mut child = std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    child
        .arg("--vertex-contributor")
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x08000000); // CREATE_NO_WINDOW: the app owns the visible controls.
    }
    let mut child = child.spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// A separately consented worker can survive closing the window, not a reboot.
pub fn run_if_requested() -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--vertex-contributor")) {
        return false;
    }
    let Some(dir) = args.next().map(PathBuf::from) else {
        return true;
    };
    if !settings(&dir).enabled {
        return true;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let watched_dir = dir.clone();
    let watched_stop = stop.clone();
    std::thread::spawn(move || {
        while !watched_stop.load(Ordering::Acquire) {
            if watched_dir.join("contributor.stop").exists() || !settings(&watched_dir).enabled {
                watched_stop.store(true, Ordering::Release);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    {
        let dir = dir.clone();
        let stop = stop.clone();
        // Disabled by default. The saved origin is the destination accepted in
        // the separate sharing prompt, never a redirect chosen by a page.
        std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            while !stop.load(Ordering::Acquire) {
                if let Some(shared) = settings(&dir)
                    .share_origin
                    .and_then(|origin| search_rs::shared::SharedIndex::new(&origin).ok())
                {
                    let _ = runtime.block_on(suggest_discoveries(&dir, &shared, &stop));
                }
                for _ in 0..300 {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        });
    }
    let config = crawler_rs::runner::ContinuousConfig {
        crawl: crawler_rs::runner::CrawlConfig {
            workers: 1,
            per_domain_delay_ms: 3000,
            max_pages: Some(50),
            ..Default::default()
        },
        idle_secs: 60,
        max_storage_bytes: 512 * 1024 * 1024,
        ..Default::default()
    };
    if let Err(error) =
        crawler_rs::runner::run_continuous(&dir, crawler_rs::SEEDS, config, stop.clone())
    {
        let _ = std::fs::write(dir.join("contributor-error.txt"), error.as_bytes());
        let _ = configure(&dir, false, false, None);
    }
    stop.store(true, Ordering::Release);
    true
}

/// Proposals are URLs, not trusted page contents. The central server fetches
/// and validates its own copy. Query parameters and private tables never leave.
async fn suggest_discoveries(
    dir: &Path,
    shared: &search_rs::shared::SharedIndex,
    stop: &AtomicBool,
) -> Result<(), String> {
    let conn = rusqlite::Connection::open_with_flags(
        dir.join("vertex.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )
    .map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    let cursor = conn
        .query_row(
            "SELECT value FROM meta WHERE key='shared_discovery_cursor'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    let pages: Vec<(i64, String)> = {
        let mut statement = conn
            .prepare("SELECT id,url FROM pages WHERE id>?1 ORDER BY id LIMIT 5")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([cursor], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(|e| e.to_string())?
    };
    for (id, url) in pages {
        if stop.load(Ordering::Acquire)
            || settings(dir).share_origin.as_deref() != Some(shared.origin())
        {
            break;
        }
        if let Some(url) = crawler_rs::runner::discovery_url(&url)
            .filter(|url| crawler_rs::network::validate_url(url).is_ok_and(|u| u.query().is_none()))
        {
            shared.discover(&url).await?;
        }
        conn.execute("INSERT INTO meta(key,value) VALUES('shared_discovery_cursor',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [id.to_string()]).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn participation_is_disabled_without_explicit_settings() {
        let dir = std::env::temp_dir().join("vertex-no-contribution-settings");
        assert!(!settings(&dir).enabled);
        assert!(!settings(&dir).background);
        assert!(settings(&dir).share_origin.is_none());
    }

    #[test]
    fn enabling_future_sharing_does_not_replay_old_pages_and_settings_replace_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crawler_rs::database::init_in(dir.path()).unwrap();
        conn.execute(
            "INSERT INTO pages(url,title) VALUES('https://example.com/old','Before consent')",
            [],
        )
        .unwrap();
        let value = Settings {
            enabled: true,
            background: false,
            share_origin: Some("https://index.example.com".into()),
        };
        save_settings(dir.path(), &value).unwrap();
        let cursor: i64 = conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='shared_discovery_cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cursor, 1);
        assert_eq!(settings(dir.path()).share_origin, value.share_origin);
        save_settings(dir.path(), &Settings::default()).unwrap();
        assert!(!settings(dir.path()).enabled);
        assert!(settings(dir.path()).share_origin.is_none());
    }
}
