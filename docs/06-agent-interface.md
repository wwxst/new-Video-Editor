# 06 — Agent Interface

This document specifies how an AI agent edits video in Arcline with the same authority as a human, and why
that is safe. It is the product differentiator; the rest of the architecture exists to make it possible.

v2 changes: the trust model is local and unauthenticated (single-user process, path allow-lists), the live
GUI-attach topology is deferred, transport is stdio only, and the tool surface is a compact 20 tools. The
command model, revisions, dry-run, typed errors, and image inspection — the parts that make agents effective
— are unchanged.

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

| Surface | Purpose | Transport |
|---|---|---|
| **JSON-RPC 2.0** | Sessions, commands, jobs | stdio (NDJSON) in v1 |
| **MCP server** (`arcline-cli mcp`) | Standard agent integration | stdio |
| **CLI** (`arcline-cli`) | Stateless operations: `probe`, `render`, `convert`, `mcp` | process |

All three are views over the same command engine. The CLI's `render` path uses the same `arc-render`
compositor as the GUI, so headless output cannot diverge from preview.

**Deferred:** attaching to a live GUI session via a control socket. Agents operate on project files. This
removes an IPC surface and a trust model from v1; the protocol is designed so the topology can be added
without changing semantics ([§9](#9-future-topologies)).

## 3. Session model

```
project.open {path, create?} → {session, protocol, schema, renderer, revision, summary}
```

- One process, one open project in v1. The project file is locked (`project.lock` with PID + stale
  detection) so two processes cannot write the same file.
- Sessions are cheap: opening a project with read-only intent is the same call; the client simply does not
  send writes.
- Long-lived sessions are normal: the process keeps the document in memory and applies command sequences
  with optimistic concurrency.

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
      {"op": "effect.add", "clip": "cl_…", "effect": "core.color.correct"}
    ],
    "comment": "split at the eye-line change, then grade"
  }
}
```

Semantics ([01 §6](01-system-architecture.md)): atomic batches, whole-batch validation, dry-run,
idempotency keys, one revision increment per commit.

**Read side.** `project.describe` returns a compact summary, not the full document:

```json
{
  "revision": 88,
  "sequence": {"name": "Main", "duration": "00:01:30:12", "timebase": "30000/1001",
               "resolution": [1920, 1080]},
  "tracks": [{"id": "tr_V1", "kind": "video", "clips": 3},
             {"id": "tr_A1", "kind": "audio", "clips": 2}],
  "clips": [{"id": "cl_…", "track": "tr_V1", "asset": "interview_a.mov",
             "start": "00:00:00:00", "end": "00:00:12:14", "source_in": "00:00:04:02",
             "effects": ["core.color.correct"], "enabled": true}],
  "issues": [], "missing": []
}
```

Timecodes are `HH:MM:SS:FF` at the project timebase for legibility; exact rationals are available with
`exact: true`. Timecodes render rationals — they never replace them.

## 5. Text timeline (read view)

LLMs reason better over linear text than nested JSON. `inspect.timeline` returns a dense EDL-like view:

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

- **Read-only in v1.** `timeline.apply_text` (diff a rewritten timeline into a command batch) is deferred
  to v1.1. Writes go through JSON commands, which are typed, validated, and unambiguous.
- **Stable and parseable.** One clip per line, fixed field order, quoting for spaces. The grammar ships in
  [schema/timeline-text.abnf](schema/timeline-text.abnf).
- **Partial reads.** `inspect.timeline` accepts a time range so a 2,000-clip project is consumable in
  chunks.
- **Lossless for structure, lossy for depth.** Full parameter and keyframe detail stays in JSON
  (`inspect.clip <id> --json`).

Both `inspect.timeline` and `inspect.clip` are MCP resources so an agent can re-read state after edits.

## 6. MCP surface

`arcline-cli mcp --project <path>` speaks MCP over stdio, wrapping the JSON-RPC session.

### 6.1 Tools (20)

| Group | Tools |
|---|---|
| Project | `project_open` (creates if missing), `project_describe`, `project_save` |
| Media | `media_import` (returns probe summary) |
| Edit | `timeline_add_clip`, `timeline_move_clip`, `timeline_trim_clip`, `timeline_split_clip`, `timeline_remove_clip`, `timeline_add_transition` |
| Effects | `effect_list` (returns manifests), `effect_add`, `effect_set_params` |
| Audio | `audio_set_gain` |
| Text | `title_add`, `subtitle_import` |
| Inspect | `inspect_timeline`, `inspect_frame` |
| Render | `render_submit`, `render_status` |

Design notes:

- **One tool per intent**, narrow parameters, no flag strings. Schemas are generated from the Rust command
  types so tool docs cannot drift from behavior.
- `inspect_frame` returns an image content block (PNG, ≤ 1600 px) **of the composited output**, not the raw
  source frame. This is how an agent checks its grade, transition, or title placement.
- `effect_list` returns manifests, so agents discover parameters and ranges instead of guessing.
- Every mutating tool accepts `dry_run` and returns the diff it would make.
- The full command set remains available through the JSON-RPC `project.apply` method (batch, undo/redo,
  relinking, markers). MCP exposes the common path; power users drop to JSON-RPC rather than growing the
  tool surface.

### 6.2 Resources

```
arcline://project/timeline.txt    text projection (re-readable after each edit)
arcline://project/document.json   full canonical document (large; use sparingly)
arcline://project/schema.json     JSON schema for commands
arcline://effects/catalog.json    installed effect manifests
arcline://media/{asset}/wave.png  waveform image
```

## 7. Jobs

Rendering is a job, decoupled from command calls:

```
render_submit {sequence, range, preset, output} → {job_id}
render_status {job_id} → {state: queued|running|done|failed|cancelled,
                          progress: {frames_done, frames_total},
                          report: {…} | null, error?}
