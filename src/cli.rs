//! The `axiom-cli` argv surface.
//!
//! `axiom-cli` is the canonical Axiom ecosystem *distribution* entrypoint. It owns the
//! distributed executable, the argv surface, the exit vocabulary and the `NotReady`
//! contract. It does **not** own the installation engine, the bootstrap rules, the
//! service lifecycle or the per-component update transaction: those stay in
//! `axiom-graphd` and are invoked through their published argv surface.
//!
//! Frozen by task `J-003`. Contract references (canonical in the pinned `axiom-specs`):
//!
//! - `contracts/axiom-cli-distribution-contract.md` sections 2, 6 and 7
//! - `docs/16-CLI-AND-CONTROL-API.md` sections 1, 6 and 7
//!
//! The usage text is written out by hand and is pinned against the dispatcher by
//! `tests/argv_surface.rs`: removing a verb from the dispatcher while leaving it in the
//! usage text - or the reverse - fails that test.

use crate::update::{self, Request, Subcommand};
use crate::{doctor, lifecycle, version};
use std::ffi::OsString;

/// Canonical CLI exit vocabulary.
///
/// Owned by `axiom-specs/docs/16-CLI-AND-CONTROL-API.md` section 6. The distribution
/// contract references that owner and MUST NOT define a private exit-code space.
///
/// The complete canonical set is frozen here even while this slice only produces the
/// success and validation codes: the contract owns the vocabulary, not the
/// implementation, so a code must not disappear because nothing emits it yet.
#[allow(dead_code)]
pub mod exit {
    /// `0` success.
    pub const SUCCESS: i32 = 0;
    /// `2` validation.
    pub const VALIDATION: i32 = 2;
    /// `3` not found.
    pub const NOT_FOUND: i32 = 3;
    /// `4` not-ready / stale.
    pub const NOT_READY: i32 = 4;
    /// `5` authorization.
    pub const AUTHORIZATION: i32 = 5;
    /// `6` conflict.
    pub const CONFLICT: i32 = 6;
    /// `7` timeout / busy.
    pub const TIMEOUT_BUSY: i32 = 7;
    /// `8` I/O or internal.
    pub const IO_INTERNAL: i32 = 8;
    /// `9` incompatible.
    pub const INCOMPATIBLE: i32 = 9;
    /// `10` lock unavailable.
    pub const LOCK_UNAVAILABLE: i32 = 10;
    /// `20` partial multi-repository operation.
    pub const PARTIAL: i32 = 20;
}

/// The distributed program name (`axiom-cli.exe` on Windows).
pub const PROGRAM: &str = "axiom-cli";
/// The repository that owns the installation engine this layer wraps.
pub const ENGINE_OWNER: &str = "axiom-graphd";
/// Distribution version, taken from `Cargo.toml` and kept in step with `VERSION`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The five verbs of the distribution contract, section 2 "Entrypoint and verbs".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    /// `install` - install or repair this platform's pinned component set.
    Install,
    /// `update` - `check`, `plan`, `apply`, `rollback` through the update channel.
    Update,
    /// `doctor` - report prerequisites, installed versions, digests and target state.
    Doctor,
    /// `version` - report installed and available versions per component and target.
    Version,
    /// `uninstall` - remove binaries and service registration; preserve user data.
    Uninstall,
}

impl Verb {
    /// Resolve a verb from its argv token.
    pub fn from_name(name: &str) -> Option<Verb> {
        match name {
            "install" => Some(Verb::Install),
            "update" => Some(Verb::Update),
            "doctor" => Some(Verb::Doctor),
            "version" => Some(Verb::Version),
            "uninstall" => Some(Verb::Uninstall),
            _ => None,
        }
    }

    /// Canonical argv token for this verb.
    pub fn name(self) -> &'static str {
        match self {
            Verb::Install => "install",
            Verb::Update => "update",
            Verb::Doctor => "doctor",
            Verb::Version => "version",
            Verb::Uninstall => "uninstall",
        }
    }

    /// One-line contract summary shown by `--help`.
    pub fn summary(self) -> &'static str {
        match self {
            Verb::Install => "Install or repair this platform's pinned component set",
            Verb::Update => "check | plan | apply | rollback through the update channel",
            Verb::Doctor => "Report prerequisites, installed versions, digests and target state",
            Verb::Version => "Report installed and available versions per component and per target",
            Verb::Uninstall => "Remove binaries and service registration; preserve user data",
        }
    }
}

