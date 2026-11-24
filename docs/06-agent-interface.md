# 06 — Agent Interface

This document specifies how an AI agent edits video in Arcline with the same authority as a human, and why
that is safe. It is the product differentiator; the rest of the architecture exists to make it possible.

v2 scope: local trust (single-user process, path allow-lists), stdio transport only, no live GUI attach,
13 JSON-RPC methods, and 21 MCP tools. The command model, revisions, dry-run, typed errors, and rendered-frame
inspection — the parts that make agents effective — are unchanged.

This document is the **single normative definition** of the JSON-RPC methods (§4.1) and the MCP tools
(§6.1). Other documents reference it.

## 1. Principles

1. **Parity, not a wrapper.** Every GUI action maps to a command; every command is available to agents.
   If a feature cannot be expressed as a command, it is not finished.
2. **Intent, not pixels.** Agents address clips, tracks, and parameters by ID and value.
3. **Read like a human, write like a compiler.** A compact text view for reasoning; typed JSON for writes.
4. **Verify, don't hope.** Writes can be dry-run and diffed; renders produce a report; frames can be
   inspected mid-edit.
5. **Fail loudly and specifically.** Stable error `kind` strings, no prose parsing.
6. **Local trust.** v1 assumes a single local user; isolation machinery arrives with remote access, not
   before.

## 2. Surfaces

| Surface | Entry point | Transport |
|---|---|---|
| **JSON-RPC 2.0** | `arcline-cli rpc [--project <path>]` | stdio, newline-delimited JSON |
| **MCP server** | `arcline-cli mcp [--project <path>]` | stdio (MCP) |
| **CLI** | `arcline-cli new \| describe \| apply \| probe \| render \| diag` | process (one-shot) |

All three are views over the same command engine. `arcline-cli render` uses the same `arc-render`
compositor as the GUI, and the GUI itself exports by spawning `arcline-cli render`
([04 §1.2](04-media-pipeline.md)), so headless output cannot diverge from preview.

