# Audit follow-up — status of every open finding

**Date:** 2026-11-25. **Baseline audited:** `88e0707`
([reports/audit-2026-11-25-88e0707.md](audit-2026-11-25-88e0707.md)). **This branch:** `main` after the
FFmpeg 9.1 merge (`e5d444f`) and the follow-up commits listed below.

**State:** `cargo build --workspace --all-targets` exit 0; `cargo test --workspace` → **110 passed,
0 failed, 1 ignored**; `CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets` exit 0 (18
warnings from lints that rustc 1.100's clippy added to untouched code — 15 `collapsible_if`,
2 `manual_is_multiple_of`, 1 `chunks_exact` — and none from the follow-up changes, see §5).
Every finding is either fixed with a named regression test or deferred with the reason and the
recommended next step; nothing is silently open.

## 1. Summary

| Finding | Severity | Status | Commit | Evidence |
|---|---|---|---|---|
| F-01 rejected replay loses acknowledged edits | Critical | fixed (P0, earlier) | `e9734ae` | `arc-core/tests/recovery.rs` |
| F-02 mid-stream resolution change overflow | Critical | fixed (P0, earlier) | `5df12d2` | `arc-media/tests/param_change.rs` |
| F-03 mid-stream audio channel change | Critical | fixed (P0, earlier) | `c079c2b` | `arc-media/tests/param_change.rs` |
| F-04 create overwrites before locking | Critical | fixed (P0, earlier) | `fdfd080` | `arc-core/tests/recovery.rs` |
| F-05 stale sidecar / foreign snapshot | High | fixed (P0, earlier) | `f015dc6`, `932728e` | `arc-core/tests/recovery.rs` |
| F-06 audio corrupted every 4,096 samples | High | **fixed** | `c040ec4` | `arc-media/tests/audio_blocks.rs` |
| F-07 Opus/MKV 1 ms (48 sample) lag | High | **fixed** (initial seek removed; test pins alignment) | `c040ec4`, `31014b7` | `arc-media/tests/audio_anchor.rs` |
| F-08 random access returns a later frame | High | **fixed** | `a1420d9` | `arc-media/tests/seek_accuracy.rs` |
| F-09 non-file protocols / import check bypass | High | fixed (P0, earlier) | `20853c6`, `f52eaf9` | `arc-media/tests/paths.rs`, CLI/RPC tests |
| F-10 no `--root` / `--output-root` allow-lists | Medium | **deferred** (§3.1) | — | — |
| F-11 Phase B status overstated | Medium | **fixed** (status corrected, §4) | this change | `HANDOVER.md` §2, `docs/10` §4 |
| F-12 phantom redo after recovery | Medium | **fixed** | `7ac106f` | `arc-core/tests/engine.rs` |
| F-13 speed ≠ 1 split/trim rejected | Medium | **fixed** | `bd8dc63` | `arc-core/tests/engine.rs` |
| F-14 non-atomic snapshot/journal rotation | Medium | **fixed** | `b986e20` | `arc-core/tests/store.rs` |
| F-15 no panic containment in the engine | Medium | **deferred** (§3.2) | — | — |
| F-16 rotation and SAR ignored | Medium | **deferred** (§3.3) | — | — |
| F-17 whole export audio in memory; WAV overflow | Medium | **deferred** (§3.4) | — | — |
| F-18 >8,192 px sources abort the export | Medium | **deferred** (§3.5) | — | — |
| F-19 software H.264 encoders vs posture | Medium | fixed by the 9.1 build (D2, no `libopenh264`) | `c6b0808` | `third_party/ffmpeg/verify.sh` |
| F-20 pinned build licence posture | Medium | fixed (LGPL 2.1+ in-repo build) | `c6b0808` | `verify.sh`, manifest |
| F-21 no version/licence check | Medium | fixed (compile-time pin + run-time check) | `4ad2a54` | `arc-media/tests/runtime.rs` |
| F-22 VFR detection drops the half-frame bias | Low | **deferred** (§3.6) | — | — |
| F-23 colour parameters fixed at converter creation | Low | **deferred** (§3.7) | — | — |
| F-24 lock/error-kind mapping | Low | **fixed** | `c66eeee` | `arc-core/src/store.rs` unit test |
| F-25 AAC output depends on decode history | Info | accepted; affects test design only | — | differential report §2 |
| F-26 declared MSRV wrong | Low | **fixed** | `563f93f` | `Cargo.toml` |
| F-27 job and temporary-file hygiene | Low | **deferred** (§3.8) | — | — |
| F-28 `send_packet` EAGAIN drops the packet | Low | **fixed** (defensive; no reproducible trigger) | `27b67fe` | — |
| F-29 removing a locked track allowed | Low | **fixed** | `05a595f` | `arc-core/tests/engine.rs` |
| F-30 idempotency keys not bound to the request | Low | **deferred** (§3.9) | — | — |
| F-31 PoC embed messages in the product binary | Low | **fixed** | `03495c4` | `arc-app/src/app.rs` unit test |
| F-32 VBR probe duration | Info | **deferred** (§3.10) | — | — |
| F-33 known performance debt | Info | deferred with the phase it belongs to | — | budgets still unmeasured |

