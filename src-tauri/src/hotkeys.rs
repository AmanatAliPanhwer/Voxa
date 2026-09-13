use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::session::{Activation, Inbound};

const TAP_MS: u64 = 150;
const WINDOW_MS: u64 = 300;
const TICK_MS: u64 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Idle,
    Pressed { down: u64, second: bool },
    Holding { down: u64 },
    TapWindow { up: u64 },
    HoldWindow { up: u64 },
}

impl Default for Key {
    fn default() -> Self {
        Key::Idle
    }
}

pub struct Arbiter {
    key: Key,
    hands_free: bool,
}

impl Default for Arbiter {
    fn default() -> Self {
        Self {
            key: Key::Idle,
            hands_free: false,
        }
    }
}

impl Arbiter {
    pub fn key_down(&mut self, now: u64) -> Vec<Activation> {
        match self.key {
            Key::Idle => {
                self.key = Key::Pressed {
                    down: now,
                    second: false,
                };
                Vec::new()
            }
            Key::TapWindow { up } => {
                if now <= up.saturating_add(WINDOW_MS) {
                    self.key = Key::Pressed {
                        down: now,
                        second: true,
                    };
                } else {
                    self.key = Key::Pressed {
                        down: now,
                        second: false,
                    };
                }
                Vec::new()
            }
            Key::HoldWindow { up } => {
                if now <= up.saturating_add(WINDOW_MS) {
                    self.hands_free = true;
                    self.key = Key::Holding { down: now };
                    vec![Activation::ToggleOn]
                } else {
                    self.key = Key::Pressed {
                        down: now,
                        second: false,
                    };
                    Vec::new()
                }
            }
            Key::Pressed { .. } | Key::Holding { .. } => Vec::new(),
        }
    }

    pub fn key_up(&mut self, now: u64) -> Vec<Activation> {
        match self.key {
            Key::Pressed { down, second } => {
                let duration = now.saturating_sub(down);
                if duration <= TAP_MS {
                    if second {
                        self.hands_free = !self.hands_free;
                        let fired = if self.hands_free {
                            Activation::ToggleOn
                        } else {
                            Activation::ToggleOff
                        };
                        self.key = Key::Idle;
                        vec![fired]
                    } else {
                        self.key = Key::TapWindow { up: now };
                        Vec::new()
                    }
                } else if self.hands_free {
                    self.key = Key::Idle;
                    Vec::new()
                } else {
                    self.key = Key::HoldWindow { up: now };
                    vec![Activation::HoldBegan, Activation::HoldReleased]
                }
            }
            Key::Holding { .. } => {
                if self.hands_free {
                    self.key = Key::Idle;
                    Vec::new()
                } else {
                    self.key = Key::HoldWindow { up: now };
                    vec![Activation::HoldReleased]
                }
            }
            _ => Vec::new(),
        }
    }

