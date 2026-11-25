//! Locating and running the FFmpeg command-line tools as child processes (docs/04 §1.2, §6).
//!
//! Child processes are always invoked with argument vectors, never shell strings.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn tool(name: &str) -> PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Ok(dir) = std::env::var("ARCLINE_FFMPEG_DIR") {
        let p = PathBuf::from(dir).join("bin").join(&exe);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(exe)
}

/// `ffmpeg` — prefer the LGPL build in `ARCLINE_FFMPEG_DIR/bin`, else PATH.
pub fn ffmpeg() -> PathBuf {
    tool("ffmpeg")
}

/// `ffprobe` — same lookup as [`ffmpeg`].
pub fn ffprobe() -> PathBuf {
    tool("ffprobe")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Ok,
    /// Normal non-zero exit (the tool reported an error).
    Failed(i32),
    /// Abnormal termination (crash: access violation, abort, signal).
    Crashed(i32),
    TimedOut,
}

#[derive(Debug, Clone)]
pub struct Output {
    pub exit: Exit,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Run a command with a timeout, killing it if the deadline passes.
pub fn run(mut cmd: Command, timeout: Duration) -> std::io::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut v);
        }
        v
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + timeout;
    let exit = loop {
        match child.try_wait()? {
            Some(status) => break classify(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break Exit::TimedOut;
            }
            None => std::thread::sleep(Duration::from_millis(15)),
        }
    };
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    Ok(Output {
        exit,
        stdout,
        stderr,
    })
}

fn classify(status: std::process::ExitStatus) -> Exit {
    if status.success() {
        return Exit::Ok;
    }
    match status.code() {
        // Windows reports crashes as NTSTATUS codes (0xC0000005 access violation, 0xC0000409
        // stack buffer overrun / abort, ...): negative as i32.
        Some(c) if cfg!(windows) && c < 0 => Exit::Crashed(c),
        // abort() on Windows CRT exits with 3
        Some(3) if cfg!(windows) => Exit::Crashed(3),
        Some(c) => Exit::Failed(c),
        None => Exit::Crashed(-1), // killed by a signal (unix)
    }
}

/// Names of encoders compiled into the `ffmpeg` binary.
pub fn encoders() -> Vec<String> {
    let mut cmd = Command::new(ffmpeg());
    cmd.args(["-hide_banner", "-encoders"]);
    let Ok(out) = run(cmd, Duration::from_secs(20)) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let flags = parts.next()?;
            let name = parts.next()?;
            (flags.len() == 6 && flags.starts_with(['V', 'A', 'S'])).then(|| name.to_string())
        })
        .collect()
}
