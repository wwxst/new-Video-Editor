# ADR-0010 — Read-only text timeline for agents

**Status:** Accepted, revised 2026-11-24 after architecture review.

## Context

Language models reason better over linear text than deeply nested JSON. A video timeline is inherently
structured, so a text projection helps agents read the edit and propose changes. The original proposal also
specified a round-tripping `apply_text` path (parse text → diff → commands).

## Decision

Provide a **versioned, read-only text projection** (`inspect.timeline`) with a published ABNF grammar.
Writes go through typed JSON commands; `timeline.apply_text` is deferred to post-1.0.

- The view carries structure, timing, and top-level effect parameters; full fidelity stays in JSON.
- `inspect.timeline` accepts a time range so large projects are consumable in chunks.
- The grammar is stable (`v1`) and the parser is fuzzed.

## Consequences

- Agents get the reasoning-friendly view at low cost, with no parser/diff maintenance burden in v1.
- Writes remain typed, validated, and unambiguous; no lossy text round-trip can silently corrupt an edit.
- If `apply_text` is added later, it will diff against the current timeline and emit a normal command batch
  through validation — the text layer can never bypass the command engine.
- The format doubles as a human-readable export and debugging aid.

## Alternatives considered

- **JSON only**: precise but verbose; expensive in tokens and poor for whole-timeline reasoning.
- **Full round-trip text in v1**: attractive for agents, but a parser + diff + ambiguity policy is a
  subsystem with no proven need before the agent surface exists; deferred.
- **YAML**: indentation-sensitive parsing is a known LLM failure mode.
- **OTIO as the primary format**: lossy for our parameter model and heavy for frequent reads.