/// Verb block of the global usage text.
///
/// Written by hand on purpose: it is an independent statement of the documented surface,
/// so a drift between it and [`Verb::from_name`] is detectable instead of impossible.
const USAGE_VERBS: &str = "\
VERBS:
    install      Install or repair this platform's pinned component set
    update       check | plan | apply | rollback through the update channel
    doctor       Report prerequisites, installed versions, digests and target state
    version      Report installed and available versions per component and per target
    uninstall    Remove binaries and service registration; preserve user data
";

/// Canonical exit vocabulary as printed by `--help`.
///
/// Written with explicit `\n` separators: a `\`-continued literal would swallow the
/// leading indentation of its first line and print the first code unindented.
const USAGE_EXIT_CODES: &str = concat!(
    "    0   success\n",
    "    2   validation\n",
    "    3   not found\n",
    "    4   not ready / stale\n",
    "    5   authorization\n",
    "    6   conflict\n",
    "    7   timeout / busy\n",
    "    8   I/O internal\n",
    "    9   incompatible\n",
    "    10  lock unavailable\n",
    "    20  partial multi-repository operation\n",
);

/// Global usage text, printed by `axiom-cli --help`.
pub fn usage() -> String {
    format!(
        "\
{PROGRAM} {VERSION} - Axiom ecosystem distribution entrypoint

USAGE:
    {PROGRAM} [GLOBAL OPTIONS] <verb> [verb options]

{USAGE_VERBS}
GLOBAL OPTIONS:
    -h, --help    Print this help and exit 0
    --json        Machine-readable mode: exactly one JSON object on stdout
    --verbose     Emit diagnostics on stderr

VERB OPTIONS:
    install      [--yes] [--no-modify-path] [--adopt <path>] | --dry-run |
                 --apply, --approve-digest <sha256>, --from <path>
    update       [--yes] [--dry-run] [--channel <url|path>] |
                 check [--all] | plan --to <version> [--out <file>] |
                 apply --plan <file> --approve-digest <sha256> |
                 rollback --transaction <id> [--approve-digest <sha256>]
    doctor       [--all]
    version      [--all]
    uninstall    [--yes] | --dry-run | --apply, --approve-digest <sha256>, [--purge-data]

EXIT CODES:
{USAGE_EXIT_CODES}
NOTES:
    Every documented verb is dispatched to real work. A verb that cannot complete
    answers its canonical failure code with a stated reason on stderr and exactly one
    JSON object on stdout; it never returns 0 and never emits an empty success
    envelope.
    A bare install or uninstall prints its plan and asks once (Proceed? [Y/n]); the
    answer approves exactly that plan's digest. --yes (or AXIOM_INSTALL_YES=1) approves
    without a prompt. Without a terminal and without --yes nothing changes and the exit
    code is 4. --dry-run reports the plan; --apply needs --approve-digest <sha256> bound
    to the canonical plan digest; supplying --plan or --from is not approval.
    axm is the short name of axiom-cli: the same program, installed next to it in the
    same bin. Every verb, option and exit code is identical.
    The installation engine stays owned by {ENGINE_OWNER}; this layer wraps it.
"
    )
}

