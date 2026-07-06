use serde_json::{json, Value};

/// No tools yet — this is a scaffold build. The first real ones (per the
/// design sketch) will be things like `warmup_queue_list`, `skill_search`,
/// and `skill_promote`, once the ingestion/extraction pipeline exists.
pub fn tool_definitions() -> Value {
    json!([])
}

pub fn call(_params: Value) -> anyhow::Result<Value> {
    anyhow::bail!("no tools implemented yet — this is a scaffold build")
}
