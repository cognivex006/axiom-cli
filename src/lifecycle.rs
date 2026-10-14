//! `axiom-cli install` and `axiom-cli uninstall` - the distribution transaction.
//!
//! The distribution contract makes this repository the owner of *what is distributed and how it
//! is delivered, verified and updated*, while the installation engine, the bootstrap rules, the
//! service lifecycle and the per-component update transaction stay in `axiom-graphd`. This
//! module implements exactly the distribution half and delegates the engine half through the
//! engine's published argv surface:
//!
//! 1. resolve the delivery target and refuse to act on a target the contract does not declare;
//! 2. resolve the release set - a local release set named by `--from`, else the channel manifest
//!    the installed release records, and never a branch tip, a tag alias or a network `latest`;
//! 3. verify every artifact for this host by byte length and sha256 *before* it is used;
//! 4. build the canonical plan and its digest, and require `--approve-digest` to name that exact
//!    digest before anything mutating runs (supplying `--plan` or `--from` is not approval);
//! 5. delegate the engine-owned placement/removal to the engine binary, and report a precise
//!    *not found* when the engine is absent rather than pretending the work happened.
//!
//! `--dry-run` performs steps 1-4 and changes nothing. `--apply` performs all five. Neither
//! form touches user data, the graph output root or portable workspace state.

use std::path::{Path, PathBuf};

use crate::bundle;
use crate::engine::{self, Located};
use crate::target;
use crate::update::channel::Manifest;
use crate::update::error::{Class, Refusal};
use crate::update::json::Json;
use crate::update::report::Report;
use crate::update::{fetch, plan as plan_contract, state, time::Stamp};

/// The modes of the distribution transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// `--dry-run`: report the plan without changing the host.
    DryRun,
    /// `--apply`: run the transaction; requires an approval digest bound to the plan digest.
    Apply,
    /// Bare verb (ADR-0033 decision 4): print the plan, ask once (or `--yes`), then apply that
    /// exact plan with its own digest. Without a terminal and without `--yes`: exit `4`, no change.
    Interactive,
}

impl Mode {
    /// Canonical mode token.
    pub fn name(self) -> &'static str {
        match self {
            Mode::DryRun => "dry-run",
            Mode::Apply => "apply",
            Mode::Interactive => "interactive",
        }
    }
}

/// A parsed `install` request.
#[derive(Clone, Debug)]
pub struct InstallRequest {
    /// `--dry-run` or `--apply`.
    pub mode: Mode,
    /// `--from <path>`: a local release set instead of the channel.
    pub from: Option<String>,
    /// `--approve-digest <sha256>`.
    pub approve_digest: Option<String>,
    /// `--yes` / `-Yes`: pre-approve the interactive plan.
    pub yes: bool,
    /// `false` with `--no-modify-path`: the interactive install leaves the user PATH alone.
    pub modify_path: bool,
    /// `--adopt <path>`: turn a legacy bootstrap root into the install root, in place.
    pub adopt: Option<String>,
}

/// A parsed `uninstall` request.
#[derive(Clone, Debug)]
pub struct UninstallRequest {
    /// `--dry-run` or `--apply`.
    pub mode: Mode,
    /// `--approve-digest <sha256>`.
    pub approve_digest: Option<String>,
    /// `--purge-data`: additionally delete workspace data, which needs the engine.
    pub purge_data: bool,
    /// `--yes`: pre-approve the interactive plan.
    pub yes: bool,
}

/// Run `install`.
pub fn run_install(request: InstallRequest, json: bool, verbose: bool) -> i32 {
    match install(request, json) {
        Ok(report) => report.emit(json, verbose),
        Err(refusal) => refusal_report("install", &refusal).emit(json, verbose),
    }
}

/// Run `uninstall`.
pub fn run_uninstall(request: UninstallRequest, json: bool, verbose: bool) -> i32 {
    match uninstall(request, json) {
        Ok(report) => report.emit(json, verbose),
        Err(refusal) => refusal_report("uninstall", &refusal).emit(json, verbose),
    }
}

fn refusal_report(subject: &'static str, refusal: &Refusal) -> Report {
    let mut report = match refusal.class {
        Class::NotReady => Report::not_ready(refusal.message.clone()),
        Class::Validation => Report::refused(refusal.message.clone()),
        other => Report::new(other.exit_code(), other.status(), refusal.message.clone()),
    }
    .subject(subject);
    report.detail("reason", Json::text(&refusal.message));
    report.detail("reason_code", Json::text(&refusal.reason));
    if refusal.class.retryable() {
        report = report.retryable();
    }
    report
}

/// The resolved release set and the canonical plan built from it.
struct ReleaseSet {
    manifest: Manifest,
    manifest_sha256: String,
    manifest_source: String,
    /// Directory that local artifact bytes are resolved from, when the set is local.
    local_root: Option<PathBuf>,
    /// Digest of the canonical plan body.
    plan: Json,
    plan_digest: String,
}

fn install(request: InstallRequest, json: bool) -> Result<Report, Refusal> {
    if request.mode != Mode::DryRun {
        if let Some(refusal) = crate::elevation::refusal() {
            return Err(refusal);
        }
    }
    if let Some(target) = request.adopt.as_deref() {
        let path = PathBuf::from(target);
        match crate::legacy::classify(&path) {
            Some(layout) if matches!(layout.kind, "bootstrap-root" | "cli-store") => {
                // The adopted root is this process's install root for the whole transaction.
                std::env::set_var(state::INSTALL_ROOT_ENV, &path);
            }
            Some(layout) => {
                return Err(Refusal::validation(
                    "adopt_target_unknown_layout",
                    format!(
                        "{} is not a recognised Axiom layout ({}); it was not modified",
                        path.display(),
                        layout.kind
                    ),
                ))
            }
            None => {
                return Err(Refusal::validation(
                    "adopt_target_not_legacy",
                    format!(
                        "{} is not a legacy Axiom installation (absent, empty or already an \
                         current installation root); nothing was changed",
                        path.display()
                    ),
                ))
            }
        }
    }
    let path_change = (request.mode == Mode::Interactive).then_some(request.modify_path);
    let set = resolve_release_set(request.from.as_deref(), path_change)?;

    if request.mode == Mode::Apply {
        let approval = plan_contract::approval_reasons(
            &set.plan,
            request.approve_digest.as_deref().unwrap_or(""),
        );
        if !approval.is_empty() {
            return Err(Refusal::conflict(
                "approval_required",
                format!(
                    "the install plan digest is {}; {}",
                    set.plan_digest,
                    plan_contract::describe(&approval)
                ),
            ));
        }
    }

    // Verify every artifact this host would receive, before anything is used.
    let verified = verify_artifacts(&set)?;

    // A plan this host would install nothing from is not a plan, and that is a property of the
    // release set rather than of the mode: `--dry-run` and `--apply` answer the same way, so an
    // operator who approves a digest is never approving a no-op. The message states the fact
    // rather than the mode.
    if verified.is_empty() {
        return Err(nothing_to_install(&set));
    }

    let mut decision = None;
    if request.mode == Mode::Interactive {
        let answer = crate::confirm::decide(request.yes, json, &plan_lines(&set));
        decision = Some(answer);
        if let Some(report) = unconfirmed(
            "install",
            answer,
            &set.plan,
            &set.plan_digest,
            plan_lines(&set),
        ) {
            return Ok(report);
        }
    }

    let mut report = if matches!(request.mode, Mode::Apply | Mode::Interactive) {
        apply_install(&set, &verified)?
    } else {
        Report::ok("ok", "install plan reported; nothing was changed").subject("install")
    };
    if let Some(answer) = decision {
        report.detail("confirmation", Json::text(answer.name()));
    }
    report.detail("mode", Json::text(request.mode.name()));
    report.detail("generated_at", Json::text(&Stamp::now().format()));
    report.detail(
        "target",
        Json::text(set.plan.get("host").and_then(Json::as_text).unwrap_or("")),
    );
    report.detail(
        "install_root",
        set.plan.get("install_root").cloned().unwrap_or(Json::Null),
    );
    report.detail("channel", Json::text(&set.manifest.channel));
    report.detail("manifest_source", Json::text(&set.manifest_source));
    report.detail("manifest_sha256", Json::text(&set.manifest_sha256));
    report.detail("published", Json::bool(set.manifest.published));
    report.detail("plan", set.plan.clone());
    report.detail("plan_digest", Json::text(&set.plan_digest));
    report.detail("verified_artifacts", Json::array(verified));
    if decision != Some(crate::confirm::Decision::Confirmed) {
        // A confirmed plan was already printed above the prompt.
        for line in plan_lines(&set) {
            report.line(line);
        }
    }
    Ok(report)
}