/// Verb-specific help text, printed by `axiom-cli <verb> --help`.
///
/// Written with [`concat!`] and explicit `\n` separators for the same reason as
/// [`USAGE_EXIT_CODES`]: a `\`-continued literal swallows the leading indentation of the line that
/// follows the escape, which silently unindented the first option of every verb.
pub fn verb_help(verb: Verb) -> String {
    let options = match verb {
        Verb::Install => concat!(
            "    A bare `install` prints the plan (version, components, root, PATH, size) and asks once.\n",
            "    --yes, -y                  Approve the printed plan without a prompt (AXIOM_INSTALL_YES=1)\n",
            "    --no-modify-path           Do not add the install's bin to the user PATH\n",
            "    --adopt <path>             Adopt a legacy 0.1.x root in place instead of side by side\n",
            "    --dry-run                  Validate and report the plan without changing the host\n",
            "    --apply                    Apply the transaction; requires --approve-digest\n",
            "    --approve-digest <sha256>  Approval bound to the canonical plan digest\n",
            "    --from <path>              Read the release set from a local path instead of the channel\n",
        ),
        Verb::Update => concat!(
            "    update [--yes] [--dry-run] [--channel <src>]          Check the channel, show the plan, ask once, apply with rollback\n",
            "    update check [--all]                                  Report available transitions\n",
            "    update plan --to <version> [--out <file>]             Write the canonical plan\n",
            "    update apply --plan <file> --approve-digest <sha256>  Apply the approved plan\n",
            "    update rollback --transaction <id>                    Restore the previous generation\n",
        ),
        Verb::Doctor => {
            "    --all  Probe every declared component, not only the pinned set\n"
        }
        Verb::Version => {
            "    --all  Report every declared component, not only the installed set\n"
        }
        Verb::Uninstall => concat!(
            "    A bare `uninstall` prints what it removes and keeps, and asks once.\n",
            "    --yes, -y                  Approve the printed plan without a prompt (AXIOM_INSTALL_YES=1)\n",
            "    --dry-run                  Report what would be removed without removing it\n",
            "    --apply                    Remove owned binaries and startup entries; requires --approve-digest\n",
            "    --approve-digest <sha256>  Approval bound to the canonical plan digest\n",
            "    --purge-data               Also delete workspace data; needs --apply and separate approval\n",
        ),
    };
    format!(
        "\
{PROGRAM} {verb_name} - {summary}

USAGE:
    {PROGRAM} {verb_name} [options]

OPTIONS:
{options}
GLOBAL OPTIONS:
    -h, --help    Print this help and exit 0
    --json        Machine-readable mode: exactly one JSON object on stdout
    --verbose     Emit diagnostics on stderr

EXIT CODES:
{USAGE_EXIT_CODES}
NOTES:
    Every documented verb is dispatched to real work. A verb that cannot complete
    answers its canonical failure code with a stated reason; it never returns 0 and
    never emits an empty success envelope.
",
        verb_name = verb.name(),
        summary = verb.summary(),
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OptKind {
    DryRun,
    Apply,
    ApproveDigest,
    From,
    Plan,
    To,
    Out,
    Transaction,
    All,
    PurgeData,
    Yes,
    NoModifyPath,
    Channel,
    Adopt,
}

impl OptKind {
    fn flag(self) -> &'static str {
        match self {
            OptKind::DryRun => "--dry-run",
            OptKind::Apply => "--apply",
            OptKind::ApproveDigest => "--approve-digest",
            OptKind::From => "--from",
            OptKind::Plan => "--plan",
            OptKind::To => "--to",
            OptKind::Out => "--out",
            OptKind::Transaction => "--transaction",
            OptKind::All => "--all",
            OptKind::PurgeData => "--purge-data",
            OptKind::Yes => "--yes",
            OptKind::NoModifyPath => "--no-modify-path",
            OptKind::Channel => "--channel",
            OptKind::Adopt => "--adopt",
        }
    }
}

/// The concrete work one parsed invocation dispatches to.
///
/// Every documented verb has an action, so the dispatcher has no "unbuilt verb" fallback. A verb
/// that cannot complete still reports its own precise refusal from inside its action; it never
/// falls through to a blanket answer.
enum Action {
    /// `update check|plan|apply|rollback`.
    Update(Request),
    /// Bare `update`: the one-command update of ADR-0033 decision 6.
    UpdateOneShot(update::oneshot::Request),
    /// `install`.
    Install(lifecycle::InstallRequest),
    /// `uninstall`.
    Uninstall(lifecycle::UninstallRequest),
    /// `doctor`.
    Doctor { all: bool },
    /// `version`.
    Version { all: bool },
}

/// A parsed invocation: the verb, its frozen one-line parse summary, and the action to run.
struct Invocation {
    verb: Verb,
    /// The frozen one-line parse summary the `NotReady` envelope reports.
    detail: String,
    /// The work this invocation dispatches to.
    action: Action,
}

impl Invocation {
    /// Build an `update` invocation from the structured request and its parse summary.
    fn update(request: Request, detail: String) -> Invocation {
        Invocation {
            verb: Verb::Update,
            detail,
            action: Action::Update(request),
        }
    }

    /// Build an `install` invocation.
    fn install(request: lifecycle::InstallRequest, detail: String) -> Invocation {
        Invocation {
            verb: Verb::Install,
            detail,
            action: Action::Install(request),
        }
    }

    /// Build an `uninstall` invocation.
    fn uninstall(request: lifecycle::UninstallRequest, detail: String) -> Invocation {
        Invocation {
            verb: Verb::Uninstall,
            detail,
            action: Action::Uninstall(request),
        }
    }

    /// Build a `doctor` invocation.
    fn doctor(all: bool, detail: String) -> Invocation {
        Invocation {
            verb: Verb::Doctor,
            detail,
            action: Action::Doctor { all },
        }
    }

    /// Build a `version` invocation.
    fn version(all: bool, detail: String) -> Invocation {
        Invocation {
            verb: Verb::Version,
            detail,
            action: Action::Version { all },
        }
    }
}

/// Parse the program argv and return the process exit code.
///
/// The first element of `args` is the program name; it is skipped.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let mut tokens: Vec<String> = Vec::new();
    for (index, arg) in args.into_iter().enumerate() {
        if index == 0 {
            continue;
        }
        match arg.to_str() {
            Some(text) => tokens.push(text.to_string()),
            None => {
                return fail(
                    false,
                    false,
                    None,
                    "argument is not valid UTF-8; argv is passed as a program and argument list",
                )
            }
        }
    }

    // The machine-readable contract belongs to the invocation, not to token order: `--json`
    // anywhere in argv must be honoured even when the refusal happens before the scan below
    // reaches it (`axiom-cli unknown-verb --json`). Without this pre-scan that invocation
    // answered prose on stderr with an empty stdout, so the same request answered differently
    // depending on where the flag sat. Latched here so every refusal on this path already knows
    // it owes stdout exactly one JSON object.
    let mut json = tokens.iter().any(|token| token == "--json");
    let mut verbose = tokens.iter().any(|token| token == "--verbose");
    let mut help = false;
    let mut verb: Option<Verb> = None;
    let mut rest: Vec<String> = Vec::new();

    for token in tokens {
        if token == "-h" || token == "--help" {
            help = true;
            continue;
        }
        if verb.is_none() {
            if token == "--json" {
                json = true;
            } else if token == "--verbose" {
                verbose = true;
            } else if token.starts_with('-') {
                return fail(
                    json,
                    verbose,
                    None,
                    &format!("unknown global option `{token}`"),
                );
            } else {
                match Verb::from_name(&token) {
                    Some(found) => verb = Some(found),
                    None => return fail(json, verbose, None, &format!("unknown verb `{token}`")),
                }
            }
        } else if token == "--json" {
            json = true;
        } else if token == "--verbose" {
            verbose = true;
        } else {
            rest.push(token);
        }
    }

    if help {
        let body = match verb {
            Some(found) => verb_help(found),
            None => usage(),
        };
        if json {
            println!(
                "{}",
                envelope(
                    exit::SUCCESS,
                    "ok",
                    "help",
                    false,
                    &[("usage", body.as_str())],
                )
            );
        } else {
            print!("{body}");
        }
        return exit::SUCCESS;
    }

    let Some(verb) = verb else {
        return fail(
            json,
            verbose,
            None,
            "no verb supplied; expected one of install, update, doctor, version, uninstall",
        );
    };

    let invocation = match parse_invocation(verb, &rest) {
        Ok(invocation) => invocation,
        Err(message) => return fail(json, verbose, Some(verb.name()), &message),
    };

    if verbose && !json {
        eprintln!(
            "{PROGRAM}: diagnostic: parsed {} invocation: {}",
            invocation.verb.name(),
            invocation.detail
        );
    }
    match invocation.action {
        Action::Update(request) => update::run_update(request, json, verbose),
        Action::UpdateOneShot(request) => update::oneshot::run(request, json, verbose),
        Action::Install(request) => lifecycle::run_install(request, json, verbose),
        Action::Uninstall(request) => lifecycle::run_uninstall(request, json, verbose),
        Action::Doctor { all } => doctor::run_doctor(all, json, verbose),
        Action::Version { all } => version::run_version(all, json, verbose),
    }
}

