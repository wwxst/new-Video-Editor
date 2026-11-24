# 06 — Agent Interface

This is the document the whole architecture exists to support. It specifies how an AI agent (or any
automation) edits video in Arcline with the same authority as a human, and why that is safe.

## 1. Principles

1. **Parity, not a wrapper.** Every GUI action maps to a command; every command is available to agents.
   If a feature cannot be expressed as a command, it is not finished.
2. **Intent, not pixels.** Agents address clips, tracks, and parameters by ID and value. They never
   "click" or send keystrokes.
3. **Read like a human, write like a compiler.** The document is exposed in a compact, stable text form
   for reading and reasoning; writes are typed and validated.
4. **Verify, don't hope.** Every write can be dry-run and diffed; every export produces a report; frames
   can be inspected mid-edit.
5. **Least privilege.** Capabilities are granted at session open; destructive and host-touching operations
   are opt-in.
6. **Fail loudly and specifically.** Stable error `kind` strings, no prose parsing.

## 2. Surfaces

| Surface | Purpose | Transport |
|---|---|---|
| **JSON-RPC 2.0** | Full access to sessions, commands, jobs | stdio, named pipe/unix socket, WebSocket |
| **MCP server** (`arcline mcp`) | Standard agent integration: tools, resources, prompts | stdio (spawned by the host) |
| **CLI** (`arcline`) | Scripted, stateless operations: `probe`, `render`, `convert`, `serve` | process |
| **Text timeline** | Read/write a human-style EDL projection | via RPC resource or file |

All four are thin views over the same command engine — none has a private path to state.

## 3. Session model

```
session.open {capabilities, root, project?} → {session, protocol, schema, renderer, revision}
```

- **One writer per session.** Multiple agent processes may connect; each gets its own session. Two sessions
  editing the same project file are prevented by a lock file (`project.lock` with PID/session and stale
  detection). Concurrent sessions on one *live* GUI session are allowed only as read-only inspectors.
- **Sessions are cheap.** An agent that wants to inspect without editing opens with `project:read` and a
  snapshot; no side effects.
- **Long-lived sessions are the norm.** The daemon keeps the document in memory; agents issue sequences of
  commands with optimistic concurrency (`base_revision`), retrying on `stale_revision` with the returned
  diff.

## 4. Command API

The command set is [02 §4](02-domain-model-and-project-format.md). Wire form:

```json
{
  "jsonrpc": "2.0", "id": "1", "method": "project.apply",
  "params": {
    "session": "s_…",
    "base_revision": 88,
    "commands": [
      {"op": "timeline.split_clip", "clip": "cl_…", "at": {"num": 125, "den": 30}},
      {"op": "effect.add", "clip": "cl_…", "effect": "core.color.correct", "after": "fx_…"}
    ],
    "comment": "split at the eye-line change, then grade"
  }
}
```

Batch rules ([01 §6](01-system-architecture.md)): atomic, validated as a whole, dry-runnable, idempotent by
key, one revision increment per committed batch.

**Read side** — `project.describe` returns a compact, stable summary (not the full document):

```json
{
  "revision": 88,
  "sequence": {"name": "Main", "duration": "00:01:30:12", "timebase": "30000/1001",
               "resolution": [1920, 1080]},
  "tracks": [
    {"id": "tr_V1", "kind": "video", "clips": 3},
    {"id": "tr_A1", "kind": "audio", "clips": 2}
  ],
  "clips": [
    {"id": "cl_…", "track": "tr_V1", "asset": "interview_a.mov",
     "start": "00:00:00:00", "end": "00:00:12:14", "source_in": "00:00:04:02",
     "effects": ["core.color.correct"], "enabled": true}
  ],
  "issues": [],
  "missing": []
}
```

Timecodes are `HH:MM:SS:FF` at project timebase for human/LLM legibility, with exact rationals available
under `--exact` or in the full document resource. Both map deterministically; the timecode is a rendering
of the rational, not a replacement.

## 5. Text timeline (the agent's "document view")

LLMs reason far better over linear text than over deeply nested JSON. `inspect.timeline` returns a
round-trippable EDL-like projection:

