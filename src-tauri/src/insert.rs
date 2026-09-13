use crate::error::{ErrorInfo, ErrorKind};
use crate::session::Inserter;
use std::thread;
use std::time::Duration;

pub struct NaiveInserter;

impl Inserter for NaiveInserter {
    fn insert(&self, text: &str) -> Result<Option<String>, ErrorInfo> {
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("clipboard: {e}")))?;
        clipboard
            .set_text(text.to_owned())
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("clipboard: {e}")))?;
        thread::sleep(Duration::from_millis(80));
        paste_chord();
        Ok(None)
    }
}

fn paste_chord() {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = match Enigo::new(&Settings::default()) {
        Ok(e) => e,
        Err(_) => return,
    };
    let _ = enigo.key(Key::Control, Direction::Press);
    let _ = enigo.key(Key::Unicode('v'), Direction::Click);
    let _ = enigo.key(Key::Control, Direction::Release);
}