fn parse_invocation(verb: Verb, rest: &[String]) -> Result<Invocation, String> {
    let mut flags: Vec<OptKind> = Vec::new();
    let mut values: Vec<(OptKind, String)> = Vec::new();
    let mut positionals: Vec<String> = Vec::new();

    let mut index = 0usize;
    while index < rest.len() {
        let token = rest[index].as_str();
        if token.starts_with('-') {
            let Some((kind, needs_value)) = option_spec(verb, token) else {
                return Err(format!(
                    "unknown option `{token}` for verb `{}`",
                    verb.name()
                ));
            };
            if needs_value {
                match rest.get(index + 1) {
                    Some(next) if !next.starts_with('-') => {
                        values.push((kind, next.clone()));
                        index += 2;
                    }
                    _ => return Err(format!("option `{token}` requires a value")),
                }
            } else {
                flags.push(kind);
                index += 1;
            }
        } else {
            positionals.push(rest[index].clone());
            index += 1;
        }
    }

    match verb {
        Verb::Update => parse_update(&flags, &values, &positionals),
        Verb::Install => {
            reject_positionals(verb, &positionals)?;
            if has(&flags, OptKind::DryRun) && has(&flags, OptKind::Apply) {
                return Err("`--dry-run` and `--apply` are mutually exclusive on `install`".into());
            }
            if has(&flags, OptKind::Apply) && value(&values, OptKind::ApproveDigest).is_none() {
                return Err(
                    "`install --apply` requires `--approve-digest <sha256>`; approval is bound to the canonical plan digest"
                        .into(),
                );
            }
            if let Some(digest) = value(&values, OptKind::ApproveDigest) {
                check_digest(&digest)?;
            }
            let mode = mode_of(verb, &flags)?;
            interactive_only(verb, mode, &flags)?;
            let from = value(&values, OptKind::From).unwrap_or_else(|| "-".to_string());
            Ok(Invocation::install(
                lifecycle::InstallRequest {
                    mode,
                    from: value(&values, OptKind::From),
                    approve_digest: value(&values, OptKind::ApproveDigest),
                    yes: has(&flags, OptKind::Yes),
                    modify_path: !has(&flags, OptKind::NoModifyPath),
                    adopt: value(&values, OptKind::Adopt),
                },
                format!("install mode={} from={from}", mode.name()),
            ))
        }
        Verb::Uninstall => {
            reject_positionals(verb, &positionals)?;
            if has(&flags, OptKind::DryRun) && has(&flags, OptKind::Apply) {
                return Err(
                    "`--dry-run` and `--apply` are mutually exclusive on `uninstall`".into(),
                );
            }
            let apply = has(&flags, OptKind::Apply);
            let purge = has(&flags, OptKind::PurgeData);
            if purge && !apply {
                return Err(
                    "`--purge-data` requires `uninstall --apply --approve-digest <sha256>`; deleting workspace data needs a separate explicit approval"
                        .into(),
                );
            }
            if apply && value(&values, OptKind::ApproveDigest).is_none() {
                return Err("`uninstall --apply` requires `--approve-digest <sha256>`".into());
            }
            if let Some(digest) = value(&values, OptKind::ApproveDigest) {
                check_digest(&digest)?;
            }
            let mode = mode_of(verb, &flags)?;
            interactive_only(verb, mode, &flags)?;
            Ok(Invocation::uninstall(
                lifecycle::UninstallRequest {
                    mode,
                    approve_digest: value(&values, OptKind::ApproveDigest),
                    purge_data: purge,
                    yes: has(&flags, OptKind::Yes),
                },
                format!("uninstall mode={} purge_data={purge}", mode.name()),
            ))
        }
        Verb::Doctor => {
            reject_positionals(verb, &positionals)?;
            let all = has(&flags, OptKind::All);
            Ok(Invocation::doctor(all, format!("doctor all={all}")))
        }
        Verb::Version => {
            reject_positionals(verb, &positionals)?;
            let all = has(&flags, OptKind::All);
            Ok(Invocation::version(all, format!("version all={all}")))
        }
    }
}

