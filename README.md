# Myelin

**Mine Your Everyday Learned Instincts, Naturally.**

Myelin — the layer that turns repeated practice into instinct, the same way the biological process it's named after insulates a repeatedly-used neural pathway until it's faster and eventually automatic.

**Status:** MVP, registered as a live user-scoped MCP server (`claude mcp add myelin`). The full loop — observation → warmup queue → promotion → real `SKILL.md` → correction/confirmation feedback mutating that same file — works end-to-end, verified over the actual MCP stdio protocol. What's *not* built yet: automatic transcript ingestion, redaction, embeddings-based similarity, and atrophy — see §4/§5 for what's real vs. still sketch.

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

## 4. Data model

**Implemented** (`crates/myelin-index`, SQLite):
- `observations` — title, summary, project, context_signal, high_stakes, linked to its candidate
- `skill_candidates` — title, token-overlap `key`, `rep_count`, `status` (`warming`/`promoted`)
- `skills` — slug, path to the written `SKILL.md`, `promoted_reason` (`reps`/`context_signal`/`manual`), observation count, provenance timestamp
- `corrections` — skill_id, `kind` (`correction`/`confirmation`), note, timestamp; corrections also get appended live into the skill's actual `SKILL.md`

**Sketch, not yet implemented:**
- `Session` node (a source transcript) — there's no transcript ingestion yet, so observations are reported directly by the calling agent instead of mined from a `Session`
- `SUPERSEDES` edges — no skill versioning yet, a correction appends to the file rather than creating a new version
- Any notion of skill staleness/atrophy — nothing tracks whether a promoted skill is actually still being invoked

## 5. Pipeline

**Implemented:**
1. The calling agent reports a noteworthy procedure via the `record_observation` MCP tool (or `myelin observe` for debugging) — this **is** the extraction step for now: no transcript mining, no separate LLM call, no redaction subsystem, because the reporting agent already decides what's worth saying and never passes along anything it shouldn't.
2. Token-overlap (Jaccard) matching against existing candidates — a real, crude, no-embeddings-required stand-in for "is this the same procedure." (`PROMOTION_REPS = 3`, `SIMILARITY_THRESHOLD = 0.4` in `store.rs` — guesses, easy to retune.)
3. Promotion, either path: reps threshold crossed, or `high_stakes: true` fast-tracks off a single observation.
4. On promotion: a real `SKILL.md` is drafted from the accumulated observation summaries and written to `~/.claude/skills/<slug>/`, live immediately.
5. After a skill is in use, `record_skill_feedback` (or `myelin feedback`) reports back on it: a `correction` appends the fix directly into the live `SKILL.md` (the file itself gets better over time) and a `confirmation` just logs, building a visible confidence count in `list_skills` without touching the file.

**Still sketch, not yet implemented:**
- Automatic transcript ingestion (watching `*.jsonl` session files instead of relying on an explicit tool call)
- A redaction pass (moot right now since nothing auto-ingests raw transcripts, but a hard blocker before that changes)
- Embeddings-based similarity (upgrade path once token-overlap proves too blunt)
- Atrophy (flagging skills nobody's invoked in a while) and the scoped-neighborhood graph visualizer

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
  myelin-core/   # shared lib: Config, Paths, Error
  myelin-index/  # SQLite store, similarity matching, promotion, SKILL.md drafting
  myelind/       # daemon: `mcp` (stdio JSON-RPC, 5 tools) and `serve` (control socket) subcommands
  myelin-cli/    # `myelin status|observe|queue|skills|promote|feedback`
packaging/systemd/myelin.service
```

## 9. Try it

As an MCP server (what Claude Code actually speaks):

```
cargo build
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"record_observation","arguments":{"title":"...","summary":"..."}}}' \
  | ./target/debug/myelind mcp
```

Or directly via the CLI, against the same SQLite store:

```
./target/debug/myelin observe --title "apply db migration hotfix" --summary "run migrate.sh, restart service, verify health endpoint" --project repoA
./target/debug/myelin queue     # candidates still warming up
./target/debug/myelin skills    # promoted skills, with provenance
./target/debug/myelin promote <candidate_id>   # force-promote early
./target/debug/myelin feedback <skill_id> --kind correction --note "..."  # mutates the live SKILL.md
```

Three similarly-worded `observe` calls (or one with `--high-stakes`) will drop a real `SKILL.md` into `~/.claude/skills/<slug>/` — override the location with `MYELIN_SKILLS_DIR` for testing.

Registered in this environment via `claude mcp add myelin -s user -- <path>/target/debug/myelind mcp` — live in every session from the next `claude`/`claude --resume` onward.

The daemon's control socket (`myelind serve` / `myelin status`) is unrelated to this loop — it's the separate GUI/status-check channel from the original scaffold, not yet wired to anything new.
