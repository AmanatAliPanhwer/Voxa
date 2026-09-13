use crate::config::InsertOverride;
use crate::error::{ErrorInfo, ErrorKind};
use crate::session::{Inserter, InsertOutcome};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const SETTLE_MS: u64 = 80;
const PASTE_MS: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Auto,
    ClipboardOnly,
}

pub fn current_tier() -> Tier {
    if cfg!(windows) {
        Tier::Auto
    } else if cfg!(target_os = "macos") || is_native_wayland() {
        Tier::ClipboardOnly
    } else {
        Tier::ClipboardOnly
    }
}

fn is_native_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub struct ClipboardInserter {
    overrides: Arc<Mutex<Vec<InsertOverride>>>,
}

impl ClipboardInserter {
    pub fn new(overrides: Arc<Mutex<Vec<InsertOverride>>>) -> Self {
        Self { overrides }
    }

    fn tier(&self) -> Tier {
        if let Ok(guard) = self.overrides.lock() {
            if let Some(desc) = native::window_title_of_frontmost() {
                let lower = desc.to_lowercase();
                for rule in guard.iter() {
                    if lower.contains(&rule.pattern.to_lowercase()) {
                        return if rule.clipboard_only {
                            Tier::ClipboardOnly
                        } else {
                            Tier::Auto
                        };
                    }
                }
            }
        }
        current_tier()
    }
}

impl Inserter for ClipboardInserter {
    fn insert(&self, text: &str) -> Result<InsertOutcome, ErrorInfo> {
        let tier = self.tier();
        match tier {
            Tier::Auto => self.insert_auto(text),
            Tier::ClipboardOnly => self.insert_clipboard_only(text),
        }
    }
}

impl ClipboardInserter {
    #[cfg(windows)]
    fn insert_auto(&self, text: &str) -> Result<InsertOutcome, ErrorInfo> {
        let target = native::foreground_target()?;
        let snapshot = native::snapshot_clipboard()?;
        native::write_clipboard(text)?;
        thread::sleep(Duration::from_millis(SETTLE_MS));
        if let Err(err) = native::fire_paste() {
            native::restore_clipboard(&snapshot);
            return Err(err);
        }
        thread::sleep(Duration::from_millis(PASTE_MS));
        native::restore_clipboard(&snapshot);
        Ok(InsertOutcome::Inserted { target })
    }

    #[cfg(not(windows))]
    fn insert_auto(&self, text: &str) -> Result<InsertOutcome, ErrorInfo> {
        let snapshot = match native::snapshot_text() {
            Some(snapshot) => snapshot,
            None => return self.insert_clipboard_only(text),
        };
        native::write_clipboard(text)?;
        thread::sleep(Duration::from_millis(SETTLE_MS));
        native::fire_paste();
        thread::sleep(Duration::from_millis(PASTE_MS));
        let _ = native::restore_text(snapshot);
        Ok(InsertOutcome::Inserted { target: None })
    }

    fn insert_clipboard_only(&self, text: &str) -> Result<InsertOutcome, ErrorInfo> {
        native::write_clipboard(text)?;
        let hint = if cfg!(target_os = "macos") {
            "Cmd+V".to_owned()
        } else {
            "Ctrl+V".to_owned()
        };
        Ok(InsertOutcome::PendingManualPaste { hint })
    }
}

#[cfg(not(windows))]
mod native {
    use super::*;

