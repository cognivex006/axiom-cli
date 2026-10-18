//! Host onboarding from the installed verified bundle; never fetch mutable Git content.
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::confirm::{self, Decision};
use crate::update::error::{Class, Refusal};
use crate::update::json::{self, Json};
use crate::update::report::Report;
use crate::update::{sha256, state::State};

pub const HELP: &str = "axm skills - install Axiom skills into your agent\n\nUSAGE: axm skills <install|update|doctor|uninstall|recover> <codex|claude> [--yes] [--dry-run] [--json]\n       axm skills list [--json]\n       axm skills version|check [options]\n\nManaged adapters: Codex and Claude Code (user scope). Other hosts are unsupported.\nThe verified installed bundle supplies the plugin; no Git main download is used.\nHost registration, next-session skill discovery and MCP query readiness are separate.\nCodex: $axsk <intent>. Claude Code: /axiom:axsk <intent>. Terminal axsk is deprecated compatibility only.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Adapter {
    Codex,
    Claude,
}

impl Adapter {
    const ALL: [Self; 2] = [Self::Codex, Self::Claude];

    fn parse(name: &str) -> Result<Self, Refusal> {
        match name {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            _ => Err(invalid(
                "supported managed hosts: codex, claude; use axm skills --help",
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }

    fn receipt(self) -> String {
        format!("host-integrations/{}.json", self.name())
    }

    fn arguments(self, args: &[&str]) -> Vec<String> {
        if self == Self::Codex {
            return args.iter().map(|a| (*a).into()).collect();
        }
        let query = args.starts_with(&["plugin", "list"])
            || args.starts_with(&["plugin", "marketplace", "list"]);
        let mut words: Vec<String> = args
            .iter()
            .filter(|a| query || **a != "--json")
            .map(|a| (*a).into())
            .collect();
        if args.starts_with(&["plugin", "list"]) {
            words = vec!["plugin".into(), "list".into(), "--json".into()];
        }
        if args.starts_with(&["plugin", "add"]) {
            words[1] = "install".into();
        }
        if args.starts_with(&["plugin", "remove"]) {
            words[1] = "uninstall".into();
            words.push("--keep-data".into());
        }
        if !query {
            words.extend(["--scope".into(), "user".into()]);
        }
        words
    }

    fn normalize(self, args: &[&str], result: Json) -> Result<Json, Refusal> {
        if self == Self::Codex {
            return Ok(result);
        }
        if args.starts_with(&["plugin", "marketplace", "list"]) {
            let rows = result
                .as_array()
                .ok_or_else(|| invalid("unsupported Claude marketplace response"))?;
            let mut markets = Vec::new();
            for row in rows {
                if row.get("name").and_then(Json::as_text) != Some("axiom-skills") {
                    continue;
                }
                if row.get("source").and_then(Json::as_text) != Some("directory") {
                    return Err(Refusal::new(Class::Conflict, "host_marketplace_foreign", "Claude axiom-skills marketplace is not a managed local directory; it was preserved"));
                }
                markets.push(Json::from_pairs(vec![
                    ("name", Json::text("axiom-skills")),
                    ("root", Json::text(text(row, "path")?)),
                ]));
            }
            return Ok(Json::from_pairs(vec![(
                "marketplaces",
                Json::array(markets),
            )]));
        }
        if args.starts_with(&["plugin", "list"]) {
            let rows = result
                .as_array()
                .ok_or_else(|| invalid("unsupported Claude plugin response"))?;
            let mut plugins = Vec::new();
            for row in rows {
                let id = text(row, "id")?;
                if !id.ends_with("@axiom-skills") {
                    continue;
                }
                if id != "axiom@axiom-skills" || text(row, "scope")? != "user" {
                    return Err(Refusal::new(Class::Conflict, "host_scope_conflict", "Claude has another plugin or scope using axiom-skills; its configuration was preserved"));
                }
                plugins.push(Json::from_pairs(vec![
                    ("name", Json::text("axiom")),
                    ("marketplaceName", Json::text("axiom-skills")),
                    ("installed", Json::bool(true)),
                    (
                        "enabled",
                        Json::bool(
                            row.get("enabled")
                                .and_then(Json::as_bool)
                                .ok_or_else(|| invalid("Claude enabled state unavailable"))?,
                        ),
                    ),
                    ("version", Json::text(text(row, "version")?)),
                    (
                        "installPath",
                        Json::text(
                            row.get("readFromFolder")
                                .and_then(Json::as_text)
                                .filter(|s| !s.is_empty())
                                .or_else(|| {
                                    row.get("installPath")
                                        .and_then(Json::as_text)
                                        .filter(|s| !s.is_empty())
                                })
                                .ok_or_else(|| invalid("Claude plugin location missing"))?,
                        ),
                    ),
                ]));
            }
            if plugins.len() > 1 {
                return Err(invalid("ambiguous Claude user plugin installation"));
            }
            return Ok(Json::from_pairs(vec![("installed", Json::array(plugins))]));
        }
        Ok(result)
    }
}

fn verify_host_bundle(adapter: Adapter, bundle: &Bundle) -> Result<(), Refusal> {
    if adapter == Adapter::Codex {
        return Ok(());
    }
    let market = read(&safe_path(
        &bundle.source,
        ".claude-plugin/marketplace.json",
    )?)?;
    if text(&market, "name")? != "axiom-skills" {
        return Err(invalid("unexpected Claude marketplace identity"));
    }
    let plugins = market
        .get("plugins")
        .and_then(Json::as_array)
        .ok_or_else(|| invalid("Claude marketplace plugins missing"))?;
    if plugins.len() != 1
        || text(&plugins[0], "name")? != "axiom"
        || text(&plugins[0], "source")? != "./plugins/axiom"
    {
        return Err(invalid(
            "Claude marketplace must reference only the verified local Axiom plugin",
        ));
    }
    let plugin = read(&safe_path(
        &bundle.source,
        "plugins/axiom/.claude-plugin/plugin.json",
    )?)?;
    if text(&plugin, "name")? != "axiom" || text(&plugin, "version")? != bundle.version {
        return Err(invalid("Claude plugin identity/version mismatch"));
    }
    Ok(())
}

fn verify_host_cache(adapter: Adapter, receipt: &Json) -> Result<(), Refusal> {
    if adapter != Adapter::Claude {
        return Ok(());
    }
    let result = host(adapter, &["plugin", "list", "--json"])?;
    let rows = result
        .get("installed")
        .and_then(Json::as_array)
        .ok_or_else(|| invalid("missing installed Claude plugins"))?;
    let Some(plugin) = rows.first() else {
        return Ok(());
    };
    let cache = PathBuf::from(text(plugin, "installPath")?);
    let manifest = receipt
        .get("manifest")
        .ok_or_else(|| invalid("owned cache manifest missing"))?;
    let rows = manifest
        .get("entries")
        .and_then(Json::as_array)
        .ok_or_else(|| invalid("owned plugin entries missing"))?;
    let mut entries = Vec::new();
    for row in rows {
        if let Some(relative) = text(row, "path")?.strip_prefix("plugins/axiom/") {
            let mut entry = row.clone();
            entry.set("path", Json::text(relative)).map_err(invalid)?;
            entries.push(entry);
        }
    }
    verify_entries(
        &cache,
        &Json::from_pairs(vec![("entries", Json::array(entries))]),
    )
    .map_err(|_| {
        Refusal::new(
            Class::Conflict,
            "host_cache_modified",
            "Claude plugin cache differs from the managed verified payload; it was preserved",
        )
    })
}

pub fn managed_hosts() -> Vec<String> {
    let Some(root) = crate::update::state::default_root() else {
        return Vec::new();
    };
    Adapter::ALL
        .into_iter()
        .filter(|a| root.join(a.receipt()).is_file())
        .map(|a| a.name().into())
        .collect()
}

fn invalid(message: impl Into<String>) -> Refusal {
    Refusal::validation("host_skills_invalid", message)
}

fn io(error: std::io::Error) -> Refusal {
    Refusal::io("host_skills_io", "managed host integration", &error)
}

fn linked(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn read(path: &Path) -> Result<Json, Refusal> {
    if linked(path) {
        return Err(invalid("linked ownership records are refused"));
    }
    if std::fs::metadata(path).map_err(io)?.len() > 4 * 1024 * 1024 {
        return Err(invalid("ownership record exceeds budget"));
    }
    let bytes = std::fs::read(path).map_err(io)?;
    json::parse(std::str::from_utf8(&bytes).map_err(|_| invalid("record is not UTF-8"))?)
        .map_err(invalid)
}

fn text<'a>(value: &'a Json, key: &str) -> Result<&'a str, Refusal> {
    value
        .get(key)
        .and_then(Json::as_text)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(format!("missing field {key}")))
}

fn safe_path(root: &Path, relative: &str) -> Result<PathBuf, Refusal> {
    if relative.is_empty()
        || relative.contains(['\\', ':'])
        || relative.split('/').any(|s| matches!(s, "" | "." | ".."))
    {
        return Err(invalid("unsafe skills payload path"));
    }
    let mut path = root.to_path_buf();
    if linked(&path) {
        return Err(invalid("linked skills root"));
    }
    for part in relative.split('/') {
        path.push(part);
        if linked(&path) {
            return Err(invalid("linked skills payload ancestor"));
        }
    }
    Ok(path)
}

struct Bundle {
    source: PathBuf,
    manifest: Json,
    digest: String,
    version: String,
}

fn verify_entries(root: &Path, manifest: &Json) -> Result<(), Refusal> {
    let entries = manifest
        .get("entries")
        .and_then(Json::as_array)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid("skills entries are absent"))?;
    let mut seen = std::collections::BTreeSet::new();
    for row in entries {
        let name = text(row, "path")?;
        if !seen.insert(name) {
            return Err(invalid("duplicate skills payload path"));
        }
        let path = safe_path(root, name)?;
        let size = row
            .get("size_bytes")
            .and_then(Json::as_int)
            .filter(|s| *s >= 0)
            .ok_or_else(|| invalid("invalid skills byte count"))?;
        crate::update::fetch::verify_file(&path, text(row, "sha256")?, size)?;
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).map_err(io)? {
            let path = entry.map_err(io)?.path();
            if linked(&path) {
                return Err(invalid("linked snapshot content"));
            }
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid("snapshot escape"))?
                .to_string_lossy()
                .replace('\\', "/");
            if relative != "bundle.json" && !seen.contains(relative.as_str()) {
                return Err(Refusal::new(
                    Class::Conflict,
                    "snapshot_modified",
                    "undeclared content in managed snapshot was preserved",
                ));
            }
        }
    }
    Ok(())
}

