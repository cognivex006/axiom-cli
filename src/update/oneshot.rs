//! One-command update (ADR-0033 decision 6, task L-005): bare `axm update`.
//!
//! ```text
//! read installed.json (installed version, recorded channel source)
//! -> fetch the channel manifest (`channel.json` release asset) from that source
//! -> pin the exact version, archive URL, SHA-256 and manifest digest into a plan
//!    ("up to date" when the channel offers nothing newer)
//! -> print the plan, ask once (`--yes` pre-approves; no terminal and no --yes: exit 4)
//! -> re-fetch the manifest and refuse if its digest changed after planning
//! -> download the archive, verify length and SHA-256 before use, extract to staging
//! -> hand the extracted release to its own `axm install` in engine-update mode, so the
//!    graphd engine runs its update transaction and the new bin + installed.json are recorded
//!    with the old generation kept as `previous_generation`
//! -> health-check the new bin; on failure roll back the engine transaction, the bin files and
//!    installed.json
//! ```
//!
//! Versions come only from the fetched manifest's pinned entry: a tag alias, a branch tip, `*` or
//! a `releases/latest` archive URL is refused. The channel *source* may be the GitHub
//! `releases/latest/download/channel.json` discovery URL, but nothing inside the transaction ever
//! resolves `latest`: the archive is the exact pinned URL and digest that were approved. User data
//! and the graph output root are outside everything this module writes.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::error::{Class, Refusal};
use super::json::{self, Json};
use super::report::Report;
use super::state::{self, State};
use super::{plan as plan_contract, rules, sha256};

/// Where an install created by the one-line installer discovers newer releases.
pub const CANONICAL_CHANNEL_SOURCE: &str =
    "https://github.com/orchex006/axiom-cli/releases/latest/download/channel.json";
/// Test seam: force the post-update health check to fail so the rollback path can be exercised.
pub const FAIL_HEALTH_TEST_ENV: &str = "AXIOM_CLI_TEST_FAIL_HEALTH";

/// A parsed bare `update` request.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// `--channel <url|path>`: an explicit channel manifest source.
    pub channel: Option<String>,
    /// `--yes`: approve the printed plan without a prompt.
    pub yes: bool,
    /// `--dry-run`: print the plan and stop.
    pub dry_run: bool,
}

/// Run bare `update`.
pub fn run(request: Request, json_output: bool, verbose: bool) -> i32 {
    let Some(root) = state::default_root() else {
        return refusal_report(&Refusal::not_ready(
            "no_install_root",
            "this host offers no per-user install root",
        ))
        .emit(json_output, verbose);
    };
    match update(&root, &request, json_output) {
        Ok(report) => report.emit(json_output, verbose),
        Err(refusal) => refusal_report(&refusal).emit(json_output, verbose),
    }
}

fn refusal_report(refusal: &Refusal) -> Report {
    let mut report = match refusal.class {
        Class::NotReady => Report::not_ready(refusal.message.clone()),
        Class::Validation => Report::refused(refusal.message.clone()),
        other => Report::new(other.exit_code(), other.status(), refusal.message.clone()),
    }
    .subject("update");
    report.detail("reason", Json::text(&refusal.message));
    report.detail("reason_code", Json::text(&refusal.reason));
    report
}

/// The release a channel manifest offers for this host.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub channel: String,
    pub version: String,
    pub tag: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: i64,
}

/// Parse and validate the `release` block of a channel manifest for `host`.
pub fn offer(manifest: &[u8], host: &str) -> Result<Offer, Refusal> {
    let text = std::str::from_utf8(manifest).map_err(|_| {
        Refusal::validation("channel_not_utf8", "the channel manifest is not UTF-8")
    })?;
    let value = json::parse(text).map_err(|error| {
        Refusal::validation(
            "channel_not_json",
            format!("the channel manifest is not JSON: {error}"),
        )
    })?;
    let channel = value
        .get("channel")
        .and_then(Json::as_text)
        .unwrap_or("")
        .to_string();
    let release = value.get("release").ok_or_else(|| {
        Refusal::validation(
            "channel_release_missing",
            "the channel manifest has no `release` block naming a pinned release archive",
        )
    })?;
    let field = |key: &str| {
        release
            .get(key)
            .and_then(Json::as_text)
            .unwrap_or("")
            .to_string()
    };
    let version = field("version");
    let tag = field("tag");
    if let Some(pin) = rules::forbidden_pin_in(&version).or_else(|| rules::forbidden_pin_in(&tag)) {
        return Err(Refusal::validation(
            format!("forbidden_pin:{pin}"),
            format!("the channel offers `{pin}`, which is a moving reference, not a release"),
        ));
    }
    if !rules::is_semver(&version) || tag != format!("v{version}") {
        return Err(Refusal::validation(
            "channel_release_version_invalid",
            format!("the channel offers version `{version}` with tag `{tag}`; a pinned vX.Y.Z tag is required"),
        ));
    }
    let archive = release
        .get("archives")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .find(|item| item.get("platform").and_then(Json::as_text) == Some(host))
        .ok_or_else(|| {
            Refusal::new(
                Class::Incompatible,
                "channel_platform_missing",
                format!("the channel offers {version} but no archive for host {host}"),
            )
        })?;
    let url = archive
        .get("url")
        .and_then(Json::as_text)
        .unwrap_or("")
        .to_string();
    let sha = archive
        .get("sha256")
        .and_then(Json::as_text)
        .unwrap_or("")
        .to_string();
    let size = archive
        .get("size_bytes")
        .and_then(Json::as_int)
        .unwrap_or(0);
    let pinned_path = format!("/{tag}/");
    let moving = url
        .split('/')
        .any(|segment| segment == "latest" || rules::FORBIDDEN_PINS.contains(&segment));
    if moving
        || !(url.starts_with("https://")
            || url.starts_with("http://127.0.0.1")
            || url.starts_with("http://localhost")
            || url.starts_with("file:"))
        || !(url.contains(&pinned_path) || url.starts_with("file:"))
    {
        return Err(Refusal::validation(
            "channel_archive_url_not_pinned",
            format!("archive URL `{url}` is not an exact URL under the pinned tag {tag}"),
        ));
    }
    if !rules::is_digest64(&sha) || size <= 0 {
        return Err(Refusal::validation(
            "channel_archive_digest_invalid",
            "the archive entry needs a 64-hex `sha256` and a positive `size_bytes`",
        ));
    }
    Ok(Offer {
        channel,
        version,
        tag,
        url,
        sha256: sha,
        size_bytes: size,
    })
}

