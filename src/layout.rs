//! The single per-user install layout of ADR-0033 decision 3 (task L-002).
//!
//! One install root holds everything a first install produces:
//!
//! ```text
//! <root>/
//!   bin/axm, axsk, axge, axmp[.exe]   public commands
//!   bin/graphd-engine/axiom[.exe]    private installation engine
//!   installs/ecosystem/   placed by the engine (`install plan` / `install apply`)
//!   generations/<id>/     the verified artifact bytes of the active generation
//!   recorded-manifest.json
//!   installed.json        written last and atomically; the commit point of a first install
//! ```
//!
//! The engine still owns placement of the ecosystem. This module only records what the engine
//! placed and puts the three executables side by side, so `version`, `doctor`, `update` and
//! `uninstall` read one record instead of hunting through two roots.

use std::path::{Path, PathBuf};

use crate::update::error::Refusal;
use crate::update::generation::{Entry, Generation, GENERATION_SCHEMA_VERSION};
use crate::update::json::Json;
use crate::update::state::{self, Installed, State, STATE_SCHEMA_VERSION};
use crate::update::time::Stamp;
use crate::update::{rules, sha256};

/// The one `bin` directory under the install root.
pub const BIN_DIR: &str = "bin";

/// Entries a root may already contain without being foreign: everything this layout, the engine,
/// the update channel, the MCP runtime provisioner or an earlier Axiom release writes.
const OWNED_ENTRIES: [&str; 20] = [
    "bin",
    "installs",
    "generations",
    "staging",
    "journal",
    "state",
    "mcp-runtime",
    "installed.json",
    "installed.previous.json",
    "mcp-runtime.previous.json",
    "recorded-manifest.previous.json",
    "installed.json.tmp",
    "recorded-manifest.json",
    "recorded-manifest.json.tmp",
    "update.lock",
    "cli",
    "install-manifest.json",
    "path-change.json",
    "logs",
    "adoption.json",
];

/// File name of a program on this host.
pub fn program_file(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// `<root>/bin`.
pub fn bin_dir(root: &Path) -> PathBuf {
    root.join(BIN_DIR)
}

/// Refuse a root that exists, is not empty and holds something no Axiom layout writes.
///
/// A first install must never adopt or overwrite a directory the user filled with their own
/// files. The refusal names the first foreign entry so the operator can choose another root.
pub fn foreign_root_refusal(root: &Path) -> Option<Refusal> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut foreign: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !OWNED_ENTRIES.contains(&name.as_str()))
        .collect();
    if foreign.is_empty() {
        return None;
    }
    foreign.sort();
    Some(Refusal::conflict(
        "install_root_foreign",
        format!(
            "the install root {} is not empty and holds entries no Axiom install writes ({}); \
             nothing was placed. Choose an empty root or move those entries",
            root.display(),
            foreign.join(", ")
        ),
    ))
}

/// One executable to place in `bin`, with the digest it was verified against.
pub struct BinSource {
    pub name: &'static str,
    pub source: PathBuf,
    pub sha256: String,
}

/// The short name of `axiom-cli` (ADR-0036): a byte-identical, digest-recorded copy in `bin`.
pub const SHORT_COMMAND: &str = "axm";

/// Add `bin/axm` as a verified copy of the `axiom-cli` source, if one is being placed.
///
/// The alias is the same bytes under a second name, not a link or a shim, so it is placed,
/// recorded, rolled back and removed exactly like every other owned executable.
pub fn with_short_command(sources: &mut Vec<BinSource>) {
    if sources.iter().any(|item| item.name == SHORT_COMMAND) {
        return;
    }
    if let Some(cli) = sources.iter().find(|item| item.name == "axiom-cli") {
        let alias = BinSource {
            name: SHORT_COMMAND,
            source: cli.source.clone(),
            sha256: cli.sha256.clone(),
        };
        sources.push(alias);
    }
}