/// The report for an interactive plan that was not confirmed, or `None` to proceed.
///
/// No terminal and no `--yes`: the plan is printed, nothing changes, exit `4`. Declined at the
/// prompt: nothing changes, exit `5` (authorization).
pub(crate) fn unconfirmed(
    subject: &'static str,
    answer: crate::confirm::Decision,
    plan: &Json,
    digest: &str,
    lines: Vec<String>,
) -> Option<Report> {
    let mut report = match answer {
        crate::confirm::Decision::NoTerminal => {
            let mut report = Report::not_ready(format!(
                "the {subject} plan was printed and nothing was changed: there is no terminal to \
                 confirm it. Re-run with `--yes` (or AXIOM_INSTALL_YES=1) to apply plan {digest}, \
                 or use `--dry-run` / `--apply --approve-digest`"
            ))
            .subject(subject);
            report.detail("reason_code", Json::text("confirmation_required"));
            report.lines(lines);
            report
        }
        crate::confirm::Decision::Declined => {
            let mut report = Report::new(
                crate::cli::exit::AUTHORIZATION,
                "authorization",
                format!("{subject} declined at the confirmation prompt; nothing was changed"),
            )
            .subject(subject);
            report.detail("reason_code", Json::text("confirmation_declined"));
            report
        }
        crate::confirm::Decision::PreApproved | crate::confirm::Decision::Confirmed => return None,
    };
    report.detail("plan", plan.clone());
    report.detail("plan_digest", Json::text(digest));
    report.detail("confirmation", Json::text(answer.name()));
    Some(report)
}

/// Build the plan, and - for `--apply` - hand the engine-owned placement to the engine.
///
/// The ownership boundary is explicit here, because getting it wrong is how a distribution layer
/// reports an install that never happened. `axiom-cli` resolves and verifies the release set; the
/// placement of verified bytes is owned by `axiom-graphd`. The engine publishes that placement as
/// `install plan --bundle <dir>` followed by `install apply --plan <file>` over its own ecosystem
/// plan document, built from a verified local bundle. This layer therefore
///
/// * refuses with `not_found` when no engine binary is present, naming every place it looked;
/// * refuses with `not_ready` when verified bytes exist but the engine bundle they belong to
///   cannot be assembled (`bundle::assemble` states exactly which input is missing) - never
///   handing the engine a distribution plan document it cannot validate; and
/// * otherwise assembles the bundle from bytes it already verified and invokes the engine, then
///   reports the engine's own status and exit code with its raw stdout and stderr as evidence.
///
/// Every refusal states that nothing was placed, and none is a bare "unbuilt verb": the
/// verification work above really ran.
/// The refusal for a release set that declares nothing this host could receive.
fn nothing_to_install(set: &ReleaseSet) -> Refusal {
    Refusal::not_ready(
        "nothing_to_install",
        format!(
            "the install plan digest is {}; the release set from {} declares no artifact for host \
             {} (channel `{}` published={}); there is nothing to download or place, so nothing was \
             installed or repaired",
            set.plan_digest,
            set.manifest_source,
            set.plan.get("host").and_then(Json::as_text).unwrap_or("?"),
            set.manifest.channel,
            set.manifest.published
        ),
    )
}

fn apply_install(set: &ReleaseSet, verified: &[Json]) -> Result<Report, Refusal> {
    if verified.is_empty() {
        // The install flow refuses this before the mode branch, so the guard cannot be reached
        // from the verb. It is kept because `apply_install` owns the placement contract and a
        // future caller must not be able to reach the engine with nothing to place.
        return Err(nothing_to_install(set));
    }
    let engine = install_engine(verified)?;
    let root = state::default_root().ok_or_else(|| {
        Refusal::not_ready(
            "no_install_root",
            "this host offers no per-user install root, so the assembled bundle has nowhere to \
             live and the engine has no `AXIOM_HOME`",
        )
    })?;
    let adopting: Vec<crate::legacy::Layout> = crate::legacy::classify(&root)
        .filter(|layout| matches!(layout.kind, "cli-store" | "bootstrap-root"))
        .into_iter()
        .collect();
    crate::layout::public_command_conflict(&root, verified)?;
    if !root.join(state::INSTALLED_FILE).is_file() && adopting.is_empty() {
        if let Some(refusal) = crate::layout::foreign_root_refusal(&root) {
            return Err(refusal);
        }
    }
    for layout in adopting.iter().filter(|layout| layout.kind == "cli-store") {
        crate::legacy::backup_cli_store(layout)?;
    }
    let staging_root = root.join("staging").join("engine-bundle");
    if let (Some(bound), Some(inputs)) = (
        set.plan.get("mcp_runtime"),
        set.local_root.as_deref().and_then(crate::runtime::locate),
    ) {
        let composite = std::env::var_os("AXIOM_CLI_COMPOSITE_UPDATE").as_deref()
            == Some(std::ffi::OsStr::new("1"));
        if composite || !crate::runtime::provisioned(&root) {
            let version = set
                .plan
                .get("components")
                .and_then(Json::as_array)
                .unwrap_or(&[])
                .iter()
                .find(|item| item.get("component").and_then(Json::as_text) == Some("axiom-mcp"))
                .and_then(|item| item.get("version"))
                .and_then(Json::as_text)
                .unwrap_or("")
                .to_string();
            crate::runtime::provision(&inputs, bound, &root, &version)?;
        }
    }
    let mcp_runtime_bin = if matches!(
        set.plan.get("host").and_then(Json::as_text),
        Some("macos-x64" | "linux-x64" | "windows-x64")
    ) {
        provisioned_mcp_runtime_bin(&root)?
    } else {
        None
    };
    let plan_components = set
        .plan
        .get("components")
        .and_then(Json::as_array)
        .unwrap_or(&[]);
    let request = bundle::Request {
        staging_root: staging_root.clone(),
        install_root: root,
        host: set
            .plan
            .get("host")
            .and_then(Json::as_text)
            .unwrap_or("")
            .to_string(),
        channel: set.manifest.channel.clone(),
        created_at: set.manifest.updated_at.format(),
        release_root: set.local_root.clone(),
        verified,
        plan_components,
        mcp_runtime_bin,
    };
    // A refusal from the assembly or the engine is *this* layer's answer, so it names the plan
    // the operator approved: the engine's refusal alone would leave `--apply --approve-digest`'s
    // subject unstated. The class and reason token are the ones the boundary chose.
    let composite_update = std::env::var_os("AXIOM_CLI_COMPOSITE_UPDATE").as_deref()
        == Some(std::ffi::OsStr::new("1"));
    if composite_update
        && !request
            .install_root
            .join("installs/ecosystem/current")
            .is_file()
    {
        return Err(Refusal::not_ready(
            "engine_generation_missing",
            "a composite update requires an already installed engine generation",
        ));
    }
    let result = if composite_update {
        bundle::update(&engine, &request)
    } else {
        bundle::apply(&engine, &request)
    };
    let mut report = result.map_err(|refusal| {
        Refusal::new(
            refusal.class,
            refusal.reason,
            format!(
                "the install plan digest is {}; {}",
                set.plan_digest, refusal.message
            ),
        )
    })?;
    if report.code() != 0 {
        // The engine refused or did not finish: nothing is recorded, so the next run repeats the
        // install instead of trusting a placement that did not happen.
        report.detail(
            "staging_root",
            Json::text(&staging_root.display().to_string()),
        );
        report.detail(
            "engine_program",
            Json::text(&engine.program().display().to_string()),
        );
        report.detail("engine_source", Json::text(engine.source()));
        return Ok(report);
    }
    let recorded = record_layout(set, request.verified, &request.install_root, &engine)
        .map_err(|refusal| {
            Refusal::new(
                refusal.class,
                refusal.reason,
                format!(
                    "the engine placed the approved bundle but the install record was not                      written, so the next run repeats the install: {}",
                    refusal.message
                ),
            )
        })?;
    report.detail("installed_record", recorded.to_json());
    if let Some(block) = set.plan.get("path_change") {
        if block.get("action").and_then(Json::as_text) == Some("already-present")
            && !adopting.is_empty()
        {
            // The legacy installer put this bin on the user PATH; the adopting install now owns
            // the entry, so uninstall removes it.
            crate::pathenv::adopt(&request.install_root, block)?;
        }
        let change = crate::pathenv::apply(&request.install_root, block).map_err(|refusal| {
            Refusal::new(
                refusal.class,
                refusal.reason,
                format!(
                    "Axiom is installed and recorded, but the announced PATH change failed, so \
                     run {} directly or add it to PATH yourself: {}",
                    crate::layout::bin_dir(&request.install_root).display(),
                    refusal.message
                ),
            )
        })?;
        report.detail("path_change", change);
        if block.get("action").and_then(Json::as_text) == Some("add") {
            report.line(format!(
                "PATH: added {} for this user; open a new terminal to use `axiom-cli`",
                crate::layout::bin_dir(&request.install_root).display()
            ));
        }
    }
    report.line(format!(
        "installed: generation {} recorded in {}",
        recorded.current_generation,
        request.install_root.join(state::INSTALLED_FILE).display()
    ));
    report.detail(
        "staging_root",
        Json::text(&staging_root.display().to_string()),
    );
    report.detail(
        "engine_program",
        Json::text(&engine.program().display().to_string()),
    );
    report.detail("engine_source", Json::text(engine.source()));
    if let Some(bin) = &request.mcp_runtime_bin {
        report.detail("mcp_runtime_bin", Json::text(&bin.display().to_string()));
    }
    Ok(report)
}

