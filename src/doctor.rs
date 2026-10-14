//! `axm doctor` - report prerequisites, installed versions, digests and target state.
//!
//! The distribution contract (section 2) fixes this verb's purpose and section 7 fixes the
//! dependency table this verb reports against. `doctor` is the distribution layer's own health
//! report: it never installs, repairs or removes anything, and it never reports a check it did
//! not run as a pass.
//!
//! Ownership is respected in both directions:
//!
//! * The prerequisite probe owned by `axiom-graphd` is *delegated*: when the engine binary is
//!   present its own `version` report is included, and the absence of the engine is reported as
//!   an unverified engine-owned check rather than as a local failure. This module does not
//!   re-implement the engine's host probe.
//! * The distribution-owned checks *are* performed here, because they are this repository's:
//!   the install root, the recorded channel manifest digest, the active generation's payload
//!   digests, the recovery journal, the coordinator lock and the delivery target.
//!
//! A check has three states - `ok`, `unverified` and `fail` - and the exit code follows the
//! worst one: a failed integrity check keeps its canonical class, an unverified required check
//! is `not ready/stale` (4), and an all-clear report exits `0`.

use crate::engine::{self, Located};
use crate::target;
use crate::update::error::Refusal;
use crate::update::generation::Generation;
use crate::update::json::Json;
use crate::update::report::Report;
use crate::update::state::{self, State};

/// Run the `doctor` verb and return its process exit code.
pub fn run_doctor(all: bool, json: bool, verbose: bool) -> i32 {
    let mut worst: i32 = crate::cli::exit::SUCCESS;
    let mut findings: Vec<Json> = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    findings.push(host_finding(&mut lines));

    // A failed integrity check keeps its canonical class; it is not folded into "not ready".
    let mut integrity_failure: Option<(i32, String, String)> = None;
    match &state::default_root() {
        None => {
            findings.push(finding(
                "install_root",
                "fail",
                "this host offers no per-user data directory, so no install root can be read",
                Vec::new(),
            ));
            lines.push("install root: unavailable".to_string());
            worst = worst.max(crate::cli::exit::NOT_READY);
        }
        Some(path) => {
            let state = State::new(path.clone());
            findings.push(finding(
                "install_root",
                "ok",
                &format!("install root {}", path.display()),
                vec![("path", Json::text(&path.display().to_string()))],
            ));
            lines.push(format!("install root: {}", path.display()));

            if state.has_installed() {
                match inspect_installed(&state, &mut lines) {
                    Ok(value) => findings.push(value),
                    Err((code, reason, message)) => {
                        findings.push(finding(
                            "installed_release",
                            "fail",
                            &message,
                            vec![("reason", Json::text(&message))],
                        ));
                        lines.push(format!("installed release: FAIL - {message}"));
                        integrity_failure = Some((code, reason, message));
                    }
                }
            } else {
                findings.push(finding(
                    "installed_release",
                    "ok",
                    "no installed release is recorded; the host is clean",
                    vec![("installed", Json::bool(false))],
                ));
                lines.push("installed release: none (clean host)".to_string());
            }
        }
    }

    // The distribution-owned prerequisite table from the contract's section 7.
    let installed_root =
        state::default_root().filter(|root| State::new(root.clone()).has_installed());
    let (prerequisite_finding, prerequisite_lines, prerequisite_worst) =
        prerequisites(all, installed_root.as_deref());
    findings.push(prerequisite_finding);
    lines.extend(prerequisite_lines);
    worst = worst.max(prerequisite_worst);

    findings.push(engine_finding(&mut lines, &mut worst));
    findings.push(legacy_finding(&mut lines));

    if let Some((code, reason, message)) = integrity_failure {
        let mut failure = Report::new(code, class_status(code), message).subject("doctor");
        failure.detail("reason_code", Json::text(&reason));
        failure.detail("findings", Json::array(findings.clone()));
        failure.lines(lines);
        return emit_doctor(failure, &findings, json, verbose);
    }

    if worst != crate::cli::exit::SUCCESS {
        let mut failure = Report::not_ready(
            "one or more required checks could not be verified; see `findings` for the exact item \
             and the plain output for the missing prerequisite",
        )
        .subject("doctor")
        .retryable();
        failure.detail("worst_code", Json::int(i64::from(worst)));
        failure.detail("findings", Json::array(findings.clone()));
        failure.lines(lines);
        return emit_doctor(failure, &findings, json, verbose);
    }

    let mut report = Report::ok(
        "ok",
        "installation checks passed; project and host setup are not checked",
    )
    .subject("doctor");
    report.detail("all", Json::bool(all));
    report.detail("findings", Json::array(findings.clone()));
    report.lines(lines);
    emit_doctor(report, &findings, json, verbose)
}

