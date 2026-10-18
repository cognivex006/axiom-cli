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
    if !matches!(name.as_str(), "axsk" | "axmp") {
        let prefix = args
            .iter()
            .take_while(|a| *a == "--json" || *a == "--verbose")
            .count();
        if let Some(group) = args.get(prefix).and_then(|a| a.to_str()) {
            if group == "skills" {
                let mut forwarded = args[prefix + 1..].to_vec();
                forwarded.extend_from_slice(&args[..prefix]);
                let legacy = forwarded.first().and_then(|a| a.to_str());
                if matches!(legacy, Some("version" | "check"))
                    || (legacy == Some("update")
                        && forwarded
                            .get(1)
                            .is_some_and(|a| a == "plan" || a == "apply"))
                {
                    return Some(skills(&forwarded));
                }
                return Some(crate::host_skills::run(&forwarded));
            }
            if group == "mcp" {
                let mut forwarded = args[prefix + 1..].to_vec();
                forwarded.extend_from_slice(&args[..prefix]);
                return Some(mcp(&forwarded));
            }
        }
    }
    match name.as_str() {
        "axsk" => {
            if !args.iter().any(|a| a == "--help" || a == "-h") {
                eprintln!("axsk terminal compatibility is deprecated; use axm skills. Invoke $axsk in your agent.");
            }
            Some(skills(args))
        }
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
        println!("axsk - deprecated terminal compatibility for Axiom Skills Manager\nUse axm skills for agent installation and management.\nUse $axsk <intent> inside your agent.\nLegacy terminal verbs: version, check, update plan/apply.\n{}", crate::host_skills::HELP);
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
        println!("axmp - Axiom MCP Server / Gateway\n\nUSAGE: axmp [serve|start|list|status|stop|stdio|http|version|doctor|update|entrypoints] [options]\nserve: named HTTP gateway; three-hour idle default, optional owner PID.\nDefault: stdio (compatibility). HTTP requires explicit Host/Origin allowlists.\nBackground lifecycle: axm mcp start|list|status|stop.\nRun axmp <command> --help for command options.");
        return 0;
    }
    let (module, forwarded) =
        match args.first().and_then(|arg| arg.to_str()) {
            Some("serve" | "start" | "list" | "status" | "stop") => ("axiom_mcp.gateway", args),
            Some("stdio") => ("axiom_mcp.stdio", &args[1..]),
            Some("http") => ("axiom_mcp.http", &args[1..]),
            Some("entrypoints") => ("axiom_mcp.entrypoints", &args[1..]),
            Some("version" | "doctor" | "update") => ("axiom_mcp.cli", args),
            None => ("axiom_mcp.stdio", args),
            Some(_) => return failure(
                "axmp",
                Refusal::validation(
                    "command_unknown",
                    "expected serve, start, list, status, stop, stdio, http, version, doctor, update or entrypoints; use axmp --help",
                ),
                args,
            ),
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

/// Read-only component diagnostics through the provisioned runtime, never PATH Python.
pub fn mcp_diagnostics() -> Result<Option<crate::update::json::Json>, String> {
    let Some(root) = state::default_root() else {
        return Ok(None);
    };
    let Some(bin) = crate::lifecycle::provisioned_mcp_runtime_bin(&root)
        .map_err(|_| "runtime unavailable".to_string())?
    else {
        return Ok(None);
    };
    let mut command = Command::new(bin.join(crate::layout::program_file("python")));
    command
        .args(["-I", "-m", "axiom_mcp.cli", "doctor", "--json"])
        .env("AXIOM_PROGRESS", "off");
    let output = crate::progress::bounded_output(&mut command, std::time::Duration::from_secs(15))
        .map_err(|_| "component diagnostics unavailable".to_string())?;
    if output.stdout.len() > 1024 * 1024 {
        return Err("component diagnostics exceeded limit".to_string());
    }
    crate::update::json::parse(
        std::str::from_utf8(&output.stdout)
            .map_err(|_| "invalid component diagnostics".to_string())?,
    )
    .map(Some)
}