/// Put `axiom-cli`, `axiom` and `axiom-graphd` side by side in `<root>/bin` and write the single
/// `installed.json` record (ADR-0033 decision 3, task L-002) after the engine placed the bundle.
fn record_layout(
    set: &ReleaseSet,
    verified: &[Json],
    root: &Path,
    engine: &engine::Engine,
) -> Result<state::Installed, Refusal> {
    use crate::layout::{self, BinSource};
    let resolved_of = |component: &str| -> Option<(PathBuf, String)> {
        verified
            .iter()
            .find(|entry| entry.get("component").and_then(Json::as_text) == Some(component))
            .and_then(|entry| {
                Some((
                    PathBuf::from(entry.get("resolved").and_then(Json::as_text)?),
                    entry.get("sha256").and_then(Json::as_text)?.to_string(),
                ))
            })
    };
    let mut sources = Vec::new();
    let cli = match resolved_of("axiom-cli") {
        Some(found) => found,
        None => {
            let exe = std::env::current_exe()
                .map_err(|error| Refusal::io("current_exe_unresolved", "axiom-cli", &error))?;
            let digest = layout::file_digest(&exe)?;
            (exe, digest)
        }
    };
    sources.push(BinSource {
        name: "axiom-cli",
        source: cli.0,
        sha256: cli.1,
    });
    let engine_program = engine.program().to_path_buf();
    let engine_digest = match resolved_of("axiom") {
        Some((_, digest)) => digest,
        None => layout::file_digest(&engine_program)?,
    };
    sources.push(BinSource {
        name: "axiom",
        source: engine_program,
        sha256: engine_digest,
    });
    if let Some((path, digest)) = resolved_of("axiom-graphd") {
        sources.push(BinSource {
            name: "axiom-graphd",
            source: path,
            sha256: digest,
        });
    }
    // ADR-0036: `axm` is the short name of `axiom-cli`, a verified copy of the same bytes.
    layout::with_short_command(&mut sources);
    layout::with_public_commands(&mut sources);
    let bin = layout::place_bin(root, &sources)?;
    let bin_payloads: Vec<(String, PathBuf, String)> = sources
        .iter()
        .map(|item| {
            (
                format!("bin/{}", item.name),
                layout::bin_dir(root).join(layout::program_file(item.name)),
                item.sha256.clone(),
            )
        })
        .collect();

    let manifest_path = Path::new(&set.manifest_source);
    let manifest_bytes = state::read_bytes(manifest_path, "manifest_unreadable")?;
    if crate::update::sha256::hex(&crate::update::sha256::digest(&manifest_bytes))
        != set.manifest_sha256
    {
        return Err(Refusal::validation(
            "manifest_changed_after_plan",
            format!(
                "the channel manifest {} changed after the plan was approved",
                manifest_path.display()
            ),
        ));
    }
    let plan_components = set
        .plan
        .get("components")
        .and_then(Json::as_array)
        .unwrap_or(&[]);
    let mut components = Vec::new();
    for entry in verified {
        let component = entry.get("component").and_then(Json::as_text).unwrap_or("");
        let planned = plan_components
            .iter()
            .find(|item| item.get("component").and_then(Json::as_text) == Some(component));
        let field = |key: &str| {
            planned
                .and_then(|item| item.get(key))
                .and_then(Json::as_text)
                .unwrap_or("")
                .to_string()
        };
        components.push((
            component.to_string(),
            field("version"),
            field("revision"),
            entry
                .get("sha256")
                .and_then(Json::as_text)
                .unwrap_or("")
                .to_string(),
            PathBuf::from(entry.get("resolved").and_then(Json::as_text).unwrap_or("")),
            planned
                .and_then(|item| item.get("needs_restart"))
                .and_then(Json::as_bool)
                .unwrap_or(false),
        ));
    }
    let release_version = components
        .iter()
        .find(|item| item.0 == "axiom-graphd")
        .or_else(|| components.first())
        .map(|item| item.1.clone())
        .unwrap_or_default();
    for (component, path, digest) in bin_payloads {
        components.push((
            component,
            release_version.clone(),
            String::new(),
            digest,
            path,
            false,
        ));
    }
    layout::record(
        root,
        &layout::Record {
            channel: &set.manifest.channel,
            host: set.plan.get("host").and_then(Json::as_text).unwrap_or(""),
            manifest_bytes: &manifest_bytes,
            manifest_sha256: &set.manifest_sha256,
            plan_digest: &set.plan_digest,
            components,
            bin,
        },
    )
}

