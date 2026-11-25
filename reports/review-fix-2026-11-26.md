# Independent review fixes — `657f34a..207222f` (2026-11-26)

**Branch:** `fix/review-207222f` (from `main` at `599d114`). **Review verdict acted on:** conditional
pass, no P0/P1; every must-fix item below is fixed with a test or a documented narrowing, and the
deferred items are recorded with their reason.

| # | Finding | Status | Commit / evidence |
|---|---|---|---|
| P2-1 | `Quarantined` only covered the probe phase; `rustc` missing passed silently | **fixed** | `a1424c1` — stub takes `STUB_BAD_TOOL`; decode-phase crash and hang cases (`test decode crashed`/`timed out`, bounded by `PROBE_TIMEOUT`); a missing `rustc` now panics. `cargo test -p arc-media --test quarantine` 1 passed (42.4 s) |
| P3-1 | "no frames" gate applied to audio too | **fixed** | `6f5bbc5` — `decode_shortfall`: video by decoded frames, audio-only by `out_time_us` against `min(1 s, remaining)`, images not judged; unit tests cover the branches; malformed 1 passed, differential 1 passed (0 unexplained), CLI 5 passed; docs/10 and the robustness report reworded |
| P3-2 | HANDOVER contradictions (Quarantined open vs covered, CI runs) | **fixed** | docs commit — Phase B row, §6.3 (runs 7/9/12), §7 Phase B note, §6 quarantine test row, numbering restored |
| P3-3 | `render --help` missed `hevc` | **fixed** | `2622071` — `VideoCodec::NAMES`/`HELP` shared by the clap help, the parse error and a sync unit test; `render_help_and_parse_offer_the_same_codecs` (6 CLI tests) |
| P3-4 | repaired file at the same path was not retried | **fixed** | `55fcc2f` — preview fingerprints `(path, len, mtime)`; unavailable verdicts and open decoders are invalidated when the file changes; `repairing_the_file_at_the_same_path_is_picked_up` (15 preview tests) |
| P3-5 | `libav.rs` test outside `mod tests`, missing newline, `use` mid-file | **fixed** | `2e7c168` — test moved into the module, file newline, `use` at the top; `cargo test -p arc-media --lib` 11 passed |
| P3-7 | no hardware-independent test for the NoEncoder CLI path | **fixed** | `356587f` — `ARCLINE_ENCODER_CANDIDATES` (debug builds) replaces the candidate list; `render_without_a_usable_hevc_encoder_reports_no_encoder` asserts exit 2, `render_failed`, and the tried candidate (6 CLI tests); HANDOVER §6 lists the aid |
| P3-6 | CI maintainability (hard-coded runner paths, duplicated annotations, Node 20 deprecation) | **deferred** | Current CI is green and not self-hosted; recorded as technical debt for the CI-hardening batch |
| — | Non-Windows session-lock weakening | **deferred** | Must be resolved before porting; out of scope here |
| — | Real fuzz corpus (crashed media) | **deferred** | Phase B footnote; fuzz task |
| — | Sampling granularity | **deferred** | Backlog |
| Review §7.2 | Log artifacts instead of `::error` annotations | **accepted as-is this round** | Annotations are readable without a token and sufficed for the media root causes; log artifacts are a CI-hardening item |

## Verification

```text
cargo test -p arc-media --test quarantine            1 passed (42.4 s, two 20 s timeout cases)
cargo test -p arc-media --lib                        13 passed
cargo test -p arc-media --test malformed             1 passed
cargo test -p arc-media --test differential          1 passed, 0 unexplained
cargo test -p arc-cli --test cli                     6 passed
cargo test -p arc-cli --test preview                 15 passed
cargo test --workspace                               141 passed, 0 failed, 1 ignored
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets   exit 0 (pre-existing warnings only)
```

## Notes

- The decode-phase quarantine cases add ~42 s to the media suite (two 20 s timeout runs); this is
  deliberate coverage of the timeout cap.
- `ARCLINE_ENCODER_CANDIDATES` is a debug-build-only development aid, documented in HANDOVER §6
  alongside `ARCLINE_ALLOW_ANY_FFMPEG`; release builds ignore it.
- The preview file fingerprint is checked per decoded asset per frame (a `metadata` call); it is the
  mechanism that makes an in-place repair take effect without an edit.
