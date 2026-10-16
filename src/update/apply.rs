//! The four `update` verbs and the transaction behind `apply`.
//!
//! `update check` and `update plan` are read-only. `update apply` is the one mutating verb, and
//! the order it works in is the point of the whole task:
//!
//! ```text
//! recover an interrupted transaction
//! -> take the coordinator lock
//! -> read the recorded channel manifest of the installed release
//! -> refuse unless the plan agrees with it, component by component
//! -> stage, then verify length and sha256 of every artifact before it is used
//! -> write the journal, then rename the staging tree into generations/<id>
//! -> swap installed.json atomically, keeping the previous generation
//! -> health check the new generation
//! -> on failure: restore the previous generation and report a rollback
//! -> on success: finalise the journal, prune older generations, release the lock
//! ```
//!
//! Nothing in this module pushes to a remote, opens a socket, resolves a branch tip, a tag
//! alias or a network `latest`, or writes outside the install root. User data, the graph output
//! root and portable workspace state are never part of the transaction, which is why a rollback
//! is a pointer move between generations and cannot damage them.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::error::{Class, Refusal};
use super::generation::{self, Entry, Generation, Probe, GENERATION_SCHEMA_VERSION};
use super::json::{canonical_text, Json};
use super::report::Report;
use super::sha256;
use super::state::{self, Installed, State};
use super::time::Stamp;
use super::{channel, fetch, health, journal, plan};

/// The `update` subcommand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Subcommand {
    /// `check` - report installed and available versions without changing anything.
    Check,
    /// `plan` - build the plan document and its digest; apply nothing.
    Plan,
    /// `apply` - run one transaction against an approved plan.
    Apply,
    /// `rollback` - restore a retained generation.
    Rollback,
}

impl Subcommand {
    /// Canonical argv token.
    pub fn name(self) -> &'static str {
        match self {
            Subcommand::Check => "check",
            Subcommand::Plan => "plan",
            Subcommand::Apply => "apply",
            Subcommand::Rollback => "rollback",
        }
    }

    /// Resolve a subcommand from its argv token.
    pub fn from_name(name: &str) -> Option<Subcommand> {
        match name {
            "check" => Some(Subcommand::Check),
            "plan" => Some(Subcommand::Plan),
            "apply" => Some(Subcommand::Apply),
            "rollback" => Some(Subcommand::Rollback),
            _ => None,
        }
    }
}

/// A parsed `update` invocation, produced by the argv layer.
#[derive(Clone, Debug)]
pub struct Request {
    /// Which subcommand.
    pub subcommand: Subcommand,
    /// `--all` on `check`.
    pub all: bool,
    /// `--to` on `plan`.
    pub to: Option<String>,
    /// `--out` on `plan`; `-` means stdout.
    pub out: Option<String>,
    /// `--plan` on `apply`.
    pub plan_path: Option<String>,
    /// `--approve-digest`, the single approval digest for the transaction.
    pub approve_digest: Option<String>,
    /// `--transaction` on `rollback`, or the generation selector `previous`.
    pub transaction: Option<String>,
}

/// The coordinator lock, released when it goes out of scope.
pub(crate) struct Lock {
    path: PathBuf,
}

impl Lock {
    /// Create the lock, or refuse because another transaction holds it.
    pub(crate) fn acquire(state: &State, transaction: &str) -> Result<Lock, Refusal> {
        std::fs::create_dir_all(state.root()).map_err(|error| {
            Refusal::io(
                "install_root_unwritable",
                &state.root().display().to_string(),
                &error,
            )
        })?;
        let path = state.lock_path();
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut handle) => {
                let body = format!("transaction={transaction}\npid={}\n", std::process::id());
                let _ = handle.write_all(body.as_bytes());
                let _ = handle.sync_all();
                Ok(Lock { path })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let held = std::fs::read_to_string(&path).unwrap_or_default();
                Err(Refusal::new(
                    Class::LockUnavailable,
                    "lock_unavailable",
                    format!(
                        "another update transaction holds the coordinator lock {} ({}); recovery \
                         releases a lock left behind by an interrupted transaction",
                        path.display(),
                        held.trim().replace('\n', " ")
                    ),
                ))
            }
            Err(error) => Err(Refusal::io(
                "lock_unavailable",
                &path.display().to_string(),
                &error,
            )),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Run one `update` invocation and return its process exit code.
pub fn run_update(request: Request, json: bool, verbose: bool) -> i32 {
    let Some(root) = state::default_root() else {
        let reason = "this host offers no per-user install root, so no recorded channel manifest \
                      can be read; axiom-cli reads an update channel rooted at one installation, \
                      not at an ambient set of paths";
        let mut report = Report::not_ready(reason).retryable();
        report.detail("reason", Json::text(reason));
        report.detail("reason_code", Json::text("no_install_root"));
        return report.emit(json, verbose);
    };
    let state = State::new(root);
    if state.root().join("installs/ecosystem/current").exists() {
        if let Some(kit) = std::env::var_os("AXIOM_CLI_COMPOSITE_KIT") {
            return super::composite::run(state.root(), Path::new(&kit), &request, json);
        }
    }
    match execute(&state, &request) {
        Ok(report) => report.emit(json, verbose),
        Err(refusal) => refusal_report(&refusal).emit(json, verbose),
    }
}

fn refusal_report(refusal: &Refusal) -> Report {
    // The class chooses the constructor, so the exit code and the status token cannot drift
    // apart from the vocabulary the rest of the CLI speaks.
    let mut report = match refusal.class {
        Class::NotReady => Report::not_ready(refusal.message.clone()),
        Class::Validation => Report::refused(refusal.message.clone()),
        other => Report::new(other.exit_code(), other.status(), refusal.message.clone()),
    };
    // `reason` carries the full stated reason, which is what the frozen argv contract asserts:
    // `tests/argv_surface.rs` requires the NotReady reason to be longer than 40 characters. The
    // short machine token stays available under `reason_code`.
    report.detail("reason", Json::text(&refusal.message));
    report.detail("reason_code", Json::text(&refusal.reason));
    if refusal.class.retryable() {
        report = report.retryable();
    }
    report
}

fn execute(state: &State, request: &Request) -> Result<Report, Refusal> {
    // K-003: the J-007 generation store and the installed engine ecosystem do not yet share
    // an activation authority. Moving only installed.json would expose a mixed release.
    // Refuse every update verb while the engine pointer exists, including read-only plans that
    // would otherwise offer an approval for an unsafe apply.
    let engine_pointer = state.root().join("installs/ecosystem/current");
    if engine_pointer.exists()
        && request.subcommand == Subcommand::Rollback
        && request.transaction.as_deref() == Some("previous")
        && state
            .read_installed()
            .map(|record| record.extra.contains_key("last_update"))
            .unwrap_or(false)
    {
        // L-005: the one-command update recorded its engine transaction and the previous
        // record, so `rollback --transaction previous` can restore the composite exactly.
        return super::oneshot::rollback_last(state.root());
    }
    if engine_pointer.exists() {
        return Err(Refusal::not_ready(
            "composite_activation_not_ready",
            format!(
                "the installed engine ecosystem is active at {}; distribution update needs \
                 one approved composite activation transaction for CLI, core, MCP and skills",
                engine_pointer.display()
            ),
        ));
    }
    match request.subcommand {
        Subcommand::Check => check(state, request),
        Subcommand::Plan => plan_verb(state, request),
        Subcommand::Apply => apply(state, request),
        Subcommand::Rollback => rollback(state, request),
    }
}

/// The canonical host id of this process, or a refusal when the host is not a declared target.
fn host_id() -> Result<String, Refusal> {
    let host = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x64"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x64"
    } else {
        return Err(Refusal::validation(
            "unsupported_host",
            format!(
                "this process runs on {}/{} which is not one of {:?}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                channel::HOSTS
            ),
        ));
    };
    Ok(host.to_string())
}

