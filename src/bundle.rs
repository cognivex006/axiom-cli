//! Engine-bundle assembly, and the engine invocation that follows it.
//!
//! `axiom-cli` owns *what is distributed and how it is verified*; `axiom-graphd` owns *where the
//! verified bytes are placed*. The engine publishes that placement as two argv steps over one
//! sealed ecosystem plan:
//!
//! 1. `axiom install plan  --bundle <abs dir> --out <plan file>`
//! 2. `axiom install apply --plan <plan file> --approve-digest <plan digest>`
//!
//! Step 1 reads a *bundle*: a directory holding `bundle.json` (the `BundleManifest` of
//! `axiom-graphd/crates/axiom/src/install/plan.rs`), one payload per declared artifact at the
//! bundle-relative path the manifest names, and a `skills/bundle.json` plus `skills/payload/**`
//! tree (the `SkillBundle` of `.../skills/install.rs`). This module assembles exactly that
//! directory from a release set this layer has already byte-verified, then invokes the engine.
//!
//! Three rules shape it:
//!
//! * **Assembly is from verified bytes only.** Every payload written into the bundle is copied
//!   from a path `lifecycle::verify_artifacts` already checked by length and sha256, and the copy
//!   is re-hashed before it is declared. A bundle this layer cannot prove is a refusal, never a
//!   bundle handed to the engine "because it will check it anyway".
//! * **Values the release set does not declare are refusals, not guesses.** The engine requires a
//!   version per component; a component whose version the distribution does not declare cannot be
//!   planned, and this layer says so instead of inventing `0.0.0-dev`. Two manifest fields are
//!   *policy*, not declaration, and are stated as such: a local release set declares no service
//!   (so `service` is `null`) and requests no network access (so `network_access` is empty) — this
//!   layer is asserting what *it* will do, never claiming the release set said it. The artifact's
//!   `permissions` follow its `kind`: a `binary` announces `read` + `execute`, a `python` artifact
//!   `read` alone.
//! * **The engine's answer is the answer.** An engine refusal keeps the engine's own exit code —
//!   its vocabulary and this CLI's are the same eleven codes — and carries the engine's raw
//!   stdout, stderr and exit code as evidence.
//!
//! The engine's `plan_digest` is *not* this layer's `plan_digest`: they digest different
//! documents (an ecosystem plan vs the distribution plan). The engine's one is carried in
//! `details.engine.plan_digest` and is the only value ever passed to `--approve-digest`.

use std::path::{Path, PathBuf};

use crate::engine::{Engine, Outcome};
use crate::update::error::{Class, Refusal};
use crate::update::json::{self, Json};
use crate::update::report::Report;
use crate::update::sha256;

/// Bundle manifest file name, owned by `axiom-graphd` (`install/plan.rs`).
const BUNDLE_MANIFEST_FILE: &str = "bundle.json";
/// Bundle manifest schema version this build writes (`BUNDLE_SCHEMA_VERSION`).
const BUNDLE_SCHEMA_VERSION: i64 = 1;
/// Portable bundle id this layer writes. The engine only requires a portable slug.
const BUNDLE_ID: &str = "axiom-core";
/// Directory holding the skills bundle (`ecosystem::SKILLS_DIRECTORY`).
const SKILLS_DIRECTORY: &str = "skills";
/// Directory holding skill payload files under the skills bundle.
const SKILLS_PAYLOAD_DIRECTORY: &str = "payload";
/// File name of the skills bundle manifest inside `skills/` (`skills::install::MANIFEST_FILE`).
const SKILLS_MANIFEST_FILE: &str = "bundle.json";
/// Component id the engine's ecosystem calls the skills bundle (`ecosystem::SKILLS_COMPONENT`).
const SKILLS_COMPONENT: &str = "axiom-skills";
/// Component id the *distribution* channel uses for the skills bundle.
const SKILLS_CHANNEL_COMPONENT: &str = "skills";
/// A skills source in the `axiom-skills` repository schema.
const SKILLS_SOURCE_MANIFEST_FILE: &str = "skills-manifest.json";
/// Entry kinds the engine accepts in a skills bundle (`skills::install::ENTRY_KINDS`).
const ENTRY_KINDS: [&str; 4] = ["instruction", "reference", "asset", "script"];
/// Capabilities an executable entry must have been reviewed against (`skills::install::CAPABILITIES`).
const CAPABILITIES: [&str; 3] = ["read", "write", "execute"];
/// Suffixes that make an entry executable (`skills::install::EXECUTABLE_SUFFIXES`).
const EXECUTABLE_SUFFIXES: [&str; 8] = ["exe", "bat", "cmd", "ps1", "sh", "py", "js", "mjs"];
/// Largest single skills payload the engine accepts (`skills::install::MAX_ENTRY_BYTES`).
const MAX_ENTRY_BYTES: u64 = 8 * 1024 * 1024;

/// The two core components the engine's ecosystem requires, in `INSTALL_ORDER`.
const CORE_COMPONENTS: [&str; 2] = ["axiom-graphd", "axiom-mcp"];

/// One core component this layer carries into the engine bundle.
#[derive(Clone, Debug)]
struct CoreArtifact {
    /// Engine component id.
    component: &'static str,
    /// Declared version, from the release set.
    version: String,
    /// Engine artifact kind.
    kind: &'static str,
    /// Bundle-relative artifact path.
    artifact: String,
    /// Verified local bytes.
    source: PathBuf,
    /// Verified sha256.
    sha256: String,
    /// Verified byte length.
    size_bytes: i64,
}

/// Everything the assembler reads, all of it already resolved by the caller.
pub struct Request<'a> {
    /// Directory the bundle is assembled into (derived from the install root, not a temp dir, so
    /// a re-run produces the same plan digest).
    pub staging_root: PathBuf,
    /// Install root the engine is told to place into (`AXIOM_HOME`).
    pub install_root: PathBuf,
    /// Delivery target, already validated against the engine's `HOSTS`.
    pub host: String,
    /// Release channel, from the resolved manifest.
    pub channel: String,
    /// Bundle build time; the manifest's own `updated_at`, so the bytes are reproducible.
    pub created_at: String,
    /// Directory a local release set resolved bytes from, when the set is local.
    pub release_root: Option<PathBuf>,
    /// Verified artifacts, from `lifecycle::verify_artifacts`.
    pub verified: &'a [Json],
    /// Distribution plan components, which carry the declared version and revision.
    pub plan_components: &'a [Json],
    /// Verified per-user MCP runtime bin directory, when provisioned for this host.
    pub mcp_runtime_bin: Option<PathBuf>,
}

/// What assembly produced, for the report and for the engine steps.
struct Assembly {
    root: PathBuf,
    manifest_sha256: String,
    manifest_bytes: usize,
    core: Vec<Json>,
    skills: Json,
    /// Artifacts the release set declared but the engine bundle does not carry, with the reason.
    not_carried: Vec<Json>,
}