```
# arcline timeline v1
SEQ "Main" 1920x1080 30000/1001 dur=00:01:30:12
TRK V1 name="A-Roll" op=1.0
  [cl_a91f] interview_a.mov  in=00:00:04:02 len=00:00:12:14 @00:00:00:00 fx=color.correct(exposure=0.15,contrast=1.1)
  [cl_b00e] broll_city.mov    in=00:00:00:00 len=00:00:04:00  @00:00:12:14 fx=scale(1.2)
TRK V2 name="Titles"
  [cl_c41d] <<TITLE "Chapter One">> len=00:00:05:00 @00:00:02:00 style=lower_third
TRK A1 name="Music" gain=-6dB
  [cl_d7e2] music_main.wav in=00:00:00:00 len=00:01:30:00 @00:00:00:00 fade=in:0.5s,out:2s
MARK
  00:00:12:14 "cut point"
```

Rules for the projection:

- **Stable and parseable.** One clip per line, fixed field order, quoting for names with spaces.
  ABNF grammar ships in `docs/schema/timeline-text.abnf`; the parser is fuzzed.
- **Read is complete; write is a convenience layer.** `timeline.apply_text` diffs a submitted text against
  the current timeline and emits the equivalent command batch — it never bypasses validation. This lets an
  agent "rewrite the edit" as text and get a previewable diff back.
- **Lossless for structure, lossy for depth.** Full parameter/keyframe detail stays in JSON
  (`inspect.clip <id> --json`); the text view carries structure, timing, and top-level effect params.
- Both views are MCP **resources** so an agent can subscribe to changes.

## 6. MCP surface

`arcline mcp` speaks MCP over stdio, wrapping the same JSON-RPC session.

### 6.1 Tools

| Group | Tools |
|---|---|
| Project | `project_open`, `project_create`, `project_describe`, `project_save`, `project_export_report` |
| Assets | `media_import`, `media_probe`, `media_generate_proxy`, `media_relink` |
| Edit | `timeline_add_clip`, `timeline_move_clip`, `timeline_trim_clip`, `timeline_split_clip`, `timeline_remove_clip`, `timeline_set_speed`, `timeline_set_transform`, `timeline_add_transition` |
| Effects | `effect_list`, `effect_describe`, `effect_add`, `effect_set_params`, `effect_set_keyframes` |
| Audio | `audio_set_gain`, `audio_set_fades`, `audio_normalize`, `audio_set_track_mix` |
| Text | `title_add`, `subtitle_import`, `subtitle_from_transcript` |
| Inspect | `inspect_timeline`, `inspect_clip`, `inspect_frame` (image), `inspect_waveform` (image), `inspect_scopes` (image) |
| History | `history_undo`, `history_redo`, `history_list` |
| Batch | `edit_apply` (raw command batch, for power users), `edit_validate` (dry-run) |
| Jobs | `render_submit`, `render_status`, `render_cancel`, `job_list` |

Design notes:

- **One tool per intent**, narrow parameters, no `--flags` strings. Schemas are generated from the same
  Rust command types (single source of truth), so tool docs cannot drift from behavior.
- `inspect_frame` returns an image content block (PNG, ≤ 1600 px on the long edge, `quality` param). This is
  how an agent *sees* its work — after a grade, after a transition, after a title placement.
- `effect_list` / `effect_describe` return the manifest ([02 §3.4](02-domain-model-and-project-format.md)),
  so an agent discovers parameters and ranges instead of guessing.
- Every mutating tool accepts `dry_run` and returns the diff it would make.

### 6.2 Resources

```
arcline://session/info            capabilities, schema, renderer, revision
arcline://project/timeline.txt    text projection (subscribe-able)
arcline://project/document.json   full canonical document (large; use with care)
arcline://project/schema.json     JSON schema for commands
arcline://effects/catalog.json    all installed effect manifests
arcline://media/{asset}/wave.png  waveform image
```

### 6.3 Prompts (optional convenience)

`rough_cut(footage_dir, brief)`, `caption_video(language)`, `color_match(reference_frame)`. These are
scaffolding for agent hosts, not required for correctness, and are thin wrappers over the tools above.

## 7. Jobs and long-running work

Rendering, proxy generation, and analysis are **jobs**, decoupled from command calls:

```
render_submit {sequence, range, preset|explicit, output, strict_loudness?} → {job_id}
render_status {job_id} → {state: queued|running|done|failed|cancelled,
                          progress: {frames_done, frames_total, current_time},
                          report: {…} | null, error? }
```

