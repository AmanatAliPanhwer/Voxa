use crate::error::ErrorInfo;
use crate::store::Store;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Idle,
    Listening,
    Processing,
    Inserting,
    Done,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    HoldBegan,
    HoldReleased,
    ToggleOn,
    ToggleOff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inbound {
    Activation(Vec<Activation>),
    InsertLast,
    StartHold,
    StopHold,
    ToggleHandsFree,
    SetMicDevice(Option<String>),
    Arm,
    Disarm,
}

pub trait Broadcast: Send {
    fn emit(&self, event: Event);
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    State(State),
    Levels(f32),
    Progress { phase: String, percent: f32 },
    Error(ErrorInfo),
    Clip { outcome: String, target: Option<String>, timestamp: u64 },
    Notify { title: String, body: String },
}

pub trait CaptureDevice: Send {
    fn start(&mut self) -> Result<(), ErrorInfo>;
    fn promote(&mut self) -> Result<(), ErrorInfo>;
    fn stop(&mut self) -> Vec<f32>;
    fn cancel(&mut self);
    fn levels(&mut self) -> Vec<f32>;
    fn take_error(&mut self) -> Option<ErrorInfo>;
    fn set_mic_device(&mut self, _mic: Option<String>) -> Result<(), ErrorInfo> {
        Ok(())
    }
}

pub trait Transcriber: Send {
    fn transcribe(&self, pcm: &[f32]) -> Result<String, ErrorInfo>;
}

pub trait Cleaner: Send {
    fn clean(&self, raw: &str) -> String;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted { target: Option<String> },
    PendingManualPaste { hint: String },
}

pub trait Inserter: Send {
    fn insert(&self, text: &str) -> Result<InsertOutcome, ErrorInfo>;
}

pub struct Session {
    state: State,
    broadcasts: Box<dyn Broadcast>,
    capture: Box<dyn CaptureDevice>,
    transcriber: Box<dyn Transcriber>,
    cleaner: Box<dyn Cleaner>,
    inserter: Box<dyn Inserter>,
    store: Store,
    recovery_chord: String,
    state_sink: Option<tokio::sync::watch::Sender<State>>,
    silent: bool,
}