fn emit_doctor(report: Report, findings: &[Json], json: bool, verbose: bool) -> i32 {
    if json || verbose {
        return report.emit(json, verbose);
    }
    let code = report.code();
    for line in summary_lines(findings, code) {
        if code == 0 {
            println!("{line}");
        } else {
            eprintln!("{line}");
        }
    }
    code
}

/// Present only what a user needs to know; technical evidence stays in verbose/JSON.
fn summary_lines(findings: &[Json], code: i32) -> Vec<String> {
    let mut lines = vec![format!(
        "Axiom installation: {}",
        if code == 0 {
            "OK"
        } else if code == crate::cli::exit::NOT_READY {
            "NotReady - needs attention"
        } else {
            "needs attention"
        }
    )];
    let mut flat = Vec::new();
    for row in findings {
        if row.get("check").and_then(Json::as_text) == Some("prerequisites") {
            if let Some(items) = row.get("items").and_then(Json::as_array) {
                flat.extend(items.iter());
            }
        } else {
            flat.push(row);
        }
    }
    for row in flat {
        let check = row.get("check").and_then(Json::as_text).unwrap_or("");
        let status = row.get("status").and_then(Json::as_text).unwrap_or("");
        let message = row
            .get("message")
            .and_then(Json::as_text)
            .unwrap_or("check unavailable");
        if check == "installed_release"
            && row.get("installed").and_then(Json::as_bool) == Some(false)
        {
            lines[0] = if code == crate::cli::exit::NOT_READY {
                "Axiom: NotReady - not installed"
            } else {
                "Axiom: not installed"
            }
            .to_string();
            lines.push("Next: axm install".to_string());
        } else if status == "fail" {
            match check {
                "engine" => lines.push(
                    "Engine: unavailable. Reinstall Axiom using the install URL.".to_string(),
                ),
                "python-interpreter" => lines.push(
                    "MCP Python: missing or incompatible. Reinstall Axiom using the install URL."
                        .to_string(),
                ),
                "installed_release" => lines.push(format!(
                    "Installation: {message}. Restore it using the install URL."
                )),
                _ => lines.push(format!("{check}: {message}")),
            }
        } else if status == "unverified" {
            lines.push(if check == "sqlite-driver" {
                "Note: bundled SQLite version is unverified (non-blocking).".to_string()
            } else {
                format!("Unverified: {check}. {message}")
            });
        } else if status == "warn" {
            lines.push(
                "Warning: an older install, PATH conflict or leftover AXIOM_* setting was found."
                    .to_string(),
            );
        } else if check == "installed_release" {
            if row
                .get("journal_entries")
                .and_then(Json::as_int)
                .unwrap_or(0)
                > 0
            {
                lines.push(
                    "Warning: an interrupted transaction needs inspection; see --verbose."
                        .to_string(),
                );
            }
            if row.get("lock_held").and_then(Json::as_bool) == Some(true) {
                lines.push(
                    "Warning: another installation operation may be running; see --verbose."
                        .to_string(),
                );
            }
        }
    }
    lines.push("Project and AI-host setup: not checked.".to_string());
    lines.push("Details: axm doctor --verbose".to_string());
    lines
}