/// Assemble the engine bundle, invoke the engine, and report the engine's own answer.
pub fn apply(engine: &Engine, request: &Request<'_>) -> Result<Report, Refusal> {
    run(engine, request, false)
}

/// Hand an already verified distribution set to the engine's pointer-bound update transaction.
/// The distribution coordinator requests this path only after checking the installed generation.
pub fn update(engine: &Engine, request: &Request<'_>) -> Result<Report, Refusal> {
    run(engine, request, true)
}

fn run(engine: &Engine, request: &Request<'_>, updating: bool) -> Result<Report, Refusal> {
    let assembly = crate::progress::during(
        "Staging components",
        "Preparing verified component files and skills for the engine",
        || assemble(request),
    )?;
    let plan_file = request.staging_root.join("engine-plan.json");
    let engine_env = engine_environment(&request.install_root, request.mcp_runtime_bin.as_deref());

    // Step 1 - the engine plans from the bundle. `--out` is used rather than stdout so the plan
    // file the engine writes is the exact byte form the engine will re-read, with no re-encoding.
    let verb = if updating { "update" } else { "install" };
    let mut plan_args = vec![verb.to_string(), "plan".to_string()];
    if updating {
        let version = request
            .plan_components
            .iter()
            .find(|row| row.get("component").and_then(Json::as_text) == Some("axiom-graphd"))
            .and_then(|row| row.get("version"))
            .and_then(Json::as_text)
            .ok_or_else(|| {
                Refusal::validation(
                    "core_version_missing",
                    "the update set has no declared core version",
                )
            })?;
        plan_args.extend(["--to".to_string(), version.to_string()]);
    }
    plan_args.extend([
        "--bundle".to_string(),
        assembly.root.display().to_string(),
        "--out".to_string(),
        plan_file.display().to_string(),
        "--json".to_string(),
    ]);
    let plan = engine.invoke_with_env(&plan_args, &engine_env)?;
    let plan_stdout = engine_json(&plan, "plan")?;
    let steps = vec![step_evidence("plan", &plan, plan_stdout.as_ref())];
    if plan.exit_code() != 0 {
        return Ok(refused(
            request,
            &assembly,
            steps,
            verb,
            "plan",
            &plan,
            plan_stdout.as_ref(),
        ));
    }
    let (plan_digest, engine_plan_id) = plan_identity(&plan_stdout, &plan_file, &plan)?;

    // Step 2 - the engine applies that exact plan, approved at the engine's own digest.
    let apply = engine.invoke_with_env(
        &[
            verb.to_string(),
            "apply".to_string(),
            "--plan".to_string(),
            plan_file.display().to_string(),
            "--approve-digest".to_string(),
            plan_digest.clone(),
            "--json".to_string(),
        ],
        &engine_env,
    )?;
    let apply_stdout = engine_json(&apply, "apply")?;
    let mut steps = steps;
    steps.push(step_evidence("apply", &apply, apply_stdout.as_ref()));
    if apply.exit_code() != 0 {
        return Ok(refused(
            request,
            &assembly,
            steps,
            verb,
            "apply",
            &apply,
            apply_stdout.as_ref(),
        ));
    }

    // The engine's own `status` string, or `unreported` when it sent none. This layer does not
    // substitute "installed" for a status the engine never sent: exit 0 is the engine's answer,
    // but what the engine did is named only by the engine's own words.
    let engine_status = apply_stdout
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(Json::as_text);
    let status = engine_status.unwrap_or("unreported");
    let code = apply.exit_code();
    let message = match status {
        "already-installed" => "The approved components are already installed.".to_string(),
        "installed" | "updated" | "applied" => "The approved components are ready.".to_string(),
        _ => format!("Component installation completed (engine status: {status})."),
    };
    let mut report = Report::new(code, "ok", message).subject(verb);
    report.detail("engine", engine_block(request, &assembly, steps));
    report.detail("engine_plan_id", Json::text(&engine_plan_id));
    report.detail("engine_plan_digest", Json::text(&plan_digest));
    report.detail("engine_status", Json::text(status));
    if updating {
        report.detail(
            "engine_transaction",
            apply_stdout
                .as_ref()
                .and_then(|value| value.get("transaction_id"))
                .cloned()
                .unwrap_or(Json::Null),
        );
    }
    report.detail(
        "engine_status_reported",
        Json::bool(engine_status.is_some()),
    );
    report.detail(
        "components",
        apply_stdout
            .as_ref()
            .and_then(|value| value.get("components"))
            .cloned()
            .unwrap_or_else(|| Json::array(Vec::new())),
    );
    Ok(report)
}

/// Give the engine the verified runtime first on this host's native PATH.
pub(crate) fn engine_environment(
    install_root: &Path,
    mcp_runtime_bin: Option<&Path>,
) -> Vec<(&'static str, String)> {
    let mut env = vec![("AXIOM_HOME", install_root.display().to_string())];
    if let Some(bin) = mcp_runtime_bin {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let path = if cfg!(windows) {
            format!("{};{}\\System32", bin.display(), system_root)
        } else {
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
        };
        env.push(("PATH", path));
    }
    env
}

/// Assemble the bundle directory from verified bytes only.
fn assemble(request: &Request<'_>) -> Result<Assembly, Refusal> {
    let core = core_artifacts(request)?;
    let root = request.staging_root.clone();
    reset_directory(&root)?;

    let mut declared: Vec<Json> = Vec::new();
    for artifact in &core {
        let destination = root.join(&artifact.artifact);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                Refusal::io(
                    "bundle_payload_unwritable",
                    &parent.display().to_string(),
                    &error,
                )
            })?;
        }
        std::fs::copy(&artifact.source, &destination).map_err(|error| {
            Refusal::io(
                "bundle_payload_unwritable",
                &destination.display().to_string(),
                &error,
            )
        })?;
        // Re-hash the copy: the bundle must be assembled from bytes this layer proved, not from
        // bytes it merely believed a moment earlier.
        let observed = sha256::file_hex(&destination).map_err(|error| {
            Refusal::io(
                "bundle_payload_unreadable",
                &destination.display().to_string(),
                &error,
            )
        })?;
        if observed != artifact.sha256 {
            return Err(Refusal::validation(
                format!("bundle_payload_unverified:{}", artifact.component),
                format!(
                    "the payload copied for `{}` from {} hashes to {observed}, not the verified \
                     {}; the bundle was not assembled",
                    artifact.component,
                    artifact.source.display(),
                    artifact.sha256
                ),
            ));
        }
        declared.push(Json::from_pairs(vec![
            ("component", Json::text(artifact.component)),
            ("version", Json::text(&artifact.version)),
            ("host", Json::text(&request.host)),
            ("artifact", Json::text(&artifact.artifact)),
            ("kind", Json::text(artifact.kind)),
            ("sha256", Json::text(&artifact.sha256)),
            ("size_bytes", Json::int(artifact.size_bytes)),
            (
                "permissions",
                Json::text_array(artifact_permissions(artifact.kind)),
            ),
            // Policy, not declaration: a local release set carries no service definition and asks
            // for no network access, so the plan it produces registers no service. See the module
            // doc; these are the two values the release set cannot declare today.
            ("service", Json::Null),
            ("network_access", Json::array(Vec::new())),
        ]));
    }

    let skills = assemble_skills(request)?;

    let manifest = Json::from_pairs(vec![
        ("schema_version", Json::int(BUNDLE_SCHEMA_VERSION)),
        ("bundle_id", Json::text(BUNDLE_ID)),
        ("channel", Json::text(&request.channel)),
        ("created_at", Json::text(&request.created_at)),
        ("components", Json::array(declared.clone())),
    ]);
    let manifest_file = root.join(BUNDLE_MANIFEST_FILE);
    let bytes = json::canonical_bytes(&manifest);
    std::fs::write(&manifest_file, &bytes).map_err(|error| {
        Refusal::io(
            "bundle_manifest_unwritable",
            &manifest_file.display().to_string(),
            &error,
        )
    })?;

    let (not_carried, core_ids) = not_carried(request);
    let _ = core_ids;
    Ok(Assembly {
        root,
        manifest_sha256: sha256::digest_hex(&bytes),
        manifest_bytes: bytes.len(),
        core: declared,
        skills,
        not_carried,
    })
}

