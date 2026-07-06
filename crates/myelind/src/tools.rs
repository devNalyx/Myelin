use myelin_index::{NewObservation, Store, StoreConfig};
use serde_json::{json, Value};

fn open_store() -> anyhow::Result<Store> {
    let paths = myelin_core::Paths::resolve();
    let config = myelin_core::Config::load(&paths.config_file())?;
    Store::open(
        &paths.db_file(),
        StoreConfig {
            promotion_reps: config.promotion.reps,
            similarity_threshold: config.promotion.similarity_threshold,
            stale_after_secs: config.atrophy.stale_after_secs,
        },
    )
}

fn skills_dir() -> std::path::PathBuf {
    myelin_core::Paths::resolve().skills_dir()
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "record_observation",
            "description": "Record a noteworthy, domain-specific procedure you (the calling agent) just performed or noticed — NOT routine tool use. Call this when something feels like it's worth remembering across sessions: a non-obvious fix, a team/org-specific convention, a workaround. Similar observations accumulate reps in a warmup queue and auto-promote into a real Claude Code Skill once they recur enough, or immediately if flagged high_stakes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short name for the procedure, e.g. 'apply DB migration hotfix across services'." },
                    "summary": { "type": "string", "description": "The goal and the steps taken, generalized away from this specific repo's particulars." },
                    "project": { "type": "string", "description": "Optional: which project/repo this was observed in." },
                    "context_signal": { "type": "string", "description": "Optional: why this looks reusable beyond this one instance, e.g. a ticket saying it needs to be applied fleet-wide." },
                    "high_stakes": { "type": "boolean", "description": "Set true to fast-track promotion off a single observation, when context_signal makes the future reuse obvious." }
                },
                "required": ["title", "summary"]
            }
        },
        {
            "name": "list_warmup_queue",
            "description": "List skill candidates still accumulating reps (not yet promoted to a real skill), with their rep counts.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_skills",
            "description": "List skills that have been promoted (auto or manual), with provenance: how many observations backed it, why it was promoted, and where the SKILL.md lives.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "promote_skill",
            "description": "Force-promote a warmup-queue candidate into a real skill immediately, bypassing the reps/high-stakes gate.",
            "inputSchema": {
                "type": "object",
                "properties": { "candidate_id": { "type": "integer" } },
                "required": ["candidate_id"]
            }
        },
        {
            "name": "record_skill_feedback",
            "description": "Report feedback on a promoted skill after actually using it. Call this whenever you follow an existing skill and it turns out wrong or incomplete (kind='correction' — this appends your note directly into the live SKILL.md, so the skill itself improves) or it worked exactly as written (kind='confirmation' — logged to build confidence, doesn't touch the file). This is what keeps a promoted skill a living document instead of a static one-shot artifact.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "skill_id": { "type": "integer", "description": "From list_skills." },
                    "kind": { "type": "string", "enum": ["correction", "confirmation"] },
                    "note": { "type": "string", "description": "For a correction: what was wrong and what actually worked. For a confirmation: brief context on what was done." }
                },
                "required": ["skill_id", "kind", "note"]
            }
        },
        {
            "name": "mark_skill_used",
            "description": "Record that a promoted skill was just followed/invoked, independent of whether you also have feedback to give. This is what list_skills' `stale` flag is judged against - call it whenever you actually use an existing skill, even if it worked perfectly and you have nothing to correct.",
            "inputSchema": {
                "type": "object",
                "properties": { "skill_id": { "type": "integer", "description": "From list_skills." } },
                "required": ["skill_id"]
            }
        },
        {
            "name": "list_pending_review",
            "description": "List redacted, heuristically-flagged excerpts staged automatically from past session transcripts (via a SessionEnd hook) - candidates that MIGHT be worth an observation, not yet judged by any agent. Review each one: if it's genuinely worth capturing, call record_observation yourself based on it, then dismiss_pending_review it. If it's not worth it, just dismiss it.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "dismiss_pending_review",
            "description": "Clear a staged item from the pending-review queue, whether or not you turned it into an observation. Keeps the queue from accumulating things already handled.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "From list_pending_review." } },
                "required": ["id"]
            }
        }
    ])
}

pub fn call(params: Value) -> anyhow::Result<Value> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing tool name"))?;
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    match name {
        "record_observation" => {
            let store = open_store()?;
            let input = NewObservation {
                title: field_str(&args, "title")?,
                summary: field_str(&args, "summary")?,
                project: args
                    .get("project")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                context_signal: args
                    .get("context_signal")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                high_stakes: args
                    .get("high_stakes")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            };
            let result = store.record_observation(input, &skills_dir())?;
            Ok(serde_json::to_value(result)?)
        }
        "list_warmup_queue" => {
            let store = open_store()?;
            let candidates = store.list_candidates()?;
            Ok(
                json!({ "candidates": candidates.into_iter().filter(|c| c.status == "warming").collect::<Vec<_>>() }),
            )
        }
        "list_skills" => {
            let store = open_store()?;
            Ok(json!({ "skills": store.list_skills()? }))
        }
        "promote_skill" => {
            let store = open_store()?;
            let candidate_id = args
                .get("candidate_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing candidate_id"))?;
            let path = store.promote_candidate(candidate_id, &skills_dir())?;
            Ok(json!({ "promoted": true, "skill_path": path }))
        }
        "record_skill_feedback" => {
            let store = open_store()?;
            let skill_id = args
                .get("skill_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing skill_id"))?;
            let kind = field_str(&args, "kind")?;
            let note = field_str(&args, "note")?;
            let result = store.record_skill_feedback(skill_id, &kind, &note)?;
            Ok(serde_json::to_value(result)?)
        }
        "mark_skill_used" => {
            let store = open_store()?;
            let skill_id = args
                .get("skill_id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing skill_id"))?;
            store.mark_skill_used(skill_id)?;
            Ok(json!({ "marked_used": true }))
        }
        "list_pending_review" => {
            let store = open_store()?;
            Ok(json!({ "pending": store.list_pending_review()? }))
        }
        "dismiss_pending_review" => {
            let store = open_store()?;
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| anyhow::anyhow!("missing id"))?;
            store.dismiss_pending_review(id)?;
            Ok(json!({ "dismissed": true }))
        }
        other => anyhow::bail!("unknown tool: {other}"),
    }
}

fn field_str(args: &Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required field: {key}"))
}