    pub fn tick(&mut self, now: u64) -> Vec<Activation> {
        match self.key {
            Key::Pressed { down, .. } => {
                if now > down.saturating_add(TAP_MS) {
                    self.key = Key::Holding { down };
                    if self.hands_free {
                        Vec::new()
                    } else {
                        vec![Activation::HoldBegan]
                    }
                } else {
                    Vec::new()
                }
            }
            Key::TapWindow { up } => {
                if now > up.saturating_add(WINDOW_MS) {
                    self.key = Key::Idle;
                }
                Vec::new()
            }
            Key::HoldWindow { up } => {
                if now > up.saturating_add(WINDOW_MS) {
                    self.key = Key::Idle;
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn set_hands_free(&mut self, on: bool) {
        self.hands_free = on;
    }

    pub fn is_hands_free(&self) -> bool {
        self.hands_free
    }

    pub fn is_idle(&self) -> bool {
        self.key == Key::Idle
    }
}

pub struct Hotkeys {
    arbiter: Arbiter,
    inbox: tokio::sync::mpsc::Sender<Inbound>,
    notify: Option<Box<dyn Fn(bool) + Send>>,
}

impl Hotkeys {
    pub fn with_notify(
        inbox: tokio::sync::mpsc::Sender<Inbound>,
        notify: Option<Box<dyn Fn(bool) + Send>>,
    ) -> Self {
        Self {
            arbiter: Arbiter::default(),
            inbox,
            notify,
        }
    }

    pub fn key_down(&mut self, now: u64) {
        let was = self.arbiter.is_hands_free();
        let _ = self.inbox.try_send(Inbound::Arm);
        let events = self.arbiter.key_down(now);
        self.observe(was);
        self.flush(events);
    }

    pub fn key_up(&mut self, now: u64) {
        let was = self.arbiter.is_hands_free();
        let events = self.arbiter.key_up(now);
        self.observe(was);
        self.flush(events);
    }

    pub fn tick(&mut self, now: u64) {
        let was = self.arbiter.is_hands_free();
        let events = self.arbiter.tick(now);
        self.observe(was);
        let empty = events.is_empty();
        self.flush(events);
        if empty && !self.arbiter.is_hands_free() && self.arbiter.is_idle() {
            let _ = self.inbox.try_send(Inbound::Disarm);
        }
    }

    pub fn set_hands_free(&mut self, on: bool) {
        self.arbiter.set_hands_free(on);
        if let Some(notify) = &self.notify {
            notify(on);
        }
    }

    pub fn is_hands_free(&self) -> bool {
        self.arbiter.is_hands_free()
    }

    fn flush(&self, events: Vec<Activation>) {
        if !events.is_empty() {
            let _ = self.inbox.try_send(Inbound::Activation(events));
        }
    }

    fn observe(&self, was: bool) {
        let now = self.arbiter.is_hands_free();
        if now != was {
            if let Some(notify) = &self.notify {
                notify(now);
            }
        }
    }
}

pub fn now_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_millis() as u64
}

pub fn register(
    app: &AppHandle,
    runner: &Arc<Mutex<Hotkeys>>,
    chord: &str,
) -> Result<(), tauri_plugin_global_shortcut::Error> {
    let runner = Arc::clone(runner);
    app.global_shortcut()
        .on_shortcut(chord, move |_app, _shortcut, event| {
            let now = now_ms();
            let mut guard = runner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match event.state {
                ShortcutState::Pressed => guard.key_down(now),
                ShortcutState::Released => guard.key_up(now),
            }
        })
}

pub fn set_chord(
    app: &AppHandle,
    runner: &Arc<Mutex<Hotkeys>>,
    chord: &str,
) -> Result<(), tauri_plugin_global_shortcut::Error> {
    app.global_shortcut().unregister_all()?;
    register(app, runner, chord)
}

pub fn register_direct(
    app: &AppHandle,
    inbox: &tokio::sync::mpsc::Sender<Inbound>,
    chord: &str,
) -> Result<(), tauri_plugin_global_shortcut::Error> {
    let inbox = inbox.clone();
    app.global_shortcut()
        .on_shortcut(chord, move |_app, _shortcut, event| {
            if matches!(event.state, ShortcutState::Pressed) {
                let _ = inbox.try_send(Inbound::InsertLast);
            }
        })
}

pub fn spawn_ticker(runner: Arc<Mutex<Hotkeys>>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
            let now = now_ms();
            let mut guard = runner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.tick(now);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn hotkeys_with_notifier() -> (Hotkeys, mpsc::Receiver<bool>) {
        let (tx, rx) = mpsc::channel::<bool>();
        let (inbox_tx, _inbox_rx) = tokio::sync::mpsc::channel::<Inbound>(8);
        let notify_tx = tx.clone();
        let h = Hotkeys::with_notify(
            inbox_tx,
            Some(Box::new(move |armed: bool| {
                let _ = notify_tx.send(armed);
            })),
        );
        (h, rx)
    }

    #[test]
    fn hotkeys_notifies_lone_tap_stays_silent() {
        let (mut h, rx) = hotkeys_with_notifier();
        h.key_down(0);
        h.key_up(100);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn hotkeys_notifies_double_tap_on_and_off() {
        let (mut h, rx) = hotkeys_with_notifier();
        h.key_down(0);
        h.key_up(100);
        h.key_down(150);
        h.key_up(200);
        assert_eq!(rx.recv(), Ok(true));
        h.key_down(250);
        h.key_up(300);
        h.key_down(320);
        h.key_up(360);
        assert_eq!(rx.recv(), Ok(false));
    }

    #[test]
    fn hotkeys_notifies_set_hands_free() {
        let (mut h, rx) = hotkeys_with_notifier();
        h.set_hands_free(true);
        assert_eq!(rx.recv(), Ok(true));
        h.set_hands_free(false);
        assert_eq!(rx.recv(), Ok(false));
    }

    #[test]
    fn hold_crossing_threshold_emits_hold_began() {
        let mut a = Arbiter::default();
        assert!(a.key_down(0).is_empty());
        assert!(a.tick(150).is_empty());
        assert_eq!(a.tick(151), vec![Activation::HoldBegan]);
        assert_eq!(a.key_up(300), vec![Activation::HoldReleased]);
    }

    #[test]
    fn hold_at_exactly_150ms_is_a_tap() {
        let mut a = Arbiter::default();
        a.key_down(0);
        assert!(a.key_up(150).is_empty());
        assert!(a.tick(400).is_empty());
        a.key_up(500);
    }

    #[test]
    fn hold_noticed_only_at_release_emits_both_borders() {
        let mut a = Arbiter::default();
        a.key_down(0);
        assert_eq!(
            a.key_up(180),
            vec![Activation::HoldBegan, Activation::HoldReleased]
        );
    }

    #[test]
    fn lone_tap_is_a_silent_noop() {
        let mut a = Arbiter::default();
        a.key_down(0);
        assert!(a.key_up(100).is_empty());
        assert!(a.tick(341).is_empty());
        assert!(a.tick(401).is_empty());
        assert!(!a.is_hands_free());
        assert_eq!(a.key, Key::Idle);
    }

    #[test]
    fn double_tap_arms_and_disarms_hands_free() {
        let mut a = Arbiter::default();
        a.key_down(0);
        assert!(a.key_up(100).is_empty());
        assert!(a.key_down(150).is_empty());
        assert_eq!(a.key_up(200), vec![Activation::ToggleOn]);
        assert!(a.is_hands_free());

        a.key_down(250);
        assert!(a.key_up(300).is_empty());
        assert!(a.key_down(320).is_empty());
        assert_eq!(a.key_up(360), vec![Activation::ToggleOff]);
        assert!(!a.is_hands_free());
    }

    #[test]
    fn double_tap_window_is_inclusive_at_300ms() {
        let mut a = Arbiter::default();
        a.set_hands_free(true);
        a.key_down(0);
        a.key_up(100);
        assert!(a.key_down(400).is_empty());
        assert_eq!(a.key_up(500), vec![Activation::ToggleOff]);
        assert!(!a.is_hands_free());
    }

    #[test]
    fn second_press_beyond_window_is_a_lone_tap() {
        let mut a = Arbiter::default();
        a.key_down(0);
        a.key_up(100);
        a.tick(401);
        a.key_down(450);
        assert!(a.key_up(500).is_empty());
        assert!(a.tick(801).is_empty());
        assert!(!a.is_hands_free());
    }

    #[test]
    fn second_press_that_holds_becomes_push_to_talk() {
        let mut a = Arbiter::default();
        a.key_down(0);
        a.key_up(100);
        a.key_down(120);
        assert_eq!(a.tick(271), vec![Activation::HoldBegan]);
        assert_eq!(a.key_up(400), vec![Activation::HoldReleased]);
        assert!(!a.is_hands_free());
    }

    #[test]
    fn lock_in_converts_live_hold_to_hands_free() {
        let mut a = Arbiter::default();
        a.key_down(0);
        assert_eq!(a.tick(151), vec![Activation::HoldBegan]);
        assert_eq!(a.key_up(160), vec![Activation::HoldReleased]);
        assert_eq!(a.key_down(170), vec![Activation::ToggleOn]);
        assert!(a.is_hands_free());
        assert!(a.key_up(180).is_empty());
        assert!(a.is_hands_free());

        a.key_down(200);
        a.key_up(250);
        a.key_down(270);
        assert_eq!(a.key_up(300), vec![Activation::ToggleOff]);
        assert!(!a.is_hands_free());
    }

    #[test]
    fn lock_in_window_is_inclusive_at_300ms() {
        let mut a = Arbiter::default();
        a.key_down(0);
        a.tick(151);
        a.key_up(160);
        assert_eq!(a.key_down(460), vec![Activation::ToggleOn]);
        assert!(a.is_hands_free());
    }

    #[test]
    fn lock_in_window_expiry_drops_to_idle() {
        let mut a = Arbiter::default();
        a.key_down(0);
        a.tick(151);
        a.key_up(160);
        a.tick(461);
        assert!(a.key_down(470).is_empty());
        assert!(a.key_up(500).is_empty());
        assert!(a.tick(801).is_empty());
        assert!(!a.is_hands_free());
    }

    #[test]
    fn hold_during_hands_free_is_ignored() {
        let mut a = Arbiter::default();
        a.set_hands_free(true);
        a.key_down(0);
        assert!(a.tick(151).is_empty());
        assert!(a.key_up(200).is_empty());
        assert!(a.is_hands_free());
    }

    #[test]
    fn orphaned_tap_dies_when_a_slow_press_arrives() {
        let mut a = Arbiter::default();
        a.key_down(0);
        a.key_up(100);
        assert!(a.key_down(120).is_empty());
        assert_eq!(a.tick(271), vec![Activation::HoldBegan]);
        assert_eq!(a.key_up(350), vec![Activation::HoldReleased]);
    }
}