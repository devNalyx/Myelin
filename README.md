# Myelin

**Mine Your Everyday Learned Instincts, Naturally.**

Myelin — the layer that turns repeated practice into instinct, the same way the biological process it's named after insulates a repeatedly-used neural pathway until it's faster and eventually automatic.

**Status:** MVP, registered as a live user-scoped MCP server (`claude mcp add myelin`). The full loop — observation → warmup queue → promotion → real `SKILL.md` → correction/confirmation feedback mutating that same file → usage/atrophy tracking — works end-to-end, verified over the actual MCP stdio protocol. Automatic session ingestion also exists now: a `SessionEnd` hook redacts and heuristically stages candidates from each session's transcript into a review queue — no daemon-side LLM judgment, a later live agent session still decides what's actually worth an observation. Candidate matching is plain token-overlap (Jaccard) only — an embeddings-based upgrade was built, then deliberately decommissioned before any real evidence it was needed (see §7). See §4/§5 for what's real vs. still sketch.

---

## 1. Thesis

A local daemon that watches your agentic coding sessions, mines them for recurring or high-stakes procedures, and — once a pattern earns it — crystallizes it into a real [Claude Code Skill](https://docs.claude.com/) (`SKILL.md`) that keeps mutating from how it's actually used afterward.

It isn't a codebase indexer. It's about noticing what you keep doing, not what your code contains — the goal is to eventually stop making you re-explain the same fix, convention, or workaround.

## 2. Architecture

- Daemon + MCP (stdio) + control-socket (Unix) split
- SQLite + WAL + FTS5 for storage/search (once storage logic lands)
- Config/TOML, systemd user unit, CLI, `.deb` packaging conventions
- Export/import as a portable snapshot
- Warm/cold gating (judge staleness by last-used, not last-modified) — an informational `stale` flag on skills; acting on it (`archive_skill`) is always an explicit, agent/user-invoked call, never automatic, since there's no evidence yet for what threshold would be safe to act on unsupervised
- A scoped-neighborhood visualization for browsing the graph — never the whole graph at once, always a bounded slice (one skill, its candidate, its observations, its corrections) rendered via Graphviz

## 3. What makes this different from a typical indexer

- **Ingestion source:** session transcripts (Claude Code `*.jsonl`), not source files — no tree-sitter, no AST
- **Extraction is two-tiered, not one LLM call:** cheap, deterministic heuristics (pattern-matching over tool sequences and phrasing, not an LLM) decide what's worth *surfacing*; an actual agent, later, still decides what's worth *capturing*. No daemon-side model ever judges content on its own.
- **A capture-worthiness pre-filter** — most tool calls are baseline agent capability, not a skill, regardless of how often they recur
- **Two independent promotion paths**, not one:
  - *Retrospective* — a candidate procedure recurs enough times across sessions to earn promotion (the "reps" model)
  - *Prospective* — an agent judges, from context alone (a Jira ticket saying "roll this out across 100+ repos," your own stated scope), that a pattern is worth capturing off a single occurrence, because the reps are clearly coming
- **Living skills** — a promoted `SKILL.md` keeps mutating from usage feedback (corrections, rejections) instead of being a static, one-shot artifact
- **A redaction pass ahead of anything that touches storage** — broad and aggressive on purpose (known secret formats, generic `*_KEY=`/`*_SECRET=`-style assignments, high-entropy tokens, emails, IPs); not comprehensive PII scrubbing, but the highest-severity leak class is covered before a byte of transcript content is ever persisted

## 4. Data model

**Implemented** (`crates/myelin-index`, SQLite):
- `observations` — title, summary, project, context_signal, high_stakes, linked to its candidate
- `skill_candidates` — title, token-overlap `key`, `rep_count`, `status` (`warming`/`promoted`)
- `skills` — slug, path to the written `SKILL.md`, `promoted_reason` (`reps`/`context_signal`/`manual`), observation count, provenance timestamp
- `corrections` — skill_id, `kind` (`correction`/`confirmation`), note, timestamp; corrections also get appended live into the skill's actual `SKILL.md`
- `skills.last_invoked_at` — set by `mark_skill_used`; `list_skills` derives a `stale` flag from it (falling back to `created_at` if a skill's never been marked used)
- `skills.status` (`active`/`archived`) — set by `archive_skill`/`restore_skill`, which also physically move the `SKILL.md` file between the live skills directory and an `.myelin-archived/` subfolder so an archived skill actually stops being loadable
- `pending_reviews` — session_id, project, `heuristic_reason` (`multi-step-sequence`/`error-then-fix`/`correction-language`/`high-stakes-phrasing`), an already-redacted and bounded `excerpt`, `status` (`pending`/`dismissed`). This is the only table that ever holds anything derived from a raw transcript, and only ever the redacted excerpt — never the transcript itself

**Sketch, not yet implemented:**
- A proper `Session` node with real structure — `pending_reviews` covers the practical need (surface candidates for review) without a full session/observation graph relationship
- `SUPERSEDES` edges — no skill versioning yet, a correction appends to the file rather than creating a new version

## 5. Pipeline

**Implemented:**
1. Capture happens two ways now: the calling agent reports a noteworthy procedure directly via `record_observation` (or `myelin observe`), **or** a `SessionEnd` hook automatically stages candidates from the session that just ended (see the ingestion sub-pipeline below). Either way, an agent still makes the final call on whether something becomes a real observation — nothing auto-promotes straight from a transcript.
2. Matching against existing candidates: token-overlap (Jaccard) similarity. Threshold tunable via `[promotion]` in `config.toml` (`reps = 3`, `similarity_threshold = 0.4` by default — guesses, not measured values). An embeddings-based cosine-similarity upgrade was built and later decommissioned (see §7) before any real evidence Jaccard needed the help.
3. Promotion, either path: reps threshold crossed, or `high_stakes: true` fast-tracks off a single observation.
4. On promotion: a real `SKILL.md` is drafted from the accumulated observation summaries and written to `~/.claude/skills/<slug>/`, live immediately.
5. After a skill is in use, `record_skill_feedback` (or `myelin feedback`) reports back on it: a `correction` appends the fix directly into the live `SKILL.md` (the file itself gets better over time) and a `confirmation` just logs, building a visible confidence count in `list_skills` without touching the file.
6. `mark_skill_used` (or `myelin mark-used`) records that a skill was actually invoked, independent of feedback. `list_skills` flags a skill `stale` once `[atrophy] stale_after_secs` (default 30 days) has passed since its last use (or since promotion, if it's never been used) — informational only; nothing acts on it automatically.
7. `archive_skill` (or `myelin archive-skill`) moves a skill's file out of the live skills directory into `.myelin-archived/` and marks it `archived` — always an explicit call an agent or the user makes after looking at the `stale` flag, never triggered by the flag itself. `restore_skill` reverses it exactly.
8. `render_skill_graph` (or `myelin graph <id>`) renders one skill's bounded neighborhood — its candidate, the observations that backed it, its corrections/confirmations — as a PNG via Graphviz, using the original design ontology's edge names (`EVIDENCE_FOR`, `HARDENED_INTO`, `CORRECTS`/`REINFORCES`). Falls back to returning the raw DOT source if `dot` isn't installed (a Recommends, not a hard dependency).

**Ingestion sub-pipeline (implemented):**
1. A `SessionEnd` hook (`matcher: "*"`, every reason) runs `myelin ingest-session`, fed the hook's JSON payload (`session_id`, `transcript_path`, `cwd`) on stdin.
2. The transcript is parsed into turns (`crates/myelin-index/src/transcript.rs`) — defensively, since the exact schema was never verified against real content (see open risks).
3. Every string that will ever leave this stage is redacted first (`crates/myelin-index/src/redact.rs`) — private keys, AWS keys, JWTs, bearer tokens, generic secret-looking assignments, emails, IPs, and high-entropy tokens as a fallback.
4. Cheap heuristics (`crates/myelin-index/src/staging.rs`) flag candidates — a multi-step tool sequence, an error followed by more activity, correction-flavored language, high-stakes phrasing — capped at 5 per session. No LLM call anywhere in this path.
5. Flagged candidates land in `pending_reviews`. Nothing else from the transcript is ever written anywhere; raw and redacted-but-unflagged content is discarded the moment the hook process exits.
6. `list_pending_review` / `dismiss_pending_review` surface the queue to whichever agent session looks at it next — same judgment bar as `record_observation` today, just working from staged material instead of live memory.

**Still sketch, not yet implemented:**
- Broader verification of the transcript parser — it correctly parsed one real session (see §7), which is real signal but not broad coverage
- Anything automatic acting on the `stale` flag — archiving stays a deliberate, explicit call by design (see §2), not a gap to fill later

## 6. Interop note: Open Knowledge Format (OKF)

Google Cloud announced [OKF](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing/) in June 2026 — a vendor-neutral spec for representing knowledge as one Markdown file per entity, YAML frontmatter with a required `type` field, relationships expressed as plain Markdown links. It's a near-exact match for the *hardened* tier of this graph (promoted skills, confirmed corrections).

Not adopted yet — noted here as the likely export/interop format once there's something worth exporting, so a promoted skill can be portable to other OKF-reading tools, not just this one. It does not replace the working-memory layer (SQLite) that decides what gets promoted in the first place.

## 7. Open risks, unresolved

- What "same procedure" means as a similarity metric — untested, and the threshold (0.4) is a guess, not a measured value
- Trigger-description precision on auto-drafted skills — false-positive firing risk grows with skill count
- Decay/promotion/atrophy thresholds are guesses until there's real usage data
- **Embeddings-based similarity was built, then decommissioned.** An optional cosine-similarity upgrade (OpenAI-compatible endpoint, policy-gated, off by default) was added, then removed before any real observation ever needed it — with exactly one real observation in the system at the time, there was no evidence Jaccard was insufficient, and the feature had already accumulated real surface area (a network dependency, an unverified-against-any-live-endpoint code path) ahead of that evidence. Consistent with this project's own "observe before you build" principle applied retroactively rather than followed the first time.
- **The transcript parser's schema was mostly built without verification, and has now been checked against exactly one real transcript.** Only top-level `type` field names were ever safely inspected before that (reading another session's actual content without explicit authorization wasn't something to do casually, even for development); the content-block shape it assumes is the standard, documented Anthropic Messages API format. It correctly parsed a real 454-tool-call session on the first real run — genuine signal, but one session isn't broad coverage, and a real bug was found in the same run (see below), so "worked once" isn't "confirmed correct" either.
- The heuristic staging thresholds are first-guess pattern matching. One (`100+` in high-stakes-phrasing) was already found broken against real text — a shared trailing `\b` could never match `+` followed by a space — and fixed; the rest (4+ tool calls, the other regexes) are still unverified against real sessions beyond that one run.
- Redaction is broad/aggressive by design but not comprehensive PII scrubbing — a secret in a format it doesn't recognize gets through
- No `license` field is set anywhere in the workspace (`cargo deb` flags this at build time) — an open decision, not an oversight; nothing here should be treated as licensed for reuse until that's resolved

## 8. Current layout

```
crates/
  myelin-core/   # shared lib: Config, Paths, Error
  myelin-index/  # SQLite store, similarity matching, promotion, SKILL.md drafting,
                 # redact.rs / transcript.rs / staging.rs (the ingestion sub-pipeline)
  myelind/       # daemon: `mcp` (stdio JSON-RPC, 8 tools) and `serve` (control socket) subcommands
  myelin-cli/    # `myelin status|observe|queue|skills|promote|feedback|mark-used|
                 #   ingest-session|pending-review|dismiss-review`
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

**Session ingestion**, driven by a Claude Code `SessionEnd` hook (`~/.claude/settings.json`, user scope):

```json
{
  "hooks": {
    "SessionEnd": [
      {
        "matcher": "*",
        "hooks": [
          { "type": "command", "command": "<path>/target/release/myelin ingest-session" }
        ]
      }
    ]
  }
}
```

Fires on every real session end, fed `{session_id, transcript_path, cwd, reason}` on stdin. Never fails loudly — a SessionEnd hook has no decision control anyway, so silent best-effort is the only sane behavior. Review what it's staged:

```
./target/debug/myelin pending-review
./target/debug/myelin dismiss-review <id>
```

Config lives at `~/.config/myelin/config.toml` (all optional, missing file = defaults):

```toml
[promotion]
reps = 3                    # observations needed before a candidate auto-promotes
similarity_threshold = 0.4  # Jaccard token-overlap threshold, 0.0-1.0

[atrophy]
stale_after_secs = 2592000  # 30 days; flags a skill `stale` in list_skills, doesn't act on it
```

**Packaging:** a `.deb` bundling both binaries plus the systemd unit.

```
cargo build --release
cargo deb -p myelind --no-build
sudo dpkg -i target/debian/myelin_*.deb
```

Verified end to end: installs `myelind`/`myelin` to `/usr/bin/`, the unit to `/usr/lib/systemd/user/myelin.service` (`systemd-analyze --user verify` passes), and `systemctl --user start myelin.service` actually runs — confirmed live, not just built. `sudo dpkg -r myelin` removes cleanly. Not yet published anywhere (no releases, no `.rpm`) — this is a local build target for now, and independent of the `target/release` binary this environment's MCP server/hook actually point at.

## 10. Tests

`cargo test --workspace` — 40 tests total: 36 in `myelin-index` (tokenize/jaccard, both promotion paths, manual promotion + double-promotion error, the feedback loop's file mutation, staleness/mark-used, redaction per secret category, transcript parsing against hand-written synthetic fixtures, staging heuristics including the `100+` regression, and the pending-review lifecycle), and 4 integration tests in `myelind` that spawn the real `myelind mcp` binary and drive it over actual stdio JSON-RPC (tool listing, the full observe→promote→feedback loop with real file assertions, the pending-review queue round-trip, and that bad input returns JSON-RPC errors rather than crashing). `myelin-core` currently has no tests of its own — its only tested logic (embeddings policy gating) was removed along with the feature. The `ingest-session` → redact → stage → review path has also been verified twice manually end to end: once against a synthetic transcript with an embedded fake secret (came out redacted), and once for real against this project's own development session (correctly parsed 454 tool calls, staged 2 genuine candidates, and surfaced the `100+` regex bug that got fixed above).