```

Jobs survive client disconnect because they belong to the process, not the connection. Cancellation is
cooperative and checked per frame. Results include `export_report.json` inline so the agent can verify
rather than trust.

## 8. Trust and safety (v1)

| Concern | Control |
|---|---|
| File access | `--root` media allow-list and `--output-root` for renders; canonicalized paths; symlink escapes rejected before syscalls |
| Untrusted media | decode panics caught per clip; no shell interpolation of paths (`Command` args only) |
| Resource abuse | per-session caps: batch size, concurrent jobs, total render minutes (configurable) |
| Audit | every applied batch logged with timestamp, command types, and diff hash; queryable via `history.list` |
| Destructive writes | `project.save` writes atomically; an optional `--snapshot-before-save` keeps the previous container |

No authentication, capability tokens, or multi-tenant isolation: the process runs as the user, locally.
Remote transports and tokens are added with the remote topology, and the threat model grows with them.

## 9. Future topologies

When a live-attach or remote mode is added (v1.1+), the protocol does not change; only transport and
authorization change:

1. Local named pipe to a running GUI: a per-user socket path in a discovery file.
2. Remote WebSocket: TLS required for non-loopback binds; a scoped capability token per session.
3. Multi-session daemon: sessions become addressable and isolated.

The design deliberately keeps these additive. Nothing in the command semantics depends on how the client
connected.

## 10. Worked example: agent produces a 45-second cut

```
1  project_create {name:"Launch Teaser", res:[1920,1080], timebase:"30000/1001"}
2  media_import {files:["a_roll.mov","b_roll.mov","music.flac"]}
   → [{asset:"as_a", duration:"00:03:12:00"}, {asset:"as_b", …}, {asset:"as_m", …}]
3  project_describe → assets, durations, fps, issues
4  inspect_frame {asset:"as_b", at:"00:00:41:02"} → image  (the agent looks at the footage)
5  edit_apply {base_revision:0, commands:[
     {op:"track.add", kind:"video", name:"A-Roll"},
     {op:"track.add", kind:"video", name:"B-Roll"},
     {op:"track.add", kind:"audio", name:"Music"},
     {op:"timeline.add_clip", track:"tr_V1", asset:"as_a",
        start:T(0), source_in:T(12.5), duration:T(18)},
     {op:"timeline.add_clip", track:"tr_V2", asset:"as_b",
        start:T(18), source_in:T(41), duration:T(6)},
     {op:"timeline.add_clip", track:"tr_A1", asset:"as_m",
        start:T(0), duration:T(45), audio:{gain_db:-9, fade_out:T(2)}},
   ]}
6  timeline_add_transition {a:"cl_1", b:"cl_2", kind:"cross_dissolve", duration:T(0.5)}
7  effect_add {clip:"cl_1", effect:"core.color.correct"}
   effect_set_params {clip:"cl_1", effect:"fx_1", params:{exposure:0.2, contrast:1.08}}
8  inspect_frame {clip:"cl_1", at_local:T(3)} → image   (verify the grade)
9  render_submit {preset:"web_1080p", output:"launch_teaser.mp4"}
10 render_status → done; report {frames:1350, encoder:"nvenc", …}
11 project_describe → confirm final state; project_save
```

Every step is typed, inspectable, and reversible. Step 8 is the essential loop: the agent verifies visually
instead of assuming success.

## 11. Versioning

- Protocol `1.0` freezes at Phase F. Additive changes bump the minor; unknown fields are ignored; unknown
  methods return `unsupported_method` with the protocol version.
- Tool schemas are generated; a schema diff is a breaking change requiring a one-release deprecation note.
- The text timeline format carries an explicit version line (`v1`) and remains backward-compatible.
