//! third-probe: the same operations as `shim-probe` (and `arc-media`), but routed
//! through the pure-Rust binding crate `ffmpeg-the-third` (which wraps
//! `ffmpeg-sys-the-third`) instead of the project's own C shim.
//!
//! ```text
//! third-probe --file X --mode video-seq  --frames 300
//! third-probe --file X --mode video-seek --frames 100
//! third-probe --file X --mode audio
//! third-probe --file X --mode filter --filter "scale=1280:720" --limit 100
//! ```
//!
//! One JSON object is printed on stdout. Failures print `{"tool":"third","error":...}`
//! and exit non-zero.

use std::path::Path;
use std::time::Instant;

// The crate is published as `ffmpeg-the-third` but its lib target is
// `ffmpeg_the_third`; alias it so the call sites read like the upstream
// `ffmpeg` crate (as documented by the project).
use ffmpeg_the_third as ffmpeg;

use ffmpeg_the_third::codec;
use ffmpeg_the_third::filter;
use ffmpeg_the_third::format::{self, Pixel};
use ffmpeg_the_third::frame;
use ffmpeg_the_third::media::Type;
use ffmpeg_the_third::software::{resampling, scaling};
use ffmpeg_the_third::ChannelLayout;
use serde_json::json;

/// FNV-1a basis / prime, same constants shim-probe uses.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

const OUT_RATE: u32 = 48_000;
const OUT_CHANNELS: u32 = 2;

fn fnv_into(state: &mut u64, bytes: &[u8]) {
    let mut h = *state;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    *state = h;
}

/// Per-frame FNV-1a, XOR-accumulated into a running digest (shim-probe shape).
fn fnv(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    fnv_into(&mut h, bytes);
    h
}

fn us_of(pts: i64, num: i32, den: i32) -> i64 {
    if den == 0 {
        return 0;
    }
    (pts as i128 * num as i128 * 1_000_000 / den as i128) as i64
}

fn fps_of(ms: f64, frames: usize) -> f64 {
    if ms <= 0.0 { 0.0 } else { frames as f64 / (ms / 1000.0) }
}

fn params() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        map.insert(k.trim_start_matches("--").to_string(), it.next().unwrap_or_default());
    }
    map
}

const API_NOTE: &str =
    "FFmpeg 9.1 via ffmpeg-the-third 6.0: demux/decode/scale/resample/filter are safe wrappers. \
     Manual/unsafe gaps: (1) seek takes RangeBounds, there is no SeekRange::Backward -> used ..=ts \
     which maps to avformat_seek_file(min=MIN, ts, max=ts); (2) buffer source args need the numeric \
     AVPixelFormat, so AVPixelFormat::from(pixel).0 is taken from ffmpeg::ffi; (3) frame data is \
     stride/linesize padded, so RGB24 rows are hashed row-by-row and audio is clamped to \
     samples*channels*4; (4) decoder + resampler need explicit EOF/drain and flush loops; \
     (5) filter graph: buffer/buffersink + the parsed filter are added with the safe \
     Graph::add, but the safe Parser did not connect them (buffer output pad left unconnected), \
     so the three pads are wired with unsafe ffmpeg::ffi::avfilter_link; \
     (6) the safe resampler allocates its output from input.samples() and truncates 44.1k->48k, \
     so the probe allocates the output frame itself.";

fn main() {
    let args = params();
    let Some(file) = args.get("file").cloned() else {
        println!("{}", json!({"tool":"third","error":"missing --file"}));
        std::process::exit(2);
    };
    let mode = args.get("mode").cloned().unwrap_or_else(|| "video-seq".into());
    let frames: usize = args.get("frames").and_then(|v| v.parse().ok()).unwrap_or(300);
    let limit: usize = args.get("limit").and_then(|v| v.parse().ok()).unwrap_or(100);
    let spec = args
        .get("filter")
        .cloned()
        .unwrap_or_else(|| "scale=1280:720".into());
    let path = Path::new(&file);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Err(e) = ffmpeg::init() {
            return Err(format!("ffmpeg::init: {e}"));
        }
        match mode.as_str() {
            "audio" => audio(path),
            "video-seek" => video_seek(path, frames),
            "filter" => filter_mode(path, &spec, limit),
            _ => video_seq(path, frames),
        }
    }));

    match result {
        Ok(Ok(value)) => println!("{value}"),
        Ok(Err(e)) => {
            println!("{}", json!({"tool":"third","mode": mode,"error": e}));
            std::process::exit(2);
        }
        Err(_) => {
            println!("{}", json!({"tool":"third","mode": mode,"error":"panic"}));
            std::process::exit(3);
        }
    }
}