/// Read a provisioned candidate runtime as a verified engine input. An absent
/// runtime retains the pre-K-104 behavior; a present but changed record refuses.
pub(crate) fn provisioned_mcp_runtime_bin(root: &Path) -> Result<Option<PathBuf>, Refusal> {
    let mut runtime_root = root.join("mcp-runtime");
    if !runtime_root.join("current.json").exists() {
        // An adopted 0.1.2 Windows bootstrap root keeps its runtime at the nested location the
        // old bootstrap produced; the venv holds absolute paths, so it is used where it is.
        if let Some(pointer) = crate::legacy::nested_runtime_pointer(root) {
            runtime_root = pointer
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or(runtime_root);
        }
    }
    let pointer = runtime_root.join("current.json");
    if !pointer.exists() {
        return Ok(None);
    }
    if pointer.is_symlink() {
        return Err(Refusal::validation(
            "mcp_runtime_pointer_link",
            "MCP runtime pointer is a symlink",
        ));
    }
    let bytes = std::fs::read_to_string(&pointer).map_err(|error| {
        Refusal::io(
            "mcp_runtime_pointer_unreadable",
            &pointer.display().to_string(),
            &error,
        )
    })?;
    let value = crate::update::json::parse(&bytes).map_err(|_| {
        Refusal::validation(
            "mcp_runtime_pointer_invalid",
            "MCP runtime pointer is invalid JSON",
        )
    })?;
    let generation = value
        .get("generation")
        .and_then(Json::as_text)
        .unwrap_or("");
    if generation.len() != 24 || !generation.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Refusal::validation(
            "mcp_runtime_generation_invalid",
            "MCP runtime generation is invalid",
        ));
    }
    let version = runtime_root.join("versions").join(generation);
    if version.is_symlink()
        || version.join("owner.json").is_symlink()
        || !version.join("owner.json").is_file()
    {
        return Err(Refusal::validation(
            "mcp_runtime_unowned",
            "MCP runtime generation is not owned",
        ));
    }
    let bin = version
        .join("venv")
        .join(if cfg!(windows) { "Scripts" } else { "bin" });
    if version.join("venv").is_symlink() || bin.is_symlink() {
        return Err(Refusal::validation(
            "mcp_runtime_path_link",
            "MCP runtime path is linked outside the owned generation",
        ));
    }
    if !bin.is_absolute() || bin.to_string_lossy().contains(';') {
        return Err(Refusal::validation(
            "mcp_runtime_path_invalid",
            "MCP runtime path cannot enter PATH safely",
        ));
    }
    let canonical_bin = bin.canonicalize().map_err(|error| {
        Refusal::io(
            "mcp_runtime_path_unreadable",
            &bin.display().to_string(),
            &error,
        )
    })?;
    let canonical_version = version.canonicalize().map_err(|error| {
        Refusal::io(
            "mcp_runtime_path_unreadable",
            &version.display().to_string(),
            &error,
        )
    })?;
    if !canonical_bin.starts_with(canonical_version) {
        return Err(Refusal::validation(
            "mcp_runtime_path_invalid",
            "MCP runtime path escapes its owned generation",
        ));
    }
    let programs = if cfg!(windows) {
        [
            ("python.exe", "python_sha256"),
            ("axiom-mcp.exe", "executable_sha256"),
        ]
    } else {
        [
            ("python", "python_sha256"),
            ("axiom-mcp", "executable_sha256"),
        ]
    };
    for (name, key) in programs {
        let file = bin.join(name);
        if file.is_symlink() || !file.is_file() {
            return Err(Refusal::validation(
                "mcp_runtime_executable_missing",
                "MCP runtime executable is missing or linked",
            ));
        }
        let expected = value.get(key).and_then(Json::as_text).unwrap_or("");
        let actual = crate::update::sha256::file_hex(&file).map_err(|error| {
            Refusal::io(
                "mcp_runtime_executable_unreadable",
                &file.display().to_string(),
                &error,
            )
        })?;
        if actual != expected {
            return Err(Refusal::validation(
                "mcp_runtime_executable_changed",
                "MCP runtime executable digest changed",
            ));
        }
    }
    Ok(Some(bin))
}

/// Prefer the digest-verified engine CLI that belongs to this release set.
/// Older local bundles without an `axiom` artifact still use explicit/PATH discovery.
fn install_engine(verified: &[Json]) -> Result<engine::Engine, Refusal> {
    if let Some(artifact) = verified
        .iter()
        .find(|entry| entry.get("component").and_then(Json::as_text) == Some("axiom"))
    {
        let path = artifact
            .get("resolved")
            .and_then(Json::as_text)
            .ok_or_else(|| {
                Refusal::validation(
                    "engine_artifact_path_missing",
                    "verified axiom artifact has no resolved path",
                )
            })?;
        return engine::from_verified_artifact(Path::new(path));
    }
    match engine::locate() {
        Located::Found(found) => Ok(found),
        Located::Missing(searched) => Err(engine::missing_refusal(&searched)),
    }
}

/// Run `uninstall`.
fn uninstall(request: UninstallRequest, json: bool) -> Result<Report, Refusal> {
    if request.mode != Mode::DryRun {
        if let Some(refusal) = crate::elevation::refusal() {
            return Err(refusal);
        }
    }
    if request.purge_data {
        return Err(Refusal::validation(
            "purge_data_unsupported",
            "uninstall preserves user data; this distribution wrapper does not expose purge-data",
        ));
    }
    let root = state::default_root().ok_or_else(|| {
        Refusal::not_ready(
            "no_install_root",
            "this host offers no per-user install root, so there is nothing this layer owns to remove",
        )
    })?;
    let st = state::State::new(root.clone());

    let mut plan = uninstall_plan(&st, request.purge_data)?;
    if st.has_installed() || engine_current_exists(&root) {
        plan.set("installed", Json::bool(true))
            .map_err(|e| Refusal::validation("installed", e))?;
        plan.set("engine_plan", engine_uninstall_plan(&root)?)
            .map_err(|e| Refusal::validation("engine_plan", e))?;
        if st.has_installed() {
            let bytes = std::fs::read(st.installed_path()).map_err(|e| {
                Refusal::io(
                    "installed_marker_read",
                    &st.installed_path().display().to_string(),
                    &e,
                )
            })?;
            plan.set(
                "installed_marker_sha256",
                Json::text(&crate::update::sha256::hex(&crate::update::sha256::digest(
                    &bytes,
                ))),
            )
            .map_err(|e| Refusal::validation("installed_marker", e))?;
        }
    }
    if let Some(block) = owned_removals(&root)? {
        plan.set("owned_removals", block)
            .map_err(|e| Refusal::validation("owned_removals", e))?;
    }
    let digest = plan_contract::digest(&plan).ok_or_else(|| {
        Refusal::validation(
            "plan_unbuildable",
            "the uninstall plan could not be digested",
        )
    })?;
    seal(&mut plan, &digest)?;

    let mut decision = None;
    if request.mode == Mode::Interactive {
        let lines = uninstall_lines(&plan, &root);
        let answer = crate::confirm::decide(request.yes, json, &lines);
        decision = Some(answer);
        if let Some(report) = unconfirmed("uninstall", answer, &plan, &digest, lines) {
            return Ok(report);
        }
    }

    if request.mode == Mode::Apply {
        let reasons =
            plan_contract::approval_reasons(&plan, request.approve_digest.as_deref().unwrap_or(""));
        if !reasons.is_empty() {
            return Err(Refusal::conflict(
                "approval_required",
                format!(
                    "the uninstall plan digest is {digest}; {}",
                    plan_contract::describe(&reasons)
                ),
            ));
        }
    }

    let mut report = if matches!(request.mode, Mode::Apply | Mode::Interactive) {
        apply_uninstall(&plan, &digest, request.purge_data)?
    } else {
        let mut report =
            Report::ok("ok", "uninstall plan reported; nothing was removed").subject("uninstall");
        report.lines(uninstall_lines(&plan, &root));
        report
    };
    if let Some(answer) = decision {
        report.detail("confirmation", Json::text(answer.name()));
    }
    report.detail("mode", Json::text(request.mode.name()));
    report.detail("generated_at", Json::text(&Stamp::now().format()));
    report.detail("purge_data", Json::bool(request.purge_data));
    report.detail("install_root", Json::text(&root.display().to_string()));
    report.detail("plan", plan);
    report.detail("plan_digest", Json::text(&digest));
    Ok(report)
}

fn engine_current_exists(root: &std::path::Path) -> bool {
    root.join("installs/ecosystem/current").is_file()
}

