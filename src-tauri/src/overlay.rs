//! In-memory resident entry lifecycle. Persisted preferences belong to step 1.4.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

pub const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+Space";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayView {
    pub enabled: bool,
    pub expanded: bool,
    pub shortcut: String,
    pub shortcut_error: Option<&'static str>,
    pub window_error: Option<&'static str>,
    pub draft: String,
    pub tray_ready: bool,
}

#[derive(Clone, Deserialize, Serialize)]
struct StoredPosition {
    x: i32,
    y: i32,
    monitor: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct OverlayPreferences {
    enabled: bool,
    shortcut: String,
    position: Option<StoredPosition>,
}

impl Default for OverlayPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            shortcut: DEFAULT_SHORTCUT.into(),
            position: None,
        }
    }
}

pub struct OverlayState(pub Mutex<OverlayView>, Mutex<()>, Mutex<OverlayPreferences>);
impl Default for OverlayState {
    fn default() -> Self {
        Self(
            Mutex::new(OverlayView {
                enabled: true,
                expanded: false,
                shortcut: DEFAULT_SHORTCUT.into(),
                shortcut_error: None,
                window_error: None,
                draft: String::new(),
                tray_ready: false,
            }),
            Mutex::new(()),
            Mutex::new(OverlayPreferences::default()),
        )
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum OverlayAction {
    Expand,
    Collapse,
    Enable,
    Disable,
    OpenMain,
    Exit,
    RepairPosition,
    SetShortcut { shortcut: String },
    Draft { draft: String },
}

pub fn view(app: &tauri::AppHandle) -> OverlayView {
    app.state::<OverlayState>().0.lock().unwrap().clone()
}

fn preferences_path(app: &tauri::AppHandle) -> Result<PathBuf, &'static str> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| "PA_OVERLAY_SETTINGS_PATH_FAILED")?
        .join("overlay-settings.json"))
}

fn load_preferences(app: &tauri::AppHandle) {
    let Ok(path) = preferences_path(app) else { return };
    let Ok(contents) = fs::read_to_string(path) else { return };
    let Ok(preferences) = serde_json::from_str::<OverlayPreferences>(&contents) else { return };
    let state = app.state::<OverlayState>();
    *state.2.lock().unwrap() = preferences.clone();
    let mut view = state.0.lock().unwrap();
    view.enabled = preferences.enabled;
    view.shortcut = preferences.shortcut;
}

pub fn persist_preferences(app: &tauri::AppHandle) -> Result<(), &'static str> {
    let path = preferences_path(app)?;
    let state = app.state::<OverlayState>();
    let mut preferences = state.2.lock().unwrap().clone();
    let view = state.0.lock().unwrap().clone();
    preferences.enabled = view.enabled;
    preferences.shortcut = view.shortcut;
    if let Some(window) = app.get_webview_window("overlay") {
        if let Ok(position) = window.outer_position() {
            let monitor = window
                .current_monitor()
                .ok()
                .flatten()
                .and_then(|value| value.name().map(|name| name.to_owned()));
            preferences.position = Some(StoredPosition {
                x: position.x,
                y: position.y,
                monitor,
            });
        }
    }
    let contents = serde_json::to_vec_pretty(&preferences).map_err(|_| "PA_OVERLAY_SETTINGS_WRITE_FAILED")?;
    let directory = path.parent().ok_or("PA_OVERLAY_SETTINGS_PATH_FAILED")?;
    fs::create_dir_all(directory).map_err(|_| "PA_OVERLAY_SETTINGS_WRITE_FAILED")?;
    fs::write(path, contents).map_err(|_| "PA_OVERLAY_SETTINGS_WRITE_FAILED")?;
    *state.2.lock().unwrap() = preferences;
    Ok(())
}

