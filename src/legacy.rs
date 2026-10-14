//! Detection and adoption of pre-ADR-0033 installations (task L-006).
//!
//! Two layouts were in real use on 2026-10-06, plus leftovers around them:
//!
//! * a **CLI store** written by `Install-AxiomCli.ps1` / the J-004 installers: `bin/axiom-cli`,
//!   `cli/` (generations + `state.json`) and `install-manifest.json` (`release_version` 0.1.0 or
//!   0.1.2), normally at `%LOCALAPPDATA%\Axiom`;
//! * a **bootstrap root** written by `bootstrap_windows.ps1` / `bootstrap_linux.py`: an engine
//!   ecosystem under `installs/ecosystem/current`, an MCP runtime (0.1.2 Windows nests it as
//!   `mcp-runtime/mcp-runtime`) and staging, with no `installed.json`, e.g. `%USERPROFILE%\axiom`;
//! * an older `axiom-cli` earlier on PATH, and `AXIOM_CLI_INSTALL_ROOT` / `AXIOM_ENGINE_BIN` /
//!   `AXIOM_HOME` left in the user environment.
//!
//! Detection never modifies anything. A directory that looks like Axiom but matches neither
//! layout is reported as `unknown` and is never modified.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::update::json::{self, Json};
use crate::update::state;

/// One detected legacy layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// `cli-store`, `bootstrap-root` or `unknown`.
    pub kind: &'static str,
    pub path: PathBuf,
    pub version: Option<String>,
    /// Whether the MCP runtime sits in the nested 0.1.2 `mcp-runtime/mcp-runtime` location.
    pub nested_mcp_runtime: bool,
}

impl Layout {
    pub fn to_json(&self) -> Json {
        Json::from_pairs(vec![
            ("kind", Json::text(self.kind)),
            ("path", Json::text(&self.path.display().to_string())),
            (
                "version",
                self.version.as_deref().map_or(Json::Null, Json::text),
            ),
            ("nested_mcp_runtime", Json::bool(self.nested_mcp_runtime)),
        ])
    }

    pub fn describe(&self) -> String {
        format!(
            "{} {} at {}",
            self.kind,
            self.version.as_deref().unwrap_or("(version unknown)"),
            self.path.display()
        )
    }
}

/// Classify one directory; `None` when it is absent, empty, or already an ADR-0033 root.
pub fn classify(path: &Path) -> Option<Layout> {
    let entries: Vec<String> = std::fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    if entries.is_empty() || path.join(state::INSTALLED_FILE).is_file() {
        return None;
    }
    let manifest = path.join("install-manifest.json");
    if let Some(record) = read_json(&manifest) {
        if record.get("document_kind").and_then(Json::as_text) == Some("axiom-cli-install-manifest")
        {
            return Some(Layout {
                kind: "cli-store",
                path: path.to_path_buf(),
                version: record
                    .get("release_version")
                    .and_then(Json::as_text)
                    .map(str::to_string),
                nested_mcp_runtime: false,
            });
        }
    }
    let current = path.join("installs").join("ecosystem").join("current");
    if current.is_file() {
        let version = read_json(&current).and_then(|pointer| {
            pointer
                .get("activated")?
                .as_array()?
                .iter()
                .find(|row| row.get("component").and_then(Json::as_text) == Some("axiom-graphd"))?
                .get("version")?
                .as_text()
                .map(str::to_string)
        });
        return Some(Layout {
            kind: "bootstrap-root",
            path: path.to_path_buf(),
            version,
            nested_mcp_runtime: nested_runtime_pointer(path).is_some()
                && !path.join("mcp-runtime").join("current.json").is_file(),
        });
    }
    let axiom_like = ["installs", "mcp-runtime", "install-manifest.json", "cli"]
        .iter()
        .any(|name| entries.iter().any(|entry| entry == name))
        || path
            .join("bin")
            .join(crate::layout::program_file("axiom-cli"))
            .is_file();
    axiom_like.then(|| Layout {
        kind: "unknown",
        path: path.to_path_buf(),
        version: None,
        nested_mcp_runtime: false,
    })
}

