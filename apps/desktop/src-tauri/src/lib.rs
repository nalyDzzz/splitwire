//! The Splitwire desktop app: UI host, tunnel storage, tray, and the client
//! side of the service pipe. Runs without admin rights; the privileged work
//! happens in `splitwire-service`.

mod commands;
mod elevate;
mod icons;
mod processes;
mod service_client;
mod store;
mod tray;
mod updater;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Duration;

use splitwire_core::ipc::{ConnectOptions, ConnectionState, Request};
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, WindowEvent};
use tokio::sync::Notify;

use crate::commands::CommandError;
use crate::icons::IconCache;
use crate::service_client::ServiceStatus;
use crate::store::Store;

pub struct AppState {
    store: Mutex<Store>,
    icons: IconCache,
    /// Last status seen by the poller.
    status: Mutex<ServiceStatus>,
    /// Wakes the poller for an immediate refresh.
    poke: Notify,
    tray: Mutex<Option<tray::Tray>>,
    /// Launched by autostart: stay in the tray until opened.
    started_hidden: bool,
    frontend_ready: AtomicBool,
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let started_hidden = std::env::args().any(|arg| arg == "--hidden");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(updater::UpdateState::default())
        .setup(move |app| {
            let dir = app.path().app_local_data_dir()?;
            let store = Store::load(dir).map_err(|e| format!("{e:#}"))?;
            // Reconnect what an update interrupted, else honour connect-on-launch.
            let connect_on_launch = store
                .take_resume()
                .or_else(|| store.settings.connect_on_launch.then(|| store.settings.last_tunnel_id.clone()).flatten());
            app.manage(AppState {
                store: Mutex::new(store),
                icons: IconCache::default(),
                status: Mutex::new(ServiceStatus::Unreachable { message: "Checking the service…".into() }),
                poke: Notify::new(),
                tray: Mutex::new(None),
                started_hidden,
                frontend_ready: AtomicBool::new(false),
            });

            let tray = tray::create(app.handle())?;
            *app.state::<AppState>().tray.lock().unwrap() = Some(tray);

            tauri::async_runtime::spawn(poll_status(app.handle().clone()));
            tauri::async_runtime::spawn(updater::check_periodically(app.handle().clone()));
            if !started_hidden {
                // The window starts hidden and the UI shows it once rendered
                // (`frontend_ready`). Don't stay invisible if that never comes.
                let app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    if !app.state::<AppState>().frontend_ready.load(std::sync::atomic::Ordering::SeqCst) {
                        tracing::warn!("UI didn't report ready; showing the window anyway");
                        show_main_window(&app);
                    }
                });
            }
            if let Some(id) = connect_on_launch {
                tauri::async_runtime::spawn(connect_when_ready(app.handle().clone(), id));
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                api.prevent_close();
                let close_to_tray = window.state::<AppState>().store.lock().unwrap().settings.close_to_tray;
                if close_to_tray {
                    let _ = window.hide();
                } else {
                    quit(window.app_handle());
                }
            }
            WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) => {
                for path in paths.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("conf"))) {
                    match commands::import_path(path) {
                        Ok(imported) => {
                            let _ = window.emit("import-tunnel", imported);
                        }
                        Err(error) => tracing::warn!("couldn't import {}: {error:?}", path.display()),
                    }
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_tunnels,
            commands::save_tunnel,
            commands::delete_tunnel,
            commands::import_tunnel_file,
            commands::import_tunnel_text,
            commands::export_tunnel_file,
            commands::generate_keypair,
            commands::public_key,
            commands::get_service_status,
            commands::install_service,
            commands::connect,
            commands::disconnect,
            commands::get_connections,
            commands::get_logs,
            commands::list_processes,
            commands::get_icons,
            commands::pick_executable,
            commands::pick_folder,
            commands::get_settings,
            commands::update_settings,
            commands::frontend_ready,
            updater::check_for_update,
            updater::get_available_update,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Splitwire");
}

/// Polls the service about once a second (or right away when poked) and
/// pushes changes to the UI and tray.
async fn poll_status(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut versions = None;
    let mut last: Option<ServiceStatus> = None;
    loop {
        let status = service_client::probe(&mut versions).await;
        if last.as_ref() != Some(&status) {
            *state.status.lock().unwrap() = status.clone();
            let _ = app.emit("service-status", &status);
            if let Some(tray) = state.tray.lock().unwrap().as_mut() {
                tray.update(&app, &status);
            }
            last = Some(status);
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = state.poke.notified() => {}
        }
    }
}

/// Ask the poller to refresh now (after connect, disconnect, install...).
pub(crate) fn refresh(app: &AppHandle) {
    app.state::<AppState>().poke.notify_one();
}

pub(crate) async fn connect_tunnel(app: &AppHandle, state: &AppState, tunnel_id: &str) -> Result<(), CommandError> {
    let (tunnel, options) = {
        let mut store = state.store.lock().unwrap();
        let tunnel = store.tunnel(tunnel_id).cloned().ok_or_else(|| CommandError::Other { message: "Tunnel not found".into() })?;
        if store.settings.last_tunnel_id.as_deref() != Some(tunnel_id) {
            store.settings.last_tunnel_id = Some(tunnel_id.to_string());
            store.save_settings()?;
        }
        (tunnel, ConnectOptions { health_check: store.settings.health_check })
    };
    tunnel.validate().map_err(|errors| CommandError::Validation { errors })?;
    refresh(app);
    let result = service_client::request(Request::Connect { tunnel: Box::new(tunnel), options }).await;
    refresh(app);
    result?;
    Ok(())
}

/// Connect-on-launch: wait for the service to come up, then connect once.
async fn connect_when_ready(app: AppHandle, tunnel_id: String) {
    for _ in 0..30 {
        let status = app.state::<AppState>().status.lock().unwrap().clone();
        match status.engine().map(|e| e.state) {
            Some(ConnectionState::Disconnected | ConnectionState::Failed) => {
                if let Err(error) = connect_tunnel(&app, &app.state::<AppState>(), &tunnel_id).await {
                    tracing::warn!("connect on launch failed: {error:?}");
                }
                return;
            }
            Some(_) => return, // already connected or busy
            None => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub(crate) fn toggle_from_tray(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let engine_state = state.status.lock().unwrap().engine().map(|e| e.state);
        let result = match engine_state {
            Some(ConnectionState::Connected | ConnectionState::Connecting) => {
                service_client::request(Request::Disconnect).await.map(|_| ()).map_err(CommandError::from)
            }
            Some(_) => {
                let target = {
                    let store = state.store.lock().unwrap();
                    store
                        .settings
                        .last_tunnel_id
                        .clone()
                        .filter(|id| store.tunnel(id).is_some())
                        .or_else(|| store.tunnels.first().map(|t| t.id.clone()))
                };
                match target {
                    Some(id) => connect_tunnel(&app, &state, &id).await,
                    None => Ok(()),
                }
            }
            None => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!("tray toggle failed: {error:?}");
            show_main_window(&app);
        }
        refresh(&app);
    });
}

/// Quits the app. A running tunnel is disconnected first, so nothing keeps
/// routing traffic without a visible tray icon.
pub(crate) fn quit(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let running = app
            .state::<AppState>()
            .status
            .lock()
            .unwrap()
            .engine()
            .is_some_and(|e| matches!(e.state, ConnectionState::Connected | ConnectionState::Connecting));
        if running {
            let _ = service_client::request(Request::Disconnect).await;
        }
        app.exit(0);
    });
}