/// The permissions the engine's plan announces for one artifact kind.
///
/// A `binary` is executed, so it announces `read` + `execute`; every other kind this layer carries
/// is read. Nothing here grants `write`, `network` or `elevated`: those are not implied by the
/// artifact's kind and the release set does not declare them.
fn artifact_permissions(kind: &str) -> &'static [&'static str] {
    match kind {
        "binary" => &["read", "execute"],
        _ => &["read"],
    }
}

/// Map the verified artifacts onto the core components the engine's ecosystem requires.
fn core_artifacts(request: &Request<'_>) -> Result<Vec<CoreArtifact>, Refusal> {
    let mut found: Vec<CoreArtifact> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for component in CORE_COMPONENTS {
        let Some(artifact) = request
            .verified
            .iter()
            .find(|entry| entry.get("component").and_then(Json::as_text) == Some(component))
        else {
            missing.push(component);
            continue;
        };
        let source = artifact
            .get("resolved")
            .and_then(Json::as_text)
            .map(PathBuf::from);
        let Some(source) = source else {
            missing.push(component);
            continue;
        };
        let (Some(sha), Some(size)) = (
            artifact.get("sha256").and_then(Json::as_text),
            artifact.get("size_bytes").and_then(Json::as_int),
        ) else {
            missing.push(component);
            continue;
        };
        let declared = request
            .plan_components
            .iter()
            .find(|entry| entry.get("component").and_then(Json::as_text) == Some(component));
        let version = declared
            .and_then(|entry| entry.get("version"))
            .and_then(Json::as_text)
            .unwrap_or("");
        if version.trim().is_empty() {
            return Err(Refusal::not_ready(
                format!("engine_bundle_component_undeclared:{component}"),
                format!(
                    "the release set carries verified bytes for `{component}`, but declares no \
                     version for it, and the engine's bundle manifest requires one. This layer \
                     will not invent a version to make the bundle parse. Declare one for \
                     `{component}` in the release set manifest and re-run."
                ),
            ));
        }
        let (kind, artifact_path) = match component {
            "axiom-graphd" => ("binary", format!("bin/{component}")),
            _ => {
                let basename = artifact
                    .get("url")
                    .and_then(Json::as_text)
                    .and_then(crate::update::fetch::url_basename)
                    .unwrap_or_else(|| format!("{component}.artifact"));
                ("python", format!("python/{basename}"))
            }
        };
        found.push(CoreArtifact {
            component,
            version: version.to_string(),
            kind,
            artifact: artifact_path,
            source,
            sha256: sha.to_string(),
            size_bytes: size,
        });
    }
    if !missing.is_empty() {
        return Err(Refusal::not_ready(
            "engine_bundle_components_missing",
            format!(
                "the engine's ecosystem plan requires the core components {} in `bundle.json`, \
                 but this release set verified no artifact for {}. A bundle without them would be \
                 refused by the engine after the fact, so it is not assembled at all. Nothing was \
                 installed or repaired.",
                CORE_COMPONENTS.join(" and "),
                missing.join(", ")
            ),
        ));
    }
    Ok(found)
}

/// Assemble `skills/`, from an engine-format subtree when the release set provides one.
fn assemble_skills(request: &Request<'_>) -> Result<Json, Refusal> {
    let Some(release) = request.release_root.as_deref() else {
        return assemble_installed_skills(request);
    };
    let provided = release.join(SKILLS_DIRECTORY);
    if provided.join(SKILLS_MANIFEST_FILE).is_file() {
        let destination = request.staging_root.join(SKILLS_DIRECTORY);
        let mut count = 0usize;
        copy_tree(&provided, &destination, &mut count)?;
        let manifest = std::fs::read(destination.join(SKILLS_MANIFEST_FILE)).map_err(|error| {
            Refusal::io(
                "skills_bundle_unreadable",
                &destination.join(SKILLS_MANIFEST_FILE).display().to_string(),
                &error,
            )
        })?;
        let digest = sha256::digest_hex(&manifest);
        let parsed = json::parse(&String::from_utf8_lossy(&manifest)).map_err(|error| {
            Refusal::validation(
                "skills_bundle_unreadable",
                format!(
                    "the skills bundle manifest at {} is not JSON: {error}",
                    destination.join(SKILLS_MANIFEST_FILE).display()
                ),
            )
        })?;
        verify_skills_payload(&destination, &parsed)?;
        let entries = parsed
            .get("entries")
            .and_then(Json::as_array)
            .map_or(0, <[Json]>::len);
        return Ok(Json::from_pairs(vec![
            ("source", Json::text("release_set")),
            ("component", Json::text(SKILLS_COMPONENT)),
            (
                "revision",
                parsed.get("revision").cloned().unwrap_or(Json::Null),
            ),
            (
                "version",
                parsed.get("version").cloned().unwrap_or(Json::Null),
            ),
            ("entries", Json::int(entries as i64)),
            ("files", Json::int(count as i64)),
            ("manifest_sha256", Json::text(&digest)),
        ]));
    }
    if release.join(SKILLS_SOURCE_MANIFEST_FILE).is_file() {
        return convert_skills(request, release);
    }
    Err(skills_missing(Some(release)))
}