**Deferred (post-1.0, unscheduled):** attaching to a live GUI session via a control socket. Agents operate
on project files ([§9](#9-future-topologies)).

## 3. Session model

- One process, one open project, one session in v1. `session.lock` in the project's sidecar directory
  (PID + start time, stale detection) prevents two processes from writing the same project
  ([08 §4.1](08-performance-reliability.md)).
- Long-lived sessions are normal: the process keeps the document in memory and applies command sequences
  with optimistic concurrency (`base_revision`).
- Read-only use is the same session; the client simply sends no writes.

## 4. JSON-RPC API

### 4.1 Methods (13)

All time values follow [02 §2.1/§2.3](02-domain-model-and-project-format.md): exact rational
`{"num","den"}` seconds, or a timecode at the sequence frame rate — NDF `"HH:MM:SS:FF"`, or DF
`"HH:MM:SS;FF"` at 29.97/59.94. Arcline always emits NDF.

| Method | Params | Result |
|---|---|---|
| `project.open` | `path`, `create?: {name, resolution: [w,h], frame_rate: "30000/1001", sample_rate?: 48000}`, `durable?: bool` | `session`, `protocol`, `schema`, `renderer`, `revision`, `summary` (= `project.describe`), `recovered?` |
| `project.describe` | `session`, `exact?: bool` | compact summary (§4.3) |
| `project.apply` | `session`, `base_revision`, `commands[]`, `dry_run?`, `idempotency_key?`, `comment?` | `revision`, `applied`, `resulting[]`, `warnings[]`, `diff?` |
| `project.save` | `session`, `path?` | `path`, `revision` |
| `history.undo` | `session`, `base_revision?` | `revision`, `undone` (batch summary) |
| `history.redo` | `session`, `base_revision?` | `revision`, `redone` |
| `history.list` | `session`, `limit?` | audit entries: `revision`, time, command types, comment, diff hash |
| `effect.list` | `session` | effect manifests ([02 §3.4](02-domain-model-and-project-format.md)) |
| `inspect.timeline` | `session`, `range?: {from, to}` | text view (§5) |
| `inspect.frame` | `session`, one of `at` (sequence time) · `clip` + `at_local` · `asset` + `at`, `max_px?: 1600` | PNG (base64) of the **composited** output (source frame for `asset`) |
| `job.submit` | `session`, `kind: "render" \| "interchange"`, `spec` (render: `{range?, preset, output}`; interchange: [12 §9](12-project-interchange.md)) | `job_id` |
| `job.status` | `job_id` | `state` (`queued \| running \| done \| failed \| cancelled`), `progress`, `report?` (`export_report.json` or `interchange_report.json`), `error?` |
| `job.cancel` | `job_id` | `state` |

Rules:

- **Edits only through `project.apply`.** Save, undo/redo, inspection, and jobs are methods, not commands
  ([02 §4](02-domain-model-and-project-format.md)).
- **Commit and acknowledgement.** A `project.apply` (or undo/redo) response is sent only after the batch's
  journal record has been **completely written** (and, with `durable: true`, `fsync`ed). If the write fails,
  the batch is rolled back and the response is `journal_unavailable` ([08 §4.3–4.4](08-performance-reliability.md)).
- **File-referencing commands** (`asset.import`, `subtitle.import`, LUT assets) are resolved by the session
  layer before the engine sees them: paths are checked against `--root`, media runs the subprocess import
  check and probe ([04 §1.2](04-media-pipeline.md)), subtitle files are parsed. The engine thread never does
  file I/O. The per-asset outcome (`ok`, `quarantined`, `offline`) is returned in `resulting`.
- **Errors** are `{code, kind, message, path?, details?}` with stable `kind` strings, including
  `stale_revision` (with the current revision), `off_frame_boundary`, `invalid_timecode`,
  `drop_frame_not_allowed`, `unsupported_frame_rate`, `inexact_frame_rate`, `overlap`, `unknown_clip`,
  `duplicate_id`, `path_denied`, `media_quarantined`, `journal_unavailable`, `unsupported_method`.
- Batch semantics (atomicity, whole-batch validation, dry-run, idempotency) are in
  [01 §6.2](01-system-architecture.md).

### 4.2 Example request

```json
{
  "jsonrpc": "2.0", "id": "1", "method": "project.apply",
  "params": {
    "session": "s_01J9…",
    "base_revision": 88,
    "commands": [
      {"op": "timeline.split_clip", "clip": "cl_a91f", "at": "00:00:04:05"},
      {"op": "effect.add", "id": "fx_grade", "clip": "cl_a91f", "effect": "core.color.correct"}
    ],
    "comment": "split at the eye-line change, then grade"
  }
}
```

### 4.3 Read side

`project.describe` returns a compact summary, not the full document:

```json
{
  "revision": 88,
  "sequence": {"name": "Main", "duration": "00:01:30:12", "frame_rate": "30000/1001",
               "resolution": [1920, 1080]},
  "tracks": [{"id": "tr_v1", "kind": "video", "clips": 3},
             {"id": "tr_a1", "kind": "audio", "clips": 2}],
  "clips": [{"id": "cl_a91f", "track": "tr_v1", "asset": "interview_a.mov",
             "start": "00:00:00:00", "end": "00:00:12:14", "source_in": "00:00:04:02",
             "effects": ["core.color.correct"], "enabled": true}],
  "assets": [{"id": "as_a", "path": "interview_a.mov", "status": "ok",
              "duration": "00:03:12:00", "frame_rate": "30000/1001", "vfr": false}],
  "issues": [], "missing": []
}
```

Timecodes are NDF `HH:MM:SS:FF` at the sequence frame rate ([02 §2.3](02-domain-model-and-project-format.md));
`exact: true` adds rationals. Timecodes render
rationals — they never replace them.

## 5. Text timeline (read view)

LLMs reason better over linear text than nested JSON. `inspect.timeline` returns a dense EDL-like view
([grammar](schema/timeline-text.abnf)):

```
# arcline timeline v1
SEQ "Main" 1920x1080 fps=30000/1001 dur=00:01:30:12
TRK V1 name="A-Roll" op=1.0
  [cl_a91f] "interview_a.mov" in=00:00:04:02 len=00:00:12:14 @00:00:00:00 fx=core.color.correct(exposure=0.15,contrast=1.1)
  [cl_b00e] "broll_city.mov" in=00:00:00:00 len=00:00:04:00 @00:00:12:14
TRK V2 name="Titles"
  [cl_c41d] << TITLE "Chapter One" >> in=00:00:00:00 len=00:00:05:00 @00:00:02:00 style=lower_third
TRK A1 name="Music" gain=-6dB
  [cl_d7e2] "music_main.wav" in=00:00:00:00 len=00:01:30:00 @00:00:00:00 fade=in:500ms,out:2s
MARK
  00:00:12:14 "cut point"
```

- **Read-only in v1.** Writes go through JSON commands. Round-trip text editing (`timeline.apply_text`) is
  post-1.0 ([ADR-0010](adr/0010-text-timeline-for-agents.md)).
- **Stable and parseable.** One clip per line, fixed field order, quoted names. `fps=` is the frame rate.
- **Partial reads.** `range` makes a 2,000-clip project consumable in chunks.
- **Lossless for structure, lossy for depth.** Full parameters and keyframes stay in the JSON document
  resource.

## 6. MCP surface

`arcline-cli mcp [--project <path>]` speaks MCP over stdio and wraps one JSON-RPC session. The facade
injects `session`; for single-intent tools `base_revision` is optional and defaults to the current revision
(the process has one client). `edit_apply` accepts an explicit `base_revision`.

### 6.1 Tools (21) and their JSON-RPC mapping

| # | Tool | JSON-RPC | Notes |
|---|---|---|---|
| 1 | `project_open` | `project.open` | `create` makes a new project; `durable` optional |
| 2 | `project_describe` | `project.describe` | |
| 3 | `project_save` | `project.save` | |
| 4 | `media_import` | `project.apply` → `asset.import` ×N | `files: [{path, id?}]`; returns per-asset `ok \| quarantined \| offline` |
| 5 | `edit_apply` | `project.apply` | any command batch from [02 §4](02-domain-model-and-project-format.md): tracks, audio, subtitles, markers, transforms… |
| 6 | `timeline_add_clip` | `project.apply` → `timeline.add_clip` | |
| 7 | `timeline_move_clip` | `project.apply` → `timeline.move_clip` | |
| 8 | `timeline_trim_clip` | `project.apply` → `timeline.trim_clip` | `ripple` explicit |
| 9 | `timeline_split_clip` | `project.apply` → `timeline.split_clip` | |
| 10 | `timeline_remove_clip` | `project.apply` → `timeline.remove_clip` | |
| 11 | `timeline_add_transition` | `project.apply` → `timeline.add_transition` | adjacent clips on the same track |
| 12 | `effect_list` | `effect.list` | manifests: parameters, ranges, defaults |
| 13 | `effect_add` | `project.apply` → `effect.add` | |
| 14 | `effect_set_params` | `project.apply` → `effect.set_params` | |
| 15 | `title_add` | `project.apply` → `title.add` | title is a clip on a video track |
| 16 | `inspect_timeline` | `inspect.timeline` | text view |
| 17 | `inspect_frame` | `inspect.frame` | image content block |
| 18 | `history_undo` | `history.undo` | |
| 19 | `render_submit` | `job.submit` (`kind: "render"`) | |
| 20 | `render_status` | `job.status` | status of any job; includes the export or interchange report when done |
| 21 | `interchange_export` | `job.submit` (`kind: "interchange"`) | FCPXML / FCP7 XML / OTIO + package + compatibility report ([12](12-project-interchange.md)) |

Design notes:

- Every mutating tool (4–11, 13–15) accepts `dry_run` and returns the diff it would make.
- Schemas are generated from the Rust command types, so tool docs cannot drift from behavior.
- Anything not covered by a dedicated tool is reachable through `edit_apply` (e.g. `track.add`,
  `audio.set_gain`, `audio.set_fades`, `subtitle.import`). Methods without a tool (`history.redo`,
  `history.list`, `job.cancel`) are available over JSON-RPC. The tool surface is kept deliberately small.

### 6.2 Resources

```
arcline://project/timeline.txt      text view (same as inspect.timeline, full range)
arcline://project/document.json     canonical document (large; use sparingly)
arcline://project/schema.json       JSON schema for commands
arcline://effects/catalog.json      effect manifests (same as effect.list)
```

## 7. Jobs

Rendering is a job, decoupled from edits. `job.submit` snapshots the document at the current revision and
spawns `arcline-cli render` as a child process, so later edits do not affect a running render and a render
crash cannot end the session. Jobs belong to the process, not the connection; cancellation is checked per
frame; `job.status` returns `export_report.json` inline when done, so the agent verifies rather than trusts.

Interchange exports (`kind: "interchange"`) run the same way in an `arcline-cli export` child process and
return `interchange_report.json`; an agent should read its `unverified`, `baked`, and `annotated` items before
telling a user that a project is ready to open in another editor ([12 §6](12-project-interchange.md)).

## 8. Trust and safety (v1)

| Concern | Control |
|---|---|
| File access | `--root` media allow-list and `--output-root` for renders; canonicalized paths; symlink escapes rejected before any syscall |
| Untrusted media | subprocess import check, quarantine, crash breadcrumbs, out-of-process render ([04 §1.2](04-media-pipeline.md)); child processes get argument vectors, never shell strings |
| Resource abuse | per-session caps: batch size, concurrent jobs, total render minutes (configurable) |
| Audit | every committed batch is journaled and listed by `history.list` |
| Data safety | acknowledged batches survive process crashes; `durable: true` extends this to power loss ([08 §4.7](08-performance-reliability.md)) |

No authentication, capability tokens, or multi-tenant isolation: the process runs as the user, locally.
Remote transports and tokens arrive with the remote topology, and the threat model grows with them.

## 9. Future topologies

Post-1.0 and unscheduled. When added, the protocol does not change; only transport and authorization do:

1. Local named pipe to a running GUI: per-user socket path in a discovery file.
2. Remote WebSocket: TLS required for non-loopback binds; a scoped capability token per session.
3. Multi-session daemon: sessions become addressable and isolated.

## 10. Worked example: agent produces a 24-second cut

Sequence at `frame_rate` 30000/1001; all times are timecodes at that rate. Agent-chosen IDs are used so
one batch can create and reference entities ([02 §4](02-domain-model-and-project-format.md)).

```
1  project_open {path:"launch_teaser.arcline",
                 create:{name:"Launch Teaser", resolution:[1920,1080], frame_rate:"30000/1001"}}
   → revision 0
2  media_import {files:[{path:"a_roll.mov", id:"as_a"}, {path:"b_roll.mov", id:"as_b"},
                        {path:"music.flac", id:"as_m"}]}
   → revision 1; resulting: [{id:"as_a", status:"ok", duration:"00:03:12:00", frame_rate:"30000/1001"}, …]
3  inspect_frame {asset:"as_b", at:"00:00:41:00"} → image          (look at the source footage)
4  edit_apply {base_revision:1, commands:[
     {op:"track.add", id:"tr_v1", kind:"video", name:"Main"},
     {op:"track.add", id:"tr_v2", kind:"video", name:"Titles"},
     {op:"track.add", id:"tr_a1", kind:"audio", name:"Music"},
     {op:"timeline.add_clip", id:"cl_a", track:"tr_v1", asset:"as_a",
        start:"00:00:00:00", source_in:"00:00:12:15", duration:"00:00:18:00"},
     {op:"timeline.add_clip", id:"cl_b", track:"tr_v1", asset:"as_b",
        start:"00:00:18:00", source_in:"00:00:41:00", duration:"00:00:06:00"},
     {op:"timeline.add_clip", id:"cl_m", track:"tr_a1", asset:"as_m",
        start:"00:00:00:00", source_in:"00:00:00:00", duration:"00:00:24:00"},
     {op:"audio.set_gain",  clip:"cl_m", gain_db:-9},
     {op:"audio.set_fades", clip:"cl_m", fade_out:{num:2, den:1}}
   ]}
   → revision 2
5  timeline_add_transition {a:"cl_a", b:"cl_b", kind:"cross_dissolve", duration:"00:00:00:15"}
   → revision 3                                                        (same track, adjacent)
6  effect_add {clip:"cl_a", id:"fx_grade", effect:"core.color.correct"} → revision 4
7  effect_set_params {clip:"cl_a", effect:"fx_grade", params:{exposure:0.2, contrast:1.08},
                      dry_run:true} → diff
   effect_set_params {…same, dry_run:false} → revision 5
8  inspect_frame {clip:"cl_a", at_local:"00:00:03:00"} → composited image   (verify the grade)
9  title_add {id:"cl_t", track:"tr_v2", text:"Launch Day", style:"lower_third",
              start:"00:00:02:00", duration:"00:00:04:00"} → revision 6
10 render_submit {preset:"web_1080p", output:"launch_teaser.mp4"} → {job_id:"j_1"}
11 render_status {job_id:"j_1"} → running … done; report {frames:720, encoder:"h264_nvenc", …}
12 project_describe → confirm final state; project_save
```

Every step is typed, inspectable, and reversible (`history_undo`). Step 8 is the essential loop: the agent
verifies visually instead of assuming success. Step 11's frame count is exact: 540 + 180 frames; the
transition uses source handles and does not change the sequence length.

## 11. Versioning

- Protocol `1.0` freezes at Phase F. Additive changes bump the minor; unknown fields are ignored; unknown
  methods return `unsupported_method` with the protocol version.
- Tool schemas are generated; a schema diff is a breaking change requiring a one-release deprecation note.
- The text timeline format carries an explicit version line (`v1`) and remains backward-compatible.
