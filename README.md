# Myelin

**Mine Your Everyday Learned Instincts, Naturally.**

Myelin — the layer that turns repeated practice into instinct, the same way the biological process it's named after insulates a repeatedly-used neural pathway until it's faster and eventually automatic.

**Status:** scaffold only. This repo currently builds a daemon/MCP/CLI skeleton with no ingestion, extraction, or skill-promotion logic yet. Everything below is the design sketch that scaffold is shaped around, not a description of what's implemented.

---

## 1. Thesis

A local daemon that watches your agentic coding sessions, mines them for recurring or high-stakes procedures, and — once a pattern earns it — crystallizes it into a real [Claude Code Skill](https://docs.claude.com/) (`SKILL.md`) that keeps mutating from how it's actually used afterward.

This is not "index my codebase" (that's [NexusContext](https://github.com/devNalyx/NexusContext), the sibling project this one's daemon/MCP/storage skeleton is forked from in spirit). It's "notice what I keep doing, and eventually stop making me re-explain it."

## 2. What's reused from NexusContext's architecture

- Daemon + MCP (stdio) + control-socket (Unix) split
- SQLite + WAL + FTS5 for storage/search (once storage logic lands)
- Config/TOML, systemd user unit, CLI, `.deb` packaging conventions
- Export/import as a portable snapshot
- The OpenAI-compatible embeddings client shape — promoted from optional to load-bearing here, since there's no AST equivalent for behavior
- The warm/cold gating pattern (judge staleness by last-used, not last-modified) — reused conceptually for skill atrophy
- The scoped-neighborhood + Graphviz visualization pattern (never render the whole graph, only a bounded slice) — reused for browsing the learned graph

## 3. What's new, not reused

- **Ingestion source:** session transcripts (Claude Code `*.jsonl`), not source files — no tree-sitter, no AST
- **Extraction:** an LLM normalization pass, not a deterministic parser
- **A capture-worthiness pre-filter** — most tool calls are baseline agent capability, not a skill, regardless of how often they recur
- **Two independent promotion paths**, not one:
  - *Retrospective* — a candidate procedure recurs enough times across sessions to earn promotion (the "reps" model)
  - *Prospective* — an LLM judges, from context alone (a Jira ticket saying "roll this out across 100+ repos," your own stated scope), that a pattern is worth capturing off a single occurrence, because the reps are clearly coming
- **Living skills** — a promoted `SKILL.md` keeps mutating from usage feedback (corrections, rejections) instead of being a static, one-shot artifact
- **A redaction/privacy pass** before anything touches storage — transcripts can contain pasted secrets, credentials, PII; source code never raised this concern

## 4. Data model (sketch, not yet implemented)

**Nodes:**
- `Session` — a source transcript (project, timestamp)
- `Observation` — a normalized "goal + steps" summary extracted from a session
- `SkillCandidate` — a cluster of similar Observations; carries confidence/rep-count and a decay timer
- `Skill` — a promoted, live `SKILL.md` (trigger description + instructions + provenance + usage stats)
- `Correction` — explicit feedback tied to a Skill or Observation
- `ContextSignal` — an external stakes marker (ticket text, an explicit scope statement) that can justify fast-track promotion

**Edges:**
- `OBSERVED_IN` (Observation → Session)
- `EVIDENCE_FOR` (Observation → SkillCandidate)
- `HARDENED_INTO` (SkillCandidate → Skill)
- `CORRECTS` / `REINFORCES` (Correction → Skill)
- `JUSTIFIES` (ContextSignal → SkillCandidate/Skill, the fast-track path)
- `SUPERSEDES` (Skill → Skill, when a mutation is significant enough to version rather than edit in place)

## 5. Pipeline (sketch, not yet implemented)

1. Watch session transcripts (reuse the notify-debounce pattern)
2. Redact obvious secrets before anything else touches the content
3. Capture-worthiness gate — domain/org-specific, or just baseline agent behavior?
4. Extract into a normalized `Observation`
5. Embed + match against the `SkillCandidate` queue — increment reps on a match, spawn a new candidate on a miss
6. Promotion check — reps threshold crossed, **or** a `ContextSignal` judged high-stakes enough to fast-track off rep 1
7. On promotion: auto-draft `SKILL.md`, goes live immediately, tagged with provenance (observation count, source sessions)
8. Every real invocation afterward (success/correction/rejection) is itself new evidence, feeding back into the skill's confidence and potentially rewriting its instructions
9. Atrophy: unused skills get flagged (not silently deleted) via the inverted warm/cold gate
10. Visualization: a bounded-neighborhood graph view (one skill, one campaign, one time window — never the whole graph) as the primary tool for manual merge/demote/prune

## 6. Interop note: Open Knowledge Format (OKF)

Google Cloud announced [OKF](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing/) in June 2026 — a vendor-neutral spec for representing knowledge as one Markdown file per entity, YAML frontmatter with a required `type` field, relationships expressed as plain Markdown links. It's a near-exact match for the *hardened* tier of this graph (promoted skills, confirmed corrections), and NexusContext's own Phase 9 (Obsidian-vault export) was accidentally a preview of the same idea before OKF existed.

Not adopted yet — noted here as the likely export/interop format once there's something worth exporting, so a promoted skill can be portable to other OKF-reading tools, not just this one. It does not replace the working-memory layer (SQLite + embeddings) that decides what gets promoted in the first place.

## 7. Open risks, unresolved

- What "same procedure" means as a similarity metric — untested assumption
- Trigger-description precision on auto-drafted skills — false-positive firing risk grows with skill count
- Redaction is a real, undesigned subsystem — the single biggest departure from NexusContext's threat model
- Decay/promotion thresholds are guesses until there's real usage data

## 8. Current layout

```
crates/
  myelin-core/   # shared lib: Config, Paths, Error — no pipeline types yet
  myelind/       # daemon: `mcp` (stdio JSON-RPC) and `serve` (control socket) subcommands
  myelin-cli/    # `myelin status` — pings the control socket
packaging/systemd/myelin.service
```

## 9. Try it

```
cargo build
./target/debug/myelind serve &
./target/debug/myelin status
```
