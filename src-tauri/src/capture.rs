use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rubato::Resampler;

use crate::config::Config;
use crate::error::ErrorInfo;
use crate::error::ErrorKind;
use crate::session::CaptureDevice;

const CLIP_SAMPLE_RATE: usize = 16_000;
const CHUNK_SIZE: usize = 1024;
const RING_CAPACITY: usize = 65_536;
const PREAMBLE_MS: usize = 300;
const PREAMBLE_SAMPLES: usize = CLIP_SAMPLE_RATE * PREAMBLE_MS / 1000;
const SILENT_RUN_MS: usize = 800;
const SILENT_WINDOW_MS: usize = 2_000;
const SILENT_RUN_SAMPLES: usize = CLIP_SAMPLE_RATE * SILENT_RUN_MS / 1000;
const SILENT_WINDOW_SAMPLES: usize = CLIP_SAMPLE_RATE * SILENT_WINDOW_MS / 1000;
const SILENCE_THRESHOLD: f32 = 1e-4;
const LEVEL_WINDOW: usize = 800;
const ATTACK_ALPHA: f32 = 0.998;
const RELEASE_ALPHA: f32 = 0.221;
const GATE_OPEN_DB: f32 = -55.0;
const GATE_CLOSE_DB: f32 = -70.0;
const LEVEL_THROTTLE_S: f64 = 0.1;
const LEVEL_MIN_DELTA: f32 = 0.005;

#[derive(Default)]
struct Shared {
    levels: Vec<f32>,
    error: Option<ErrorInfo>,
}

