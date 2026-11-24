# HANDOVER — WIP checkpoint (Phase A in progress)

> **Status: work in progress. Nothing in `crates/` has been compiled or tested yet.**
> The workspace does not build in its current state (see §4). Treat every line of Rust here as a
> reviewed draft, not as working code.

Checkpoint date: 2026-11-25. Architecture: approved docs under `docs/` (start at `docs/01`, `docs/13`,
`docs/10`). This file covers implementation status only.

---

## 1. Progress at a glance

| Area | Roadmap phase | State |
|---|---|---|
| Architecture docs, ADRs 0001–0014 | — | Done, approved |
| Cargo workspace manifest | A | Written; lists six members, only `arc-core` exists → **does not build** |
| `arc-core` source (time, ids, model, effects, error, command, engine, journal, store) | A | **Drafted, never compiled** |
| `arc-core` `lib.rs` | A | **Missing** — crate cannot compile without it |
| `arc-core::runtime` (engine thread + journal thread) | A/E | **Not started** (design in §6.5) |
| `arc-core` tests | A | Unit tests drafted inside `time.rs`, `ids.rs`, `model.rs`; **no engine, journal, store, or property tests yet** |
| Embedding PoC | P | Not started |
| `arc-media`, `arc-render`, `arc-audio`, `arc-cli`, `arc-app` | B–J | Not started (no directories exist) |
| CI workflow (`.github/workflows`) | A | Not started |
| Handover docs for the 1-to-10 phase (development guide, interface contracts, acceptance criteria, implementation roadmap) | — | **Not started** — still owed from the original task |

## 2. Files in this checkpoint

```
Cargo.toml                          workspace: deps, lints, profiles
crates/arc-core/Cargo.toml
crates/arc-core/src/time.rs         Time, Ratio, FrameRate (8 rates), rounding, NDF/DF timecode, sample mapping + tests
crates/arc-core/src/ids.rs          prefixed ULID IDs (prj_/sq_/tr_/cl_/as_/fx_/tx_/mk_) + test
crates/arc-core/src/model.rs        Project/Sequence/Track/Clip/Asset/Param/... on im::Vector/OrdMap; canonical JSON + BLAKE3 hash + tests
crates/arc-core/src/effects.rs      EffectManifest data + EffectCatalog trait + StaticCatalog
crates/arc-core/src/error.rs        CommandError with stable snake_case ErrorKind
crates/arc-core/src/command.rs      Command enum (28 ops + engine-only internal.restore), resolution, apply, inverse_of, validate_project
crates/arc-core/src/engine.rs       Engine: revisions, idempotency, control/takeover, undo/redo, replay, confirm/rollback, diff
crates/arc-core/src/journal.rs      record encode/decode, CRC32C, JournalWriter (complete/partial/failure), FaultyFile injector
crates/arc-core/src/store.rs        .arcline ZIP container, atomic write, migrations stub, session lock, sidecar snapshots/journal, open-for-recovery
```

## 3. Module status

### Completed as drafts (need compile + tests)

- **`time`** — follows docs/02 §2.1–2.3 exactly. Includes the docs/08 §6 vectors and an exhaustive 24 h
  DF round trip (`exhaustive_round_trip_24h`). `RationalWire` was made `pub` after writing (serde
  `try_from`/`into` needs it); check that edit compiled as intended.
- **`ids`**, **`model`**, **`effects`**, **`error`** — straightforward; small test sets.
- **`command`** — all commands from docs/02 §4 except those listed in §5 below; whole-project invariant
  check after every batch.
- **`engine`** — synchronous state machine. A batch is applied speculatively, then `confirm(rev)` after the
  journal write or `rollback_unconfirmed()` on failure (docs/08 §4.3).
- **`journal`** — format and write states per docs/08 §4.2–4.4.
- **`store`** — container plus sidecar per docs/08 §4.1, §4.5, §4.6. `ProjectStore::open` returns the base
  snapshot and the decoded entries; **replay is the caller's job** (the runtime, not yet written).

### Incomplete / not started

- `crates/arc-core/src/lib.rs` (module declarations and re-exports).
- `runtime.rs` (threads, ack, autosave, flush, notifications, recovery replay) — see §6.5.
- Everything in `arc-media`, `arc-render`, `arc-audio`, `arc-cli`, `arc-app`, the reference embed host, CI.

## 4. Unverified code and outstanding tests

**Nothing has been compiled.** Expect compile errors. Known issues spotted on review but not fixed:

1. No `lib.rs` → the crate cannot build.
2. The workspace lists five member crates that do not exist → every `cargo` command at the root fails.
   Either create them or temporarily reduce `members` to `["crates/arc-core"]`.
3. `store.rs`, `ProjectStore::open`: the binding `p` in the first match arm is unused (warning). Rename it
   `_p`.
4. `zip = { version = "9", default-features = false, features = ["deflate"] }` — the feature name in
   zip 9.x has not been verified. If it fails, check the crate's feature list; it must stay pure Rust
   (no bzip2/zstd C code).