fn installed_bundle(root: &Path) -> Result<Bundle, Refusal> {
    let state = State::new(root.to_path_buf());
    let installed = state.read_installed()?;
    let channel = state.load_recorded_manifest(&installed)?;
    let selected = channel
        .manifest
        .components
        .iter()
        .find(|c| c.component == "skills")
        .ok_or_else(|| invalid("installed release has no skills component"))?;
    let pointer_path = safe_path(root, "installs/ecosystem/skills/current")?;
    let pointer = read(&pointer_path)?;
    let version = text(&pointer, "version")?.to_string();
    if !version
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        || matches!(version.as_str(), "." | "..")
    {
        return Err(invalid("invalid skills version"));
    }
    let source = safe_path(root, &format!("installs/ecosystem/skills/{version}"))?;
    let manifest_path = safe_path(&source, "bundle.json")?;
    let digest = sha256::file_hex(&manifest_path).map_err(io)?;
    let manifest = read(&manifest_path)?;
    if digest != text(&pointer, "manifest_sha256")?
        || selected.version.as_deref() != Some(version.as_str())
        || selected.revision.as_deref() != Some(text(&pointer, "revision")?)
        || text(&manifest, "version")? != version
        || text(&manifest, "revision")? != text(&pointer, "revision")?
    {
        return Err(invalid(
            "installed skills differ from the recorded release; run axm install",
        ));
    }
    verify_entries(&source, &manifest)?;
    // The full marketplace root is required, including its host package and skill references.
    let market = read(&safe_path(&source, ".agents/plugins/marketplace.json")?)?;
    if text(&market, "name")? != "axiom-skills" {
        return Err(invalid("unexpected skills marketplace identity"));
    }
    Ok(Bundle {
        source,
        manifest,
        digest,
        version,
    })
}