impl Session {
    pub fn new(
        broadcasts: Box<dyn Broadcast>,
        capture: Box<dyn CaptureDevice>,
        transcriber: Box<dyn Transcriber>,
        cleaner: Box<dyn Cleaner>,
        inserter: Box<dyn Inserter>,
        recovery_chord: String,
        state_sink: Option<tokio::sync::watch::Sender<State>>,
    ) -> Self {
        Self {
            state: State::Idle,
            broadcasts,
            capture,
            transcriber,
            cleaner,
            inserter,
            store: Store::new(),
            recovery_chord,
            state_sink,
            silent: false,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn apply(&mut self, activation: Activation) {
        match activation {
            Activation::HoldBegan | Activation::ToggleOn => self.begin_listening(),
            Activation::HoldReleased | Activation::ToggleOff => self.end_listening(),
        }
    }

    pub fn apply_batch(&mut self, events: &[Activation]) {
        if events == [Activation::HoldReleased, Activation::ToggleOn] && self.state == State::Listening
        {
            return;
        }
        for activation in events {
            self.apply(*activation);
        }
    }

    pub fn start_hold(&mut self) {
        self.apply(Activation::HoldBegan);
    }

    pub fn stop_hold(&mut self) {
        self.apply(Activation::HoldReleased);
    }

    pub fn toggle_hands_free(&mut self) {
        if self.state == State::Listening {
            self.apply(Activation::ToggleOff);
        } else {
            self.apply(Activation::ToggleOn);
        }
    }

    pub fn arm(&mut self) {
        if self.state == State::Idle {
            let _ = self.capture.start();
        }
    }

    pub fn unarm(&mut self) {
        if self.state == State::Idle {
            self.capture.cancel();
        }
    }

    pub fn set_mic_device(&mut self, mic: Option<String>) -> Result<(), ErrorInfo> {
        self.capture.set_mic_device(mic)
    }

    pub fn insert_last_result(&mut self) {
        let last = match self.store.last() {
            Some(entry) => entry.clone(),
            None => return,
        };
        self.to(State::Inserting);
        match self.inserter.insert(&last.clean_text) {
            Ok(outcome) => self.settle_insert(outcome, last.timestamp),
            Err(err) => {
                self.notify(
                    "Insertion failed",
                    format!("Press {} to insert the last result again", self.recovery_chord),
                );
                self.fail(err);
            }
        }
    }

    pub fn drain_levels(&mut self) {
        if self.state != State::Listening {
            return;
        }
        if let Some(err) = self.capture.take_error() {
            self.fail(err);
            return;
        }
        for level in self.capture.levels() {
            if level <= 1e-3 {
                if !self.silent {
                    self.broadcasts.emit(Event::Levels(0.0));
                    self.silent = true;
                }
            } else {
                self.silent = false;
                self.broadcasts.emit(Event::Levels(level));
            }
        }
    }

    pub fn finish_done(&mut self) {
        if self.state == State::Done {
            self.to(State::Idle);
        }
    }

    fn begin_listening(&mut self) {
        if self.state != State::Idle {
            return;
        }
        match self.capture.promote() {
            Ok(()) => {
                self.silent = false;
                self.to(State::Listening);
            }
            Err(err) => self.fail(err),
        }
    }

    fn end_listening(&mut self) {
        if self.state != State::Listening {
            return;
        }
        let pcm = self.capture.stop();
        self.silent = false;
        if is_blank(&pcm) {
            self.to(State::Idle);
            return;
        }
        self.to(State::Processing);
        let raw = match self.transcriber.transcribe(&pcm) {
            Ok(raw) => raw,
            Err(err) => {
                self.fail(err);
                return;
            }
        };
        self.broadcasts.emit(Event::Progress {
            phase: "cleanup".into(),
            percent: 100.0,
        });
        let clean = self.cleaner.clean(&raw);
        self.to(State::Inserting);
        match self.inserter.insert(&clean) {
            Ok(outcome) => {
                let entry = self.store.push(clean, outcome_target(&outcome));
                self.settle_insert(outcome, entry.timestamp);
            }
            Err(err) => {
                self.store.push(clean, None);
                self.notify(
                    "Insertion failed",
                    format!("Press {} to insert the last result again", self.recovery_chord),
                );
                self.fail(err);
            }
        }
    }

    fn settle_insert(&mut self, outcome: InsertOutcome, timestamp: u64) {
        match outcome {
            InsertOutcome::Inserted { target } => {
                self.broadcasts.emit(Event::Clip {
                    outcome: "inserted".into(),
                    target,
                    timestamp,
                });
                self.to(State::Done);
            }
            InsertOutcome::PendingManualPaste { hint } => {
                self.notify("Clipboard ready", format!("Press {hint} to paste"));
                self.broadcasts.emit(Event::Clip {
                    outcome: "manual".into(),
                    target: None,
                    timestamp,
                });
                self.to(State::Done);
            }
        }
    }

    fn notify(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.broadcasts.emit(Event::Notify {
            title: title.into(),
            body: body.into(),
        });
    }

    fn fail(&mut self, err: ErrorInfo) {
        self.broadcasts.emit(Event::Error(err));
        self.to(State::Error);
    }

    fn to(&mut self, state: State) {
        self.state = state;
        if let Some(sink) = &self.state_sink {
            let _ = sink.send(state);
        }
        self.broadcasts.emit(Event::State(state));
    }
}

fn is_blank(pcm: &[f32]) -> bool {
    if pcm.is_empty() {
        return true;
    }
    let energy: f32 = pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32;
    energy < 1e-6
}

fn outcome_target(outcome: &InsertOutcome) -> Option<String> {
    match outcome {
        InsertOutcome::Inserted { target } => target.clone(),
        InsertOutcome::PendingManualPaste { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct TestBroadcast {
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl Broadcast for TestBroadcast {
        fn emit(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[derive(Default, Clone)]
    struct CallCount {
        start: usize,
        promote: usize,
        cancel: usize,
    }

    #[derive(Default)]
    struct FakeCapture {
        pcm: Vec<f32>,
        start_ok: bool,
        levels_in: Vec<f32>,
        levels_out: Vec<f32>,
        error: Option<ErrorInfo>,
        calls: Arc<Mutex<CallCount>>,
    }

    impl CaptureDevice for FakeCapture {
        fn start(&mut self) -> Result<(), ErrorInfo> {
            self.calls.lock().unwrap().start += 1;
            if self.start_ok {
                Ok(())
            } else {
                Err(ErrorInfo::new(ErrorKind::Capture, true, "no mic"))
            }
        }
        fn promote(&mut self) -> Result<(), ErrorInfo> {
            self.calls.lock().unwrap().promote += 1;
            if self.start_ok {
                Ok(())
            } else {
                Err(ErrorInfo::new(ErrorKind::Capture, true, "no mic"))
            }
        }
        fn stop(&mut self) -> Vec<f32> {
            std::mem::take(&mut self.pcm)
        }
        fn cancel(&mut self) {
            self.calls.lock().unwrap().cancel += 1;
        }
        fn levels(&mut self) -> Vec<f32> {
            self.levels_out.drain(..).collect()
        }
        fn take_error(&mut self) -> Option<ErrorInfo> {
            self.error.take()
        }
    }

    struct FakeTranscriber {
        result: Result<String, ErrorInfo>,
    }

    impl Transcriber for FakeTranscriber {
        fn transcribe(&self, _pcm: &[f32]) -> Result<String, ErrorInfo> {
            self.result.clone()
        }
    }

    #[derive(Clone)]
    struct FakeCleaner;

    impl Cleaner for FakeCleaner {
        fn clean(&self, raw: &str) -> String {
            format!("clean: {raw}")
        }
    }

    struct FakeInserter {
        result: Result<InsertOutcome, ErrorInfo>,
    }

    impl Inserter for FakeInserter {
        fn insert(&self, _text: &str) -> Result<InsertOutcome, ErrorInfo> {
            self.result.clone()
        }
    }

    fn inserted(target: Option<&str>) -> Result<InsertOutcome, ErrorInfo> {
        Ok(InsertOutcome::Inserted {
            target: target.map(str::to_owned),
        })
    }

    fn harness(
        capture: FakeCapture,
        transcribe: Result<String, ErrorInfo>,
        insert: Result<InsertOutcome, ErrorInfo>,
    ) -> (Session, Arc<Mutex<Vec<Event>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let session = Session {
            state: State::Idle,
            broadcasts: Box::new(TestBroadcast {
                events: events.clone(),
            }),
            capture: Box::new(capture),
            transcriber: Box::new(FakeTranscriber { result: transcribe }),
            cleaner: Box::new(FakeCleaner),
            inserter: Box::new(FakeInserter { result: insert }),
            store: Store::new(),
            recovery_chord: "Ctrl+Alt+V".into(),
            state_sink: None,
            silent: false,
        };
        (session, events)
    }

    fn states(events: &[Event]) -> Vec<State> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::State(s) => Some(*s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn hold_began_moves_idle_to_listening() {
        let (mut s, events) = harness(
            FakeCapture {
                start_ok: true,
                ..Default::default()
            },
            Ok("raw".into()),
            inserted(None),
        );
        s.apply(Activation::HoldBegan);
        assert_eq!(s.state(), State::Listening);
        assert_eq!(states(&events.lock().unwrap()), vec![State::Listening]);
    }

    #[test]
    fn capture_failure_enters_error() {
        let (mut s, _events) = harness(
            FakeCapture {
                start_ok: false,
                ..Default::default()
            },
            Ok("raw".into()),
            inserted(None),
        );
        s.apply(Activation::HoldBegan);
        assert_eq!(s.state(), State::Error);
    }

    #[test]
    fn blank_clip_is_a_silent_noop_back_to_idle() {
        let (mut s, events) = harness(
            FakeCapture {
                start_ok: true,
                ..Default::default()
            },
            Ok("raw".into()),
            inserted(None),
        );
        s.apply(Activation::HoldBegan);
        assert_eq!(s.state(), State::Listening);
        s.apply(Activation::HoldReleased);
        assert_eq!(s.state(), State::Idle);
        assert_eq!(states(&events.lock().unwrap()), vec![State::Listening, State::Idle]);
    }

    #[test]
    fn full_clip_runs_processing_inserting_done_and_stores_result() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw phrase".into()),
            inserted(Some("Notes")),
        );
        s.apply(Activation::HoldBegan);
        s.apply(Activation::HoldReleased);
        assert_eq!(s.state(), State::Done);
        let events = events.lock().unwrap();
        assert_eq!(
            states(&events),
            vec![
                State::Listening,
                State::Processing,
                State::Inserting,
                State::Done
            ]
        );
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Clip { outcome, target, .. }
                if outcome == "inserted" && target.as_deref() == Some("Notes")
        )));
        drop(events);
        let last = s.store.last().unwrap().clone();
        assert_eq!(last.clean_text, "clean: raw phrase");
        assert_eq!(last.target.as_deref(), Some("Notes"));
    }

    #[test]
    fn clean_text_falls_back_to_raw_when_cleaner_is_a_stub() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw phrase".into()),
            inserted(None),
        );
        s.apply(Activation::HoldBegan);
        s.apply(Activation::ToggleOff);
        assert_eq!(s.state(), State::Done);
        drop(events);
        assert_eq!(s.store.last().unwrap().clean_text, "clean: raw phrase");
    }

    #[test]
    fn transcribe_failure_enters_error_and_never_inserts() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Err(ErrorInfo::new(ErrorKind::Transcribe, true, "model missing")),
            inserted(None),
        );
        s.apply(Activation::HoldBegan);
        s.apply(Activation::HoldReleased);
        assert_eq!(s.state(), State::Error);
        let events = events.lock().unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Error(err) if err.kind == ErrorKind::Transcribe
        )));
    }

    #[test]
    fn insert_failure_keeps_result_and_enters_error() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw phrase".into()),
            Err(ErrorInfo::new(ErrorKind::Insert, true, "no foreground app")),
        );
        s.apply(Activation::HoldBegan);
        s.apply(Activation::HoldReleased);
        assert_eq!(s.state(), State::Error);
        let events = events.lock().unwrap();
        let error = events
            .iter()
            .find_map(|e| match e {
                Event::Error(err) => Some(err.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(error.kind, ErrorKind::Insert);
        drop(events);
        assert_eq!(s.store.len(), 1);
    }

    #[test]
    fn lock_in_stays_listening_and_never_splits_the_clip() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw phrase".into()),
            inserted(Some("Notes")),
        );
        s.apply(Activation::HoldBegan);
        assert_eq!(s.state(), State::Listening);
        s.apply_batch(&[Activation::HoldReleased, Activation::ToggleOn]);
        assert_eq!(s.state(), State::Listening);
        assert_eq!(states(&events.lock().unwrap()), vec![State::Listening]);
        s.apply(Activation::ToggleOff);
        assert_eq!(s.state(), State::Done);
        assert_eq!(
            states(&events.lock().unwrap()),
            vec![
                State::Listening,
                State::Processing,
                State::Inserting,
                State::Done
            ]
        );
    }

    #[test]
    fn tray_toggle_hands_free_starts_and_stops_listening() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw".into()),
            inserted(None),
        );
        s.toggle_hands_free();
        assert_eq!(s.state(), State::Listening);
        s.toggle_hands_free();
        assert_eq!(s.state(), State::Done);
        drop(events);
        assert_eq!(s.store.last().unwrap().clean_text, "clean: raw");
    }

    #[test]
    fn recovery_reinserts_last_result_after_failure() {
        let (mut s, _events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw phrase".into()),
            Err(ErrorInfo::new(ErrorKind::Insert, true, "no foreground app")),
        );
        s.apply(Activation::HoldBegan);
        s.apply(Activation::HoldReleased);
        assert_eq!(s.state(), State::Error);
        s.inserter = Box::new(FakeInserter {
            result: inserted(Some("Notes")),
        });
        s.insert_last_result();
        assert_eq!(s.state(), State::Done);
    }

    #[test]
    fn recovery_with_empty_history_is_a_noop() {
        let (mut s, _events) = harness(
            FakeCapture::default(),
            Ok("raw".into()),
            inserted(None),
        );
        s.insert_last_result();
        assert_eq!(s.state(), State::Idle);
    }

    #[test]
    fn done_returns_to_idle_after_the_flash() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw".into()),
            inserted(None),
        );
        s.apply(Activation::HoldBegan);
        s.apply(Activation::HoldReleased);
        assert_eq!(s.state(), State::Done);
        s.finish_done();
        assert_eq!(s.state(), State::Idle);
        assert_eq!(
            states(&events.lock().unwrap()).last(),
            Some(&State::Idle)
        );
    }

    #[test]
    fn arm_then_hold_promotes_capture() {
        let calls = Arc::new(Mutex::new(CallCount::default()));
        let capture = FakeCapture {
            start_ok: true,
            calls: calls.clone(),
            ..Default::default()
        };
        let (mut s, _events) = harness(capture, Ok("raw".into()), inserted(None));
        s.apply(Activation::HoldBegan);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.start, 0, "hold path must promote, not start");
        assert_eq!(calls.promote, 1);
    }

    #[test]
    fn key_down_arm_starts_capture_and_hold_promotes_it() {
        let calls = Arc::new(Mutex::new(CallCount::default()));
        let capture = FakeCapture {
            start_ok: true,
            calls: calls.clone(),
            ..Default::default()
        };
        let (mut s, events) = harness(capture, Ok("raw".into()), inserted(None));
        s.arm();
        s.apply(Activation::HoldBegan);
        assert_eq!(s.state(), State::Listening);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.start, 1);
        assert_eq!(calls.promote, 1);
    }

    #[test]
    fn unarm_cancels_capture_and_stays_idle() {
        let calls = Arc::new(Mutex::new(CallCount::default()));
        let capture = FakeCapture {
            start_ok: true,
            calls: calls.clone(),
            ..Default::default()
        };
        let (mut s, events) = harness(capture, Ok("raw".into()), inserted(None));
        s.arm();
        s.unarm();
        assert_eq!(s.state(), State::Idle);
        assert_eq!(states(&events.lock().unwrap()), Vec::<State>::new());
        assert_eq!(calls.lock().unwrap().cancel, 1);
    }

    #[test]
    fn arm_failure_is_deferred_until_promote() {
        let capture = FakeCapture {
            start_ok: false,
            ..Default::default()
        };
        let (mut s, events) = harness(capture, Ok("raw".into()), inserted(None));
        s.arm();
        assert_eq!(s.state(), State::Idle, "arm errors surface later");
        s.apply(Activation::HoldBegan);
        assert_eq!(s.state(), State::Error);
        assert!(events.lock().unwrap().iter().any(|e| matches!(
            e,
            Event::Error(err) if err.kind == ErrorKind::Capture
        )));
    }

    #[test]
    fn manual_paste_tier_notifies_and_marks_outcome_manual() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw phrase".into()),
            Ok(InsertOutcome::PendingManualPaste {
                hint: "Ctrl+V".into(),
            }),
        );
        s.apply(Activation::HoldBegan);
        s.apply(Activation::HoldReleased);
        assert_eq!(s.state(), State::Done);
        let events = events.lock().unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Notify { title, body }
                if title == "Clipboard ready" && body.contains("Ctrl+V")
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Clip { outcome, target, .. }
                if outcome == "manual" && target.is_none()
        )));
    }

    #[test]
    fn failed_insert_emits_recovery_notification() {
        let (mut s, events) = harness(
            FakeCapture {
                pcm: vec![0.1; 1600],
                start_ok: true,
                ..Default::default()
            },
            Ok("raw phrase".into()),
            Err(ErrorInfo::new(ErrorKind::Insert, true, "no foreground app")),
        );
        s.apply(Activation::HoldBegan);
        s.apply(Activation::HoldReleased);
        assert_eq!(s.state(), State::Error);
        let events = events.lock().unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Notify { title, body }
                if title == "Insertion failed" && body.contains("Ctrl+Alt+V")
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Error(err) if err.kind == ErrorKind::Insert
        )));
    }
}