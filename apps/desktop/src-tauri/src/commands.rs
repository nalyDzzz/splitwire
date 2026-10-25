//! Commands invoked by the UI (see `Backend` in src/lib/api.ts).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;
use splitwire_core::ipc::{Connection, LogEntry, Request, Response};
use splitwire_core::model::{Tunnel, ValidationError};
use splitwire_core::wgconf::{self, ImportedTunnel};
use splitwire_core::keys;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt;

use crate::processes::{self, PickedApp, ProcessInfo};
use crate::service_client::{self, ClientError, ServiceStatus};
use crate::store::Settings;
use crate::{elevate, AppState};

/// Shape of a rejected command promise. Mirrors `CommandError` in types.ts.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CommandError {
    Validation { errors: Vec<ValidationError> },
    Service { message: String },
    Other { message: String },
}

impl CommandError {
    fn other(message: impl Into<String>) -> Self {
        Self::Other { message: message.into() }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(error: anyhow::Error) -> Self {
        Self::other(format!("{error:#}"))
    }
}

impl From<ClientError> for CommandError {
    fn from(error: ClientError) -> Self {
        Self::Service { message: error.to_string() }
    }
}

impl From<tauri::Error> for CommandError {
    fn from(error: tauri::Error) -> Self {
        Self::other(error.to_string())
    }
}

type CmdResult<T> = Result<T, CommandError>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyPair {
    private_key: String,
    public_key: String,
}

// ---- tunnels ----

#[tauri::command]
pub fn list_tunnels(state: State<'_, AppState>) -> Vec<Tunnel> {
    state.store.lock().unwrap().tunnels.clone()
}

#[tauri::command]
pub fn save_tunnel(state: State<'_, AppState>, mut tunnel: Tunnel) -> CmdResult<Tunnel> {
    if tunnel.id.trim().is_empty() {
        tunnel.id = Tunnel::new_id();
    }
    tunnel.validate().map_err(|errors| CommandError::Validation { errors })?;
    state.store.lock().unwrap().upsert(tunnel.clone())?;
    Ok(tunnel)
}

#[tauri::command]
pub async fn delete_tunnel(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let active = state.status.lock().unwrap().engine().and_then(|s| s.tunnel_id.clone());
    if active.as_deref() == Some(id.as_str()) {
        service_client::request(Request::Disconnect).await?;
    }
    state.store.lock().unwrap().remove(&id)?;
    crate::refresh(&app);
    Ok(())
}

#[tauri::command]
pub async fn import_tunnel_file(app: AppHandle) -> CmdResult<Option<ImportedTunnel>> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Import WireGuard config").add_filter("WireGuard config", &["conf"]).blocking_pick_file()
    })
    .await
    .map_err(|e| CommandError::other(e.to_string()))?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    import_path(&path).map(Some)
}

pub fn import_path(path: &std::path::Path) -> CmdResult<ImportedTunnel> {
    let meta = std::fs::metadata(path).map_err(|e| CommandError::other(format!("Can't read {}: {e}", path.display())))?;
    if meta.len() > 1024 * 1024 {
        return Err(CommandError::other("That file is too large to be a WireGuard config"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| CommandError::other(format!("Can't read {}: {e}", path.display())))?;
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
    import_tunnel_text(text, name)
}

#[tauri::command]
pub fn import_tunnel_text(text: String, name: String) -> CmdResult<ImportedTunnel> {
    let name = if name.trim().is_empty() { "Imported".to_string() } else { name };
    wgconf::parse(&text, &name).map_err(|e| {
        if e.line == 0 {
            CommandError::other(e.message)
        } else {
            CommandError::other(format!("Line {}: {}", e.line, e.message))
        }
    })
}

#[tauri::command]
pub async fn export_tunnel_file(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<bool> {
    let tunnel = state.store.lock().unwrap().tunnel(&id).cloned().ok_or_else(|| CommandError::other("Tunnel not found"))?;
    let file_name = format!("{}.conf", sanitize_file_name(&tunnel.name));
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Export WireGuard config").set_file_name(file_name).add_filter("WireGuard config", &["conf"]).blocking_save_file()
    })
    .await
    .map_err(|e| CommandError::other(e.to_string()))?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(false);
    };
    std::fs::write(&path, wgconf::to_conf(&tunnel, true))
        .map_err(|e| CommandError::other(format!("Couldn't write {}: {e}", path.display())))?;
    Ok(true)
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_end_matches('.');
    if cleaned.is_empty() { "tunnel".into() } else { cleaned.into() }
}