/// Repair from the active owner bundle when the extracted release no longer exists.
fn assemble_installed_skills(request: &Request<'_>) -> Result<Json, Refusal> {
    let store = request.install_root.join("installs/ecosystem/skills");
    let read = |path: &Path| -> Result<Json, Refusal> {
        if path.is_symlink() {
            return Err(Refusal::validation(
                "skills_bundle_symlink",
                "the installed skills record is a symbolic link",
            ));
        }
        let bytes = std::fs::read(path).map_err(|_| Refusal::not_ready("installed_skills_missing", "installed skills are missing; run the install URL again to restore the complete release"))?;
        json::parse(&String::from_utf8_lossy(&bytes))
            .map_err(|error| Refusal::validation("installed_skills_invalid", error))
    };
    let pointer = read(&store.join("current"))?;
    let version = pointer.get("version").and_then(Json::as_text).unwrap_or("");
    if version.is_empty()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        || version == "."
        || version == ".."
    {
        return Err(Refusal::validation(
            "installed_skills_invalid",
            "the installed skills version is invalid",
        ));
    }
    let planned = request
        .plan_components
        .iter()
        .find(|row| row.get("component").and_then(Json::as_text) == Some(SKILLS_CHANNEL_COMPONENT));
    let source = store.join(version);
    // Refuse linked ancestors as well as linked payload files.
    let mut ancestor = source.as_path();
    while ancestor != request.install_root {
        if ancestor.is_symlink() {
            return Err(Refusal::validation(
                "skills_bundle_symlink",
                "the installed skills path is a symbolic link",
            ));
        }
        ancestor = ancestor.parent().ok_or_else(|| {
            Refusal::validation(
                "installed_skills_invalid",
                "the installed skills path escapes its root",
            )
        })?;
    }
    let manifest_path = source.join(SKILLS_MANIFEST_FILE);
    let manifest = read(&manifest_path)?;
    let expected = pointer
        .get("manifest_sha256")
        .and_then(Json::as_text)
        .unwrap_or("");
    let digest = sha256::file_hex(&manifest_path)
        .map_err(|e| Refusal::io("skills_bundle_unreadable", "installed skills manifest", &e))?;
    if !is_lowercase_hex64(expected)
        || digest != expected
        || planned.is_none()
        || ["version", "revision"].iter().any(|key| {
            let wanted = planned.and_then(|p| p.get(key)).and_then(Json::as_text);
            wanted.is_none()
                || wanted != pointer.get(key).and_then(Json::as_text)
                || wanted != manifest.get(key).and_then(Json::as_text)
        })
    {
        return Err(Refusal::validation("installed_skills_mismatch", "installed skills do not match the approved release or manifest hash; run the install URL again"));
    }
    let destination = request.staging_root.join(SKILLS_DIRECTORY);
    let payload = destination.join(SKILLS_PAYLOAD_DIRECTORY);
    std::fs::create_dir_all(&payload)
        .map_err(|e| Refusal::io("skills_bundle_unwritable", "skills repair staging", &e))?;
    let mut count = 0;
    copy_tree(&source, &payload, &mut count)?;
    std::fs::remove_file(payload.join(SKILLS_MANIFEST_FILE))
        .map_err(|e| Refusal::io("skills_bundle_unwritable", "skills repair manifest", &e))?;
    std::fs::copy(&manifest_path, destination.join(SKILLS_MANIFEST_FILE))
        .map_err(|e| Refusal::io("skills_bundle_unwritable", "skills repair manifest", &e))?;
    let copied = read(&destination.join(SKILLS_MANIFEST_FILE))?;
    if sha256::file_hex(&destination.join(SKILLS_MANIFEST_FILE))
        .ok()
        .as_deref()
        != Some(expected)
    {
        return Err(Refusal::validation(
            "installed_skills_mismatch",
            "the skills manifest changed during repair",
        ));
    }
    verify_skills_payload(&destination, &copied)?;
    Ok(Json::from_pairs(vec![
        ("source", Json::text("installed_bundle")),
        ("component", Json::text(SKILLS_COMPONENT)),
        ("version", Json::text(version)),
        (
            "revision",
            pointer.get("revision").cloned().unwrap_or(Json::Null),
        ),
        ("manifest_sha256", Json::text(expected)),
        (
            "entries",
            Json::int(
                copied
                    .get("entries")
                    .and_then(Json::as_array)
                    .map_or(0, |e| e.len()) as i64,
            ),
        ),
    ]))
}

/// Re-hash every payload an engine-format skills manifest declares.
///
/// The engine verifies the skills bundle at its own `plan` step, so a bad bundle would be caught
/// there. It is caught *here* instead, because this layer's rule is that it hands over verified
/// bytes only: a release set whose `skills/bundle.json` and `skills/payload/**` disagree is
/// refused by name, before the engine is invoked at all.
fn verify_skills_payload(destination: &Path, manifest: &Json) -> Result<(), Refusal> {
    let manifest_file = destination.join(SKILLS_MANIFEST_FILE);
    let Some(entries) = manifest.get("entries").and_then(Json::as_array) else {
        return Err(Refusal::validation(
            "skills_bundle_manifest_invalid",
            format!(
                "the skills bundle manifest at {} declares no `entries` array",
                manifest_file.display()
            ),
        ));
    };
    let payload = destination.join(SKILLS_PAYLOAD_DIRECTORY);
    for entry in entries {
        let path = entry.get("path").and_then(Json::as_text).unwrap_or("");
        let sha = entry.get("sha256").and_then(Json::as_text).unwrap_or("");
        let size = entry.get("size_bytes").and_then(Json::as_int).unwrap_or(-1);
        if !is_portable_relative_path(path) || !is_lowercase_hex64(sha) || size < 0 {
            return Err(Refusal::validation(
                "skills_bundle_manifest_invalid",
                format!(
                    "a declared entry in {} is not a portable relative path with a lowercase \
                     64-hex digest and a byte length: {path:?}",
                    manifest_file.display()
                ),
            ));
        }
        let file = payload.join(path);
        crate::update::fetch::verify_file(&file, sha, size).map_err(|refusal| {
            Refusal::new(
                refusal.class,
                format!("skills_payload_unverified:{path}"),
                format!(
                    "the skills payload `{path}` in the release set failed verification: {}",
                    refusal.message
                ),
            )
        })?;
    }
    // Every regular file under the payload tree must be declared. The bundle is assembled by
    // copying the release set's tree, so without this walk an undeclared file would sit inside the
    // bundle unhashed. The engine refuses such a bundle at its own plan step, but this layer's
    // rule is that it hands over verified bytes only: it refuses first and names the file.
    let declared: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.get("path").and_then(Json::as_text))
        .collect();
    let mut undeclared: Vec<String> = Vec::new();
    collect_undeclared(&payload, &payload, &declared, &mut undeclared)?;
    if !undeclared.is_empty() {
        undeclared.sort();
        return Err(Refusal::validation(
            "skills_payload_undeclared",
            format!(
                "the skills payload tree at {} holds {} file(s) that {} does not declare (first:                  `{}`). Every byte handed to the engine must be one this layer hashed against the                  release set manifest; an undeclared payload is not.",
                payload.display(),
                undeclared.len(),
                manifest_file.display(),
                undeclared[0]
            ),
        ));
    }
    Ok(())
}

