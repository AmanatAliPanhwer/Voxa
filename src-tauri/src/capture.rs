use crate::error::ErrorInfo;
use crate::session::CaptureDevice;

pub struct StubCapture {
    levels_out: Vec<f32>,
    pcm: Vec<f32>,
    t: usize,
}

impl Default for StubCapture {
    fn default() -> Self {
        Self {
            levels_out: Vec::new(),
            pcm: Vec::new(),
            t: 0,
        }
    }
}

impl CaptureDevice for StubCapture {
    fn start(&mut self) -> Result<(), ErrorInfo> {
        self.levels_out.clear();
        self.pcm.clear();
        self.t = 0;
        Ok(())
    }

    fn stop(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.pcm)
    }

    fn levels(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..2 {
            self.t += 1;
            let level = 0.5 + 0.5 * (self.t as f32 * 0.7).sin();
            out.push(level.abs());
            for _ in 0..160 {
                self.pcm.push((self.t as f32 * 0.02).sin() * 0.05);
            }
        }
        out
    }
}