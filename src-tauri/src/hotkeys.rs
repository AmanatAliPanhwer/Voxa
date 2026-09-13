use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::session::{Activation, Inbound};

pub fn start(
    app: &AppHandle,
    inbox: &tokio::sync::mpsc::Sender<Inbound>,
    chord: &str,
) -> Result<(), tauri_plugin_global_shortcut::Error> {
    let inbox = inbox.clone();
    app.global_shortcut()
        .on_shortcut(chord, move |_app, _shortcut, event| {
            let activation = match event.state {
                ShortcutState::Pressed => Activation::HoldBegan,
                ShortcutState::Released => Activation::HoldReleased,
            };
            let _ = inbox.try_send(Inbound::Activation(activation));
        })
}