/// The 0.1.2 Windows bootstrap's nested MCP runtime pointer, if present.
pub fn nested_runtime_pointer(root: &Path) -> Option<PathBuf> {
    let pointer = root
        .join("mcp-runtime")
        .join("mcp-runtime")
        .join("current.json");
    pointer.is_file().then_some(pointer)
}

fn read_json(path: &Path) -> Option<Json> {
    json::parse(&std::fs::read_to_string(path).ok()?).ok()
}

/// Directories worth inspecting: the target root, the documented 0.1.2 bootstrap root and any
/// root a leftover environment variable still names.
pub fn candidates(target: &Path) -> Vec<PathBuf> {
    let mut out = vec![target.to_path_buf()];
    // The platform default root, even when an environment override points elsewhere.
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            out.push(PathBuf::from(local).join("Axiom"));
        }
    } else if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(data).join("axiom"));
    } else if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        out.push(
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("axiom"),
        );
    }
    let home = std::env::var_os("USERPROFILE")
        .filter(|_| cfg!(windows))
        .or_else(|| std::env::var_os("HOME"));
    if let Some(home) = home {
        out.push(PathBuf::from(home).join("axiom"));
    }
    for name in ["AXIOM_CLI_INSTALL_ROOT", "AXIOM_HOME"] {
        if let Some(value) = std::env::var_os(name).filter(|v| !v.is_empty()) {
            out.push(PathBuf::from(value));
        }
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    out.retain(|path| {
        let key = normal(path);
        if seen.iter().any(|other| normal(other) == key) {
            false
        } else {
            seen.push(path.clone());
            true
        }
    });
    out
}

fn normal(path: &Path) -> String {
    let text = path.display().to_string().replace('/', "\\");
    let text = text.trim_end_matches('\\').to_string();
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

/// True when two paths name the same directory (case-insensitively on Windows).
pub fn same_path(a: &Path, b: &Path) -> bool {
    normal(a) == normal(b)
}

/// Every legacy layout among the candidates.
pub fn detect(target: &Path) -> Vec<Layout> {
    candidates(target)
        .iter()
        .filter_map(|path| classify(path))
        .collect()
}

/// An `axm` on PATH that is not this root's `bin`, and whether it comes first.
#[derive(Clone, Debug)]
pub struct Shadow {
    pub path: PathBuf,
    pub version: String,
    pub before_root_bin: bool,
}

/// Every other `axm` on PATH, in PATH order.
pub fn shadows(root_bin: &Path) -> Vec<Shadow> {
    let Some(path) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let own = dirs.iter().position(|dir| same_path(dir, root_bin));
    let mut found = Vec::new();
    for (index, dir) in dirs.iter().enumerate() {
        if same_path(dir, root_bin) {
            continue;
        }
        let exe = dir.join(crate::layout::program_file("axm"));
        if exe.is_file() && !found.iter().any(|s: &Shadow| same_path(&s.path, &exe)) {
            found.push(Shadow {
                version: probe_version(&exe),
                path: exe,
                before_root_bin: own.is_none_or(|own| index < own),
            });
        }
    }
    found
}

/// The version an `axiom-cli` prints in its `--help` header (every release since 0.1.0 starts
/// `axiom-cli X.Y.Z - ...`), bounded to two seconds; `unknown` when it does not answer.
fn probe_version(exe: &Path) -> String {
    let Ok(mut child) = Command::new(exe)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return "unknown".to_string();
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < Duration::from_secs(2) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return "unknown".to_string();
            }
        }
    }
    let mut text = String::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        let _ = out.read_to_string(&mut text);
    }
    text.lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()) && word.contains('.'))
        .unwrap_or("unknown")
        .to_string()
}

/// Leftover environment variables from the two-script 0.1.2 procedure.
pub fn leftover_env() -> Vec<(&'static str, String)> {
    ["AXIOM_CLI_INSTALL_ROOT", "AXIOM_ENGINE_BIN", "AXIOM_HOME"]
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|v| (name, v))
        })
        .collect()
}

