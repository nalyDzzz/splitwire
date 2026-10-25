//! In-app updates from GitHub Releases.
//!
//! The release workflow publishes `latest.json` next to each installer; it
//! carries a minisign signature made with the project's updater key. The
//! public half is in tauri.conf.json, so a download that doesn't match is
//! never run. Installing runs the new NSIS installer in passive mode (it
//! updates the service too), which exits this app and relaunches it after.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use splitwire_core::ipc::ConnectionState;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};
use tokio::sync::Mutex;

use crate::commands::CommandError;
use crate::AppState;

/// First background check, a little after startup.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<Update>>,
    installing: AtomicBool,
}

/// Mirrors `UpdateInfo` in types.ts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    /// Release notes (Markdown).
    pub notes: Option<String>,
    /// RFC 3339 publish date.
    pub date: Option<String>,
}

impl From<&Update> for UpdateInfo {
    fn from(update: &Update) -> Self {
        Self {
            version: update.version.clone(),
            current_version: update.current_version.clone(),
            notes: update.body.clone().filter(|b| !b.trim().is_empty()),
            date: update
                .date
                .and_then(|d| d.format(&time::format_description::well_known::Rfc3339).ok()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

fn updater(app: &AppHandle) -> anyhow::Result<Updater> {
    let mut builder = app.updater_builder();
    // For testing against a local or staged manifest. Signatures are still
    // checked against the built-in public key.
    if let Ok(endpoint) = std::env::var("SPLITWIRE_UPDATE_ENDPOINT") {
        builder = builder.endpoints(vec![endpoint.parse()?])?;
    }
    Ok(builder.build()?)
}

/// Asks GitHub for a newer release and remembers it for `install_update`.
pub async fn check(app: &AppHandle) -> anyhow::Result<Option<UpdateInfo>> {
    let update = updater(app)?.check().await?;
    let info = update.as_ref().map(UpdateInfo::from);
    *app.state::<UpdateState>().pending.lock().await = update;
    Ok(info)
}

/// Background checks while the app runs, when enabled in settings.
pub async fn check_periodically(app: AppHandle) {
    tokio::time::sleep(FIRST_CHECK_DELAY).await;
    loop {
        let enabled = app.state::<AppState>().store.lock().unwrap().settings.auto_check_updates;
        if enabled {
            match check(&app).await {
                Ok(Some(info)) => {
                    tracing::info!("update {} is available", info.version);
                    let _ = app.emit("update-available", &info);
                }
                Ok(None) => tracing::debug!("no update available"),
                Err(error) => tracing::warn!("update check failed: {error:#}"),
            }
        }
        tokio::time::sleep(CHECK_INTERVAL).await;
    }
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<UpdateInfo>, CommandError> {
    check(&app).await.map_err(|e| CommandError::Other { message: format!("Couldn't check for updates: {e:#}") })
}

/// The update found by the last check, without hitting the network.
#[tauri::command]
pub async fn get_available_update(app: AppHandle) -> Option<UpdateInfo> {
    app.state::<UpdateState>().pending.lock().await.as_ref().map(UpdateInfo::from)
}

/// Downloads, verifies and installs the pending update. On Windows the app
/// exits as the installer starts, so a successful call never returns.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), CommandError> {
    let state = app.state::<UpdateState>();
    if state.installing.swap(true, Ordering::SeqCst) {
        return Err(CommandError::Other { message: "An update is already installing".into() });
    }
    let result = download_and_install(&app).await;
    state.installing.store(false, Ordering::SeqCst);
    result.map_err(|e| CommandError::Other { message: format!("Couldn't install the update: {e:#}") })
}

async fn download_and_install(app: &AppHandle) -> anyhow::Result<()> {
    let update_state = app.state::<UpdateState>();
    let mut pending = update_state.pending.lock().await;
    if pending.is_none() {
        *pending = updater(app)?.check().await?;
    }
    let Some(update) = pending.as_ref() else {
        anyhow::bail!("Splitwire is already up to date");
    };

    let mut downloaded = 0u64;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    // `download` verifies the signature before returning the bytes.
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                if last_emit.elapsed() >= Duration::from_millis(100) || Some(downloaded) == total {
                    last_emit = Instant::now();
                    let _ = app.emit("update-progress", UpdateProgress { downloaded, total });
                }
            },
            || {},
        )
        .await?;

    if std::env::var_os("SPLITWIRE_UPDATE_DRY_RUN").is_some() {
        tracing::info!("update {} downloaded and verified; dry run, not installing", update.version);
        return Ok(());
    }

    // The installer stops the service, which drops the tunnel. Reconnect it
    // when the new version starts.
    let app_state = app.state::<AppState>();
    let active = app_state
        .status
        .lock()
        .unwrap()
        .engine()
        .filter(|e| matches!(e.state, ConnectionState::Connected | ConnectionState::Connecting))
        .and_then(|e| e.tunnel_id.clone());
    if let Some(id) = active {
        if let Err(error) = app_state.store.lock().unwrap().set_resume(&id) {
            tracing::warn!("couldn't remember the tunnel to reconnect: {error:#}");
        }
    }

    tracing::info!("installing update {}", update.version);
    update.install(bytes)?;
    Ok(())
}
