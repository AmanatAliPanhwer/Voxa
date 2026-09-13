use serde::Serialize;
use tauri::{Emitter, Manager, State};

use crate::frontend::AppState;

#[derive(Serialize)]
pub struct WizardState {
    pub step: usize,
    pub hotkey: String,
    pub recovery_hotkey: String,
    pub launch_at_login: bool,
    pub sounds: bool,
    pub compute_device: String,
    pub models: Vec<ModelInfo>,
    pub selected_model: String,
    pub downloading: bool,
    pub progress: f32,
}

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub size_mb: u32,
    pub downloaded: bool,
}

pub fn wizard_state(app: tauri::AppHandle) -> WizardState {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();
    let models = crate::model::all()
        .iter()
        .map(|spec| ModelInfo {
            id: spec.id.into(),
            name: spec.name.into(),
            size_mb: spec.size_mb as u32,
            downloaded: crate::model::downloaded(&state.models_dir, spec),
        })
        .collect();
    let selected = cfg.model_id.clone();
    let downloading = state.wizard_downloading.load(std::sync::atomic::Ordering::Relaxed);
    let progress = state.wizard_progress.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0;
    WizardState {
        step: state.wizard_step.load(std::sync::atomic::Ordering::Relaxed),
        hotkey: cfg.hotkey,
        recovery_hotkey: cfg.recovery_hotkey,
        launch_at_login: cfg.launch_at_login,
        sounds: cfg.sounds,
        compute_device: cfg.compute_device,
        models,
        selected_model: selected,
        downloading,
        progress,
    }
}

pub fn wizard_set_step(_app: tauri::AppHandle, state: State<'_, AppState>, step: usize) -> Result<(), String> {
    let current = state.wizard_step.load(std::sync::atomic::Ordering::Relaxed);
    if step > current + 1 {
        return Err("cannot skip steps".into());
    }
    state.wizard_step.store(step, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

pub fn wizard_close(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.wizard_step.store(5, std::sync::atomic::Ordering::Relaxed);
    let mut cfg = state.config.lock().unwrap();
    cfg.first_run = false;
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::config::save(&app_data, &cfg).map_err(|e| e.to_string())?;
    drop(cfg);
    let _ = app.get_webview_window("wizard").map(|w| w.hide());
    Ok(())
}

pub fn wizard_download(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let Some(spec) = crate::model::find(&id) else {
        return Err("unknown model".into());
    };
    let app_handle = app.clone();
    let downloading = state.wizard_downloading.clone();
    let progress = state.wizard_progress.clone();
    let models_dir = state.models_dir.clone();
    downloading.store(true, std::sync::atomic::Ordering::Relaxed);
    progress.store(0, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(move || {
        let result = crate::model::ensure(&models_dir, spec, |p| {
            progress.store((p * 100.0) as u32, std::sync::atomic::Ordering::Relaxed);
            let _ = app_handle.emit(
                "wizard:progress",
                serde_json::json!({ "id": spec.id, "percent": p }),
            );
        });
        downloading.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Err(e) = result {
            let _ = app_handle.emit("wizard:error", e.detail);
        }
    });
    Ok(())
}

#[tauri::command]
pub fn wizard_state_cmd(app: tauri::AppHandle, _state: State<'_, AppState>) -> WizardState {
    wizard_state(app)
}

#[tauri::command]
pub fn wizard_set_step_cmd(app: tauri::AppHandle, state: State<'_, AppState>, step: usize) -> Result<(), String> {
    wizard_set_step(app, state, step)
}

#[tauri::command]
pub fn wizard_close_cmd(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    wizard_close(app, state)
}

#[tauri::command]
pub fn wizard_download_cmd(app: tauri::AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    wizard_download(app, state, id)
}

#[tauri::command]
pub fn model_list_wizard(_app: tauri::AppHandle, state: State<'_, AppState>) -> Vec<ModelInfo> {
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