/// The transaction mode an `install` or `uninstall` invocation names.
///
/// `--dry-run` and `--apply` keep their automation meaning. A bare verb is the interactive
/// single-confirmation mode of ADR-0033 decision 4 (distribution contract section 2 rule 4): it
/// prints the plan and asks once, and without a terminal and without `--yes` it changes nothing
/// and exits `4`. It never reports success for a transaction that did not run.
fn mode_of(_verb: Verb, flags: &[OptKind]) -> Result<lifecycle::Mode, String> {
    if has(flags, OptKind::Apply) {
        Ok(lifecycle::Mode::Apply)
    } else if has(flags, OptKind::DryRun) {
        Ok(lifecycle::Mode::DryRun)
    } else {
        Ok(lifecycle::Mode::Interactive)
    }
}

/// `--yes` and `--no-modify-path` belong to the interactive mode only.
fn interactive_only(verb: Verb, mode: lifecycle::Mode, flags: &[OptKind]) -> Result<(), String> {
    if mode == lifecycle::Mode::Interactive {
        return Ok(());
    }
    for kind in [OptKind::Yes, OptKind::NoModifyPath] {
        if has(flags, kind) {
            return Err(format!(
                "`{}` applies to the interactive `{}` (no `--dry-run` / `--apply`); \
                 `--apply` is already approved by `--approve-digest`",
                kind.flag(),
                verb.name()
            ));
        }
    }
    Ok(())
}

