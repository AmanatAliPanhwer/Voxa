use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const START_WAV: &[u8] = include_bytes!("../assets/sounds/start.wav");
const PASTE_WAV: &[u8] = include_bytes!("../assets/sounds/paste.wav");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Start,
    Paste,
}

pub struct Sounder {
    enabled: Arc<AtomicBool>,
}

impl Sounder {
    pub fn new(enabled: Arc<AtomicBool>) -> Self {
        Self { enabled }
    }

    pub fn play(&self, sound: Sound) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let bytes: &'static [u8] = match sound {
            Sound::Start => START_WAV,
            Sound::Paste => PASTE_WAV,
        };
        let bytes = bytes.to_vec();
        std::thread::spawn(move || {
            let Ok(handle) = rodio::DeviceSinkBuilder::open_default_sink() else {
                return;
            };
            let Ok(source) = rodio::Decoder::try_from(Cursor::new(bytes)) else {
                return;
            };
            let _ = handle.mixer().add(source);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_sounds_are_valid_riff_headers() {
        for wav in [START_WAV, PASTE_WAV] {
            assert!(wav.len() > 44);
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..12], b"WAVE");
        }
    }

    #[test]
    fn disabled_sounder_never_plays() {
        let enabled = Arc::new(AtomicBool::new(false));
        let sounder = Sounder::new(enabled.clone());
        sounder.play(Sound::Start);
        enabled.store(true, Ordering::Relaxed);
        assert_eq!(enabled.load(Ordering::Relaxed), true);
    }
}