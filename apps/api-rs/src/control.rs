use super::{AppState, server_error};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::Ordering;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Start,
    Stop,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    action: Action,
    workers: Option<u32>,
}

pub async fn command(State(state): State<AppState>, Json(command): Json<Command>) -> Response {
    if state.access.is_public() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"Commandes reservees au panneau de ce PC"})),
        )
            .into_response();
    }
    if command.workers.is_some_and(|n| !(1..=64).contains(&n)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"Nombre de workers invalide (1 a 64)"})),
        )
            .into_response();
    }
    if !cfg!(windows) {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error":"Superviseur Windows indisponible sur cet hote"})),
        )
            .into_response();
    }
    if std::env::var_os("VERTEX_LOCAL_CONTROL").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"Pilotage local non active par le lanceur"})),
        )
            .into_response();
    }
    if state
        .control_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"Une commande est deja en cours"})),
        )
            .into_response();
    }
    let guard = RunningGuard(state.control_running.clone());
    let result = execute(command).await;
    drop(guard);
    match result {
        Ok(()) => (StatusCode::ACCEPTED, Json(json!({"accepted":true}))).into_response(),
        Err(error) => server_error(error),
    }
}

struct RunningGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

async fn execute(command: Command) -> Result<(), String> {
    let root = search_rs::workspace_root();
    let script = root.join("scripts/crawl-supervisor.ps1");
    if !script.is_file() {
        return Err("Lanceur de supervision absent".into());
    }
    let mut process = tokio::process::Command::new("pwsh.exe");
    process
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .arg("-Action")
        .arg(match command.action {
            Action::Start => "Start",
            Action::Stop => "Stop",
        });
    if matches!(command.action, Action::Start) {
        process
            .args(["-Profile", "release", "-Workers"])
            .arg(command.workers.unwrap_or(64).to_string());
    }
    process.current_dir(root).kill_on_drop(true);
    #[cfg(windows)]
    process.creation_flags(0x08000000);
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), process.output())
        .await
        .map_err(|_| "Delai de commande depasse".to_string())?
        .map_err(|e| format!("Lanceur indisponible : {e}"))?;
    if !output.status.success() {
        eprintln!(
            "Vertex control failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Err("Commande refusee ; consulter le journal du panneau".into());
    }
    Ok(())
}