/// Collect the payload files under `directory` that `declared` does not name.
///
/// Paths are reported bundle-relative with `/` separators, matching the manifest's own spelling.
/// A symlink is refused here too: `copy_tree` refuses one on the way in, and a payload tree that
/// acquired one by another route must not reach the engine.
fn collect_undeclared(
    root: &Path,
    directory: &Path,
    declared: &[&str],
    undeclared: &mut Vec<String>,
) -> Result<(), Refusal> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        Refusal::io(
            "skills_bundle_unreadable",
            &directory.display().to_string(),
            &error,
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            Refusal::io(
                "skills_bundle_unreadable",
                &directory.display().to_string(),
                &error,
            )
        })?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            Refusal::io(
                "skills_bundle_unreadable",
                &path.display().to_string(),
                &error,
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(Refusal::validation(
                "skills_bundle_symlink",
                format!(
                    "{} is a symbolic link; the engine's skills bundle declares regular files only",
                    path.display()
                ),
            ));
        }
        if metadata.is_dir() {
            collect_undeclared(root, &path, declared, undeclared)?;
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if !declared.contains(&relative.as_str()) {
            undeclared.push(relative);
        }
    }
    Ok(())
}

/// True when `path` is a bundle-relative path this layer will resolve under the payload root.
///
/// The manifest is part of a release set, so a path it names must not be able to escape the
/// payload directory: no absolute path, no `..` segment, no backslash, no empty segment.
fn is_portable_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// True when `value` is a lowercase 64-hex digest.
fn is_lowercase_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn skills_missing(release: Option<&Path>) -> Refusal {
    let where_looked = match release {
        Some(path) => format!(
            "{} (a `{SKILLS_DIRECTORY}/{SKILLS_MANIFEST_FILE}` tree) and {} (an axiom-skills \
             source manifest)",
            path.display(),
            path.join(SKILLS_SOURCE_MANIFEST_FILE).display()
        ),
        None => "the release set directory".to_string(),
    };
    Refusal::not_ready(
        "engine_bundle_skills_missing",
        format!(
            "the engine's ecosystem plan requires a skills bundle at \
             `{SKILLS_DIRECTORY}/{SKILLS_MANIFEST_FILE}` with its payload under \
             `{SKILLS_DIRECTORY}/{SKILLS_PAYLOAD_DIRECTORY}/`, and this release set provides none: \
             looked for {where_looked}. A skills bundle is not a component artifact in the \
             distribution channel, so it must be supplied alongside the release set. Nothing was \
             installed or repaired."
        ),
    )
}

