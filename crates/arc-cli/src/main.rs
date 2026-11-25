//! `arcline-cli` — headless Arcline (docs/06 §2).
//!
//! Output contract: stdout carries exactly one JSON document (or the text timeline for
//! `timeline`). Errors print `{"error": {...}}` with a stable `kind` and exit with code 2;
//! usage errors exit with code 64.
//!
//! Each invocation is one session; undo history does not persist across sessions (docs/02 §5),
//! so undo/redo live in the JSON-RPC session (`arcline-cli rpc`, Phase F), not here.

use arc_cli::inspect;
use arc_core::engine::{ApplyRequest, Source};
use arc_core::journal::Durability;
use arc_core::model::{Project, Resolution};
use arc_core::runtime::EngineHost;
use arc_core::{Command, CommandError, FrameRate};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "arcline-cli",
    version,
    about = "Headless Arcline: create, inspect, and edit projects"
)]
struct Cli {
    /// fsync every journal record before acknowledging (docs/08 §4.4)
    #[arg(long, global = true)]
    durable: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create a new project file
    New {
        path: PathBuf,
        #[arg(long, default_value = "Untitled")]
        name: String,
        /// Frame rate: 23.976, 24, 25, 29.97, 30, 50, 59.94, 60, or an exact rational
        #[arg(long, default_value = "30")]
        fps: String,
        /// Resolution WxH
        #[arg(long, default_value = "1920x1080")]
        res: String,
    },
    /// Print the project.describe summary (JSON)
    Describe {
        path: PathBuf,
        /// Include exact rationals
        #[arg(long)]
        exact: bool,
    },
    /// Print the text timeline view
    Timeline { path: PathBuf },
    /// Apply a JSON command batch (array, or {"commands": [...]}) from a file or stdin, then save
    Apply {
        path: PathBuf,
        /// Command file; reads stdin when omitted
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        base_revision: Option<u64>,
    },
}

struct Fail {
    code: u8,
    error: Value,
}

impl From<CommandError> for Fail {
    fn from(e: CommandError) -> Self {
        Fail {
            code: 2,
            error: serde_json::to_value(e).unwrap_or_default(),
        }
    }
}

fn fail(kind: &str, message: impl std::fmt::Display) -> Fail {
    Fail {
        code: 2,
        error: json!({"kind": kind, "message": message.to_string()}),
    }
}

fn durability(cli: &Cli) -> Durability {
    if cli.durable {
        Durability::Durable
    } else {
        Durability::Default
    }
}

fn open(path: &PathBuf, d: Durability) -> Result<EngineHost, Fail> {
    EngineHost::open(path, arc_render::builtin::catalog(), d).map_err(|e| match e {
        arc_core::runtime::HostError::Store(arc_core::store::StoreError::Locked(_)) => {
            fail("project_locked", e)
        }
        arc_core::runtime::HostError::Command(c) => Fail::from(c),
        other => fail("io_error", other),
    })
}

fn finish(host: EngineHost) -> Result<u64, Fail> {
    let rev = host.save().map_err(Fail::from)?;
    host.shutdown();
    Ok(rev)
}

fn parse_resolution(s: &str) -> Result<Resolution, Fail> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| fail("invalid_value", "resolution must be WxH"))?;
    let w: u32 = w.parse().map_err(|_| fail("invalid_value", "bad width"))?;
    let h: u32 = h.parse().map_err(|_| fail("invalid_value", "bad height"))?;
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 || w > 8192 || h > 8192 {
        return Err(fail(
            "out_of_range",
            "resolution must be even and within 2..=8192",
        ));
    }
    Ok(Resolution { w, h })
}

fn read_commands(file: &Option<PathBuf>) -> Result<Vec<Command>, Fail> {
    let text = match file {
        Some(p) => std::fs::read_to_string(p).map_err(|e| fail("io_error", e))?,
        None => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| fail("io_error", e))?;
            s
        }
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| fail("invalid_command", e))?;
    let list = match v {
        Value::Array(_) => v,
        Value::Object(mut o) => o.remove("commands").unwrap_or(Value::Null),
        _ => Value::Null,
    };
    serde_json::from_value(list).map_err(|e| fail("invalid_command", e))
}

fn run(cli: Cli) -> Result<String, Fail> {
    let d = durability(&cli);
    match &cli.cmd {
        Cmd::New {
            path,
            name,
            fps,
            res,
        } => {
            let rate = FrameRate::parse(fps).map_err(|e| Fail::from(CommandError::from(e)))?;
            let project = Project::new(name, rate, parse_resolution(res)?);
            let host = EngineHost::create(path, project, arc_render::builtin::catalog(), d)
                .map_err(|e| fail("io_error", e))?;
            let p = host.published();
            let out = inspect::describe(&p.project, p.revision, false);
            finish(host)?;
            Ok(out.to_string())
        }
        Cmd::Describe { path, exact } => {
            let host = open(path, d)?;
            let p = host.published();
            let mut out = inspect::describe(&p.project, p.revision, *exact);
            if host.recovery().recovered {
                out["recovered"] = serde_json::to_value(host.recovery()).unwrap_or_default();
            }
            host.shutdown();
            Ok(out.to_string())
        }
        Cmd::Timeline { path } => {
            let host = open(path, d)?;
            let text = inspect::timeline_text(&host.published().project, None);
            host.shutdown();
            Ok(text)
        }
        Cmd::Apply {
            path,
            file,
            dry_run,
            base_revision,
        } => {
            let commands = read_commands(file)?;
            let host = open(path, d)?;
            let r = host.apply(ApplyRequest {
                commands,
                dry_run: *dry_run,
                base_revision: *base_revision,
                source: Some(Source::Client("arcline-cli".into())),
                ..Default::default()
            });
            let r = match r {
                Ok(r) => r,
                Err(e) => {
                    host.shutdown();
                    return Err(e.into());
                }
            };
            if !dry_run {
                finish(host)?;
            } else {
                host.shutdown();
            }
            Ok(serde_json::to_string(&r).unwrap_or_default())
        }
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { 64 } else { 0 });
        }
    };
    match run(cli) {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(f) => {
            println!("{}", json!({"error": f.error}));
            ExitCode::from(f.code)
        }
    }
}