/// Compare two SemVer strings (`X.Y.Z[-pre]`); a prerelease sorts before its release.
pub fn semver_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |v: &str| {
        let (core, pre) = v
            .split_once('-')
            .map_or((v, None), |(c, p)| (c, Some(p.to_string())));
        let nums: Vec<u64> = core
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect();
        (nums, pre)
    };
    let (na, pa) = split(a);
    let (nb, pb) = split(b);
    na.cmp(&nb).then_with(|| match (pa, pb) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
    })
}

/// Copy a local file or download an http(s) URL to `dest` (program + argv, never a shell).
pub fn fetch(source: &str, dest: &Path) -> Result<(), Refusal> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            Refusal::io("staging_unwritable", &parent.display().to_string(), &error)
        })?;
    }
    let local = source
        .strip_prefix("file:")
        .map(|rest| rest.trim_start_matches("//"));
    if let Some(path) = local.or_else(|| (!source.contains("://")).then_some(source)) {
        std::fs::copy(path, dest)
            .map_err(|error| Refusal::io("channel_source_unreadable", path, &error))?;
        return Ok(());
    }
    let curl = system_tool("curl");
    let status = Command::new(&curl)
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(dest)
        .arg(source)
        .status()
        .map_err(|error| {
            Refusal::not_ready(
                "download_tool_unavailable",
                format!(
                    "`{}` could not be started ({error}); curl is required to download updates",
                    curl.display()
                ),
            )
        })?;
    if !status.success() {
        let _ = std::fs::remove_file(dest);
        return Err(Refusal::new(
            Class::NotReady,
            "download_failed",
            format!(
                "downloading {source} failed (curl exit {:?}); nothing was changed",
                status.code()
            ),
        ));
    }
    Ok(())
}

/// The OS-provided tool: `%SystemRoot%\System32\<name>.exe` on Windows (its `tar` reads zip;
/// a Git-for-Windows GNU tar earlier on PATH would not), else the name on PATH.
fn system_tool(name: &str) -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        let path = Path::new(&root)
            .join("System32")
            .join(format!("{name}.exe"));
        if path.is_file() {
            return path;
        }
    }
    PathBuf::from(name)
}

fn digest_of(bytes: &[u8]) -> String {
    sha256::hex(&sha256::digest(bytes))
}

/// The installed release version: the core (`axiom-graphd`) version, because the daemon and the
/// CLI share one core release while MCP and skills version independently.
fn installed_version(st: &State) -> Result<(state::Installed, String), Refusal> {
    let installed = st.read_installed()?;
    let version = installed
        .extra
        .get("components")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .find(|item| item.get("component").and_then(Json::as_text) == Some("axiom-graphd"))
        .and_then(|item| item.get("version").and_then(Json::as_text))
        .map(str::to_string)
        .ok_or_else(|| {
            Refusal::not_ready(
                "installed_version_unknown",
                "installed.json does not record the core release version (an install made \
                 by an older installer); run the one-line installer once to record it",
            )
        })?;
    Ok((installed, version))
}

/// The MCP runtime pointer the provisioner owns; snapshotted so a failed or rolled-back update
/// points the runtime back at the generation that was active before.
fn mcp_pointer(root: &Path) -> PathBuf {
    root.join("mcp-runtime").join("current.json")
}

fn restore_mcp_pointer(root: &Path, snapshot: Option<&[u8]>) -> Result<(), Refusal> {
    let Some(bytes) = snapshot else {
        return Ok(());
    };
    let pointer = mcp_pointer(root);
    state::write_atomic(&pointer.with_extension("json.axiom-tmp"), &pointer, bytes)
}