/// Convert an `axiom-skills` source manifest into the engine's skills bundle.
///
/// The two schemas differ: the source calls a file's kind a `role` and its path a repository
/// path, while `SkillBundle` calls it `kind` and a bundle-relative path. The conversion is
/// deliberately strict about the two facts the source does not carry and this layer must not
/// invent: a pinned `axiom-specs` revision, and the capability review of any executable entry.
fn convert_skills(request: &Request<'_>, release: &Path) -> Result<Json, Refusal> {
    let source_file = release.join(SKILLS_SOURCE_MANIFEST_FILE);
    let text = std::fs::read_to_string(&source_file).map_err(|error| {
        Refusal::io(
            "skills_source_unreadable",
            &source_file.display().to_string(),
            &error,
        )
    })?;
    let source = json::parse(&text).map_err(|error| {
        Refusal::validation(
            "skills_source_unreadable",
            format!("{} is not JSON: {error}", source_file.display()),
        )
    })?;

    // The pinned revision of the skills pack itself: the channel declares it.
    let revision = request
        .plan_components
        .iter()
        .find(|entry| {
            entry.get("component").and_then(Json::as_text) == Some(SKILLS_CHANNEL_COMPONENT)
        })
        .and_then(|entry| entry.get("revision"))
        .and_then(Json::as_text)
        .unwrap_or("");
    require_pinned(revision).map_err(|_| {
        Refusal::not_ready(
            "skills_revision_not_pinned",
            format!(
                "the release set declares no pinned 40-hex revision for the `{SKILLS_CHANNEL_COMPONENT}` \
                 component (observed {revision:?}), but the engine's skills bundle manifest requires \
                 one. Nothing was installed or repaired."
            ),
        )
    })?;

    // The `axiom-specs` revision the bundle was reviewed against: only the source can declare it.
    let spec_revision = source
        .get("spec_revision")
        .and_then(Json::as_text)
        .unwrap_or("");
    require_pinned(spec_revision).map_err(|_| {
        Refusal::not_ready(
            "skills_spec_revision_not_pinned",
            format!(
                "{} declares no pinned 40-hex `spec_revision` (observed {spec_revision:?}). The \
                 engine's skills bundle records the immutable axiom-specs revision the bundle was \
                 reviewed against, and this layer will not invent one. Add `spec_revision` to the \
                 source manifest, or supply the skills bundle already converted.",
                source_file.display()
            ),
        )
    })?;

    let version = source
        .get("component_version")
        .and_then(Json::as_text)
        .unwrap_or("");
    if version.trim().is_empty() {
        return Err(Refusal::not_ready(
            "skills_version_undeclared",
            format!(
                "{} declares no `component_version`, and the engine's skills bundle manifest \
                 requires one",
                source_file.display()
            ),
        ));
    }

    let payload_root = request
        .staging_root
        .join(SKILLS_DIRECTORY)
        .join(SKILLS_PAYLOAD_DIRECTORY);
    reset_directory(&payload_root)?;
    let payload_files_root = release.join(SKILLS_PAYLOAD_DIRECTORY);
    let files: Vec<Json> = source
        .get("files")
        .and_then(Json::as_array)
        .map(<[Json]>::to_vec)
        .unwrap_or_default();
    if files.is_empty() {
        return Err(Refusal::not_ready(
            "skills_source_empty",
            format!("{} declares no files to bundle", source_file.display()),
        ));
    }
    let mut entries: Vec<Json> = Vec::with_capacity(files.len());
    for file in &files {
        let path = file.get("path").and_then(Json::as_text).unwrap_or("");
        let role = file.get("role").and_then(Json::as_text).unwrap_or("");
        let expected = file.get("sha256").and_then(Json::as_text).unwrap_or("");
        let bytes = file.get("bytes").and_then(Json::as_int).unwrap_or(0);
        if path.is_empty() || !has_declared_shape(path) {
            return Err(Refusal::validation(
                "skills_path_shape_not_declared",
                format!("`{path}` is not a bundle-relative file path the engine accepts"),
            ));
        }
        let kind = skills_kind(role, path);
        let capabilities: Vec<String> = file
            .get("capabilities")
            .and_then(Json::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Json::as_text)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if is_executable_path(path) {
            if kind != "script" {
                return Err(Refusal::validation(
                    "skills_entry_kind_invalid",
                    format!("`{path}` can run code, so its entry kind must be `script`"),
                ));
            }
            if capabilities.is_empty() {
                return Err(Refusal::not_ready(
                    format!("skills_capability_review_required:{path}"),
                    format!(
                        "`{path}` is executable, and the engine installs an executable skill entry \
                         only against a capability review ({}) that the source manifest does not \
                         declare. This layer will not invent one. Add `capabilities` to that entry \
                         in {}.",
                        CAPABILITIES.join(", "),
                        source_file.display()
                    ),
                ));
            }
            for capability in &capabilities {
                if !CAPABILITIES.contains(&capability.as_str()) {
                    return Err(Refusal::validation(
                        format!("skills_capability_not_reviewed:{path}"),
                        format!(
                            "`{path}` declares the capability `{capability}`, which is outside the \
                             engine's allowlist ({})",
                            CAPABILITIES.join(", ")
                        ),
                    ));
                }
            }
        }
        if bytes < 0 || bytes as u64 > MAX_ENTRY_BYTES {
            return Err(Refusal::validation(
                format!("skills_entry_too_large:{path}"),
                format!("`{path}` declares {bytes} bytes, above the engine's {MAX_ENTRY_BYTES}"),
            ));
        }
        let source_file_on_disk = release.join(path);
        let destination = payload_root.join(path);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                Refusal::io(
                    "skills_payload_unwritable",
                    &parent.display().to_string(),
                    &error,
                )
            })?;
        }
        std::fs::copy(&source_file_on_disk, &destination).map_err(|error| {
            Refusal::io(
                format!("skills_payload_unreachable:{path}"),
                &source_file_on_disk.display().to_string(),
                &error,
            )
        })?;
        let observed = sha256::file_hex(&destination).map_err(|error| {
            Refusal::io(
                "skills_payload_unreadable",
                &destination.display().to_string(),
                &error,
            )
        })?;
        if observed != expected {
            return Err(Refusal::validation(
                format!("skills_payload_unverified:{path}"),
                format!(
                    "`{path}` hashes to {observed}, not the recorded {expected}; the skills bundle \
                     was not assembled"
                ),
            ));
        }
        // The field is written even when it is empty: the engine's `DeclaredEntry` declares
        // `capabilities` without a serde default, so an entry that omits it is not a declaration
        // the engine accepts. An empty review is a review, and it is stated rather than implied.
        let mut entry = Json::from_pairs(vec![
            ("path", Json::text(path)),
            ("kind", Json::text(kind)),
            ("sha256", Json::text(expected)),
            ("size_bytes", Json::int(bytes)),
        ]);
        entry
            .set(
                "capabilities",
                Json::array(capabilities.iter().map(|item| Json::text(item)).collect()),
            )
            .map_err(|error| Refusal::validation("skills_entry_unbuildable", error.to_string()))?;
        entries.push(entry);
    }

    let manifest = Json::from_pairs(vec![
        ("schema_version", Json::int(BUNDLE_SCHEMA_VERSION)),
        // `SkillBundle::validate` requires the *bundle* component `skills`; `axiom-skills` is the
        // ecosystem plan's component id, and naming it here is refused as `unexpected_component`.
        ("component", Json::text(SKILLS_CHANNEL_COMPONENT)),
        ("version", Json::text(version)),
        ("revision", Json::text(revision)),
        ("spec_revision", Json::text(spec_revision)),
        ("entries", Json::array(entries.clone())),
    ]);
    let manifest_file = request
        .staging_root
        .join(SKILLS_DIRECTORY)
        .join(SKILLS_MANIFEST_FILE);
    if let Some(parent) = manifest_file.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            Refusal::io(
                "skills_bundle_unwritable",
                &parent.display().to_string(),
                &error,
            )
        })?;
    }
    let bytes = json::canonical_bytes(&manifest);
    std::fs::write(&manifest_file, &bytes).map_err(|error| {
        Refusal::io(
            "skills_bundle_unwritable",
            &manifest_file.display().to_string(),
            &error,
        )
    })?;
    let _ = payload_files_root;
    Ok(Json::from_pairs(vec![
        ("source", Json::text("skills_manifest_converted")),
        ("component", Json::text(SKILLS_COMPONENT)),
        ("version", Json::text(version)),
        ("revision", Json::text(revision)),
        ("spec_revision", Json::text(spec_revision)),
        ("entries", Json::int(entries.len() as i64)),
        ("manifest_sha256", Json::text(&sha256::digest_hex(&bytes))),
    ]))
}

/// The engine entry kind for one source `role` and path.
fn skills_kind(role: &str, path: &str) -> &'static str {
    if is_executable_path(path) {
        return "script";
    }
    if ENTRY_KINDS.contains(&role) {
        return match role {
            "instruction" => "instruction",
            "reference" => "reference",
            "asset" => "asset",
            _ => "script",
        };
    }
    // A named skill is an instruction; every other reviewed document is a reference.
    if role == "skill" {
        "instruction"
    } else {
        "reference"
    }
}

/// True when an entry path can run code, by the engine's suffix list.
fn is_executable_path(path: &str) -> bool {
    let Some(name) = path.rsplit('/').next() else {
        return false;
    };
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty() && EXECUTABLE_SUFFIXES.contains(&extension.to_ascii_lowercase().as_str())
}

/// True when a declared path has the shape of a file inside a named skill.
fn has_declared_shape(path: &str) -> bool {
    let mut segments = path.split('/');
    let Some(root) = segments.next() else {
        return false;
    };
    let rest: Vec<&str> = segments.collect();
    if root.is_empty() || rest.is_empty() {
        return false;
    }
    let Some(name) = rest.last() else {
        return false;
    };
    match name.rsplit_once('.') {
        Some((stem, extension)) => !stem.is_empty() && !extension.is_empty(),
        None => false,
    }
}

/// True when `value` is a pinned 40-hex revision.
fn require_pinned(value: &str) -> Result<(), ()> {
    let pinned = value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if pinned {
        Ok(())
    } else {
        Err(())
    }
}

/// Replace a directory with an empty one.
fn reset_directory(path: &Path) -> Result<(), Refusal> {
    if path.exists() {
        std::fs::remove_dir_all(path).map_err(|error| {
            Refusal::io(
                "bundle_staging_unwritable",
                &path.display().to_string(),
                &error,
            )
        })?;
    }
    std::fs::create_dir_all(path).map_err(|error| {
        Refusal::io(
            "bundle_staging_unwritable",
            &path.display().to_string(),
            &error,
        )
    })
}

