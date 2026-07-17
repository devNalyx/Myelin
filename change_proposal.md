# Change Proposal: unbounded skill growth + tool-schema token footprint

**Status:** proposal, not yet implemented
**Context:** Myelin's stated goal is to improve *localized* skills — learn from an
agent's behavior and turn recurring patterns into reusable Skills, cheaply. In
practice it costs a fixed **~2.9k tokens of MCP tool schemas** on every session start,
plus an **unbounded and growing** amount of system-prompt space for every
auto-promoted skill it creates, with no automatic pruning. The first cost is fixed and
tolerable; the second gets worse forever unless a human manually intervenes, which
works against the "improve performance, reduce token burn" goal as the skill library
grows. Findings below, ordered by long-term impact.

## 1. Unbounded skill promotion, no automatic pruning (highest impact, grows over time)

`draft_and_write` (`crates/myelin-index/src/skillfile.rs:46-83`) writes a new
`SKILL.md` every time a skill is promoted (`promote_internal`,
`crates/myelin-index/src/store.rs:371-410`). Each promoted skill is then loaded as an
available skill in *every future session* — this is exactly the "Auto-promoted by
Myelin (context_signal) after 1 observation(s)" entry that showed up in this session
after a single observation.

There is no cap on how many skills can be live at once, and no automatic archival.
Grepped `store.rs:140-144, 210, 466, 548-556` — the only related logic is a `stale`
flag `list_skills` computes from `stale_after_secs` (`store.rs:466`), which is purely
informational. Moving a skill out of the live set requires an explicit
`archive_skill`/`restore_skill` call (`store.rs:547-556`) — nothing in the codebase
ever calls this automatically. README §2/§5/§7 confirm this is intentional
("never automatic"), but the consequence is that the live skill count — and therefore
the system-prompt tokens spent on skills — can only grow, session after session,
unless a human or agent remembers to prune.

**Proposal:**
- Add a soft cap (e.g. config `max_active_skills`, default ~20-30) enforced at
  promotion time: once exceeded, auto-archive the least-recently-used skill (by
  `list_skills`' existing staleness computation) rather than blocking promotion.
- Surface staleness more assertively: e.g. have `list_pending_review` or a periodic
  hook *suggest* archival candidates instead of only exposing `stale` as a passive
  flag nothing acts on.
- This is the single highest-leverage fix — the ~2.9k-token tool-schema cost (below)
  is fixed and one-time-per-session; unpruned skill growth is compounding.

## 2. `record_observation` / `record_skill_feedback` are the two largest tool schemas — trim the prose

`tools.rs:24-37` (`record_observation`) and `tools.rs:58-69`
(`record_skill_feedback`) are large primarily because their `description` fields are
full paragraphs justifying *when and why* to call the tool (~473 and ~336 tokens
respectively — larger than any other tool in either MCP server connected this
session). Neither has embedded examples or deep nesting; `record_observation` also
individually describes all 5 of its input fields, and `record_skill_feedback` re-states
the consequence of each `kind` enum branch in the top-level description as well as
implicitly in the field.

**Proposal:** cut both descriptions to a single sentence stating the mechanical
contract (what gets written, what triggers what), and move the "when should I call
this" guidance to README/docs where it can be read once instead of re-sent as tool
schema every session. Target: ~473→~150 tokens and ~336→~120 tokens, a combined
savings of roughly 540 tokens — about 20% of this server's entire schema footprint —
from two tools.

## 3. Redundant caveat duplicated across tools

The caveat "the `stale` flag is informational only, it never triggers archival
automatically" appears near-verbatim in both `mark_skill_used` (`tools.rs:72`) and
`archive_skill` (`tools.rs:95`), and is also implied in `record_observation`'s
proximity to promotion logic. State it once — ideally in `list_skills`' description,
since that's where `stale` is actually surfaced — and drop it from the others.

## 4. Documentation bug: README understates the tool count

`README.md` §8 states the MCP server exposes "8 tools" over stdio JSON-RPC.
`tools.rs:21-121` actually defines **11** tools (`archive_skill`,
`dismiss_pending_review`, `list_pending_review`, `list_skills`, `list_warmup_queue`,
`mark_skill_used`, `promote_skill`, `record_observation`, `record_skill_feedback`,
`render_skill_graph`, `restore_skill`). This is a plain factual bug — someone reading
the README to estimate the server's footprint (as this investigation initially tried
to do) gets a number 27% below reality. Fix: update the count, or better, generate it
from `tool_definitions().len()` at doc-build/test time so it can't drift again.

## 5. Lazy/on-demand tool exposure — gap (same class of issue as NexusContext)

No mechanism exists to expose a subset of the 11 tools. `tools/list`
(`mcp.rs:65`) always returns the full static array from `tool_definitions()`
(`tools.rs:21-121`); there's no config-driven filtering. A session that's purely
*consuming* skills (not actively curating them) doesn't need
`record_skill_feedback`, `render_skill_graph`, `dismiss_pending_review`, etc. every
time.

**Proposal:** mirror the NexusContext proposal — a `[tools]` config section with a
preset (e.g. `"consume-only"` exposing just `list_skills`/`mark_skill_used`, vs.
`"full"` exposing all 11) so a session that isn't doing skill curation doesn't pay for
schemas it won't call.

## 6. Ruled out: no dynamic/cache-breaking content, no LLM cost in the SessionEnd hook

Two hypotheses worth documenting so they aren't re-investigated later as phantom bugs:

- **Tool schema cache-invalidation:** `tools/list` and `initialize`
  (`mcp.rs:60-64,65`) are built entirely from static `json!()` literals plus
  `env!("CARGO_PKG_VERSION")` — no skill counts, names, or timestamps are
  interpolated into the schema itself. Schema content is cache-stable across
  sessions. (Note: an individual skill's *file content* can change across sessions
  when a correction is appended with a live timestamp — `skillfile.rs:89-98` — but
  this is a rare, event-driven change to that one skill's file, not a per-turn schema
  change.)
- **`myelin ingest-session` hook cost:** `ingest_session()`
  (`crates/myelin-cli/src/main.rs:182-213`) is confirmed purely local — regex/heuristic
  transcript parsing (`staging.rs`) writing to SQLite, no network client anywhere in
  the dependency tree (`myelin-index/Cargo.toml:6-11`: only `rusqlite`, `anyhow`,
  `serde`, `serde_json`, `chrono`, `regex`). Explicitly commented as "deliberately NOT
  an LLM call" (`staging.rs:7`) and designed to never block or error
  (`main.rs:180-184`). Zero token/LLM cost per session end — safe to keep enabled.

## Priority order

1. Skill-promotion cap + auto-archival (#1) — the only *unbounded* cost; fix before it
   compounds further.
2. Trim `record_observation`/`record_skill_feedback` (#2) — mechanical, ~540 tokens,
   low risk.
3. `[tools]` preset config (#5) — same pattern as the NexusContext proposal, biggest
   remaining fixed-cost lever.
4. Dedupe the stale-flag caveat (#3) and fix the README tool count (#4) — small,
   low-risk cleanups.