/// A timestamp-ordered, lowercase id built from the wall clock and the process id.
fn make_id(prefix: &str) -> String {
    let compact = Stamp::now().format().replace(['-', ':'], "").to_lowercase();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    let mix = (u64::from(nanos)) ^ (u64::from(std::process::id()) << 16);
    format!("{prefix}-{compact}-{mix:08x}")
}

/// Everything a mutating verb needs from the install root.
struct Context {
    installed: Installed,
    manifest: channel::Manifest,
    manifest_sha256: String,
    manifest_path: PathBuf,
    active: Generation,
    root_text: String,
}

fn load_context(state: &State) -> Result<Context, Refusal> {
    let installed = state.read_installed()?;
    let loaded = state.load_recorded_manifest(&installed)?;
    let active = Generation::read(state, &installed.current_generation)?;
    Ok(Context {
        manifest_sha256: loaded.sha256,
        manifest_path: loaded.path,
        manifest: loaded.manifest,
        installed,
        active,
        root_text: fetch::install_root_text(state),
    })
}

fn generations_on_disk(state: &State) -> Vec<String> {
    let directory = state.generations_dir();
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&directory) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                found.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    found.sort();
    found
}
fn optional_text(value: Option<String>) -> Json {
    match value {
        Some(text) => Json::text(&text),
        None => Json::null(),
    }
}

/// The recorded trust window of a channel manifest.
///
/// An expired trust window is neither a malformed request nor a missing file: the manifest is
/// well-formed and present, and the host may simply no longer accept what it declares. That is
/// the refusal class that means "this installation cannot be updated from this manifest".
fn check_trust_window(manifest: &channel::Manifest) -> Result<(), Refusal> {
    let now = Stamp::now();
    if manifest.trust.metadata_expiry.seconds() <= now.seconds() {
        return Err(Refusal::new(
            Class::Incompatible,
            "channel_metadata_expired",
            format!(
                "the recorded channel manifest declares trust metadata that expires at {} and it \
                 is {}: this CLI accepts the recorded manifest only inside its declared trust \
                 window, so no version was resolved from it and nothing was applied",
                manifest.trust.metadata_expiry.format(),
                now.format()
            ),
        ));
    }
    Ok(())
}

/// The manifest-level facts every `update` report carries, so one result names the exact
/// metadata revision it resolved from instead of only the channel name.
fn manifest_facts(report: &mut Report, manifest: &channel::Manifest) {
    report.detail("manifest_version", Json::int(manifest.manifest_version));
    report.detail(
        "manifest_updated_at",
        Json::text(&manifest.updated_at.format()),
    );
    report.detail("note", Json::text(&manifest.note));
    report.detail("published", Json::bool(manifest.published));
    report.detail(
        "trust_metadata_version",
        Json::int(manifest.trust.metadata_version),
    );
    report.detail(
        "trust_metadata_expiry",
        Json::text(&manifest.trust.metadata_expiry.format()),
    );
}

/// Whether every artifact this manifest publishes for `host` is present locally and digests to
/// the recorded value.
///
/// `update check` is read-only, so it verifies what is already in the local artifact cache
/// instead of acquiring anything. An artifact that is not local is reported `unreachable`, and
/// one that is local but does not match is reported `unverified` with the observed reason: it is
/// never reported as verified, and nothing is applied for it.
fn artifact_verification(component: &channel::Component, host: &str) -> Json {
    let rows = component
        .artifacts
        .iter()
        .filter(|artifact| artifact.platform == host)
        .map(|artifact| {
            let mut row: Vec<(&str, Json)> = vec![
                ("class", Json::text(&artifact.class)),
                ("url", Json::text(&artifact.url)),
                ("recorded_sha256", Json::text(&artifact.sha256)),
                ("recorded_size_bytes", Json::int(artifact.size_bytes)),
            ];
            match fetch::resolve(&artifact.url) {
                Err(refusal) => {
                    row.push(("state", Json::text("unreachable")));
                    row.push(("reason_code", Json::text(&refusal.reason)));
                }
                Ok(origin) => {
                    let path = match &origin {
                        fetch::Origin::Cache(path) | fetch::Origin::File(path) => path.clone(),
                    };
                    match fetch::verify_file(&path, &artifact.sha256, artifact.size_bytes) {
                        Ok(()) => {
                            row.push(("state", Json::text("verified")));
                            row.push(("origin", Json::text(&origin.label())));
                        }
                        Err(refusal) => {
                            row.push(("state", Json::text("unverified")));
                            row.push(("reason_code", Json::text(&refusal.reason)));
                            row.push(("reason", Json::text(&refusal.message)));
                        }
                    }
                }
            }
            Json::from_pairs(row)
        })
        .collect();
    Json::array(rows)
}

