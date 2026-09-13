use crate::error::{ErrorInfo, ErrorKind};
use crate::model;
use crate::session::{Broadcast, Event, Transcriber};
use std::path::PathBuf;
use std::sync::mpsc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

struct Job {
    pcm: Vec<f32>,
    reply: mpsc::Sender<Result<String, ErrorInfo>>,
}

pub struct WhisperTranscriber {
    jobs: mpsc::Sender<Job>,
}

impl WhisperTranscriber {
    pub fn new(
        models_dir: PathBuf,
        model_id: String,
        compute_device: String,
        broadcast: Box<dyn Broadcast>,
    ) -> Self {
        let (jobs_tx, jobs_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("voxa-whisper".into())
            .spawn(move || worker(models_dir, model_id, compute_device, broadcast, jobs_rx))
            .expect("whisper worker thread must spawn");
        Self { jobs: jobs_tx }
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, pcm: &[f32]) -> Result<String, ErrorInfo> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.jobs
            .send(Job {
                pcm: pcm.to_vec(),
                reply: reply_tx,
            })
            .map_err(|_| transcribe_err("whisper worker is gone"))?;
        reply_rx
            .recv()
            .map_err(|_| transcribe_err("whisper worker stopped"))
            .and_then(|result| result)
    }
}

fn worker(
    models_dir: PathBuf,
    model_id: String,
    compute_device: String,
    broadcast: Box<dyn Broadcast>,
    jobs_rx: mpsc::Receiver<Job>,
) {
    whisper_rs::install_logging_hooks();
    let mut context: Option<WhisperContext> = None;
    while let Ok(job) = jobs_rx.recv() {
        let result = full_transcribe(
            &mut context,
            &models_dir,
            &model_id,
            &compute_device,
            broadcast.as_ref(),
            &job.pcm,
        );
        let _ = job.reply.send(result);
    }
}

fn full_transcribe(
    context: &mut Option<WhisperContext>,
    models_dir: &PathBuf,
    model_id: &str,
    compute_device: &str,
    broadcast: &dyn Broadcast,
    pcm: &[f32],
) -> Result<String, ErrorInfo> {
    if context.is_none() {
        *context = Some(load_context(models_dir, model_id, compute_device, broadcast)?);
    }
    let ctx = context.as_ref().unwrap();
    let mut state = ctx
        .create_state()
        .map_err(|err| transcribe_err(format!("whisper state failed: {err}")))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_n_threads(cpu_threads());
    params.set_language(Some("en"));
    state
        .full(params, pcm)
        .map_err(|err| transcribe_err(format!("whisper inference failed: {err}")))?;
    let text = state
        .as_iter()
        .map(|segment| segment.to_string())
        .collect::<String>();
    Ok(text.trim().to_string())
}

fn load_context(
    models_dir: &PathBuf,
    model_id: &str,
    compute_device: &str,
    broadcast: &dyn Broadcast,
) -> Result<WhisperContext, ErrorInfo> {
    let spec = model::find(model_id).ok_or_else(|| transcribe_err(format!("unknown model: {model_id}")))?;
    let model_path = model::ensure(models_dir, spec, |percent| {
        broadcast.emit(Event::Progress {
            phase: "model".into(),
            percent,
        });
    })?;
    let mut params = WhisperContextParameters::new();
    params.use_gpu(use_gpu(compute_device));
    WhisperContext::new_with_params(&model_path, params)
        .map_err(|err| transcribe_err(format!("failed to load model {model_id}: {err}")))
}

fn use_gpu(compute_device: &str) -> bool {
    compute_device == "gpu"
}

fn cpu_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|count| count.get() as i32)
        .unwrap_or(4)
}

fn transcribe_err(detail: impl Into<String>) -> ErrorInfo {
    ErrorInfo::new(ErrorKind::Transcribe, true, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn use_gpu_resolves_compute_devices_for_cpu_builds() {
        assert!(!use_gpu("cpu"));
        assert!(!use_gpu("auto"));
        assert!(use_gpu("gpu"));
        assert!(!use_gpu("anything else"));
    }

    #[derive(Default)]
    struct CollectBroadcast {
        events: Arc<Mutex<Vec<Event>>>,
    }

    impl Broadcast for CollectBroadcast {
        fn emit(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[test]
    #[ignore]
    fn real_small_en_transcribes_jfk_and_reuses_warm_context() {
        let models_dir =
            std::path::PathBuf::from(std::env::var("VOXA_MODEL_DIR").expect("VOXA_MODEL_DIR"));
        let wav_path =
            std::path::PathBuf::from(std::env::var("VOXA_SAMPLE_WAV").expect("VOXA_SAMPLE_WAV"));
        let events = Arc::new(Mutex::new(Vec::new()));
        let transcriber = WhisperTranscriber::new(
            models_dir,
            "small.en".into(),
            "cpu".into(),
            Box::new(CollectBroadcast {
                events: events.clone(),
            }),
        );
        let pcm = load_wav(&wav_path);
        assert!(!pcm.is_empty());
        let first = transcriber.transcribe(&pcm).unwrap();
        assert!(first.to_lowercase().contains("country"), "got: {first}");
        let second = transcriber.transcribe(&pcm).unwrap();
        assert!(second.to_lowercase().contains("country"), "got: {second}");
        let progress = events.lock().unwrap().iter().any(|event| {
            matches!(event, Event::Progress { phase, .. } if phase == "model")
        });
        assert!(progress, "model progress events must be emitted");
    }

    fn load_wav(path: &std::path::Path) -> Vec<f32> {
        let bytes = std::fs::read(path).expect("read wav");
        let mut pos = 12;
        let mut channels = 1u16;
        let mut sample_rate = 16000u32;
        let mut bits = 16u16;
        let mut data: Option<&[u8]> = None;
        while pos + 8 <= bytes.len() {
            let chunk = &bytes[pos..pos + 4];
            let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            pos += 8;
            match chunk {
                b"fmt " => {
                    channels = u16::from_le_bytes(bytes[pos + 2..pos + 4].try_into().unwrap());
                    sample_rate = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap());
                    bits = u16::from_le_bytes(bytes[pos + 14..pos + 16].try_into().unwrap());
                    pos += size;
                }
                b"data" => {
                    data = Some(&bytes[pos..pos + size]);
                    pos += size;
                }
                _ => pos += size,
            }
            if chunk == b"data" {
                break;
            }
        }
        let data = data.expect("no data chunk");
        let samples: Vec<f32> = match bits {
            16 => data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect(),
            _ => panic!("unsupported bits {bits}"),
        };
        if channels == 1 && sample_rate == 16000 {
            return samples;
        }
        let mono: Vec<f32> = (0..(samples.len() / channels as usize))
            .map(|i| {
                let mut sum = 0.0;
                for ch in 0..channels as usize {
                    sum += samples[i * channels as usize + ch];
                }
                sum / channels as f32
            })
            .collect();
        if sample_rate == 16000 {
            return mono;
        }
        linear_resample(&mono, sample_rate, 16000)
    }

    fn linear_resample(data: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        if data.is_empty() {
            return Vec::new();
        }
        let out_len = (data.len() as f64 * to_rate as f64 / from_rate as f64) as usize;
        (0..out_len)
            .map(|i| {
                let src = i as f64 * from_rate as f64 / to_rate as f64;
                let idx = src.floor() as usize;
                let frac = (src - src.floor()) as f32;
                let a = data[idx.min(data.len() - 1)];
                let b = data[(idx + 1).min(data.len() - 1)];
                a + (b - a) * frac
            })
            .collect()
    }
}