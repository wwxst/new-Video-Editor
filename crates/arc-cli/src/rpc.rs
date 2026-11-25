//! JSON-RPC 2.0 session over stdio (docs/06 §4). Reference implementation of the core methods;
//! the remaining methods of docs/06 §4.1 (control, inspect.frame, notifications) follow the same
//! dispatch pattern.
//!
//! One process = one session = one open project. Requests and responses are single JSON lines.

use crate::inspect;
use arc_core::engine::{ApplyRequest, Source};
use arc_core::journal::Durability;
use arc_core::model::{Project, Resolution};
use arc_core::runtime::{EngineHost, HostError};
use arc_core::{Command, CommandError, ErrorKind, FrameRate, Time};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Child, Stdio};

pub const PROTOCOL: &str = "1.0";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const DOMAIN_ERROR: i64 = -32000;

struct RpcError {
    code: i64,
    kind: String,
    message: String,
    data: Option<Value>,
}

impl From<CommandError> for RpcError {
    fn from(e: CommandError) -> Self {
        RpcError {
            code: DOMAIN_ERROR,
            kind: e.kind.as_str(),
            message: e.message.clone(),
            data: serde_json::to_value(&e).ok(),
        }
    }
}

fn err(code: i64, kind: &str, message: impl Into<String>) -> RpcError {
    RpcError {
        code,
        kind: kind.into(),
        message: message.into(),
        data: None,
    }
}

fn params<T: serde::de::DeserializeOwned>(p: &Value) -> Result<T, RpcError> {
    serde_json::from_value(p.clone())
        .map_err(|e| err(INVALID_PARAMS, "invalid_params", e.to_string()))
}

struct Job {
    child: Child,
    output: PathBuf,
    snapshot: PathBuf,
    result: Option<Value>,
}

/// A session: the open project plus running jobs.
pub struct Session {
    host: Option<EngineHost>,
    path: Option<PathBuf>,
    client: String,
    jobs: BTreeMap<String, Job>,
    next_job: u64,
    exe: PathBuf,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("path", &self.path)
            .field("jobs", &self.jobs.len())
            .finish()
    }
}

