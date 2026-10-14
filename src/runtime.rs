//! MCP runtime provisioning from an extracted release set (task L-002).
//!
//! Before ADR-0033 a separate bootstrap script ran the owner provisioner and only then called
//! `axm install`. A first install through `axiom-cli` alone must place the MCP runtime too,
//! so this module runs the same provisioner the release already ships, with the same pinned
//! inputs. The provisioner script and its input record are bound into the install plan digest, so
//! the approval covers them. Invocation is program plus argv; no shell string is built.
//!
//! * Windows: `Provision-McpRuntime.ps1` with `runtime-input.json` (embedded CPython, pip wheel,
//!   offline MCP inputs), run by Windows PowerShell 5.1.
//! * POSIX: `runtime/provision.py` with `runtime/manifest.json`, run by the host `python3`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::update::error::Refusal;
use crate::update::json::{self, Json};
use crate::update::state;

/// The provisioning inputs an extracted release carries for this host.
#[derive(Clone, Debug)]
pub struct Inputs {
    kind: &'static str,
    release: PathBuf,
    record: PathBuf,
    provisioner: PathBuf,
}

/// Find the release's runtime inputs, if it ships any for this host.
pub fn locate(release: &Path) -> Option<Inputs> {
    let (kind, record, provisioner) = if cfg!(windows) {
        (
            "windows-embedded-python",
            release.join("runtime-input.json"),
            release.join("Provision-McpRuntime.ps1"),
        )
    } else {
        (
            "posix-python-provision",
            release.join("runtime").join("manifest.json"),
            release.join("runtime").join("provision.py"),
        )
    };
    (record.is_file() && provisioner.is_file()).then(|| Inputs {
        kind,
        release: release.to_path_buf(),
        record,
        provisioner,
    })
}

/// The plan block that binds the provisioner and its inputs into the approval digest.
pub fn plan_block(inputs: &Inputs) -> Result<Json, Refusal> {
    Ok(Json::from_pairs(vec![
        ("kind", Json::text(inputs.kind)),
        (
            "input_record_sha256",
            Json::text(&state::digest_file(
                &inputs.record,
                "runtime_input_unreadable",
            )?),
        ),
        (
            "provisioner_sha256",
            Json::text(&state::digest_file(
                &inputs.provisioner,
                "runtime_provisioner_unreadable",
            )?),
        ),
    ]))
}

/// Whether the install root already has a provisioned runtime generation.
pub fn provisioned(root: &Path) -> bool {
    root.join("mcp-runtime").join("current.json").is_file()
        || crate::legacy::nested_runtime_pointer(root).is_some()
}

