use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use myelin_index::{NewObservation, Store, StoreConfig};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

#[derive(Parser)]
#[command(name = "myelin", about = "Myelin CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ping the running myelind daemon over its control socket.
    Status,
    /// Record an observation directly (bypasses the daemon — for debugging
    /// the same store the MCP `record_observation` tool writes to).
    Observe {
        #[arg(long)]
        title: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        context_signal: Option<String>,
        #[arg(long)]
        high_stakes: bool,
    },
    /// List warmup-queue candidates (not yet promoted).
    Queue,
    /// List promoted skills.
    Skills,
    /// Force-promote a candidate now.
    Promote { candidate_id: i64 },
    /// Record feedback on a promoted skill (correction appends into the
    /// live SKILL.md; confirmation just logs).
    Feedback {
        skill_id: i64,
        #[arg(long, value_parser = ["correction", "confirmation"])]
        kind: String,
        #[arg(long)]
        note: String,
    },
    /// Mark a skill as just used (what the `stale` flag is judged against).
    MarkUsed { skill_id: i64 },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Status => status(),
        Command::Observe {
            title,
            summary,
            project,
            context_signal,
            high_stakes,
        } => observe(title, summary, project, context_signal, high_stakes),
        Command::Queue => queue(),
        Command::Skills => skills(),
        Command::Promote { candidate_id } => promote(candidate_id),
        Command::Feedback {
            skill_id,
            kind,
            note,
        } => feedback(skill_id, kind, note),
        Command::MarkUsed { skill_id } => mark_used(skill_id),
    }
}

fn open_store() -> Result<Store> {
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

fn observe(
    title: String,
    summary: String,
    project: Option<String>,
    context_signal: Option<String>,
    high_stakes: bool,
) -> Result<()> {
    let store = open_store()?;
    let skills_dir = myelin_core::Paths::resolve().skills_dir();
    let result = store.record_observation(
        NewObservation {
            title,
            summary,
            project,
            context_signal,
            high_stakes,
        },
        &skills_dir,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn queue() -> Result<()> {
    let store = open_store()?;
    let candidates: Vec<_> = store
        .list_candidates()?
        .into_iter()
        .filter(|c| c.status == "warming")
        .collect();
    println!("{}", serde_json::to_string_pretty(&candidates)?);
    Ok(())
}

fn skills() -> Result<()> {
    let store = open_store()?;
    println!("{}", serde_json::to_string_pretty(&store.list_skills()?)?);
    Ok(())
}

fn promote(candidate_id: i64) -> Result<()> {
    let store = open_store()?;
    let skills_dir = myelin_core::Paths::resolve().skills_dir();
    let path = store.promote_candidate(candidate_id, &skills_dir)?;
    println!("promoted -> {path}");
    Ok(())
}

fn feedback(skill_id: i64, kind: String, note: String) -> Result<()> {
    let store = open_store()?;
    let result = store.record_skill_feedback(skill_id, &kind, &note)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn mark_used(skill_id: i64) -> Result<()> {
    let store = open_store()?;
    store.mark_skill_used(skill_id)?;
    println!("marked used");
    Ok(())
}

fn status() -> Result<()> {
    let paths = myelin_core::Paths::resolve();
    let socket_path = paths.control_socket();
    let mut stream = UnixStream::connect(&socket_path).with_context(|| {
        format!(
            "could not connect to {} — is `myelind serve` running?",
            socket_path.display()
        )
    })?;

    writeln!(stream, r#"{{"method":"status.get"}}"#)?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let value: serde_json::Value = serde_json::from_str(line.trim())?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