fn check_row(
    component: &channel::Component,
    host: &str,
    installed_version: Option<String>,
    installed_revision: Option<String>,
    update_available: bool,
    with_artifacts: bool,
) -> Json {
    let mut pairs = vec![
        ("component", Json::text(&component.component)),
        (
            "status",
            Json::text(if component.declared {
                "declared"
            } else {
                "undeclared"
            }),
        ),
        ("declared_source", Json::text(&component.declared_source)),
        ("installed_version", optional_text(installed_version)),
        ("installed_revision", optional_text(installed_revision)),
        (
            "available_version",
            optional_text(component.version.clone()),
        ),
        (
            "available_revision",
            optional_text(component.revision.clone()),
        ),
        ("needs_restart", Json::bool(component.needs_restart)),
        ("update_available", Json::bool(update_available)),
    ];
    if with_artifacts {
        pairs.push((
            "artifacts",
            Json::array(
                component
                    .artifacts
                    .iter()
                    .map(|artifact| {
                        Json::from_pairs(vec![
                            ("platform", Json::text(&artifact.platform)),
                            ("class", Json::text(&artifact.class)),
                            ("url", Json::text(&artifact.url)),
                            ("sha256", Json::text(&artifact.sha256)),
                            ("size_bytes", Json::int(artifact.size_bytes)),
                        ])
                    })
                    .collect(),
            ),
        ));
        pairs.push((
            "artifact_verification",
            artifact_verification(component, host),
        ));
    }
    Json::from_pairs(pairs)
}