fn write(path: &Path, value: &Json) -> Result<(), Refusal> {
    if linked(path) {
        return Err(invalid("linked integration record"));
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let bytes = json::canonical_text(value);
    let mut staged = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(io)?;
    staged.write_all(bytes.as_bytes()).map_err(io)?;
    staged.sync_all().map_err(io)?;
    drop(staged);
    if path.exists() {
        // Keep the previous complete record until the new record has been written.
        let previous = std::fs::read(path).map_err(io)?;
        let backup =
            path.with_extension(format!("previous-{}.json", sha256::digest_hex(&previous)));
        if linked(&backup) {
            return Err(invalid("linked integration backup"));
        }
        if backup.exists() {
            if std::fs::read(&backup).map_err(io)? != previous {
                return Err(invalid("integration backup was modified; preserved"));
            }
        } else {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)
                .map_err(io)?;
            file.write_all(&previous).map_err(io)?;
            file.sync_all().map_err(io)?;
        }
        std::fs::remove_file(path).map_err(io)?;
    }
    std::fs::rename(temporary, path).map_err(io)
}

fn host(adapter: Adapter, args: &[&str]) -> Result<Json, Refusal> {
    let executable = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path).map(|p| p.join(crate::layout::program_file(adapter.name())))
            .find(|p| p.is_file())
    }).ok_or_else(|| Refusal::not_ready("host_missing", "A native Agent CLI executable is unavailable on PATH; install a supported Agent client first"))?;
    let words = adapter.arguments(args);
    let phase = crate::progress::Progress::waiting(
        "Agent setup",
        &format!("configuring Axiom skills in {}", adapter.label()),
    );
    let mut command = Command::new(executable);
    command.args(&words);
    let output = crate::progress::bounded_output(&mut command, Duration::from_secs(60))
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                Refusal::not_ready("host_timeout", format!(
                    "{} command timed out; inspect axm skills doctor {} before retrying",
                    adapter.label(), adapter.name()
                ))
            } else {
                Refusal::not_ready("host_command_failed", "Agent command could not complete; retained content and pending recovery record were kept")
            }
        })?;
    let status = output.status;
    let bytes = output.stdout;
    phase.finish(status.success());
    if !status.success() {
        return Err(Refusal::not_ready("host_command_failed", format!("Agent plugin command failed (exit {}); retained content and pending recovery record were kept", status.code().unwrap_or(8))));
    }
    if words.iter().any(|a| a == "--json") {
        if bytes.len() > 4_194_304 {
            return Err(invalid("host output exceeds budget"));
        }
        json::parse(std::str::from_utf8(&bytes).map_err(|_| invalid("host output is not UTF-8"))?)
            .map_err(invalid)
            .and_then(|result| adapter.normalize(args, result))
    } else {
        Ok(Json::Null)
    }
}

fn marketplace(adapter: Adapter) -> Result<Option<PathBuf>, Refusal> {
    let result = host(adapter, &["plugin", "marketplace", "list", "--json"])?;
    let rows = result
        .get("marketplaces")
        .and_then(Json::as_array)
        .ok_or_else(|| invalid("unsupported Agent marketplace response"))?;
    let found: Vec<_> = rows
        .iter()
        .filter(|r| r.get("name").and_then(Json::as_text) == Some("axiom-skills"))
        .collect();
    if found.len() > 1 {
        return Err(invalid("ambiguous Agent marketplace"));
    }
    found
        .first()
        .map(|r| text(r, "root").map(PathBuf::from))
        .transpose()
}

fn plugin(adapter: Adapter, version: &str) -> Result<bool, Refusal> {
    let result = host(
        adapter,
        &["plugin", "list", "--marketplace", "axiom-skills", "--json"],
    )?;
    let rows = result
        .get("installed")
        .and_then(Json::as_array)
        .ok_or_else(|| invalid("unsupported Agent plugin response"))?;
    Ok(rows.iter().any(|r| {
        r.get("name").and_then(Json::as_text) == Some("axiom")
            && r.get("marketplaceName").and_then(Json::as_text) == Some("axiom-skills")
            && r.get("installed").and_then(Json::as_bool) == Some(true)
            && r.get("enabled").and_then(Json::as_bool) == Some(true)
            && r.get("version").and_then(Json::as_text) == Some(version)
    }))
}