/// Everything needed before the packet loop; the decoder owns copies of the
/// codec parameters, so no borrow of the demuxer escapes this block.
struct VideoSetup {
    index: usize,
    tb_num: i32,
    tb_den: i32,
    nominal_fps: f64,
    codec: String,
    width: u32,
    height: u32,
    pix_fmt: String,
    decoder: ffmpeg::decoder::Video,
}

fn open_video(ictx: &mut format::context::Input) -> Result<VideoSetup, String> {
    let stream = ictx
        .streams()
        .best(Type::Video)
        .ok_or_else(|| "no video stream".to_string())?;
    let index = stream.index();
    let tb = stream.time_base();
    let nominal = stream.avg_frame_rate();
    let nominal_fps = if nominal.denominator() > 0 {
        nominal.numerator() as f64 / nominal.denominator() as f64
    } else {
        0.0
    };
    let codec = stream.parameters().id().name().to_string();
    let ctx = codec::context::Context::from_parameters(stream.parameters())
        .map_err(|e| format!("codec context: {e}"))?;
    let decoder = ctx.decoder().video().map_err(|e| format!("open decoder: {e}"))?;
    let (width, height, pix) = (decoder.width(), decoder.height(), decoder.format());
    Ok(VideoSetup {
        index,
        tb_num: tb.numerator(),
        tb_den: tb.denominator(),
        nominal_fps,
        codec,
        width,
        height,
        pix_fmt: format!("{:?}", pix),
        decoder,
    })
}

fn video_seq(path: &Path, want: usize) -> Result<serde_json::Value, String> {
    let mut ictx = format::input(path).map_err(|e| format!("format::input: {e}"))?;
    let mut vs = open_video(&mut ictx)?;

    let w = vs.width;
    let h = vs.height;
    let rgb_stride_bytes = w as usize * 3;
    let mut scaler = scaling::Context::get(
        vs.decoder.format(),
        w,
        h,
        Pixel::RGB24,
        w,
        h,
        scaling::Flags::BILINEAR,
    )
    .map_err(|e| format!("sws_getContext: {e}"))?;

    let started = Instant::now();
    let mut decoded = 0usize;
    let mut bytes = 0u64;
    let mut digest = 0u64;
    let (mut first_pts, mut last_pts) = (None::<i64>, None::<i64>);
    let mut src = frame::Video::empty();
    let mut rgb = frame::Video::empty();

    'packets: for item in ictx.packets() {
        let (stream, packet) = item.map_err(|e| format!("read packet: {e}"))?;
        if stream.index() != vs.index {
            continue;
        }
        vs.decoder
            .send_packet(&packet)
            .map_err(|e| format!("send_packet: {e}"))?;
        while vs.decoder.receive_frame(&mut src).is_ok() {
            let pts = src
                .pts()
                .or_else(|| src.timestamp())
                .map(|p| us_of(p, vs.tb_num, vs.tb_den));
            scaler
                .run(&src, &mut rgb)
                .map_err(|e| format!("sws_scale: {e}"))?;
            let stride = rgb.stride(0);
            let mut fh = FNV_OFFSET;
            for y in 0..h as usize {
                let off = y * stride;
                fnv_into(&mut fh, &rgb.data(0)[off..off + rgb_stride_bytes]);
            }
            digest ^= fh;
            bytes += (rgb_stride_bytes * h as usize) as u64;
            decoded += 1;
            if first_pts.is_none() {
                first_pts = pts;
            }
            last_pts = pts;
            if decoded >= want {
                break 'packets;
            }
        }
    }

    // Drain whatever the decoder still holds (B-frames / delayed frames).
    if decoded < want {
        let _ = vs.decoder.send_eof();
        while vs.decoder.receive_frame(&mut src).is_ok() && decoded < want {
            let pts = src
                .pts()
                .or_else(|| src.timestamp())
                .map(|p| us_of(p, vs.tb_num, vs.tb_den));
            scaler
                .run(&src, &mut rgb)
                .map_err(|e| format!("sws_scale: {e}"))?;
            let stride = rgb.stride(0);
            let mut fh = FNV_OFFSET;
            for y in 0..h as usize {
                let off = y * stride;
                fnv_into(&mut fh, &rgb.data(0)[off..off + rgb_stride_bytes]);
            }
            digest ^= fh;
            bytes += (rgb_stride_bytes * h as usize) as u64;
            decoded += 1;
            if first_pts.is_none() {
                first_pts = pts;
            }
            last_pts = pts;
        }
    }

    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "tool": "third",
        "mode": "video-seq",
        "api_note": API_NOTE,
        "codec": vs.codec,
        "pix_fmt": vs.pix_fmt,
        "width": w,
        "height": h,
        "requested": want,
        "frames": decoded,
        "decode_ms": ms,
        "fps": fps_of(ms, decoded),
        "fps_nominal": vs.nominal_fps,
        "bytes": bytes,
        "checksum": digest,
        "first_pts": first_pts,
        "last_pts": last_pts,
    }))
}