/// `update check`: report the resolved set, change nothing.
fn check(state: &State, request: &Request) -> Result<Report, Refusal> {
    let host = host_id()?;
    let root_text = fetch::install_root_text(state);
    if !state.has_installed() {
        let named = std::env::var(state::CHANNEL_MANIFEST_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let Some(path) = named else {
            return Err(Refusal::not_ready(
                "no_installed_release",
                format!(
                    "no installed release is recorded at {} and no candidate channel manifest was \
                     named by {}: the update channel resolves versions only from the manifest the \
                     installed release came from, so there is nothing to check here and no \
                     version was invented. Nothing was downloaded, swapped, rolled back or pushed.",
                    state.installed_path().display(),
                    state::CHANNEL_MANIFEST_ENV
                ),
            ));
        };
        let loaded = State::load_manifest_file(Path::new(&path))?;
        check_trust_window(&loaded.manifest)?;
        let mut report = Report::ok(
            "ok",
            format!(
                "channel {} declares {} component(s) in {}; nothing is installed at {} yet, so no \
                 installed version exists to compare against and none was invented",
                loaded.manifest.channel,
                loaded.manifest.components.len(),
                path,
                root_text
            ),
        );
        report.detail("state", Json::text("not_installed"));
        report.detail("subcommand", Json::text(request.subcommand.name()));
        report.detail("channel", Json::text(&loaded.manifest.channel));
        report.detail("channel_manifest_sha256", Json::text(&loaded.sha256));
        report.detail("channel_manifest_path", Json::text(&path));
        report.detail("published", Json::bool(loaded.manifest.published));
        manifest_facts(&mut report, &loaded.manifest);
        report.detail("host", Json::text(&host));
        report.detail("install_root", Json::text(&root_text));
        report.detail(
            "components",
            Json::array(
                loaded
                    .manifest
                    .components
                    .iter()
                    .map(|component| {
                        check_row(
                            component,
                            &host,
                            None,
                            None,
                            component.declared,
                            request.all,
                        )
                    })
                    .collect(),
            ),
        );
        report.line("state: not_installed");
        for component in &loaded.manifest.components {
            report.line(format!(
                "{}: status={} declared_version={}",
                component.component,
                if component.declared {
                    "declared"
                } else {
                    "undeclared"
                },
                component.version.clone().unwrap_or_else(|| "-".to_string())
            ));
        }
        return Ok(report);
    }

    let context = load_context(state)?;
    check_trust_window(&context.manifest)?;
    let mut installed_versions = context.active.installed_versions();
    let mut rows: Vec<Json> = Vec::new();
    let mut updates_available = false;
    for component in &context.manifest.components {
        let (installed_version, installed_revision) = installed_versions
            .remove(&component.component)
            .unwrap_or((None, None));
        let update_available = match (&installed_version, &component.version, component.declared) {
            (Some(current), Some(available), true) => current != available,
            (None, Some(_), true) => true,
            _ => false,
        };
        if update_available {
            updates_available = true;
        }
        rows.push(check_row(
            component,
            &host,
            installed_version,
            installed_revision,
            update_available,
            request.all,
        ));
    }

    let generations = generations_on_disk(state);
    let mut report = Report::ok(
        "ok",
        format!(
            "channel {} at generation {} resolves {} component(s); updates_available={}",
            context.manifest.channel,
            context.installed.current_generation,
            context.manifest.components.len(),
            updates_available
        ),
    );
    report.detail("state", Json::text("installed"));
    report.detail("subcommand", Json::text(request.subcommand.name()));
    report.detail("channel", Json::text(&context.manifest.channel));
    report.detail(
        "channel_manifest_sha256",
        Json::text(&context.manifest_sha256),
    );
    report.detail(
        "channel_manifest_path",
        Json::text(&context.manifest_path.display().to_string()),
    );
    report.detail("published", Json::bool(context.manifest.published));
    manifest_facts(&mut report, &context.manifest);
    report.detail("host", Json::text(&host));
    report.detail("install_root", Json::text(&context.root_text));
    report.detail(
        "current_generation",
        Json::text(&context.installed.current_generation),
    );
    report.detail(
        "previous_generation",
        optional_text(context.installed.previous_generation.clone()),
    );
    report.detail(
        "generations_on_disk",
        Json::text_array(
            &generations
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
        ),
    );
    report.detail(
        "needs_restart",
        Json::text_array(
            &context
                .active
                .needs_restart()
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
        ),
    );
    report.detail("updates_available", Json::bool(updates_available));
    report.detail("components", Json::array(rows));

    report.line("state: installed");
    report.line(format!("channel: {}", context.manifest.channel));
    report.line(format!(
        "generation: {} (previous: {})",
        context.installed.current_generation,
        context
            .installed
            .previous_generation
            .clone()
            .unwrap_or_else(|| "none".to_string())
    ));
    for component in &context.manifest.components {
        let installed = context
            .active
            .installed_versions()
            .get(&component.component)
            .cloned()
            .unwrap_or((None, None))
            .0
            .unwrap_or_else(|| "not-installed".to_string());
        report.line(format!(
            "{}: installed={} available={} needs_restart={}",
            component.component,
            installed,
            component
                .version
                .clone()
                .unwrap_or_else(|| "undeclared".to_string()),
            component.needs_restart
        ));
    }
    Ok(report)
}

/// `update plan`: build the plan document and its digest; change nothing.
fn plan_verb(state: &State, request: &Request) -> Result<Report, Refusal> {
    let host = host_id()?;
    let context = load_context(state)?;
    check_trust_window(&context.manifest)?;
    let to = request.to.clone().ok_or_else(|| {
        Refusal::validation(
            "missing_target_version",
            "`update plan` requires `--to <version>`",
        )
    })?;
    let installed_versions = context.active.installed_versions();
    let built = plan::build(&plan::Planning {
        manifest: &context.manifest,
        host: &host,
        install_root: &context.root_text,
        channel: &context.manifest.channel,
        target_component: "axiom-cli",
        to_version: &to,
        installed_versions: &installed_versions,
        now: Stamp::now(),
        approved_by: None,
    })?;
    let reasons = plan::structural_reasons(&built.plan);
    if !reasons.is_empty() {
        return Err(Refusal::new(
            Class::IoInternal,
            "plan_contract_selfcheck_failed",
            format!(
                "the plan this CLI built was refused by the canonical contract it mirrors: {}",
                plan::describe(&reasons)
            ),
        ));
    }
    let text = canonical_text(&built.plan);
    let out = request.out.clone().unwrap_or_else(|| "-".to_string());
    let mut written: Option<String> = None;
    if out != "-" {
        let path = PathBuf::from(&out);
        let temporary = path.with_extension("json.tmp");
        state::write_atomic(&temporary, &path, text.as_bytes())?;
        written = Some(out.clone());
    }
    let mut report = Report::ok(
        "ok",
        format!(
            "planned {} component(s) from the recorded channel manifest {} with one approval \
             digest {}",
            built.planned.len(),
            context.manifest_path.display(),
            built.digest
        ),
    );
    report.detail("subcommand", Json::text(request.subcommand.name()));
    manifest_facts(&mut report, &context.manifest);
    report.detail(
        "plan_id",
        Json::text(
            built
                .plan
                .get("plan_id")
                .and_then(Json::as_text)
                .unwrap_or(""),
        ),
    );
    report.detail("plan_digest", Json::text(&built.digest));
    report.detail("approval", Json::text("unapproved"));
    report.detail("channel", Json::text(&context.manifest.channel));
    report.detail(
        "channel_manifest_sha256",
        Json::text(&context.manifest_sha256),
    );
    report.detail("host", Json::text(&host));
    report.detail("install_root", Json::text(&context.root_text));
    report.detail(
        "planned",
        Json::text_array(
            &built
                .planned
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
        ),
    );
    report.detail(
        "not_planned",
        Json::array(
            built
                .excluded
                .iter()
                .map(|excluded| {
                    Json::from_pairs(vec![
                        ("component", Json::text(&excluded.component)),
                        ("reason", Json::text(&excluded.reason)),
                    ])
                })
                .collect(),
        ),
    );
    report.detail(
        "needs_restart",
        Json::text_array(
            &built
                .needs_restart
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
        ),
    );
    report.detail("out", optional_text(written.clone()));
    report.detail("plan", built.plan.clone());
    match &written {
        Some(path) => report.line(format!("plan written to {path}")),
        None => report.line(text),
    };
    Ok(report)
}
/// One component row read back out of a plan document.
struct PlanRow {
    component: String,
    action: String,
    version: String,
    revision: String,
    artifact_sha256: String,
}

fn plan_rows(document: &Json) -> Vec<PlanRow> {
    let mut rows = Vec::new();
    let Some(items) = document.get("components").and_then(Json::as_array) else {
        return rows;
    };
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        let text = |key: &str| {
            object
                .get(key)
                .and_then(Json::as_text)
                .unwrap_or("")
                .to_string()
        };
        rows.push(PlanRow {
            component: text("component"),
            action: text("action"),
            version: text("target_version"),
            revision: text("target_revision"),
            artifact_sha256: text("artifact_sha256"),
        });
    }
    rows
}

fn plan_string(document: &Json, path: &[&str]) -> String {
    let mut current = document;
    for step in path {
        match current.get(step) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_text().unwrap_or("").to_string()
}

/// Whether two install roots name the same directory.
fn same_root(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        let slashed = value.replace('\\', "/");
        let trimmed = slashed.trim_end_matches('/').to_string();
        if cfg!(windows) {
            trimmed.to_lowercase()
        } else {
            trimmed
        }
    };
    normalize(left) == normalize(right)
}