/// Copy a directory tree, refusing any symlink.
///
/// The engine refuses a bundle whose payload is not a regular file, so a symlink here would only
/// move the refusal later and lose the reason. It is refused where it is found.
fn copy_tree(source: &Path, destination: &Path, files: &mut usize) -> Result<(), Refusal> {
    std::fs::create_dir_all(destination).map_err(|error| {
        Refusal::io(
            "skills_bundle_unwritable",
            &destination.display().to_string(),
            &error,
        )
    })?;
    let entries = std::fs::read_dir(source).map_err(|error| {
        Refusal::io(
            "skills_bundle_unreadable",
            &source.display().to_string(),
            &error,
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            Refusal::io(
                "skills_bundle_unreadable",
                &source.display().to_string(),
                &error,
            )
        })?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&from).map_err(|error| {
            Refusal::io(
                "skills_bundle_unreadable",
                &from.display().to_string(),
                &error,
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(Refusal::validation(
                "skills_bundle_symlink",
                format!(
                    "{} is a symbolic link; the engine's skills bundle declares regular files only",
                    from.display()
                ),
            ));
        }
        if metadata.is_dir() {
            copy_tree(&from, &to, files)?;
        } else {
            std::fs::copy(&from, &to).map_err(|error| {
                Refusal::io(
                    "skills_bundle_unwritable",
                    &to.display().to_string(),
                    &error,
                )
            })?;
            *files += 1;
        }
    }
    Ok(())
}

/// Artifacts the release set declares that the engine bundle does not carry.
fn not_carried(request: &Request<'_>) -> (Vec<Json>, Vec<&'static str>) {
    let mut skipped: Vec<Json> = Vec::new();
    for artifact in request.verified {
        let component = artifact
            .get("component")
            .and_then(Json::as_text)
            .unwrap_or("?");
        let reason = match component {
            "axiom-graphd" | "axiom-mcp" => continue,
            "axiom" => {
                "the engine's ecosystem plan orders `axiom-graphd` then `axiom-mcp` and refuses a \
                 third core entry; the `axiom` entrypoint shares axiom-graphd's release"
            }
            "axiom-cli" => {
                "the engine's bundle components do not include `axiom-cli`; this layer never \
                 installs itself"
            }
            _ => "not a component of the engine's bundle vocabulary",
        };
        skipped.push(Json::from_pairs(vec![
            ("component", Json::text(component)),
            ("reason", Json::text(reason)),
        ]));
    }
    (skipped, CORE_COMPONENTS.to_vec())
}

/// Parse the engine's stdout as one JSON object, when it wrote one.
fn engine_json(outcome: &Outcome, step: &str) -> Result<Option<Json>, Refusal> {
    let text = outcome.stdout.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match json::parse(text) {
        Ok(value) => Ok(Some(value)),
        Err(error) => Err(Refusal::new(
            Class::IoInternal,
            "engine_response_unparsable",
            format!(
                "the engine's `install {step}` step exited {} but its stdout is not one JSON \
                 object ({error}); this layer will not report a placement it cannot read. stdout \
                 was {} bytes, stderr: {}",
                outcome.exit_code(),
                outcome.stdout.len(),
                outcome.stderr.trim()
            ),
        )),
    }
}

/// The engine's plan identity, from its summary stdout and the plan file it wrote.
fn plan_identity(
    value: &Option<Json>,
    plan_file: &Path,
    outcome: &Outcome,
) -> Result<(String, String), Refusal> {
    let digest = value
        .as_ref()
        .and_then(|value| value.get("plan_digest"))
        .and_then(Json::as_text)
        .unwrap_or("");
    let plan_id = value
        .as_ref()
        .and_then(|value| value.get("plan_id"))
        .and_then(Json::as_text)
        .unwrap_or("")
        .to_string();
    if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Refusal::new(
            Class::IoInternal,
            "engine_response_unparsable",
            "the engine's `install plan` step exited 0 but reported no 64-hex plan_digest; \
             nothing was applied",
        ));
    }
    if !plan_file.is_file() {
        return Err(Refusal::new(
            Class::IoInternal,
            "engine_plan_unwritten",
            format!(
                "the engine's `install plan` step exited 0 but wrote no plan file at {} (stderr: \
                 {}); nothing was applied",
                plan_file.display(),
                outcome.stderr.trim()
            ),
        ));
    }
    Ok((digest.to_string(), plan_id))
}

/// One engine step's raw evidence.
fn step_evidence(step: &str, outcome: &Outcome, value: Option<&Json>) -> Json {
    Json::from_pairs(vec![
        ("step", Json::text(step)),
        ("exit_code", Json::int(i64::from(outcome.exit_code()))),
        (
            "stdout",
            Json::text(outcome.stdout.trim_end_matches(['\n', '\r'])),
        ),
        (
            "stderr",
            Json::text(outcome.stderr.trim_end_matches(['\n', '\r'])),
        ),
        (
            "code",
            match value
                .and_then(|value| value.get("code"))
                .and_then(Json::as_text)
            {
                Some(code) => Json::text(code),
                None => Json::Null,
            },
        ),
        (
            "status",
            match value
                .and_then(|value| value.get("status"))
                .and_then(Json::as_text)
            {
                Some(status) => Json::text(status),
                None => Json::Null,
            },
        ),
    ])
}

/// The engine block carried in every `install --apply` report.
fn engine_block(request: &Request<'_>, assembly: &Assembly, steps: Vec<Json>) -> Json {
    Json::from_pairs(vec![
        (
            "bundle_root",
            Json::text(&assembly.root.display().to_string()),
        ),
        (
            "bundle_manifest_sha256",
            Json::text(&assembly.manifest_sha256),
        ),
        (
            "bundle_manifest_bytes",
            Json::int(assembly.manifest_bytes as i64),
        ),
        ("bundle_components", Json::array(assembly.core.clone())),
        ("skills", assembly.skills.clone()),
        ("not_carried", Json::array(assembly.not_carried.clone())),
        ("host", Json::text(&request.host)),
        (
            "install_root",
            Json::text(&request.install_root.display().to_string()),
        ),
        ("steps", Json::array(steps)),
    ])
}

/// Build the report for an engine refusal, carrying the engine's own exit code.
fn refused(
    request: &Request<'_>,
    assembly: &Assembly,
    steps: Vec<Json>,
    verb: &str,
    step: &str,
    outcome: &Outcome,
    value: Option<&Json>,
) -> Report {
    let engine_code = value
        .and_then(|value| value.get("code"))
        .and_then(Json::as_text)
        .unwrap_or("");
    let (exit_code, status) = engine_vocabulary(outcome.exit_code(), engine_code);
    let engine_message = value
        .and_then(|value| value.get("message"))
        .and_then(Json::as_text)
        .unwrap_or("");
    let message = if engine_message.is_empty() {
        let stderr = outcome.stderr.trim();
        if stderr.is_empty() {
            format!(
                "the engine's `{verb} {step}` step exited {exit_code} without a readable reason"
            )
        } else {
            format!("the engine's `{verb} {step}` step exited {exit_code}: {stderr}")
        }
    } else {
        engine_message.to_string()
    };
    let mut report = Report::new(exit_code, status, message.clone()).subject(if verb == "update" {
        "update"
    } else {
        "install"
    });
    if matches!(
        value
            .and_then(|value| value.get("retryable"))
            .and_then(Json::as_bool),
        Some(true)
    ) {
        report = report.retryable();
    }
    report.detail("reason", Json::text(&message));
    report.detail(
        "reason_code",
        Json::text(&format!(
            "engine_{}",
            if engine_code.is_empty() {
                format!("exit_{exit_code}")
            } else {
                engine_code.to_ascii_lowercase()
            }
        )),
    );
    report.detail("engine", engine_block(request, assembly, steps));
    report.detail("engine_step", Json::text(step));
    report.detail(
        "engine_exit_code",
        Json::int(i64::from(outcome.exit_code())),
    );
    report.detail(
        "engine_code",
        if engine_code.is_empty() {
            Json::Null
        } else {
            Json::text(engine_code)
        },
    );
    if let Some(details) = value.and_then(|value| value.get("details")) {
        report.detail("engine_details", details.clone());
    }
    report
}