fn parse_update(
    flags: &[OptKind],
    values: &[(OptKind, String)],
    positionals: &[String],
) -> Result<Invocation, String> {
    let subcommand = match positionals.len() {
        0 => {
            // ADR-0033 decision 6: bare `update` checks the recorded channel, prints the plan,
            // asks once and applies with rollback.
            for flag in flags {
                if !matches!(flag, OptKind::Yes | OptKind::DryRun) {
                    return Err(format!(
                        "option `{}` is not valid for bare `update`",
                        flag.flag()
                    ));
                }
            }
            for (kind, _) in values {
                if *kind != OptKind::Channel {
                    return Err(format!(
                        "option `{}` is not valid for bare `update`",
                        kind.flag()
                    ));
                }
            }
            if has(flags, OptKind::Yes) && has(flags, OptKind::DryRun) {
                return Err("`--yes` and `--dry-run` are mutually exclusive on `update`".into());
            }
            let request = update::oneshot::Request {
                channel: value(values, OptKind::Channel),
                yes: has(flags, OptKind::Yes),
                dry_run: has(flags, OptKind::DryRun),
            };
            let detail = format!("update one-shot dry_run={}", request.dry_run);
            return Ok(Invocation {
                verb: Verb::Update,
                detail,
                action: Action::UpdateOneShot(request),
            });
        }
        1 => positionals[0].as_str(),
        count => {
            return Err(format!(
                "`update` accepts exactly one subcommand, got {count} positional arguments"
            ))
        }
    };

    // The parsed token and the structured request share one vocabulary: a subcommand that the
    // request type does not know is refused by name instead of falling through to a default.
    let Some(kind) = Subcommand::from_name(subcommand) else {
        return Err(format!(
            "unknown update subcommand `{subcommand}`; expected check, plan, apply or rollback"
        ));
    };

    match kind {
        Subcommand::Check => {
            for flag in flags {
                if *flag != OptKind::All {
                    return Err(format!(
                        "option `{}` is not valid for `update check`",
                        flag.flag()
                    ));
                }
            }
            if let Some((kind, _)) = values.first() {
                return Err(format!(
                    "option `{}` is not valid for `update check`",
                    kind.flag()
                ));
            }
            Ok(Invocation::update(
                Request {
                    subcommand: Subcommand::Check,
                    all: has(flags, OptKind::All),
                    to: None,
                    out: None,
                    plan_path: None,
                    approve_digest: None,
                    transaction: None,
                },
                format!("update check all={}", has(flags, OptKind::All)),
            ))
        }
        Subcommand::Plan => {
            reject_update_flags(flags, subcommand)?;
            for (kind, _) in values {
                if !matches!(kind, OptKind::To | OptKind::Out) {
                    return Err(format!(
                        "option `{}` is not valid for `update plan`",
                        kind.flag()
                    ));
                }
            }
            let to = value(values, OptKind::To)
                .ok_or_else(|| "`update plan` requires `--to <version>`".to_string())?;
            let out = value(values, OptKind::Out).unwrap_or_else(|| "-".to_string());
            Ok(Invocation::update(
                Request {
                    subcommand: Subcommand::Plan,
                    all: false,
                    to: Some(to.clone()),
                    out: Some(out.clone()),
                    plan_path: None,
                    approve_digest: None,
                    transaction: None,
                },
                format!("update plan to={to} out={out}"),
            ))
        }
        Subcommand::Apply => {
            reject_update_flags(flags, subcommand)?;
            for (kind, _) in values {
                if !matches!(kind, OptKind::Plan | OptKind::ApproveDigest) {
                    return Err(format!(
                        "option `{}` is not valid for `update apply`",
                        kind.flag()
                    ));
                }
            }
            let plan = value(values, OptKind::Plan)
                .ok_or_else(|| "`update apply` requires `--plan <file>`".to_string())?;
            let digest = value(values, OptKind::ApproveDigest).ok_or_else(|| {
                "`update apply` requires `--approve-digest <sha256>`; supplying `--plan` alone is not approval"
                    .to_string()
            })?;
            check_digest(&digest)?;
            Ok(Invocation::update(
                Request {
                    subcommand: Subcommand::Apply,
                    all: false,
                    to: None,
                    out: None,
                    plan_path: Some(plan.clone()),
                    approve_digest: Some(digest.clone()),
                    transaction: None,
                },
                format!("update apply plan={plan}"),
            ))
        }
        Subcommand::Rollback => {
            reject_update_flags(flags, subcommand)?;
            for (kind, _) in values {
                if !matches!(kind, OptKind::Transaction | OptKind::ApproveDigest) {
                    return Err(format!(
                        "option `{}` is not valid for `update rollback`",
                        kind.flag()
                    ));
                }
            }
            let transaction = value(values, OptKind::Transaction)
                .ok_or_else(|| "`update rollback` requires `--transaction <id>`".to_string())?;
            if let Some(digest) = value(values, OptKind::ApproveDigest) {
                check_digest(&digest)?;
            }
            Ok(Invocation::update(
                Request {
                    subcommand: Subcommand::Rollback,
                    all: false,
                    to: None,
                    out: None,
                    plan_path: None,
                    approve_digest: value(values, OptKind::ApproveDigest),
                    transaction: Some(transaction.clone()),
                },
                format!("update rollback transaction={transaction}"),
            ))
        }
    }
}