Fixed in this follow-up: **F-06, F-07, F-08, F-11, F-12, F-13, F-14, F-24, F-26, F-28, F-29, F-31**
(twelve findings). Earlier work closed F-01…F-05, F-09, F-19…F-21.

## 2. Evidence for the fixes in this follow-up

### F-06 — audio corrupted every 4,096 samples

`arc_av_audio_read` now serves overlapping reads from a buffer with one second of history, seeks to
`target − 12,000` frames on a real jump and decodes the pre-roll before the first delivered sample,
keeps the resampler primed, and does not seek on the first read from position 0.

`crates/arc-media/tests/audio_blocks.rs` reads every fixture with the mixer's exact pattern (advance
4,096, read 4,098) and requires **bit equality** with one continuous read: FLAC 48 kHz, FLAC 44.1 kHz
(resampled to 48 kHz), MP3, Ogg Opus, MKV Opus, AAC. Before the fix the first difference was at
sample 448 (AAC) or the second block (MP3/Opus) with errors up to 0.19 (a full-scale click).

### F-07 — Opus in MKV placed 48 samples late

The 48-sample lag came from the unconditional seek on the very first read: position 0 was decoded as
a post-seek stream, and MKV's 1 ms timestamps could not place the frame. That seek is gone (F-06
commit) and the first frame anchors the stream at 0 when decoding starts at the beginning.

`crates/arc-media/tests/audio_anchor.rs` encodes one Opus stream, remuxes it to MKV without
re-encoding, and requires the two containers to decode **bit-identically**; a read at 20 s / 50 s /
80 s must align with the continuous decode (zero shift; mean error below −60 dB), also from a fresh
decoder. A seek can still land inside one Opus packet (the decoder's first frame after a flush
carries or omits the overlap samples); the test bounds that at 5 ms and documents it.

### F-08 — random access returns a later frame

`arc_av_video_at` retries with a doubling back-off when the first decoded frame is later than the
target, rewinds to the first packet for a target at the stream start, and retries when a seek lands
past the last decodable frame.

`crates/arc-media/tests/seek_accuracy.rs` (4 tests) covers MPEG-2 open GOP in MKV, MPEG-2 in
MPEG-TS with a 1.433 s start offset, MPEG-4 Part 2 with B-frames in MP4 and the hardware H.264
fixture: every frame sequentially, backward, repeated and from fresh decoders. The audited failures
(TS frame 0 → 15, MP4 frame 13 → 15) are now exact.

### F-12 — phantom redo after recovery

Replay of a `Redo` record pops the redo stack and restores the undone entry as the undo entry, as
`Engine::redo` does. Test: `replayed_redo_does_not_leave_a_phantom_redo` rebuilds a fresh engine from
edit/undo/redo records and requires `can_redo() == false` and `nothing_to_redo`.

### F-13 — split and in-trim with speed ≠ 1

`validate_clip` keeps `start`/`duration` on the frame grid and exempts `source_in` when
`speed != 1`; at speed 1 the rule is unchanged. Test: a half-speed clip splits at frame 31
(`source_in 31/60 s`) and trims in to frame 32; a speed-1 clip with `source_in 1/60 s` still fails
with `off_frame_boundary`.

### F-14 — non-atomic snapshot/journal rotation

The snapshot is written to `snapshot-R.json.tmp`, the new journal is opened, and only then is the
snapshot renamed into place; either failure leaves the previous revision intact. Test: a directory
in place of `journal-1.bin` makes the rotation fail with `io_error`, no `snapshot-1.json` appears,
`snapshot-0`/`journal-0` survive, and the rotation succeeds once the blocker is removed.

### F-24 — lock error mapping

Only a Windows sharing violation (os error 32) is `Locked`; other failures to open the lock file are
`StoreError::Io`. Unit test in `arc-core/src/store.rs`.

### F-26 — MSRV

