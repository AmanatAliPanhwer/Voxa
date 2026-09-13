use serde::Serialize;
use std::path::PathBuf;
use tauri::{Emitter, Manager, State};

use crate::config::{Config, InsertOverride};
use crate::session::{Inbound, State as SessionState};

pub struct AppState {
    pub inbox: tokio::sync::mpsc::Sender<Inbound>,
    pub state_rx: tokio::sync::watch::Receiver<SessionState>,
    pub config: std::sync::Mutex<Config>,
    pub hotkeys: std::sync::Arc<std::sync::Mutex<crate::hotkeys::Hotkeys>>,
    pub models_dir: PathBuf,
    pub hands_free: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub tone_preset: std::sync::Arc<std::sync::Mutex<String>>,
    pub insert_overrides: std::sync::Arc<std::sync::Mutex<Vec<InsertOverride>>>,
    pub sounds: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub logs_dir: PathBuf,
}

#[tauri::command]
pub fn session_snapshot(state: State<'_, AppState>) -> SessionState {
    *state.state_rx.borrow()
}

#[tauri::command]
pub fn app_open_settings(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("settings")
        .ok_or("no settings window")?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn app_insert_last(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state
        .inbox
        .try_send(Inbound::InsertLast)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn bubble_stop(app: tauri::AppHandle) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let state = app.state::<AppState>();
    let mut runner = state
        .hotkeys
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if runner.is_hands_free() {
        runner.set_hands_free(false);
        state.hands_free.store(false, Ordering::Relaxed);
    }
    drop(runner);
    state
        .inbox
        .try_send(Inbound::ToggleHandsFree)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn app_quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn hotkeys_local_event(
    state: State<'_, AppState>,
    kind: String,
) -> Result<(), String> {
    let now = crate::hotkeys::now_ms();
    let mut runner = state
        .hotkeys
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match kind.as_str() {
        "down" => {
            runner.key_down(now);
            Ok(())
        }
        "up" => {
            runner.key_up(now);
            Ok(())
        }
        _ => Err("unknown hotkey event".into()),
    }
}

#[tauri::command]
pub fn hotkeys_set_chord(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    chord: String,
) -> Result<(), String> {
    let runner = state.hotkeys.clone();
    crate::hotkeys::set_chord(&app, &runner, &chord).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn settings_apply(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> Result<Config, String> {
    let mut cfg = state.config.lock().unwrap().clone();
    if let Some(value) = patch.get("hotkey").and_then(|v| v.as_str()) {
        cfg.hotkey = value.to_owned();
    }
    if let Some(value) = patch.get("recovery_hotkey").and_then(|v| v.as_str()) {
        cfg.recovery_hotkey = value.to_owned();
    }
    if patch.get("hotkey").is_some() || patch.get("recovery_hotkey").is_some() {
        let runner = state.hotkeys.clone();
        if let Err(err) = crate::hotkeys::set_chord(&app, &runner, &cfg.hotkey) {
            return Err(format!("hotkey is reserved or unavailable: {err}"));
        }
        let _ = crate::hotkeys::register_direct(&app, &state.inbox, &cfg.recovery_hotkey);
    }
    if let Some(value) = patch.get("model_id").and_then(|v| v.as_str()) {
        cfg.model_id = value.to_owned();
    }
    if let Some(value) = patch.get("compute_device").and_then(|v| v.as_str()) {
        cfg.compute_device = value.to_owned();
    }
    if let Some(value) = patch.get("tone_preset").and_then(|v| v.as_str()) {
        cfg.tone_preset = value.to_owned();
        *state.tone_preset.lock().unwrap() = value.to_owned();
    }
    if let Some(value) = patch.get("mic_device") {
        cfg.mic_device = value.as_str().map(|v| v.to_owned());
    }
    if let Some(value) = patch.get("insert_overrides").and_then(|v| v.as_array()) {
        let rules: Vec<InsertOverride> = value
            .iter()
            .filter_map(|item| serde_json::from_value(item.clone()).ok())
            .collect();
        cfg.insert_overrides = rules.clone();
        *state.insert_overrides.lock().unwrap() = rules;
    }
    if let Some(value) = patch.get("launch_at_login").and_then(|v| v.as_bool()) {
        cfg.launch_at_login = value;
        use tauri_plugin_autostart::ManagerExt;
        let result = if value {
            app.autolaunch().enable()
        } else {
            app.autolaunch().disable()
        };
        result.map_err(|e| format!("launch-at-login: {e}"))?;
    }
    if let Some(value) = patch.get("sounds").and_then(|v| v.as_bool()) {
        cfg.sounds = value;
        state.sounds.store(value, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(value) = patch.get("first_run").and_then(|v| v.as_bool()) {
        cfg.first_run = value;
    }
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::config::save(&app_data, &cfg).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg.clone();
    if patch.get("mic_device").is_some() {
        let _ = state.inbox.try_send(Inbound::SetMicDevice(cfg.mic_device.clone()));
    }
    let _ = app.emit("config:changed", &cfg);
    Ok(cfg)
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub size_mb: u32,
    pub downloaded: bool,
}

#[tauri::command]
pub fn model_list(state: State<'_, AppState>) -> Vec<ModelInfo> {
    crate::model::all()
        .iter()
        .map(|spec| ModelInfo {
            id: spec.id.into(),
            name: spec.name.into(),
            size_mb: spec.size_mb as u32,
            downloaded: crate::model::downloaded(&state.models_dir, spec),
        })
        .collect()
}

#[tauri::command]
pub async fn model_download(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let Some(spec) = crate::model::find(&id) else {
        return Err("unknown model".into());
    };
    let models_dir = app.state::<AppState>().models_dir.clone();
    let app_for_progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::model::ensure(&models_dir, spec, |percent| {
            let _ = app_for_progress.emit(
                "model:progress",
                serde_json::json!({ "id": spec.id, "percent": percent }),
            );
        })
        .map(|_| ())
        .map_err(|e| e.detail)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn model_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(spec) = crate::model::find(&id) else {
        return Err("unknown model".into());
    };
    let path = crate::model::file_path(&state.models_dir, spec);
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let part = state.models_dir.join(format!("{}.part", spec.file));
    if part.is_file() {
        std::fs::remove_file(&part).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn capture_devices() -> Vec<String> {
    crate::capture::list_input_devices()
}

#[tauri::command]
pub fn diagnostics_peek(state: State<'_, AppState>) -> String {
    crate::diagnostics::Log::new(state.logs_dir.clone()).tail(8192)
}

#[tauri::command]
pub fn diagnostics_reveal(app: tauri::AppHandle) -> Result<(), String> {
    let logs_dir = app.state::<AppState>().logs_dir.clone();
    let _ = crate::diagnostics::Log::new(logs_dir.clone());
    reveal_dir(&logs_dir)
}

pub fn reveal_dir(dir: &PathBuf) -> Result<(), String> {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(&dir).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&dir).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&dir).spawn()
    };
    result.map(|_| ()).map_err(|e| format!("cannot open logs dir: {e}"))
}

#[derive(Clone, Debug, Serialize)]
pub struct TonePresetInfo {
    pub id: String,
    pub label: String,
}

#[tauri::command]
pub fn cleanup_presets() -> Vec<TonePresetInfo> {
    crate::cleanup::presets()
        .iter()
        .map(|preset| TonePresetInfo {
            id: preset.id.into(),
            label: preset.label.into(),
        })
        .collect()
}

#[tauri::command]
pub fn cleanup_key_status() -> Result<bool, String> {
    crate::cleanup::load_key()
        .map(|key| key.map(|k| !k.is_empty()).unwrap_or(false))
        .map_err(|e| e.detail)
}

#[tauri::command]
pub fn cleanup_key_save(key: String) -> Result<(), String> {
    crate::cleanup::save_key(key.trim()).map_err(|e| e.detail)
}

#[tauri::command]
pub fn cleanup_key_delete() -> Result<(), String> {
    crate::cleanup::delete_key().map_err(|e| e.detail)
}

#[tauri::command]
pub async fn cleanup_test_key(key: Option<String>) -> Result<bool, String> {
    let resolved = match key {
        Some(provided) => Some(provided),
        None => crate::cleanup::load_key().map_err(|e| e.detail)?,
    };
    let Some(resolved) = resolved else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || crate::cleanup::test_key(&resolved))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.detail)
}