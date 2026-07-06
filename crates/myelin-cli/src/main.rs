use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Status => status(),
    }
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