fn video_seek(path: &Path, want: usize) -> Result<serde_json::Value, String> {
    let mut ictx = format::input(path).map_err(|e| format!("format::input: {e}"))?;
    let mut vs = open_video(&mut ictx)?;
    let duration_us = ictx.duration();
    if duration_us <= 0 {
        return Err(format!("no usable duration ({duration_us})"));
    }

    let w = vs.width;
    let h = vs.height;
    let rgb_stride_bytes = w as usize * 3;
    let mut scaler = scaling::Context::get(
        vs.decoder.format(),
        w,
        h,
        Pixel::RGB24,
        w,
        h,
        scaling::Flags::BILINEAR,
    )
    .map_err(|e| format!("sws_getContext: {e}"))?;

    let started = Instant::now();
    let mut decoded = 0usize;
    let mut misses = 0usize;
    let mut bytes = 0u64;
    let mut digest = 0u64;
    let (mut first_pts, mut last_pts) = (None::<i64>, None::<i64>);
    let mut rgb = frame::Video::empty();

    for i in 0..want {
        let ts = (duration_us as f64 * (i as f64 + 0.5) / want as f64) as i64;
        vs.decoder.flush();
        // `..=ts` == seek to the keyframe at or before ts (old SeekRange::Backward).
        ictx.seek(ts, ..=ts)
            .map_err(|e| format!("seek({ts}): {e}"))?;

        // A backward seek lands on the previous keyframe, so decode forward until
        // the first frame whose timestamp reaches the requested one.
        let mut one = frame::Video::empty();
        let mut candidate: Option<frame::Video> = None;
        'packets: for item in ictx.packets() {
            let (stream, packet) = item.map_err(|e| format!("read packet: {e}"))?;
            if stream.index() != vs.index {
                continue;
            }
            if vs.decoder.send_packet(&packet).is_err() {
                continue;
            }
            while vs.decoder.receive_frame(&mut one).is_ok() {
                let fpts = one
                    .pts()
                    .or_else(|| one.timestamp())
                    .map(|p| us_of(p, vs.tb_num, vs.tb_den))
                    .unwrap_or(i64::MIN);
                let reached = fpts >= ts;
                candidate = Some(std::mem::replace(&mut one, frame::Video::empty()));
                if reached {
                    break 'packets;
                }
            }
        }
        let Some(one) = candidate else {
            misses += 1;
            continue;
        };
        let pts = one
            .pts()
            .or_else(|| one.timestamp())
            .map(|p| us_of(p, vs.tb_num, vs.tb_den));
        scaler
            .run(&one, &mut rgb)
            .map_err(|e| format!("sws_scale: {e}"))?;
        let stride = rgb.stride(0);
        let mut fh = FNV_OFFSET;
        for y in 0..h as usize {
            let off = y * stride;
            fnv_into(&mut fh, &rgb.data(0)[off..off + rgb_stride_bytes]);
        }
        digest ^= fh;
        bytes += (rgb_stride_bytes * h as usize) as u64;
        decoded += 1;
        if first_pts.is_none() {
            first_pts = pts;
        }
        last_pts = pts;
    }

    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "tool": "third",
        "mode": "video-seek",
        "api_note": API_NOTE,
        "codec": vs.codec,
        "width": w,
        "height": h,
        "requested": want,
        "frames": decoded,
        "misses": misses,
        "decode_ms": ms,
        "fps": fps_of(ms, decoded),
        "bytes": bytes,
        "checksum": digest,
        "duration_us": duration_us,
        "first_pts": first_pts,
        "last_pts": last_pts,
    }))
}