fn reject_update_flags(flags: &[OptKind], subcommand: &str) -> Result<(), String> {
    if let Some(flag) = flags.first() {
        return Err(format!(
            "option `{}` is not valid for `update {subcommand}`",
            flag.flag()
        ));
    }
    Ok(())
}

fn option_spec(verb: Verb, token: &str) -> Option<(OptKind, bool)> {
    let (kind, needs_value) = match token {
        "--dry-run" => (OptKind::DryRun, false),
        "--apply" => (OptKind::Apply, false),
        "--approve-digest" => (OptKind::ApproveDigest, true),
        "--from" => (OptKind::From, true),
        "--plan" => (OptKind::Plan, true),
        "--to" => (OptKind::To, true),
        "--out" => (OptKind::Out, true),
        "--transaction" => (OptKind::Transaction, true),
        "--all" => (OptKind::All, false),
        "--purge-data" => (OptKind::PurgeData, false),
        "--yes" | "-y" => (OptKind::Yes, false),
        "--no-modify-path" => (OptKind::NoModifyPath, false),
        "--channel" => (OptKind::Channel, true),
        "--adopt" => (OptKind::Adopt, true),
        _ => return None,
    };
    let allowed = match kind {
        OptKind::DryRun => matches!(verb, Verb::Install | Verb::Uninstall | Verb::Update),
        OptKind::Apply => matches!(verb, Verb::Install | Verb::Uninstall),
        OptKind::ApproveDigest => matches!(verb, Verb::Install | Verb::Update | Verb::Uninstall),
        OptKind::From => matches!(verb, Verb::Install),
        OptKind::Plan | OptKind::To | OptKind::Out | OptKind::Transaction => {
            matches!(verb, Verb::Update)
        }
        OptKind::All => matches!(verb, Verb::Update | Verb::Doctor | Verb::Version),
        OptKind::PurgeData => matches!(verb, Verb::Uninstall),
        OptKind::Yes => matches!(verb, Verb::Install | Verb::Uninstall | Verb::Update),
        OptKind::NoModifyPath => matches!(verb, Verb::Install),
        OptKind::Channel => matches!(verb, Verb::Update),
        OptKind::Adopt => matches!(verb, Verb::Install),
    };
    if allowed {
        Some((kind, needs_value))
    } else {
        None
    }
}

