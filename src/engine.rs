//! Discovery of, and delegation to, the `axiom-graphd` installation engine.
//!
//! The distribution contract puts the installation engine, the bootstrap rules, the service
//! lifecycle and the per-component update transaction in `axiom-graphd`. `axiom-cli` is a
//! distribution layer: where a behaviour already exists in the engine, this CLI invokes it
//! through the engine's documented argv surface and reports its result. It never forks the
//! engine and never re-implements it.
//!
//! Two rules shape this module:
//!
//! * Invocation is program plus argv. Nothing here builds a shell string, so an argument can
//!   never be re-parsed as a command.
//! * A missing engine is a *not found* condition with the exact places that were searched, not a
//!   silent success and not a blanket "not ready". The operator is told which binary to place.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::update::error::{Class, Refusal};

/// Environment override naming the engine binary explicitly.
pub const ENGINE_ENV: &str = "AXIOM_ENGINE_BIN";
/// The engine's program name, from `axiom-graphd` (`axiom.exe` on Windows).
pub const ENGINE_PROGRAM: &str = "axiom";

/// Where an engine binary was found.
#[derive(Clone, Debug)]
pub struct Engine {
    program: PathBuf,
    source: &'static str,
}

impl Engine {
    /// The resolved program path.
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// How the program was resolved, for the report and for evidence.
    pub fn source(&self) -> &'static str {
        self.source
    }

    /// Run the engine with argv and capture its result.
    ///
    /// A non-zero engine exit code is data, not an I/O error: the caller maps it into the shared
    /// exit vocabulary. Only a failure to *launch* the program is an I/O refusal.
    pub fn invoke(&self, args: &[String]) -> Result<Outcome, Refusal> {
        self.invoke_with_env(args, &[])
    }

    /// Run the engine with argv and an explicit environment, and capture its result.
    ///
    /// The engine resolves its install root from `AXIOM_HOME`, so a caller that owns a
    /// distribution install root must hand it over explicitly rather than inherit whatever the
    /// operator's shell happened to export. Only the named variables are set; everything else is
    /// inherited, so the engine still sees `PATH` (it probes for a Python interpreter there).
    pub fn invoke_with_env(
        &self,
        args: &[String],
        env: &[(&str, String)],
    ) -> Result<Outcome, Refusal> {
        let mut command = Command::new(&self.program);
        command.args(args);
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command.output().map_err(|error| {
            Refusal::io(
                "engine_unlaunchable",
                &self.program.display().to_string(),
                &error,
            )
        })?;
        Ok(Outcome {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Use the engine CLI from a release set after the distribution has verified its digest.
/// A declared but unusable artifact must not silently fall back to a checkout on PATH.
pub fn from_verified_artifact(path: &Path) -> Result<Engine, Refusal> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        Refusal::io(
            "engine_artifact_missing",
            &path.display().to_string(),
            &error,
        )
    })?;
    if !metadata.is_file() || !is_executable_file(path) {
        return Err(Refusal::new(
            Class::Incompatible,
            "engine_artifact_not_executable",
            format!(
                "verified engine artifact {} is not a regular executable file",
                path.display()
            ),
        ));
    }
    Ok(Engine {
        program: path.to_path_buf(),
        source: "release_set",
    })
}

/// The captured result of one engine invocation.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// The engine's process exit status, when it exited normally.
    pub code: Option<i32>,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl Outcome {
    /// The exit code the engine returned, or `-1` when it was terminated by a signal.
    pub fn exit_code(&self) -> i32 {
        self.code.unwrap_or(-1)
    }
}

/// A candidate location that was searched and did not hold the engine.
#[derive(Clone, Debug)]
pub struct Searched {
    /// Human description of the location.
    pub description: String,
    /// The concrete path or program name that was tried.
    pub path: String,
}

