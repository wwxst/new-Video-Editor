//! `arcline-cli` ??headless Arcline (docs/06 ?2).
//!
//! Output contract: stdout carries exactly one JSON document (or the text timeline for
//! `timeline`). Errors print `{"error": {...}}` with a stable `kind` and exit with code 2;
//! usage errors exit with code 64.
//!
//! Each invocation is one session; undo history does not persist across sessions (docs/02 ?5),
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
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "arcline-cli",
    version,
    about = "Headless Arcline: create, inspect, and edit projects"
)]
struct Cli {
    /// fsync every journal record before acknowledging (docs/08 ?4.4)
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
        /// Replace an existing project file (refused by default; never replaces a project that is
        /// open in another session)
        #[arg(long)]
        overwrite: bool,
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
    /// Serve a JSON-RPC 2.0 session on stdin/stdout (docs/06 ?4)
    Rpc {
        /// Client id recorded as the source of every edit
        #[arg(long, default_value = "rpc")]
        client: String,
    },
    /// Run the subprocess import check on a media file (JSON)
    Probe { media: PathBuf },
    /// Import media files into a project (import check + probe), then save
    Import { path: PathBuf, media: Vec<PathBuf> },
    /// Render the project (last saved state, or --snapshot) to a file and write <output>.report.json
    Render {
        path: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// ffv1 | h264 | av1 | prores
        #[arg(long, default_value = "h264")]
        codec: String,
        /// Start timecode (inclusive)
        #[arg(long)]
        from: Option<String>,
        /// End timecode (exclusive)
        #[arg(long)]
        to: Option<String>,
        /// Render this document snapshot (JSON) instead of the saved project, e.g. one written by
        /// the GUI or job.submit while the project is open
        #[arg(long)]
        snapshot: Option<PathBuf>,
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

fn open(path: &Path, d: Durability) -> Result<EngineHost, Fail> {
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

/// Media arguments must be local paths, never URLs, protocol strings or device namespaces
/// (audit F-09).
fn reject_non_local(p: &Path) -> Result<(), Fail> {
    let s = p.to_string_lossy();
    if arc_cli::media::has_url_scheme(&s) || arc_cli::media::is_device_path(&s) {
        return Err(fail(
            "invalid_value",
            format!("media path must be a local file path, not a URL, protocol or device: {s}"),
        ));
    }
    Ok(())
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

fn outcome_json(o: &arc_media::probe::ImportOutcome) -> Value {
    use arc_media::probe::ImportOutcome::*;
    match o {
        Ok { kind, probe } => json!({"status": "ok", "kind": kind, "probe": probe}),
        Quarantined {
            reason,
            kind,
            probe,
        } => json!({"status": "quarantined", "reason": reason, "kind": kind, "probe": probe}),
        Offline { reason } => json!({"status": "offline", "reason": reason}),
    }
}

fn run(cli: Cli) -> Result<String, Fail> {
    let d = durability(&cli);
    match &cli.cmd {
        Cmd::Rpc { .. } => Err(fail("invalid_command", "rpc is handled before dispatch")),
        Cmd::New {
            path,
            name,
            fps,
            res,
            overwrite,
        } => {
            let rate = FrameRate::parse(fps).map_err(|e| Fail::from(CommandError::from(e)))?;
            let project = Project::new(name, rate, parse_resolution(res)?);
            let host = EngineHost::create(
                path,
                project,
                arc_render::builtin::catalog(),
                d,
                *overwrite,
            )
            .map_err(|e| match e {
                arc_core::runtime::HostError::Store(arc_core::store::StoreError::Locked(_)) => {
                    fail("project_locked", &e)
                }
                arc_core::runtime::HostError::Store(arc_core::store::StoreError::Exists(_)) => {
                    fail("project_exists", &e)
                }
                other => fail("io_error", other),
            })?;
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
            let mut commands = read_commands(file)?;
            arc_cli::media::resolve_imports(path, &mut commands).map_err(Fail::from)?;
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
        Cmd::Probe { media } => {
            reject_non_local(media)?;
            Ok(outcome_json(&arc_media::probe::import_check(media)).to_string())
        }
        Cmd::Import { path, media } => {
            use arc_media::probe::ImportOutcome;
            let mut commands = Vec::new();
            let mut outcomes = Vec::new();
            for m in media {
                reject_non_local(m)?;
                let o = arc_media::probe::import_check(m);
                outcomes.push(json!({"path": m, "result": outcome_json(&o)}));
                let rel = arc_cli::media::project_relative(path, m);
                let (kind, probe, status) = match o {
                    ImportOutcome::Ok { kind, probe } => {
                        (kind, Some(probe), arc_core::model::AssetStatus::Ok)
                    }
                    ImportOutcome::Quarantined { kind, probe, .. } => (
                        kind.unwrap_or(arc_core::model::AssetKind::Video),
                        probe,
                        arc_core::model::AssetStatus::Quarantined,
                    ),
                    ImportOutcome::Offline { .. } => (
                        arc_core::model::AssetKind::Video,
                        None,
                        arc_core::model::AssetStatus::Offline,
                    ),
                };
                commands.push(Command::AssetImport {
                    id: None,
                    path: rel,
                    kind,
                    probe,
                    status,
                });
            }
            let host = open(path, d)?;
            let r = host.apply(ApplyRequest {
                commands,
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
            finish(host)?;
            Ok(
                json!({"revision": r.revision, "imported": r.resulting, "checks": outcomes})
                    .to_string(),
            )
        }
        Cmd::Render {
            path,
            output,
            codec,
            from,
            to,
            snapshot,
        } => {
            let codec = arc_media::encode::VideoCodec::parse(codec)
                .ok_or_else(|| fail("invalid_value", "codec must be ffv1, h264, av1, or prores"))?;
            let project = match snapshot {
                Some(s) => {
                    let bytes = std::fs::read(s).map_err(|e| fail("io_error", e))?;
                    arc_core::store::parse_document(&bytes)
                        .map_err(|e| fail("invalid_document", e))?
                }
                None => {
                    arc_core::store::load_container(path)
                        .map_err(|e| fail("io_error", e))?
                        .0
                }
            };
            let rate = project
                .sequence()
                .map(|s| s.frame_rate())
                .ok_or_else(|| fail("invalid_document", "no sequence"))?;
            let frame = |tc: &Option<String>| -> Result<Option<i64>, Fail> {
                tc.as_ref()
                    .map(|t| {
                        rate.parse_timecode(t)
                            .map_err(|e| Fail::from(CommandError::from(e)))
                    })
                    .transpose()
            };
            let spec = arc_cli::export::ExportSpec {
                output: output.clone(),
                codec,
                from: frame(from)?,
                to: frame(to)?,
                media_root: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            };
            let report = arc_cli::export::export(&project, &spec, |done, total| {
                if done == total || done % 30 == 0 {
                    eprintln!(
                        "{}",
                        json!({"progress": {"frames_done": done, "frames_total": total}})
                    );
                }
            })
            .map_err(|e| fail("render_failed", e))?;
            let report_json = serde_json::to_value(&report).unwrap_or_default();
            let mut rp = output.clone().into_os_string();
            rp.push(".report.json");
            std::fs::write(
                &rp,
                serde_json::to_vec_pretty(&report_json).unwrap_or_default(),
            )
            .map_err(|e| fail("io_error", e))?;
            Ok(report_json.to_string())
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
    if let Cmd::Rpc { client } = &cli.cmd {
        let stdin = std::io::stdin();
        arc_cli::rpc::serve(stdin.lock(), std::io::stdout(), client);
        return ExitCode::SUCCESS;
    }
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