/// Canonical entrypoints are owned, digest-bound copies of verified native programs.
pub fn with_public_commands(sources: &mut Vec<BinSource>) {
    for (name, component) in [
        ("axsk", "axiom-cli"),
        ("axmp", "axiom-cli"),
        ("axge", "axiom-graphd"),
    ] {
        if sources.iter().any(|source| source.name == name) {
            continue;
        }
        if let Some(source) = sources.iter().find(|source| source.name == component) {
            sources.push(BinSource {
                name,
                source: source.source.clone(),
                sha256: source.sha256.clone(),
            });
        }
    }
    for source in sources.iter_mut() {
        if source.name == "axiom" {
            source.name = "graphd-engine/axiom";
        }
    }
    sources.retain(|source| !matches!(source.name, "axiom-cli" | "axiom-graphd"));
}

/// Read unchanged, recorded legacy public files before any installation mutation.
fn legacy_public_sources(root: &Path) -> Result<Vec<BinSource>, Refusal> {
    let state = State::new(root);
    let entries = if state.has_installed() {
        let installed = state.read_installed()?;
        Generation::read(&state, &installed.current_generation)?.entries
    } else {
        Vec::new()
    };
    let mut sources = Vec::new();
    for (old, private) in [
        (".internal/axiom", ".internal/retired-private-engine"),
        ("axiom-cli", ".internal/retired-axiom-cli"),
        ("axiom", ".internal/retired-axiom"),
        ("axiom-graphd", ".internal/retired-axiom-graphd"),
        ("axiom-mcp", ".internal/retired-axiom-mcp"),
        (
            "axiom-mcp-entrypoints",
            ".internal/retired-axiom-mcp-entrypoints",
        ),
    ] {
        let path = bin_dir(root).join(program_file(old));
        if !path.exists() && !path.is_symlink() {
            let destination = bin_dir(root).join(program_file(private));
            if let Some(entry) = entries
                .iter()
                .find(|entry| entry.component == format!("bin/{private}"))
            {
                if destination.is_symlink()
                    || !destination.is_file()
                    || file_digest(&destination)? != entry.artifact_sha256
                {
                    return Err(Refusal::conflict(
                        "legacy_backup_changed",
                        "owned private migration file changed; nothing was installed",
                    ));
                }
                sources.push(BinSource {
                    name: private,
                    source: destination,
                    sha256: entry.artifact_sha256.clone(),
                });
            }
            continue;
        }
        let tracked = entries
            .iter()
            .find(|entry| entry.component == format!("bin/{old}"));
        let expected = tracked.map(|entry| entry.artifact_sha256.as_str());
        if path.is_symlink() || !path.is_file() || Some(file_digest(&path)?.as_str()) != expected {
            return Err(Refusal::conflict("legacy_command_foreign", format!("{} is unowned, linked or modified; move it aside before running axm install; nothing was changed", path.display())));
        }
        let destination = bin_dir(root).join(program_file(private));
        if destination.parent().is_some_and(Path::is_symlink)
            || destination.is_symlink()
            || (destination.exists()
                && (!destination.is_file()
                    || Some(file_digest(&destination)?.as_str()) != expected))
        {
            return Err(Refusal::conflict(
                "legacy_backup_foreign",
                "private migration destination is occupied; nothing was changed",
            ));
        }
        sources.push(BinSource {
            name: private,
            source: path,
            sha256: expected.unwrap().to_string(),
        });
    }
    Ok(sources)
}

/// Retire only verified legacy names into an owned private location (also works for a running Windows executable).
pub fn retire_legacy_commands(root: &Path) -> Result<Vec<BinSource>, Refusal> {
    let mut sources = legacy_public_sources(root)?;
    for source in &mut sources {
        let destination = bin_dir(root).join(program_file(source.name));
        if source.source == destination {
            continue;
        }
        std::fs::create_dir_all(destination.parent().unwrap()).map_err(|error| {
            Refusal::io(
                "legacy_retirement_failed",
                &destination.display().to_string(),
                &error,
            )
        })?;
        if destination.exists() {
            std::fs::remove_file(&destination).map_err(|error| {
                Refusal::io(
                    "legacy_retirement_failed",
                    &destination.display().to_string(),
                    &error,
                )
            })?;
        }
        std::fs::rename(&source.source, &destination).map_err(|error| {
            Refusal::io(
                "legacy_retirement_failed",
                &source.source.display().to_string(),
                &error,
            )
        })?;
        source.source = destination;
    }
    Ok(sources)
}

