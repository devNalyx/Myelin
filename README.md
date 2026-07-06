# Myelin

**Mine Your Everyday Learned Instincts, Naturally.**

Myelin — the layer that turns repeated practice into instinct, the same way the biological process it's named after insulates a repeatedly-used neural pathway until it's faster and eventually automatic.

**Status:** MVP, registered as a live user-scoped MCP server (`claude mcp add myelin`). The full loop — observation → warmup queue → promotion → real `SKILL.md` → correction/confirmation feedback mutating that same file → usage/atrophy tracking — works end-to-end, verified over the actual MCP stdio protocol, plus an optional embeddings-based similarity upgrade. What's *not* built yet: automatic transcript ingestion and redaction — see §4/§5 for what's real vs. still sketch.

---

## 1. Thesis

A local daemon that watches your agentic coding sessions, mines them for recurring or high-stakes procedures, and — once a pattern earns it — crystallizes it into a real [Claude Code Skill](https://docs.claude.com/) (`SKILL.md`) that keeps mutating from how it's actually used afterward.

It isn't a codebase indexer. It's about noticing what you keep doing, not what your code contains — the goal is to eventually stop making you re-explain the same fix, convention, or workaround.

## 2. Architecture

- Daemon + MCP (stdio) + control-socket (Unix) split
- SQLite + WAL + FTS5 for storage/search (once storage logic lands)
- Config/TOML, systemd user unit, CLI, `.deb` packaging conventions
- Export/import as a portable snapshot
- An OpenAI-compatible embeddings client with an explicit policy gate (`is_loopback_or_private` check) — off by default, opt-in, and remote endpoints are blocked unless explicitly allowed
- Warm/cold gating (judge staleness by last-used, not last-modified) — currently an informational flag on skills; a natural next step is acting on it (e.g. no longer surfacing a cold skill in search) rather than just flagging it
- A scoped-neighborhood visualization pattern planned for browsing the learned graph — never the whole graph at once, always a bounded slice around whatever's being inspected

## 3. What makes this different from a typical indexer

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
- `skills.last_invoked_at` — set by `mark_skill_used`; `list_skills` derives a `stale` flag from it (falling back to `created_at` if a skill's never been marked used)
- `skill_candidates.embedding` — JSON-encoded vector, populated only when embeddings are enabled and reachable; candidates without one always fall back to Jaccard for that comparison

**Sketch, not yet implemented:**
- `Session` node (a source transcript) — there's no transcript ingestion yet, so observations are reported directly by the calling agent instead of mined from a `Session`
- `SUPERSEDES` edges — no skill versioning yet, a correction appends to the file rather than creating a new version

## 5. Pipeline

**Implemented:**
1. The calling agent reports a noteworthy procedure via the `record_observation` MCP tool (or `myelin observe` for debugging) — this **is** the extraction step for now: no transcript mining, no separate LLM call, no redaction subsystem, because the reporting agent already decides what's worth saying and never passes along anything it shouldn't.
2. Matching against existing candidates: token-overlap (Jaccard) by default, or cosine similarity over embeddings if `[embeddings] enabled = true` and policy-allowed (loopback/private endpoints allowed by default, anything else needs `allow_remote = true`). A candidate created before embeddings were enabled, or a call that fails mid-flight, transparently falls back to Jaccard for that comparison rather than erroring — this is an enhancement, never load-bearing. Threshold tunable via `[promotion]` in `config.toml` (`reps = 3`, `similarity_threshold = 0.4` by default — guesses, not measured values, and used as-is for both scoring methods even though cosine and Jaccard aren't guaranteed to mean the same thing at the same cutoff).
3. Promotion, either path: reps threshold crossed, or `high_stakes: true` fast-tracks off a single observation.
4. On promotion: a real `SKILL.md` is drafted from the accumulated observation summaries and written to `~/.claude/skills/<slug>/`, live immediately.
5. After a skill is in use, `record_skill_feedback` (or `myelin feedback`) reports back on it: a `correction` appends the fix directly into the live `SKILL.md` (the file itself gets better over time) and a `confirmation` just logs, building a visible confidence count in `list_skills` without touching the file.
6. `mark_skill_used` (or `myelin mark-used`) records that a skill was actually invoked, independent of feedback. `list_skills` flags a skill `stale` once `[atrophy] stale_after_secs` (default 30 days) has passed since its last use (or since promotion, if it's never been used) — informational only, nothing deletes or unregisters a stale skill automatically.

**Still sketch, not yet implemented:**
- Automatic transcript ingestion (watching `*.jsonl` session files instead of relying on an explicit tool call)
- A redaction pass (moot right now since nothing auto-ingests raw transcripts, but a hard blocker before that changes)
- The scoped-neighborhood graph visualizer, and any actual action taken on stale skills beyond the flag
- Re-embedding existing candidates after embeddings are turned on later — only newly-created candidates get a vector; nothing backfills old ones

## 6. Interop note: Open Knowledge Format (OKF)

Google Cloud announced [OKF](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing/) in June 2026 — a vendor-neutral spec for representing knowledge as one Markdown file per entity, YAML frontmatter with a required `type` field, relationships expressed as plain Markdown links. It's a near-exact match for the *hardened* tier of this graph (promoted skills, confirmed corrections).

Not adopted yet — noted here as the likely export/interop format once there's something worth exporting, so a promoted skill can be portable to other OKF-reading tools, not just this one. It does not replace the working-memory layer (SQLite + embeddings) that decides what gets promoted in the first place.

## 7. Open risks, unresolved

- What "same procedure" means as a similarity metric — untested for both Jaccard and cosine, and the same `similarity_threshold` is reused for both despite no guarantee they're comparable at the same cutoff
- Trigger-description precision on auto-drafted skills — false-positive firing risk grows with skill count
- Redaction is a real, undesigned subsystem — the biggest open gap before any automatic transcript ingestion is safe to turn on
- Decay/promotion/atrophy thresholds are guesses until there's real usage data
- No embedding model has actually been exercised against this yet — the client is implemented and unit-tested, but never verified end-to-end against a live endpoint

## 8. Current layout

```
crates/
  myelin-core/   # shared lib: Config, Paths, Error
  myelin-index/  # SQLite store, similarity matching, promotion, SKILL.md drafting
  myelind/       # daemon: `mcp` (stdio JSON-RPC, 6 tools) and `serve` (control socket) subcommands
  myelin-cli/    # `myelin status|observe|queue|skills|promote|feedback|mark-used`
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
./target/debug/myelin mark-used <skill_id>   # resets the staleness clock
```

Three similarly-worded `observe` calls (or one with `--high-stakes`) will drop a real `SKILL.md` into `~/.claude/skills/<slug>/` — override the location with `MYELIN_SKILLS_DIR` for testing.

Registered in this environment via `claude mcp add myelin -s user -- <path>/target/release/myelind mcp` (pointed at the release build, not `target/debug` — `cargo clean` or moving the repo will break it either way, since it's not installed anywhere yet) — live in every session from the next `claude`/`claude --resume` onward.

The daemon's control socket (`myelind serve` / `myelin status`) is unrelated to this loop — it's the separate GUI/status-check channel from the original scaffold, not yet wired to anything new.

Config lives at `~/.config/myelin/config.toml` (all optional, missing file = defaults):

```toml
[promotion]
reps = 3                    # observations needed before a candidate auto-promotes
similarity_threshold = 0.4  # Jaccard token-overlap threshold, 0.0-1.0

[atrophy]
stale_after_secs = 2592000  # 30 days; flags a skill `stale` in list_skills, doesn't act on it

[embeddings]
enabled = false              # off by default; structural matching (Jaccard) works with zero config
endpoint = "http://localhost:11434/v1"   # any OpenAI-compatible /v1/embeddings server
model = "nomic-embed-text"
api_key = ""                 # blank for local servers that don't need one
timeout_secs = 30
allow_remote = false          # required if endpoint isn't loopback/private
```

## 10. Tests

`cargo test --workspace` — 27 tests total: 6 unit tests in `myelin-core` (embeddings policy gating: loopback/private detection, enabled/disabled/remote-blocked states), 18 in `myelin-index` (tokenize/jaccard/cosine, both promotion paths, manual promotion + double-promotion error, the feedback loop's file mutation, staleness/mark-used, and an unreachable embeddings endpoint falling back to Jaccard rather than erroring), and 3 integration tests in `myelind` that spawn the real `myelind mcp` binary and drive it over actual stdio JSON-RPC (tool listing, the full observe→promote→feedback loop with real file assertions, and that bad input returns JSON-RPC errors rather than crashing).