`rust-version = "1.95"` (egui 0.37's floor; zip 9.8 needs 1.89, wgpu 30 needs 1.87). The tested
toolchain stays 1.100.0, as recorded in HANDOVER §1.

### F-28 — `send_packet` EAGAIN

The packet is kept (`pkt_pending`) and resent after draining a frame. No regression test: the
receive-first loop does not produce `EAGAIN` on any fixture.

### F-29 — locked track removal

`track.remove` calls `ensure_unlocked`, like every clip command. Test: a locked track is refused with
`track_locked`, an unlocked one is removed.

### F-31 — PoC embed messages

`poc.hang` and `poc.stats` are ignored unless the process runs with `--poc-seconds`, and a hang
request is capped at 5 s. Unit test gates the decision function.

## 3. Deferred findings — reason and recommendation

### 3.1 F-10 — `--root` / `--output-root` allow-lists

**Reason:** the allow-lists are a product decision, not a code gap. Network-share media is
intentionally supported (HANDOVER §6.11), so the default root set, the treatment of UNC paths and
the error contract (`path_not_allowed`?) must be decided together; a half-default would break
existing projects. **Recommendation:** add both flags with an explicit opt-in, default to the
project folder and the output folder, and cover the UNC/NTLM exposure in the same change; this is
the security half of the Phase E "relink/import" work.

### 3.2 F-15 — panic containment in the engine

**Reason:** `catch_unwind` alone is not a fix: a panic inside `apply_command` can leave the engine
mid-batch, and the contract for what the session does next (roll back to the last confirmed
revision? restart the engine thread? mark the session degraded?) is a design decision that docs/01
§9 states only in outline. Implementing it ad hoc would trade a crash for silent state divergence.
**Recommendation:** implement per-batch `catch_unwind` plus rollback to the last *confirmed* state
and an `engine_degraded` error, with a test seam that injects a panicking command under a cargo
feature. This belongs with the Phase E reliability work.

### 3.3 F-16 — rotation and pixel aspect ratio

**Reason:** applying rotation/SAR belongs to the compositor and the import normalisation, and the
GUI viewer that would validate it does not exist yet (Phase C). The probe already records rotation.
**Recommendation:** normalise at import (bake rotation into the asset metadata) and apply SAR in
`plan_frame`, with still-image and phone-clip fixtures; do it with the Phase C viewer.

### 3.4 F-17 — whole export audio in memory; WAV size field

**Reason:** streaming the mixed audio requires an export rework (pipe or chunked WAV plus RF64/W64
above 4 GB) and only matters for ranges above ~3 hours; the current end-to-end tests render seconds.
**Recommendation:** write 1-second blocks to the WAV/RF64 writer, add a >4 GB writer test with a
sparse file, and keep `Mixer::render` for tests.

### 3.5 F-18 — sources larger than the default GPU texture limit

**Reason:** needs an adapter-limit query, a downscale policy in the shim and a GPU test fixture
(100 MP still); the compositor is being extended in Phase C.
**Recommendation:** request `adapter.limits()` in `Renderer::new_headless`, clamp the texture size
and downscale in the shim; add a large-still fixture to `arc-render/tests`.

### 3.6 F-22 — VFR detection disables the half-frame bias

**Reason:** analysis-only and unverified: no fixture reproduces it, and the fix depends on the
product rule for mixed-rate media (which the time model must define). **Recommendation:** add the
generated `avg != r` MKV fixture first; if it reproduces, key `vfr` on the frame-rate comparison with
a tolerance (for example more than 1 % apart) rather than exact inequality.

### 3.7 F-23 — colour parameters fixed at converter creation

**Reason:** a mid-stream `colorspace`/`color_range` change is rare, and rebuilding the swscale
context per change must not disturb F-02's destination-size guarantee; the fix needs a differential
fixture (mixed-tag MPEG-2) to prove it. **Recommendation:** treat `colorspace`/`color_range` like the
size/format in `to_rgba`'s rebuild condition, with a fixture from `testsrc2` re-tagged mid-stream.

### 3.8 F-27 — job and temporary-file hygiene

**Reason:** the job map and the temporary snapshot are owned by the RPC session, which is being
extended in Phase F (progress, `queued`, notifications); cleaning up on cancel is part of that
lifecycle. `job.submit` also writes the snapshot to `%TEMP%`, so the fix should move it under the
project's sidecar while touching the same code. **Recommendation:** delete the snapshot in the job
worker on completion, cancel and session shutdown; stream the WAV through a `TempFile` guard so a
failed encode removes it.

### 3.9 F-30 — idempotency keys not bound to the request

**Reason:** the fix changes the documented idempotency contract (a repeated key with different
commands currently returns the cached result; after the fix it must fail). That is an additive
protocol decision for the agent surface (Phase F) and needs a decision on the error kind
(`conflict`? `invalid_value`?). **Recommendation:** build the key as `hash(client, commands,
base_revision)` and return `idempotency_conflict` on a mismatch, before the control and
`base_revision` checks.

### 3.10 F-32 — probe duration for variable-bitrate elementary streams

**Reason:** informational: the duration comes from the container/bitrate estimate, which is what the
model records today; a full scan belongs to the background probe planned in docs/04 §2.
**Recommendation:** record a confidence flag on `Probe` and run a full-scan pass for elementary
streams without a container duration.

## 4. Phase B exit criteria (docs/10 §4)

The audit's F-11 correction is kept and updated with the fixes above:

| Criterion | Status | Evidence |
|---|---|---|
| `arcline-cli render …` is frame-accurate against a reference decode | **Met** | `render_e2e` verifies every output frame (FFV1, H.264, ranges); `seek_accuracy` covers long-GOP/B-frame/TS sources |
| Hardware H.264/HEVC and AV1 (SVT) encode paths work on the reference machine | **Partially met** | H.264 via `h264_qsv` and AV1 via `libsvtav1` are tested end to end; **HEVC has no codec option yet** (the encoder set is probed, but no `--codec hevc` path exists) |
| A corrupt file produces a clean per-clip error; a crashing/hanging file is quarantined and the parent survives (mutated-media corpus) | **Partially met** | the import check classifies truncated/zero/text files as `offline` and the renderer reports per-clip errors with warnings; the *quarantine* path is implemented but has no crashing/hanging fixture, so the parent-survival half is unproven |
| Windows FFmpeg LGPL build pipeline pinned in the repo | **Met** | `third_party/ffmpeg/` (recipe, profile, manifest with hashes, SBOM, verify.sh) |
| macOS/Linux CI per ADR-0011 | **Not met** | no CI workflow exists yet |

Phase B therefore remains "functionally complete; exit criteria open" in HANDOVER §2: the
frame-accuracy and FFmpeg-package criteria are met, HEVC and the crashing-media corpus are not.
## 5. Verification at the merge state (2026-11-25)

Run from the repository root with `scripts/dev-env.sh` applied (`ARCLINE_FFMPEG_DIR` = the pinned
`~/.local/toolchains/ffmpeg-9.1-lgpl`, MinGW on `PATH`), Windows 11, rustc 1.100.0
`x86_64-pc-windows-gnu`:

| Command | Result |
|---|---|
| `cargo build --workspace --all-targets` | exit 0 (`Finished dev profile … in 2m 08s`) |
| `cargo test --workspace` | **110 passed, 0 failed, 1 ignored** across 27 test binaries (the ignored test is the forced-kill child half) |
| `CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets` | exit 0; 18 warnings, all from newer clippy lints in code not touched by this work (`collapsible_if` ×15, `manual_is_multiple_of` ×2, `chunks_exact` ×1). No follow-up change produces a lint. A dedicated lint-cleanup commit is recommended but was kept out of this change (no unrelated refactoring) |
| differential harness (`ARCLINE_CORPUS_DIR=…arcline-corpus-8.1.3`, allow-list) | `1 passed`, 12 fields explained (F-06/F-07/F-08 fixes), **0 unexplained** |
| `third_party/ffmpeg/verify.sh` | not re-run (no package change); last run exit 0, 40 checks |

Per-suite counts at this state: arc-core unit 16, `engine` 14, `journal` 9, `property` 1,
`recovery` 7, `store` 8; arc-media unit 9, `decode` 3, `audio_blocks` 2, `audio_anchor` 2,
`param_change` 2, `paths` 2, `runtime` 3, `differential` 1, `seek_accuracy` 4; arc-render 1+1+5;
arc-audio 3; arc-cli 4+4+4; arc-app 1.
**Batch 2 (2026-11-25, `feat/phase-b-exit`):** the two open halves above were worked:
HEVC export is implemented through the hardware encoder (`hevc_qsv` on the reference machine,
`--codec hevc`, validated with ffprobe) and is **met**; the malformed-media corpus and the mixed
good/broken export are in place, so the "corrupt file produces a clean per-clip error" half is met
but the crashing/hanging quarantine fixture is still missing (**partially met**); the CI workflow
exists and runs the core matrix on GitHub, with the media job building the pinned FFmpeg
(**met as a workflow; first GitHub run unobserved**). Details and evidence:
[phase-b-exit-2026-11-25.md](phase-b-exit-2026-11-25.md).