/// Refuse a new command slot occupied by user data or a modified owned executable.
pub fn public_command_conflict(root: &Path, verified: &[Json]) -> Result<(), Refusal> {
    if bin_dir(root).is_symlink()
        || bin_dir(root).join(".internal").is_symlink()
        || bin_dir(root).join("graphd-engine").is_symlink()
    {
        return Err(Refusal::conflict(
            "bin_path_link",
            "executable directory is linked; nothing was installed",
        ));
    }
    legacy_public_sources(root)?;
    let state = State::new(root);
    let owned = if state.has_installed() {
        let installed = state.read_installed()?;
        Generation::read(&state, &installed.current_generation)?.entries
    } else {
        Vec::new()
    };
    for (name, component) in [
        ("graphd-engine/axiom", "axiom"),
        ("axm", "axiom-cli"),
        ("axsk", "axiom-cli"),
        ("axmp", "axiom-cli"),
        ("axge", "axiom-graphd"),
    ] {
        let path = bin_dir(root).join(program_file(name));
        if !path.exists() && !path.is_symlink() {
            continue;
        }
        let actual = if path.is_file() && !path.is_symlink() {
            Some(file_digest(&path)?)
        } else {
            None
        };
        let expected = verified
            .iter()
            .find(|entry| entry.get("component").and_then(Json::as_text) == Some(component))
            .and_then(|entry| entry.get("sha256"))
            .and_then(Json::as_text);
        let tracked = owned
            .iter()
            .find(|entry| entry.component == format!("bin/{name}"))
            .map(|entry| entry.artifact_sha256.as_str());
        if actual
            .as_deref()
            .is_some_and(|digest| Some(digest) == expected || Some(digest) == tracked)
        {
            continue;
        }
        return Err(Refusal::conflict(
            "public_command_foreign",
            format!(
                "{name} is occupied by an unowned, linked or modified file; nothing was installed"
            ),
        ));
    }
    Ok(())
}

/// Place each executable in `bin` atomically and idempotently.
///
/// A destination already holding the verified bytes is left alone, which is also what lets a
/// re-run from the installed `bin/axiom-cli` succeed while that very file is executing. Any other
/// destination is replaced through a temporary sibling and one rename; on Windows a running
/// executable cannot be replaced in place, so the old file is first renamed aside.
pub fn place_bin(root: &Path, sources: &[BinSource]) -> Result<Vec<Json>, Refusal> {
    let bin = bin_dir(root);
    std::fs::create_dir_all(&bin)
        .map_err(|error| Refusal::io("bin_unwritable", &bin.display().to_string(), &error))?;
    let mut placed = Vec::new();
    for item in sources {
        if !rules::is_relative_path(item.name) {
            return Err(Refusal::validation(
                "bin_path_invalid",
                "invalid executable destination",
            ));
        }
        let destination = bin.join(program_file(item.name));
        let parent = destination.parent().unwrap_or(&bin);
        let mut ancestor = Some(parent);
        while let Some(directory) = ancestor {
            if directory.is_symlink() {
                return Err(Refusal::conflict(
                    "bin_path_link",
                    "executable directory is linked; nothing was placed",
                ));
            }
            if directory == bin {
                break;
            }
            ancestor = directory.parent();
        }
        std::fs::create_dir_all(parent).map_err(|error| {
            Refusal::io("bin_unwritable", &parent.display().to_string(), &error)
        })?;
        let filename = destination.file_name().unwrap().to_string_lossy();
        let action = if destination.is_file()
            && state::digest_file(&destination, "bin_unreadable")? == item.sha256
        {
            "unchanged"
        } else {
            let temporary = parent.join(format!(".{filename}.tmp"));
            std::fs::copy(&item.source, &temporary).map_err(|error| {
                Refusal::io("bin_copy_failed", &temporary.display().to_string(), &error)
            })?;
            let copied = state::digest_file(&temporary, "bin_unreadable")?;
            if copied != item.sha256 {
                let _ = std::fs::remove_file(&temporary);
                return Err(Refusal::validation(
                    format!("bin_digest_mismatch:{}", item.name),
                    format!(
                        "{} changed while it was copied (expected {}, copied {}); nothing \
                         replaced the existing {}",
                        item.source.display(),
                        item.sha256,
                        copied,
                        destination.display()
                    ),
                ));
            }
            if destination.exists() && std::fs::rename(&temporary, &destination).is_err() {
                let aside = parent.join(format!(".{filename}.old"));
                let _ = std::fs::remove_file(&aside);
                std::fs::rename(&destination, &aside).map_err(|error| {
                    Refusal::io(
                        "bin_replace_failed",
                        &destination.display().to_string(),
                        &error,
                    )
                })?;
                std::fs::rename(&temporary, &destination).map_err(|error| {
                    Refusal::io(
                        "bin_replace_failed",
                        &destination.display().to_string(),
                        &error,
                    )
                })?;
                let _ = std::fs::remove_file(&aside);
            } else if !destination.exists() {
                std::fs::rename(&temporary, &destination).map_err(|error| {
                    Refusal::io(
                        "bin_replace_failed",
                        &destination.display().to_string(),
                        &error,
                    )
                })?;
            }
            "placed"
        };
        placed.push(Json::from_pairs(vec![
            ("name", Json::text(item.name)),
            (
                "path",
                Json::text(&format!("{BIN_DIR}/{}", program_file(item.name))),
            ),
            ("sha256", Json::text(&item.sha256)),
            ("action", Json::text(action)),
        ]));
    }
    Ok(placed)
}

