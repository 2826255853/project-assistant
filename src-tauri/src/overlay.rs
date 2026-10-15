//! Resident entry lifecycle; all persistent preferences use the SQLite settings service.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
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
struct OverlayPreferences {
    enabled: bool,
    shortcut: String,
    position: Option<crate::settings::Position>,
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

pub struct OverlayState(pub Mutex<OverlayView>, Mutex<OverlayPreferences>);
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

fn load_preferences(app: &tauri::AppHandle) {
    let failure = *app
        .state::<crate::settings::SettingsState>()
        .failure
        .lock()
        .unwrap();
    let result = crate::settings::get(app);
    if failure.is_some() || result.is_err() {
        let state = app.state::<OverlayState>();
        state.0.lock().unwrap().enabled = false;
        state.0.lock().unwrap().window_error = Some("PA_SETTINGS_UNAVAILABLE");
        return;
    }
    let (enabled, shortcut, position) = result.unwrap().resident();
    let preferences = OverlayPreferences {
        enabled,
        shortcut,
        position,
    };
    let state = app.state::<OverlayState>();
    *state.1.lock().unwrap() = preferences.clone();
    let mut view = state.0.lock().unwrap();
    view.enabled = preferences.enabled;
    view.shortcut = preferences.shortcut;
}

pub fn persist_preferences(app: &tauri::AppHandle) -> Result<(), crate::ipc::ErrorCode> {
    crate::settings::save_position(app)
}

pub fn sync_position(app: &tauri::AppHandle, position: crate::settings::Position) {
    app.state::<OverlayState>().1.lock().unwrap().position = Some(position);
}

pub fn sample_position(app: &tauri::AppHandle) -> Option<crate::settings::Position> {
    let mut position_value = None;
    if let Some(window) = app.get_webview_window("overlay") {
        if let Ok(position) = window.outer_position() {
            let monitor = window.current_monitor().ok().flatten();
            position_value = Some(crate::settings::Position {
                x: position.x,
                y: position.y,
                monitor: monitor.as_ref().and_then(|m| m.name().cloned()),
                scale_factor: monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0),
            });
        }
    }
    position_value
}

fn restore_position(app: &tauri::AppHandle, window: &WebviewWindow) -> tauri::Result<()> {
    let position = app
        .state::<OverlayState>()
        .1
        .lock()
        .unwrap()
        .position
        .clone();
    if let Some(position) = position {
        let monitors = window.available_monitors()?;
        let monitor = monitors
            .iter()
            .find(|m| m.name() == position.monitor.as_ref())
            .cloned()
            .or(window.primary_monitor()?);
        if let Some(monitor) = monitor {
            let area = monitor.work_area();
            let found = monitor.name() == position.monitor.as_ref();
            let size = window.outer_size()?;
            let ratio = monitor.scale_factor() / position.scale_factor;
            let (x, y) = if found {
                (
                    area.position.x.saturating_add(
                        ((position.x as f64 - area.position.x as f64) * ratio) as i32,
                    ),
                    area.position.y.saturating_add(
                        ((position.y as f64 - area.position.y as f64) * ratio) as i32,
                    ),
                )
            } else {
                (
                    area.position
                        .x
                        .saturating_add(area.size.width.saturating_sub(size.width + 24) as i32),
                    area.position
                        .y
                        .saturating_add(area.size.height.saturating_sub(size.height + 100) as i32),
                )
            };
            window.set_position(tauri::PhysicalPosition::new(
                clamp_axis(x, area.position.x, area.size.width, size.width),
                clamp_axis(y, area.position.y, area.size.height, size.height),
            ))?;
        }
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
    crate::app_state::publish(app, crate::ipc::correlation_id());
}

pub fn validate_action(
    value: serde_json::Value,
    overlay_caller: bool,
) -> Result<OverlayAction, crate::ipc::ErrorCode> {
    use crate::ipc::ErrorCode;
    let params: serde_json::Value = crate::ipc::parse_request(value)?;
    let fields = params.as_object().ok_or(ErrorCode::InvalidRequest)?;
    let allowed: &[&str] = match fields.get("action").and_then(|v| v.as_str()) {
        Some("draft") => &["action", "draft"],
        Some("set_shortcut") => &["action", "shortcut"],
        _ => &["action"],
    };
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ErrorCode::InvalidRequest);
    }
    let action: OverlayAction =
        serde_json::from_value(params).map_err(|_| ErrorCode::InvalidRequest)?;
    if overlay_caller
        && matches!(
            action,
            OverlayAction::Exit | OverlayAction::SetShortcut { .. } | OverlayAction::RepairPosition
        )
    {
        return Err(ErrorCode::PermissionDenied);
    }
    if !overlay_caller && matches!(action, OverlayAction::Draft { .. }) {
        return Err(ErrorCode::PermissionDenied);
    }
    match &action {
        OverlayAction::Draft { draft } if draft.len() > 32768 => {
            return Err(ErrorCode::InputTooLarge);
        }
        OverlayAction::SetShortcut { shortcut } if shortcut.len() > 80 => {
            return Err(ErrorCode::InputTooLarge);
        }
        OverlayAction::SetShortcut { shortcut }
            if shortcut
                .parse::<tauri_plugin_global_shortcut::Shortcut>()
                .is_err() =>
        {
            return Err(ErrorCode::ShortcutInvalid);
        }
        _ => {}
    }
    Ok(action)
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
    Ok(())
}

