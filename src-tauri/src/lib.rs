mod capture;
mod cleanup;
mod config;
mod error;
mod frontend;
mod hotkeys;
mod insert;
mod session;
mod store;
mod transcribe;

use frontend::AppState;
use session::{Broadcast, Event, Inbound, Session, State};
use tauri::menu::{Menu, MenuItem};
use tauri::{Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

struct TauriBroadcast {
    app: tauri::AppHandle,
}

impl Broadcast for TauriBroadcast {
    fn emit(&self, event: Event) {
        let _ = match event {
            Event::State(state) => self.app.emit("session:state", state),
            Event::Levels(level) => self.app.emit("session:levels", level),
            Event::Progress { phase, percent } => {
                self.app.emit("session:progress", serde_json::json!({ "phase": phase, "percent": percent }))
            }
            Event::Error(err) => self.app.emit("session:error", err),
            Event::Clip { outcome, target, timestamp } => {
                self.app.emit("session:clip", serde_json::json!({ "outcome": outcome, "target": target, "timestamp": timestamp }))
            }
        };
    }
}

async fn session_loop(
    session: &mut Session,
    mut inbox: tokio::sync::mpsc::Receiver<Inbound>,
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
                }
                if session.state() == State::Done {
                    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
                    session.finish_done();
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                session.drain_levels();
            }
        }
    }
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
        .setup(|app| {
            let app_handle = app.handle().clone();

            let app_data = app
                .path()
                .app_data_dir()
                .expect("app data dir must resolve");
            let cfg = config::load(&app_data);

            let (state_tx, state_rx) = tokio::sync::watch::channel(State::Idle);
            let (inbox_tx, inbox_rx) = tokio::sync::mpsc::channel::<Inbound>(32);

            let mut session = Session::new(
                Box::new(TauriBroadcast {
                    app: app_handle.clone(),
                }),
                Box::new(capture::StubCapture::default()),
                Box::new(transcribe::StubTranscriber),
                Box::new(cleanup::PassthroughCleaner),
                Box::new(insert::NaiveInserter),
                Some(state_tx.clone()),
            );
            tauri::async_runtime::spawn(async move {
                session_loop(&mut session, inbox_rx).await;
            });

            let config_lock = std::sync::Mutex::new(cfg.clone());
            let hotkeys = std::sync::Arc::new(std::sync::Mutex::new(hotkeys::Hotkeys::new(
                inbox_tx.clone(),
            )));
            hotkeys::spawn_ticker(hotkeys.clone());
            app.manage(AppState {
                inbox: inbox_tx.clone(),
                state_rx,
                config: config_lock,
                hotkeys,
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

            WebviewWindowBuilder::new(
                app,
                "settings",
                WebviewUrl::App("settings.html".into()),
            )
            .title("Voxa Settings")
            .inner_size(760.0, 560.0)
            .build()?;

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
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
            }
        });
}