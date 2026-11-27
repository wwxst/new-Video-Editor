//! CPAL output: device enumeration, the two callback designs, and what they cost.
//!
//! `direct` mixes inside the device callback (Concat's design, but from PCM instead of FFmpeg);
//! `ring` renders on a producer thread and hands finished blocks over an SPSC ring (the design
//! Arcline's docs propose). Both are measured with the same mixer and source.

use arc_audio::Mixer;
use serde_json::json;
use cpal::traits::DeviceTrait;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Lock-free, allocation-free callback recorder: one slot per callback, written as an atomic.
pub struct Recorder {
    durations: Vec<AtomicU64>,
    index: AtomicUsize,
    pub callbacks: AtomicU64,
    pub underruns: AtomicU64,
    pub frames: AtomicU64,
}

impl Recorder {
    pub fn new(capacity: usize) -> Recorder {
        Recorder {
            durations: (0..capacity).map(|_| AtomicU64::new(0)).collect(),
            index: AtomicUsize::new(0),
            callbacks: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            frames: AtomicU64::new(0),
        }
    }
    pub fn record(&self, us: u64) {
        let i = self.index.fetch_add(1, Ordering::Relaxed);
        if let Some(slot) = self.durations.get(i % self.durations.len()) {
            slot.store(us, Ordering::Relaxed);
        }
        self.callbacks.fetch_add(1, Ordering::Relaxed);
    }
    pub fn summary(&self) -> serde_json::Value {
        let n = self.index.load(Ordering::Relaxed).min(self.durations.len());
        let mut v: Vec<u64> = self.durations[..n]
            .iter()
            .map(|d| d.load(Ordering::Relaxed))
            .filter(|d| *d > 0)
            .collect();
        v.sort_unstable();
        let pct = |p: f64| -> u64 {
            if v.is_empty() {
                0
            } else {
                v[((v.len() - 1) as f64 * p).round() as usize]
            }
        };
        json!({
            "callbacks": self.callbacks.load(Ordering::Relaxed),
            "frames": self.frames.load(Ordering::Relaxed),
            "underruns": self.underruns.load(Ordering::Relaxed),
            "cb_us_p50": pct(0.5),
            "cb_us_p95": pct(0.95),
            "cb_us_max": v.last().copied().unwrap_or(0),
            "cb_us_mean": if v.is_empty() { 0.0 } else { v.iter().sum::<u64>() as f64 / v.len() as f64 },
        })
    }
}

pub fn devices() -> Result<serde_json::Value, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let mut list = Vec::new();
    for device in host.output_devices().map_err(|e| e.to_string())? {
        let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "?".into());
        let default = device.default_output_config().ok();
        let configs = device
            .supported_output_configs()
            .map(|it| {
                it.map(|c| {
                    json!({
                        "channels": c.channels(),
                        "min_rate": c.min_sample_rate(),
                        "max_rate": c.max_sample_rate(),
                        "format": format!("{:?}", c.sample_format()),
                    })
                })
                .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        list.push(json!({
            "name": name,
            "default": default.as_ref().map(|c| json!({
                "rate": c.sample_rate(),
                "channels": c.channels(),
                "format": format!("{:?}", c.sample_format()),
            })),
            "configs": configs,
        }));
    }
    Ok(json!({
        "host": format!("{:?}", cpal::default_host().id()),
        "default_input": host.default_input_device().and_then(|d| d.description().ok()).map(|d| d.name().to_string()),
        "outputs": list,
    }))
}

/// Play `seconds` of a mix through the default output device.
///
/// `ring` == false mixes in the callback; `ring` == true renders on a producer thread into an
/// SPSC ring. The source is a cached PCM span, so neither design touches a file from the callback.
pub fn play(
    mixer: Mixer,
    source: Box<dyn arc_audio::SampleSource + Send>,
    seconds: f64,
    ring: bool,
) -> Result<serde_json::Value, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;
    let channels = config.channels as usize;
    let device_name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "?".into());

    let rec = Arc::new(Recorder::new(200_000));
    let stop = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let total_frames = (rate as f64 * seconds) as u64;

    let stream = if ring {
        build_ring(&device, &config, sample_format, mixer, source, rate, channels, Arc::clone(&rec), Arc::clone(&stop), total_frames)?
    } else {
        build_direct(&device, &config, sample_format, mixer, source, rate, channels, Arc::clone(&rec), Arc::clone(&stop), total_frames)?
    };
    stream.play().map_err(|e| e.to_string())?;
    while started.elapsed() < Duration::from_secs_f64(seconds) && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(120));
    drop(stream);
    let elapsed = started.elapsed().as_secs_f64();
    let frames = rec.frames.load(Ordering::Relaxed);
    let mut extra = rec.summary();
    extra["device"] = json!(device_name);
    extra["rate"] = json!(rate);
    extra["channels"] = json!(channels);
    extra["format"] = json!(format!("{sample_format:?}"));
    extra["mode"] = json!(if ring { "ring" } else { "direct" });
    extra["elapsed_s"] = json!(elapsed);
    extra["expected_frames"] = json!((rate as f64 * elapsed) as u64);
    extra["realtime_ratio"] = json!(frames as f64 / (rate as f64 * elapsed).max(1.0));
    Ok(extra)
}