fn apply_uninstall(plan: &Json, digest: &str, _purge_data: bool) -> Result<Report, Refusal> {
    let installed = plan
        .get("installed")
        .and_then(Json::as_bool)
        .unwrap_or(false);
    if !installed {
        return Err(Refusal::new(
            Class::NotFound,
            "nothing_installed",
            "no installed release is recorded, so there is nothing for this layer to remove",
        ));
    }
    let engine = match engine::locate() {
        Located::Found(found) => found,
        Located::Missing(searched) => return Err(engine::missing_refusal(&searched)),
    };
    let root = plan
        .get("install_root")
        .and_then(Json::as_text)
        .unwrap_or_default();
    let embedded = plan.get("engine_plan").ok_or_else(|| {
        Refusal::validation(
            "engine_plan_missing",
            "outer uninstall plan has no engine plan",
        )
    })?;
    let engine_digest = embedded
        .get("plan_digest")
        .and_then(Json::as_text)
        .map(str::to_owned)
        .ok_or_else(|| {
            Refusal::validation(
                "engine_uninstall_plan_digest",
                "engine uninstall plan has no digest",
            )
        })?;
    let plan_path = std::path::Path::new(root).join("state/engine-uninstall-approved.json");
    std::fs::write(&plan_path, crate::update::json::canonical_bytes(embedded)).map_err(|e| {
        Refusal::io(
            "engine_uninstall_plan_write",
            &plan_path.display().to_string(),
            &e,
        )
    })?;
    let applied = engine.invoke_with_env(
        &[
            "uninstall".into(),
            "apply".into(),
            "--plan".into(),
            plan_path.display().to_string(),
            "--approve-digest".into(),
            engine_digest,
        ],
        &[("AXIOM_HOME", root.to_owned())],
    )?;
    if applied.exit_code() != 0 {
        return Err(Refusal::not_ready(
            "engine_uninstall_apply_failed",
            applied.stderr,
        ));
    }
    clear_installed_marker(
        std::path::Path::new(root),
        plan.get("installed_marker_sha256").and_then(Json::as_text),
    )?;
    let mut report = Report::ok(
        "ok",
        format!(
            "engine-owned uninstall completed; distribution approval {digest} bound the request"
        ),
    )
    .subject("uninstall");
    if let Some(block) = plan.get("owned_removals") {
        let removed = remove_owned(std::path::Path::new(root), block)?;
        for line in removed
            .get("lines")
            .and_then(Json::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(Json::as_text)
        {
            report.line(line.to_string());
        }
        report.detail("owned_removals", removed);
    }
    report.line("user data, workspaces and graph output were kept".to_string());
    Ok(report)
}

/// What uninstall removes beyond the engine plan: this layout's `bin` entries (only when their
/// bytes still match `installed.json`) and the PATH entry recorded in `path-change.json`.
fn owned_removals(root: &Path) -> Result<Option<Json>, Refusal> {
    let mut bin = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join(state::INSTALLED_FILE)) {
        if let Ok(record) = crate::update::json::parse(&text) {
            for item in record.get("bin").and_then(Json::as_array).unwrap_or(&[]) {
                if let (Some(path), Some(digest)) = (
                    item.get("path").and_then(Json::as_text),
                    item.get("sha256").and_then(Json::as_text),
                ) {
                    bin.push(Json::from_pairs(vec![
                        ("path", Json::text(path)),
                        ("sha256", Json::text(digest)),
                    ]));
                }
            }
        }
    }
    let path_record = root.join(crate::pathenv::RECORD_FILE);
    let path_entry = if path_record.is_file() {
        let text = std::fs::read_to_string(&path_record).map_err(|error| {
            Refusal::io(
                "path_record_unreadable",
                &path_record.display().to_string(),
                &error,
            )
        })?;
        crate::update::json::parse(&text)
            .ok()
            .and_then(|record| record.get("entry").cloned())
    } else {
        None
    };
    if bin.is_empty() && path_entry.is_none() {
        return Ok(None);
    }
    Ok(Some(Json::from_pairs(vec![
        ("bin", Json::array(bin)),
        ("path_entry", path_entry.unwrap_or(Json::Null)),
    ])))
}

/// Remove the owned `bin` entries and revert the recorded PATH change.
fn remove_owned(root: &Path, block: &Json) -> Result<Json, Refusal> {
    let mut lines = Vec::new();
    let mut removed = Vec::new();
    let running = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok());
    for item in block.get("bin").and_then(Json::as_array).unwrap_or(&[]) {
        let relative = item.get("path").and_then(Json::as_text).unwrap_or("");
        let expected = item.get("sha256").and_then(Json::as_text).unwrap_or("");
        if !crate::update::rules::is_relative_path(relative) || !relative.starts_with("bin/") {
            continue;
        }
        let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !path.is_file() {
            continue;
        }
        if crate::layout::file_digest(&path)? != expected {
            lines.push(format!("kept {} (changed since install)", path.display()));
            continue;
        }
        let is_running = running.is_some() && path.canonicalize().ok() == running;
        if std::fs::remove_file(&path).is_ok() {
            removed.push(Json::text(relative));
            continue;
        }
        if is_running {
            // Windows cannot delete a running executable; move it aside so the name is free and
            // the leftover is obviously stale. It is removed by the next install or by hand.
            let aside = path.with_extension("exe.uninstalled");
            if std::fs::rename(&path, &aside).is_ok() {
                lines.push(format!(
                    "{} is running; moved aside to {} (delete it after this command exits)",
                    path.display(),
                    aside.display()
                ));
                removed.push(Json::text(relative));
                continue;
            }
        }
        lines.push(format!("could not remove {}", path.display()));
    }
    let path_change = crate::pathenv::revert(root)?;
    if let Some(change) = &path_change {
        lines.push(format!(
            "PATH: {} ({})",
            change.get("action").and_then(Json::as_text).unwrap_or("?"),
            change
                .get("entry")
                .and_then(Json::as_text)
                .unwrap_or("recorded entry")
        ));
    }
    let bin = crate::layout::bin_dir(root);
    let _ = std::fs::remove_dir(&bin);
    lines.push(format!("removed {} owned executable(s)", removed.len()));
    Ok(Json::from_pairs(vec![
        ("bin_removed", Json::array(removed)),
        ("path_change", path_change.unwrap_or(Json::Null)),
        (
            "lines",
            Json::array(lines.iter().map(|line| Json::text(line)).collect()),
        ),
    ]))
}

/// The printed uninstall plan.
fn uninstall_lines(plan: &Json, root: &Path) -> Vec<String> {
    let mut lines = vec![format!("uninstall from {}", root.display())];
    if plan.get("installed").and_then(Json::as_bool) != Some(true) {
        lines.push("nothing is installed here".to_string());
    } else {
        lines.push(
            "remove: the installed ecosystem and its service registration (engine plan)"
                .to_string(),
        );
    }
    if let Some(block) = plan.get("owned_removals") {
        let count = block
            .get("bin")
            .and_then(Json::as_array)
            .map_or(0, <[Json]>::len);
        lines.push(format!(
            "remove: {count} executable(s) in {}",
            crate::layout::bin_dir(root).display()
        ));
        if let Some(entry) = block.get("path_entry").and_then(Json::as_text) {
            lines.push(format!("PATH: remove {entry} from the user PATH"));
        }
    }
    lines.push("keep: user data, workspaces and graph output".to_string());
    lines.push(format!(
        "plan digest: {}",
        plan.get("plan_digest")
            .and_then(Json::as_text)
            .unwrap_or("?")
    ));
    lines
}

fn clear_installed_marker(root: &std::path::Path, expected: Option<&str>) -> Result<(), Refusal> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let path = root.join("installed.json");
    if !path.exists() {
        return Ok(());
    }
    let bytes = std::fs::read(&path)
        .map_err(|e| Refusal::io("installed_marker_read", &path.display().to_string(), &e))?;
    if crate::update::sha256::hex(&crate::update::sha256::digest(&bytes)) != expected {
        return Err(Refusal::conflict(
            "installed_marker_changed",
            "installed marker changed after approval and was preserved",
        ));
    }
    std::fs::remove_file(path)
        .map_err(|e| Refusal::io("installed_marker_remove", "installed.json", &e))
}