/// Stage and verify every artifact the plan needs.
fn stage_artifacts(
    staging: &Path,
    host: &str,
    context: &Context,
    rows: &[PlanRow],
) -> Result<(Vec<Entry>, Vec<Json>), Refusal> {
    std::fs::create_dir_all(staging.join(state::PAYLOAD_DIR)).map_err(|error| {
        Refusal::io("staging_unwritable", &staging.display().to_string(), &error)
    })?;
    let mut entries: Vec<Entry> = Vec::new();
    let mut acquired: Vec<Json> = Vec::new();
    for row in rows {
        let component = context.manifest.component(&row.component).ok_or_else(|| {
            Refusal::validation(
                format!("plan_names_unknown_component:{}", row.component),
                format!(
                    "the plan names `{}`, which the recorded channel manifest does not declare",
                    row.component
                ),
            )
        })?;
        let artifact = component
            .artifact(host, "per-user-installer")
            .or_else(|| component.artifact(host, "oci-image"))
            .ok_or_else(|| {
                Refusal::validation(
                    format!("plan_artifact_not_in_recorded_manifest:{}", row.component),
                    format!(
                        "the recorded channel manifest publishes no artifact for `{}` on {host}",
                        row.component
                    ),
                )
            })?;
        let payload = if row.action == "noop" {
            None
        } else {
            let name = fetch::url_basename(&artifact.url)
                .unwrap_or_else(|| format!("{}.bin", row.component));
            let relative = format!("{}/{}/{}", state::PAYLOAD_DIR, row.component, name);
            let destination = staging.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            let origin = fetch::acquire(
                &artifact.url,
                &artifact.sha256,
                artifact.size_bytes,
                &destination,
            )?;
            acquired.push(Json::from_pairs(vec![
                ("component", Json::text(&row.component)),
                ("action", Json::text(&row.action)),
                ("origin", Json::text(&origin.label())),
                ("url", Json::text(&artifact.url)),
                ("sha256", Json::text(&artifact.sha256)),
                ("size_bytes", Json::int(artifact.size_bytes)),
            ]));
            Some(relative)
        };
        entries.push(Entry {
            component: row.component.clone(),
            action: row.action.clone(),
            version: row.version.clone(),
            revision: row.revision.clone(),
            artifact_sha256: artifact.sha256.clone(),
            payload,
            needs_restart: component.needs_restart && row.action != "noop",
            probe: component.health.as_ref().map(|probe| Probe {
                program: probe.program.clone(),
                args: probe.args.clone(),
                expect_exit: probe.expect_exit,
            }),
        });
    }
    Ok((entries, acquired))
}

/// The `needs_restart` report: one record per component, never a bare boolean.
fn restart_report(generation: &Generation) -> Json {
    Json::array(
        generation
            .entries
            .iter()
            .map(|entry| {
                Json::from_pairs(vec![
                    ("component", Json::text(&entry.component)),
                    ("action", Json::text(&entry.action)),
                    ("needs_restart", Json::bool(entry.needs_restart)),
                ])
            })
            .collect(),
    )
}

/// The verified on-disk payload path of every component this generation carries bytes for.
///
/// The path is recorded so a caller (or an evidence run) can name the exact bytes that became
/// active without guessing the layout, and every one of them has already been re-verified
/// against the digest the recorded channel manifest declares.
fn payload_report(state: &State, generation: &Generation) -> Json {
    let directory = state.generation_dir(&generation.generation_id);
    let rows = generation
        .entries
        .iter()
        .filter_map(|entry| {
            generation
                .payload_path(&directory, &entry.component)
                .map(|path| {
                    Json::from_pairs(vec![
                        ("component", Json::text(&entry.component)),
                        (
                            "path",
                            Json::text(&path.to_string_lossy().replace('\\', "/")),
                        ),
                        ("sha256", Json::text(&entry.artifact_sha256)),
                    ])
                })
        })
        .collect();
    Json::array(rows)
}

/// Pin `recorded-manifest.json` to the exact bytes this transaction resolves versions from.
///
/// The manifest was read under the coordinator lock, and this re-reads and re-verifies it at the
/// transaction boundary before anything is staged. The write is idempotent in content: it keeps
/// the record equal to the bytes whose digest `installed.json` declares, so a manifest edited
/// after the read cannot be silently inherited by the generation this transaction commits.
fn pin_recorded_manifest(state: &State, context: &Context) -> Result<(), Refusal> {
    let bytes = state::read_bytes(&context.manifest_path, "recorded_manifest_unreadable")?;
    if sha256::digest_hex(&bytes) != context.manifest_sha256 {
        return Err(Refusal::validation(
            "recorded_manifest_digest_mismatch",
            format!(
                "the recorded channel manifest {} changed after it was read: the bytes on disk no \
                 longer digest to {}, so nothing was applied",
                context.manifest_path.display(),
                context.manifest_sha256
            ),
        ));
    }
    state.record_manifest_bytes(&bytes)
}