fn restore_position(app: &tauri::AppHandle, window: &WebviewWindow) -> tauri::Result<()> {
    let position = app.state::<OverlayState>().2.lock().unwrap().position.clone();
    if let Some(position) = position {
        window.set_position(tauri::PhysicalPosition::new(position.x, position.y))?;
    } else if let Some(monitor) = window.primary_monitor()? {
        let area = monitor.work_area();
        let size = window.outer_size()?;
        window.set_position(tauri::PhysicalPosition::new(
            area.position.x + area.size.width as i32 - size.width as i32 - 24,
            area.position.y + area.size.height as i32 - size.height as i32 - 100,
        ))?;
    }
    correct_position(window)
}
fn publish(app: &tauri::AppHandle) {
    let current = view(app);
    // Draft goes only to the owned overlay, never to logs or main diagnostics.
    let _ = app.emit_to(
        tauri::EventTarget::WebviewWindow {
            label: "overlay".into(),
        },
        "overlay-state",
        current.clone(),
    );
    let mut public = current;
    public.draft.clear();
    let _ = app.emit_to(
        tauri::EventTarget::WebviewWindow {
            label: "main".into(),
        },
        "overlay-state",
        public,
    );
}

pub fn clamp_axis(position: i32, start: i32, extent: u32, size: u32) -> i32 {
    position.clamp(
        start,
        start.saturating_add(extent.saturating_sub(size) as i32),
    )
}

fn correct_position(window: &WebviewWindow) -> tauri::Result<()> {
    let position = window.outer_position()?;
    let size = window.outer_size()?;
    let monitors = window.available_monitors()?;
    let current = monitors
        .iter()
        .find(|monitor| {
            let area = monitor.work_area();
            position.x >= area.position.x
                && position.y >= area.position.y
                && position.x < area.position.x + area.size.width as i32
                && position.y < area.position.y + area.size.height as i32
        })
        .cloned()
        .or(window.primary_monitor()?);
    if let Some(monitor) = current {
        let area = monitor.work_area();
        let next = tauri::PhysicalPosition::new(
            clamp_axis(position.x, area.position.x, area.size.width, size.width),
            clamp_axis(position.y, area.position.y, area.size.height, size.height),
        );
        if next != position {
            window.set_position(next)?;
        }
    }
    Ok(())
}

fn ensure_window(app: &tauri::AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(window) = app.get_webview_window("overlay") {
        return Ok(window);
    }
    WebviewWindowBuilder::new(
        app,
        "overlay",
        WebviewUrl::App("index.html?window=overlay".into()),
    )
    .title("Project Assistant · 常驻入口")
    .inner_size(72.0, 72.0)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .transparent(true)
    .shadow(false)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .build()
}

fn show_entry(app: &tauri::AppHandle, expanded: bool, focus: bool) -> tauri::Result<()> {
    let window = ensure_window(app)?;
    window.set_size(if expanded {
        tauri::LogicalSize::new(390.0, 390.0)
    } else {
        tauri::LogicalSize::new(72.0, 72.0)
    })?;
    let top = window.set_always_on_top(true);
    correct_position(&window)?;
    window.show()?;
    if focus {
        window.set_focus()?;
    }
    app.state::<OverlayState>().0.lock().unwrap().window_error =
        top.err().map(|_| "PA_OVERLAY_TOPMOST_FAILED");
    let _ = persist_preferences(app);
    Ok(())
}

pub fn register_shortcut(app: &tauri::AppHandle, candidate: &str) {
    let old = view(app).shortcut;
    if candidate.len() > 80
        || candidate
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .is_err()
    {
        app.state::<OverlayState>().0.lock().unwrap().shortcut_error = Some("PA_SHORTCUT_INVALID");
    } else if candidate != old || !app.global_shortcut().is_registered(candidate) {
        if app.global_shortcut().register(candidate).is_err() {
            app.state::<OverlayState>().0.lock().unwrap().shortcut_error =
                Some("PA_SHORTCUT_CONFLICT");
        } else {
            if candidate != old {
                let _ = app.global_shortcut().unregister(old.as_str());
            }
            let state = app.state::<OverlayState>();
            let mut state = state.0.lock().unwrap();
            state.shortcut = candidate.into();
            state.shortcut_error = None;
        }
    } else {
        app.state::<OverlayState>().0.lock().unwrap().shortcut_error = None;
    }
    let _ = persist_preferences(app);
    publish(app);
}