// Called under the settings writer lock, before SQLite commits. The caller restores
// the old preferences on any window/registration/commit failure.
pub fn apply_settings(
    app: &tauri::AppHandle,
    next: &crate::settings::View,
    expanded: bool,
) -> Result<(), crate::ipc::ErrorCode> {
    use crate::ipc::ErrorCode;
    let (enabled, shortcut, position) = next.resident();
    let old = view(app);
    let shortcut_changed = old
        .shortcut
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .ok()
        != shortcut
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .ok();
    if (enabled || shortcut != old.shortcut)
        && !app.global_shortcut().is_registered(shortcut.as_str())
    {
        if app.global_shortcut().register(shortcut.as_str()).is_err() {
            app.state::<OverlayState>().0.lock().unwrap().shortcut_error =
                Some("PA_SHORTCUT_CONFLICT");
            return Err(ErrorCode::ShortcutConflict);
        }
    }
    let state = app.state::<OverlayState>();
    let position_changed = state.1.lock().unwrap().position != position;
    *state.1.lock().unwrap() = OverlayPreferences {
        enabled,
        shortcut: shortcut.clone(),
        position,
    };
    {
        let mut view = state.0.lock().unwrap();
        view.enabled = enabled;
        view.expanded = enabled && expanded;
        view.shortcut = shortcut.clone();
        view.shortcut_error = None;
    }
    let window = ensure_window(app).map_err(|_| ErrorCode::OverlayControlFailed)?;
    if position_changed {
        restore_position(app, &window).map_err(|_| ErrorCode::OverlayControlFailed)?;
    }
    if enabled {
        show_entry(app, expanded, false).map_err(|_| ErrorCode::OverlayControlFailed)?;
        if view(app).window_error.is_some() {
            return Err(ErrorCode::OverlayControlFailed);
        }
    } else {
        window.hide().map_err(|_| ErrorCode::OverlayControlFailed)?;
        show_main(app);
    }
    if shortcut_changed && app.global_shortcut().is_registered(old.shortcut.as_str()) {
        app.global_shortcut()
            .unregister(old.shortcut.as_str())
            .map_err(|_| ErrorCode::SettingsRecoveryFailed)?;
    }
    if !enabled && app.global_shortcut().is_registered(shortcut.as_str()) {
        app.global_shortcut()
            .unregister(shortcut.as_str())
            .map_err(|_| ErrorCode::SettingsRecoveryFailed)?;
    }
    Ok(())
}

fn register_shortcut(app: &tauri::AppHandle, candidate: &str) {
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
}

pub fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn tray_action(app: &tauri::AppHandle, id: &str) -> Result<(), crate::ipc::ErrorCode> {
    let action = match id {
        "expand" => OverlayAction::Expand,
        "enable" => OverlayAction::Enable,
        "disable" => OverlayAction::Disable,
        "main" => OverlayAction::OpenMain,
        "exit" => OverlayAction::Exit,
        _ => return Err(crate::ipc::ErrorCode::InvalidRequest),
    };
    apply(app, action)
}

pub fn apply(app: &tauri::AppHandle, action: OverlayAction) -> Result<(), crate::ipc::ErrorCode> {
    apply_with_id(app, action, crate::ipc::correlation_id())
}

