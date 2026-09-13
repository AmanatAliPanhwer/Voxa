use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct HistoryEntry {
    pub clean_text: String,
    pub target: Option<String>,
    pub timestamp: u64,
}

pub struct Store {
    entries: Vec<HistoryEntry>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, clean_text: String, target: Option<String>) -> HistoryEntry {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let entry = HistoryEntry {
            clean_text,
            target,
            timestamp,
        };
        self.entries.push(entry.clone());
        entry
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn last(&self) -> Option<&HistoryEntry> {
        self.entries.last()
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}