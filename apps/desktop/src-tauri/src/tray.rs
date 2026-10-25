//! System tray icon and menu.

use splitwire_core::ipc::ConnectionState;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::service_client::ServiceStatus;
use crate::AppState;

const ICON_ON: &[u8] = include_bytes!("../icons/tray-on.png");
const ICON_OFF: &[u8] = include_bytes!("../icons/tray-off.png");

pub struct Tray {
    icon: TrayIcon<Wry>,
    status: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    quit: MenuItem<Wry>,
    /// What the tray currently shows, to skip redundant updates.
    shown: Option<(Option<ConnectionState>, Option<String>, Option<String>)>,
}

pub fn create(app: &AppHandle) -> tauri::Result<Tray> {
    let open = MenuItem::with_id(app, "open", "Open Splitwire", true, None::<&str>)?;
    let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Connect", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &PredefinedMenuItem::separator(app)?, &status, &toggle, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let icon = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON_OFF)?)
        .tooltip("Splitwire")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => crate::show_main_window(app),
            "toggle" => crate::toggle_from_tray(app),
            "quit" => crate::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(Tray { icon, status, toggle, quit, shown: None })
}

impl Tray {
    pub fn update(&mut self, app: &AppHandle, status: &ServiceStatus) {
        let engine = status.engine();
        let state = engine.map(|e| e.state);
        let active_name = engine.and_then(|e| e.tunnel_name.clone());
        let app_state = app.state::<AppState>();
        let last_name = {
            let store = app_state.store.lock().unwrap();
            store
                .settings
                .last_tunnel_id
                .as_deref()
                .and_then(|id| store.tunnel(id))
                .or(store.tunnels.first())
                .map(|t| t.name.clone())
        };
        let key = (state, active_name.clone(), last_name.clone());
        if self.shown.as_ref() == Some(&key) {
            return;
        }
        self.shown = Some(key);

        let named = |prefix: &str| match &active_name {
            Some(name) => format!("{prefix} {name}"),
            None => prefix.to_string(),
        };
        let (label, toggle, toggle_enabled, on) = match (status, state) {
            (ServiceStatus::NotInstalled, _) => ("Service not installed".to_string(), "Connect".to_string(), false, false),
            (ServiceStatus::Stopped, _) => ("Service stopped".to_string(), "Connect".to_string(), false, false),
            (ServiceStatus::Unreachable { .. }, _) => ("Service not responding".to_string(), "Connect".to_string(), false, false),
            (_, Some(ConnectionState::Connected)) => (named("Connected to"), "Disconnect".to_string(), true, true),
            (_, Some(ConnectionState::Connecting)) => (named("Connecting to"), "Disconnect".to_string(), true, true),
            (_, Some(ConnectionState::Disconnecting)) => ("Disconnecting…".to_string(), "Disconnect".to_string(), false, false),
            (_, Some(ConnectionState::Failed)) => (named("Couldn't connect to"), connect_label(&last_name), last_name.is_some(), false),
            _ => ("Disconnected".to_string(), connect_label(&last_name), last_name.is_some(), false),
        };

        let _ = self.status.set_text(&label);
        let _ = self.toggle.set_text(&toggle);
        let _ = self.toggle.set_enabled(toggle_enabled);
        let _ = self.quit.set_text(if on { "Disconnect and quit" } else { "Quit" });
        let _ = self.icon.set_tooltip(Some(format!("Splitwire · {label}")));
        if let Ok(image) = Image::from_bytes(if on { ICON_ON } else { ICON_OFF }) {
            let _ = self.icon.set_icon(Some(image));
        }
    }
}

fn connect_label(last: &Option<String>) -> String {
    match last {
        Some(name) => format!("Connect to {name}"),
        None => "Connect".to_string(),
    }
}
