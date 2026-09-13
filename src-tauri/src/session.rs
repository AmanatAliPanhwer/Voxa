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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inbound {
    Activation(Activation),
    InsertLast,
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
}

pub trait CaptureDevice: Send {
    fn start(&mut self) -> Result<(), ErrorInfo>;
    fn stop(&mut self) -> Vec<f32>;
    fn levels(&mut self) -> Vec<f32>;
}

pub trait Transcriber: Send {
    fn transcribe(&self, pcm: &[f32]) -> Result<String, ErrorInfo>;
}

pub trait Cleaner: Send {
    fn clean(&self, raw: &str) -> String;
}

pub trait Inserter: Send {
    fn insert(&self, text: &str) -> Result<Option<String>, ErrorInfo>;
}

pub struct Session {
    state: State,
    broadcasts: Box<dyn Broadcast>,
    capture: Box<dyn CaptureDevice>,
    transcriber: Box<dyn Transcriber>,
    cleaner: Box<dyn Cleaner>,
    inserter: Box<dyn Inserter>,
    store: Store,
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

    pub fn insert_last_result(&mut self) {
        let last = match self.store.last() {
            Some(entry) => entry.clone(),
            None => return,
        };
        self.to(State::Inserting);
        match self.inserter.insert(&last.clean_text) {
            Ok(target) => {
                self.broadcasts.emit(Event::Clip {
                    outcome: "inserted".into(),
                    target: target.clone(),
                    timestamp: last.timestamp,
                });
                self.to(State::Done);
            }
            Err(err) => self.fail(err),
        }
    }

    pub fn drain_levels(&mut self) {
        if self.state != State::Listening {
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
        match self.capture.start() {
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
            Ok(target) => {
                let entry = self.store.push(clean, target.clone());
                self.broadcasts.emit(Event::Clip {
                    outcome: "inserted".into(),
                    target,
                    timestamp: entry.timestamp,
                });
                self.to(State::Done);
            }
            Err(err) => {
                self.store.push(clean, None);
                self.fail(err);
            }
        }
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

    #[derive(Default)]
    struct FakeCapture {
        pcm: Vec<f32>,
        start_ok: bool,
        levels_in: Vec<f32>,
        levels_out: Vec<f32>,
    }

    impl CaptureDevice for FakeCapture {
        fn start(&mut self) -> Result<(), ErrorInfo> {
            if self.start_ok {
                Ok(())
            } else {
                Err(ErrorInfo::new(ErrorKind::Capture, true, "no mic"))
            }
        }
        fn stop(&mut self) -> Vec<f32> {
            std::mem::take(&mut self.pcm)
        }
        fn levels(&mut self) -> Vec<f32> {
            self.levels_out.drain(..).collect()
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
        result: Result<Option<String>, ErrorInfo>,
    }

    impl Inserter for FakeInserter {
        fn insert(&self, _text: &str) -> Result<Option<String>, ErrorInfo> {
            self.result.clone()
        }
    }

    fn harness(
        capture: FakeCapture,
        transcribe: Result<String, ErrorInfo>,
        insert: Result<Option<String>, ErrorInfo>,
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
            Ok(None),
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
            Ok(None),
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
            Ok(None),
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
            Ok(Some("Notes".into())),
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
            Ok(None),
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
            Ok(None),
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
            result: Ok(Some("Notes".into())),
        });
        s.insert_last_result();
        assert_eq!(s.state(), State::Done);
    }

    #[test]
    fn recovery_with_empty_history_is_a_noop() {
        let (mut s, _events) = harness(
            FakeCapture::default(),
            Ok("raw".into()),
            Ok(None),
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
            Ok(None),
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
    fn levels_gate_emits_zero_once_then_stays_silent() {
        let mut capture = FakeCapture {
            start_ok: true,
            ..Default::default()
        };
        capture.levels_out = vec![0.4, 0.0, 0.0, 0.0];
        let (mut s, events) = harness(capture, Ok("raw".into()), Ok(None));
        s.apply(Activation::HoldBegan);
        s.drain_levels();
        let emitted: Vec<f32> = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                Event::Levels(l) => Some(*l),
                _ => None,
            })
            .collect();
        assert_eq!(emitted, vec![0.4, 0.0]);
        s.drain_levels();
        let after = events.lock().unwrap();
        assert_eq!(
            after.iter().filter(|e| matches!(e, Event::Levels(_))).count(),
            2
        );
    }
}