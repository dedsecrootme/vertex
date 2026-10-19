use clap::Parser;
use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use std::{collections::HashMap, path::PathBuf, time::Instant};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    dir: PathBuf,
    #[arg(long)]
    fts_integrity: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let path = args.dir.join("vertex.db");
    if !path.is_file() {
        return Err("Existing index required; no database will be created".into());
    }
    let flags = if args.fts_integrity {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let conn = Connection::open_with_flags(path, flags)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch("PRAGMA cache_size=-8192;")?;
    let started = Instant::now();
    let engine: String = conn.query_row("SELECT sqlite_version()", [], |r| r.get(0))?;
    let structure = conn
        .prepare("PRAGMA quick_check")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if structure != ["ok"] {
        return Err(format!("SQLite check failed: {structure:?}").into());
    }
    let tx = conn.unchecked_transaction()?;
    let pages: i64 = tx.query_row("SELECT count(*) FROM pages", [], |r| r.get(0))?;
    let mut actual: HashMap<String, i64> = tx
        .prepare("SELECT 'urls:'||status,count(*) FROM urls GROUP BY status")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    actual.insert("pages".into(), pages);
    let counters: HashMap<String, i64> = tx
        .prepare("SELECT key,value FROM monitor_counts WHERE key!='ready'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let matches = actual
        .iter()
        .all(|(key, value)| counters.get(key).unwrap_or(&0) == value)
        && counters
            .iter()
            .all(|(key, value)| actual.get(key).unwrap_or(&0) == value);
    tx.commit()?;
    if !matches {
        return Err("Stored counters differ from real rows".into());
    }
    if args.fts_integrity {
        conn.execute(
            "INSERT INTO pages_fts(pages_fts,rank) VALUES('integrity-check',1)",
            [],
        )?;
    }
    let report = json!({"at":chrono_time(&conn)?,"engine":engine,"structure":"ok","pages":pages,
        "counter_mismatches":0,"fts":if args.fts_integrity {"external-content integrity-check rank=1 passed"}else{"not requested"},
        "seconds":started.elapsed().as_secs_f64(),"repair_performed":false});
    let output = serde_json::to_string_pretty(&report)?;
    std::fs::write(args.dir.join("verification-current.json"), &output)?;
    println!("{output}");
    Ok(())
}

fn chrono_time(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
        r.get(0)
    })
}
