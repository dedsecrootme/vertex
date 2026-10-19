use super::{AppState, db_call, server_error};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct Verification {
    running: Arc<AtomicBool>,
    result: Arc<Mutex<Value>>,
}
impl Default for Verification {
    fn default() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            result: Arc::new(Mutex::new(json!({"status":"not_run"}))),
        }
    }
}

pub async fn snapshot(State(state): State<AppState>) -> Response {
    let started = std::time::Instant::now();
    let running = search_rs::runner::crawl_is_running(&search_rs::default_db_dir());
    let result=db_call(&state, move |conn| {
        let tx=conn.unchecked_transaction()?;
        let stats=search_rs::stats(&tx)?;
        let skipped=search_rs::monitor::count(&tx,"urls:skipped")?;
        let total:i64=tx.query_row("SELECT coalesce(sum(value),0) FROM monitor_counts WHERE key LIKE 'urls:%'",[],|r|r.get(0))?;
        let runtime:Option<(i64,String)>=tx.query_row("SELECT heartbeat,payload FROM crawler_runtime WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let now:i64=tx.query_row("SELECT unixepoch()",[],|r|r.get(0))?;
        let runtime=runtime.and_then(|(heartbeat,payload)| serde_json::from_str::<search_rs::monitor::Runtime>(&payload).ok().map(|runtime| {
            let age=(now-heartbeat).max(0);
            let fresh=running && age<=10;
            let online=if fresh {runtime.workers.iter().filter(|w|w.online).count()} else {0};
            json!({"heartbeat":heartbeat,"age_seconds":age,"fresh":fresh,"online":online,"data":runtime})
        }));
        let mut stmt=tx.prepare("SELECT id,started_at,finished_at,duration_secs,pages_ok,pages_failed,pages_total,status FROM crawl_runs ORDER BY id DESC LIMIT 20")?;
        let runs=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"started_at":r.get::<_,String>(1)?,"finished_at":r.get::<_,Option<String>>(2)?,"duration_secs":r.get::<_,Option<i64>>(3)?,"ok":r.get::<_,i64>(4)?,"failed":r.get::<_,i64>(5)?,"total":r.get::<_,i64>(6)?,"status":r.get::<_,String>(7)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt=tx.prepare("SELECT substr(error,1,600),created_at FROM errors ORDER BY id DESC LIMIT 10")?;
        let errors=stmt.query_map([],|r|Ok(json!({"message":r.get::<_,String>(0)?,"at":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let schema:String=tx.query_row("SELECT value FROM meta WHERE key='schema_version'",[],|r|r.get(0))?;
        let journal:String=tx.query_row("PRAGMA journal_mode",[],|r|r.get(0))?;
        let version:String=tx.query_row("SELECT sqlite_version()",[],|r|r.get(0))?;
        Ok(json!({"at":now,"running":running,"runtime":runtime,"stats":stats,"skipped":skipped,"total":total,"runs":runs,"errors":errors,"database":{"engine":"SQLite + FTS5","version":version,"schema":schema,"journal":journal}}))
    }).await;
    match result {
        Ok(mut value) => {
            let dir = search_rs::default_db_dir();
            value["control"] = json!({"available": cfg!(windows) && !state.access.is_public() && std::env::var("VERTEX_LOCAL_CONTROL").as_deref() == Ok("1"), "busy":state.control_running.load(Ordering::Acquire)});
            value["supervisor"] = std::fs::metadata(dir.join("supervisor-state.json"))
                .ok()
                .filter(|m| m.len() < 32768)
                .and_then(|_| std::fs::read(dir.join("supervisor-state.json")).ok())
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .unwrap_or(Value::Null);
            value["database"]["path"] = json!(dir.display().to_string());
            value["maintenance"] = std::fs::metadata(dir.join("night-cleanup.json"))
                .ok()
                .filter(|m| m.len() < 32768)
                .and_then(|_| std::fs::read(dir.join("night-cleanup.json")).ok())
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .unwrap_or(Value::Null);
            value["database"]["bytes"] = json!(
                std::fs::metadata(dir.join("vertex.db"))
                    .map(|m| m.len())
                    .ok()
            );
            value["database"]["wal_bytes"] = json!(
                std::fs::metadata(dir.join("vertex.db-wal"))
                    .map(|m| m.len())
                    .unwrap_or(0)
            );
            value["database"]["free_bytes"] = json!(search_rs::monitor::free_space(&dir).ok());
            value["verification"] = state.verification.result.lock().unwrap().clone();
            value["audit"] = std::fs::metadata(dir.join("queue-audit.json"))
                .ok()
                .filter(|m| m.len() < 131072)
                .and_then(|_| std::fs::read(dir.join("queue-audit.json")).ok())
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or(Value::Null);
            value["query_ms"] = json!(started.elapsed().as_millis());
            Json(value).into_response()
        }
        Err(error) => server_error(error),
    }
}

pub async fn verify(State(state): State<AppState>) -> Response {
    if state
        .verification
        .running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"Verification deja en cours"})),
        )
            .into_response();
    }
    let verification = state.verification.clone();
    *verification.result.lock().unwrap() = json!({"status":"running"});
    tokio::task::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let result = (|| -> rusqlite::Result<Value> {
            let conn = Connection::open_with_flags(
                search_rs::default_db_dir().join("vertex.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            conn.busy_timeout(std::time::Duration::from_secs(3))?;
            let tx = conn.unchecked_transaction()?;
            // Global quick_check asks FTS5 to write during its virtual-table check.
            // Check ordinary and shadow tables individually on this read-only connection.
            let tables=tx.prepare("SELECT name FROM pragma_table_list WHERE schema='main' AND type IN ('table','shadow') AND name!='sqlite_schema'")?
                .query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let mut integrity = String::from("ok");
            for table in &tables {
                let result: String = tx.query_row(
                    "SELECT quick_check FROM pragma_quick_check(?1)",
                    [table],
                    |r| r.get(0),
                )?;
                if result != "ok" {
                    integrity = format!("{table}: {result}");
                    break;
                }
            }
            let mismatches:i64=tx.query_row("WITH real AS (SELECT 'urls:'||status AS key,count(*) AS value FROM urls GROUP BY status UNION ALL SELECT 'pages',count(*) FROM pages), keys AS (SELECT key FROM real UNION SELECT key FROM monitor_counts WHERE key LIKE 'urls:%' OR key='pages') SELECT count(*) FROM keys LEFT JOIN real USING(key) LEFT JOIN monitor_counts AS cached USING(key) WHERE coalesce(real.value,0)!=coalesce(cached.value,0)",[],|r|r.get(0))?;
            let claim: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='idx_urls_claim')",
                [],
                |r| r.get(0),
            )?;
            let now: i64 = tx.query_row("SELECT unixepoch()", [], |r| r.get(0))?;
            Ok(
                json!({"status":if integrity=="ok" && mismatches==0 && claim {"ok"} else {"error"},"checked_at":now,"sqlite":integrity,"tables_checked":tables.len(),"counter_mismatches":mismatches,"claim_index":claim,"fts":"coherence semantique non verifiee"}),
            )
        })();
        let mut value=result.unwrap_or_else(|error| {eprintln!("Verification Vertex : {error}");json!({"status":"error","error":"Verification impossible ; consulter le journal serveur"})});
        value["duration_ms"] = json!(started.elapsed().as_millis());
        *verification.result.lock().unwrap() = value;
        verification.running.store(false, Ordering::Release);
    });
    (StatusCode::ACCEPTED, Json(json!({"started":true}))).into_response()
}