fn build_direct(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    mut mixer: Mixer,
    mut source: Box<dyn arc_audio::SampleSource + Send>,
    rate: u32,
    channels: usize,
    rec: Arc<Recorder>,
    stop: Arc<AtomicBool>,
    total_frames: u64,
) -> Result<cpal::Stream, String> {
    let err_fn = |e| eprintln!("audio-probe: stream error: {e}");
    let mut pos: i64 = 0;
    let mut scratch: Vec<f32> = Vec::new();
    match format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                config.clone(),
                move |data: &mut [f32], _| {
                    let t0 = Instant::now();
                    let frames = data.len() / channels;
                    if pos as u64 >= total_frames {
                        data.fill(0.0);
                        stop.store(true, Ordering::Relaxed);
                    } else {
                        scratch.resize(frames * 2, 0.0);
                        if mixer.process(source.as_mut(), pos, &mut scratch).is_err() {
                            rec.underruns.fetch_add(1, Ordering::Relaxed);
                            scratch.fill(0.0);
                        }
                        // Simple channel map: stereo out to N channels, silence on the rest.
                        for f in 0..frames {
                            for c in 0..channels {
                                data[f * channels + c] = if c < 2 { scratch[f * 2 + c] } else { 0.0 };
                            }
                        }
                        pos += frames as i64;
                    }
                    rec.frames.fetch_add(frames as u64, Ordering::Relaxed);
                    rec.record(t0.elapsed().as_micros() as u64);
                },
                err_fn,
                None,
            )
            .map_err(|e| e.to_string()),
        _ => Err(format!("default format {format:?} not f32; the POC handles f32 only")),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_ring(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    mut mixer: Mixer,
    mut source: Box<dyn arc_audio::SampleSource + Send>,
    rate: u32,
    channels: usize,
    rec: Arc<Recorder>,
    stop: Arc<AtomicBool>,
    total_frames: u64,
) -> Result<cpal::Stream, String> {
    let capacity = (rate as usize) * 2 * 2; // 2 s of stereo
    let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(capacity);
    let produced = Arc::new(AtomicU64::new(0));
    let producer_stop = Arc::clone(&stop);
    let produced_count = Arc::clone(&produced);
    std::thread::spawn(move || {
        let block = (rate as usize / 100) * 2; // 10 ms stereo
        let mut scratch = vec![0f32; block];
        let mut pos: i64 = 0;
        while !producer_stop.load(Ordering::Relaxed) && (pos as u64) < total_frames * 2 {
            if producer.slots() < block {
                std::thread::yield_now();
                continue;
            }
            if mixer.process(source.as_mut(), pos / 2, &mut scratch).is_err() {
                break;
            }
            for &s in &scratch {
                let _ = producer.push(s);
            }
            pos += block as i64;
            produced_count.fetch_add(block as u64, Ordering::Relaxed);
        }
    });
    let err_fn = |e| eprintln!("audio-probe: stream error: {e}");
    match format {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                config.clone(),
                move |data: &mut [f32], _| {
                    let t0 = Instant::now();
                    let frames = data.len() / channels;
                    let need = frames * 2;
                    if consumer.slots() < need {
                        rec.underruns.fetch_add(1, Ordering::Relaxed);
                    }
                    for f in 0..frames {
                        let l = consumer.pop().unwrap_or(0.0);
                        let r = consumer.pop().unwrap_or(0.0);
                        for c in 0..channels {
                            data[f * channels + c] = match c {
                                0 => l,
                                1 => r,
                                _ => 0.0,
                            };
                        }
                    }
                    rec.frames.fetch_add(frames as u64, Ordering::Relaxed);
                    rec.record(t0.elapsed().as_micros() as u64);
                },
                err_fn,
                None,
            )
            .map_err(|e| e.to_string()),
        _ => Err(format!("default format {format:?} not f32; the POC handles f32 only")),
    }
}