    pub fn write_clipboard(text: &str) -> Result<(), ErrorInfo> {
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("clipboard: {e}")))?;
        clipboard
            .set_text(text.to_owned())
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("clipboard: {e}")))
    }

    pub fn window_title_of_frontmost() -> Option<String> {
        None
    }

    pub struct TextSnapshot(String);

    pub fn snapshot_text() -> Option<TextSnapshot> {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        clipboard.get_text().map(TextSnapshot).ok()
    }

    pub fn restore_text(snapshot: TextSnapshot) -> Result<(), ErrorInfo> {
        let mut clipboard = arboard::Clipboard::new()
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("clipboard: {e}")))?;
        clipboard
            .set_text(snapshot.0)
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("clipboard: {e}")))
    }

    pub fn fire_paste() {
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};
        if let Ok(mut enigo) = Enigo::new(&Settings::default()) {
            let _ = enigo.key(Key::Control, Direction::Press);
            let _ = enigo.key(Key::Unicode('v'), Direction::Click);
            let _ = enigo.key(Key::Control, Direction::Release);
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, OpenClipboard,
        SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId,
    };

    pub struct Snapshot {
        formats: Vec<(u32, Vec<u8>)>,
    }

    pub fn foreground_target() -> Result<Option<String>, ErrorInfo> {
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() {
            return Err(ErrorInfo::new(
                ErrorKind::Insert,
                true,
                "no foreground app",
            ));
        }
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
        let own_pid = std::process::id();
        if pid == own_pid {
            return Err(ErrorInfo::new(
                ErrorKind::Insert,
                true,
                "Voxa is frontmost; inserting into itself is impossible",
            ));
        }
        let title = window_title(window);
        Ok(title)
    }

    fn window_title(window: HWND) -> Option<String> {
        let length = unsafe { GetWindowTextLengthW(window) };
        if length <= 0 {
            return None;
        }
        let mut buffer = vec![0u16; (length + 1) as usize];
        unsafe {
            GetWindowTextW(window, &mut buffer);
        }
        buffer.truncate(buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len()));
        Some(String::from_utf16_lossy(&buffer))
    }

    pub fn window_title_of_frontmost() -> Option<String> {
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() {
            return None;
        }
        window_title(window)
    }

    fn window_class(window: HWND) -> Option<String> {
        let mut buffer = [0u16; 128];
        let len = unsafe { GetClassNameW(window, &mut buffer) };
        if len <= 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }

    pub fn snapshot_clipboard() -> Result<Snapshot, ErrorInfo> {
        let _guard = ClipboardHandle::open()?;
        let mut formats = Vec::new();
        let mut format = 0u32;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if let Ok(data) = unsafe { GetClipboardData(format) } {
                let bytes = lock_copy(data);
                if !bytes.is_empty() {
                    formats.push((format, bytes));
                }
            }
        }
        Ok(Snapshot { formats })
    }

    fn lock_copy(data: HANDLE) -> Vec<u8> {
        let hglobal = HGLOBAL(data.0);
        let ptr = unsafe { GlobalLock(hglobal) };
        if ptr.is_null() {
            return Vec::new();
        }
        let size = unsafe { GlobalSize(hglobal) };
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) }.to_vec();
        let _ = unsafe { GlobalUnlock(hglobal) };
        bytes
    }

    pub fn write_clipboard(text: &str) -> Result<(), ErrorInfo> {
        let _guard = ClipboardHandle::open()?;
        unsafe { EmptyClipboard() }
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("empty clipboard: {e}")))?;
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: Vec<u8> = wide
            .iter()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        let handle = global_from_bytes(&bytes)?;
        unsafe { SetClipboardData(13, Some(HANDLE(handle.0))) }
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("set text: {e}")))?;
        Ok(())
    }

    pub fn restore_clipboard(snapshot: &Snapshot) {
        let guard = match ClipboardHandle::open() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if let Err(err) = unsafe { EmptyClipboard() } {
            let _ = err;
            drop(guard);
            return;
        }
        for (format, bytes) in &snapshot.formats {
            let handle = match global_from_bytes(bytes) {
                Ok(handle) => handle,
                Err(_) => continue,
            };
            let _ = unsafe { SetClipboardData(*format, Some(HANDLE(handle.0))) };
        }
        let _ = guard;
    }

    fn global_from_bytes(
        bytes: &[u8],
    ) -> Result<HGLOBAL, ErrorInfo> {
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }.map_err(|e| {
            ErrorInfo::new(ErrorKind::Insert, true, format!("allocate clipboard: {e}"))
        })?;
        let ptr = unsafe { GlobalLock(handle) };
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            let _ = GlobalUnlock(handle);
        }
        Ok(handle)
    }

    pub fn fire_paste() -> Result<(), ErrorInfo> {
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};
        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| ErrorInfo::new(ErrorKind::Insert, true, format!("enigo: {e}")))?;
        if console_window() {
            let _ = enigo.key(Key::Shift, Direction::Press);
            let _ = enigo.key(Key::Insert, Direction::Click);
            let _ = enigo.key(Key::Shift, Direction::Release);
        } else {
            let _ = enigo.key(Key::Control, Direction::Press);
            let _ = enigo.key(Key::Unicode('v'), Direction::Click);
            let _ = enigo.key(Key::Control, Direction::Release);
        }
        Ok(())
    }

    fn console_window() -> bool {
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() {
            return false;
        }
        match window_class(window) {
            Some(class) => {
                class == "ConsoleWindowClass" || class == "CASCADIA_HOSTING_WINDOW_CLASS"
            }
            None => false,
        }
    }

    struct ClipboardHandle(());

    impl ClipboardHandle {
        fn open() -> Result<Self, ErrorInfo> {
            for _ in 0..50 {
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(ClipboardHandle(()));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(ErrorInfo::new(
                ErrorKind::Insert,
                true,
                "clipboard is busy",
            ))
        }
    }

    impl Drop for ClipboardHandle {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }

    #[cfg(test)]
    pub fn probe_read_text() -> Option<String> {
        let _guard = ClipboardHandle::open().ok()?;
        let mut text: Option<String> = None;
        let mut format = 0u32;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if format == 13 {
                if let Ok(data) = unsafe { GetClipboardData(format) } {
                    let bytes = lock_copy(data);
                    text = Some(
                        bytes
                            .chunks_exact(2)
                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                            .take_while(|&c| c != 0)
                            .map(|c| char::from_u32(c as u32).unwrap_or('�'))
                            .collect::<String>(),
                    );
                }
            }
        }
        text
    }

    static CLIPBOARD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn round_trip_preserves_prior_text_clipboard() {
        let _guard = CLIPBOARD_TEST_LOCK.lock().unwrap();
        let original = "prior clipboard payload";
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard.set_text(original.to_owned()).unwrap();
        let snapshot = snapshot_clipboard().unwrap();
        write_clipboard("voxa clean text").unwrap();
        thread::sleep(Duration::from_millis(SETTLE_MS));
        restore_clipboard(&snapshot);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            probe_read_text().as_deref(),
            Some(original),
            "restore must hand the prior clipboard back byte-for-byte"
        );
    }

    #[test]
    fn restore_with_empty_snapshot_clears_clipboard() {
        let _guard = CLIPBOARD_TEST_LOCK.lock().unwrap();
        let snapshot = Snapshot { formats: Vec::new() };
        let mut clipboard = arboard::Clipboard::new().unwrap();
        clipboard.set_text("to be cleared".to_owned()).unwrap();
        restore_clipboard(&snapshot);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(probe_read_text(), None);
    }

    #[test]
    fn write_clipboard_set_unicode_text_visible_to_arboard() {
        let _guard = CLIPBOARD_TEST_LOCK.lock().unwrap();
        write_clipboard("dictated sentence").unwrap();
        thread::sleep(Duration::from_millis(50));
        let mut clipboard = arboard::Clipboard::new().unwrap();
        assert_eq!(clipboard.get_text().unwrap(), "dictated sentence");
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    #[test]
    fn non_windows_tier_is_clipboard_only() {
        use super::{current_tier, Tier};
        assert_eq!(current_tier(), Tier::ClipboardOnly);
    }
}