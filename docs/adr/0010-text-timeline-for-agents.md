# ADR-0010 — Text timeline projection for agents

**Status:** Accepted · 2026-11-24

## Context

Language models read and write linear text far more reliably than deeply nested JSON, but a video timeline
is inherently structured. Exposing only JSON makes agent interaction token-expensive and error-prone;
exposing only text risks losing precision. Meanwhile, human editors already understand EDL-style listings.

## Decision

Provide a **round-trippable text projection** of the timeline as a first-class, versioned view
(`inspect.timeline` → text; `timeline.apply_text` → diffed command batch). The text view carries structure,
timing, and top-level effect parameters; full fidelity stays in JSON (`inspect.clip --json`). The parser has
a published ABNF grammar and is fuzzed. Writes always pass through normal command validation — text is a
convenience layer, never a bypass.

## Consequences

- Agents can reason about an edit holistically, propose a rewritten timeline, and receive a precise diff of
  what would change.
- The format doubles as a human-readable export and a debugging aid.
- Costs: a parser and serializer to maintain, a grammar that must stay stable (versioned `v1` forever), and
  an explicit lossiness boundary that must be documented per field.
- It constrains the text format to be *dense* (one line per clip) so that a 2,000-clip project remains
  consumable in chunks; paging by time range is built into the API.

## Alternatives considered

- **JSON only**: precise but verbose; poor for whole-timeline reasoning and expensive in tokens.
- **YAML**: indentation-sensitive parsing is a known LLM failure mode; ambiguous typing.
- **OTIO as the primary agent format**: interoperable, but lossy for our parameter model and heavy for
  frequent small reads.
- **Custom binary**: unreadable by agents; rejected.