pub struct CpalCapture {
    mic_device: Option<String>,
    stream: Option<cpal::Stream>,
    worker: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Mutex<Shared>>,
    pcm: Arc<Mutex<Vec<f32>>>,
    clip_on: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl CpalCapture {
    pub fn new(cfg: &Config) -> Self {
        Self {
            mic_device: cfg.mic_device.clone(),
            stream: None,
            worker: None,
            shared: Arc::new(Mutex::new(Shared::default())),
            pcm: Arc::new(Mutex::new(Vec::new())),
            clip_on: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    fn open(&mut self) -> Result<(), ErrorInfo> {
        let host = cpal::default_host();
        let device = match &self.mic_device {
            Some(name) => host
                .input_devices()
                .map_err(|e| capture_error(&e))?
                .find(|device| {
                    device
                        .description()
                        .map(|d| d.name() == name.as_str())
                        .unwrap_or(false)
                })
                .ok_or_else(|| {
                    ErrorInfo::new(ErrorKind::Capture, true, "microphone device not found")
                })?,
            None => host.default_input_device().ok_or_else(|| {
                ErrorInfo::new(ErrorKind::Capture, true, "no default microphone found")
            })?,
        };
        let supported = device.default_input_config().map_err(|e| capture_error(&e))?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.config();

        let ratio = CLIP_SAMPLE_RATE as f64 / config.sample_rate as f64;
        let resampler = rubato::SincFixedIn::<f32>::new(ratio, 1.0, sinc_params(), CHUNK_SIZE, 1)
            .map_err(|e| {
                ErrorInfo::new(ErrorKind::Capture, true, format!("resampler init failed: {e}"))
            })?;
        let out = resampler.output_buffer_allocate(true);
        let chunk_size = resampler.input_frames_next();
        let delay = resampler.output_delay();

        let shared = Arc::new(Mutex::new(Shared::default()));
        let pcm = Arc::new(Mutex::new(Vec::<f32>::new()));
        let clip_on = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);

        let stream = match format {
            SampleFormat::I8 => build_with::<i8>(&device, &config, producer, shared.clone())?,
            SampleFormat::I16 => build_with::<i16>(&device, &config, producer, shared.clone())?,
            SampleFormat::I24 => build_with::<cpal::I24>(&device, &config, producer, shared.clone())?,
            SampleFormat::I32 => build_with::<i32>(&device, &config, producer, shared.clone())?,
            SampleFormat::I64 => build_with::<i64>(&device, &config, producer, shared.clone())?,
            SampleFormat::U8 => build_with::<u8>(&device, &config, producer, shared.clone())?,
            SampleFormat::U16 => build_with::<u16>(&device, &config, producer, shared.clone())?,
            SampleFormat::U24 => build_with::<cpal::U24>(&device, &config, producer, shared.clone())?,
            SampleFormat::U32 => build_with::<u32>(&device, &config, producer, shared.clone())?,
            SampleFormat::U64 => build_with::<u64>(&device, &config, producer, shared.clone())?,
            SampleFormat::F32 => build_with::<f32>(&device, &config, producer, shared.clone())?,
            SampleFormat::F64 => build_with::<f64>(&device, &config, producer, shared.clone())?,
            other => {
                return Err(ErrorInfo::new(
                    ErrorKind::Capture,
                    true,
                    format!("unsupported microphone sample format: {other:?}"),
                ))
            }
        };

        let shared_worker = shared.clone();
        let pcm_worker = pcm.clone();
        let clip_on_worker = clip_on.clone();
        let stop_worker = stop.clone();
        let worker = std::thread::Builder::new()
            .name("voxa-capture".into())
            .spawn(move || {
                Worker {
                    consumer,
                    resampler,
                    out,
                    shared: shared_worker,
                    pcm: pcm_worker,
                    clip_on: clip_on_worker,
                    stop: stop_worker,
                    chunk_size,
                    delay,
                    input_buf: Vec::with_capacity(CHUNK_SIZE),
                    preamble: Preamble::new(PREAMBLE_SAMPLES),
                    meter: Meter::new(),
                    was_clip_on: false,
                    clip_frames: 0,
                    silent_frames: 0,
                    frames_at: 0,
                }
                .run();
            })
            .map_err(|e| {
                ErrorInfo::new(ErrorKind::Capture, true, format!("capture thread spawn failed: {e}"))
            })?;

        self.stream = Some(stream);
        self.worker = Some(worker);
        self.shared = shared;
        self.pcm = pcm;
        self.clip_on = clip_on;
        self.stop = stop;
        Ok(())
    }

    fn stop_capture(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.stream = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl CaptureDevice for CpalCapture {
    fn start(&mut self) -> Result<(), ErrorInfo> {
        if self.stream.is_some() {
            return Ok(());
        }
        self.open()
    }

    fn promote(&mut self) -> Result<(), ErrorInfo> {
        if let Some(error) = self.take_error() {
            return Err(error);
        }
        self.start()?;
        self.clip_on.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn stop(&mut self) -> Vec<f32> {
        self.stop_capture();
        self.clip_on.store(false, Ordering::SeqCst);
        let mut clip = Vec::new();
        if let Ok(mut p) = self.pcm.lock() {
            std::mem::swap(&mut clip, &mut p);
        }
        clip
    }

    fn cancel(&mut self) {
        self.stop_capture();
        self.clip_on.store(false, Ordering::SeqCst);
        if let Ok(mut p) = self.pcm.lock() {
            p.clear();
        }
        if let Ok(mut shared) = self.shared.lock() {
            shared.levels.clear();
            shared.error = None;
        }
    }

    fn levels(&mut self) -> Vec<f32> {
        match self.shared.lock() {
            Ok(mut shared) => std::mem::take(&mut shared.levels),
            Err(_) => Vec::new(),
        }
    }

    fn take_error(&mut self) -> Option<ErrorInfo> {
        match self.shared.lock() {
            Ok(mut shared) => shared.error.take(),
            Err(_) => None,
        }
    }
}

fn capture_error(e: &cpal::Error) -> ErrorInfo {
    ErrorInfo::new(ErrorKind::Capture, true, e.to_string())
}

fn build_with<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut producer: rtrb::Producer<f32>,
    shared: Arc<Mutex<Shared>>,
) -> Result<cpal::Stream, ErrorInfo>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let data_cb = move |data: &[T], _info: &cpal::InputCallbackInfo| {
        if channels == 1 {
            for sample in data {
                let _ = producer.push(sample.to_sample::<f32>());
            }
        } else {
            let mut frame = Vec::with_capacity(channels);
            for sample in data {
                frame.push(sample.to_sample::<f32>());
                if frame.len() == channels {
                    let mono: f32 = frame.iter().sum::<f32>() / frame.len() as f32;
                    let _ = producer.push(mono);
                    frame.clear();
                }
            }
        }
    };
    let error_cb = move |e: cpal::Error| {
        if let Ok(mut guard) = shared.lock() {
            if guard.error.is_none() {
                guard.error = Some(ErrorInfo::new(
                    ErrorKind::Capture,
                    true,
                    format!("microphone stream error: {e}"),
                ));
            }
        }
    };
    let stream = device
        .build_input_stream::<T, _, _>(config.clone(), data_cb, error_cb, None)
        .map_err(|e| capture_error(&e))?;
    stream.play().map_err(|e| capture_error(&e))?;
    Ok(stream)
}

fn sinc_params() -> rubato::SincInterpolationParameters {
    rubato::SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: rubato::SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: rubato::WindowFunction::BlackmanHarris2,
    }
}

struct Preamble {
    buf: VecDeque<f32>,
    cap: usize,
}

impl Preamble {
    fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap),
            cap,
        }
    }

    fn push(&mut self, sample: f32) {
        if self.buf.len() >= self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(sample);
    }

    fn drain(&mut self) -> Vec<f32> {
        self.buf.drain(..).collect()
    }
}