fn plugin_enabled(adapter: Adapter) -> Result<Option<bool>, Refusal> {
    let result = host(
        adapter,
        &["plugin", "list", "--marketplace", "axiom-skills", "--json"],
    )?;
    let rows = result
        .get("installed")
        .and_then(Json::as_array)
        .ok_or_else(|| invalid("unsupported Agent plugin response"))?;
    rows.iter()
        .find(|r| {
            r.get("name").and_then(Json::as_text) == Some("axiom")
                && r.get("marketplaceName").and_then(Json::as_text) == Some("axiom-skills")
                && r.get("installed").and_then(Json::as_bool) == Some(true)
        })
        .map(|r| {
            r.get("enabled")
                .and_then(Json::as_bool)
                .ok_or_else(|| invalid("host enabled state is unavailable"))
        })
        .transpose()
}

fn remove_plugin_if_present(adapter: Adapter) -> Result<(), Refusal> {
    let result = host(
        adapter,
        &["plugin", "list", "--marketplace", "axiom-skills", "--json"],
    )?;
    let rows = result
        .get("installed")
        .and_then(Json::as_array)
        .ok_or_else(|| invalid("unsupported Agent plugin response"))?;
    if rows.iter().any(|r| {
        r.get("name").and_then(Json::as_text) == Some("axiom")
            && r.get("marketplaceName").and_then(Json::as_text) == Some("axiom-skills")
            && r.get("installed").and_then(Json::as_bool) == Some(true)
    }) {
        host(adapter, &["plugin", "remove", "axiom@axiom-skills"])?;
    }
    Ok(())
}

fn recover(adapter: Adapter, root: &Path, args: &[&str]) -> Result<Report, Refusal> {
    let receipt_path = safe_path(root, &adapter.receipt())?;
    let pending_path = receipt_path.with_extension("pending.json");
    let pending = read(&pending_path)?;
    let current = marketplace(adapter)?;
    if pending.get("previous_enabled").and_then(Json::as_bool) == Some(false)
        || (current.is_some() && plugin_enabled(adapter)? == Some(false))
    {
        return Err(Refusal::new(Class::Conflict, "host_disabled_preserved", "Recovery would affect a disabled plugin; its host settings and pending inputs were preserved for client-managed recovery"));
    }
    let pending_digest = sha256::file_hex(&pending_path).map_err(io)?;
    let previous = pending
        .get("previous")
        .filter(|v| matches!(v, Json::Object(_)));
    let next = pending.get("next").filter(|v| matches!(v, Json::Object(_)));
    let mut owned = Vec::new();
    for record in [previous, next].into_iter().flatten() {
        if text(record, "host")? != adapter.name() {
            return Err(invalid("pending ownership belongs to another host"));
        }
        let path = std::fs::canonicalize(owned_snapshot(root, record)?).map_err(io)?;
        if current
            .as_ref()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .as_ref()
            == Some(&path)
        {
            verify_host_cache(adapter, record)?;
        }
        owned.push(path);
    }
    if let Some(path) = &current {
        if !owned.contains(&std::fs::canonicalize(path).map_err(io)?) {
            return Err(Refusal::new(
                Class::Conflict,
                "host_registration_modified",
                "Recovery found a foreign marketplace; nothing was changed",
            ));
        }
    }
    let receipt_digest = receipt_path
        .exists()
        .then(|| sha256::file_hex(&receipt_path))
        .transpose()
        .map_err(io)?;
    if receipt_path.exists() {
        let actual = json::canonical_text(&read(&receipt_path)?);
        if ![previous, next]
            .into_iter()
            .flatten()
            .any(|v| json::canonical_text(v) == actual)
        {
            return Err(Refusal::new(
                Class::Conflict,
                "host_receipt_modified",
                "Managed host receipt changed; recovery preserved it",
            ));
        }
    }
    let lines = vec!["Recover only the pending Axiom-owned Agent plugin registration to its previous state; retain all bundle snapshots.".to_string()];
    if args.contains(&"--dry-run") {
        let mut r = Report::ok("planned", "host recovery plan");
        r.lines(lines);
        return Ok(r);
    }
    if matches!(
        confirm::decide(args.contains(&"--yes"), args.contains(&"--json"), &lines),
        Decision::Declined | Decision::NoTerminal
    ) {
        return Err(Refusal::not_ready(
            "confirmation_required",
            "Host recovery requires confirmation or --yes",
        ));
    }
    let state = State::new(root.to_path_buf());
    let _lock = crate::update::apply::Lock::acquire(&state, "host-skills-recovery")?;
    if sha256::file_hex(&pending_path).map_err(io)? != pending_digest
        || marketplace(adapter)? != current
        || receipt_path
            .exists()
            .then(|| sha256::file_hex(&receipt_path))
            .transpose()
            .map_err(io)?
            != receipt_digest
    {
        return Err(Refusal::new(
            Class::Conflict,
            "host_recovery_drift",
            "Host recovery inputs changed after approval",
        ));
    }
    let apply = || -> Result<(), Refusal> {
        if current.is_some() {
            remove_plugin_if_present(adapter)?;
            host(
                adapter,
                &["plugin", "marketplace", "remove", "axiom-skills"],
            )?;
        }
        if let Some(record) = previous {
            let path = owned_snapshot(root, record)?;
            host(
                adapter,
                &[
                    "plugin",
                    "marketplace",
                    "add",
                    &path.display().to_string(),
                    "--json",
                ],
            )?;
            host(adapter, &["plugin", "add", "axiom@axiom-skills", "--json"])?;
            if !plugin(adapter, text(record, "version")?)? {
                return Err(invalid(
                    "previous plugin registration could not be verified",
                ));
            }
            verify_host_cache(adapter, record)?;
            write(&receipt_path, record)?;
        } else if receipt_path.exists() {
            std::fs::remove_file(&receipt_path).map_err(io)?;
        }
        std::fs::remove_file(&pending_path).map_err(io)?;
        Ok(())
    };
    apply().map_err(|e| {
        Refusal::not_ready(
            "host_apply_partial",
            format!(
                "Host recovery is incomplete: {}. Pending record and content retained.",
                e.message
            ),
        )
    })?;
    let mut report = Report::ok("ok", "previous Agent registration restored");
    report.lines(["Agent host recovery: OK; retained snapshots were preserved."]);
    Ok(report)
}