/// What a completed first install records.
pub struct Record<'a> {
    pub channel: &'a str,
    pub host: &'a str,
    pub manifest_bytes: &'a [u8],
    pub manifest_sha256: &'a str,
    pub plan_digest: &'a str,
    /// `(component, version, revision, sha256, resolved path, needs_restart)` per verified artifact.
    pub components: Vec<(String, String, String, String, PathBuf, bool)>,
    pub bin: Vec<Json>,
}

/// The generation id a plan digest maps to; stable, so a re-run reuses the same generation.
pub fn generation_id(plan_digest: &str) -> String {
    format!(
        "install-{}-graphd",
        &plan_digest[..plan_digest.len().min(16)]
    )
}

/// Copy the verified artifacts into the generation, record the manifest, then write
/// `installed.json` last. A crash before the final rename leaves no `installed.json`, so the next
/// run sees an unfinished install and redoes it; a generation directory that no longer verifies is
/// rebuilt rather than trusted.
pub fn record(root: &Path, record: &Record) -> Result<Installed, Refusal> {
    let st = State::new(root);
    let id = generation_id(record.plan_digest);
    if !rules::is_plan_id(&id) {
        return Err(Refusal::validation(
            "generation_id_invalid",
            format!("`{id}` is not a valid generation id"),
        ));
    }
    let directory = st.generation_dir(&id);
    let mut entries = Vec::new();
    for (component, version, revision, digest, resolved, needs_restart) in &record.components {
        let name = resolved
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| component.clone());
        entries.push(Entry {
            component: component.clone(),
            action: "install".to_string(),
            version: version.clone(),
            revision: revision.clone(),
            artifact_sha256: digest.clone(),
            payload: Some(format!("{}/{component}/{name}", state::PAYLOAD_DIR)),
            needs_restart: *needs_restart,
            probe: None,
        });
    }
    let generation = Generation {
        schema_version: GENERATION_SCHEMA_VERSION,
        generation_id: id.clone(),
        transaction_id: id.clone(),
        plan_id: id.clone(),
        plan_digest: record.plan_digest.to_string(),
        channel: record.channel.to_string(),
        host: record.host.to_string(),
        install_root: root.display().to_string(),
        created_at: Stamp::now(),
        entries,
    };
    let reusable = directory.join(state::GENERATION_FILE).is_file()
        && Generation::read(&st, &id)
            .and_then(|existing| existing.verify(&directory))
            .is_ok();
    if !reusable {
        if directory.exists() {
            std::fs::remove_dir_all(&directory).map_err(|error| {
                Refusal::io(
                    "generation_reset_failed",
                    &directory.display().to_string(),
                    &error,
                )
            })?;
        }
        for (component, _, _, _, resolved, _) in &record.components {
            let entry = generation
                .entries
                .iter()
                .find(|entry| &entry.component == component)
                .and_then(|entry| entry.payload.clone())
                .unwrap_or_default();
            let target = directory.join(entry.replace('/', std::path::MAIN_SEPARATOR_STR));
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    Refusal::io(
                        "generation_unwritable",
                        &parent.display().to_string(),
                        &error,
                    )
                })?;
            }
            std::fs::copy(resolved, &target).map_err(|error| {
                Refusal::io(
                    "generation_copy_failed",
                    &target.display().to_string(),
                    &error,
                )
            })?;
        }
        generation.verify(&directory)?;
        generation.write(&directory)?;
    }
    st.record_manifest_bytes(record.manifest_bytes)?;
    let previous = st
        .read_installed()
        .ok()
        .map(|installed| installed.current_generation)
        .filter(|current| current != &id);
    let mut installed = Installed {
        schema_version: STATE_SCHEMA_VERSION,
        channel: record.channel.to_string(),
        channel_manifest_sha256: record.manifest_sha256.to_string(),
        current_generation: id,
        previous_generation: previous,
        installed_at: Stamp::now(),
        extra: Default::default(),
    };
    installed.extra.insert(
        "components".to_string(),
        Json::array(
            record
                .components
                .iter()
                .map(|(component, version, _, digest, _, _)| {
                    Json::from_pairs(vec![
                        ("component", Json::text(component)),
                        ("version", Json::text(version)),
                        ("sha256", Json::text(digest)),
                    ])
                })
                .collect(),
        ),
    );
    installed
        .extra
        .insert("bin".to_string(), Json::array(record.bin.clone()));
    installed
        .extra
        .insert("layout".to_string(), Json::text("adr-0033-single-root"));
    let source = st
        .read_installed()
        .ok()
        .and_then(|old| old.extra.get("channel_source").cloned())
        .unwrap_or_else(|| Json::text(crate::update::oneshot::CANONICAL_CHANNEL_SOURCE));
    installed.extra.insert("channel_source".to_string(), source);
    st.write_installed(&installed)?;
    Ok(installed)
}