struct Meter {
    window: usize,
    sq_sum: f32,
    count: usize,
    smooth_db: f32,
    gated: bool,
    emit_val: f32,
    emit_at: f64,
}

impl Meter {
    fn new() -> Self {
        Self {
            window: LEVEL_WINDOW,
            sq_sum: 0.0,
            count: 0,
            smooth_db: GATE_CLOSE_DB,
            gated: false,
            emit_val: 0.0,
            emit_at: f64::MIN,
        }
    }

    fn reset(&mut self) {
        *self = Meter::new();
    }

    fn feed(&mut self, sample: f32, frames_at: usize, out_rate: usize, out: &mut Vec<f32>) {
        self.sq_sum += sample * sample;
        self.count += 1;
        if self.count < self.window {
            return;
        }
        let rms = (self.sq_sum / self.window as f32).sqrt();
        self.sq_sum = 0.0;
        self.count = 0;
        let db = 20.0 * (rms + 1e-9).log10();
        let alpha = if db >= self.smooth_db {
            ATTACK_ALPHA
        } else {
            RELEASE_ALPHA
        };
        self.smooth_db += alpha * (db - self.smooth_db);
        if !self.gated && self.smooth_db > GATE_OPEN_DB {
            self.gated = true;
        }
        if self.gated && self.smooth_db < GATE_CLOSE_DB {
            self.gated = false;
        }
        let value = if self.gated {
            ((self.smooth_db + 60.0) / 60.0).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let now = frames_at as f64 / out_rate as f64;
        let changed = (value - self.emit_val).abs() >= LEVEL_MIN_DELTA
            || (value == 0.0) != (self.emit_val == 0.0);
        if changed && now - self.emit_at >= LEVEL_THROTTLE_S {
            out.push(value);
            self.emit_val = value;
            self.emit_at = now;
        }
    }
}

struct Worker {
    consumer: rtrb::Consumer<f32>,
    resampler: rubato::SincFixedIn<f32>,
    out: Vec<Vec<f32>>,
    shared: Arc<Mutex<Shared>>,
    pcm: Arc<Mutex<Vec<f32>>>,
    clip_on: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    chunk_size: usize,
    delay: usize,
    input_buf: Vec<f32>,
    preamble: Preamble,
    meter: Meter,
    was_clip_on: bool,
    clip_frames: usize,
    silent_frames: usize,
    frames_at: usize,
}

impl Worker {
    fn run(mut self) {
        loop {
            let mut progressed = false;
            while let Ok(sample) = self.consumer.pop() {
                self.input_buf.push(sample);
                if self.input_buf.len() >= self.chunk_size {
                    let buf = std::mem::take(&mut self.input_buf);
                    let (_, n) = self
                        .resampler
                        .process_into_buffer(&[buf.as_slice()], &mut self.out, None)
                        .expect("rubato resampling");
                    self.feed(false, &self.out[0][..n].to_vec());
                    progressed = true;
                }
            }
            if progressed {
                continue;
            }
            if self.stop.load(Ordering::SeqCst) {
                self.finish();
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn feed(&mut self, force_clip: bool, samples: &[f32]) {
        let in_clip = force_clip || self.clip_on.load(Ordering::SeqCst);
        if in_clip && !self.was_clip_on {
            if let Ok(mut p) = self.pcm.lock() {
                p.extend(self.preamble.drain());
            }
            self.meter.reset();
            self.clip_frames = 0;
            self.silent_frames = 0;
            self.was_clip_on = true;
        }
        let mut levels = Vec::new();
        for &sample in samples {
            if in_clip {
                if let Ok(mut p) = self.pcm.lock() {
                    p.push(sample);
                }
                self.meter.feed(sample, self.frames_at, CLIP_SAMPLE_RATE, &mut levels);
                self.frames_at += 1;
                self.clip_frames += 1;
                if sample.abs() < SILENCE_THRESHOLD {
                    self.silent_frames += 1;
                    if self.clip_frames < SILENT_WINDOW_SAMPLES
                        && self.silent_frames >= SILENT_RUN_SAMPLES
                    {
                        self.flag(ErrorInfo::new(
                            ErrorKind::Capture,
                            true,
                            "no signal detected from the microphone",
                        ));
                        self.silent_frames = 0;
                    }
                } else {
                    self.silent_frames = 0;
                }
            } else {
                self.preamble.push(sample);
            }
        }
        if !in_clip {
            self.was_clip_on = false;
        }
        if !levels.is_empty() {
            if let Ok(mut shared) = self.shared.lock() {
                shared.levels.extend(levels);
            }
        }
    }

    fn flag(&self, error: ErrorInfo) {
        if let Ok(mut shared) = self.shared.lock() {
            if shared.error.is_none() {
                shared.error = Some(error);
            }
        }
    }

    fn finish(&mut self) {
        if !self.input_buf.is_empty() {
            let (_, n) = self
                .resampler
                .process_partial_into_buffer(Some(&[self.input_buf.as_slice()]), &mut self.out, None)
                .expect("rubato resampling");
            self.feed(true, &self.out[0][..n].to_vec());
        }
        let (_, n) = self
            .resampler
            .process_partial_into_buffer(None::<&[Vec<f32>]>, &mut self.out, None)
            .expect("rubato resampling");
        self.feed(true, &self.out[0][..n].to_vec());
        if let Ok(mut p) = self.pcm.lock() {
            if p.len() >= self.delay {
                p.drain(0..self.delay);
            } else {
                p.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resampler() -> rubato::SincFixedIn<f32> {
        rubato::SincFixedIn::<f32>::new(
            1.0 / 3.0,
            1.0,
            sinc_params(),
            CHUNK_SIZE,
            1,
        )
        .expect("resampler")
    }

    fn run_meter(feeds: &[&[f32]]) -> Vec<f32> {
        let mut meter = Meter::new();
        let mut out = Vec::new();
        let mut frames = 0;
        for feed in feeds {
            for &s in *feed {
                meter.feed(s, frames, CLIP_SAMPLE_RATE, &mut out);
                frames += 1;
            }
        }
        out
    }

    #[test]
    fn silence_keeps_meter_closed_and_quiet() {
        let out = run_meter(&[&[0.0; LEVEL_WINDOW * 4]]);
        assert!(out.is_empty());
    }

    #[test]
    fn loud_signal_opens_gate_and_tracks_level() {
        let signal: Vec<f32> = std::iter::repeat(0.5).take(LEVEL_WINDOW * 6).collect();
        let out = run_meter(&[&signal]);
        assert!(!out.is_empty());
        let last = *out.last().unwrap();
        assert!(last > 0.8, "expected a high level, got {last}");
    }

    #[test]
    fn gate_snaps_to_zero_when_signal_drops() {
        let loud: Vec<f32> = std::iter::repeat(0.5).take(LEVEL_WINDOW * 6).collect();
        let quiet: Vec<f32> = std::iter::repeat(0.0).take(LEVEL_WINDOW * 10).collect();
        let out = run_meter(&[&loud, &quiet]);
        assert!(!out.is_empty());
        assert_eq!(*out.last().unwrap(), 0.0);
    }

    #[test]
    fn meter_emits_at_most_ten_per_second() {
        let secs = 10;
        let signal: Vec<f32> = std::iter::repeat(0.5)
            .take(CLIP_SAMPLE_RATE * secs)
            .collect();
        let chunks: Vec<&[f32]> = signal.chunks(LEVEL_WINDOW * 2).collect();
        let out = run_meter(&chunks);
        assert!(out.len() <= secs * 10 + 1);
        assert!(!out.is_empty());
    }

    #[test]
    fn preamble_keeps_only_the_last_cap_samples() {
        let mut pream = Preamble::new(100);
        for i in 0..1000 {
            pream.push(i as f32);
        }
        let drained = pream.drain();
        assert_eq!(drained.len(), 100);
        assert_eq!(*drained.first().unwrap(), 900.0);
        assert_eq!(*drained.last().unwrap(), 999.0);
    }

    #[test]
    fn worker_resamples_and_finishes_with_trimmed_delay() {
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);
        drop(producer);
        let mut resampler = resampler();
        let out = resampler.output_buffer_allocate(true);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let pcm = Arc::new(Mutex::new(Vec::new()));
        let clip_on = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let mut worker = Worker {
            consumer,
            resampler,
            out,
            shared,
            pcm,
            clip_on: clip_on.clone(),
            stop,
            chunk_size: CHUNK_SIZE,
            delay: 42,
            input_buf: Vec::new(),
            preamble: Preamble::new(PREAMBLE_SAMPLES),
            meter: Meter::new(),
            was_clip_on: false,
            clip_frames: 0,
            silent_frames: 0,
            frames_at: 0,
        };
        clip_on.store(true, Ordering::SeqCst);
        let block: Vec<f32> = (0..CHUNK_SIZE)
            .map(|i| (i as f32 * 0.05).sin() * 0.4)
            .collect();
        for _ in 0..3 {
            let (_, n) = worker
                .resampler
                .process_into_buffer(&[block.as_slice()], &mut worker.out, None)
                .expect("rubato resampling");
            worker.feed(false, &worker.out[0][..n].to_vec());
        }
        worker.finish();
        let pcm = worker.pcm.lock().unwrap();
        assert!(pcm.len() >= 900, "expected a full clip, got {}", pcm.len());
        assert!(pcm.len() <= 4200, "expected a short clip, got {}", pcm.len());
        assert!(pcm.iter().any(|s| s.abs() > 0.1));
    }

    #[test]
    fn worker_without_clip_yields_no_audio() {
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);
        drop(producer);
        let mut resampler = resampler();
        let out = resampler.output_buffer_allocate(true);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let pcm = Arc::new(Mutex::new(Vec::new()));
        let clip_on = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let mut worker = Worker {
            consumer,
            resampler,
            out,
            shared,
            pcm: pcm.clone(),
            clip_on,
            stop,
            chunk_size: CHUNK_SIZE,
            delay: 42,
            input_buf: Vec::new(),
            preamble: Preamble::new(PREAMBLE_SAMPLES),
            meter: Meter::new(),
            was_clip_on: false,
            clip_frames: 0,
            silent_frames: 0,
            frames_at: 0,
        };
        let block: Vec<f32> = vec![0.0; CHUNK_SIZE];
        worker.feed(false, &block);
        worker.preamble.drain();
        worker.finish();
        let pcm = pcm.lock().unwrap();
        assert!(pcm.iter().all(|s| s.abs() < 1e-6));
        assert!(pcm.len() < CHUNK_SIZE);
    }
}