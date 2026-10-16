//! Candidate-only bridge from the public update verb to the installed ecosystem.
//!
//! The platform coordinator calls the graphd-owned update plan/apply/rollback argv;
//! this bridge never performs engine placement. An explicit local kit is needed
//! because no signed or published container channel exists yet.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::apply::{Request, Subcommand};
use super::json::{self, Json};

fn runtime_python(root: &Path) -> Result<PathBuf, String> {
    let pointer = root.join("mcp-runtime/current.json");
    let bytes = std::fs::read_to_string(&pointer)
        .map_err(|error| format!("cannot read installed MCP runtime pointer: {error}"))?;
    let body = json::parse(&bytes).map_err(|_| "invalid MCP runtime pointer JSON".to_string())?;
    let generation = body
        .get("generation")
        .and_then(Json::as_text)
        .ok_or_else(|| "MCP runtime pointer has no generation".to_string())?;
    if generation.is_empty()
        || !generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("MCP runtime generation is not a portable name".to_string());
    }
    let python = root
        .join("mcp-runtime/versions")
        .join(generation)
        .join(if cfg!(windows) {
            "venv/Scripts/python.exe"
        } else {
            "venv/bin/python"
        });
    let generation_root = root.join("mcp-runtime/versions").join(generation);
    let resolved_root = generation_root
        .canonicalize()
        .map_err(|_| "installed MCP runtime generation is missing".to_string())?;
    let resolved = python
        .canonicalize()
        .map_err(|_| "installed MCP Python is missing".to_string())?;
    if !resolved.starts_with(&resolved_root) || !resolved.is_file() {
        return Err("installed MCP Python escapes its versioned runtime".to_string());
    }
    let expected = body
        .get("python_sha256")
        .and_then(Json::as_text)
        .ok_or_else(|| "MCP runtime pointer has no Python digest".to_string())?;
    let bytes = std::fs::read(&python)
        .map_err(|error| format!("cannot read installed MCP Python: {error}"))?;
    if super::sha256::digest_hex(&bytes) != expected {
        return Err("installed MCP Python differs from its pointer".to_string());
    }
    Ok(python)
}

/// Execute a local candidate coordinator from a named, manifest-checked kit.
/// The caller invokes this only with an active engine pointer and an explicit kit.
pub fn run(root: &Path, kit: &Path, request: &Request, json_output: bool) -> i32 {
    let error = |message: &str| {
        if json_output {
            println!(
                "{}",
                json::canonical_text(&Json::from_pairs(vec![
                    ("status", Json::text("refused")),
                    ("reason", Json::text(message)),
                    ("certified", Json::bool(false)),
                ]))
            );
        } else {
            eprintln!("axiom-cli: {message}");
        }
        2
    };
    if !kit.is_absolute() || kit.is_symlink() || !kit.is_dir() {
        return error("composite candidate kit must be an absolute ordinary directory");
    }
    let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let Some(home) = std::env::var_os(home_key).map(PathBuf::from) else {
        return error(&format!(
            "{home_key} is required for the per-user candidate"
        ));
    };
    if !home.is_absolute() {
        return error("per-user candidate home must be absolute");
    }
    let python = match runtime_python(root) {
        Ok(path) => path,
        Err(message) => return error(&message),
    };
    let mut process = Command::new(python);
    process
        .arg(kit.join("Update-Distribution.py"))
        .arg(request.subcommand.name())
        .arg("--release")
        .arg(kit.join("release"))
        .arg("--files")
        .arg(kit.join("candidate-files.json"))
        .arg("--runtime-input")
        .arg(kit.join("runtime-input.json"))
        .arg("--home")
        .arg(home)
        .arg("--root")
        .arg(root)
        .arg("--cli-installer")
        .arg(kit.join(if cfg!(windows) {
            "Install-AxiomCli.ps1"
        } else {
            "Install-AxiomCli.sh"
        }))
        .arg("--kit-manifest")
        .arg(kit.join("kit-manifest.json"))
        .env("PYTHONDONTWRITEBYTECODE", "1");
    match request.subcommand {
        Subcommand::Check => {}
        Subcommand::Plan => {
            if let Some(to) = &request.to {
                process.arg("--to").arg(to);
            }
            if let Some(out) = &request.out {
                if out != "-" {
                    process.arg("--out").arg(out);
                }
            }
        }
        Subcommand::Apply => {
            if let Some(path) = &request.plan_path {
                process.arg("--plan-file").arg(path);
            }
            if let Some(digest) = &request.approve_digest {
                process.arg("--approve-digest").arg(digest);
            }
        }
        Subcommand::Rollback => {
            if request.transaction.as_deref() != Some("previous") {
                return error("candidate rollback requires --transaction previous");
            }
        }
    }
    let output = match crate::progress::capture(
        &mut process,
        "Updating components",
        "Waiting for the approved runtime and component transaction",
    ) {
        Ok(output) => output,
        Err(error_message) => {
            return error(&format!(
                "cannot run candidate coordinator: {error_message}"
            ))
        }
    };
    if output.status.success() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    } else if json_output {
        print!("{}", String::from_utf8_lossy(&output.stderr));
    } else {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    output.status.code().unwrap_or(8)
}