/// Back up the CLI store's executable before the adopting install replaces it.
///
/// Idempotent: an existing backup with the same bytes is kept, so a repeated (or interrupted and
/// resumed) adoption never loses the original bytes.
pub fn backup_cli_store(layout: &Layout) -> Result<Option<PathBuf>, crate::update::error::Refusal> {
    let exe = layout
        .path
        .join("bin")
        .join(crate::layout::program_file("axiom-cli"));
    if !exe.is_file() {
        return Ok(None);
    }
    let directory = layout.path.join("legacy").join(format!(
        "cli-store-{}",
        layout.version.as_deref().unwrap_or("unknown")
    ));
    let target = directory.join(crate::layout::program_file("axiom-cli"));
    if target.is_file() {
        return Ok(Some(target));
    }
    std::fs::create_dir_all(&directory).map_err(|error| {
        crate::update::error::Refusal::io("legacy_backup", &directory.display().to_string(), &error)
    })?;
    let temporary = directory.join(".axiom-cli.backup.tmp");
    std::fs::copy(&exe, &temporary).map_err(|error| {
        crate::update::error::Refusal::io("legacy_backup", &exe.display().to_string(), &error)
    })?;
    std::fs::rename(&temporary, &target).map_err(|error| {
        crate::update::error::Refusal::io("legacy_backup", &target.display().to_string(), &error)
    })?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axiom-legacy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_two_known_layouts_and_an_unknown_one_are_classified() {
        let store = temp("store");
        std::fs::create_dir_all(store.join("bin")).unwrap();
        std::fs::create_dir_all(store.join("cli")).unwrap();
        std::fs::write(
            store.join("install-manifest.json"),
            br#"{"document_kind":"axiom-cli-install-manifest","release_version":"0.1.0"}"#,
        )
        .unwrap();
        let layout = classify(&store).unwrap();
        assert_eq!(
            (layout.kind, layout.version.as_deref()),
            ("cli-store", Some("0.1.0"))
        );

        let boot = temp("boot");
        std::fs::create_dir_all(boot.join("installs/ecosystem")).unwrap();
        std::fs::write(
            boot.join("installs/ecosystem/current"),
            br#"{"activated":[{"component":"axiom-graphd","version":"0.1.2"}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(boot.join("mcp-runtime/mcp-runtime")).unwrap();
        std::fs::write(boot.join("mcp-runtime/mcp-runtime/current.json"), b"{}").unwrap();
        let layout = classify(&boot).unwrap();
        assert_eq!(layout.kind, "bootstrap-root");
        assert_eq!(layout.version.as_deref(), Some("0.1.2"));
        assert!(layout.nested_mcp_runtime);

        let odd = temp("odd");
        std::fs::create_dir_all(odd.join("installs")).unwrap();
        std::fs::write(odd.join("thing.txt"), b"x").unwrap();
        assert_eq!(classify(&odd).unwrap().kind, "unknown");

        let plain = temp("plain");
        std::fs::write(plain.join("notes.txt"), b"x").unwrap();
        assert!(classify(&plain).is_none());
        std::fs::write(boot.join("installed.json"), b"{}").unwrap();
        assert!(classify(&boot).is_none(), "an ADR-0033 root is not legacy");
    }

    #[test]
    fn backing_up_the_store_executable_is_idempotent() {
        let store = temp("backup");
        std::fs::create_dir_all(store.join("bin")).unwrap();
        let exe = store
            .join("bin")
            .join(crate::layout::program_file("axiom-cli"));
        std::fs::write(&exe, b"old cli").unwrap();
        let layout = Layout {
            kind: "cli-store",
            path: store.clone(),
            version: Some("0.1.0".into()),
            nested_mcp_runtime: false,
        };
        let first = backup_cli_store(&layout).unwrap().unwrap();
        std::fs::write(&exe, b"new cli").unwrap();
        let second = backup_cli_store(&layout).unwrap().unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), b"old cli");
    }
}