/// `update apply`: one transaction, one approval digest, self-update included.
fn apply(state: &State, request: &Request) -> Result<Report, Refusal> {
    let host = host_id()?;
    let plan_path = request.plan_path.clone().ok_or_else(|| {
        Refusal::validation("missing_plan", "`update apply` requires `--plan <file>`")
    })?;
    let document = plan::read_plan_file(Path::new(&plan_path))?;
    let structural = plan::structural_reasons(&document);
    if !structural.is_empty() {
        return Err(Refusal::validation(
            "plan_refused",
            format!(
                "the plan {plan_path} was refused by the canonical contract: {}; nothing was \
                 downloaded, swapped or pushed",
                plan::describe(&structural)
            ),
        ));
    }
    let approved = request.approve_digest.clone().ok_or_else(|| {
        Refusal::validation(
            "missing_approval_digest",
            "`update apply` requires `--approve-digest <sha256>`",
        )
    })?;
    let approvals = plan::approval_reasons(&document, &approved);
    if !approvals.is_empty() {
        // A digest that does not cover the plan body is a *conflict* with current state, not a
        // malformed argument: the same condition is classified the same way by `install --apply`
        // and `uninstall --apply`, so one canonical exit code (6) covers approval for every verb.
        return Err(Refusal::conflict(
            approvals[0].clone(),
            format!(
                "the plan {plan_path} does not match the approval digest {approved}: {}; nothing \
                 was applied",
                approvals.join("; ")
            ),
        ));
    }
    let plan_digest = plan::digest(&document).unwrap_or_default();
    let plan_id = plan_string(&document, &["plan_id"]);
    let plan_host = plan_string(&document, &["target", "host"]);
    if plan_host != host {
        return Err(Refusal::validation(
            "host_mismatch",
            format!(
                "the plan targets {plan_host} but this process is a {host} build: this \
                 distribution never applies another host's artifact set",
            ),
        ));
    }
    let now = Stamp::now();
    let expires = plan_string(&document, &["expires_at"]);
    if let Ok(deadline) = Stamp::parse(&expires) {
        if deadline.seconds() <= now.seconds() {
            return Err(Refusal::validation(
                "plan_expired",
                format!(
                    "the plan expired at {expires} and it is {}: rebuild the plan and approve the \
                     new digest instead of reusing a stale approval",
                    now.format()
                ),
            ));
        }
    }

    let recovery = journal::recover(state)?;
    let transaction = make_id("t");
    let _lock = Lock::acquire(state, &transaction)?;
    let context = load_context(state)?;
    // Section 5 of docs/20-VERSION-CHECK-UPDATE-RELEASE.md requires apply to re-verify that the
    // installed state and the trust metadata still match: an approval is bound to one trust
    // window, so a manifest that expired after the plan was approved is refused here instead of
    // being silently applied from a stale approval.
    check_trust_window(&context.manifest)?;
    let plan_root = plan_string(&document, &["target", "install_root"]);
    if !same_root(&plan_root, &context.root_text) {
        return Err(Refusal::validation(
            "install_root_mismatch",
            format!(
                "the plan records install_root {plan_root} but this installation is {}: an \
                 approval is bound to one install root",
                context.root_text
            ),
        ));
    }
    let plan_channel = plan_string(&document, &["channel"]);
    if plan_channel != context.manifest.channel {
        return Err(Refusal::validation(
            "channel_mismatch",
            format!(
                "the plan targets channel {plan_channel} but the installed release records \
                 channel {}: a channel manifest that was not the recorded one is never used",
                context.manifest.channel
            ),
        ));
    }
    pin_recorded_manifest(state, &context)?;

    let rows = plan_rows(&document);
    for row in &rows {
        let component = context.manifest.component(&row.component).ok_or_else(|| {
            Refusal::validation(
                format!("plan_names_unknown_component:{}", row.component),
                format!(
                    "the plan names `{}`, which the recorded channel manifest does not declare",
                    row.component
                ),
            )
        })?;
        if !component.declared {
            return Err(Refusal::validation(
                format!("plan_component_not_declared:{}", row.component),
                format!(
                    "the plan names `{}`, which the recorded channel manifest records as \
                     undeclared ({}): a version the owner does not declare is never applied",
                    row.component, component.declared_source
                ),
            ));
        }
        let artifact = component
            .artifact(&host, "per-user-installer")
            .or_else(|| component.artifact(&host, "oci-image"))
            .ok_or_else(|| {
                Refusal::validation(
                    format!("plan_artifact_not_in_recorded_manifest:{}", row.component),
                    format!(
                        "the recorded channel manifest publishes no artifact for `{}` on {host}",
                        row.component
                    ),
                )
            })?;
        let recorded_version = component.version.clone().unwrap_or_default();
        let recorded_revision = component.revision.clone().unwrap_or_default();
        if row.version != recorded_version
            || row.revision != recorded_revision
            || row.artifact_sha256 != artifact.sha256
        {
            return Err(Refusal::validation(
                format!("plan_not_in_recorded_manifest:{}", row.component),
                format!(
                    "the plan records `{}` as version {} revision {} digest {}, but the manifest \
                     this release recorded declares version {} revision {} digest {}: the plan is \
                     refused rather than applied, because a recorded digest that does not match \
                     the recorded bytes is not an approval",
                    row.component,
                    row.version,
                    row.revision,
                    row.artifact_sha256,
                    recorded_version,
                    recorded_revision,
                    artifact.sha256
                ),
            ));
        }
    }

    let from = context.installed.current_generation.clone();
    if context.active.plan_digest == plan_digest {
        let directory = state.generation_dir(&context.active.generation_id);
        context.active.verify(&directory)?;
        let message = format!(
            "generation {} is already the active generation for plan digest {} and every \
             recorded payload still digests to the value it was verified against: no new \
             generation was created and nothing was swapped",
            context.active.generation_id, plan_digest
        );
        let mut report = Report::ok("already_applied", message.clone());
        report.detail("subcommand", Json::text(request.subcommand.name()));
        report.detail("idempotent", Json::bool(true));
        report.detail("generation", Json::text(&context.active.generation_id));
        report.detail(
            "previous_generation",
            optional_text(context.installed.previous_generation.clone()),
        );
        report.detail("plan_id", Json::text(&plan_id));
        report.detail("plan_digest", Json::text(&plan_digest));
        report.detail("needs_restart", restart_report(&context.active));
        report.detail(
            "generations_on_disk",
            Json::text_array(
                &generations_on_disk(state)
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<&str>>(),
            ),
        );
        if recovery.acted() {
            report.detail(
                "recovered",
                Json::text_array(
                    &recovery
                        .actions
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<&str>>(),
                ),
            );
        }
        report.detail("reason", Json::text(&message));
        report.detail("reason_code", Json::text("already_applied"));
        report.line(format!(
            "generation {} already satisfies plan digest {}",
            context.active.generation_id, plan_digest
        ));
        return Ok(report);
    }

    let generation_id = make_id("g");
    let staging = state.staging_tree(&transaction);
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|error| {
            Refusal::io(
                "staging_cleanup_failed",
                &staging.display().to_string(),
                &error,
            )
        })?;
    }
    let staged = stage_artifacts(&staging, &host, &context, &rows);
    let (entries, acquired) = match staged {
        Ok(value) => value,
        Err(refusal) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(refusal);
        }
    };
    let generation = Generation {
        schema_version: GENERATION_SCHEMA_VERSION,
        generation_id: generation_id.clone(),
        transaction_id: transaction.clone(),
        plan_id: plan_id.clone(),
        plan_digest: plan_digest.clone(),
        channel: context.manifest.channel.clone(),
        host: host.clone(),
        install_root: context.root_text.clone(),
        created_at: Stamp::now(),
        entries,
    };
    let prepared = generation
        .write(&staging)
        .and_then(|_| generation.verify(&staging));
    if let Err(refusal) = prepared {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(refusal);
    }

    let mut record = context.installed.clone();
    let mut entry = journal::Journal {
        schema_version: journal::JOURNAL_SCHEMA_VERSION,
        transaction_id: transaction.clone(),
        plan_id: plan_id.clone(),
        plan_digest: plan_digest.clone(),
        channel: context.manifest.channel.clone(),
        host: host.clone(),
        install_root: context.root_text.clone(),
        state: journal::STATE_STAGED.to_string(),
        started_at: now,
        updated_at: now,
        from_generation: Some(from.clone()),
        to_generation: generation_id.clone(),
        needs_restart: generation.needs_restart(),
        actions: rows
            .iter()
            .map(|row| {
                Json::from_pairs(vec![
                    ("component", Json::text(&row.component)),
                    ("action", Json::text(&row.action)),
                    ("target_version", Json::text(&row.version)),
                ])
            })
            .collect(),
    };
    journal::write(state, &entry)?;

    let destination = state.generation_dir(&generation_id);
    if destination.exists() {
        return Err(Refusal::conflict(
            "generation_exists",
            format!(
                "generation {generation_id} already exists at {}; a generation is never \
                 overwritten in place",
                destination.display()
            ),
        ));
    }
    std::fs::rename(&staging, &destination).map_err(|error| {
        Refusal::io(
            "generation_commit_failed",
            &format!("{} -> {}", staging.display(), destination.display()),
            &error,
        )
    })?;
    record.current_generation = generation_id.clone();
    record.previous_generation = Some(from.clone());
    record.installed_at = Stamp::now();
    state.write_installed(&record)?;
    entry.advance(journal::STATE_SWAPPED, Stamp::now());
    journal::write(state, &entry)?;

    if let Err(failure) = health::check(state, &generation) {
        let mut restored = record.clone();
        restored.current_generation = from.clone();
        // The generation that failed its health check is NOT promoted to the retained
        // rollback target: AC1 keeps the previous generation until the new one is verified,
        // and a generation that just failed its probe is not verified. The failed generation
        // stays on disk under the journal that recorded it and is pruned by the next
        // successful transaction.
        restored.previous_generation = context.installed.previous_generation.clone();
        restored.installed_at = Stamp::now();
        state.write_installed(&restored)?;
        entry.advance(journal::STATE_ROLLED_BACK, Stamp::now());
        journal::write(state, &entry)?;
        return Err(Refusal::new(
            Class::Conflict,
            failure.reason,
            format!(
                "{}; the new generation {generation_id} did not pass its health check, so the \
                 transaction {transaction} rolled back and generation {from} is active again with \
                 its bytes unchanged",
                failure.message
            ),
        ));
    }

    entry.advance(journal::STATE_FINALIZED, Stamp::now());
    journal::write(state, &entry)?;
    let keep = vec![generation_id.clone(), from.clone()];
    let pruned = generation::prune(state, &keep)?;

    let mut report = Report::ok(
        "ok",
        format!(
            "transaction {transaction} applied generation {generation_id} over generation {from} \
             with one approval digest {plan_digest}; {} component(s) changed",
            generation.changed().len()
        ),
    );
    report.detail("idempotent", Json::bool(false));
    report.detail("subcommand", Json::text(request.subcommand.name()));
    report.detail("transaction", Json::text(&transaction));
    report.detail("generation", Json::text(&generation_id));
    report.detail("previous_generation", Json::text(&from));
    report.detail("plan_id", Json::text(&plan_id));
    report.detail("plan_digest", Json::text(&plan_digest));
    report.detail("channel", Json::text(&context.manifest.channel));
    report.detail(
        "channel_manifest_sha256",
        Json::text(&context.manifest_sha256),
    );
    report.detail("host", Json::text(&host));
    report.detail("install_root", Json::text(&context.root_text));
    report.detail(
        "changed",
        Json::text_array(
            &generation
                .changed()
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
        ),
    );
    report.detail(
        "needs_restart",
        Json::text_array(
            &generation
                .needs_restart()
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
        ),
    );
    report.detail("restart", restart_report(&generation));
    report.detail("payloads", payload_report(state, &generation));
    report.detail("journal_state", Json::text(journal::STATE_FINALIZED));
    report.detail("artifacts", Json::array(acquired));
    report.detail(
        "pruned",
        Json::text_array(&pruned.iter().map(String::as_str).collect::<Vec<&str>>()),
    );
    report.detail(
        "generations_on_disk",
        Json::text_array(
            &generations_on_disk(state)
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
        ),
    );
    if recovery.acted() {
        report.detail(
            "recovered",
            Json::text_array(
                &recovery
                    .actions
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<&str>>(),
            ),
        );
        report.lines(recovery.actions.clone());
    }
    report.line(format!("generation {generation_id} is now active"));
    report.lines(
        generation
            .entries
            .iter()
            .map(|entry| {
                format!(
                    "{}: action={} version={} needs_restart={}",
                    entry.component, entry.action, entry.version, entry.needs_restart
                )
            })
            .collect::<Vec<String>>(),
    );
    Ok(report)
}