fn audio(path: &Path) -> Result<serde_json::Value, String> {
    let mut ictx = format::input(path).map_err(|e| format!("format::input: {e}"))?;

    // Extract everything we need, then let the stream borrow go.
    let (index, codec_name, mut decoder) = {
        let stream = ictx
            .streams()
            .best(Type::Audio)
            .ok_or_else(|| "no audio stream".to_string())?;
        let index = stream.index();
        let codec_name = stream.parameters().id().name().to_string();
        let ctx = codec::context::Context::from_parameters(stream.parameters())
            .map_err(|e| format!("codec context: {e}"))?;
        let decoder = ctx.decoder().audio().map_err(|e| format!("open decoder: {e}"))?;
        (index, codec_name, decoder)
    };

    let in_rate = decoder.rate();
    let in_fmt = decoder.format();
    let in_mask = decoder
        .ch_layout()
        .mask()
        .ok_or_else(|| "audio stream has no channel layout mask".to_string())?;
    let in_channels = decoder.ch_layout().channels();

    let need_resample = !(in_rate == OUT_RATE
        && in_channels == OUT_CHANNELS
        && in_fmt == format::Sample::F32(format::sample::Type::Packed));

    let mut resampler = if need_resample {
        let src_layout = ChannelLayout::from_mask(in_mask)
            .ok_or_else(|| format!("unsupported channel layout {in_mask:?}"))?;
        Some(
            resampling::Context::get2(
                in_fmt,
                src_layout,
                in_rate,
                format::Sample::F32(format::sample::Type::Packed),
                ChannelLayout::STEREO,
                OUT_RATE,
            )
            .map_err(|e| format!("swr_alloc_set_opts2: {e}"))?,
        )
    } else {
        None
    };

    let started = Instant::now();
    let mut samples = 0u64;
    let mut digest = 0u64;
    let mut src = frame::Audio::empty();

    // Hash one decoded (or resampled) frame.
    fn take(frame: &frame::Audio, samples: &mut u64, digest: &mut u64) -> Result<(), String> {
        let n = frame.samples();
        let ch = frame.ch_layout().channels() as usize;
        if n == 0 || ch == 0 {
            return Ok(());
        }
        let expected = n * ch * std::mem::size_of::<f32>();
        let data = frame.data(0);
        let len = expected.min(data.len());
        *digest ^= fnv(&data[..len]);
        *samples += n as u64;
        Ok(())
    }

    // ffmpeg-the-third's `Context::run` allocates the output frame with
    // `input.samples()` elements, which is too small when upsampling
    // (44100 -> 48000 needs ~1.088x) and silently truncates every frame, so the
    // output frame is sized and allocated here instead.
    fn out_frame(capacity: usize) -> frame::Audio {
        frame::Audio::new(
            format::Sample::F32(format::sample::Type::Packed),
            capacity,
            ChannelLayout::STEREO.mask().unwrap(),
        )
    }
    let out_cap = if need_resample {
        ((1024.0 * OUT_RATE as f64 / in_rate as f64).ceil() as usize + 64).max(2048)
    } else {
        0
    };

    for item in ictx.packets() {
        let (stream, packet) = item.map_err(|e| format!("read packet: {e}"))?;
        if stream.index() != index {
            continue;
        }
        decoder
            .send_packet(&packet)
            .map_err(|e| format!("send_packet: {e}"))?;
        while decoder.receive_frame(&mut src).is_ok() {
            if let Some(rs) = resampler.as_mut() {
                let mut out = out_frame(out_cap);
                rs.run(&src, &mut out)
                    .map_err(|e| format!("swr_convert_frame: {e}"))?;
                take(&out, &mut samples, &mut digest)?;
            } else {
                take(&src, &mut samples, &mut digest)?;
            }
        }
    }

    // EOF: let the decoder emit delayed frames, then flush the resampler.
    let _ = decoder.send_eof();
    while decoder.receive_frame(&mut src).is_ok() {
        if let Some(rs) = resampler.as_mut() {
            let mut out = out_frame(out_cap);
            rs.run(&src, &mut out)
                .map_err(|e| format!("swr_convert_frame: {e}"))?;
            take(&out, &mut samples, &mut digest)?;
        } else {
            take(&src, &mut samples, &mut digest)?;
        }
    }
    if let Some(rs) = resampler.as_mut() {
        loop {
            let mut out = out_frame(8192);
            match rs.flush(&mut out) {
                Ok(delay) => {
                    let produced = out.samples();
                    take(&out, &mut samples, &mut digest)?;
                    if produced == 0 || delay.is_none() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "tool": "third",
        "mode": "audio",
        "api_note": API_NOTE,
        "codec": codec_name,
        "in_rate": in_rate,
        "in_channels": in_channels,
        "in_sample_fmt": format!("{:?}", in_fmt),
        "out_rate": OUT_RATE,
        "out_channels": OUT_CHANNELS,
        "resampled": need_resample,
        "samples": samples,
        "decode_ms": ms,
        "samples_per_sec": if ms <= 0.0 { 0.0 } else { samples as f64 / (ms / 1000.0) },
        "checksum": digest,
    }))
}

fn split_filter(spec: &str) -> (String, String) {
    match spec.split_once('=') {
        Some((name, args)) => (name.trim().to_string(), args.trim().to_string()),
        None => (spec.trim().to_string(), String::new()),
    }
}

/// `Graph::get` borrows the graph mutably for the lifetime of the returned
/// `Context`, so two live contexts cannot both come from `graph.get`. Take the
/// raw `AVFilterContext` pointer once and re-wrap it: the graph owns the filter
/// contexts for as long as the `Graph` lives.
fn graph_pad(graph: &mut filter::Graph, name: &str) -> Result<filter::Context<'static>, String> {
    let ctx = graph
        .get(name)
        .ok_or_else(|| format!("graph has no '{name}'"))?;
    let ptr = unsafe { ctx.as_ptr() } as *mut ffmpeg::ffi::AVFilterContext;
    drop(ctx);
    // SAFETY: the filter context is owned by `graph` and outlives the wrapper.
    Ok(unsafe { filter::Context::wrap(ptr) })
}

