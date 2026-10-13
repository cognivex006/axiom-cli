//! `axiom-cli version` - report installed and available versions.
//!
//! Owner: `axiom-cli`. The distribution contract (section 2) requires this verb to "report
//! installed and available versions per component and per target". The data it reports is
//! owned by this repository:
//!
//! * the running CLI's own version, from `Cargo.toml`/`VERSION`;
//! * the active generation, read from the install root this CLI owns; and
//! * the available versions, resolved **only** from the channel manifest the installed release
//!   records, or from a candidate manifest an operator names explicitly.
//!
//! Nothing here invents a version. A component whose owning repository declared a version that
//! is not SemVer is reported as `undeclared` with the source of the declaration, exactly as the
//! channel module records it. When no manifest is reachable at all, the available set is
//! reported as `unresolved` with the reason instead of being guessed.
//!
//! Unlike `update check`, a successful `version` is not contingent on anything being installed:
//! "nothing is installed" is a fact this verb exists to report, so it is data and exits `0`.

use crate::engine::{self, Located};
use crate::target;
use crate::update::channel::{self, Manifest};
use crate::update::error::{Class, Refusal};
use crate::update::generation::Generation;
use crate::update::json::Json;
use crate::update::report::Report;
use crate::update::state::{self, State};

/// Run the `version` verb and return its process exit code.
pub fn run_version(all: bool, json: bool, verbose: bool) -> i32 {
    match collect(all, !json && !verbose && !all) {
        Ok(report) => report.emit(json, verbose),
        Err(refusal) => refusal_report(&refusal).emit(json, verbose),
    }
}

fn refusal_report(refusal: &Refusal) -> Report {
    let mut report = match refusal.class {
        Class::NotReady => Report::not_ready(refusal.message.clone()),
        Class::Validation => Report::refused(refusal.message.clone()),
        other => Report::new(other.exit_code(), other.status(), refusal.message.clone()),
    }
    .subject("version");
    report.detail("reason", Json::text(&refusal.message));
    report.detail("reason_code", Json::text(&refusal.reason));
    if refusal.class.retryable() {
        report = report.retryable();
    }
    report
}

fn collect(all: bool, concise: bool) -> Result<Report, Refusal> {
    let mut report =
        Report::ok("ok", "reported installed and available versions").subject("version");
    report.detail("cli", cli_block());
    report.line(format!(
        "Axiom CLI {} (axm / axiom-cli)",
        crate::cli::VERSION
    ));
    report.detail("host", host_block());
    report.detail("all", Json::bool(all));

    let root = state::default_root();
    match &root {
        Some(path) => {
            report.detail("install_root", Json::text(&path.display().to_string()));
        }
        None => {
            report.detail("install_root", Json::null());
        }
    }

    // The active generation and the manifest it came from. Both are optional: a host with
    // nothing installed still gets a complete, honest report.
    let mut available: Option<Manifest> = None;
    let mut available_source = "unresolved".to_string();
    let mut available_note =
        "no installed release records a channel manifest and no candidate manifest was named"
            .to_string();

    match &root {
        None => {
            report.line("install root: unavailable on this host");
            report.detail("installed", Json::bool(false));
            report.detail(
                "install_root_reason",
                Json::text(
                    "this host offers no per-user data directory, so no install root can be read",
                ),
            );
        }
        Some(path) => {
            let state = State::new(path.clone());
            if !state.has_installed() {
                report.line(format!(
                    "installed: no (nothing recorded at {})",
                    state.installed_path().display()
                ));
                report.detail("installed", Json::bool(false));
            } else {
                let installed = state.read_installed()?;
                let generation = Generation::read(&state, &installed.current_generation)?;
                let loaded = state.load_recorded_manifest(&installed)?;
                available = Some(loaded.manifest.clone());
                available_source = format!("recorded:{}", loaded.path.display());
                available_note =
                    "resolved from the channel manifest the installed release records".to_string();
                report.detail(
                    "installed",
                    installed_block(&installed, &generation, &loaded.manifest, all),
                );
                for line in installed_lines(&installed, &generation, concise) {
                    report.line(line);
                }
            }
        }
    }

    // A candidate manifest may be named explicitly while nothing is installed, which is how an
    // operator previews a channel before the first install. It never overrides a recorded
    // manifest.
    if available.is_none() {
        if let Ok(candidate) = std::env::var(state::CHANNEL_MANIFEST_ENV) {
            if !candidate.trim().is_empty() {
                let loaded = State::load_manifest_file(std::path::Path::new(&candidate))?;
                available = Some(loaded.manifest.clone());
                available_source =
                    format!("{}:{}", state::CHANNEL_MANIFEST_ENV, loaded.path.display());
                available_note =
                    "resolved from the candidate manifest named on this invocation".to_string();
            }
        }
    }

    match &available {
        Some(manifest) => {
            report.detail("available_source", Json::text(&available_source));
            report.detail("available_note", Json::text(&available_note));
            report.detail("available", manifest_block(manifest, all));
            if concise {
                report.line(format!(
                    "Recorded channel: {} (use axm update check for newer releases)",
                    manifest.channel
                ));
            } else {
                for line in manifest_lines(manifest, all) {
                    report.line(line);
                }
            }
        }
        None => {
            report.detail("available_source", Json::text(&available_source));
            report.detail("available_note", Json::text(&available_note));
            report.detail("available", Json::null());
            if !concise {
                report.line(format!("available: unresolved ({available_note})"));
            }
        }
    }

    report.detail("engine", engine_block());
    Ok(report)
}