/// `update rollback`: restore a retained generation, never unverified bytes.
fn rollback(state: &State, request: &Request) -> Result<Report, Refusal> {
    let selector = request.transaction.clone().ok_or_else(|| {
        Refusal::validation(
            "missing_transaction",
            "`update rollback` requires `--transaction <id>`; the reserved id `previous` selects \
             the generation the installed record retained",
        )
    })?;
    let recovery = journal::recover(state)?;
    let _lock = Lock::acquire(state, &make_id("rb"))?;
    let installed = state.read_installed()?;
    let target = if selector == "previous" {
        installed.previous_generation.clone().ok_or_else(|| {
            Refusal::conflict(
                "no_previous_generation",
                format!(
                    "generation {} is active and the installed record retains no previous \
                     generation, so there is nothing to roll back to",
                    installed.current_generation
                ),
            )
        })?
    } else {
        let record = journal::read(state, &selector)?;
        record.from_generation.clone().ok_or_else(|| {
            Refusal::conflict(
                "transaction_has_no_previous_generation",
                format!(
                    "transaction {selector} was started from no generation, so rolling it back \
                     would leave nothing active"
                ),
            )
        })?
    };
    if target == installed.current_generation {
        return Err(Refusal::conflict(
            "already_at_generation",
            format!("generation {target} is already the active generation"),
        ));
    }
    let generation = Generation::read(state, &target)?;
    generation.verify(&state.generation_dir(&target))?;
    if let Some(digest) = &request.approve_digest {
        if &generation.plan_digest != digest {
            return Err(Refusal::validation(
                "approval_stale",
                format!(
                    "the approval digest {digest} does not match the plan digest {} that \
                     generation {target} was applied from: nothing was rolled back",
                    generation.plan_digest
                ),
            ));
        }
    }
    let replaced = installed.current_generation.clone();
    let mut record = installed.clone();
    record.current_generation = target.clone();
    record.previous_generation = Some(replaced.clone());
    record.installed_at = Stamp::now();
    state.write_installed(&record)?;
    let pruned = generation::prune(state, &[target.clone(), replaced.clone()])?;

    let mut report = Report::ok(
        "rolled_back",
        format!(
            "generation {target} is active again and generation {replaced} is retained as the \
             previous generation; every payload of {target} was re-verified before it became \
             active"
        ),
    );
    report.detail("subcommand", Json::text(request.subcommand.name()));
    report.detail("generation", Json::text(&target));
    report.detail("replaced_generation", Json::text(&replaced));
    report.detail("plan_id", Json::text(&generation.plan_id));
    report.detail("plan_digest", Json::text(&generation.plan_digest));
    report.detail("selector", Json::text(&selector));
    report.detail("needs_restart", restart_report(&generation));
    report.detail(
        "pruned",
        Json::text_array(&pruned.iter().map(String::as_str).collect::<Vec<&str>>()),
    );
    report.detail(
        "generations_on_disk",
        Json::text_array(
            &generations_on_disk(state)
                .iter()
                .map(String::as_str)
                .collect::<Vec<&str>>(),
        ),
    );
    if recovery.acted() {
        report.detail(
            "recovered",
            Json::text_array(
                &recovery
                    .actions
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<&str>>(),
            ),
        );
        report.lines(recovery.actions.clone());
    }
    report.line(format!("generation {target} is now active"));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::json::parse as parse_json;

    #[test]
    fn an_install_root_is_compared_after_normalising_separators_and_trailing_slashes() {
        assert!(same_root("/a/b", "/a/b"));
        assert!(same_root("/a/b/", "/a/b"));
        assert!(same_root("a\\b", "a/b"));
        assert!(same_root("a\\b\\", "a/b/"));
        assert!(!same_root("/a/b", "/a/c"));
        assert!(!same_root("/a/b", "/a/b/c"));
        if cfg!(windows) {
            assert!(same_root("C:\\Users\\X", "c:/users/x"));
        } else {
            assert!(!same_root("/a/B", "/a/b"));
        }
    }

    #[test]
    fn plan_rows_read_every_component_row_and_skip_the_rest() {
        let document = parse_json(
            "{\"components\":[\
             {\"component\":\"axiom-cli\",\"action\":\"update\",\"target_version\":\"0.0.2\",\
             \"target_revision\":\"ab\",\"artifact_sha256\":\"cd\"},\
             \"not-an-object\",\
             {\"component\":\"skills\"}]}",
        )
        .expect("the fixture plan must parse");
        let rows = plan_rows(&document);
        assert_eq!(
            rows.len(),
            2,
            "a non-object entry is skipped, not guessed at"
        );
        assert_eq!(rows[0].component, "axiom-cli");
        assert_eq!(rows[0].action, "update");
        assert_eq!(rows[0].version, "0.0.2");
        assert_eq!(rows[0].revision, "ab");
        assert_eq!(rows[0].artifact_sha256, "cd");
        assert_eq!(rows[1].component, "skills");
        assert_eq!(
            rows[1].version, "",
            "an absent field stays empty instead of being invented"
        );
        assert!(
            plan_rows(&parse_json("{}").expect("an empty object parses")).is_empty(),
            "a plan without a components list has no rows"
        );
    }

    #[test]
    fn a_transaction_id_is_lowercase_stamped_and_prefixed() {
        let id = make_id("t");
        assert!(id.starts_with("t-"), "{id}");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{id}"
        );
        assert!(id.len() >= 20, "{id}");
        let suffix = &id[id.len() - 8..];
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{id}"
        );
        assert_ne!(id, make_id("g"), "the prefix is part of the id");
    }
}