/// The canonical exit vocabulary of this CLI, which is also the engine's.
const CANONICAL: [i32; 11] = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20];

/// Map an engine exit code plus its code string onto the shared vocabulary.
///
/// The engine's exit code is authoritative: the two vocabularies are the same eleven codes. The
/// code string only names the status token, so an engine that grows a code string this build does
/// not know still reports its real exit code.
fn engine_vocabulary(exit_code: i32, engine_code: &str) -> (i32, &'static str) {
    let status = match engine_code {
        "VALIDATION_ERROR" => "validation_error",
        "NOT_FOUND" => "not_found",
        "NOT_READY" => "not_ready",
        "FORBIDDEN" => "authorization_error",
        "CONFLICT" => "conflict",
        "TIMEOUT_BUSY" | "TIMEOUT" => "timeout_busy",
        "IO_ERROR" | "IO_INTERNAL" => "io_error",
        "INCOMPATIBLE" => "incompatible",
        "LOCK_UNAVAILABLE" => "lock_unavailable",
        "PARTIAL" => "partial",
        _ => "",
    };
    if CANONICAL.contains(&exit_code) && exit_code != 0 {
        let status = if status.is_empty() {
            "io_error"
        } else {
            status
        };
        return (exit_code, status);
    }
    if !status.is_empty() {
        let code = match status {
            "validation_error" => 2,
            "not_found" => 3,
            "not_ready" => 4,
            "authorization_error" => 5,
            "conflict" => 6,
            "timeout_busy" => 7,
            "incompatible" => 9,
            "lock_unavailable" => 10,
            "partial" => 20,
            _ => 8,
        };
        return (code, status);
    }
    // An exit code outside the vocabulary, or a signal, is an internal failure of the boundary.
    (8, "io_error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_skill_is_an_instruction_and_a_reviewed_document_is_a_reference() {
        assert_eq!(
            skills_kind("skill", "skills/graph-context/SKILL.md"),
            "instruction"
        );
        assert_eq!(skills_kind("policy", "policy/POLICY.md"), "reference");
        assert_eq!(skills_kind("asset", "skills/a/data.json"), "asset");
        assert_eq!(
            skills_kind("plugin-package", "plugins/axiom/plugin.json"),
            "reference"
        );
    }

    #[test]
    fn an_executable_suffix_forces_the_script_kind() {
        // The engine refuses a `.py` entry whose kind is not `script`, so the converter must not
        // let a source `role` override the suffix.
        assert_eq!(
            skills_kind("host-hook", "adapters/hooks/graph_stop.py"),
            "script"
        );
        assert_eq!(skills_kind("skill", "skills/a/run.sh"), "script");
        assert!(!is_executable_path("skills/a/SKILL.md"));
        assert!(is_executable_path("skills/a/SKILL.PY"));
    }

    #[test]
    fn a_declared_path_needs_a_named_root_and_a_filename() {
        assert!(has_declared_shape("skills/graph-context/SKILL.md"));
        assert!(has_declared_shape("policy/POLICY.md"));
        assert!(has_declared_shape(".agents/plugins/marketplace.json"));
        assert!(!has_declared_shape("SKILL.md"));
        assert!(!has_declared_shape("skills/"));
        assert!(!has_declared_shape("skills/graph-context/"));
    }

    #[test]
    fn only_a_pinned_lowercase_hex_revision_is_pinned() {
        assert!(require_pinned(&"a".repeat(40)).is_ok());
        assert!(require_pinned("0123456789abcdef0123456789abcdef01234567").is_ok());
        assert!(require_pinned(&"A".repeat(40)).is_err());
        assert!(require_pinned(&"a".repeat(39)).is_err());
        assert!(require_pinned("").is_err());
    }

    #[test]
    fn the_engine_vocabulary_keeps_a_real_engine_exit_code() {
        // The engine's own code wins: 20 is in the vocabulary even though this CLI has no
        // `partial` class of its own.
        assert_eq!(
            engine_vocabulary(5, "FORBIDDEN"),
            (5, "authorization_error")
        );
        assert_eq!(engine_vocabulary(20, "PARTIAL"), (20, "partial"));
        assert_eq!(engine_vocabulary(3, ""), (3, "io_error"));
        // A code string with no exit code is still mapped rather than dropped.
        assert_eq!(engine_vocabulary(0, "NOT_READY"), (4, "not_ready"));
        // An unknown code with a non-canonical exit is an internal failure, not a success.
        assert_eq!(engine_vocabulary(99, "SOMETHING_NEW"), (8, "io_error"));
    }

    #[test]
    fn a_bundle_without_a_declared_version_is_refused_before_the_engine_sees_it() {
        // The engine's `BundleComponent.version` is required, so an undeclared version must be a
        // refusal here rather than a fabricated `0.0.0-dev`.
        let verified = vec![Json::from_pairs(vec![
            ("component", Json::text("axiom-graphd")),
            ("url", Json::text("https://example.invalid/axiom-graphd")),
            ("sha256", Json::text(&"a".repeat(64))),
            ("size_bytes", Json::int(10)),
            ("resolved", Json::text("/nowhere/axiom-graphd")),
        ])];
        let plan_components = vec![Json::from_pairs(vec![
            ("component", Json::text("axiom-graphd")),
            ("version", Json::Null),
        ])];
        let request = Request {
            staging_root: std::env::temp_dir().join("axiom-bundle-unit-unused"),
            install_root: std::env::temp_dir().join("axiom-bundle-unit-unused-root"),
            host: "macos-x64".to_string(),
            channel: "stable".to_string(),
            created_at: "2026-09-20T00:00:00Z".to_string(),
            release_root: None,
            verified: &verified,
            plan_components: &plan_components,
            mcp_runtime_bin: None,
        };
        let refusal = core_artifacts(&request).expect_err("an undeclared version must be refused");
        assert_eq!(
            refusal.reason,
            "engine_bundle_component_undeclared:axiom-graphd"
        );
        assert_eq!(refusal.class, Class::NotReady);
    }
}