fn cli_block() -> Json {
    Json::from_pairs(vec![
        ("program", Json::text(crate::cli::PROGRAM)),
        ("version", Json::text(crate::cli::VERSION)),
        ("engine_owner", Json::text(crate::cli::ENGINE_OWNER)),
    ])
}

fn host_block() -> Json {
    let id = target::host_id();
    let mut pairs = vec![
        ("host", Json::text(&target::host_description())),
        (
            "target",
            match id {
                Some(value) => Json::text(value),
                None => Json::null(),
            },
        ),
    ];
    match id.and_then(target::platform) {
        Some(platform) => {
            pairs.push(("os", Json::text(platform.os)));
            pairs.push(("arch", Json::text(platform.arch)));
            pairs.push(("artifact_class", Json::text(platform.artifact_class)));
        }
        None => {
            pairs.push(("declared", Json::bool(false)));
        }
    }
    if let Some(value) = id {
        if let Some(tier) = target::tier(value) {
            pairs.push(("tier", Json::text(tier.name())));
        }
        pairs.push((
            "evidence_target",
            Json::text(target::evidence_target_for(value)),
        ));
    }
    Json::from_pairs(pairs)
}

fn installed_block(
    installed: &state::Installed,
    generation: &Generation,
    manifest: &Manifest,
    all: bool,
) -> Json {
    Json::from_pairs(vec![
        ("channel", Json::text(&installed.channel)),
        ("generation", Json::text(&installed.current_generation)),
        (
            "previous_generation",
            match &installed.previous_generation {
                Some(value) => Json::text(value),
                None => Json::null(),
            },
        ),
        ("installed_at", Json::text(&installed.installed_at.format())),
        (
            "channel_manifest_sha256",
            Json::text(&installed.channel_manifest_sha256),
        ),
        ("recorded_manifest_verified", Json::bool(true)),
        (
            "components",
            installed_components(generation, manifest, all),
        ),
    ])
}

fn installed_components(generation: &Generation, manifest: &Manifest, all: bool) -> Json {
    let mut items: Vec<Json> = Vec::new();
    for entry in &generation.entries {
        let available = manifest.component(&entry.component);
        let mut pairs = vec![
            ("component", Json::text(&entry.component)),
            ("action", Json::text(&entry.action)),
            ("version", Json::text(&entry.version)),
            ("revision", Json::text(&entry.revision)),
            ("artifact_sha256", Json::text(&entry.artifact_sha256)),
            ("needs_restart", Json::bool(entry.needs_restart)),
        ];
        pairs.push(("available", available_block(available)));
        items.push(Json::from_pairs(pairs));
    }
    if all {
        for component in &manifest.components {
            if generation
                .entries
                .iter()
                .any(|entry| entry.component == component.component)
            {
                continue;
            }
            items.push(Json::from_pairs(vec![
                ("component", Json::text(&component.component)),
                ("action", Json::text("absent")),
                ("version", Json::null()),
                ("revision", Json::null()),
                ("artifact_sha256", Json::null()),
                ("needs_restart", Json::bool(component.needs_restart)),
                ("available", available_block(Some(component))),
            ]));
        }
    }
    Json::array(items)
}

fn manifest_block(manifest: &Manifest, all: bool) -> Json {
    let mut items: Vec<Json> = Vec::new();
    for component in &manifest.components {
        if !all && !component.declared {
            continue;
        }
        items.push(Json::from_pairs(vec![
            ("component", Json::text(&component.component)),
            ("available", available_block(Some(component))),
            ("needs_restart", Json::bool(component.needs_restart)),
        ]));
    }
    Json::from_pairs(vec![
        ("channel", Json::text(&manifest.channel)),
        ("published", Json::bool(manifest.published)),
        ("updated_at", Json::text(&manifest.updated_at.format())),
        ("components", Json::array(items)),
    ])
}