impl Session {
    pub fn new(client: &str) -> Session {
        Session {
            host: None,
            path: None,
            client: client.into(),
            jobs: BTreeMap::new(),
            next_job: 1,
            exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("arcline-cli")),
        }
    }

    fn host(&self) -> Result<&EngineHost, RpcError> {
        self.host
            .as_ref()
            .ok_or_else(|| err(DOMAIN_ERROR, "no_project", "call project.open first"))
    }

    fn source(&self) -> Option<Source> {
        Some(Source::Client(self.client.clone()))
    }

    fn handle(&mut self, method: &str, p: &Value) -> Result<Value, RpcError> {
        match method {
            "project.open" => self.open(p),
            "project.describe" => {
                let exact = p.get("exact").and_then(Value::as_bool).unwrap_or(false);
                let pubd = self.host()?.published();
                Ok(inspect::describe(&pubd.project, pubd.revision, exact))
            }
            "project.apply" => {
                #[derive(serde::Deserialize)]
                struct P {
                    #[serde(default)]
                    base_revision: Option<u64>,
                    commands: Vec<Command>,
                    #[serde(default)]
                    dry_run: bool,
                    #[serde(default)]
                    idempotency_key: Option<String>,
                    #[serde(default)]
                    comment: Option<String>,
                }
                let a: P = params(p)?;
                let project_path = self
                    .path
                    .clone()
                    .ok_or_else(|| err(DOMAIN_ERROR, "no_project", "call project.open first"))?;
                let mut commands = a.commands;
                crate::media::resolve_imports(&project_path, &mut commands)?;
                let r = self.host()?.apply(ApplyRequest {
                    base_revision: a.base_revision,
                    commands,
                    dry_run: a.dry_run,
                    idempotency_key: a.idempotency_key,
                    source: self.source(),
                    comment: a.comment,
                })?;
                Ok(serde_json::to_value(r).unwrap_or_default())
            }
            "project.save" => {
                let rev = self.host()?.save()?;
                Ok(json!({"path": self.path, "revision": rev}))
            }
            "history.undo" | "history.redo" => {
                let host = self.host()?;
                if let Some(base) = p.get("base_revision").and_then(Value::as_u64) {
                    let cur = host.published().revision;
                    if base != cur {
                        return Err(CommandError::new(
                            ErrorKind::StaleRevision,
                            format!("base_revision {base} is not the current revision {cur}"),
                        )
                        .with_details(json!({"current_revision": cur}))
                        .into());
                    }
                }
                let src = Source::Client(self.client.clone());
                let r = if method == "history.undo" {
                    host.undo(src)?
                } else {
                    host.redo(src)?
                };
                Ok(serde_json::to_value(r).unwrap_or_default())
            }
            "inspect.timeline" => {
                let pubd = self.host()?.published();
                let rate = pubd
                    .project
                    .sequence()
                    .map(|s| s.frame_rate())
                    .unwrap_or(FrameRate::Fps30);
                let range = match p.get("range") {
                    Some(r) => {
                        let tc = |k: &str| -> Result<Time, RpcError> {
                            let s = r.get(k).and_then(Value::as_str).ok_or_else(|| {
                                err(
                                    INVALID_PARAMS,
                                    "invalid_params",
                                    format!("range.{k} must be a timecode"),
                                )
                            })?;
                            rate.timecode_to_time(s)
                                .map_err(|e| RpcError::from(CommandError::from(e)))
                        };
                        Some((tc("from")?, tc("to")?))
                    }
                    None => None,
                };
                Ok(
                    json!({"text": inspect::timeline_text(&pubd.project, range), "revision": pubd.revision}),
                )
            }
            "effect.list" => Ok(
                json!({"effects": arc_render::builtin::manifests(), "gpu": arc_render::Renderer::supported_effects()}),
            ),
            "job.submit" => self.submit(p),
            "job.status" => self.status(p),
            "job.cancel" => {
                let id = p
                    .get("job_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let job = self
                    .jobs
                    .get_mut(&id)
                    .ok_or_else(|| err(DOMAIN_ERROR, "unknown_job", format!("unknown job {id}")))?;
                let _ = job.child.kill();
                Ok(json!({"job_id": id, "state": "cancelled"}))
            }
            other => Err(err(
                METHOD_NOT_FOUND,
                "unsupported_method",
                format!("unsupported method {other} (protocol {PROTOCOL})"),
            )),
        }
    }

    fn open(&mut self, p: &Value) -> Result<Value, RpcError> {
        #[derive(serde::Deserialize)]
        struct Create {
            #[serde(default = "untitled")]
            name: String,
            #[serde(default = "res")]
            resolution: [u32; 2],
            #[serde(default = "fps")]
            frame_rate: String,
            /// Replace an existing project file (default false; never replaces a project that is
            /// open in another session).
            #[serde(default)]
            overwrite: bool,
        }
        fn untitled() -> String {
            "Untitled".into()
        }
        fn res() -> [u32; 2] {
            [1920, 1080]
        }
        fn fps() -> String {
            "30".into()
        }
        #[derive(serde::Deserialize)]
        struct P {
            path: PathBuf,
            #[serde(default)]
            create: Option<Create>,
            #[serde(default)]
            durable: bool,
        }
        if self.host.is_some() {
            return Err(err(
                DOMAIN_ERROR,
                "already_open",
                "this session already has a project open",
            ));
        }
        let a: P = params(p)?;
        let d = if a.durable {
            Durability::Durable
        } else {
            Durability::Default
        };
        let cat = arc_render::builtin::catalog();
        let host = match a.create {
            Some(c) => {
                let rate = FrameRate::parse(&c.frame_rate)
                    .map_err(|e| RpcError::from(CommandError::from(e)))?;
                let project = Project::new(
                    &c.name,
                    rate,
                    Resolution {
                        w: c.resolution[0],
                        h: c.resolution[1],
                    },
                );
                EngineHost::create(&a.path, project, cat, d, c.overwrite)
            }
            None => EngineHost::open(&a.path, cat, d),
        }
        .map_err(|e| match e {
            HostError::Store(arc_core::store::StoreError::Locked(_)) => {
                err(DOMAIN_ERROR, "project_locked", e.to_string())
            }
            HostError::Store(arc_core::store::StoreError::Exists(_)) => {
                err(DOMAIN_ERROR, "project_exists", e.to_string())
            }
            HostError::Command(c) => RpcError::from(c),
            other => err(DOMAIN_ERROR, "io_error", other.to_string()),
        })?;
        let pubd = host.published();
        let out = json!({
            "session": "s_1",
            "protocol": PROTOCOL,
            "schema": arc_core::model::SCHEMA_VERSION,
            "renderer": env!("CARGO_PKG_VERSION"),
            "revision": pubd.revision,
            "summary": inspect::describe(&pubd.project, pubd.revision, false),
            "recovered": host.recovery().recovered.then(|| serde_json::to_value(host.recovery()).unwrap_or_default()),
        });
        self.path = Some(a.path);
        self.host = Some(host);
        Ok(out)
    }

    /// Render jobs run in a child `arcline-cli render` process from a snapshot (docs/06 §7).
    fn submit(&mut self, p: &Value) -> Result<Value, RpcError> {
        #[derive(serde::Deserialize)]
        struct Spec {
            output: PathBuf,
            #[serde(default = "h264")]
            codec: String,
            #[serde(default)]
            from: Option<String>,
            #[serde(default)]
            to: Option<String>,
        }
        fn h264() -> String {
            "h264".into()
        }
        #[derive(serde::Deserialize)]
        struct P {
            kind: String,
            spec: Spec,
        }
        let a: P = params(p)?;
        if a.kind != "render" {
            return Err(err(
                INVALID_PARAMS,
                "unsupported_job",
                format!("job kind {} is not available yet", a.kind),
            ));
        }
        let path = self
            .path
            .clone()
            .ok_or_else(|| err(DOMAIN_ERROR, "no_project", "call project.open first"))?;
        let pubd = self.host()?.published();
        let id = format!("j_{}", self.next_job);
        self.next_job += 1;
        let snapshot =
            std::env::temp_dir().join(format!("arcline-{}-{id}.json", std::process::id()));
        std::fs::write(&snapshot, pubd.project.canonical_json())
            .map_err(|e| err(DOMAIN_ERROR, "io_error", e.to_string()))?;
        let mut cmd = std::process::Command::new(&self.exe);
        cmd.arg("render")
            .arg(&path)
            .arg("-o")
            .arg(&a.spec.output)
            .args(["--codec", &a.spec.codec])
            .arg("--snapshot")
            .arg(&snapshot);
        if let Some(f) = &a.spec.from {
            cmd.args(["--from", f]);
        }
        if let Some(t) = &a.spec.to {
            cmd.args(["--to", t]);
        }
        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| err(DOMAIN_ERROR, "io_error", e.to_string()))?;
        self.jobs.insert(
            id.clone(),
            Job {
                child,
                output: a.spec.output,
                snapshot,
                result: None,
            },
        );
        Ok(json!({"job_id": id, "revision": pubd.revision}))
    }

    fn status(&mut self, p: &Value) -> Result<Value, RpcError> {
        let id = p
            .get("job_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let job = self
            .jobs
            .get_mut(&id)
            .ok_or_else(|| err(DOMAIN_ERROR, "unknown_job", format!("unknown job {id}")))?;
        if let Some(r) = &job.result {
            return Ok(r.clone());
        }
        match job.child.try_wait() {
            Ok(None) => Ok(json!({"job_id": id, "state": "running"})),
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut so) = job.child.stdout.take() {
                    let _ = std::io::Read::read_to_string(&mut so, &mut out);
                }
                let _ = std::fs::remove_file(&job.snapshot);
                let parsed: Value =
                    serde_json::from_str(out.trim()).unwrap_or(json!({"raw": out.trim()}));
                let r = if status.success() {
                    json!({"job_id": id, "state": "done", "output": job.output, "report": parsed})
                } else {
                    json!({"job_id": id, "state": "failed", "error": parsed.get("error").cloned().unwrap_or(parsed)})
                };
                job.result = Some(r.clone());
                Ok(r)
            }
            Err(e) => Err(err(DOMAIN_ERROR, "io_error", e.to_string())),
        }
    }

    pub fn close(mut self) {
        if let Some(h) = self.host.take() {
            h.shutdown();
        }
    }
}

