//! Drives the real `myelind mcp` binary as a subprocess over its actual
//! stdio JSON-RPC transport - everything so far had only been verified by
//! hand-piping JSON-RPC or via myelin-index's unit tests, never as an
//! automated check that the wire protocol itself behaves.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl McpSession {
    fn start(data_dir: &std::path::Path, skills_dir: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_myelind"))
            .arg("mcp")
            .env("MYELIN_DATA_DIR", data_dir)
            .env("MYELIN_SKILLS_DIR", skills_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn myelind mcp");

        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();

        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(line.trim()).unwrap()
    }

    fn tool(&mut self, id: i64, name: &str, arguments: Value) -> Value {
        let response = self.call(
            id,
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        response["result"].clone()
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        // Dropping stdin closes the pipe, which ends the child's
        // `stdin.lines()` loop so it exits cleanly instead of hanging.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn scratch_dirs(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("myelind-it-{label}-{nanos}"));
    (root.join("data"), root.join("skills"))
}

#[test]
fn initialize_and_tools_list_match_the_real_dispatch() {
    let (data_dir, skills_dir) = scratch_dirs("init-list");
    let mut session = McpSession::start(&data_dir, &skills_dir);

    let init = session.call(1, "initialize", json!({}));
    assert_eq!(init["result"]["serverInfo"]["name"], "myelin");

    let list = session.call(2, "tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "record_observation",
        "list_warmup_queue",
        "list_skills",
        "promote_skill",
        "record_skill_feedback",
    ] {
        assert!(names.contains(&expected), "missing tool: {expected}");
    }
}

#[test]
fn full_loop_over_the_real_protocol_promotes_and_accepts_feedback() {
    let (data_dir, skills_dir) = scratch_dirs("full-loop");
    let mut session = McpSession::start(&data_dir, &skills_dir);

    let title = "apply db migration hotfix";
    let summary = "run migrate.sh then restart service then verify health";

    session.tool(
        1,
        "record_observation",
        json!({ "title": title, "summary": summary }),
    );
    session.tool(
        2,
        "record_observation",
        json!({ "title": title, "summary": summary }),
    );
    let third = session.tool(
        3,
        "record_observation",
        json!({ "title": title, "summary": summary }),
    );

    assert_eq!(third["promoted"], true);
    let skill_path = third["skill_path"].as_str().unwrap().to_string();
    assert!(std::path::Path::new(&skill_path).exists());

    let skills = session.tool(4, "list_skills", json!({}));
    let skill_id = skills["skills"][0]["id"].as_i64().unwrap();

    let feedback = session.tool(
        5,
        "record_skill_feedback",
        json!({ "skill_id": skill_id, "kind": "correction", "note": "also check downstream caches" }),
    );
    assert_eq!(feedback["correction_count"], 1);

    let content = std::fs::read_to_string(&skill_path).unwrap();
    assert!(content.contains("also check downstream caches"));

    let queue = session.tool(6, "list_warmup_queue", json!({}));
    assert_eq!(queue["candidates"].as_array().unwrap().len(), 0);
}

#[test]
fn unknown_tool_and_unknown_method_return_json_rpc_errors_not_crashes() {
    let (data_dir, skills_dir) = scratch_dirs("errors");
    let mut session = McpSession::start(&data_dir, &skills_dir);

    let bad_method = session.call(1, "not/a/real/method", json!({}));
    assert!(bad_method.get("error").is_some());

    let bad_tool = session.call(
        2,
        "tools/call",
        json!({ "name": "does_not_exist", "arguments": {} }),
    );
    assert!(bad_tool.get("error").is_some());
}