fn available_block(component: Option<&channel::Component>) -> Json {
    match component {
        None => Json::from_pairs(vec![
            ("status", Json::text("not_in_channel")),
            ("version", Json::null()),
            ("revision", Json::null()),
        ]),
        Some(component) if component.declared => Json::from_pairs(vec![
            ("status", Json::text("declared")),
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
        ]),
        Some(component) => Json::from_pairs(vec![
            ("status", Json::text("undeclared")),
            ("version", Json::null()),
            ("revision", Json::null()),
            ("declared_source", Json::text(&component.declared_source)),
        ]),
    }
}

/// The one release version every recorded component shares, if they agree.
fn release_version(generation: &Generation) -> Option<&str> {
    if let Some(core) = generation
        .entries
        .iter()
        .find(|entry| entry.component == "axiom-graphd")
    {
        return Some(core.version.as_str());
    }
    let first = generation.entries.first()?.version.as_str();
    generation
        .entries
        .iter()
        .all(|entry| entry.version == first)
        .then_some(first)
}

fn installed_lines(
    installed: &state::Installed,
    generation: &Generation,
    concise: bool,
) -> Vec<String> {
    if concise {
        let mut lines = vec![format!(
            "installed: {}",
            release_version(generation).unwrap_or("mixed")
        )];
        for (component, label) in [
            ("axiom-graphd", "Graph engine (axiom / axiom-graphd)"),
            ("axiom-mcp", "MCP"),
            ("skills", "Skills"),
        ] {
            let version = generation
                .entries
                .iter()
                .find(|entry| entry.component == component)
                .map(|entry| entry.version.as_str())
                .unwrap_or("not recorded");
            lines.push(format!("{label}: {version}"));
        }
        return lines;
    }
    let mut lines = vec![
        format!(
            "installed: {} channel={} generation={} at={}",
            release_version(generation).unwrap_or("mixed"),
            installed.channel,
            installed.current_generation,
            installed.installed_at.format()
        ),
        format!(
            "previous generation: {}",
            installed
                .previous_generation
                .as_deref()
                .unwrap_or("none retained")
        ),
    ];
    for entry in &generation.entries {
        lines.push(format!(
            "  {} {} version={} action={}{}",
            entry.component,
            entry.revision.chars().take(12).collect::<String>(),
            entry.version,
            entry.action,
            if entry.needs_restart {
                " needs_restart"
            } else {
                ""
            }
        ));
    }
    lines
}

fn manifest_lines(manifest: &Manifest, all: bool) -> Vec<String> {
    let mut lines = vec![format!(
        "available: channel={} published={} updated_at={}",
        manifest.channel,
        manifest.published,
        manifest.updated_at.format()
    )];
    for component in &manifest.components {
        if !all && !component.declared {
            continue;
        }
        let version = component.version.as_deref().unwrap_or("undeclared");
        lines.push(format!(
            "  {} available={} status={}",
            component.component,
            version,
            if component.declared {
                "declared"
            } else {
                "undeclared"
            }
        ));
    }
    lines
}

fn engine_block() -> Json {
    match engine::locate() {
        Located::Found(found) => {
            let mut pairs = vec![
                ("found", Json::bool(true)),
                (
                    "program",
                    Json::text(&found.program().display().to_string()),
                ),
                ("source", Json::text(found.source())),
            ];
            match found.invoke(&[
                "version".to_string(),
                "--all".to_string(),
                "--json".to_string(),
            ]) {
                Ok(outcome) if outcome.exit_code() == 0 => {
                    match crate::update::json::parse(outcome.stdout.trim()) {
                        Ok(value) => pairs.push(("report", value)),
                        Err(error) => {
                            pairs.push(("report", Json::null()));
                            pairs.push(("report_error", Json::text(&error)));
                        }
                    }
                }
                Ok(outcome) => {
                    pairs.push(("report", Json::null()));
                    pairs.push((
                        "report_error",
                        Json::text(&format!(
                            "the engine exited {}: {}",
                            outcome.exit_code(),
                            outcome.stderr.trim()
                        )),
                    ));
                }
                Err(refusal) => {
                    pairs.push(("report", Json::null()));
                    pairs.push(("report_error", Json::text(&refusal.message)));
                }
            }
            Json::from_pairs(pairs)
        }
        Located::Missing(searched) => Json::from_pairs(vec![
            ("found", Json::bool(false)),
            (
                "searched",
                Json::array(searched.iter().map(|item| Json::text(&item.path)).collect()),
            ),
        ]),
    }
}