5. `blake3` with `features = ["pure", "std"]` — chosen to avoid the C/asm build. Not yet verified.
6. `im::Vector::sort_by`, and `im` serde support for `Vector`/`OrdMap` — assumed from the `im` 15 API; not
   yet confirmed against the docs.
7. `Engine::apply` with an idempotency-key hit returns an entry with empty `commands`. The runtime must
   **not** journal that entry; only fresh commits are journaled.
8. `SessionLock` on Windows relies on error code 32 (sharing violation) from `share_mode(0)`. The non-Windows
   path is weak (Linux `/proc` check; macOS effectively unchecked).

**Outstanding tests (required before Phase A exits; criteria in docs/10 §2 and docs/08 §6):**

- Engine: every command's happy path and its main rejections. Also: stale revision, `dry_run` not
  committing, idempotency, the `internal.restore` ban for clients, control and auto-takeover, undo/redo
  chains, `diff` contents.
- **Property test (invariant 7):** random batches, apply → undo → canonical hash identical; 10,000
  sequences (`proptest`).
- Journal: encode/decode round trip; torn tail at every byte offset; bad CRC, magic, and length; short
  writes (`FaultyFile.max_chunk = 1`); a failure mid-record truncates to the record start; truncate failure
  leaves a tail that decode rejects; durable-mode `sync` failure.
- Store: create → reopen; atomic save; recovery from sidecar snapshot plus journal; corrupt snapshot falls
  back to the container; `.rejected` tail written; lock held against a second open.
- Frame-rate/timecode suite from docs/08 §6 (largely drafted in `time.rs`; run it).

## 5. Architectural decisions and deviations taken during implementation

Recorded here because they are not yet reflected in `docs/`. Fold them into docs/02 (or an ADR) once they
are confirmed by tests.

| Topic | Decision in code | Docs said / why |
|---|---|---|
| Undo inverses | Entity-level restore: `internal.restore` carries the prior tracks, markers, changed assets, and name; it is journaled like any batch. Clients sending it get `forbidden_op`. | Docs say "inverse derived during apply" without a form. This form makes invariant 7 hold by construction and replay deterministic. |
| Resolved commands | `apply_command` rewrites each command in place (generated IDs, timecodes → exact rationals, effect defaults filled); the resolved list is what gets journaled and redone. | Implements docs/02 §5 "resolved forward commands". |
| Validation | Full `validate_project` after every batch (O(n)). | Correct but not incremental; may need optimizing against the 200 µs budget (docs/08 §2). |
| Effect manifests | `EffectCatalog` trait injected into `Engine`; built-in manifests will come from `arc-render`. | Keeps `arc-core` GPU-free (docs/01 §4). |
| Sequences | `sequences: OrdMap` with exactly one sequence enforced; `settings.sequence` is the active one. | Matches docs/02 §1. |
| Track index | Not stored; the vector position is the z-order (dense by construction). | docs/02 example shows `"index"`; update the example. |
| Clip source | `asset` or `title` field, exactly one. | docs/02 §3.3 shows `asset` only. |
| Transform | Static values only (not keyframable yet). Effect params support `{"static"}` / `{"curve"}` with f64 values. | docs/02 says every animatable property uses the param form; keyframed transforms are deferred. |
| Ripple | `none` / `track` / `all` on `remove_clip` and out-edge `trim_clip`; in-edge ripple → `unsupported_ripple`. | Subset of docs/02 §4. |
| Transitions | Centered on the cut; validation requires adjacency on one track and source handles of half the duration on both sides; removing a clip cascades its transitions; moving a clip with transitions to another track is rejected. | Consistent with docs/02 §3.5. |
| `allow_subframe` audio clips | Not implemented; every clip boundary must be frame-aligned. Fades are exempt (sample-grained). | Deferred. |
| Asset kinds on tracks | Video/image → video tracks; audio → audio tracks; a video asset with an audio stream may go on an audio track. No linked A/V clips. | Simplification; document in docs/02. |
| Journal | CBOR payload (`ciborium`), in-house CRC32C table. | Per docs/08 §4.2. |
| Toolchain target | `x86_64-pc-windows-gnu` (the only toolchain on this machine). | docs do not pin a target; CI should also build MSVC. |
| First media backend (planned) | `ffmpeg`/`ffprobe` **CLI subprocesses** behind the `Decoder`/`Encoder` traits; the in-process libav backend follows once FFmpeg dev libraries are available. | The machine has no FFmpeg dev libraries or C compiler; this is a working backend, not a mock, and matches the isolation strategy (docs/04 §1.2). |
| `unsafe_code = "deny"` workspace-wide | `arc-app` will need a crate-level override for `with_parent_window`. | Keep the override local to the embedding module. |

## 6. Toolchain and dependency blockers

### 6.1 Rust is installed but not on PATH

```bash
export PATH="/c/Users/ww/.rustup/toolchains/stable-x86_64-pc-windows-gnu/bin:$PATH"
rustc --version   # rustc 1.100.0 (2026-11-09), host x86_64-pc-windows-gnu
```