fn snapshot(root: &Path, bundle: &Bundle) -> Result<PathBuf, Refusal> {
    let destination = safe_path(
        root,
        &format!("host-integrations/bundles/{}", bundle.digest),
    )?;
    if destination.exists() {
        verify_entries(&destination, &bundle.manifest)?;
        return Ok(destination);
    }
    std::fs::create_dir_all(destination.parent().unwrap()).map_err(io)?;
    let staging = destination.with_extension(format!("{}.staging", std::process::id()));
    std::fs::create_dir(&staging).map_err(io)?;
    for row in bundle
        .manifest
        .get("entries")
        .and_then(Json::as_array)
        .unwrap()
    {
        let name = text(row, "path")?;
        let source = safe_path(&bundle.source, name)?;
        let target = safe_path(&staging, name)?;
        std::fs::create_dir_all(target.parent().unwrap()).map_err(io)?;
        std::fs::copy(source, target).map_err(io)?;
    }
    verify_entries(&staging, &bundle.manifest)?;
    std::fs::rename(staging, &destination).map_err(io)?;
    Ok(destination)
}

fn owned_snapshot(root: &Path, receipt: &Json) -> Result<PathBuf, Refusal> {
    let digest = text(receipt, "manifest_sha256")?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(invalid("invalid snapshot ownership digest"));
    }
    let path = safe_path(root, &format!("host-integrations/bundles/{digest}"))?;
    if std::fs::canonicalize(&path).map_err(io)?
        != std::fs::canonicalize(text(receipt, "marketplace_root")?).map_err(io)?
    {
        return Err(invalid("snapshot ownership path was changed"));
    }
    verify_entries(
        &path,
        receipt
            .get("manifest")
            .ok_or_else(|| invalid("missing snapshot manifest"))?,
    )
    .map_err(|_| {
        Refusal::new(
            Class::Conflict,
            "host_snapshot_modified",
            "Managed plugin content was modified or is incomplete; it was preserved",
        )
    })?;
    Ok(path)
}

pub fn preview(host_name: &str) -> Result<Json, Refusal> {
    let adapter = Adapter::parse(host_name)?;
    let root = crate::update::state::default_root()
        .ok_or_else(|| invalid("installation root unavailable"))?;
    let path = safe_path(&root, &adapter.receipt())?;
    if path.with_extension("pending.json").exists() {
        return Err(Refusal::not_ready(
            "host_recovery_pending",
            "A host transaction needs recovery; run axm skills recover HOST",
        ));
    }
    let current = marketplace(adapter)?;
    if let Some(actual) = &current {
        if !path.exists() {
            return Err(Refusal::new(Class::Conflict, "host_marketplace_foreign", "axiom-skills is already registered outside Axiom management; its configuration was preserved"));
        }
        let receipt = read(&path)?;
        if text(&receipt, "host")? != adapter.name() {
            return Err(invalid("managed receipt belongs to another host"));
        }
        let owned = owned_snapshot(&root, &receipt)?;
        verify_host_cache(adapter, &receipt)?;
        if std::fs::canonicalize(actual).map_err(io)? != std::fs::canonicalize(owned).map_err(io)? {
            return Err(Refusal::new(
                Class::Conflict,
                "host_registration_modified",
                "Agent marketplace source changed; nothing was overwritten",
            ));
        }
    }
    let enabled = if current.is_some() {
        plugin_enabled(adapter)?
    } else {
        None
    };
    Ok(Json::from_pairs(vec![
        ("host", Json::text(host_name)),
        (
            "plugin_enabled",
            enabled.map(Json::bool).unwrap_or(Json::Null),
        ),
        (
            "marketplace",
            current
                .as_ref()
                .map(|p| Json::text(&p.display().to_string()))
                .unwrap_or(Json::Null),
        ),
        (
            "receipt_sha256",
            if path.exists() {
                Json::text(&sha256::file_hex(&path).map_err(io)?)
            } else {
                Json::Null
            },
        ),
    ]))
}

pub fn apply_approved(plan: &Json) -> Report {
    let Some(name) = plan.get("host").and_then(Json::as_text) else {
        return Report::refused("approved host is missing");
    };
    let adapter = match Adapter::parse(name) {
        Ok(a) => a,
        Err(e) => return Report::refused(e.message),
    };
    let managed = crate::update::state::default_root()
        .is_some_and(|root| root.join(adapter.receipt()).is_file());
    let args = [
        OsString::from(if managed { "update" } else { "install" }),
        OsString::from(name),
        OsString::from("--yes"),
        OsString::from("--json"),
    ];
    match operation(&args, Some(plan)) {
        Ok(report) => report,
        Err(error) => Report::new(
            error.class.exit_code(),
            error.class.status(),
            error.message.replace("HOST", name),
        ),
    }
}