/// sha256 of a file as lowercase hex.
pub fn file_digest(path: &Path) -> Result<String, Refusal> {
    sha256::file_hex(path)
        .map_err(|error| Refusal::io("digest_unreadable", &path.display().to_string(), &error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_aliases_are_idempotent_and_foreign_slots_are_preserved() {
        let root = temp("public-command-slots");
        std::fs::create_dir_all(bin_dir(&root)).unwrap();
        let cli = root.join("cli-source");
        let daemon = root.join("daemon-source");
        std::fs::write(&cli, b"verified cli").unwrap();
        std::fs::write(&daemon, b"verified daemon").unwrap();
        let mut sources = vec![
            BinSource {
                name: "axiom-cli",
                source: cli.clone(),
                sha256: file_digest(&cli).unwrap(),
            },
            BinSource {
                name: "axiom-graphd",
                source: daemon,
                sha256: sha256::digest_hex(b"verified daemon"),
            },
        ];
        with_short_command(&mut sources);
        with_public_commands(&mut sources);
        with_public_commands(&mut sources);
        assert_eq!(sources.len(), 4, "no legacy public entrypoints");
        place_bin(&root, &sources).unwrap();
        for old in ["axiom-cli", "axiom", "axiom-graphd"] {
            assert!(!bin_dir(&root).join(program_file(old)).exists());
        }
        assert_eq!(
            sources
                .iter()
                .filter(|source| matches!(source.name, "axm" | "axsk" | "axge" | "axmp"))
                .count(),
            4
        );
        let user_file = bin_dir(&root).join(program_file("axsk"));
        std::fs::write(&user_file, b"human data").unwrap();
        let verified = vec![
            Json::from_pairs(vec![
                ("component", Json::text("axiom-cli")),
                ("sha256", Json::text(&file_digest(&cli).unwrap())),
            ]),
            Json::from_pairs(vec![
                ("component", Json::text("axiom-graphd")),
                (
                    "sha256",
                    Json::text(&sha256::digest_hex(b"verified daemon")),
                ),
            ]),
        ];
        assert_eq!(
            public_command_conflict(&root, &verified)
                .unwrap_err()
                .reason,
            "public_command_foreign"
        );
        assert_eq!(std::fs::read(&user_file).unwrap(), b"human data");
        std::fs::copy(cli, user_file).unwrap();
        assert!(
            public_command_conflict(&root, &verified).is_ok(),
            "an interrupted exact-byte placement can be repaired"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("axiom-layout-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn legacy_migration_preserves_recorded_bytes_and_refuses_modified_files() {
        for (old_name, label) in [
            ("axiom", "legacy-retirement"),
            (".internal/axiom", "old-private-retirement"),
        ] {
            let root = temp(label);
            let source = root.join("source");
            std::fs::write(&source, b"old verified engine").unwrap();
            let digest = file_digest(&source).unwrap();
            let sources = [BinSource {
                name: old_name,
                source,
                sha256: digest.clone(),
            }];
            let bin = place_bin(&root, &sources).unwrap();
            let legacy = bin_dir(&root).join(program_file(old_name));
            assert_eq!(
                retire_legacy_commands(&root).err().unwrap().reason,
                "legacy_command_foreign"
            );
            record(
                &root,
                &Record {
                    channel: "stable",
                    host: "fixture",
                    manifest_bytes: b"{}",
                    manifest_sha256: &sha256::digest_hex(b"{}"),
                    plan_digest: &"a".repeat(64),
                    components: vec![(
                        format!("bin/{old_name}"),
                        "0.1.6".into(),
                        String::new(),
                        digest.clone(),
                        legacy.clone(),
                        false,
                    )],
                    bin,
                },
            )
            .unwrap();
            std::fs::write(&legacy, b"human edit").unwrap();
            assert_eq!(
                retire_legacy_commands(&root).err().unwrap().reason,
                "legacy_command_foreign"
            );
            assert_eq!(std::fs::read(&legacy).unwrap(), b"human edit");
            std::fs::write(&legacy, b"old verified engine").unwrap();
            let retired = retire_legacy_commands(&root).unwrap();
            assert!(!legacy.exists());
            assert_eq!(retired.len(), 1);
            assert_eq!(file_digest(&retired[0].source).unwrap(), digest);
            let installed = State::new(&root).read_installed().unwrap();
            let previous =
                Generation::read(&State::new(&root), &installed.current_generation).unwrap();
            previous
                .verify(&State::new(&root).generation_dir(&installed.current_generation))
                .unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn private_engine_placement_refuses_traversal_and_linked_directories() {
        let root = temp("private-engine-path");
        let source = root.join("source");
        std::fs::write(&source, b"verified").unwrap();
        let digest = file_digest(&source).unwrap();
        let sources = [BinSource {
            name: "../escape",
            source: source.clone(),
            sha256: digest.clone(),
        }];
        assert_eq!(
            place_bin(&root, &sources).unwrap_err().reason,
            "bin_path_invalid"
        );
        #[cfg(unix)]
        {
            let outside = root.join("outside");
            std::fs::create_dir(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, bin_dir(&root).join("graphd-engine")).unwrap();
            let sources = [BinSource {
                name: "graphd-engine/axiom",
                source,
                sha256: digest,
            }];
            assert_eq!(
                place_bin(&root, &sources).unwrap_err().reason,
                "bin_path_link"
            );
            assert!(!outside.join("axiom").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_empty_or_axiom_root_is_not_foreign_but_user_files_are() {
        let root = temp("foreign");
        assert!(foreign_root_refusal(&root).is_none());
        std::fs::create_dir_all(root.join("installs")).unwrap();
        std::fs::write(root.join("installed.json"), b"{}").unwrap();
        assert!(foreign_root_refusal(&root).is_none());
        std::fs::write(root.join("thesis.docx"), b"mine").unwrap();
        let refusal = foreign_root_refusal(&root).expect("foreign root refused");
        assert_eq!(refusal.reason, "install_root_foreign");
        assert!(refusal.message.contains("thesis.docx"));
        assert!(foreign_root_refusal(&root.join("absent")).is_none());
    }

    #[test]
    fn short_command_is_a_verified_copy_of_axiom_cli() {
        let root = temp("axm");
        let source = root.join("src-axiom-cli");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&source, b"cli bytes").unwrap();
        let digest = file_digest(&source).unwrap();
        let mut sources = vec![BinSource {
            name: "axiom-cli",
            source: source.clone(),
            sha256: digest.clone(),
        }];
        with_short_command(&mut sources);
        with_short_command(&mut sources);
        assert_eq!(sources.len(), 2, "the alias is added once");
        let placed = place_bin(&root, &sources).unwrap();
        let names: Vec<_> = placed
            .iter()
            .map(|item| {
                item.get("name")
                    .and_then(Json::as_text)
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(names, ["axiom-cli", SHORT_COMMAND]);
        let alias = bin_dir(&root).join(program_file(SHORT_COMMAND));
        assert_eq!(file_digest(&alias).unwrap(), digest);
        assert_eq!(
            placed[1].get("path").and_then(Json::as_text),
            Some(format!("bin/{}", program_file(SHORT_COMMAND)).as_str())
        );
        let mut without_cli = vec![BinSource {
            name: "axiom",
            source,
            sha256: digest,
        }];
        with_short_command(&mut without_cli);
        assert_eq!(without_cli.len(), 1, "no alias without an axiom-cli source");
    }

    #[test]
    fn placing_bin_is_idempotent_and_refuses_changed_bytes() {
        let root = temp("bin");
        let source = root.join("src-axiom");
        std::fs::write(&source, b"engine bytes").unwrap();
        let digest = file_digest(&source).unwrap();
        let first = place_bin(
            &root,
            &[BinSource {
                name: "axiom",
                source: source.clone(),
                sha256: digest.clone(),
            }],
        )
        .unwrap();
        assert_eq!(
            first[0].get("action").and_then(Json::as_text),
            Some("placed")
        );
        let again = place_bin(
            &root,
            &[BinSource {
                name: "axiom",
                source: source.clone(),
                sha256: digest.clone(),
            }],
        )
        .unwrap();
        assert_eq!(
            again[0].get("action").and_then(Json::as_text),
            Some("unchanged")
        );
        let wrong = place_bin(
            &root,
            &[BinSource {
                name: "axiom-graphd",
                source,
                sha256: "0".repeat(64),
            }],
        )
        .unwrap_err();
        assert!(wrong.reason.starts_with("bin_digest_mismatch"));
        assert!(!bin_dir(&root).join(program_file("axiom-graphd")).exists());
    }

    #[test]
    fn record_writes_installed_json_last_and_reuses_the_generation() {
        let root = temp("record");
        let artifact = root.join("axiom-graphd.bin");
        std::fs::write(&artifact, b"graphd").unwrap();
        let digest = file_digest(&artifact).unwrap();
        let manifest = br#"{"channel":"stable"}"#;
        let manifest_sha = sha256::hex(&sha256::digest(manifest));
        let plan = "a".repeat(64);
        let make = || Record {
            channel: "stable",
            host: "windows-x64",
            manifest_bytes: manifest,
            manifest_sha256: &manifest_sha,
            plan_digest: &plan,
            components: vec![(
                "axiom-graphd".to_string(),
                "0.1.2".to_string(),
                "rev".to_string(),
                digest.clone(),
                artifact.clone(),
                true,
            )],
            bin: vec![],
        };
        let installed = record(&root, &make()).unwrap();
        assert_eq!(installed.current_generation, generation_id(&plan));
        let text = std::fs::read_to_string(root.join("installed.json")).unwrap();
        let parsed = Installed::parse(&text).unwrap();
        assert_eq!(parsed.current_generation, installed.current_generation);
        assert!(text.contains("\"components\"") && text.contains(&digest));
        // A re-run keeps the same generation and does not record itself as its own predecessor.
        let again = record(&root, &make()).unwrap();
        assert_eq!(again.previous_generation, None);
        // A tampered payload is rebuilt from the verified source on the next run.
        let payload = root
            .join("generations")
            .join(generation_id(&plan))
            .join("payload/axiom-graphd/axiom-graphd.bin");
        std::fs::write(&payload, b"tampered").unwrap();
        record(&root, &make()).unwrap();
        assert_eq!(std::fs::read(&payload).unwrap(), b"graphd");
    }
}