fn update(root: &Path, request: &Request, json_output: bool) -> Result<Report, Refusal> {
    let st = State::new(root);
    let (installed, current) = installed_version(&st)?;
    let host = crate::target::host_id().ok_or_else(|| {
        Refusal::new(
            Class::Incompatible,
            "undeclared_target",
            "this host is not a declared delivery platform",
        )
    })?;
    let source = request
        .channel
        .clone()
        .or_else(|| {
            installed
                .extra
                .get("channel_source")
                .and_then(Json::as_text)
                .map(str::to_string)
        })
        .unwrap_or_else(|| CANONICAL_CHANNEL_SOURCE.to_string());
    let staging = root.join(state::STAGING_DIR).join("update");
    let manifest_path = staging.join("channel.json");
    fetch(&source, &manifest_path)?;
    let manifest = state::read_bytes(&manifest_path, "channel_unreadable")?;
    let manifest_sha = digest_of(&manifest);
    let offer = offer(&manifest, host)?;
    if offer.channel != installed.channel {
        return Err(Refusal::validation(
            "channel_mismatch",
            format!(
                "the source offers channel `{}` but this install follows `{}`",
                offer.channel, installed.channel
            ),
        ));
    }
    if semver_cmp(&offer.version, &current) != std::cmp::Ordering::Greater {
        let mut report = Report::ok(
            "ok",
            format!(
                "up to date: installed {current}; channel `{}` offers {}",
                offer.channel, offer.version
            ),
        )
        .subject("update");
        report.detail("installed_version", Json::text(&current));
        report.detail("available_version", Json::text(&offer.version));
        report.detail("manifest_sha256", Json::text(&manifest_sha));
        report.line(format!("up to date: {current}"));
        let _ = std::fs::remove_dir_all(&staging);
        return Ok(report);
    }
    let mut plan = Json::from_pairs(vec![
        ("plan_version", Json::int(1)),
        ("verb", Json::text("update")),
        ("host", Json::text(host)),
        ("install_root", Json::text(&root.display().to_string())),
        ("channel", Json::text(&offer.channel)),
        ("from_version", Json::text(&current)),
        ("to_version", Json::text(&offer.version)),
        ("tag", Json::text(&offer.tag)),
        (
            "archive",
            Json::from_pairs(vec![
                ("url", Json::text(&offer.url)),
                ("sha256", Json::text(&offer.sha256)),
                ("size_bytes", Json::int(offer.size_bytes)),
            ]),
        ),
        ("manifest_source", Json::text(&source)),
        ("manifest_sha256", Json::text(&manifest_sha)),
        (
            "keeps",
            Json::text("user data, workspaces, graph output; previous generation for rollback"),
        ),
    ]);
    let digest = plan_contract::digest(&plan).ok_or_else(|| {
        Refusal::validation("plan_unbuildable", "the update plan could not be digested")
    })?;
    let _ = plan.set("plan_digest", Json::text(&digest));
    let lines = vec![
        format!("update {current} -> {} (channel {})", offer.version, offer.channel),
        format!("download: {} ({:.1} MB, sha256 {})", offer.url, offer.size_bytes as f64 / 1_000_000.0, offer.sha256),
        format!("install root: {}", root.display()),
        "keep: user data, workspaces and graph output; the current version stays as the rollback generation".to_string(),
        format!("plan digest: {digest}"),
    ];
    if request.dry_run {
        let mut report =
            Report::ok("ok", "update plan reported; nothing was changed").subject("update");
        report.detail("plan", plan);
        report.detail("plan_digest", Json::text(&digest));
        report.lines(lines);
        return Ok(report);
    }
    let answer = crate::confirm::decide(request.yes, json_output, &lines);
    if let Some(report) =
        crate::lifecycle::unconfirmed("update", answer, &plan, &digest, lines.clone())
    {
        let _ = std::fs::remove_dir_all(&staging);
        return Ok(report);
    }
    if let Some(refusal) = crate::elevation::refusal() {
        return Err(refusal);
    }
    let mut report = apply(root, &plan, &offer, &source, &manifest_sha, &staging)?;
    report.detail("confirmation", Json::text(answer.name()));
    report.detail("plan_digest", Json::text(&digest));
    if answer != crate::confirm::Decision::Confirmed {
        report.lines(lines);
    }
    Ok(report)
}

