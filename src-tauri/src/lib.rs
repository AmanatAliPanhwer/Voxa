mod capture;
mod cleanup;
mod config;
mod diagnostics;
mod error;
mod frontend;
mod hotkeys;
mod insert;
mod model;
mod session;
mod sounds;
mod store;
mod transcribe;
mod wizard;

use frontend::AppState;
use session::{Broadcast, Event, Inbound, Session, State};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::{Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

struct TauriBroadcast {
    app: tauri::AppHandle,
    sound: sounds::Sounder,
    log: diagnostics::Log,
}

impl Broadcast for TauriBroadcast {
    fn emit(&self, event: Event) {
        let _ = match event {
            Event::State(state) => {
                if state == State::Listening {
                    self.sound.play(sounds::Sound::Start);
                }
                self.app.emit("session:state", state)
            }
            Event::Levels(level) => self.app.emit("session:levels", level),
            Event::Progress { phase, percent } => {
                self.app.emit("session:progress", serde_json::json!({ "phase": phase, "percent": percent }))
            }
            Event::Error(err) => {
                self.log.line("error", err.kind, &err.detail);
                self.app.emit("session:error", err)
            }
            Event::Clip { outcome, target, timestamp } => {
                if outcome == "inserted" {
                    self.sound.play(sounds::Sound::Paste);
                }
                self.app.emit("session:clip", serde_json::json!({ "outcome": outcome, "target": target, "timestamp": timestamp }))
            }
            Event::Notify { title, body } => {
                use tauri_plugin_notification::NotificationExt;
                let _ = self
                    .app
                    .notification()
                    .builder()
                    .title(title)
                    .body(body)
                    .show();
                Ok(())
            }
        };
    }
}

async fn session_loop(
    session: &mut Session,
    mut inbox: tokio::sync::mpsc::Receiver<Inbound>,
    app: tauri::AppHandle,
    hands_free: std::sync::Arc<AtomicBool>,
) {
    loop {
        tokio::select! {
            inbound = inbox.recv() => {
                let Some(inbound) = inbound else { break };
                match inbound {
                    Inbound::Activation(events) => session.apply_batch(&events),
                    Inbound::InsertLast => session.insert_last_result(),
                    Inbound::StartHold => session.start_hold(),
                    Inbound::StopHold => session.stop_hold(),
                    Inbound::ToggleHandsFree => session.toggle_hands_free(),
                    Inbound::SetMicDevice(mic) => {
                        let _ = session.set_mic_device(mic);
                    }
                    Inbound::Arm => session.arm(),
                    Inbound::Disarm => session.unarm(),
                }
                if session.state() == State::Done {
                    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
                    session.finish_done();
                } else if session.state() == State::Error {
                    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
                    if session.state() == State::Error {
                        session.finish_error();
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                session.drain_levels();
                reconcile_bubble(&app, hands_free.load(Ordering::Relaxed), session.state() == State::Listening);
            }
        }
    }
}

fn native_wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn place_bubble(app: &tauri::AppHandle, bubble: &tauri::WebviewWindow) -> Result<(), String> {
    let pill = app
        .get_webview_window("pill")
        .ok_or_else(|| "no pill window".to_string())?;
    let pos = pill.outer_position().map_err(|e| e.to_string())?;
    let size = pill.outer_size().map_err(|e| e.to_string())?;
    let (left, right) = app
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .map(|monitor| {
            let r = monitor.size();
            (
                monitor.position().x as i32,
                monitor.position().x as i32 + r.width as i32,
            )
        })
        .unwrap_or((0, i32::MAX));
    let margin = 12;
    let gap = 6;
    let disc = 32;
    let left_x = pos.x - gap - disc;
    let right_x = pos.x + size.width as i32 + gap;
    let fits_left = left_x >= left + margin;
    let fits_right = right_x + disc <= right - margin;
    let x = if fits_left {
        left_x
    } else if fits_right {
        right_x
    } else {
        left_x.max(left)
    };
    let y = pos.y + (size.height as i32 - disc) / 2;
    bubble.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())
}

pub(crate) fn hide_bubble(app: &tauri::AppHandle) {
    if let Some(bubble) = app.get_webview_window("bubble") {
        let _ = bubble.hide();
    }
}

fn set_bubble_visible(app: &tauri::AppHandle, visible: bool) {
    let Some(bubble) = app.get_webview_window("bubble") else {
        return;
    };
    if visible {
        if place_bubble(app, &bubble).is_ok() {
            let _ = bubble.show();
        }
        return;
    }
    let _ = bubble.hide();
}

fn reconcile_bubble(app: &tauri::AppHandle, armed: bool, listening: bool) {
    let Some(bubble) = app.get_webview_window("bubble") else {
        return;
    };
    let want = armed && listening;
    let is_visible = bubble.is_visible().unwrap_or(false);
    if want == is_visible {
        return;
    }
    set_bubble_visible(app, want);
}

fn position_primary_bottom_center(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    width: f64,
    height: f64,
    above_bottom: f64,
) -> tauri::Result<()> {
    if let Some(monitor) = app.primary_monitor()? {
        let monitor_pos = monitor.position();
        let monitor_size = monitor.size();
        let x = monitor_pos.x + (monitor_size.width as i32 - width as i32) / 2;
        let y = monitor_pos.y + monitor_size.height as i32 - height as i32 - above_bottom as i32;
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            let app_handle = app.handle().clone();

            let app_data = app
                .path()
                .app_data_dir()
                .expect("app data dir must resolve");
            let cfg = config::load(&app_data);

            let (state_tx, state_rx) = tokio::sync::watch::channel(State::Idle);
            let (inbox_tx, inbox_rx) = tokio::sync::mpsc::channel::<Inbound>(32);
            let hands_free = std::sync::Arc::new(AtomicBool::new(false));
            let insert_overrides =
                std::sync::Arc::new(std::sync::Mutex::new(cfg.insert_overrides.clone()));
            let sounds_enabled = std::sync::Arc::new(AtomicBool::new(cfg.sounds));
            let tone_preset = std::sync::Arc::new(std::sync::Mutex::new(cfg.tone_preset.clone()));
            let log_dir = app_data.join("logs");
            let log = diagnostics::Log::new(log_dir.clone());
            log.raw("voxa started");

            let mut session = Session::new(
                Box::new(TauriBroadcast {
                    app: app_handle.clone(),
                    sound: sounds::Sounder::new(sounds_enabled.clone()),
                    log: diagnostics::Log::new(log_dir.clone()),
                }),
                Box::new(capture::CpalCapture::new(&cfg)),
                Box::new(transcribe::WhisperTranscriber::new(
                    app_data.join("models"),
                    cfg.model_id.clone(),
                    cfg.compute_device.clone(),
                    Box::new(TauriBroadcast {
                        app: app_handle.clone(),
                        sound: sounds::Sounder::new(sounds_enabled.clone()),
                        log: diagnostics::Log::new(log_dir.clone()),
                    }),
                )),
                Box::new(cleanup::GroqCleaner::new(tone_preset.clone())),
                Box::new(insert::ClipboardInserter::new(insert_overrides.clone())),
                cfg.recovery_hotkey.clone(),
                Some(state_tx.clone()),
            );
            let loop_hands_free = hands_free.clone();
            let loop_app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                session_loop(&mut session, inbox_rx, loop_app, loop_hands_free).await;
            });

            let config_lock = std::sync::Mutex::new(cfg.clone());
            let hands_free_arc = hands_free.clone();
            let app_for_notify = app_handle.clone();
            let hotkeys = std::sync::Arc::new(std::sync::Mutex::new(hotkeys::Hotkeys::with_notify(
                inbox_tx.clone(),
                Some(Box::new(move |armed| {
                    hands_free_arc.store(armed, Ordering::Relaxed);
                    let _ = app_for_notify.emit("bubble:armed", armed);
                })),
            )));
            hotkeys::spawn_ticker(hotkeys.clone());
            if cfg.launch_at_login {
                use tauri_plugin_autostart::ManagerExt;
                let _ = app_handle.autolaunch().enable();
            }
app.manage(AppState {
                 inbox: inbox_tx.clone(),
                 state_rx,
                 config: config_lock,
                 hotkeys,
                 models_dir: app_data.join("models"),
                 hands_free,
                 tone_preset,
                 insert_overrides,
                 sounds: sounds_enabled,
                 logs_dir: log_dir,
                 wizard_step: std::sync::atomic::AtomicUsize::new(if cfg.first_run { 0 } else { 5 }),
                 wizard_downloading: std::sync::Arc::new(AtomicBool::new(false)),
                 wizard_progress: std::sync::Arc::new(AtomicU32::new(0)),
             });

            let pill = WebviewWindowBuilder::new(
                app,
                "pill",
                WebviewUrl::App("pill.html".into()),
            )
            .title("Voxa")
            .inner_size(140.0, 32.0)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .resizable(false)
            .build()?;
            pill.set_ignore_cursor_events(true)?;
            position_primary_bottom_center(&app_handle, &pill, 140.0, 32.0, 80.0)?;

            let bubble = WebviewWindowBuilder::new(
                app,
                "bubble",
                WebviewUrl::App("bubble.html".into()),
            )
            .title("Stop")
            .inner_size(32.0, 32.0)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .resizable(false)
            .visible(false)
            .build()?;
            if native_wayland() {
                bubble.set_ignore_cursor_events(true)?;
            }

            WebviewWindowBuilder::new(
                app,
                "settings",
                WebviewUrl::App("settings.html".into()),
            )
            .title("Voxa Settings")
            .inner_size(840.0, 620.0)
            .center()
            .build()?;

            let wizard_window = if cfg.first_run {
                let w = WebviewWindowBuilder::new(
                    app,
                    "wizard",
                    WebviewUrl::App("wizard.html".into()),
                )
                .title("Voxa Setup")
                .inner_size(520.0, 640.0)
                .center()
                .resizable(false)
                .build()?;
                w.set_focus()?;
                Some(w)
            } else {
                None
            };
            let _ = wizard_window;
            let settings_item =
                MenuItem::with_id(app, "open_settings", "Open Settings", true, None::<&str>)?;
            let start_hold_item =
                MenuItem::with_id(app, "start_hold", "Start Hold-to-Talk", true, None::<&str>)?;
            let stop_hold_item =
                MenuItem::with_id(app, "stop_hold", "Stop Hold-to-Talk", true, None::<&str>)?;
            let toggle_free_item =
                MenuItem::with_id(app, "toggle_hands_free", "Toggle Hands-Free", true, None::<&str>)?;
            let insert_item =
                MenuItem::with_id(app, "insert_last", "Insert Last Result", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &settings_item,
                    &start_hold_item,
                    &stop_hold_item,
                    &toggle_free_item,
                    &insert_item,
                    &quit_item,
                ],
            )?;
            let _tray = tauri::tray::TrayIconBuilder::with_id("voxa-tray")
                .icon(app.default_window_icon().expect("tray icon").clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open_settings" => {
                        let _ = frontend::app_open_settings(app.clone());
                    }
                    "start_hold" => {
                        let state = app.state::<AppState>();
                        let _ = state.inbox.try_send(Inbound::StartHold);
                    }
                    "stop_hold" => {
                        let state = app.state::<AppState>();
                        let _ = state.inbox.try_send(Inbound::StopHold);
                    }
                    "toggle_hands_free" => {
                        let state = app.state::<AppState>();
                        let listening = *state.state_rx.borrow() == State::Listening;
                        let _ = state.inbox.try_send(Inbound::ToggleHandsFree);
                        state
                            .hotkeys
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .set_hands_free(!listening);
                    }
                    "insert_last" => {
                        let _ = frontend::app_insert_last(app.clone());
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            let runner = app.state::<AppState>().hotkeys.clone();
            if let Err(err) = hotkeys::register(&app_handle, &runner, &cfg.hotkey) {
                eprintln!("hotkey registration failed (tray fallback): {err}");
            }
            if let Err(err) =
                hotkeys::register_direct(&app_handle, &inbox_tx, &cfg.recovery_hotkey)
            {
                eprintln!("recovery hotkey registration failed (tray fallback): {err}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            frontend::session_snapshot,
            frontend::app_open_settings,
            frontend::app_insert_last,
            frontend::app_quit,
            frontend::hotkeys_local_event,
            frontend::hotkeys_set_chord,
            frontend::settings_get,
            frontend::settings_apply,
            frontend::model_list,
            frontend::cleanup_test_key,
            frontend::cleanup_presets,
            frontend::cleanup_key_status,
            frontend::cleanup_key_save,
            frontend::cleanup_key_delete,
            frontend::capture_devices,
            frontend::model_download,
            frontend::model_delete,
            frontend::diagnostics_peek,
            frontend::diagnostics_reveal,
            frontend::bubble_stop,
            wizard::wizard_state_cmd,
            wizard::wizard_set_step_cmd,
            wizard::wizard_close_cmd,
            wizard::wizard_download_cmd,
            wizard::model_list_wizard,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
            }
        });
}