pub fn apply_with_id(
    app: &tauri::AppHandle,
    action: OverlayAction,
    correlation_id: String,
) -> Result<(), crate::ipc::ErrorCode> {
    use crate::{ipc::ErrorCode, settings::Change};
    if matches!(action, OverlayAction::Expand) && !view(app).enabled {
        crate::settings::change_resident(app, vec![Change::OverlayEnabled(true)])?;
    }
    // Persistent actions always go through the settings transaction; do not hold
    // the window lock while calling back into that service.
    match &action {
        OverlayAction::Enable => {
            return crate::settings::change_resident(app, vec![Change::OverlayEnabled(true)]);
        }
        OverlayAction::Disable => {
            return crate::settings::change_resident(app, vec![Change::OverlayEnabled(false)]);
        }
        OverlayAction::SetShortcut { shortcut } => {
            return crate::settings::change_resident(
                app,
                vec![Change::OverlayShortcut(shortcut.clone())],
            );
        }
        _ => {}
    }
    if matches!(action, OverlayAction::Exit) {
        let _ = persist_preferences(app);
    }
    let state = app.state::<crate::settings::SettingsState>();
    let operation = state.operation.lock().unwrap();
    match action {
        OverlayAction::Exit => {
            let _ = app.global_shortcut().unregister_all();
            app.exit(0);
            return Ok(());
        }
        OverlayAction::OpenMain => show_main(app),
        OverlayAction::SetShortcut { .. } | OverlayAction::Enable | OverlayAction::Disable => {
            unreachable!()
        }
        OverlayAction::Draft { draft } => {
            if draft.len() > 32768 {
                return Err(ErrorCode::InputTooLarge);
            }
            app.state::<OverlayState>().0.lock().unwrap().draft = draft;
            crate::app_state::publish(app, correlation_id);
            return Ok(());
        }
        OverlayAction::Expand | OverlayAction::Collapse | OverlayAction::RepairPosition => {
            if matches!(action, OverlayAction::RepairPosition) {
                if let Some(window) = app.get_webview_window("overlay") {
                    correct_position(&window).map_err(|_| ErrorCode::OverlayControlFailed)?;
                }
                drop(operation);
                persist_preferences(app)?;
                return Ok(());
            }
            if !view(app).enabled {
                return Err(ErrorCode::OverlayControlFailed);
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
                crate::app_state::publish(app, correlation_id);
                return Err(ErrorCode::OverlayControlFailed);
            }
            let shortcut = view(app).shortcut;
            register_shortcut(app, &shortcut);
        }
    }
    crate::app_state::publish(app, correlation_id);
    Ok(())
}

pub fn setup(app: &tauri::AppHandle) -> tauri::Result<()> {
    load_preferences(app);
    let window = ensure_window(app)?;
    restore_position(app, &window)?;
    if view(app).enabled {
        let shortcut = view(app).shortcut;
        register_shortcut(app, &shortcut);
        let _ = show_entry(app, false, false);
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
            let id = event.id.as_ref().to_owned();
            // Window rebuilding must not block the Windows event loop.
            let app = app.clone();
            std::thread::spawn(move || {
                if let Err(code) = tray_action(&app, &id) {
                    *app.state::<crate::settings::SettingsState>()
                        .failure
                        .lock()
                        .unwrap() = Some(code);
                    publish(&app);
                }
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
    fn strict_requests_limits_and_window_roles_precede_mutation() {
        use crate::ipc::ErrorCode;
        use serde_json::json;
        for request in [
            json!({"apiVersion":1,"params":{"action":"enable","secret":"synthetic-secret"}}),
            json!({"apiVersion":1,"params":{"action":"draft","draft":false}}),
            json!({"apiVersion":1,"params":{"action":"unknown"}}),
            json!({"apiVersion":1,"params":{"action":"enable"},"correlationId":"forged"}),
        ] {
            assert!(matches!(
                validate_action(request, false),
                Err(ErrorCode::InvalidRequest)
            ));
        }
        assert!(matches!(
            validate_action(json!({"apiVersion":2,"params":{"action":"enable"}}), false),
            Err(ErrorCode::VersionUnsupported)
        ));
        for action in ["exit", "repair_position"] {
            assert!(matches!(
                validate_action(json!({"apiVersion":1,"params":{"action":action}}), true),
                Err(ErrorCode::PermissionDenied)
            ));
        }
        assert!(matches!(
            validate_action(
                json!({"apiVersion":1,"params":{"action":"set_shortcut","shortcut":"Ctrl+A"}}),
                true
            ),
            Err(ErrorCode::PermissionDenied)
        ));
        assert!(matches!(
            validate_action(
                json!({"apiVersion":1,"params":{"action":"draft","draft":"hello"}}),
                false
            ),
            Err(ErrorCode::PermissionDenied)
        ));
        assert!(matches!(
            validate_action(
                json!({"apiVersion":1,"params":{"action":"draft","draft":"中".repeat(10923)}}),
                true
            ),
            Err(ErrorCode::InputTooLarge)
        ));
        assert!(
            validate_action(
                json!({"apiVersion":1,"params":{"action":"draft","draft":"a".repeat(32768)}}),
                true
            )
            .is_ok()
        );
    }
    #[test]
    fn clamping_handles_removed_monitors_negative_origins_and_large_windows() {
        assert_eq!(clamp_axis(4000, 0, 1920, 390), 1530);
        assert_eq!(clamp_axis(-4000, -1920, 1920, 72), -1920);
        assert_eq!(clamp_axis(200, 0, 320, 390), 0);
    }
}
