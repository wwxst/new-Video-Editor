//! Locating and running the FFmpeg command-line tools as child processes (docs/04 §1.2, §6; spec
//! §4.6).
//!
//! Tools are resolved from the bundle only: `ARCLINE_FFMPEG_DIR/bin`, then the directory of the
//! current executable. `PATH` is used only with `ARCLINE_ALLOW_PATH_FFMPEG=1` in a debug build
//! (this machine has a GPL build on PATH). Once per process the tools are verified against the
//! pinned build identity decoded from the shim headers.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// FFmpeg ident baked in from `include/libavutil/ffversion.h` (e.g. `n9.1-arcline-96f1be0a81`).
pub const BUILD_IDENT: &str = match option_env!("ARC_FFMPEG_BUILD_IDENT") {
    Some(v) => v,
    None => "unpinned",
};

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn path_override_allowed() -> bool {
    // The override is a development aid; a release build never trusts PATH (spec §4.6).
    std::env::var("ARCLINE_ALLOW_PATH_FFMPEG").as_deref() == Ok("1") && cfg!(debug_assertions)
}

fn bundle_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(dir) = std::env::var("ARCLINE_FFMPEG_DIR") {
        dirs.push(PathBuf::from(dir).join("bin"));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        dirs.push(exe_dir);
    }
    dirs
}

fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

fn resolve(name: &str) -> Result<PathBuf, String> {
    let exe = exe_name(name);
    for dir in bundle_dirs() {
        let p = dir.join(&exe);
        if p.is_file() {
            return Ok(p);
        }
    }
    if path_override_allowed() {
        if let Some(p) = find_on_path(&exe) {
            return Ok(p);
        }
    }
    Err(format!(
        "{name} not found in ARCLINE_FFMPEG_DIR/bin or beside the executable \
         (a PATH fallback requires ARCLINE_ALLOW_PATH_FFMPEG=1 in a debug build)"
    ))
}

/// `ffmpeg` (unverified convenience for tests/fixtures; production paths use [`ffmpeg_checked`]).
pub fn ffmpeg() -> PathBuf {
    resolve("ffmpeg").unwrap_or_else(|_| PathBuf::from(exe_name("ffmpeg")))
}

/// `ffprobe` (unverified convenience; production paths use [`ffprobe_checked`]).
pub fn ffprobe() -> PathBuf {
    resolve("ffprobe").unwrap_or_else(|_| PathBuf::from(exe_name("ffprobe")))
}

/// Verified `ffmpeg`, or the reason it cannot be used.
pub fn ffmpeg_checked() -> Result<PathBuf, String> {
    verify()?;
    resolve("ffmpeg")
}

/// Verified `ffprobe`, or the reason it cannot be used.
pub fn ffprobe_checked() -> Result<PathBuf, String> {
    verify()?;
    resolve("ffprobe")
}

struct Verified {
    override_used: bool,
}

fn version_token(first_line: &str) -> Option<String> {
    // "ffmpeg version n9.1-arcline-... Copyright ..." / "ffprobe version n9.1-... Copyright ..."
    let rest = first_line
        .strip_prefix("ffmpeg version ")
        .or_else(|| first_line.strip_prefix("ffprobe version "))?;
    rest.split_whitespace().next().map(str::to_string)
}

fn check_tool(name: &str, exe: &Path) -> Result<(), String> {
    let out = run(
        {
            let mut c = Command::new(exe);
            c.args(["-hide_banner", "-version"]);
            c
        },
        Duration::from_secs(20),
    )
    .map_err(|e| format!("cannot run {name}: {e}"))?;
    if out.exit != Exit::Ok {
        return Err(format!("{name} -version failed: {}", out.stderr.trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or("");
    let token = version_token(first)
        .ok_or_else(|| format!("cannot parse the {name} version line: {first:?}"))?;
    if BUILD_IDENT != "unpinned" && token != BUILD_IDENT {
        return Err(format!(
            "{name} is {token}, but this build is pinned to {BUILD_IDENT}; \
             update ARCLINE_FFMPEG_DIR or the dev environment"
        ));
    }
    if name == "ffmpeg" {
        let lic = run(
            {
                let mut c = Command::new(exe);
                c.args(["-hide_banner", "-L"]);
                c
            },
            Duration::from_secs(20),
        )
        .map_err(|e| format!("cannot run {name} -L: {e}"))?;
        let text = String::from_utf8_lossy(&lic.stdout).to_ascii_lowercase();
        // `-L` prints the LGPL text ("GNU Lesser General Public License ... version 2.1").
        let lgpl = text.contains("lesser general public license") || text.contains("lgpl version");
        if !lgpl || text.contains("nonfree") {
            return Err(format!("{name} does not report an LGPL licence: {text:?}"));
        }
    }
    Ok(())
}

fn verify_state() -> &'static Result<Verified, String> {
    static STATE: OnceLock<Result<Verified, String>> = OnceLock::new();
    STATE.get_or_init(|| {
        let override_allowed = std::env::var("ARCLINE_ALLOW_ANY_FFMPEG").as_deref() == Ok("1");
        let mut problems = Vec::new();
        for name in ["ffmpeg", "ffprobe"] {
            match resolve(name).and_then(|exe| check_tool(name, &exe)) {
                Ok(()) => {}
                Err(e) => problems.push(e),
            }
        }
        if problems.is_empty() {
            Ok(Verified {
                override_used: false,
            })
        } else if override_allowed {
            Ok(Verified {
                override_used: true,
            })
        } else {
            Err(problems.join("; "))
        }
    })
}

/// Verify the tools once per process (ident and licence). A mismatch is a hard error unless
/// `ARCLINE_ALLOW_ANY_FFMPEG=1` is set; the override is recorded in export reports.
pub fn verify() -> Result<(), String> {
    verify_state().as_ref().map(|_| ()).map_err(Clone::clone)
}

/// True when the tool verification was overridden in this process.
pub fn override_used() -> bool {
    verify_state().as_ref().map(|v| v.override_used).unwrap_or(false)
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

/// Names of encoders compiled into the verified `ffmpeg` binary.
pub fn encoders() -> Vec<String> {
    let Ok(exe) = ffmpeg_checked() else {
        return Vec::new();
    };
    let mut cmd = Command::new(exe);
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

#[cfg(test)]
mod tests {
    use super::version_token;

    #[test]
    fn parses_tool_version_tokens() {
        assert_eq!(
            version_token("ffmpeg version n9.1-arcline-96f1be0a81 Copyright (c) 2000-2026"),
            Some("n9.1-arcline-96f1be0a81".into())
        );
        assert_eq!(
            version_token("ffprobe version n9.1 Copyright (c) 2007-2026"),
            Some("n9.1".into())
        );
        assert_eq!(version_token("not a version line"), None);
    }
}
