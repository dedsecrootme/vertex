//! Transactional counters and process heartbeats, shared by crawler and API.
use rusqlite::{Connection, Result};
use serde::{Deserialize, Serialize};

pub fn free_space(dir: &std::path::Path) -> std::io::Result<u64> {
    fs2::available_space(dir)
}

pub fn install(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS monitor_counts(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS monitor_domains(domain TEXT PRIMARY KEY, refs INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS crawler_runtime(id INTEGER PRIMARY KEY CHECK(id=1), heartbeat INTEGER NOT NULL, payload TEXT NOT NULL);")?;
    let ready: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM monitor_counts WHERE key='ready')",
        [],
        |r| r.get(0),
    )?;
    if !ready {
        tx.execute_batch("DELETE FROM monitor_counts; DELETE FROM monitor_domains;
            INSERT INTO monitor_counts SELECT 'urls:'||status,count(*) FROM urls GROUP BY status;
            INSERT INTO monitor_counts VALUES('pages',(SELECT count(*) FROM pages));
            INSERT INTO monitor_domains SELECT domain,count(*) FROM urls WHERE domain IS NOT NULL AND domain!='' GROUP BY domain;
            INSERT INTO monitor_counts VALUES('ready',1);")?;
    }
    tx.execute_batch("CREATE TRIGGER IF NOT EXISTS monitor_url_insert AFTER INSERT ON urls BEGIN
        INSERT INTO monitor_counts VALUES('urls:'||new.status,1) ON CONFLICT(key) DO UPDATE SET value=value+1;
        INSERT INTO monitor_domains SELECT new.domain,1 WHERE new.domain IS NOT NULL AND new.domain!='' ON CONFLICT(domain) DO UPDATE SET refs=refs+1;
    END;
    CREATE TRIGGER IF NOT EXISTS monitor_url_delete AFTER DELETE ON urls BEGIN
        UPDATE monitor_counts SET value=value-1 WHERE key='urls:'||old.status;
        UPDATE monitor_domains SET refs=refs-1 WHERE domain=old.domain;
    END;
    CREATE TRIGGER IF NOT EXISTS monitor_url_status AFTER UPDATE OF status ON urls WHEN old.status!=new.status BEGIN
        UPDATE monitor_counts SET value=value-1 WHERE key='urls:'||old.status;
        INSERT INTO monitor_counts VALUES('urls:'||new.status,1) ON CONFLICT(key) DO UPDATE SET value=value+1;
    END;
    CREATE TRIGGER IF NOT EXISTS monitor_url_domain AFTER UPDATE OF domain ON urls WHEN old.domain IS NOT new.domain BEGIN
        UPDATE monitor_domains SET refs=refs-1 WHERE domain=old.domain;
        INSERT INTO monitor_domains SELECT new.domain,1 WHERE new.domain IS NOT NULL AND new.domain!='' ON CONFLICT(domain) DO UPDATE SET refs=refs+1;
    END;
    CREATE TRIGGER IF NOT EXISTS monitor_page_insert AFTER INSERT ON pages BEGIN
        UPDATE monitor_counts SET value=value+1 WHERE key='pages';
    END;
    CREATE TRIGGER IF NOT EXISTS monitor_page_delete AFTER DELETE ON pages BEGIN
        UPDATE monitor_counts SET value=value-1 WHERE key='pages';
    END;")?;
    tx.commit()
}

pub fn count(conn: &Connection, key: &str) -> Result<i64> {
    conn.query_row(
        "SELECT coalesce((SELECT value FROM monitor_counts WHERE key=?1),0)",
        [key],
        |r| r.get(0),
    )
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Worker {
    pub id: usize,
    pub online: bool,
    pub phase: String,
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runtime {
    pub pid: u32,
    pub started_at: u64,
    pub configured: usize,
    pub mode: String,
    pub max_storage_bytes: u64,
    pub min_free_bytes: u64,
    pub session_ok: i64,
    pub session_failed: i64,
    pub workers: Vec<Worker>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counters_follow_updates_rollbacks_and_duplicate_urls() {
        let dir = std::env::temp_dir().join(format!("vertex-counts-{}", std::process::id()));
        let conn = crate::database::init_in(&dir).unwrap();
        conn.execute("DELETE FROM urls", []).unwrap();
        crate::database::enqueue(&conn, "https://example.com/a", 0, 0).unwrap();
        crate::database::enqueue(&conn, "https://example.com/a", 0, 0).unwrap();
        assert_eq!(count(&conn, "urls:pending").unwrap(), 1);
        conn.execute_batch("BEGIN; UPDATE urls SET status='done'; ROLLBACK;")
            .unwrap();
        assert_eq!(count(&conn, "urls:pending").unwrap(), 1);
        conn.execute("UPDATE urls SET status='done',domain='other.example'", [])
            .unwrap();
        assert_eq!(count(&conn, "urls:pending").unwrap(), 0);
        assert_eq!(count(&conn, "urls:done").unwrap(), 1);
        conn.execute(
            "INSERT INTO pages(url,content) VALUES('https://example.com/a','one')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO pages(url,content) VALUES('https://example.com/a','two') ON CONFLICT(url) DO UPDATE SET content=excluded.content",[]).unwrap();
        assert_eq!(count(&conn, "pages").unwrap(), 1);
        conn.execute("DELETE FROM pages", []).unwrap();
        assert_eq!(count(&conn, "pages").unwrap(), 0);
        assert_eq!(crate::database::count_domains(&conn).unwrap(), 1);
        install(&conn).unwrap();
        assert_eq!(count(&conn, "urls:done").unwrap(), 1);
        conn.execute("DELETE FROM urls", []).unwrap();
        assert_eq!(crate::database::count_domains(&conn).unwrap(), 0);
        drop(conn);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
