use crate::error::ErrorKind;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const MAX_BYTES: u64 = 1_048_576;

pub struct Log {
    path: PathBuf,
}

impl Log {
    pub fn new(logs_dir: PathBuf) -> Self {
        let path = logs_dir.join("voxa.log");
        let _ = fs::create_dir_all(&logs_dir);
        Self { path }
    }

    pub fn line(&self, level: &str, kind: ErrorKind, detail: &str) {
        let now = chrono_lite();
        let kind = match kind {
            ErrorKind::Activation => "activation",
            ErrorKind::Capture => "capture",
            ErrorKind::Transcribe => "transcribe",
            ErrorKind::Cleanup => "cleanup",
            ErrorKind::Insert => "insert",
            ErrorKind::Config => "config",
        };
        let line = format!("{now} [{level}] {kind} | {detail}\n");
        self.append(&line);
    }

    pub fn raw(&self, line: &str) {
        let now = chrono_lite();
        self.append(&format!("{now} | {line}\n"));
    }

    fn append(&self, line: &str) {
        if self.path.metadata().map(|m| m.len()).unwrap_or(0) >= MAX_BYTES {
            let backup = self.path.with_extension("log.old");
            let _ = fs::copy(&self.path, &backup);
            let _ = fs::remove_file(&self.path);
        }
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = file.write_all(line.as_bytes());
        }
    }

    pub fn tail(&self, max_bytes: usize) -> String {
        let raw = fs::read_to_string(&self.path).unwrap_or_default();
        let bytes = raw.len();
        if bytes <= max_bytes {
            return raw;
        }
        let mut start = bytes - max_bytes;
        while start > 0 && !raw.is_char_boundary(start) {
            start -= 1;
        }
        raw[start..].to_owned()
    }
}

fn chrono_lite() -> String {
    let now = std::time::SystemTime::now();
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_writes_and_tails() {
        let dir = std::env::temp_dir().join(format!("voxa-log-test-{}", std::process::id()));
        let log = Log::new(dir.clone());
        for i in 0..20 {
            log.raw(&format!("line {i}"));
        }
        let tail = log.tail(40);
        assert!(tail.ends_with("line 19\n"));
        assert!(dir.join("voxa.log").is_file());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn long_lines_rotate_backup() {
        let dir = std::env::temp_dir().join(format!("voxa-rotate-{}", std::process::id()));
        let log = Log::new(dir.clone());
        for _ in 0..30 {
            log.raw(&"x".repeat(50_000));
        }
        assert!(dir.join("voxa.log.old").is_file());
        fs::remove_dir_all(&dir).ok();
    }
}