#[tauri::command]
pub fn generate_keypair() -> KeyPair {
    let private_key = keys::generate_private_key();
    let public_key = keys::public_key(&private_key).expect("generated keys are valid");
    KeyPair { private_key, public_key }
}

#[tauri::command]
pub fn public_key(private_key: String) -> CmdResult<String> {
    keys::public_key(&private_key).map_err(|_| CommandError::other("Private key must be a base64 WireGuard key"))
}

// ---- service ----

#[tauri::command]
pub fn get_service_status(state: State<'_, AppState>) -> ServiceStatus {
    state.status.lock().unwrap().clone()
}

#[tauri::command]
pub async fn install_service(app: AppHandle) -> CmdResult<()> {
    let exe = service_exe(&app).ok_or_else(|| CommandError::other("splitwire-service.exe wasn't found next to the app"))?;
    tauri::async_runtime::spawn_blocking(move || elevate::run_elevated(&exe, "install"))
        .await
        .map_err(|e| CommandError::other(e.to_string()))??;
    crate::refresh(&app);
    Ok(())
}

/// Installed builds keep the service in `engine\`; dev builds have it next to the app.
fn service_exe(app: &AppHandle) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join("engine").join("splitwire-service.exe"));
    }
    if let Ok(exe) = std::env::current_exe() {
        candidates.push(exe.with_file_name("splitwire-service.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[tauri::command]
pub async fn connect(app: AppHandle, state: State<'_, AppState>, tunnel_id: String) -> CmdResult<()> {
    crate::connect_tunnel(&app, &state, &tunnel_id).await
}

#[tauri::command]
pub async fn disconnect(app: AppHandle) -> CmdResult<()> {
    let result = service_client::request(Request::Disconnect).await;
    crate::refresh(&app);
    result?;
    Ok(())
}

#[tauri::command]
pub async fn get_connections() -> CmdResult<Vec<Connection>> {
    match service_client::request(Request::GetConnections).await {
        Ok(Response::Connections { connections }) => Ok(connections),
        Ok(_) => Err(ClientError::Protocol.into()),
        Err(ClientError::NotRunning) => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

#[tauri::command]
pub async fn get_logs(after: u64) -> CmdResult<Vec<LogEntry>> {
    match service_client::request(Request::GetLogs { after }).await {
        Ok(Response::Logs { entries }) => Ok(entries),
        Ok(_) => Err(ClientError::Protocol.into()),
        Err(ClientError::NotRunning) => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

// ---- apps ----

#[tauri::command]
pub async fn list_processes() -> CmdResult<Vec<ProcessInfo>> {
    tauri::async_runtime::spawn_blocking(processes::list).await.map_err(|e| CommandError::other(e.to_string()))
}

#[tauri::command]
pub async fn get_icons(app: AppHandle, paths: Vec<String>) -> CmdResult<HashMap<String, Option<String>>> {
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().icons.get_many(&paths))
        .await
        .map_err(|e| CommandError::other(e.to_string()))
}

#[tauri::command]
pub async fn pick_executable(app: AppHandle) -> CmdResult<Option<PickedApp>> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Choose an app").add_filter("Programs", &["exe"]).blocking_pick_file()
    })
    .await
    .map_err(|e| CommandError::other(e.to_string()))?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| PickedApp::from_path(p.to_string_lossy().into_owned())))
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> CmdResult<Option<String>> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title("Choose a folder").blocking_pick_folder()
    })
    .await
    .map_err(|e| CommandError::other(e.to_string()))?;
    Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

// ---- settings & window ----

#[tauri::command]
pub fn get_settings(app: AppHandle, state: State<'_, AppState>) -> Settings {
    let mut store = state.store.lock().unwrap();
    if let Ok(enabled) = app.autolaunch().is_enabled() {
        store.settings.launch_at_login = enabled;
    }
    store.settings.clone()
}

#[tauri::command]
pub fn update_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> CmdResult<Settings> {
    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().ok() != Some(settings.launch_at_login) {
        let result = if settings.launch_at_login { autolaunch.enable() } else { autolaunch.disable() };
        result.map_err(|e| CommandError::other(format!("Couldn't change launch at login: {e}")))?;
    }
    let mut store = state.store.lock().unwrap();
    store.settings = settings;
    store.save_settings()?;
    Ok(store.settings.clone())
}

/// Called by the UI once it has rendered, so the window never flashes blank.
#[tauri::command]
pub fn frontend_ready(window: WebviewWindow, state: State<'_, AppState>) {
    let first = !state.frontend_ready.swap(true, std::sync::atomic::Ordering::SeqCst);
    if first && state.started_hidden {
        return;
    }
    let _ = window.show();
    let _ = window.set_focus();
}