pub fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn apply(app: &tauri::AppHandle, action: OverlayAction) -> Result<(), &'static str> {
    let state = app.state::<OverlayState>();
    let _operation = state.1.lock().unwrap();
    match action {
        OverlayAction::Exit => {
            let _ = persist_preferences(app);
            let _ = app.global_shortcut().unregister_all();
            app.exit(0);
            return Ok(());
        }
        OverlayAction::OpenMain => show_main(app),
        OverlayAction::SetShortcut { shortcut } => register_shortcut(app, &shortcut),
        OverlayAction::Draft { draft } => {
            if draft.len() > 32768 {
                return Err("PA_OVERLAY_DRAFT_TOO_LARGE");
            }
            app.state::<OverlayState>().0.lock().unwrap().draft = draft;
            return Ok(());
        }
        OverlayAction::Disable => {
            app.state::<OverlayState>().0.lock().unwrap().enabled = false;
            app.state::<OverlayState>().0.lock().unwrap().expanded = false;
            let _ = app.global_shortcut().unregister_all();
            if let Some(window) = app.get_webview_window("overlay") {
                let _ = window.hide();
            }
            // Keep a reachable main window after disabling the last visible entry.
            show_main(app);
            let _ = persist_preferences(app);
        }
        OverlayAction::Expand
        | OverlayAction::Collapse
        | OverlayAction::Enable
        | OverlayAction::RepairPosition => {
            if matches!(action, OverlayAction::RepairPosition) {
                if let Some(window) = app.get_webview_window("overlay") {
                    correct_position(&window).map_err(|_| "PA_OVERLAY_WINDOW_FAILED")?;
                }
                persist_preferences(app)?;
                return Ok(());
            }
            let expanded = matches!(action, OverlayAction::Expand);
            {
                let state = app.state::<OverlayState>();
                let mut state = state.0.lock().unwrap();
                state.enabled = true;
                state.expanded = expanded;
            }
            if show_entry(app, expanded, expanded).is_err() {
                app.state::<OverlayState>().0.lock().unwrap().window_error =
                    Some("PA_OVERLAY_WINDOW_FAILED");
                publish(app);
                return Err("PA_OVERLAY_WINDOW_FAILED");
            }
            let shortcut = view(app).shortcut;
            register_shortcut(app, &shortcut);
        }
    }
    publish(app);
    Ok(())
}

pub fn setup(app: &tauri::AppHandle) -> tauri::Result<()> {
    load_preferences(app);
    let window = ensure_window(app)?;
    restore_position(app, &window)?;
    if view(app).enabled {
        let _ = apply(app, OverlayAction::Enable);
    }
    let expand = tauri::menu::MenuItem::with_id(app, "expand", "展开对话", true, None::<&str>)?;
    let enable = tauri::menu::MenuItem::with_id(app, "enable", "启用常驻入口", true, None::<&str>)?;
    let disable =
        tauri::menu::MenuItem::with_id(app, "disable", "停用常驻入口", true, None::<&str>)?;
    let main = tauri::menu::MenuItem::with_id(app, "main", "打开主窗口", true, None::<&str>)?;
    let exit = tauri::menu::MenuItem::with_id(app, "exit", "退出应用", true, None::<&str>)?;
    let menu = tauri::menu::Menu::with_items(app, &[&expand, &enable, &disable, &main, &exit])?;
    let mut tray = tauri::tray::TrayIconBuilder::with_id("resident")
        .tooltip("Project Assistant")
        .menu(&menu)
        .on_menu_event(|app, event| {
            let action = match event.id.as_ref() {
                "expand" => OverlayAction::Expand,
                "enable" => OverlayAction::Enable,
                "disable" => OverlayAction::Disable,
                "main" => OverlayAction::OpenMain,
                "exit" => OverlayAction::Exit,
                _ => return,
            };
            // Window rebuilding must not block the Windows event loop.
            let app = app.clone();
            std::thread::spawn(move || {
                let _ = apply(&app, action);
            });
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.state::<OverlayState>().0.lock().unwrap().tray_ready = true;
    let _ = persist_preferences(app);
    publish(app);
    let app = app.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if view(&app).enabled {
                if let Some(window) = app.get_webview_window("overlay") {
                    let _ = correct_position(&window);
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clamping_handles_removed_monitors_negative_origins_and_large_windows() {
        assert_eq!(clamp_axis(4000, 0, 1920, 390), 1530);
        assert_eq!(clamp_axis(-4000, -1920, 1920, 72), -1920);
        assert_eq!(clamp_axis(200, 0, 320, 390), 0);
    }
}