/// Run the provisioner into `<root>/mcp-runtime`, after re-checking the bound digests.
pub fn provision(
    inputs: &Inputs,
    bound: &Json,
    root: &Path,
    version: &str,
) -> Result<Json, Refusal> {
    let observed = plan_block(inputs)?;
    if &observed != bound {
        return Err(Refusal::conflict(
            "runtime_inputs_changed",
            "the MCP runtime provisioner or its input record changed after the plan was approved",
        ));
    }
    let record = json::parse(&std::fs::read_to_string(&inputs.record).map_err(|error| {
        Refusal::io(
            "runtime_input_unreadable",
            &inputs.record.display().to_string(),
            &error,
        )
    })?)
    .map_err(|error| Refusal::validation("runtime_input_invalid", error))?;
    let text = |path: &[&str]| -> Result<String, Refusal> {
        let mut value = &record;
        for key in path {
            value = value.get(key).ok_or_else(|| {
                Refusal::validation(
                    "runtime_input_invalid",
                    format!("the runtime input record has no `{}`", path.join(".")),
                )
            })?;
        }
        value.as_text().map(str::to_string).ok_or_else(|| {
            Refusal::validation(
                "runtime_input_invalid",
                format!(
                    "`{}` in the runtime input record is not text",
                    path.join(".")
                ),
            )
        })
    };
    let target = root.join("mcp-runtime");
    let release = &inputs.release;
    let mut command = if cfg!(windows) {
        let basename = |value: String| value.rsplit('/').next().unwrap_or("").to_string();
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&inputs.provisioner)
            // The Windows provisioner appends `mcp-runtime` to its root itself; passing
            // `<root>\mcp-runtime` produced the nested `mcp-runtime\mcp-runtime` of 0.1.2.
            .args(["-Action", "provision", "-Root"])
            .arg(root)
            .arg("-RuntimeArchive")
            .arg(release.join(basename(text(&["python", "url"])?)))
            .arg("-RuntimeSha256")
            .arg(text(&["python", "sha256"])?)
            .arg("-PipWheel")
            .arg(release.join(format!(
                "pip-{}-py3-none-any.whl",
                text(&["pip", "version"])?
            )))
            .arg("-PipSha256")
            .arg(text(&["pip", "sha256"])?)
            .arg("-Inputs")
            .arg(release.join(basename(text(&["mcp_inputs", "artifact"])?)))
            .arg("-InputsSha256")
            .arg(text(&["mcp_inputs", "sha256"])?)
            .arg("-SourceRevision")
            .arg(text(&["mcp_inputs", "source_revision"])?)
            .arg("-Version")
            .arg(version);
        command
    } else {
        let mut command = Command::new("python3");
        command
            .arg(&inputs.provisioner)
            .args(["provision", "--root"])
            .arg(&target)
            .args(["--version", version, "--source-revision"])
            .arg(text(&["mcp_revision"])?)
            .env("PYTHONNOUSERSITE", "1");
        for (key, name) in [
            ("runtime", "python.tar.gz"),
            ("wheelhouse", "wheelhouse.tar.gz"),
            ("wheel", &*format!("axiom_mcp-{version}-py3-none-any.whl")),
            ("lock", "requirements.txt"),
        ] {
            command
                .arg(format!("--{key}"))
                .arg(release.join("runtime").join(name))
                .arg(format!("--{key}-sha256"))
                .arg(text(&["files", name])?);
        }
        command
    };
    let output = command.output().map_err(|error| {
        Refusal::not_ready(
            "runtime_provisioner_unavailable",
            format!(
                "the MCP runtime provisioner could not be started ({error}); {}",
                if cfg!(windows) {
                    "Windows PowerShell 5.1 (powershell.exe) is required"
                } else {
                    "a host `python3` is required to run the release provisioner"
                }
            ),
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(Refusal::not_ready(
            "runtime_provision_failed",
            format!(
                "the MCP runtime provisioner exited {:?}: {}",
                output.status.code(),
                tail(&format!("{stdout}{stderr}"), 800)
            ),
        ));
    }
    Ok(Json::from_pairs(vec![
        ("kind", Json::text(inputs.kind)),
        ("root", Json::text(&target.display().to_string())),
        ("status", Json::text("provisioned")),
    ]))
}

fn tail(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    let start = trimmed.len().saturating_sub(limit);
    let start = (start..=trimmed.len())
        .find(|index| trimmed.is_char_boundary(*index))
        .unwrap_or(trimmed.len());
    trimmed[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_without_runtime_inputs_needs_no_provisioning() {
        let dir = std::env::temp_dir().join(format!("axiom-runtime-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(locate(&dir).is_none());
    }

    #[test]
    fn changed_inputs_after_approval_are_refused_before_running_anything() {
        let dir = std::env::temp_dir().join(format!("axiom-runtime-bound-{}", std::process::id()));
        let (record, provisioner) = if cfg!(windows) {
            (
                dir.join("runtime-input.json"),
                dir.join("Provision-McpRuntime.ps1"),
            )
        } else {
            (
                dir.join("runtime/manifest.json"),
                dir.join("runtime/provision.py"),
            )
        };
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&record, b"{}").unwrap();
        std::fs::write(&provisioner, b"# provisioner").unwrap();
        let inputs = locate(&dir).expect("inputs found");
        let bound = plan_block(&inputs).unwrap();
        std::fs::write(&provisioner, b"# changed").unwrap();
        let refusal = provision(&inputs, &bound, &dir.join("root"), "0.1.2").unwrap_err();
        assert_eq!(refusal.reason, "runtime_inputs_changed");
        assert!(!dir.join("root").exists());
    }

    #[test]
    fn tail_keeps_the_end_on_a_character_boundary() {
        assert_eq!(tail("  abcdef ", 3), "def");
        assert_eq!(tail("ไทย", 4), "ย");
    }
}