- Progress is pushed over JSON-RPC notifications (`job.progress`) for WebSocket clients; MCP clients poll.
- Jobs survive client disconnect; they are owned by the daemon, not the connection.
- Cancellation is cooperative and immediate (segment workers check a shared flag per frame).
- Job results include the `export_report.json` content inline — the verification artifact ([04 §7](04-media-pipeline.md)).

## 8. Security model

| Concern | Control |
|---|---|
| File access | `--root` allow-list; paths canonicalized and checked pre-syscall; symlink escapes rejected |
| Capabilities | scopes at session open: `project:read/write`, `media:import`, `render:submit`, `plugin:load`, `fs:root=…` |
| Untrusted media | decoders sandboxed in subprocess with rlimits, no network, no writes ([04 §1](04-media-pipeline.md)) |
| Untrusted effects | WASM sandbox with explicit buffer access, no syscalls ([09](09-plugin-system.md)) |
| Token handling | 256-bit random, stored hashed, 24 h default expiry, per-session revocation |
| Audit | every applied batch logged with session, command, diff hash, timestamp; `history.audit` queryable |
| Resource abuse | per-session limits: max batch size, max render minutes/day, max concurrent jobs (configurable) |
| Prompt-injection blast radius | an agent's session can only do what its capabilities allow; render output paths constrained to a job output root |

The threat model is explicit: adversarial agents and adversarial media are both assumed. The design answer
is capability-scoped commands + sandboxed decoders + a journal that can reconstruct exactly what happened.

## 9. Worked example: agent produces a 45-second cut

```
1  project_create {name:"Launch Teaser", res:[1920,1080], timebase:"30000/1001"}
2  media_import {files:["a_roll.mov","b_roll.mov","music.flac","logo.png"]}
   → [{asset:"as_a", duration:"00:03:12:00", streams:[…]}, …]
3  media_probe {asset:"as_b"} → key scenes / duration / fps
4  inspect_frame {asset:"as_b", at:"00:00:41:02"} → image  (agent looks at the footage)
5  edit_apply {base_revision:0, commands:[
     {op:"track.add", kind:"video", name:"A-Roll"},
     {op:"track.add", kind:"video", name:"B-Roll"},
     {op:"track.add", kind:"audio", name:"Music"},
     {op:"timeline.add_clip", track:"tr_V1", asset:"as_a",
        start:T(0), source_in:T(12.5), duration:T(18)},
     {op:"timeline.add_clip", track:"tr_V2", asset:"as_b",
        start:T(18), source_in:T(41), duration:T(6)},
     {op:"timeline.add_clip", track:"tr_A1", asset:"as_music",
        start:T(0), duration:T(45), audio:{gain_db:-9, fade_out:T(2)}},
   ]}
6  timeline_add_transition {a:"cl_1", b:"cl_2", kind:"cross_dissolve", duration:T(0.5)}
7  effect_add {clip:"cl_1", effect:"core.color.correct"}
   effect_set_params {clip:"cl_1", effect:"fx_1", params:{exposure:0.2, contrast:1.08}}
8  inspect_frame {clip:"cl_1", at_local:T(3)} → image   (verify the grade)
9  subtitle_from_transcript {track:"tr_S1", transcript:"…"}   # if captions requested
10 render_submit {preset:"web_1080p", output:"launch_teaser.mp4", strict_loudness:true}
11 render_status → progress … done; report: {loudness:-14.2 LUFS, frames:1350, …}
12 edit_validate + project_describe → confirm final state; project_save
```

Every step is typed, inspectable, and reversible. Step 8 is the crucial one: the agent checks its work
visually instead of assuming success — the same loop a human editor runs.

## 10. What we deliberately do not expose

- **Raw decoder/encoder settings.** Presets are semantic (`fast`, `quality`, `master`); encoder strings are
  an implementation detail and change per platform.
- **Direct file writes inside the project container.** Saves go through the document, so the journal and
  checksums stay coherent.
- **Arbitrary plugin loading by default.** Agents may use installed effects; loading new native code is a
  human-gated capability.
- **Screen automation or UI scripting.** Not supported, by policy — the command API is the interface.

## 11. Versioning and compatibility

- Protocol `1.0` is frozen at P3; additive changes bump the minor. Unknown fields are ignored by both
  sides; unknown *commands* return `unsupported_op` with the protocol version in the error.
- Tool schemas are generated; a schema diff is a breaking change and requires a deprecation cycle of one
  minor release with `deprecated` notes in the tool description.
- The text timeline format has an explicit version line; the parser accepts `v1` forever.
