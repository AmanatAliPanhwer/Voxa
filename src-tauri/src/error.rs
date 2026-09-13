use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    #[allow(dead_code)]
    Activation,
    Capture,
    Transcribe,
    Cleanup,
    Insert,
    Config,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ErrorInfo {
    pub kind: ErrorKind,
    pub recoverable: bool,
    pub deep_link: Option<String>,
    pub detail: String,
}

impl ErrorInfo {
    pub fn new(kind: ErrorKind, recoverable: bool, detail: impl Into<String>) -> Self {
        Self {
            kind,
            recoverable,
            deep_link: None,
            detail: detail.into(),
        }
    }

    #[allow(dead_code)]
    pub fn with_link(mut self, deep_link: impl Into<String>) -> Self {
        self.deep_link = Some(deep_link.into());
        self
    }
}