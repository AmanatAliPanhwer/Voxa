use crate::error::ErrorInfo;
use crate::session::Transcriber;

pub struct StubTranscriber;

impl Transcriber for StubTranscriber {
    fn transcribe(&self, _pcm: &[f32]) -> Result<String, ErrorInfo> {
        Ok("The quick brown fox jumps over the lazy dog.".into())
    }
}