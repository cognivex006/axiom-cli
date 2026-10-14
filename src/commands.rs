//! Native public entrypoint routing; no shell, SDK implementation or hidden approval.
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use crate::engine::{self, Located};
use crate::update::error::Refusal;
use crate::update::report::Report;
use crate::update::state;

pub fn run(arguments: &[OsString]) -> Option<i32> {
    let name = Path::new(arguments.first()?)
        .file_stem()?
        .to_str()?
        .to_ascii_lowercase();
    let args = &arguments[1..];
    match name.as_str() {
        "axsk" => Some(skills(args)),
        "axmp" => Some(mcp(args)),
        _ if args.first().and_then(|s| s.to_str()).is_some_and(|s| {
            matches!(
                s,
                "service" | "host" | "bootstrap" | "support-bundle" | "engine"
            )
        }) =>
        {
            Some(engine_command(
                "axm",
                if args.first().is_some_and(|arg| arg == "engine") {
                    &args[1..]
                } else {
                    args
                },
            ))
        }
        _ => None,
    }
}

fn failure(name: &str, refusal: Refusal, args: &[OsString]) -> i32 {
    let code = refusal.class.exit_code();
    if args.iter().any(|arg| arg == "--json") {
        Report::new(code, refusal.class.status(), refusal.message).emit(true, false)
    } else {
        eprintln!("{name}: {}", refusal.message);
        code
    }
}

fn instance(command: &mut Command) {
    if std::env::var_os("AXIOM_HOME").is_none() {
        if let Some(root) = state::default_root() {
            command.env("AXIOM_HOME", root);
        }
    }
}

fn execute(name: &str, mut command: Command, args: &[OsString]) -> i32 {
    instance(&mut command);
    match command.status() {
        Ok(status) => status.code().unwrap_or(crate::cli::exit::IO_INTERNAL),
        Err(error) => failure(
            name,
            Refusal::not_ready(
                "command_unlaunchable",
                format!("runtime could not start: {error}; run axm doctor"),
            ),
            args,
        ),
    }
}

fn engine_command(name: &str, args: &[OsString]) -> i32 {
    match engine::locate() {
        Located::Found(engine) => {
            let mut command = Command::new(engine.program());
            command.args(args);
            execute(name, command, args)
        }
        Located::Missing(_) => failure(
            name,
            Refusal::not_ready(
                "engine_missing",
                "Graph Engine runtime is missing; run axm doctor",
            ),
            args,
        ),
    }
}

fn skills(args: &[OsString]) -> i32 {
    let command = args
        .iter()
        .filter_map(|arg| arg.to_str())
        .find(|arg| !arg.starts_with('-'));
    if command.is_some_and(|command| !matches!(command, "version" | "check" | "update")) {
        return failure(
            "axsk",
            Refusal::validation(
                "command_unknown",
                "expected version, check or update; use axsk --help",
            ),
            args,
        );
    }
    if args.is_empty() || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("axsk - Axiom Skills Manager\n\nUSAGE: axsk <command> [options]\n\nCOMMANDS:\n  version\n  check [--updates] [--bundle <dir>]\n  update plan [--to <version>] [--out <file>] [--bundle <dir>]\n  update apply --plan <file> [--approve-digest <sha256>]\n\nGLOBAL OPTIONS: --json, --help\nInstall or repair the complete ecosystem with axm install.");
        return 0;
    }
    let mut forwarded = vec![OsString::from("skills")];
    forwarded.extend_from_slice(args);
    engine_command("axsk", &forwarded)
}

fn mcp(args: &[OsString]) -> i32 {
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!("axmp - Axiom MCP Server / Gateway\n\nUSAGE: axmp [stdio|http|version|doctor|update] [options]\nDefault: stdio. HTTP requires explicit Host/Origin allowlists.\nRun axmp <command> --help for command options.");
        return 0;
    }
    let (module, forwarded) = match args.first().and_then(|arg| arg.to_str()) {
        Some("stdio") => ("axiom_mcp.stdio", &args[1..]),
        Some("http") => ("axiom_mcp.http", &args[1..]),
        Some("version" | "doctor" | "update") => ("axiom_mcp.cli", args),
        None => ("axiom_mcp.stdio", args),
        Some(_) => {
            return failure(
                "axmp",
                Refusal::validation(
                    "command_unknown",
                    "expected stdio, http, version, doctor or update; use axmp --help",
                ),
                args,
            )
        }
    };
    let Some(root) = state::default_root() else {
        return failure(
            "axmp",
            Refusal::not_ready(
                "install_missing",
                "MCP runtime is not installed; run axm install",
            ),
            args,
        );
    };
    let bin = match crate::lifecycle::provisioned_mcp_runtime_bin(&root) {
        Ok(Some(bin)) => bin,
        Ok(None) => {
            return failure(
                "axmp",
                Refusal::not_ready(
                    "runtime_missing",
                    "MCP runtime is not installed; run axm install",
                ),
                args,
            )
        }
        Err(refusal) => return failure("axmp", refusal, args),
    };
    let mut command = Command::new(bin.join(crate::layout::program_file("python")));
    command.args(["-I", "-m", module]).args(forwarded);
    execute("axmp", command, args)
}