fn filter_mode(path: &Path, spec: &str, limit: usize) -> Result<serde_json::Value, String> {
    let mut ictx = format::input(path).map_err(|e| format!("format::input: {e}"))?;
    let mut vs = open_video(&mut ictx)?;
    let w = vs.width;
    let h = vs.height;

    let (name, args) = split_filter(spec);
    let filter_impl = filter::find(&name).ok_or_else(|| format!("unknown filter '{name}'"))?;
    let buffer = filter::find("buffer").ok_or_else(|| "missing buffer filter".to_string())?;
    let buffersink =
        filter::find("buffersink").ok_or_else(|| "missing buffersink filter".to_string())?;

    let sar = vs.decoder.aspect_ratio();
    let (sar_num, sar_den) = if sar.numerator() <= 0 || sar.denominator() <= 0 {
        (1, 1)
    } else {
        (sar.numerator(), sar.denominator())
    };
    // The binding's `Pixel` enum has no numeric discriminants, so the buffer
    // source argument has to use the generated AVPixelFormat newtype.
    let pix_num = ffmpeg::ffi::AVPixelFormat::from(vs.decoder.format()).0 as i32;
    let src_args = format!(
        "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
        w, h, pix_num, vs.tb_num, vs.tb_den, sar_num, sar_den
    );

    let mut graph = filter::Graph::new();
    {
        graph
            .add(&buffer, "in", &src_args)
            .map_err(|e| format!("add buffer: {e}"))?;
        // `split_filter` hands the already-parsed argument string (e.g. "1280:720")
        // to the filter's own init, so all option parsing stays inside libavfilter.
        graph
            .add(&filter_impl, "filter", &args)
            .map_err(|e| format!("add filter '{name}': {e}"))?;
        graph
            .add(&buffersink, "out", "")
            .map_err(|e| format!("add buffersink: {e}"))?;
    }

    // The safe `filter::graph::Parser` path does not connect filters that were
    // added with `Graph::add` (parsing "[in]scale=...[out]" left the buffer's
    // output pad unconnected and `validate` failed), so the three pads are
    // wired explicitly through the raw FFI.
    let mut src_ctx = graph_pad(&mut graph, "in")?;
    let mut filt_ctx = graph_pad(&mut graph, "filter")?;
    let mut sink_ctx = graph_pad(&mut graph, "out")?;
    unsafe {
        let rc = ffmpeg::ffi::avfilter_link(src_ctx.as_mut_ptr(), 0, filt_ctx.as_mut_ptr(), 0);
        if rc < 0 {
            return Err(format!("avfilter_link(in->filter): {}", ffmpeg::Error::from(rc)));
        }
        let rc = ffmpeg::ffi::avfilter_link(filt_ctx.as_mut_ptr(), 0, sink_ctx.as_mut_ptr(), 0);
        if rc < 0 {
            return Err(format!("avfilter_link(filter->out): {}", ffmpeg::Error::from(rc)));
        }
    }
    graph.validate().map_err(|e| format!("graph validate: {e}"))?;

    let started = Instant::now();
    let mut in_frames = 0usize;
    let mut out_frames = 0usize;
    let mut digest = 0u64;
    let (mut out_w, mut out_h) = (0u32, 0u32);
    let mut out = frame::Video::empty();
    let mut dec_out = frame::Video::empty();

    {
        let mut src = src_ctx.source();
        let mut sink = sink_ctx.sink();

        'packets: for item in ictx.packets() {
            let (stream, packet) = item.map_err(|e| format!("read packet: {e}"))?;
            if stream.index() != vs.index {
                continue;
            }
            vs.decoder
                .send_packet(&packet)
                .map_err(|e| format!("send_packet: {e}"))?;
            while vs.decoder.receive_frame(&mut dec_out).is_ok() {
                src.add(&dec_out).map_err(|e| format!("buffersrc add: {e}"))?;
                in_frames += 1;
                loop {
                    match sink.frame(&mut out) {
                        Ok(()) => {
                            out_frames += 1;
                            if out_w == 0 {
                                out_w = out.width();
                                out_h = out.height();
                            }
                            digest ^= fnv(&out.data(0));
                            out = frame::Video::empty();
                        }
                        Err(_) => break,
                    }
                }
                if in_frames >= limit {
                    break 'packets;
                }
                dec_out = frame::Video::empty();
            }
        }

        src.flush().map_err(|e| format!("buffersrc flush: {e}"))?;
        loop {
            match sink.frame(&mut out) {
                Ok(()) => {
                    out_frames += 1;
                    if out_w == 0 {
                        out_w = out.width();
                        out_h = out.height();
                    }
                    digest ^= fnv(&out.data(0));
                    out = frame::Video::empty();
                }
                Err(_) => break,
            }
        }
    }

    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({
        "tool": "third",
        "mode": "filter",
        "api_note": API_NOTE,
        "filter": spec,
        "filter_name": filter_impl.name(),
        "width": w,
        "height": h,
        "limit": limit,
        "in_frames": in_frames,
        "out_frames": out_frames,
        "out_width": out_w,
        "out_height": out_h,
        "filter_ms": ms,
        "checksum": digest,
    }))
}