fn engine_uninstall_plan(root: &std::path::Path) -> Result<Json, Refusal> {
    let engine = match engine::locate() {
        Located::Found(found) => found,
        Located::Missing(searched) => return Err(engine::missing_refusal(&searched)),
    };
    let path = root.join("state/engine-uninstall-preview.json");
    let outcome = engine.invoke_with_env(
        &[
            "uninstall".into(),
            "plan".into(),
            "--out".into(),
            path.display().to_string(),
        ],
        &[("AXIOM_HOME", root.display().to_string())],
    )?;
    if outcome.exit_code() != 0 {
        return Err(Refusal::not_ready(
            "engine_uninstall_plan_failed",
            outcome.stderr,
        ));
    }
    crate::update::json::parse(&std::fs::read_to_string(&path).map_err(|e| {
        Refusal::io(
            "engine_uninstall_plan_missing",
            &path.display().to_string(),
            &e,
        )
    })?)
    .map_err(|e| Refusal::validation("engine_uninstall_plan_invalid", e))
}

fn uninstall_plan(st: &state::State, purge_data: bool) -> Result<Json, Refusal> {
    let installed = st.has_installed();
    let mut pairs = vec![
        ("plan_version", Json::int(1)),
        ("verb", Json::text("uninstall")),
        (
            "host",
            match target::host_id() {
                Some(value) => Json::text(value),
                None => Json::null(),
            },
        ),
        ("install_root", Json::text(&st.root().display().to_string())),
        ("installed", Json::bool(installed)),
        ("purge_data", Json::bool(purge_data)),
        ("approval", approval_block()),
    ];
    if installed {
        let record = st.read_installed()?;
        pairs.push(("channel", Json::text(&record.channel)));
        pairs.push(("generation", Json::text(&record.current_generation)));
    }
    Ok(Json::from_pairs(pairs))
}

/// Resolve the release set: an explicit local set, else the recorded channel manifest.
fn resolve_release_set(
    from: Option<&str>,
    path_change: Option<bool>,
) -> Result<ReleaseSet, Refusal> {
    let host = target::host_id().ok_or_else(|| {
        Refusal::new(
            Class::Incompatible,
            "undeclared_target",
            format!(
                "this host ({}) is not a delivery platform the distribution contract declares, so \
                 the distribution refuses to install on it",
                target::host_description()
            ),
        )
    })?;

    let (loaded, local_root) = match from {
        Some(path) => {
            let path = Path::new(path);
            let manifest_path = if path.is_dir() {
                local_manifest_in(path).ok_or_else(|| {
                    Refusal::new(
                        Class::NotFound,
                        "release_set_manifest_missing",
                        format!(
                            "the release set {} has no channel manifest (expected channel.json or \
                             stable.json in that directory)",
                            path.display()
                        ),
                    )
                })?
            } else {
                path.to_path_buf()
            };
            let loaded = state::State::load_manifest_file(&manifest_path)?;
            let root = manifest_path.parent().map(Path::to_path_buf);
            (loaded, root)
        }
        None => {
            let root = state::default_root().ok_or_else(|| {
                Refusal::not_ready(
                    "no_install_root",
                    "this host offers no per-user install root, so no recorded channel manifest can be read",
                )
            })?;
            let st = state::State::new(root);
            let sibling = sibling_release_manifest();
            if let Some(sibling) =
                sibling.filter(|_| std::env::var_os(state::CHANNEL_MANIFEST_ENV).is_none())
            {
                // ADR-0033: an extracted release carries its channel manifest beside the
                // executable, so a first install (or a repair from that release) needs neither
                // `--from` nor an environment variable.
                let loaded = state::State::load_manifest_file(&sibling)?;
                (loaded, sibling.parent().map(Path::to_path_buf))
            } else if st.has_installed() {
                let record = st.read_installed()?;
                (st.load_recorded_manifest(&record)?, None)
            } else if let Ok(candidate) = std::env::var(state::CHANNEL_MANIFEST_ENV) {
                if candidate.trim().is_empty() {
                    return Err(no_release_set());
                }
                let loaded = state::State::load_manifest_file(Path::new(&candidate))?;
                let root = Path::new(&candidate).parent().map(Path::to_path_buf);
                (loaded, root)
            } else {
                return Err(no_release_set());
            }
        }
    };

    let mut plan = build_plan(host, &loaded.manifest, &loaded.path, &loaded.sha256)?;
    if let Some(inputs) = local_root.as_deref().and_then(crate::runtime::locate) {
        plan.set("mcp_runtime", crate::runtime::plan_block(&inputs)?)
            .map_err(|error| Refusal::validation("plan_unbuildable", error))?;
    }
    if let (Some(modify), Some(root)) = (path_change, state::default_root()) {
        plan.set(
            "path_change",
            crate::pathenv::plan_block(&crate::layout::bin_dir(&root), modify),
        )
        .map_err(|error| Refusal::validation("plan_unbuildable", error))?;
    }
    if let Some(root) = state::default_root() {
        let legacy: Vec<Json> = crate::legacy::detect(&root)
            .iter()
            .map(|layout| {
                let mut row = layout.to_json();
                let action = match (crate::legacy::same_path(&layout.path, &root), layout.kind) {
                    (true, "cli-store") => "adopt-in-place",
                    (true, "bootstrap-root") => "adopt-in-place",
                    (false, "cli-store" | "bootstrap-root") => "side-by-side",
                    _ => "report-only",
                };
                let _ = row.set("action", Json::text(action));
                row
            })
            .collect();
        if !legacy.is_empty() {
            plan.set("legacy", Json::array(legacy))
                .map_err(|error| Refusal::validation("plan_unbuildable", error))?;
        }
    }
    let digest = plan_contract::digest(&plan).ok_or_else(|| {
        Refusal::validation("plan_unbuildable", "the install plan could not be digested")
    })?;
    seal(&mut plan, &digest)?;
    Ok(ReleaseSet {
        manifest: loaded.manifest,
        manifest_sha256: loaded.sha256,
        manifest_source: loaded.path.display().to_string(),
        local_root,
        plan,
        plan_digest: digest,
    })
}

/// The channel manifest shipped beside this executable in an extracted release, if any.
///
/// Only an executable that is *not* the installed `bin/axiom-cli` qualifies: the installed copy
/// resolves from the recorded manifest instead.
fn sibling_release_manifest() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let directory = exe.parent()?;
    if directory.file_name().and_then(|n| n.to_str()) == Some(crate::layout::BIN_DIR)
        && directory
            .parent()
            .map(|p| p.join(state::INSTALLED_FILE).is_file())
            == Some(true)
    {
        return None;
    }
    let candidate = directory.join("channel.json");
    candidate.is_file().then_some(candidate)
}

fn no_release_set() -> Refusal {
    Refusal::not_ready(
        "no_release_set",
        "no release set is available: nothing is installed, no local release set was named with \
         `--from`, and no candidate channel manifest was named. The distribution resolves from a \
         recorded or explicit manifest only and never invents one",
    )
}

