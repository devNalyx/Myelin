use myelin_index::{NewObservation, Store};
use serde_json::{json, Value};

fn open_store() -> anyhow::Result<Store> {
    let paths = myelin_core::Paths::resolve();
    Store::open(&paths.db_file())
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
                project: args.get("project").and_then(|v| v.as_str()).map(str::to_string),
                context_signal: args
                    .get("context_signal")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                high_stakes: args.get("high_stakes").and_then(|v| v.as_bool()).unwrap_or(false),
            };
            let result = store.record_observation(input, &skills_dir())?;
            Ok(serde_json::to_value(result)?)
        }
        "list_warmup_queue" => {
            let store = open_store()?;
            let candidates = store.list_candidates()?;
            Ok(json!({ "candidates": candidates.into_iter().filter(|c| c.status == "warming").collect::<Vec<_>>() }))
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
        other => anyhow::bail!("unknown tool: {other}"),
    }
}

fn field_str(args: &Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required field: {key}"))
}