/// The outcome of engine discovery.
#[derive(Clone, Debug)]
pub enum Located {
    /// The engine was found.
    Found(Engine),
    /// No engine binary was found; the list names every place that was tried.
    Missing(Vec<Searched>),
}

/// Locate the engine binary.
///
/// Order, most explicit first:
/// 1. `AXIOM_ENGINE_BIN`, an absolute or relative path to the engine executable.
/// 2. The directory of the running `axiom-cli` executable, which is where a release set places
///    the engine beside the distribution entrypoint.
/// 3. `PATH`.
pub fn locate() -> Located {
    let mut searched: Vec<Searched> = Vec::new();

    if let Ok(explicit) = std::env::var(ENGINE_ENV) {
        if !explicit.trim().is_empty() {
            let path = PathBuf::from(&explicit);
            if is_executable_file(&path) {
                return Located::Found(Engine {
                    program: path,
                    source: "env",
                });
            }
            searched.push(Searched {
                description: ENGINE_ENV.to_string(),
                path: explicit,
            });
        }
    }

    if let Some(directory) = executable_directory() {
        let private = directory
            .join(".internal")
            .join(crate::layout::program_file(ENGINE_PROGRAM));
        if is_executable_file(&private) {
            return Located::Found(Engine {
                program: private,
                source: "private_runtime",
            });
        }
        for name in program_names() {
            let candidate = directory.join(name);
            if is_executable_file(&candidate) {
                return Located::Found(Engine {
                    program: candidate,
                    source: "sibling",
                });
            }
            searched.push(Searched {
                description: "beside axm (legacy migration input)".to_string(),
                path: candidate.display().to_string(),
            });
        }
    } else {
        searched.push(Searched {
            description: "the axm executable directory".to_string(),
            path: "(unresolved)".to_string(),
        });
    }

    if let Some(found) = search_path() {
        return Located::Found(Engine {
            program: found,
            source: "path",
        });
    }
    searched.push(Searched {
        description: "PATH".to_string(),
        path: ENGINE_PROGRAM.to_string(),
    });

    Located::Missing(searched)
}

/// The refusal a caller reports when no engine could be found.
pub fn missing_refusal(searched: &[Searched]) -> Refusal {
    let places = searched
        .iter()
        .map(|item| format!("{}: {}", item.description, item.path))
        .collect::<Vec<String>>()
        .join("; ");
    Refusal::new(
        Class::NotFound,
        "engine_not_found",
        format!(
            "Graph Engine installation runtime was not found; no installation or removal could run. Searched {places}. Run the Axiom installation script to repair axm, or name a verified runtime with {ENGINE_ENV}."
        ),
    )
}

/// Program file names to try for the engine on this host.
fn program_names() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["axiom.exe", "axiom"]
    } else {
        vec!["axiom"]
    }
}

/// The directory holding the running executable, when the host reports one.
fn executable_directory() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?.to_path_buf();
    if parent.as_os_str().is_empty() {
        None
    } else {
        Some(parent)
    }
}

/// Resolve the engine on `PATH` without a shell.
fn search_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for name in program_names() {
            let candidate = directory.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// True when `path` is a file this process could execute.
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_without_the_engine_reports_every_place_it_looked() {
        let searched = vec![
            Searched {
                description: "env".to_string(),
                path: "/nowhere/axiom".to_string(),
            },
            Searched {
                description: "PATH".to_string(),
                path: "axiom".to_string(),
            },
        ];
        let refusal = missing_refusal(&searched);
        assert_eq!(refusal.class, Class::NotFound);
        assert_eq!(refusal.reason, "engine_not_found");
        // The message must name the places, so an operator can act without guessing.
        assert!(refusal.message.contains("/nowhere/axiom"));
        assert!(refusal.message.contains("PATH"));
    }

    #[test]
    fn an_outcome_without_an_exit_code_is_not_a_success() {
        let outcome = Outcome {
            code: None,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert_ne!(outcome.exit_code(), 0);
    }
}