fn operation(args: &[OsString], approved: Option<&Json>) -> Result<Report, Refusal> {
    let strings: Vec<_> = args
        .iter()
        .map(|a| a.to_str().ok_or_else(|| invalid("arguments must be UTF-8")))
        .collect::<Result<_, _>>()?;
    let positional: Vec<_> = strings
        .iter()
        .copied()
        .filter(|a| !a.starts_with('-'))
        .collect();
    let action = positional.first().copied().unwrap_or("list");
    if strings.iter().any(|a| {
        a.starts_with('-') && !matches!(*a, "--yes" | "--json" | "--dry-run" | "--verbose")
    }) {
        return Err(invalid("unknown host skills option; use axm skills --help"));
    }
    if action != "list" && positional.len() != 2 {
        return Err(invalid(
            "expected a managed host: codex or claude; use axm skills --help",
        ));
    }
    if !matches!(
        action,
        "install" | "update" | "doctor" | "uninstall" | "list" | "recover"
    ) {
        return Err(invalid(
            "expected install, update, doctor, uninstall or list",
        ));
    }
    let adapter = if action == "list" {
        Adapter::Codex
    } else {
        Adapter::parse(positional[1])?
    };
    let root = crate::update::state::default_root()
        .ok_or_else(|| invalid("installation root unavailable"))?;
    if action == "recover" {
        return recover(adapter, &root, &strings);
    }
    if action == "list" {
        if positional.len() != 1 {
            return Err(invalid("axm skills list takes no host argument"));
        }
        let mut report = Report::ok("ok", "host skills registration inventory");
        let mut hosts = Vec::new();
        for adapter in Adapter::ALL {
            let managed = safe_path(&root, &adapter.receipt())?.is_file();
            let registered = match marketplace(adapter) {
                Ok(value) => Some(value.is_some()),
                Err(error) if error.reason == "host_missing" => None,
                Err(error) => return Err(error),
            };
            hosts.push(Json::from_pairs(vec![
                ("host", Json::text(adapter.name())),
                ("managed", Json::bool(managed)),
                (
                    "registered",
                    registered.map(Json::bool).unwrap_or(Json::Null),
                ),
                ("host_available", Json::bool(registered.is_some())),
            ]));
            if adapter == Adapter::Codex {
                report.detail("managed", Json::bool(managed));
                report.detail(
                    "registered",
                    registered.map(Json::bool).unwrap_or(Json::Null),
                );
                report.detail("host_available", Json::bool(registered.is_some()));
            }
            report.line(format!(
                "{}: managed record={managed}; registration={}",
                adapter.label(),
                registered
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "host unavailable".into())
            ));
        }
        report.detail("hosts", Json::array(hosts));
        return Ok(report);
    }
    let epoch = preview(adapter.name())?;
    if let Some(expected) = approved {
        let mut original = expected.clone();
        if let Json::Object(ref mut values) = original {
            values.remove("version");
            values.remove("revision");
        }
        if json::canonical_text(&original) != json::canonical_text(&epoch) {
            return Err(Refusal::new(
                Class::Conflict,
                "host_plan_drift",
                "Host registration changed after the combined plan was approved",
            ));
        }
    }
    let receipt_path = safe_path(&root, &adapter.receipt())?;
    let receipt = if receipt_path.exists() {
        let record = read(&receipt_path)?;
        if text(&record, "host")? != adapter.name() {
            return Err(invalid("managed receipt belongs to another host"));
        }
        Some(record)
    } else {
        None
    };
    let registered = marketplace(adapter)?;
    let old_path = receipt
        .as_ref()
        .map(|r| text(r, "marketplace_root").map(PathBuf::from))
        .transpose()?;
    match (&registered, &old_path) {
        (Some(actual), Some(expected)) if std::fs::canonicalize(actual).map_err(io)? != std::fs::canonicalize(expected).map_err(io)? => return Err(Refusal::new(Class::Conflict, "host_registration_modified", "Agent marketplace was changed outside Axiom; nothing was overwritten")),
        (Some(_), None) => return Err(Refusal::new(Class::Conflict, "host_marketplace_foreign", "axiom-skills already exists in Agent and is not Axiom-managed; preserve it or remove that registration explicitly before managed install")),
        _ => {}
    }
    let bundle = installed_bundle(&root)?;
    verify_host_bundle(adapter, &bundle)?;
    if let Some(expected) = approved {
        if text(expected, "version")? != bundle.version
            || text(expected, "revision")? != text(&bundle.manifest, "revision")?
        {
            return Err(Refusal::new(
                Class::Conflict,
                "host_release_drift",
                "Installed skills differ from the approved combined plan",
            ));
        }
    }
    if action == "doctor" {
        if epoch.get("plugin_enabled").and_then(Json::as_bool) == Some(false) {
            let mut report = Report::ok(
                "disabled",
                "Agent plugin is intentionally disabled; its configuration was preserved",
            );
            report.detail("host_discovery", Json::text("disabled"));
            report.detail("mcp_query", Json::text("not_checked"));
            report.lines([
                "Agent plugin: disabled (unchanged)",
                "MCP query readiness: not checked",
            ]);
            return Ok(report);
        }
        if receipt.is_none() || registered.is_none() || !plugin(adapter, &bundle.version)? {
            return Err(Refusal::not_ready("host_not_ready", "Agent plugin is absent, disabled or differs from the installed bundle; run axm skills install/update HOST"));
        }
        let mut report = Report::ok("ok", "Agent plugin registration verified");
        report.detail("registration", Json::text("verified"));
        report.detail(
            "host_discovery",
            Json::text("new_session_required_not_verified"),
        );
        report.detail("mcp_query", Json::text("not_checked"));
        report.lines([
            "Agent plugin registration: OK",
            "Agent skill discovery: open a new session to verify",
            "MCP query readiness: not checked",
        ]);
        return Ok(report);
    }
    if matches!(action, "install" | "update")
        && epoch.get("plugin_enabled").and_then(Json::as_bool) == Some(false)
    {
        if action == "install" {
            return Err(Refusal::not_ready("host_plugin_disabled", "Agent plugin is disabled in the host; enable it in your client before reinstalling. No host setting was changed"));
        }
        let mut report = Report::ok(
            "skipped_disabled",
            "Disabled Agent plugin was preserved; host content was not refreshed",
        );
        report.detail("host_discovery", Json::text("disabled"));
        report.detail("mcp_query", Json::text("not_checked"));
        report.lines(["Agent plugin: disabled; unchanged (no implicit re-enable)"]);
        return Ok(report);
    }
    if matches!(action, "update" | "uninstall") && receipt.is_none() {
        return Err(Refusal::not_ready(
            "host_not_managed",
            "No managed Agent registration; run axm skills install HOST",
        ));
    }
    let plan = vec![format!("Agent skills {action}: verified bundle {} ({})", bundle.version, bundle.digest),
        "Changes only the Axiom-owned plugin registration; retained snapshots and unrelated host settings are preserved.".into()];
    if strings.contains(&"--dry-run") {
        let mut report = Report::ok("planned", "host skills plan; nothing changed");
        report.lines(plan);
        return Ok(report);
    }
    let receipt_before = receipt_path
        .exists()
        .then(|| sha256::file_hex(&receipt_path))
        .transpose()
        .map_err(io)?;
    if matches!(
        confirm::decide(
            strings.contains(&"--yes"),
            strings.contains(&"--json"),
            &plan
        ),
        Decision::Declined | Decision::NoTerminal
    ) {
        return Err(Refusal::not_ready(
            "confirmation_required",
            "Host installation was not approved; use --yes or an interactive confirmation",
        ));
    }
    let state = State::new(root.clone());
    let _lock = crate::update::apply::Lock::acquire(&state, "host-skills")?;
    if receipt_before
        != receipt_path
            .exists()
            .then(|| sha256::file_hex(&receipt_path))
            .transpose()
            .map_err(io)?
        || json::canonical_text(&preview(adapter.name())?) != json::canonical_text(&epoch)
        || installed_bundle(&root)?.digest != bundle.digest
    {
        return Err(Refusal::new(
            Class::Conflict,
            "host_plan_drift",
            "Host or bundle changed after approval; inspect and replan",
        ));
    }
    if let Some(previous) = &receipt {
        verify_entries(
            old_path.as_ref().unwrap(),
            previous
                .get("manifest")
                .ok_or_else(|| invalid("missing owned snapshot manifest"))?,
        )?;
    }
    std::fs::create_dir_all(receipt_path.parent().unwrap()).map_err(io)?;
    let target = if action == "uninstall" {
        None
    } else {
        Some(snapshot(&root, &bundle)?)
    };
    let pending = receipt_path.with_extension("pending.json");
    write(
        &pending,
        &Json::from_pairs(vec![
            ("action", Json::text(action)),
            ("previous", receipt.clone().unwrap_or(Json::Null)),
            (
                "previous_enabled",
                epoch.get("plugin_enabled").cloned().unwrap_or(Json::Null),
            ),
            (
                "next",
                target
                    .as_ref()
                    .map(|path| {
                        Json::from_pairs(vec![
                            ("host", Json::text(adapter.name())),
                            ("version", Json::text(&bundle.version)),
                            ("manifest_sha256", Json::text(&bundle.digest)),
                            ("manifest", bundle.manifest.clone()),
                            ("marketplace_root", Json::text(&path.display().to_string())),
                        ])
                    })
                    .unwrap_or(Json::Null),
            ),
            (
                "target",
                target
                    .as_ref()
                    .map(|p| Json::text(&p.display().to_string()))
                    .unwrap_or(Json::Null),
            ),
        ]),
    )?;
    let apply = || -> Result<(), Refusal> {
        if registered.is_some() {
            remove_plugin_if_present(adapter)?;
            host(
                adapter,
                &["plugin", "marketplace", "remove", "axiom-skills"],
            )?;
        }
        if let Some(path) = &target {
            host(
                adapter,
                &[
                    "plugin",
                    "marketplace",
                    "add",
                    &path.display().to_string(),
                    "--json",
                ],
            )?;
            host(adapter, &["plugin", "add", "axiom@axiom-skills", "--json"])?;
            if !plugin(adapter, &bundle.version)? {
                return Err(Refusal::not_ready(
                    "host_verification_failed",
                    "Agent did not report the selected plugin enabled; recovery record retained",
                ));
            }
        }
        if target.is_some() {
            verify_host_cache(
                adapter,
                &Json::from_pairs(vec![("manifest", bundle.manifest.clone())]),
            )?;
        }
        Ok(())
    };
    if let Err(error) = apply() {
        return Err(Refusal::not_ready("host_apply_partial", format!("Host setup may be partial: {}. Previous content is retained; run axm skills recover HOST before retrying.", error.message)));
    }
    if installed_bundle(&root).map(|b| b.digest).ok().as_deref() != Some(bundle.digest.as_str()) {
        return Err(Refusal::not_ready("host_apply_partial", "The runtime skills changed during host setup; registration may be partial, previous content and recovery metadata were retained"));
    }
    let finalize = || -> Result<(), Refusal> {
        if receipt_before
            != receipt_path
                .exists()
                .then(|| sha256::file_hex(&receipt_path))
                .transpose()
                .map_err(io)?
        {
            return Err(Refusal::not_ready(
                "host_apply_partial",
                "The host receipt changed concurrently and was preserved",
            ));
        }
        if let Some(path) = target {
            if receipt_before
                != receipt_path
                    .exists()
                    .then(|| sha256::file_hex(&receipt_path))
                    .transpose()
                    .map_err(io)?
            {
                return Err(Refusal::not_ready("host_apply_partial", "Host registration was applied but its receipt changed concurrently; the receipt and pending recovery were preserved"));
            }
            write(
                &receipt_path,
                &Json::from_pairs(vec![
                    ("host", Json::text(adapter.name())),
                    ("version", Json::text(&bundle.version)),
                    ("manifest_sha256", Json::text(&bundle.digest)),
                    ("manifest", bundle.manifest),
                    ("marketplace_root", Json::text(&path.display().to_string())),
                ]),
            )?;
        } else {
            std::fs::remove_file(&receipt_path).map_err(io)?;
        }
        std::fs::remove_file(pending).map_err(io)?;
        Ok(())
    };
    finalize().map_err(|e| Refusal::not_ready("host_apply_partial", format!("Host registration was applied but its metadata commit is incomplete: {}. Recovery inputs are retained.", e.message)))?;
    let mut report = Report::ok("ok", format!("Agent skills {action} completed"));
    report.detail(
        "host_discovery",
        Json::text("new_session_required_not_verified"),
    );
    report.detail("mcp_query", Json::text("not_checked"));
    report.lines([
        format!("Agent skills {action}: OK"),
        "Open a new agent session. MCP credentials and graph access are checked separately.".into(),
    ]);
    Ok(report)
}