/// L-006: name every legacy layout, every `axiom-cli` that shadows this root's `bin` on PATH and
/// every leftover `AXIOM_*` variable. Informational: it never changes the exit code and never
/// modifies anything.
fn legacy_finding(lines: &mut Vec<String>) -> Json {
    let Some(root) = state::default_root() else {
        return finding(
            "legacy",
            "ok",
            "no install root to compare against",
            Vec::new(),
        );
    };
    let layouts = crate::legacy::detect(&root);
    let shadows = crate::legacy::shadows(&crate::layout::bin_dir(&root));
    let env = crate::legacy::leftover_env();
    for layout in &layouts {
        lines.push(format!("legacy layout: {}", layout.describe()));
    }
    for shadow in &shadows {
        lines.push(format!(
            "axm on PATH: {} ({}){}",
            shadow.path.display(),
            shadow.version,
            if shadow.before_root_bin {
                " - shadows this install"
            } else {
                ""
            }
        ));
    }
    for (name, value) in &env {
        lines.push(format!(
            "leftover environment: {name}={value} (not needed by a current installation)"
        ));
    }
    let clean = layouts.is_empty() && shadows.iter().all(|s| !s.before_root_bin) && env.is_empty();
    finding(
        "legacy",
        if clean { "ok" } else { "warn" },
        if clean {
            "no legacy layout, PATH shadowing or leftover environment"
        } else {
            "legacy installations or leftovers were found; see the lines above (nothing was changed)"
        },
        vec![
            (
                "layouts",
                Json::array(layouts.iter().map(crate::legacy::Layout::to_json).collect()),
            ),
            (
                "path_shadowing",
                Json::array(
                    shadows
                        .iter()
                        .map(|shadow| {
                            Json::from_pairs(vec![
                                ("path", Json::text(&shadow.path.display().to_string())),
                                ("version", Json::text(&shadow.version)),
                                ("before_this_install", Json::bool(shadow.before_root_bin)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "environment",
                Json::array(
                    env.iter()
                        .map(|(name, value)| {
                            Json::from_pairs(vec![
                                ("name", Json::text(name)),
                                ("value", Json::text(value)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ],
    )
}

fn class_status(code: i32) -> &'static str {
    match code {
        c if c == crate::cli::exit::NOT_FOUND => "not_found",
        c if c == crate::cli::exit::NOT_READY => "not_ready",
        c if c == crate::cli::exit::CONFLICT => "conflict",
        c if c == crate::cli::exit::VALIDATION => "validation_error",
        c if c == crate::cli::exit::INCOMPATIBLE => "incompatible",
        c if c == crate::cli::exit::LOCK_UNAVAILABLE => "lock_unavailable",
        _ => "io_error",
    }
}

fn finding(check: &str, status: &str, message: &str, details: Vec<(&str, Json)>) -> Json {
    let mut pairs = vec![
        ("check", Json::text(check)),
        ("status", Json::text(status)),
        ("message", Json::text(message)),
    ];
    for (key, value) in details {
        pairs.push((key, value));
    }
    Json::from_pairs(pairs)
}

fn host_finding(lines: &mut Vec<String>) -> Json {
    let id = target::host_id();
    let mut pairs = vec![("host", Json::text(&target::host_description()))];
    match id {
        None => {
            pairs.push(("target", Json::null()));
            lines.push(format!(
                "target: undeclared ({} is not a delivery platform of the contract)",
                target::host_description()
            ));
            finding(
                "target",
                "fail",
                "this host is not a delivery platform the contract declares",
                pairs,
            )
        }
        Some(value) => {
            pairs.push(("target", Json::text(value)));
            pairs.push((
                "evidence_target",
                Json::text(target::evidence_target_for(value)),
            ));
            match target::tier(value) {
                Some(tier) => {
                    pairs.push(("tier", Json::text(tier.name())));
                    lines.push(format!(
                        "target: {} tier={} evidence_target={}",
                        value,
                        tier.name(),
                        target::evidence_target_for(value)
                    ));
                }
                None => lines.push(format!("target: {value} (no tier recorded)")),
            }
            finding(
                "target",
                "ok",
                "the running host is a declared delivery platform",
                pairs,
            )
        }
    }
}

/// Inspect the recorded release: manifest digest, active generation payload digests, journal
/// and lock. Returns a `fail` finding, or the tuple the caller turns into a refusal.
fn inspect_installed(
    state: &State,
    lines: &mut Vec<String>,
) -> Result<Json, (i32, String, String)> {
    let installed = state.read_installed().map_err(classify_refusal)?;
    let loaded = state
        .load_recorded_manifest(&installed)
        .map_err(classify_refusal)?;
    let generation =
        Generation::read(state, &installed.current_generation).map_err(classify_refusal)?;
    let directory = state.generation_dir(&generation.generation_id);
    generation.verify(&directory).map_err(classify_refusal)?;

    lines.push(format!(
        "installed release: channel={} generation={} components={}",
        installed.channel,
        installed.current_generation,
        generation.entries.len()
    ));
    lines.push(format!(
        "recorded channel manifest: {} verified sha256={}",
        loaded.path.display(),
        installed.channel_manifest_sha256
    ));

    // The recovery journal and the coordinator lock are reported, not silently ignored: an
    // interrupted transaction is a real host state an operator must see.
    let journals = journal_entries(state);
    let lock_held = state.lock_path().is_file();
    lines.push(format!(
        "recovery journal: {} interrupted transaction(s)",
        journals.len()
    ));
    lines.push(format!(
        "coordinator lock: {}",
        if lock_held { "held" } else { "free" }
    ));

    Ok(finding(
        "installed_release",
        "ok",
        "the active generation and the recorded channel manifest verify",
        vec![
            ("installed", Json::bool(true)),
            ("channel", Json::text(&installed.channel)),
            ("generation", Json::text(&installed.current_generation)),
            (
                "previous_generation",
                match &installed.previous_generation {
                    Some(value) => Json::text(value),
                    None => Json::null(),
                },
            ),
            ("journal_entries", Json::int(journals.len() as i64)),
            ("lock_held", Json::bool(lock_held)),
            (
                "components_verified",
                Json::int(generation.entries.len() as i64),
            ),
        ],
    ))
}

/// Reduce a refusal to the tuple a failed integrity check reports.
fn classify_refusal(refusal: Refusal) -> (i32, String, String) {
    (refusal.class.exit_code(), refusal.reason, refusal.message)
}

fn journal_entries(state: &State) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(state.journal_dir()) else {
        return found;
    };
    for entry in entries.flatten() {
        if entry.path().is_file() {
            found.push(entry.path().display().to_string());
        }
    }
    found.sort();
    found
}

/// Evaluate the distribution contract's section 7 dependency table.
fn prerequisites(all: bool, installed: Option<&std::path::Path>) -> (Json, Vec<String>, i32) {
    let mut lines = vec!["prerequisites (distribution contract section 7):".to_string()];
    let mut items: Vec<Json> = Vec::new();
    let mut worst = crate::cli::exit::SUCCESS;

    // Rust toolchain - needed to *build* the core release and axiom-cli. Section 7 forbids
    // requiring a compiler at install time, so for an installed release it is informational.
    let rust = probe_rust();
    if installed.is_some() {
        lines.push(format!(
            "  rust toolchain: not required by an installed release (build-time only; observed: {})",
            rust.summary
        ));
        items.push(finding(
            "rust-toolchain",
            "ok",
            "build-time prerequisite; installed binaries need no compiler",
            vec![
                ("mandatory", Json::bool(false)),
                ("observed", Json::text(&rust.summary)),
            ],
        ));
    } else {
        lines.push(format!("  rust toolchain: {}", rust.summary));
        if !rust.satisfied {
            worst = worst.max(crate::cli::exit::NOT_READY);
        }
        items.push(rust.to_json("rust-toolchain", true));
    }

    // Python interpreter - needed by axiom-mcp. An installed release carries its own provisioned,
    // digest-checked runtime, which satisfies section 7 ("provision an approved runtime").
    let provisioned = installed.and_then(|root| {
        [
            "mcp-runtime/current.json",
            "mcp-runtime/mcp-runtime/current.json",
        ]
        .iter()
        .map(|rel| root.join(rel))
        .find(|path| path.is_file())
    });
    if let Some(pointer) = provisioned {
        lines.push(format!(
            "  python interpreter: provisioned MCP runtime ({})",
            pointer.display()
        ));
        items.push(finding(
            "python-interpreter",
            "ok",
            "satisfied by the installed, provisioned MCP runtime",
            vec![
                ("mandatory", Json::bool(true)),
                ("satisfied", Json::bool(true)),
                ("source", Json::text(&pointer.display().to_string())),
            ],
        ));
    } else {
        let python = probe_python();
        lines.push(format!("  python interpreter: {}", python.summary));
        if !python.satisfied {
            worst = worst.max(crate::cli::exit::NOT_READY);
        }
        items.push(python.to_json("python-interpreter", true));
    }

    // SQLite driver - bundled or explicitly declared by the engine's owner manifest.
    //
    // The contract's section 7 requires every prerequisite to name the manifest or document that
    // proves its version, and forbids guessing one. Neither the engine's published surface nor the
    // channel manifest declares a SQLite driver today, so the honest state is `unverified` (recorded
    // as undeclared), never `ok`: merely locating a binary named `axiom-graphd` proves nothing about
    // the driver it bundles. ADR-0033 / spec L goal: a remaining `unverified` item is non-blocking
    // and attributed to the component that owns its declaration.
    lines.push(
        "  sqlite driver: unverified (non-blocking): no owner manifest declares the bundled SQLite driver version"
            .to_string(),
    );
    items.push(finding(
        "sqlite-driver",
        "unverified",
        "no owner manifest or document declares the SQLite driver version, so it cannot be verified",
        vec![
            ("needed_by", Json::text("axiom-graphd, axiom-cli")),
            ("mandatory", Json::bool(true)),
            ("satisfied", Json::bool(false)),
            ("version_source", Json::text("undeclared")),
            ("blocking", Json::bool(false)),
            ("owner_component", Json::text("axiom-graphd")),
            ("observed", Json::text("owner release manifest / update channel declares no SQLite driver")),
        ],
    ));

    // The contract declares these explicitly *not* required. Reporting them keeps an operator
    // from installing a runtime the distribution forbids depending on.
    if all {
        for (name, needed_by) in [
            ("node-js", "none"),
            ("wsl", "none"),
            ("docker", "only the optional container channel"),
            ("bash", "none"),
        ] {
            lines.push(format!(
                "  {name}: not required by the distribution (needed by: {needed_by})"
            ));
            items.push(finding(
                name,
                "ok",
                "not required by the distribution contract",
                vec![
                    ("needed_by", Json::text(needed_by)),
                    ("mandatory", Json::bool(false)),
                    ("required_by_distribution", Json::bool(false)),
                ],
            ));
        }
    }

    (
        finding(
            "prerequisites",
            if worst == crate::cli::exit::SUCCESS {
                "ok"
            } else {
                "fail"
            },
            if worst == crate::cli::exit::SUCCESS {
                "every mandatory prerequisite is satisfied"
            } else {
                "at least one mandatory prerequisite is not satisfied"
            },
            vec![("items", Json::array(items))],
        ),
        lines,
        worst,
    )
}

struct Probe {
    satisfied: bool,
    summary: String,
    observed: Option<String>,
    expected: &'static str,
    detail: Option<String>,
}

impl Probe {
    fn to_json(&self, name: &str, mandatory: bool) -> Json {
        let mut pairs = vec![
            ("check", Json::text(name)),
            (
                "status",
                Json::text(if self.satisfied { "ok" } else { "fail" }),
            ),
            ("message", Json::text(&self.summary)),
            ("satisfied", Json::bool(self.satisfied)),
            ("mandatory", Json::bool(mandatory)),
            ("expected", Json::text(self.expected)),
            (
                "observed",
                match &self.observed {
                    Some(value) => Json::text(value),
                    None => Json::null(),
                },
            ),
        ];
        if let Some(detail) = &self.detail {
            pairs.push(("detail", Json::text(detail)));
        }
        Json::from_pairs(pairs)
    }
}

fn probe_rust() -> Probe {
    const EXPECTED: &str = "a Rust toolchain matching Cargo.lock (rust-version 1.85)";
    match run_probe("rustc", &["--version"]) {
        Some(text) => {
            let version = text
                .trim()
                .strip_prefix("rustc ")
                .and_then(|rest| rest.split_whitespace().next())
                .map(str::to_string);
            match version.as_deref().and_then(parse_major_minor) {
                Some((major, minor)) if (major, minor) >= (1, 85) => Probe {
                    satisfied: true,
                    summary: format!("{} meets rust-version 1.85", text.trim()),
                    observed: version,
                    expected: EXPECTED,
                    detail: None,
                },
                Some(_) => Probe {
                    satisfied: false,
                    summary: format!("{} is older than the pinned rust-version 1.85", text.trim()),
                    observed: version,
                    expected: EXPECTED,
                    detail: Some(
                        "install a toolchain that satisfies rust-version 1.85".to_string(),
                    ),
                },
                None => Probe {
                    satisfied: false,
                    summary: format!("could not parse a version from `{}`", text.trim()),
                    observed: version,
                    expected: EXPECTED,
                    detail: None,
                },
            }
        }
        None => Probe {
            satisfied: false,
            summary: "no `rustc` was found on PATH".to_string(),
            observed: None,
            expected: EXPECTED,
            detail: Some(
                "install the pinned Rust toolchain or use a prebuilt release set".to_string(),
            ),
        },
    }
}

fn probe_python() -> Probe {
    const EXPECTED: &str = "Python >=3.13,<3.14";
    const SCRIPT: &str = "import sys;print('%d.%d.%d' % sys.version_info[:3])";
    let mut tried: Vec<String> = Vec::new();
    for candidate in ["python3.13", "python3", "python"] {
        if let Some(text) = run_probe(candidate, &["-c", SCRIPT]) {
            let version = text.trim().to_string();
            if let Some((major, minor)) = parse_major_minor(&version) {
                if (major, minor) == (3, 13) {
                    return Probe {
                        satisfied: true,
                        summary: format!("{candidate} reports Python {version}"),
                        observed: Some(version),
                        expected: EXPECTED,
                        detail: None,
                    };
                }
                tried.push(format!("{candidate}={version}"));
            }
        }
    }
    let detail = Some(
        "axiom-mcp needs Python >=3.13,<3.14; install it or keep axiom-mcp out of the pinned set"
            .to_string(),
    );
    if tried.is_empty() {
        Probe {
            satisfied: false,
            summary: "no Python interpreter in the required range was found on PATH".to_string(),
            observed: None,
            expected: EXPECTED,
            detail,
        }
    } else {
        Probe {
            satisfied: false,
            summary: format!(
                "interpreters were found but none is in range ({})",
                tried.join(", ")
            ),
            observed: Some(tried.join(", ")),
            expected: EXPECTED,
            detail,
        }
    }
}

fn run_probe(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_major_minor(text: &str) -> Option<(u64, u64)> {
    let mut parts = text.trim().split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    Some((major, minor))
}

fn engine_finding(lines: &mut Vec<String>, worst: &mut i32) -> Json {
    match engine::locate() {
        Located::Found(found) => {
            lines.push(format!(
                "engine: {} ({})",
                found.program().display(),
                found.source()
            ));
            match found.invoke(&["version".to_string(), "--json".to_string()]) {
                Ok(outcome) if outcome.exit_code() == 0 => {
                    // Exit zero alone is not proof: an engine that answers exit 0 with stdout that
                    // is not a JSON object has not answered a version query, and section 8.2
                    // forbids reporting a leg as passing when it did not run.
                    match crate::update::json::parse(outcome.stdout.trim()) {
                        Ok(report) if report.as_object().is_some() => finding(
                            "engine",
                            "ok",
                            "the axiom-graphd engine was found and answers a version query",
                            vec![
                                ("found", Json::bool(true)),
                                (
                                    "program",
                                    Json::text(&found.program().display().to_string()),
                                ),
                                ("source", Json::text(found.source())),
                                ("report", report),
                            ],
                        ),
                        _ => {
                            *worst = (*worst).max(crate::cli::exit::NOT_READY);
                            lines.push(
                                "engine: found and exited 0 but its stdout is not a JSON object"
                                    .to_string(),
                            );
                            finding(
                                "engine",
                                "fail",
                                "the engine exited 0 but did not answer a JSON version report",
                                vec![
                                    ("found", Json::bool(true)),
                                    ("exit_code", Json::int(i64::from(outcome.exit_code()))),
                                    ("stdout", Json::text(outcome.stdout.trim())),
                                ],
                            )
                        }
                    }
                }
                Ok(outcome) => {
                    *worst = (*worst).max(crate::cli::exit::NOT_READY);
                    lines.push(format!(
                        "engine: found but exits {} - {}",
                        outcome.exit_code(),
                        outcome.stderr.trim()
                    ));
                    finding(
                        "engine",
                        "fail",
                        "the engine binary was found but did not answer a version query",
                        vec![
                            ("found", Json::bool(true)),
                            ("exit_code", Json::int(i64::from(outcome.exit_code()))),
                            ("stderr", Json::text(outcome.stderr.trim())),
                        ],
                    )
                }
                Err(refusal) => {
                    *worst = (*worst).max(crate::cli::exit::NOT_READY);
                    lines.push(format!("engine: unlaunchable - {}", refusal.message));
                    finding(
                        "engine",
                        "fail",
                        "the engine binary was found but could not be launched",
                        vec![("message", Json::text(&refusal.message))],
                    )
                }
            }
        }
        Located::Missing(searched) => {
            *worst = (*worst).max(crate::cli::exit::NOT_READY);
            lines.push(format!(
                "engine: not found ({} location(s) searched)",
                searched.len()
            ));
            finding(
                "engine",
                "unverified",
                "the axiom-graphd engine was not found, so engine-owned checks were not performed",
                vec![
                    ("found", Json::bool(false)),
                    (
                        "searched",
                        Json::array(searched.iter().map(|item| Json::text(&item.path)).collect()),
                    ),
                ],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_success_keeps_unverified_checks_visible_without_internal_paths() {
        let findings = vec![
            finding(
                "installed_release",
                "ok",
                "verified",
                vec![("installed", Json::bool(true))],
            ),
            finding(
                "prerequisites",
                "ok",
                "checked",
                vec![(
                    "items",
                    Json::array(vec![finding(
                        "sqlite-driver",
                        "unverified",
                        "unknown",
                        vec![],
                    )]),
                )],
            ),
        ];
        let lines = summary_lines(&findings, 0);
        assert_eq!(lines[0], "Axiom installation: OK");
        assert!(lines.iter().any(|line| line.contains("non-blocking")));
        assert!(lines.iter().any(|line| line.contains("not checked")));
        assert!(lines.len() <= 5);
    }

    #[test]
    fn compact_failure_names_the_missing_prerequisite_and_the_next_step() {
        let findings = vec![finding(
            "prerequisites",
            "fail",
            "missing",
            vec![(
                "items",
                Json::array(vec![finding(
                    "python-interpreter",
                    "fail",
                    "missing",
                    vec![],
                )]),
            )],
        )];
        let lines = summary_lines(&findings, crate::cli::exit::NOT_READY);
        assert!(lines[0].contains("needs attention"));
        assert!(lines
            .iter()
            .any(|line| line.contains("MCP Python: missing") && line.contains("install URL")));
    }

    #[test]
    fn compact_clean_host_does_not_claim_an_installed_ecosystem() {
        let findings = vec![finding(
            "installed_release",
            "ok",
            "clean",
            vec![("installed", Json::bool(false))],
        )];
        let lines = summary_lines(&findings, 0);
        assert_eq!(lines[0], "Axiom: not installed");
        assert!(lines.iter().any(|line| line == "Next: axm install"));
    }

    #[test]
    fn a_version_string_parses_into_major_and_minor() {
        assert_eq!(parse_major_minor("3.13.2"), Some((3, 13)));
        assert_eq!(parse_major_minor("1.85.0-nightly"), Some((1, 85)));
        assert_eq!(parse_major_minor("nonsense"), None);
        assert_eq!(parse_major_minor("3"), None);
    }

    #[test]
    fn a_finding_carries_its_check_name_and_status() {
        let value = finding(
            "target",
            "ok",
            "declared",
            vec![("target", Json::text("macos-x64"))],
        );
        let object = value.as_object().expect("object");
        assert_eq!(object.get("check").and_then(Json::as_text), Some("target"));
        assert_eq!(object.get("status").and_then(Json::as_text), Some("ok"));
        assert_eq!(
            object.get("target").and_then(Json::as_text),
            Some("macos-x64")
        );
    }

    #[test]
    fn every_integrity_class_maps_onto_a_status_token() {
        assert_eq!(class_status(crate::cli::exit::NOT_FOUND), "not_found");
        assert_eq!(class_status(crate::cli::exit::CONFLICT), "conflict");
        assert_eq!(
            class_status(crate::cli::exit::VALIDATION),
            "validation_error"
        );
        assert_eq!(class_status(crate::cli::exit::IO_INTERNAL), "io_error");
    }
}
