//! C-shim decode probe: the same operations `arc-media` performs today, measured.
//!
//! ```text
//! shim-probe --file X --mode video-seq --frames 300
//! shim-probe --file X --mode video-seek --frames 100
//! shim-probe --file X --mode audio
//! ```

use arc_core::time::Time;
use arc_media::{AudioDecoder, VideoDecoder};
use serde_json::json;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::time::Instant;

fn hash(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

fn parse() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        map.insert(k.trim_start_matches("--").to_string(), it.next().unwrap_or_default());
    }
    map
}

fn main() {
    let args = parse();
    let file = args.get("file").cloned().expect("--file");
    let mode = args.get("mode").cloned().unwrap_or_else(|| "video-seq".into());
    let frames: usize = args.get("frames").and_then(|v| v.parse().ok()).unwrap_or(300);
    let path = Path::new(&file);

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| match mode.as_str() {
        "audio" => audio(path),
        "video-seek" => video_seek(path, frames),
        _ => video_seq(path, frames),
    }));
    match result {
        Ok(Ok(value)) => println!("{value}"),
        Ok(Err(e)) => {
            println!("{}", json!({"tool":"shim","error": e}));
            std::process::exit(2);
        }
        Err(_) => {
            println!("{}", json!({"tool":"shim","error":"panic"}));
            std::process::exit(3);
        }
    }
}

fn video_seq(path: &Path, frames: usize) -> Result<serde_json::Value, String> {
    let mut dec = VideoDecoder::open(path).map_err(|e| e.to_string())?;
    let info = dec.info().clone();
    let rate = info.frame_rate.ok_or("no frame rate")?;
    let fps = rate.num() as f64 / rate.den() as f64;
    let started = Instant::now();
    let mut total = 0u64;
    let mut digest = 0u64;
    let (mut first_pts, mut last_pts) = (None, None);
    for i in 0..frames {
        let us = (i as f64 * 1_000_000.0 / fps) as i64;
        let t = Time::new(us, 1_000_000).map_err(|e| e.to_string())?;
        let frame = dec.frame_at(t).map_err(|e| e.to_string())?;
        total += frame.rgba.len() as u64;
        digest ^= hash(&frame.rgba[..frame.rgba.len().min(4096)]);
        let us = frame.time.num() * 1_000_000 / frame.time.den() as i64;
        if first_pts.is_none() {
            first_pts = Some(us);
        }
        last_pts = Some(us);
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "tool": "shim",
        "mode": "video-seq",
        "codec": info.codec,
        "pix_fmt": info.format,
        "width": info.width,
        "height": info.height,
        "frames": frames,
        "decode_ms": ms,
        "fps": frames as f64 / (ms / 1000.0),
        "bytes": total,
        "checksum": digest,
        "first_pts_us": first_pts,
        "last_pts_us": last_pts,
        "fps_nominal": fps,
    }))
}

fn video_seek(path: &Path, frames: usize) -> Result<serde_json::Value, String> {
    let mut dec = VideoDecoder::open(path).map_err(|e| e.to_string())?;
    let info = dec.info().clone();
    let duration_us = info
        .duration
        .map(|d| d.num() * 1_000_000 / d.den() as i64)
        .ok_or("no duration")?;
    let started = Instant::now();
    let mut total = 0u64;
    let mut digest = 0u64;
    for i in 0..frames {
        let us = (duration_us as f64 * (i as f64 + 0.5) / frames as f64) as i64;
        let t = Time::new(us, 1_000_000).map_err(|e| e.to_string())?;
        let frame = dec.frame_at(t).map_err(|e| e.to_string())?;
        total += frame.rgba.len() as u64;
        digest ^= hash(&frame.rgba[..frame.rgba.len().min(4096)]);
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "tool": "shim",
        "mode": "video-seek",
        "codec": info.codec,
        "width": info.width,
        "height": info.height,
        "frames": frames,
        "decode_ms": ms,
        "fps": frames as f64 / (ms / 1000.0),
        "bytes": total,
        "checksum": digest,
        "duration_us": duration_us,
    }))
}

fn audio(path: &Path) -> Result<serde_json::Value, String> {
    let mut dec = AudioDecoder::open(path, 48_000).map_err(|e| e.to_string())?;
    let info = dec.info().clone();
    let mut buf = vec![0f32; 4096 * 2];
    let started = Instant::now();
    let mut pos = 0i64;
    let mut samples = 0u64;
    let mut digest = 0u64;
    loop {
        let n = dec.read(pos, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        samples += n as u64;
        digest ^= hash(bytemuck_f32(&buf[..n * 2]));
        pos += n as i64;
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "tool": "shim",
        "mode": "audio",
        "codec": info.codec,
        "in_rate": info.sample_rate,
        "in_channels": info.channels,
        "out_rate": dec.rate(),
        "frames": samples,
        "decode_ms": ms,
        "samples_per_sec": samples as f64 / (ms / 1000.0),
        "checksum": digest,
    }))
}

fn bytemuck_f32(values: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding and the slice is contiguous; the bytes are only hashed.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr() as *const u8, std::mem::size_of_val(values))
    }
}