`~/.cargo/bin` (the rustup proxies) does not exist; call the toolchain binaries directly or reinstall
rustup. The GNU toolchain links with its self-contained MinGW linker — verified by compiling and running a
hello-world.

### 6.2 No C compiler, no MSVC, no Windows SDK

- No `cl.exe`, no `gcc`, no Windows SDK libs. `C:\Program Files\Microsoft Visual Studio\18` exists but is
  empty.
- Consequence: **every dependency must be pure Rust**. Watch for crates with `build.rs` C compilation
  (bzip2, zstd, ring, blake3 without `pure`, openssl, libz-sys).
- Fix if needed: install "Visual Studio Build Tools" (MSVC + Windows SDK) and the MSVC Rust target.

### 6.3 FFmpeg: CLI only

- `ffmpeg`/`ffprobe` 8.1.1 "essentials" build (gyan.dev) on PATH. No headers or import libraries, so no
  in-process libav / `ffmpeg-next`.
- That build contains GPL encoders (libx264, libx265). For development only: never rely on them in shipped
  defaults (docs/11 §2). Prefer `h264_mf`/NVENC/QSV/AMF, SVT-AV1, ProRes, and FFV1 (lossless; use it for
  frame-exact tests).

### 6.4 Dependency versions (crates.io, checked 2026-11-24)

egui / egui-winit / egui-wgpu **0.37**, egui_dock 0.22, wgpu 30.0, winit 0.31.0-beta.4, im 15.1,
arc-swap 1.9, zip 9.8, blake3 1.8, ciborium 0.2, ulid 3.0, proptest 1.12, cpal 0.18, windows-sys 0.61,
raw-window-handle 0.6.

These versions are newer than the author's API familiarity. For `arc-app`/`arc-render`, use
`egui_wgpu::wgpu` and `egui_winit::winit` **re-exports** so the versions always match, and read the
downloaded sources under `~/.cargo/registry` before writing against them.

### 6.5 Runtime design (to implement next, per docs/01 §5, docs/08 §4)

- **Engine thread** owns `Engine`. On `Apply`, it publishes the speculative snapshot (`ArcSwap`), sends the
  `JournalEntry` to the journal thread, and keeps the client's reply pending. On `Written{rev, ok}`:
  - success → `confirm(rev)` and reply;
  - failure → `rollback_unconfirmed()`, reply `journal_unavailable` to every pending client, enter
    read-only degraded mode, publish the rolled-back snapshot.
- **Journal thread** owns `ProjectStore` and processes jobs FIFO: `Append`, `Snapshot` (every 30 confirmed
  batches or 60 s), `Save`, and a `flush` on a 1 s `recv_timeout`. `Save` is rejected once the journal has
  failed.
- **Open:** `ProjectStore::open` → `Engine::new(base, base_revision)` → `engine.replay(entry)` until the
  first failure. Write the rejected entries next to the journal, then write a fresh snapshot at the
  recovered revision.
- **Notifications:** subscribers receive `Changed{revision, source, diff}` and `ControlChanged`.
- **API:** `apply_async(req) -> Receiver<Result<ApplyResult>>` for the GUI (never block the UI thread);
  `apply(req)` blocking for the CLI.

## 7. Exact next steps to resume

1. Put Rust on PATH (§6.1). Set `members = ["crates/arc-core"]` in `Cargo.toml` until the other crates exist.
2. Create `crates/arc-core/src/lib.rs`: `pub mod command; pub mod effects; pub mod engine; pub mod error;
   pub mod ids; pub mod journal; pub mod model; pub mod store; pub mod time;` plus re-exports of the main
   types.
3. `cargo build -p arc-core`, then fix compile errors and the issues listed in §4. Then
   `cargo clippy -p arc-core --all-targets` and `cargo test -p arc-core`.
4. Write the outstanding tests in §4. The property test for invariant 7 and the journal fault-injection
   suite gate Phase A.
5. Implement `runtime.rs` (§6.5) with tests: ack only after a complete write, rollback on an injected
   failure, recovery after a simulated kill (write records, drop without closing, reopen).
6. Minimal `arc-cli` (`new`, `describe`, `apply`) to meet the Phase A exit criteria (docs/10 §2).
   Commit "Phase A".
7. Phase P — embedding PoC (docs/13 §2.4): `arc-app` on `egui-winit` + `egui-wgpu`, a Win32 reference host,
   measured go/no-go recorded in ADR-0014. The Electron host part needs Electron installed (not present).
8. Continue Phase B onward per docs/10, starting with the CLI-subprocess media backend (§5).
9. Write the handover docs for the 1-to-10 phase: development guide, interface contracts (commands, JSON-RPC,
   MCP, embed channel, file formats), acceptance criteria per phase, and the implementation roadmap.
   Add a CI workflow (Windows build+test; macOS and Linux `cargo check` per ADR-0011).
