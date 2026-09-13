use serde::Serialize;
use std::path::PathBuf;
use tauri::{Emitter, Manager, State};

use crate::config::Config;
use crate::session::{Inbound, State as SessionState};

pub struct AppState {
    pub inbox: tokio::sync::mpsc::Sender<Inbound>,
    pub state_rx: tokio::sync::watch::Receiver<SessionState>,
    pub config: std::sync::Mutex<Config>,
    pub hotkeys: std::sync::Arc<std::sync::Mutex<crate::hotkeys::Hotkeys>>,
    pub models_dir: PathBuf,
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
    if let Some(value) = patch.get("model_id").and_then(|v| v.as_str()) {
        cfg.model_id = value.to_owned();
    }
    if let Some(value) = patch.get("compute_device").and_then(|v| v.as_str()) {
        cfg.compute_device = value.to_owned();
    }
    if let Some(value) = patch.get("tone_preset").and_then(|v| v.as_str()) {
        cfg.tone_preset = value.to_owned();
    }
    if let Some(value) = patch.get("mic_device").and_then(|v| v.as_str()) {
        cfg.mic_device = Some(value.to_owned());
    }
    if let Some(value) = patch.get("launch_at_login").and_then(|v| v.as_bool()) {
        cfg.launch_at_login = value;
    }
    if let Some(value) = patch.get("sounds").and_then(|v| v.as_bool()) {
        cfg.sounds = value;
    }
    if let Some(value) = patch.get("first_run").and_then(|v| v.as_bool()) {
        cfg.first_run = value;
    }
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::config::save(&app_data, &cfg).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = cfg.clone();
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
pub fn cleanup_test_key() -> Result<bool, String> {
    Ok(false)
}