fn local_manifest_in(directory: &Path) -> Option<PathBuf> {
    for name in ["channel.json", "stable.json", "manifest.json"] {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Build the canonical install plan for one host.
fn build_plan(
    host: &str,
    manifest: &Manifest,
    manifest_path: &Path,
    manifest_sha256: &str,
) -> Result<Json, Refusal> {
    let root = state::default_root();
    let mut components: Vec<Json> = Vec::new();
    for component in &manifest.components {
        let artifact = component.artifact(
            host,
            target::platform(host)
                .map(|p| p.artifact_class)
                .unwrap_or(""),
        );
        components.push(Json::from_pairs(vec![
            ("component", Json::text(&component.component)),
            (
                "version",
                match &component.version {
                    Some(value) => Json::text(value),
                    None => Json::null(),
                },
            ),
            (
                "revision",
                match &component.revision {
                    Some(value) => Json::text(value),
                    None => Json::null(),
                },
            ),
            ("declared", Json::bool(component.declared)),
            ("needs_restart", Json::bool(component.needs_restart)),
            (
                "artifact",
                match artifact {
                    Some(artifact) => Json::from_pairs(vec![
                        ("platform", Json::text(&artifact.platform)),
                        ("class", Json::text(&artifact.class)),
                        ("url", Json::text(&artifact.url)),
                        ("sha256", Json::text(&artifact.sha256)),
                        ("size_bytes", Json::int(artifact.size_bytes)),
                    ]),
                    None => Json::null(),
                },
            ),
        ]));
    }
    Ok(Json::from_pairs(vec![
        ("plan_version", Json::int(1)),
        ("verb", Json::text("install")),
        ("host", Json::text(host)),
        (
            "install_root",
            match &root {
                Some(path) => Json::text(&path.display().to_string()),
                None => Json::null(),
            },
        ),
        ("channel", Json::text(&manifest.channel)),
        ("manifest", Json::text(&manifest_path.display().to_string())),
        ("manifest_sha256", Json::text(manifest_sha256)),
        ("published", Json::bool(manifest.published)),
        ("components", Json::array(components)),
        ("approval", approval_block()),
    ]))
}

fn approval_block() -> Json {
    Json::from_pairs(vec![
        ("state", Json::text("unapproved")),
        ("approve_digest", Json::null()),
    ])
}

/// Record a plan's own digest inside the plan document.
///
/// `plan_contract::approval_reasons` mirrors the canonical planner: it accepts an approval only
/// when the plan computes to the approved digest **and** the plan carries that digest in its own
/// `plan_digest` member. A plan this layer digests but never seals can therefore never be
/// approved - `--apply` would refuse every request, including a correct one, as
/// `plan_digest_not_approved`. Sealing is safe because `plan_digest` is excluded from the digest
/// body, so writing it does not change the digest it records.
fn seal(plan: &mut Json, digest: &str) -> Result<(), Refusal> {
    plan.set("plan_digest", Json::text(digest))
        .map_err(|error| {
            Refusal::validation(
                "plan_unbuildable",
                format!("the plan digest could not be recorded in the plan: {error}"),
            )
        })
}

/// Verify every artifact this host would receive.
///
/// A local release set resolves bytes from its own directory; otherwise the local artifact cache
/// is used. An artifact that is not available locally is a refusal, never a silent skip: an
/// unverified artifact must not be mistaken for a verified one.
fn verify_artifacts(set: &ReleaseSet) -> Result<Vec<Json>, Refusal> {
    let host = set.plan.get("host").and_then(Json::as_text).unwrap_or("");
    let Some(components) = set.plan.get("components").and_then(Json::as_array) else {
        return Ok(Vec::new());
    };
    let mut verified: Vec<Json> = Vec::new();
    for entry in components {
        let Some(object) = entry.as_object() else {
            continue;
        };
        let Some(artifact) = object.get("artifact").and_then(Json::as_object) else {
            continue;
        };
        let url = artifact.get("url").and_then(Json::as_text).unwrap_or("");
        let digest = artifact.get("sha256").and_then(Json::as_text).unwrap_or("");
        let size = artifact
            .get("size_bytes")
            .and_then(Json::as_int)
            .unwrap_or(0);
        let component = object
            .get("component")
            .and_then(Json::as_text)
            .unwrap_or("?");
        let path = resolve_local(set.local_root.as_deref(), url)
            .or_else(|| installed_payload(component))
            .ok_or_else(|| {
                Refusal::not_ready(
                    "artifact_unreachable",
                    format!(
                    "artifact for `{component}` ({url}) is not available locally for host {host}; \
                     this wave resolves artifacts from a local release set or the local artifact \
                     cache and never fetches from the network"
                ),
                )
            })?;
        fetch::verify_file(&path, digest, size).map_err(|refusal| {
            Refusal::validation(
                format!("artifact_unverified:{component}"),
                format!(
                    "artifact for `{component}` at {} failed verification: {}",
                    path.display(),
                    refusal.message
                ),
            )
        })?;
        verified.push(Json::from_pairs(vec![
            ("component", Json::text(component)),
            ("url", Json::text(url)),
            ("sha256", Json::text(digest)),
            ("size_bytes", Json::int(size)),
            ("resolved", Json::text(&path.display().to_string())),
            ("verified", Json::bool(true)),
        ]));
    }
    Ok(verified)
}

/// The verified payload of `component` in the active generation, so the installed
/// `bin/axiom-cli` can repair from the bytes it already recorded.
fn installed_payload(component: &str) -> Option<PathBuf> {
    let st = state::State::new(state::default_root()?);
    let installed = st.read_installed().ok()?;
    let generation =
        crate::update::generation::Generation::read(&st, &installed.current_generation).ok()?;
    let path =
        generation.payload_path(&st.generation_dir(&installed.current_generation), component)?;
    path.is_file().then_some(path)
}

/// Resolve one artifact URL to a local file: the release-set directory first, then the cache.
fn resolve_local(local_root: Option<&Path>, url: &str) -> Option<PathBuf> {
    let name = fetch::url_basename(url)?;
    if let Some(root) = local_root {
        let candidate = root.join(&name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let cache = fetch::cache_dir()?;
    let candidate = cache.join(&name);
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

fn plan_lines(set: &ReleaseSet) -> Vec<String> {
    let mut lines = Vec::new();
    let host = set.plan.get("host").and_then(Json::as_text).unwrap_or("?");
    lines.push(format!(
        "target: {host} (evidence target {})",
        target::evidence_target_for(host)
    ));
    lines.push(format!(
        "release set: {} (sha256 {})",
        set.manifest_source, set.manifest_sha256
    ));
    lines.push(format!(
        "channel: {} published={}",
        set.manifest.channel, set.manifest.published
    ));
    if let Some(components) = set.plan.get("components").and_then(Json::as_array) {
        for entry in components {
            let Some(object) = entry.as_object() else {
                continue;
            };
            let component = object
                .get("component")
                .and_then(Json::as_text)
                .unwrap_or("?");
            let version = object
                .get("version")
                .and_then(Json::as_text)
                .unwrap_or("undeclared");
            let artifact = if component == "skills" {
                "portable skills bundle (verified during install)"
            } else {
                object
                    .get("artifact")
                    .and_then(Json::as_object)
                    .map(|_| "artifact declared")
                    .unwrap_or("no artifact for this host")
            };
            lines.push(format!("  {component} {version} - {artifact}"));
        }
    }
    if let Some(runtime) = set.plan.get("mcp_runtime") {
        lines.push(format!(
            "  mcp runtime - provisioned from the release ({})",
            runtime.get("kind").and_then(Json::as_text).unwrap_or("?")
        ));
    }
    if let Some(root) = set.plan.get("install_root").and_then(Json::as_text) {
        lines.push(format!(
            "install root: {root} (bin: {root}{}bin)",
            std::path::MAIN_SEPARATOR
        ));
    }
    let bytes: i64 = set
        .plan
        .get("components")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(|item| item.get("artifact")?.get("size_bytes")?.as_int())
        .sum();
    lines.push(format!(
        "size: {:.1} MB of verified artifacts",
        bytes as f64 / 1_000_000.0
    ));
    if let Some(block) = set.plan.get("path_change") {
        lines.push(crate::pathenv::plan_line(block));
    }
    for row in set
        .plan
        .get("legacy")
        .and_then(Json::as_array)
        .unwrap_or(&[])
    {
        let kind = row.get("kind").and_then(Json::as_text).unwrap_or("?");
        let path = row.get("path").and_then(Json::as_text).unwrap_or("?");
        let version = row
            .get("version")
            .and_then(Json::as_text)
            .unwrap_or("unknown");
        lines.push(match row.get("action").and_then(Json::as_text) {
            Some("adopt-in-place") if kind == "cli-store" => format!(
                "legacy: adopt the {version} CLI store at {path} in place (its axiom-cli is backed up under legacy\\, cli\\ and install-manifest.json are kept)"
            ),
            Some("adopt-in-place") => format!(
                "legacy: adopt the {version} bootstrap root at {path} in place (engine ecosystem and MCP runtime reused, nothing reinstalled)"
            ),
            Some("side-by-side") => format!(
                "legacy: {kind} {version} at {path} is left untouched (side by side); to adopt it instead run `axiom-cli install --adopt {path}`"
            ),
            _ => format!("legacy: unrecognised Axiom-like directory {path}; reported, not modified"),
        });
    }
    if let Some(root) = set.plan.get("install_root").and_then(Json::as_text) {
        for shadow in crate::legacy::shadows(&crate::layout::bin_dir(Path::new(root))) {
            if shadow.before_root_bin {
                lines.push(format!(
                    "warning: {} ({}) comes earlier on PATH and will shadow the new axiom-cli until it is removed",
                    shadow.path.display(),
                    shadow.version
                ));
            }
        }
    }
    lines.push(format!("plan digest: {}", set.plan_digest));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::json::Json;

    /// A minimal plan body shaped like the distribution plan, without a digest member.
    fn plan_body() -> Json {
        Json::from_pairs(vec![
            ("plan_version", Json::int(1)),
            ("verb", Json::text("install")),
            ("host", Json::text("macos-x64")),
            ("install_root", Json::text("/fixture/root")),
        ])
    }

    #[test]
    fn sealing_records_the_digest_without_changing_it() {
        let mut plan = plan_body();
        let before = plan_contract::digest(&plan).expect("the plan must digest");
        seal(&mut plan, &before).expect("sealing must succeed");
        assert_eq!(
            plan.get("plan_digest").and_then(Json::as_text),
            Some(before.as_str())
        );
        assert_eq!(
            plan_contract::digest(&plan).expect("the sealed plan must digest"),
            before,
            "`plan_digest` is excluded from the digest body, so sealing must not change the digest"
        );
    }

    #[test]
    fn an_unsealed_plan_can_never_be_approved() {
        // This is the regression guard for the defect this module fixed: the canonical approval
        // boundary requires the plan to carry its own digest, so a plan that is digested but not
        // sealed is refused even when the caller passes the exactly-correct digest.
        let plan = plan_body();
        let digest = plan_contract::digest(&plan).expect("the plan must digest");
        assert_eq!(
            plan_contract::approval_reasons(&plan, &digest),
            vec!["plan_digest_not_approved".to_string()]
        );
    }

    #[test]
    fn a_sealed_plan_is_approved_by_its_own_digest() {
        let mut plan = plan_body();
        let digest = plan_contract::digest(&plan).expect("the plan must digest");
        seal(&mut plan, &digest).expect("sealing must succeed");
        assert!(
            plan_contract::approval_reasons(&plan, &digest).is_empty(),
            "a sealed plan must be accepted by the approval boundary"
        );
    }

    #[test]
    fn a_sealed_plan_still_refuses_a_wrong_digest() {
        let mut plan = plan_body();
        let digest = plan_contract::digest(&plan).expect("the plan must digest");
        seal(&mut plan, &digest).expect("sealing must succeed");
        let wrong = "b".repeat(64);
        let reasons = plan_contract::approval_reasons(&plan, &wrong);
        assert!(
            reasons.contains(&"approval_stale".to_string()),
            "a wrong approval must stay stale: {reasons:?}"
        );
    }

    #[test]
    fn the_uninstall_plan_carries_no_volatile_timestamp() {
        // A plan whose digest body contains the observation time can never be re-derived on a
        // later `--apply`, so it could never be approved. The plan must be a pure function of its
        // decision inputs; the observation time belongs on the report, not in the plan.
        let state = state::State::new(PathBuf::from("/fixture/root"));
        let plan = uninstall_plan(&state, false).expect("the plan must build");
        assert!(
            plan.get("generated_at").is_none(),
            "the plan body must not carry a volatile timestamp: {plan:?}"
        );
        assert!(plan.get("created_at").is_none());
    }

    #[test]
    fn embedded_engine_plan_changes_outer_digest() {
        let mut first = plan_body();
        first
            .set(
                "engine_plan",
                Json::from_pairs(vec![
                    ("plan_digest", Json::text(&"a".repeat(64))),
                    ("owned", Json::text("one")),
                ]),
            )
            .unwrap();
        let mut second = first.clone();
        second
            .set(
                "engine_plan",
                Json::from_pairs(vec![
                    ("plan_digest", Json::text(&"b".repeat(64))),
                    ("owned", Json::text("two")),
                ]),
            )
            .unwrap();
        assert_ne!(
            plan_contract::digest(&first),
            plan_contract::digest(&second)
        );
    }

    #[test]
    fn marker_cleanup_only_removes_approved_bytes() {
        let temp = std::env::temp_dir().join(format!("axiom-cli-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        let path = temp.join("installed.json");
        std::fs::write(&path, b"owned").unwrap();
        let digest = crate::update::sha256::hex(&crate::update::sha256::digest(b"owned"));
        clear_installed_marker(&temp, Some(&digest)).unwrap();
        assert!(!path.exists());
        std::fs::write(&path, b"edited").unwrap();
        assert!(clear_installed_marker(&temp, Some(&digest)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"edited");
        std::fs::remove_file(&path).unwrap();
        clear_installed_marker(&temp, None).unwrap();
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn install_prefers_the_release_engine_over_host_discovery() {
        let executable = std::env::current_exe().unwrap();
        let verified = vec![Json::from_pairs(vec![
            ("component", Json::text("axiom")),
            ("resolved", Json::text(&executable.display().to_string())),
            ("verified", Json::bool(true)),
        ])];
        let selected = install_engine(&verified).unwrap();
        assert_eq!(selected.program(), executable);
        assert_eq!(selected.source(), "release_set");
    }

    #[test]
    fn declared_unusable_engine_does_not_fall_back_to_path() {
        let verified = vec![Json::from_pairs(vec![
            ("component", Json::text("axiom")),
            (
                "resolved",
                Json::text(&std::env::temp_dir().display().to_string()),
            ),
            ("verified", Json::bool(true)),
        ])];
        let refusal = install_engine(&verified).unwrap_err();
        assert_eq!(refusal.reason, "engine_artifact_not_executable");
    }

    #[test]
    fn provisioned_runtime_is_verified_before_its_bin_reaches_the_engine() {
        let root = std::env::temp_dir().join(format!(
            "axiom-cli-mcp-runtime-{}-{}",
            std::process::id(),
            crate::update::time::Stamp::now().format().replace(':', "")
        ));
        let version = root.join("mcp-runtime/versions/0123456789abcdef01234567");
        let bin = version
            .join("venv")
            .join(if cfg!(windows) { "Scripts" } else { "bin" });
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(version.join("owner.json"), b"{\"owner\":\"axiom-cli\"}\n").unwrap();
        let python_name = if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        };
        let launcher_name = if cfg!(windows) {
            "axiom-mcp.exe"
        } else {
            "axiom-mcp"
        };
        std::fs::write(bin.join(python_name), b"owned python").unwrap();
        std::fs::write(bin.join(launcher_name), b"owned launcher").unwrap();
        let python = crate::update::sha256::file_hex(&bin.join(python_name)).unwrap();
        let executable = crate::update::sha256::file_hex(&bin.join(launcher_name)).unwrap();
        let pointer = format!(
            "{{\"generation\":\"0123456789abcdef01234567\",\"python_sha256\":\"{python}\",\"executable_sha256\":\"{executable}\"}}\n"
        );
        std::fs::write(root.join("mcp-runtime/current.json"), pointer).unwrap();
        assert_eq!(
            provisioned_mcp_runtime_bin(&root).unwrap(),
            Some(bin.clone())
        );
        let env = crate::bundle::engine_environment(&root, Some(&bin));
        assert_eq!(env[0], ("AXIOM_HOME", root.display().to_string()));
        let runtime_path = env
            .iter()
            .find(|(key, _)| *key == "PATH")
            .unwrap()
            .1
            .as_str();
        assert!(runtime_path.starts_with(&bin.display().to_string()));
        if cfg!(windows) {
            assert!(runtime_path.contains(';'));
            assert!(runtime_path.contains("System32"));
        }
        std::fs::write(bin.join(launcher_name), b"tampered").unwrap();
        assert_eq!(
            provisioned_mcp_runtime_bin(&root).unwrap_err().reason,
            "mcp_runtime_executable_changed"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