fn response(id: Value, r: Result<Value, RpcError>) -> Value {
    match r {
        Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
        Err(e) => {
            let mut error = json!({"code": e.code, "message": e.message, "data": {"kind": e.kind}});
            if let Some(d) = e.data {
                error["data"] = d;
            }
            json!({"jsonrpc": "2.0", "id": id, "error": error})
        }
    }
}

/// Serve one session over the given line streams until EOF.
pub fn serve(input: impl BufRead, mut output: impl Write, client: &str) {
    let mut session = Session::new(client);
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(e) => response(
                Value::Null,
                Err(err(PARSE_ERROR, "parse_error", e.to_string())),
            ),
            Ok(req) => {
                let id = req.get("id").cloned().unwrap_or(Value::Null);
                match req.get("method").and_then(Value::as_str) {
                    None => response(
                        id,
                        Err(err(INVALID_REQUEST, "invalid_request", "missing method")),
                    ),
                    Some(m) => {
                        let p = req.get("params").cloned().unwrap_or(json!({}));
                        let r = session.handle(m, &p);
                        if req.get("id").is_none() {
                            continue; // notification: no response
                        }
                        response(id, r)
                    }
                }
            }
        };
        if writeln!(output, "{reply}")
            .and_then(|_| output.flush())
            .is_err()
        {
            break;
        }
    }
    session.close();
}