fn has(flags: &[OptKind], kind: OptKind) -> bool {
    flags.contains(&kind)
}

fn value(values: &[(OptKind, String)], kind: OptKind) -> Option<String> {
    values
        .iter()
        .find(|(candidate, _)| *candidate == kind)
        .map(|(_, text)| text.clone())
}

fn reject_positionals(verb: Verb, positionals: &[String]) -> Result<(), String> {
    match positionals.first() {
        Some(extra) => Err(format!(
            "unexpected positional argument `{extra}` for verb `{}`",
            verb.name()
        )),
        None => Ok(()),
    }
}

fn check_digest(digest: &str) -> Result<(), String> {
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!(
            "`--approve-digest` must be a 64-character hex sha256 digest, got {} characters",
            digest.len()
        ))
    }
}

fn fail(json: bool, verbose: bool, verb: Option<&str>, message: &str) -> i32 {
    if json {
        let mut details: Vec<(&str, &str)> = Vec::new();
        if let Some(name) = verb {
            details.push(("verb", name));
        }
        println!(
            "{}",
            envelope(
                exit::VALIDATION,
                "validation_error",
                message,
                false,
                &details
            )
        );
    } else {
        eprintln!("{PROGRAM}: error: {message}");
        eprintln!("{PROGRAM}: try `{PROGRAM} --help`");
        if verbose {
            eprintln!("{PROGRAM}: diagnostic: exit code {}", exit::VALIDATION);
        }
    }
    exit::VALIDATION
}

fn envelope(
    code: i32,
    status: &str,
    message: &str,
    retryable: bool,
    details: &[(&str, &str)],
) -> String {
    let mut out = String::new();
    out.push('{');
    out.push_str(&format!("\"code\":{code},"));
    out.push_str(&format!("\"status\":\"{}\",", escape(status)));
    out.push_str(&format!("\"message\":\"{}\",", escape(message)));
    out.push_str(&format!("\"retryable\":{retryable},"));
    out.push_str("\"details\":{");
    for (index, (key, item)) in details.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!("\"{}\":\"{}\"", escape(key), escape(item)));
    }
    out.push_str("},\"request_id\":\"");
    out.push_str(&escape(&request_id()));
    out.push_str("\"}");
    out
}

fn request_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{PROGRAM}-{:x}-{:x}", std::process::id(), nanos)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32))
            }
            other => out.push(other),
        }
    }
    out
}