pub fn run(args: &[OsString]) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") || args.is_empty() {
        println!("{HELP}");
        return 0;
    }
    let report = match operation(args, None).map_err(|mut error| {
        if let Some(name) = args
            .iter()
            .filter_map(|a| a.to_str())
            .find(|a| matches!(*a, "codex" | "claude"))
        {
            error.message = error.message.replace("HOST", name);
        }
        error
    }) {
        Ok(report) => report,
        Err(error) => {
            let (code, status) = if error.reason == "host_apply_partial" {
                (20, "partial")
            } else {
                (error.class.exit_code(), error.class.status())
            };
            let mut report = Report::new(code, status, error.message);
            report.detail("reason_code", Json::text(&error.reason));
            report
        }
    };
    report.emit(
        args.iter().any(|a| a == "--json"),
        args.iter().any(|a| a == "--verbose"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn payload_paths_refuse_escape_and_links_before_host_commands() {
        let root = std::env::temp_dir().join(format!("host-skills-paths-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for name in [
            "../escape",
            "/absolute",
            "a//b",
            "a/./b",
            "C:/drive",
            "a\\b",
        ] {
            assert!(safe_path(&root, name).is_err(), "{name}");
        }
        assert_eq!(
            safe_path(&root, "plugins/axiom/skills/axsk/SKILL.md").unwrap(),
            root.join("plugins/axiom/skills/axsk/SKILL.md")
        );
        std::fs::remove_dir(&root).unwrap();
    }
    #[test]
    fn modified_duplicate_and_missing_payloads_are_refused() {
        let root = std::env::temp_dir().join(format!("host-skills-bytes-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("skill.md"), b"trusted").unwrap();
        let row = Json::from_pairs(vec![
            ("path", Json::text("skill.md")),
            ("sha256", Json::text(&sha256::digest_hex(b"trusted"))),
            ("size_bytes", Json::int(7)),
        ]);
        let manifest = Json::from_pairs(vec![("entries", Json::array(vec![row.clone()]))]);
        assert!(verify_entries(&root, &manifest).is_ok());
        assert!(verify_entries(
            &root,
            &Json::from_pairs(vec![("entries", Json::array(vec![row.clone(), row]))])
        )
        .is_err());
        std::fs::write(root.join("skill.md"), b"changed").unwrap();
        assert!(verify_entries(&root, &manifest).is_err());
        std::fs::remove_file(root.join("skill.md")).unwrap();
        assert!(verify_entries(&root, &manifest).is_err());
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn changed_backups_are_preserved_before_replacing_an_owned_record() {
        let root = std::env::temp_dir().join(format!("host-record-backup-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let record = root.join("codex.json");
        let previous = b"{\"managed\":true}";
        std::fs::write(&record, previous).unwrap();
        let backup =
            record.with_extension(format!("previous-{}.json", sha256::digest_hex(previous)));
        std::fs::write(&backup, b"human-modified backup").unwrap();
        assert!(write(&record, &Json::from_pairs(vec![("next", Json::bool(true))])).is_err());
        assert_eq!(std::fs::read(&record).unwrap(), previous);
        assert_eq!(std::fs::read(&backup).unwrap(), b"human-modified backup");
        for item in std::fs::read_dir(&root).unwrap() {
            std::fs::remove_file(item.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
}