/// Re-verify the manifest, fetch and verify the archive, run the engine update through the new
/// release's own CLI, health-check, and roll back on failure.
pub fn apply(
    root: &Path,
    plan: &Json,
    offer: &Offer,
    source: &str,
    manifest_sha: &str,
    staging: &Path,
) -> Result<Report, Refusal> {
    let lock = root.join(state::LOCK_FILE);
    let _guard = LockGuard::take(&lock)?;
    let again = staging.join("channel-recheck.json");
    fetch(source, &again)?;
    let observed = digest_of(&state::read_bytes(&again, "channel_unreadable")?);
    if observed != manifest_sha {
        return Err(Refusal::conflict(
            "manifest_changed_after_plan",
            format!(
                "the channel manifest at {source} now digests to {observed}, not the {manifest_sha} \
                 that was approved; nothing was changed. Run `axm update` again to review the new plan"
            ),
        ));
    }
    let archive_name = offer
        .url
        .rsplit('/')
        .next()
        .unwrap_or("release-archive")
        .to_string();
    let archive = staging.join(&archive_name);
    fetch(&offer.url, &archive)?;
    let bytes = state::read_bytes(&archive, "archive_unreadable")?;
    if bytes.len() as i64 != offer.size_bytes || digest_of(&bytes) != offer.sha256 {
        let _ = std::fs::remove_file(&archive);
        return Err(Refusal::validation(
            "artifact_digest_mismatch",
            format!(
                "{archive_name} is {} bytes with sha256 {}, but the approved plan pins {} bytes and {}; \
                 the archive was not extracted and nothing was changed",
                bytes.len(),
                digest_of(&bytes),
                offer.size_bytes,
                offer.sha256
            ),
        ));
    }
    let release = staging.join("release");
    let _ = std::fs::remove_dir_all(&release);
    std::fs::create_dir_all(&release).map_err(|error| {
        Refusal::io("staging_unwritable", &release.display().to_string(), &error)
    })?;
    let tar = system_tool("tar");
    let flags = if archive_name.ends_with(".zip") {
        "-xf"
    } else {
        "-xzf"
    };
    let extracted = Command::new(&tar)
        .arg(flags)
        .arg(&archive)
        .arg("-C")
        .arg(&release)
        .status();
    if !matches!(extracted, Ok(status) if status.success()) {
        return Err(Refusal::validation(
            "archive_unextractable",
            format!(
                "the verified archive {archive_name} could not be extracted; nothing was changed"
            ),
        ));
    }
    let new_cli = release.join(crate::layout::program_file("axiom-cli"));
    if !new_cli.is_file() {
        return Err(Refusal::validation(
            "archive_cli_missing",
            "the release archive has no axiom-cli; nothing was changed",
        ));
    }

    let st = State::new(root);
    let before = state::read_bytes(&st.installed_path(), "installed_unreadable")?;
    let before_record = st.read_installed()?;
    st.load_recorded_manifest(&before_record)?;
    let manifest_before =
        state::read_bytes(&st.recorded_manifest_path(), "recorded_manifest_unreadable")?;
    let mcp_before = std::fs::read(mcp_pointer(root)).ok();
    let child = |args: &[&str]| -> Result<(i32, String), Refusal> {
        let output = Command::new(&new_cli)
            .args(args)
            .env(state::INSTALL_ROOT_ENV, root)
            .env("AXIOM_CLI_COMPOSITE_UPDATE", "1")
            .env_remove("AXIOM_ENGINE_BIN")
            .env_remove(crate::confirm::YES_ENV)
            .output()
            .map_err(|error| {
                Refusal::io("new_cli_unrunnable", &new_cli.display().to_string(), &error)
            })?;
        Ok((
            output.status.code().unwrap_or(1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ))
    };
    let (code, dry) = child(&["install", "--dry-run", "--json"])?;
    let inner = json::parse(dry.trim()).ok().and_then(|value| {
        value
            .get("details")?
            .get("plan_digest")?
            .as_text()
            .map(str::to_string)
    });
    let Some(inner) = inner.filter(|_| code == 0) else {
        return Err(Refusal::not_ready(
            "new_release_plan_failed",
            format!(
                "the new release could not plan its install (exit {code}); nothing was changed"
            ),
        ));
    };
    let (code, applied) = child(&["install", "--apply", "--approve-digest", &inner, "--json"])?;
    let applied_json = json::parse(applied.trim()).unwrap_or(Json::Null);
    let transaction = applied_json
        .get("details")
        .and_then(|details| details.get("engine_transaction"))
        .and_then(Json::as_text)
        .map(str::to_string);
    let _ = std::fs::write(staging.join("child-apply.json"), applied.as_bytes());
    if code != 0 {
        // The engine refused before activation or rolled its own transaction back; restore the
        // record in case the layout step started.
        restore(root, &before, &before_record, None, Some(&manifest_before))?;
        restore_mcp_pointer(root, mcp_before.as_deref())?;
        return Err(Refusal::not_ready(
            "engine_update_failed",
            format!(
                "the engine update to {} failed (exit {code}); the previous version stays active",
                offer.version
            ),
        ));
    }

    if transaction.is_none() {
        // Without the engine transaction id the engine half could not be rolled back, so an
        // update that cannot name it is refused rather than half-recorded.
        let rolled = restore(root, &before, &before_record, None, Some(&manifest_before))
            .and_then(|()| restore_mcp_pointer(root, mcp_before.as_deref()));
        return Err(Refusal::new(
            Class::IoInternal,
            "engine_transaction_unknown",
            format!(
                "the engine reported no update transaction id, so the update cannot be made                  reversible; the record was restored ({}) and the child report is kept in {}",
                if rolled.is_ok() { "ok" } else { "restore failed" },
                staging.join("child-apply.json").display()
            ),
        ));
    }
    let healthy = health(root);
    if let Err(reason) = healthy {
        let rolled = restore(
            root,
            &before,
            &before_record,
            transaction.as_deref(),
            Some(&manifest_before),
        )
        .and_then(|()| restore_mcp_pointer(root, mcp_before.as_deref()));
        let mut report = Report::new(
            Class::IoInternal.exit_code(),
            Class::IoInternal.status(),
            format!(
                "the {} update failed its health check ({reason}) and was rolled back to {}",
                offer.version,
                plan.get("from_version")
                    .and_then(Json::as_text)
                    .unwrap_or("?")
            ),
        )
        .subject("update");
        report.detail("reason_code", Json::text("health_check_failed"));
        report.detail(
            "engine_transaction",
            transaction.as_deref().map_or(Json::Null, Json::text),
        );
        report.detail("rolled_back", Json::bool(rolled.is_ok()));
        if let Err(error) = rolled {
            report.detail("rollback_error", Json::text(&error.message));
        }
        return Ok(report);
    }

    let mut record = st.read_installed()?;
    record.extra.insert(
        "last_update".to_string(),
        Json::from_pairs(vec![
            (
                "from_version",
                plan.get("from_version").cloned().unwrap_or(Json::Null),
            ),
            ("to_version", Json::text(&offer.version)),
            (
                "engine_transaction",
                transaction.as_deref().map_or(Json::Null, Json::text),
            ),
            ("previous_installed_sha256", Json::text(&digest_of(&before))),
            ("manifest_sha256", Json::text(manifest_sha)),
        ]),
    );
    record
        .extra
        .insert("channel_source".to_string(), Json::text(source));
    let previous_manifest = root.join("recorded-manifest.previous.json");
    state::write_atomic(
        &previous_manifest.with_extension("json.tmp"),
        &previous_manifest,
        &manifest_before,
    )?;
    st.write_installed(&record)?;
    let previous_record = root.join("installed.previous.json");
    state::write_atomic(
        &previous_record.with_extension("json.tmp"),
        &previous_record,
        &before,
    )?;
    let previous_mcp = root.join("mcp-runtime.previous.json");
    match &mcp_before {
        Some(bytes) => state::write_atomic(
            &previous_mcp.with_extension("json.tmp"),
            &previous_mcp,
            bytes,
        )?,
        None => {
            let _ = std::fs::remove_file(&previous_mcp);
        }
    }
    let _ = std::fs::remove_dir_all(staging);
    let mut report = Report::ok(
        "ok",
        format!(
            "updated {} -> {}; the previous version is kept for `axm update rollback --transaction previous`",
            plan.get("from_version").and_then(Json::as_text).unwrap_or("?"),
            offer.version
        ),
    )
    .subject("update");
    report.detail("to_version", Json::text(&offer.version));
    report.detail(
        "engine_transaction",
        transaction.as_deref().map_or(Json::Null, Json::text),
    );
    report.detail(
        "needs_restart",
        Json::array(vec![Json::text("axiom-graphd"), Json::text("axiom-mcp")]),
    );
    Ok(report)
}

/// The new bin answers `version` and the engine answers `version`.
fn health(root: &Path) -> Result<(), String> {
    if std::env::var_os(FAIL_HEALTH_TEST_ENV).is_some() {
        return Err("forced by the test seam".to_string());
    }
    let bin = crate::layout::bin_dir(root);
    for (program, args) in [
        ("axm", vec!["version", "--json"]),
        ("graphd-engine/axiom", vec!["version"]),
    ] {
        let path = bin.join(crate::layout::program_file(program));
        let status = Command::new(&path)
            .args(&args)
            .env(state::INSTALL_ROOT_ENV, root)
            .env("AXIOM_HOME", root)
            .output()
            .map_err(|error| format!("{} did not start: {error}", path.display()))?;
        if !status.status.success() {
            return Err(format!(
                "{} {:?} exited {:?}",
                path.display(),
                args,
                status.status.code()
            ));
        }
    }
    Ok(())
}

/// Roll back: the engine transaction (when it activated), the bin files from the previous
/// generation, and installed.json byte for byte.
fn restore(
    root: &Path,
    before: &[u8],
    before_record: &state::Installed,
    transaction: Option<&str>,
    manifest_snapshot: Option<&[u8]>,
) -> Result<(), Refusal> {
    if manifest_snapshot
        .is_some_and(|bytes| digest_of(bytes) != before_record.channel_manifest_sha256)
    {
        return Err(Refusal::conflict("previous_manifest_changed", "retained channel manifest does not match the previous installation; nothing was restored"));
    }
    let st = State::new(root);
    let generation = super::generation::Generation::read(&st, &before_record.current_generation)?;
    let directory = st.generation_dir(&before_record.current_generation);
    generation.verify(&directory)?;
    let active = st.read_installed().ok().and_then(|record| {
        super::generation::Generation::read(&st, &record.current_generation).ok()
    });
    let mut added = Vec::new();
    if let Some(active) = &active {
        for entry in &active.entries {
            let Some(name) = entry.component.strip_prefix("bin/") else {
                continue;
            };
            if generation
                .entries
                .iter()
                .any(|old| old.component == entry.component)
            {
                continue;
            }
            if !crate::update::rules::is_relative_path(name) {
                return Err(Refusal::conflict(
                    "rollback_bin_invalid",
                    "recorded executable path is invalid; nothing was restored",
                ));
            }
            let path = crate::layout::bin_dir(root).join(crate::layout::program_file(name));
            let mut ancestor = path.as_path();
            while ancestor != root {
                if ancestor.is_symlink() {
                    return Err(Refusal::conflict(
                        "rollback_bin_linked",
                        "added executable path is linked; nothing was restored",
                    ));
                }
                ancestor = ancestor.parent().ok_or_else(|| {
                    Refusal::conflict(
                        "rollback_bin_invalid",
                        "added executable path escapes its root",
                    )
                })?;
            }
            if !path.exists() {
                continue;
            }
            if !path.is_file()
                || crate::update::sha256::file_hex(&path).ok().as_deref()
                    != Some(entry.artifact_sha256.as_str())
            {
                return Err(Refusal::conflict(
                    if name == "graphd-engine/axiom" {
                        "rollback_engine_changed"
                    } else {
                        "rollback_bin_changed"
                    },
                    "added executable changed; nothing was restored",
                ));
            }
            added.push(path);
        }
    }
    if let Some(id) = transaction {
        let bin = crate::layout::bin_dir(root);
        let private = ["graphd-engine/axiom", ".internal/axiom", "axiom"]
            .iter()
            .map(|name| bin.join(crate::layout::program_file(name)))
            .find(|path| path.is_file())
            .unwrap_or_else(|| bin.join(crate::layout::program_file("graphd-engine/axiom")));
        let engine = if private.is_file() {
            private
        } else {
            bin.join(crate::layout::program_file("axiom"))
        };
        let output = Command::new(&engine)
            .args(["update", "rollback", "--transaction", id])
            .env("AXIOM_HOME", root)
            .output()
            .map_err(|error| {
                Refusal::io(
                    "engine_rollback_unrunnable",
                    &engine.display().to_string(),
                    &error,
                )
            })?;
        if !output.status.success() {
            return Err(Refusal::new(
                Class::IoInternal,
                "engine_rollback_failed",
                format!(
                    "engine rollback of {id} exited {:?}: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
    }
    let mut sources = Vec::new();
    for entry in &generation.entries {
        let Some(program) = entry.component.strip_prefix("bin/") else {
            continue;
        };
        let name: &'static str = match program {
            "axiom-cli" => "axiom-cli",
            "axm" => crate::layout::SHORT_COMMAND,
            "axsk" => "axsk",
            "axmp" => "axmp",
            "axge" => "axge",
            "graphd-engine/axiom" => "graphd-engine/axiom",
            ".internal/retired-private-engine" => ".internal/retired-private-engine",
            ".internal/axiom" => ".internal/axiom",
            ".internal/retired-axiom-cli" => ".internal/retired-axiom-cli",
            ".internal/retired-axiom" => ".internal/retired-axiom",
            ".internal/retired-axiom-graphd" => ".internal/retired-axiom-graphd",
            ".internal/retired-axiom-mcp" => ".internal/retired-axiom-mcp",
            ".internal/retired-axiom-mcp-entrypoints" => ".internal/retired-axiom-mcp-entrypoints",
            "axiom" => "axiom",
            "axiom-graphd" => "axiom-graphd",
            _ => continue,
        };
        if let Some(path) = generation.payload_path(&directory, &entry.component) {
            sources.push(crate::layout::BinSource {
                name,
                source: path,
                sha256: entry.artifact_sha256.clone(),
            });
        }
    }
    crate::layout::place_bin(root, &sources)?;
    for path in added {
        if let Err(error) = std::fs::remove_file(&path) {
            // A running Windows entrypoint can be renamed, but not deleted until it exits.
            let running = std::env::current_exe()
                .ok()
                .and_then(|p| p.canonicalize().ok());
            if running.is_some() && path.canonicalize().ok() == running {
                let staging = root.join("staging");
                if staging.is_symlink() {
                    return Err(Refusal::conflict(
                        "rollback_retirement_linked",
                        "rollback retirement directory is linked",
                    ));
                }
                std::fs::create_dir_all(&staging)
                    .map_err(|e| Refusal::io("rollback_retirement_failed", "staging", &e))?;
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let retired = staging.join(format!(
                    "rollback-retired-{}-{stamp}.exe",
                    std::process::id()
                ));
                std::fs::rename(&path, &retired).map_err(|e| {
                    Refusal::io(
                        "rollback_retirement_failed",
                        &path.display().to_string(),
                        &e,
                    )
                })?;
            } else {
                return Err(Refusal::io(
                    "rollback_bin_remove_failed",
                    &path.display().to_string(),
                    &error,
                ));
            }
        }
    }
    if let Some(bytes) = manifest_snapshot {
        st.record_manifest_bytes(bytes)?;
    }
    let path = st.installed_path();
    state::write_atomic(&path.with_extension("json.tmp"), &path, before)
}

/// Roll back the last one-command update (`update rollback --transaction previous`).
pub fn rollback_last(root: &Path) -> Result<Report, Refusal> {
    let st = State::new(root);
    let record = st.read_installed()?;
    let last = record.extra.get("last_update").cloned().ok_or_else(|| {
        Refusal::new(
            Class::Conflict,
            "no_previous_generation",
            "no one-command update is recorded to roll back",
        )
    })?;
    let previous_path = root.join("installed.previous.json");
    let before = state::read_bytes(&previous_path, "previous_record_unreadable")?;
    if Some(digest_of(&before).as_str())
        != last
            .get("previous_installed_sha256")
            .and_then(Json::as_text)
    {
        return Err(Refusal::conflict(
            "previous_record_changed",
            "installed.previous.json no longer matches the record the update kept; nothing was changed",
        ));
    }
    let before_record = state::Installed::parse(&String::from_utf8_lossy(&before))?;
    let previous_manifest = root.join("recorded-manifest.previous.json");
    let manifest_snapshot = if previous_manifest.exists() {
        if previous_manifest.is_symlink() {
            return Err(Refusal::conflict(
                "previous_manifest_linked",
                "retained channel manifest is linked; nothing was restored",
            ));
        }
        state::read_bytes(&previous_manifest, "previous_manifest_unreadable")?
    } else {
        // Older updates may have kept the same manifest. Accept it only if it verifies
        // against the previous record; never restore a record with an incompatible manifest.
        st.load_recorded_manifest(&before_record)?;
        state::read_bytes(&st.recorded_manifest_path(), "previous_manifest_missing")?
    };
    let _guard = LockGuard::take(&root.join(state::LOCK_FILE))?;
    restore(
        root,
        &before,
        &before_record,
        last.get("engine_transaction").and_then(Json::as_text),
        Some(&manifest_snapshot),
    )?;
    let previous_mcp = root.join("mcp-runtime.previous.json");
    restore_mcp_pointer(root, std::fs::read(&previous_mcp).ok().as_deref())?;
    let _ = std::fs::remove_file(&previous_mcp);
    let _ = std::fs::remove_file(&previous_path);
    let _ = std::fs::remove_file(&previous_manifest);
    Ok(Report::ok(
        "ok",
        format!(
            "rolled back {} -> {}",
            last.get("to_version")
                .and_then(Json::as_text)
                .unwrap_or("?"),
            last.get("from_version")
                .and_then(Json::as_text)
                .unwrap_or("?")
        ),
    )
    .subject("update"))
}

struct LockGuard(PathBuf);

impl LockGuard {
    fn take(path: &Path) -> Result<LockGuard, Refusal> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| {
                Refusal::new(
                    Class::LockUnavailable,
                    "lock_unavailable",
                    format!(
                        "another update holds {}; nothing was changed",
                        path.display()
                    ),
                )
            })?;
        Ok(LockGuard(path.to_path_buf()))
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_restores_all_canonical_commands_and_the_private_engine() {
        let root =
            std::env::temp_dir().join(format!("axiom-command-rollback-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let artifact = root.join("verified-source");
        std::fs::write(&artifact, b"previous verified bytes").unwrap();
        let digest = digest_of(b"previous verified bytes");
        let names = ["axm", "axsk", "axge", "axmp", "graphd-engine/axiom"];
        let sources: Vec<_> = names
            .iter()
            .map(|name| crate::layout::BinSource {
                name,
                source: artifact.clone(),
                sha256: digest.clone(),
            })
            .collect();
        let bin = crate::layout::place_bin(&root, &sources).unwrap();
        let record = crate::layout::record(
            &root,
            &crate::layout::Record {
                channel: "stable",
                host: "fixture",
                manifest_bytes: b"{}",
                manifest_sha256: &digest_of(b"{}"),
                plan_digest: &"b".repeat(64),
                bin,
                components: names
                    .iter()
                    .map(|name| {
                        (
                            format!("bin/{name}"),
                            "0.1.6".into(),
                            String::new(),
                            digest.clone(),
                            crate::layout::bin_dir(&root).join(crate::layout::program_file(name)),
                            false,
                        )
                    })
                    .collect(),
            },
        )
        .unwrap();
        let before = std::fs::read(State::new(&root).installed_path()).unwrap();
        for name in names {
            std::fs::write(
                crate::layout::bin_dir(&root).join(crate::layout::program_file(name)),
                b"new candidate bytes",
            )
            .unwrap();
        }
        restore(&root, &before, &record, None, None).unwrap();
        for name in names {
            assert_eq!(
                std::fs::read(
                    crate::layout::bin_dir(&root).join(crate::layout::program_file(name))
                )
                .unwrap(),
                b"previous verified bytes"
            );
        }
        assert_eq!(
            std::fs::read(State::new(&root).installed_path()).unwrap(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_to_older_private_folder_removes_only_the_verified_new_engine() {
        fn seed(root: &Path, name: &'static str, bytes: &[u8], plan: &str) -> state::Installed {
            let source = root.join(format!("source-{}", bytes.len()));
            std::fs::write(&source, bytes).unwrap();
            let sha = digest_of(bytes);
            let mut sources = vec![crate::layout::BinSource {
                name,
                source: source.clone(),
                sha256: sha.clone(),
            }];
            if name == "graphd-engine/axiom" {
                sources.push(crate::layout::BinSource {
                    name: "axge",
                    source,
                    sha256: sha.clone(),
                });
            }
            let bin = crate::layout::place_bin(root, &sources).unwrap();
            crate::layout::record(
                root,
                &crate::layout::Record {
                    channel: "stable",
                    host: "fixture",
                    manifest_bytes: b"{}",
                    manifest_sha256: &digest_of(b"{}"),
                    plan_digest: plan,
                    bin,
                    components: sources
                        .iter()
                        .map(|item| {
                            (
                                format!("bin/{}", item.name),
                                "0.1.6".into(),
                                String::new(),
                                sha.clone(),
                                crate::layout::bin_dir(root)
                                    .join(crate::layout::program_file(item.name)),
                                false,
                            )
                        })
                        .collect(),
                },
            )
            .unwrap()
        }
        let root =
            std::env::temp_dir().join(format!("axiom-folder-rollback-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let old = seed(
            &root,
            ".internal/axiom",
            b"old verified engine",
            &"d".repeat(64),
        );
        let before = std::fs::read(State::new(&root).installed_path()).unwrap();
        seed(
            &root,
            "graphd-engine/axiom",
            b"new verified engine bytes",
            &"e".repeat(64),
        );
        let new_path =
            crate::layout::bin_dir(&root).join(crate::layout::program_file("graphd-engine/axiom"));
        let old_path =
            crate::layout::bin_dir(&root).join(crate::layout::program_file(".internal/axiom"));
        std::fs::remove_file(&old_path).unwrap();
        let state = State::new(&root);
        state.record_manifest_bytes(b"{\"new\":true}").unwrap();
        assert_eq!(
            restore(&root, &before, &old, None, Some(b"changed manifest"))
                .unwrap_err()
                .reason,
            "previous_manifest_changed"
        );
        assert!(!old_path.exists());

        std::fs::write(&new_path, b"modified engine").unwrap();
        assert_eq!(
            restore(&root, &before, &old, None, Some(b"{}"))
                .unwrap_err()
                .reason,
            "rollback_engine_changed"
        );
        assert!(!old_path.exists());
        assert_eq!(std::fs::read(&new_path).unwrap(), b"modified engine");
        std::fs::write(&new_path, b"new verified engine bytes").unwrap();
        let added_command = crate::layout::bin_dir(&root).join(crate::layout::program_file("axge"));
        std::fs::write(&added_command, b"human edit").unwrap();
        assert_eq!(
            restore(&root, &before, &old, None, Some(b"{}"))
                .unwrap_err()
                .reason,
            "rollback_bin_changed"
        );
        assert!(!old_path.exists());
        assert_eq!(std::fs::read(&added_command).unwrap(), b"human edit");
        std::fs::write(&added_command, b"new verified engine bytes").unwrap();
        restore(&root, &before, &old, None, Some(b"{}")).unwrap();
        assert!(!new_path.exists());
        assert!(!crate::layout::bin_dir(&root)
            .join(crate::layout::program_file("axge"))
            .exists());
        assert_eq!(
            std::fs::read(state.recorded_manifest_path()).unwrap(),
            b"{}"
        );
        assert_eq!(std::fs::read(old_path).unwrap(), b"old verified engine");
        assert_eq!(
            std::fs::read(State::new(&root).installed_path()).unwrap(),
            before
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn manifest(version: &str, tag: &str, url: &str) -> Vec<u8> {
        format!(
            r#"{{"channel":"stable","release":{{"version":"{version}","tag":"{tag}","archives":[{{"platform":"windows-x64","url":"{url}","sha256":"{}","size_bytes":10}}]}}}}"#,
            "a".repeat(64)
        )
        .into_bytes()
    }

    #[test]
    fn a_pinned_offer_is_accepted() {
        let url = "https://github.com/orchex006/axiom-cli/releases/download/v0.1.3/axiom-0.1.3-windows-x64.zip";
        let offer = offer(&manifest("0.1.3", "v0.1.3", url), "windows-x64").unwrap();
        assert_eq!(offer.version, "0.1.3");
        assert_eq!(offer.url, url);
    }

    #[test]
    fn moving_references_are_refused() {
        let good = "https://github.com/orchex006/axiom-cli/releases/download/v0.1.3/a.zip";
        for (version, tag, url, reason) in [
            ("latest", "vlatest", good, "forbidden_pin:latest"),
            ("*", "v*", good, "forbidden_pin:*"),
            ("main", "vmain", good, "forbidden_pin:main"),
            (
                "0.1.3",
                "v0.1.3",
                "https://github.com/orchex006/axiom-cli/releases/latest/download/a.zip",
                "channel_archive_url_not_pinned",
            ),
            (
                "0.1.3",
                "v0.1.3",
                "https://example.test/main/a.zip",
                "channel_archive_url_not_pinned",
            ),
            ("0.1.3", "v0.1.4", good, "channel_release_version_invalid"),
        ] {
            let error = offer(&manifest(version, tag, url), "windows-x64").unwrap_err();
            assert_eq!(error.reason, reason, "{version} {tag} {url}");
        }
        let error = offer(&manifest("0.1.3", "v0.1.3", good), "linux-x64").unwrap_err();
        assert_eq!(error.reason, "channel_platform_missing");
        let error = offer(br#"{"channel":"stable"}"#, "windows-x64").unwrap_err();
        assert_eq!(error.reason, "channel_release_missing");
    }

    #[test]
    fn semver_orders_prereleases_before_releases() {
        use std::cmp::Ordering::*;
        assert_eq!(semver_cmp("0.1.3", "0.1.2"), Greater);
        assert_eq!(semver_cmp("0.1.10", "0.1.9"), Greater);
        assert_eq!(semver_cmp("0.1.3-rc.1", "0.1.3"), Less);
        assert_eq!(semver_cmp("0.1.2", "0.1.2"), Equal);
    }

    #[test]
    fn a_manifest_changed_after_planning_is_refused_before_any_download() {
        let root = std::env::temp_dir().join(format!("axiom-oneshot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("channel.json");
        let url = "https://github.com/orchex006/axiom-cli/releases/download/v0.1.3/axiom-0.1.3-windows-x64.zip";
        std::fs::write(&source, manifest("0.1.3", "v0.1.3", url)).unwrap();
        let planned = digest_of(&std::fs::read(&source).unwrap());
        let offer = offer(&std::fs::read(&source).unwrap(), "windows-x64").unwrap();
        std::fs::write(
            &source,
            manifest("0.1.4", "v0.1.4", &url.replace("0.1.3", "0.1.4")),
        )
        .unwrap();
        let error = match apply(
            &root,
            &Json::Null,
            &offer,
            source.to_str().unwrap(),
            &planned,
            &root.join("staging"),
        ) {
            Ok(_) => panic!("a changed manifest must be refused"),
            Err(error) => error,
        };
        assert_eq!(error.reason, "manifest_changed_after_plan");
        assert!(!root
            .join("staging")
            .join("axiom-0.1.3-windows-x64.zip")
            .exists());
        assert!(
            !root.join(state::LOCK_FILE).exists(),
            "the lock is released"
        );
    }
}
