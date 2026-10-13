//! The update plan document: the canonical contract evaluator and the planner.
//!
//! The canonical implementation of this contract is
//! `axiom-specs/tools/update_plan_contract.py` together with
//! `axiom-specs/contracts/schemas/update-plan.schema.json`. This module mirrors the semantic
//! rules of that script in Rust so the CLI can refuse a plan itself instead of delegating the
//! verdict, and it emits the plan digest with the same canonical input
//! (`canonical_plan_bytes`: UTF-8, lexicographic keys, compact separators, one trailing LF).
//!
//! Mirroring matters for one rule in particular, the one AC1 is written around: the digest
//! covers every planner input - channel, host, install root, every component version and
//! revision, every artifact hash, the migration set, backup, disk headroom, service
//! interruptions and the rollback posture. `axiom-cli` is itself one of those components, so a
//! self-update and the component updates in the same plan are one transaction with one
//! approval digest. A different component version, a different host, a different channel or a
//! different install root cannot reuse the old approval.
//!
//! This module never reads a branch tip, a tag alias, a network `latest`, `HEAD` or `*`: every
//! version it writes comes from the channel manifest the installed release recorded.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::error::Refusal;
use super::json::{self, canonical_bytes, Json};
use super::rules;
use super::sha256;
use super::time::Stamp;

/// Plan schema version.
pub const SCHEMA_VERSION: i64 = 1;
/// Spec version recorded in every plan this CLI builds.
pub const SPEC_VERSION: &str = "2.0.0-draft.1";
/// How long a freshly built plan stays valid.
pub const EXPIRY_SECONDS: i64 = 900;
/// Flat headroom added on top of the staged bytes, so a nearly full disk refuses early.
pub const DISK_HEADROOM_FLOOR: i64 = 64 * 1024 * 1024;

/// Every field a plan may carry, in the frozen key order of the canonical contract.
pub const PLAN_KEYS: [&str; 19] = [
    "schema_version",
    "spec_version",
    "plan_id",
    "created_at",
    "expires_at",
    "channel",
    "target",
    "components",
    "downloads",
    "migrations",
    "backup",
    "disk_headroom_bytes",
    "service_interruptions",
    "host_reconnect",
    "bootstrap_changes",
    "rollback",
    "trust",
    "approval",
    "plan_digest",
];

/// Fields excluded from the digest input: the digest cannot cover itself, and the approval
/// record is what the digest is compared against.
pub const DIGEST_EXCLUDED: [&str; 2] = ["plan_digest", "approval"];

const ACTIONS: [&str; 4] = ["install", "upgrade", "reinstall", "noop"];
const INTERRUPTION_ACTIONS: [&str; 3] = ["stop", "restart", "none"];
const APPROVAL_STATES: [&str; 2] = ["unapproved", "approved"];

/// The digest body: every plan field except `plan_digest` and `approval`.
pub fn digest_body(plan: &Json) -> Option<Json> {
    let object = plan.as_object()?;
    let mut body: BTreeMap<String, Json> = BTreeMap::new();
    for (key, value) in object {
        if !DIGEST_EXCLUDED.contains(&key.as_str()) {
            body.insert(key.clone(), value.clone());
        }
    }
    Some(Json::Object(body))
}

/// The plan digest: sha256 of the canonical digest body.
pub fn digest(plan: &Json) -> Option<String> {
    let body = digest_body(plan)?;
    Some(sha256::digest_hex(&canonical_bytes(&body)))
}

fn is_text(value: Option<&Json>) -> bool {
    match value {
        Some(Json::Text(text)) => !text.trim().is_empty(),
        _ => false,
    }
}

fn is_count(value: Option<&Json>, minimum: i64) -> bool {
    match value {
        Some(Json::Int(number)) => *number >= minimum,
        _ => false,
    }
}

fn is_bool(value: Option<&Json>) -> bool {
    matches!(value, Some(Json::Bool(_)))
}

fn text_of(value: Option<&Json>) -> Option<String> {
    value.and_then(Json::as_text).map(str::to_string)
}

fn placeholders(value: &Json, path: &str) -> Vec<String> {
    let mut found = Vec::new();
    match value {
        Json::Text(text) => {
            if rules::contains_placeholder(text) {
                found.push(path.to_string());
            }
        }
        Json::Object(object) => {
            for (key, item) in object {
                found.extend(placeholders(item, &format!("{path}.{key}")));
            }
        }
        Json::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                found.extend(placeholders(item, &format!("{path}[{index}]")));
            }
        }
        _ => {}
    }
    found
}

fn blank_required(object: &BTreeMap<String, Json>, keys: &[&str], label: &str) -> Vec<String> {
    let mut reasons = Vec::new();
    for key in keys {
        if !object.contains_key(*key) {
            reasons.push(format!("missing_required_field:{label}.{key}"));
        }
    }
    reasons
}

/// Read a plan document from disk.
pub fn read_plan_file(path: &Path) -> Result<Json, Refusal> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        Refusal::new(
            super::error::Class::NotFound,
            "plan_unreadable",
            format!("cannot read the plan document {}: {error}", path.display()),
        )
    })?;
    json::parse(&text).map_err(|error| {
        Refusal::validation(
            "plan_not_json",
            format!(
                "the plan document {} is not valid JSON: {error}",
                path.display()
            ),
        )
    })
}

/// Compare a plan against an approval recorded outside the plan.
///
/// This is the AC1 boundary: an approval names one digest, and the digest covers every planner
/// input, so re-using an old approval for a changed plan is refused as stale.
pub fn approval_reasons(plan: &Json, approved_digest: &str) -> Vec<String> {
    if plan.as_object().is_none() {
        return vec!["plan_not_object".to_string()];
    }
    if !rules::is_digest64(approved_digest) {
        return vec!["approved_digest_not_a_digest".to_string()];
    }
    let mut reasons = Vec::new();
    match digest(plan) {
        Some(computed) if computed == approved_digest => {}
        _ => reasons.push("approval_stale".to_string()),
    }
    if plan.get("plan_digest").and_then(Json::as_text) != Some(approved_digest) {
        reasons.push("plan_digest_not_approved".to_string());
    }
    reasons
}

/// One human-readable sentence for a reason list.
pub fn describe(reasons: &[String]) -> String {
    if reasons.is_empty() {
        return "accepted".to_string();
    }
    format!("refused: {}", reasons.join("; "))
}
/// Named rejection reasons for one plan document, mirroring the canonical evaluator.
///
/// An empty list means accepted.
pub fn structural_reasons(plan: &Json) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();
    let Some(object) = plan.as_object() else {
        return vec!["plan_not_object".to_string()];
    };
    for key in object.keys() {
        if !PLAN_KEYS.contains(&key.as_str()) {
            reasons.push(format!("undeclared_plan_field:{key}"));
        }
    }
    for key in PLAN_KEYS {
        if !object.contains_key(key) {
            reasons.push(format!("missing_required_field:{key}"));
        }
    }
    for path in placeholders(plan, "$") {
        reasons.push(format!("unresolved_placeholder:{path}"));
    }
    if !reasons.is_empty()
        && reasons.iter().any(|reason| {
            reason.starts_with("plan_not_object")
                || reason.starts_with("missing_required_field")
                || reason.starts_with("undeclared_plan_field")
        })
    {
        // Shape is already unusable; deeper checks would only report noise.
        return reasons;
    }

    if object.get("schema_version").and_then(Json::as_int) != Some(SCHEMA_VERSION) {
        reasons.push("unsupported_schema_version".to_string());
    }
    match text_of(object.get("spec_version")) {
        Some(version) if rules::is_semver(&version) => {}
        _ => reasons.push(format!(
            "invalid_spec_version:{}",
            diag(object.get("spec_version"))
        )),
    }
    match text_of(object.get("plan_id")) {
        Some(id) if rules::is_plan_id(&id) => {}
        _ => reasons.push("invalid_plan_id".to_string()),
    }
    let created = text_of(object.get("created_at"));
    let expires = text_of(object.get("expires_at"));
    for (field, value) in [("created_at", &created), ("expires_at", &expires)] {
        let valid = value
            .as_deref()
            .map(Stamp::parse)
            .map(|parsed| parsed.is_ok())
            .unwrap_or(false);
        if !valid {
            reasons.push(format!("invalid_timestamp:{field}"));
        }
    }
    if let (Some(start), Some(end)) = (created.as_deref(), expires.as_deref()) {
        if end <= start && Stamp::parse(start).is_ok() && Stamp::parse(end).is_ok() {
            reasons.push("expiry_not_after_creation".to_string());
        }
    }
    match text_of(object.get("channel")) {
        Some(name) if super::channel::CHANNEL_NAMES.contains(&name.as_str()) => {}
        _ => reasons.push(format!(
            "unsupported_channel:{}",
            diag(object.get("channel"))
        )),
    }

    match object.get("target").and_then(Json::as_object) {
        None => reasons.push("target_not_object".to_string()),
        Some(target) => {
            match text_of(target.get("component")) {
                Some(name) if super::channel::COMPONENTS.contains(&name.as_str()) => {}
                _ => reasons.push(format!(
                    "unsupported_target_component:{}",
                    diag(target.get("component"))
                )),
            }
            match text_of(target.get("host")) {
                Some(host) if super::channel::HOSTS.contains(&host.as_str()) => {}
                _ => reasons.push(format!("unsupported_host:{}", diag(target.get("host")))),
            }
            if !is_text(target.get("install_root")) {
                reasons.push("missing_target_install_root".to_string());
            }
        }
    }

    match object.get("components").and_then(Json::as_array) {
        None => reasons.push("components_not_list".to_string()),
        Some([]) => reasons.push("components_empty".to_string()),
        Some(rows) => {
            let mut seen: BTreeSet<Option<String>> = BTreeSet::new();
            for (index, row) in rows.iter().enumerate() {
                let label = format!("components[{index}]");
                let Some(entry) = row.as_object() else {
                    reasons.push("component_not_object".to_string());
                    continue;
                };
                reasons.extend(blank_required(
                    entry,
                    &[
                        "component",
                        "installed_version",
                        "installed_revision",
                        "target_version",
                        "target_revision",
                        "artifact_sha256",
                        "action",
                    ],
                    &label,
                ));
                let component = text_of(entry.get("component"));
                match &component {
                    Some(name) if super::channel::COMPONENTS.contains(&name.as_str()) => {}
                    _ => reasons.push(format!(
                        "unsupported_component:{}",
                        diag(entry.get("component"))
                    )),
                }
                if seen.contains(&component) {
                    reasons.push(format!(
                        "duplicate_component:{}",
                        diag(entry.get("component"))
                    ));
                }
                seen.insert(component.clone());
                let name = component.clone().unwrap_or_else(|| "None".to_string());
                match text_of(entry.get("action")) {
                    Some(action) if ACTIONS.contains(&action.as_str()) => {}
                    _ => reasons.push(format!(
                        "unsupported_action:{}:{}",
                        name,
                        diag(entry.get("action"))
                    )),
                }
                for field in ["target_version"] {
                    match text_of(entry.get(field)) {
                        Some(version) if rules::is_semver(&version) => {}
                        _ => reasons.push(format!("invalid_version:{name}.{field}")),
                    }
                }
                let installed = entry.get("installed_version");
                if !matches!(installed, None | Some(Json::Null)) {
                    let valid = text_of(installed)
                        .map(|version| rules::is_semver(&version))
                        .unwrap_or(false);
                    if !valid {
                        reasons.push(format!("invalid_version:{name}.installed_version"));
                    }
                }
                for field in ["installed_revision", "target_revision"] {
                    let value = entry.get(field);
                    if field == "installed_revision" && matches!(value, None | Some(Json::Null)) {
                        continue;
                    }
                    let valid = text_of(value)
                        .map(|revision| rules::is_revision40(&revision))
                        .unwrap_or(false);
                    if !valid {
                        reasons.push(format!("invalid_revision:{name}.{field}"));
                    }
                }
                match text_of(entry.get("artifact_sha256")) {
                    Some(value) if rules::is_digest64(&value) => {}
                    _ => reasons.push(format!("invalid_digest:{name}.artifact_sha256")),
                }
                let action = text_of(entry.get("action"));
                let target_version = text_of(entry.get("target_version"));
                let installed_version = text_of(installed);
                let semver_target = target_version
                    .as_deref()
                    .map(rules::is_semver)
                    .unwrap_or(false);
                if action
                    .as_deref()
                    .map(|a| ACTIONS.contains(&a))
                    .unwrap_or(false)
                    && semver_target
                {
                    let action = action.clone().unwrap_or_default();
                    if action == "install" && !matches!(installed, None | Some(Json::Null)) {
                        reasons.push(format!("action_version_mismatch:{name}"));
                    }
                    if (action == "upgrade" || action == "reinstall")
                        && matches!(installed, None | Some(Json::Null))
                    {
                        reasons.push(format!("action_version_mismatch:{name}"));
                    }
                    if action == "upgrade" && installed_version == target_version {
                        reasons.push(format!("action_version_mismatch:{name}"));
                    }
                    if action == "noop" && installed_version != target_version {
                        reasons.push(format!("action_version_mismatch:{name}"));
                    }
                }
            }
        }
    }
    match object.get("downloads").and_then(Json::as_array) {
        None => reasons.push("downloads_not_list".to_string()),
        Some(rows) => {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for row in rows {
                let Some(entry) = row.as_object() else {
                    reasons.push("download_not_object".to_string());
                    continue;
                };
                let artifact = text_of(entry.get("artifact"));
                match &artifact {
                    Some(name) if !name.trim().is_empty() => {
                        if seen.contains(name) {
                            reasons.push(format!("duplicate_download:{name}"));
                        }
                        seen.insert(name.clone());
                    }
                    _ => reasons.push("missing_download_artifact".to_string()),
                }
                let label = artifact.unwrap_or_else(|| "None".to_string());
                match text_of(entry.get("url")) {
                    Some(url) if url.starts_with("https://") => {}
                    _ => reasons.push(format!("download_url_not_https:{label}")),
                }
                match text_of(entry.get("sha256")) {
                    Some(value) if rules::is_digest64(&value) => {}
                    _ => reasons.push(format!("invalid_digest:{label}.sha256")),
                }
                if !is_count(entry.get("size_bytes"), 1) {
                    reasons.push(format!("invalid_size:{label}"));
                }
            }
        }
    }

    let migrations = object.get("migrations").and_then(Json::as_array);
    match migrations {
        None => reasons.push("migrations_not_list".to_string()),
        Some(rows) => {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            for row in rows {
                let Some(entry) = row.as_object() else {
                    reasons.push("migration_not_object".to_string());
                    continue;
                };
                let migration_id = text_of(entry.get("migration_id"));
                match &migration_id {
                    Some(id) if rules::is_plan_id(id) => {
                        if seen.contains(id) {
                            reasons.push(format!("duplicate_migration:{id}"));
                        }
                        seen.insert(id.clone());
                    }
                    _ => reasons.push(format!(
                        "invalid_migration_id:{}",
                        migration_id.clone().unwrap_or_else(|| "None".to_string())
                    )),
                }
                let label = migration_id.unwrap_or_else(|| "None".to_string());
                let from = entry.get("from_queue_schema").and_then(Json::as_int);
                let to = entry.get("to_queue_schema").and_then(Json::as_int);
                if from.is_none() || to.is_none() || from == Some(-1) || to == Some(-1) {
                    reasons.push(format!("invalid_queue_schema_step:{label}"));
                } else if let (Some(source), Some(target)) = (from, to) {
                    if source < 0 || target < 0 {
                        reasons.push(format!("invalid_queue_schema_step:{label}"));
                    } else if target <= source {
                        reasons.push(format!("migration_not_forward:{label}"));
                    }
                }
                if entry.get("reversible") == Some(&Json::Bool(false))
                    && entry.get("requires_backup") != Some(&Json::Bool(true))
                {
                    reasons.push(format!("irreversible_migration_without_backup:{label}"));
                }
            }
        }
    }

    match object.get("backup").and_then(Json::as_object) {
        None => reasons.push("backup_not_object".to_string()),
        Some(backup) => {
            if !is_bool(backup.get("required")) {
                reasons.push("invalid_backup_required".to_string());
            }
            match backup.get("targets").and_then(Json::as_array) {
                None => reasons.push("backup_targets_not_list".to_string()),
                Some(targets) => {
                    for name in targets {
                        let valid = text_of(Some(name))
                            .map(|text| rules::is_relative_path(&text))
                            .unwrap_or(false);
                        if !valid {
                            reasons.push(format!("invalid_backup_target:{}", diag(Some(name))));
                        }
                    }
                }
            }
            let mut needs: Vec<String> = Vec::new();
            for row in migrations.unwrap_or(&[]) {
                if let Some(entry) = row.as_object() {
                    if entry.get("requires_backup") == Some(&Json::Bool(true)) {
                        needs.push(diag(entry.get("migration_id")));
                    }
                }
            }
            if !needs.is_empty() && backup.get("required") != Some(&Json::Bool(true)) {
                needs.sort();
                reasons.push(format!("backup_required_by_migration:{}", needs.join(",")));
            }
        }
    }

    if !is_count(object.get("disk_headroom_bytes"), 0) {
        reasons.push("invalid_disk_headroom_bytes".to_string());
    }

    match object.get("service_interruptions").and_then(Json::as_array) {
        None => reasons.push("service_interruptions_not_list".to_string()),
        Some(rows) => {
            for row in rows {
                let Some(entry) = row.as_object() else {
                    reasons.push("service_interruption_not_object".to_string());
                    continue;
                };
                match text_of(entry.get("service")) {
                    Some(name) if super::channel::COMPONENTS.contains(&name.as_str()) => {}
                    _ => reasons.push(format!(
                        "unsupported_service:{}",
                        diag(entry.get("service"))
                    )),
                }
                let service = diag(entry.get("service"));
                match text_of(entry.get("action")) {
                    Some(action) if INTERRUPTION_ACTIONS.contains(&action.as_str()) => {}
                    _ => reasons.push(format!(
                        "unsupported_interruption_action:{service}:{}",
                        diag(entry.get("action"))
                    )),
                }
                if !is_count(entry.get("max_seconds"), 0) {
                    reasons.push(format!("invalid_interruption_seconds:{service}"));
                }
            }
        }
    }

    match object.get("host_reconnect").and_then(Json::as_object) {
        None => reasons.push("host_reconnect_not_object".to_string()),
        Some(reconnect) => {
            if !is_bool(reconnect.get("required")) {
                reasons.push("invalid_host_reconnect_required".to_string());
            }
            if !is_text(reconnect.get("reason")) {
                reasons.push("missing_host_reconnect_reason".to_string());
            }
        }
    }

    match object.get("bootstrap_changes").and_then(Json::as_array) {
        None => reasons.push("bootstrap_changes_not_list".to_string()),
        Some(rows) => {
            for row in rows {
                let Some(entry) = row.as_object() else {
                    reasons.push("bootstrap_change_not_object".to_string());
                    continue;
                };
                let repo_id = diag(entry.get("repo_id"));
                match text_of(entry.get("repo_id")) {
                    Some(value) if rules::is_plan_id(&value) => {}
                    _ => reasons.push(format!("invalid_bootstrap_repo:{repo_id}")),
                }
                match text_of(entry.get("template_version")) {
                    Some(value) if rules::is_semver(&value) => {}
                    _ => reasons.push(format!("invalid_bootstrap_template_version:{repo_id}")),
                }
                match entry.get("destinations").and_then(Json::as_array) {
                    None => reasons.push(format!("missing_bootstrap_destinations:{repo_id}")),
                    Some([]) => reasons.push(format!("missing_bootstrap_destinations:{repo_id}")),
                    Some(destinations) => {
                        for destination in destinations {
                            let valid = text_of(Some(destination))
                                .map(|text| rules::is_relative_path(&text))
                                .unwrap_or(false);
                            if !valid {
                                reasons.push(format!(
                                    "invalid_bootstrap_destination:{}",
                                    diag(Some(destination))
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    match object.get("rollback").and_then(Json::as_object) {
        None => reasons.push("rollback_not_object".to_string()),
        Some(rollback) => {
            if !is_bool(rollback.get("supported")) {
                reasons.push("invalid_rollback_supported".to_string());
            }
            if !is_bool(rollback.get("restores_previous_versions")) {
                reasons.push("invalid_rollback_restores".to_string());
            }
            match rollback.get("limits").and_then(Json::as_array) {
                None => reasons.push("rollback_limits_not_list".to_string()),
                Some(limits) => {
                    if rollback.get("supported") == Some(&Json::Bool(true))
                        && limits.iter().any(|limit| !is_text(Some(limit)))
                    {
                        reasons.push("invalid_rollback_limit".to_string());
                    }
                }
            }
            if rollback.get("supported") != Some(&Json::Bool(true))
                && migrations.map(|rows| !rows.is_empty()).unwrap_or(false)
            {
                reasons.push("rollback_unsupported_with_migration".to_string());
            }
        }
    }

    match object.get("trust").and_then(Json::as_object) {
        None => reasons.push("trust_not_object".to_string()),
        Some(trust) => {
            if !is_count(trust.get("metadata_version"), 1) {
                reasons.push("invalid_trust_metadata_version".to_string());
            }
            match text_of(trust.get("metadata_expiry")) {
                Some(expiry) if Stamp::parse(&expiry).is_ok() => {
                    if let Some(start) = created.as_deref() {
                        if Stamp::parse(start).is_ok() && expiry.as_str() <= start {
                            reasons.push("trust_metadata_expired".to_string());
                        }
                    }
                }
                _ => reasons.push("invalid_trust_metadata_expiry".to_string()),
            }
            match trust.get("trust_root") {
                Some(Json::Text(value)) if rules::is_digest64(value) => {}
                _ => reasons.push("unresolved_trust_root".to_string()),
            }
            if trust.get("signature_present") != Some(&Json::Bool(true)) {
                reasons.push("unsigned_plan".to_string());
            }
        }
    }

    match object.get("approval").and_then(Json::as_object) {
        None => reasons.push("approval_not_object".to_string()),
        Some(approval) => match text_of(approval.get("state")) {
            Some(state) if !APPROVAL_STATES.contains(&state.as_str()) => reasons.push(format!(
                "unsupported_approval_state:{}",
                diag(approval.get("state"))
            )),
            None => reasons.push(format!(
                "unsupported_approval_state:{}",
                diag(approval.get("state"))
            )),
            Some(state) if state == "approved" => {
                if !is_text(approval.get("approved_by")) {
                    reasons.push("approval_incomplete:approved_by".to_string());
                }
                match text_of(approval.get("approved_at")) {
                    Some(value) if Stamp::parse(&value).is_ok() => {}
                    _ => reasons.push("approval_incomplete:approved_at".to_string()),
                }
                match text_of(approval.get("approved_digest")) {
                    Some(recorded) if rules::is_digest64(&recorded) => {
                        if object.get("plan_digest").and_then(Json::as_text) != Some(&recorded) {
                            reasons.push("approval_digest_mismatch".to_string());
                        }
                    }
                    _ => reasons.push("approval_incomplete:approved_digest".to_string()),
                }
            }
            Some(_) => {
                if approval.get("approved_digest") != Some(&Json::Null) {
                    reasons.push("unapproved_plan_carries_digest".to_string());
                }
                if approval.get("approved_by") != Some(&Json::Null) {
                    reasons.push("unapproved_plan_carries_approver".to_string());
                }
                if approval.get("approved_at") != Some(&Json::Null) {
                    reasons.push("unapproved_plan_carries_timestamp".to_string());
                }
            }
        },
    }

    match text_of(object.get("plan_digest")) {
        Some(recorded) if rules::is_digest64(&recorded) => match digest(plan) {
            Some(computed) if computed == recorded => {}
            _ => reasons.push("plan_digest_mismatch".to_string()),
        },
        _ => reasons.push("invalid_plan_digest".to_string()),
    }

    reasons
}

/// Render a value for a diagnostic reason token the way the canonical Python tool does.
fn diag(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => "None".to_string(),
        Some(Json::Text(text)) => text.clone(),
        Some(other) => {
            let mut out = String::new();
            other.write(&mut out);
            out
        }
    }
}
/// A component the planner could not include, and why.
#[derive(Clone, Debug)]
pub struct Excluded {
    /// Component id.
    pub component: String,
    /// Stable reason token.
    pub reason: String,
}

/// The planner output.
#[derive(Clone, Debug)]
pub struct Built {
    /// The plan document.
    pub plan: Json,
    /// Its canonical digest.
    pub digest: String,
    /// Components included in the plan, in manifest order.
    pub planned: Vec<String>,
    /// Components deliberately left out, with the reason.
    pub excluded: Vec<Excluded>,
    /// Components that must be restarted after the transaction.
    pub needs_restart: Vec<String>,
}

/// Everything the planner resolves before it writes a plan.
pub struct Planning<'a> {
    /// The channel manifest recorded for the installed release.
    pub manifest: &'a super::channel::Manifest,
    /// Target host id.
    pub host: &'a str,
    /// Absolute install root, recorded in the plan.
    pub install_root: &'a str,
    /// Channel name.
    pub channel: &'a str,
    /// The component the requested version is resolved against.
    pub target_component: &'a str,
    /// The version the caller asked for.
    pub to_version: &'a str,
    /// Installed version and revision per component, as the active generation records them.
    pub installed_versions: &'a BTreeMap<String, (Option<String>, Option<String>)>,
    /// Plan creation time.
    pub now: Stamp,
    /// Approver identity, when the plan is built already approved.
    pub approved_by: Option<&'a str>,
}

/// Build a plan from the recorded channel manifest.
pub fn build(input: &Planning) -> Result<Built, Refusal> {
    if !super::channel::HOSTS.contains(&input.host) {
        return Err(Refusal::validation(
            "unsupported_host",
            format!(
                "host `{}` is not one of {:?}",
                input.host,
                super::channel::HOSTS
            ),
        ));
    }
    if !super::channel::CHANNEL_NAMES.contains(&input.channel) {
        return Err(Refusal::validation(
            "unsupported_channel",
            format!("channel `{}` is not a declared channel name", input.channel),
        ));
    }
    if !super::channel::COMPONENTS.contains(&input.target_component) {
        return Err(Refusal::validation(
            "unsupported_target_component",
            format!(
                "target component `{}` is not one of {:?}",
                input.target_component,
                super::channel::COMPONENTS
            ),
        ));
    }
    let target_entry = input
        .manifest
        .component(input.target_component)
        .ok_or_else(|| {
            Refusal::not_ready(
                "target_component_not_in_channel",
                format!(
                    "the recorded channel manifest does not declare the target component `{}`",
                    input.target_component
                ),
            )
        })?;
    if !target_entry.declared {
        return Err(Refusal::validation(
            "target_component_version_undeclared",
            format!(
                "the recorded channel manifest does not declare a SemVer version for `{}` \
                 ({}), so the requested version cannot be resolved from it",
                input.target_component, target_entry.declared_source
            ),
        ));
    }
    let declared_target = target_entry.version.clone().unwrap_or_default();
    if declared_target != input.to_version {
        return Err(Refusal::validation(
            "version_not_in_channel",
            format!(
                "`--to {}` is not the version the recorded channel manifest declares for `{}` \
                 (it declares {}): this CLI resolves versions only from the recorded manifest \
                 and never follows a branch tip, a tag alias or a network `latest`",
                input.to_version, input.target_component, declared_target
            ),
        ));
    }

    let mut planned: Vec<String> = Vec::new();
    let mut excluded: Vec<Excluded> = Vec::new();
    let mut rows: Vec<Json> = Vec::new();
    let mut downloads: Vec<Json> = Vec::new();
    let mut interruptions: Vec<Json> = Vec::new();
    let mut needs_restart: Vec<String> = Vec::new();
    let mut staged_bytes: i64 = 0;

    for component in &input.manifest.components {
        if !component.declared {
            excluded.push(Excluded {
                component: component.component.clone(),
                reason: format!("version_not_semver:{}", component.declared_source),
            });
            continue;
        }
        let version = component.version.clone().unwrap_or_default();
        let revision = component.revision.clone().unwrap_or_default();
        let artifact = match component.artifact(input.host, "per-user-installer") {
            Some(found) => Some(found),
            None => component.artifact(input.host, "oci-image"),
        };
        let Some(artifact) = artifact else {
            excluded.push(Excluded {
                component: component.component.clone(),
                reason: format!("no_artifact_for_host:{}", input.host),
            });
            continue;
        };
        let (installed_version, installed_revision) = input
            .installed_versions
            .get(&component.component)
            .cloned()
            .unwrap_or((None, None));
        let action = match installed_version.as_deref() {
            None => "install",
            Some(current) if current == version => "noop",
            Some(_) => "upgrade",
        };
        rows.push(component_row(
            &component.component,
            &installed_version,
            &installed_revision,
            &version,
            &revision,
            &artifact.sha256,
            action,
        ));
        if action != "noop" {
            staged_bytes = staged_bytes.saturating_add(artifact.size_bytes);
            downloads.push(download_row(&component.component, artifact));
        }
        if component.needs_restart && action != "noop" {
            needs_restart.push(component.component.clone());
        }
        interruptions.push(Json::from_pairs(vec![
            ("service", Json::text(&component.component)),
            (
                "action",
                Json::text(if component.needs_restart && action != "noop" {
                    "restart"
                } else {
                    "none"
                }),
            ),
            (
                "max_seconds",
                Json::int(if component.needs_restart && action != "noop" {
                    30
                } else {
                    0
                }),
            ),
        ]));
        planned.push(component.component.clone());
    }

    if planned.is_empty() {
        return Err(Refusal::not_ready(
            "no_plannable_component",
            format!(
                "the recorded channel manifest declares no component with an artifact for host \
                 {}, so there is nothing to plan",
                input.host
            ),
        ));
    }
    if !planned.iter().any(|name| name == "axiom-cli") {
        return Err(Refusal::not_ready(
            "self_update_not_in_plan",
            "axiom-cli is itself a component of this distribution, so a plan that does not \
             include it would split the self-update from the component updates into two \
             transactions with two approval digests; this CLI refuses that split",
        ));
    }

    let created = input.now;
    let expires = input.now.plus_seconds(EXPIRY_SECONDS);
    let plan_id = format!("update-{}-{}", input.channel, input.target_component);
    let mut body = Json::Object(BTreeMap::new());
    let _ = body.set("schema_version", Json::int(SCHEMA_VERSION));
    let _ = body.set("spec_version", Json::text(SPEC_VERSION));
    let _ = body.set("plan_id", Json::text(&plan_id));
    let _ = body.set("created_at", Json::text(&created.format()));
    let _ = body.set("expires_at", Json::text(&expires.format()));
    let _ = body.set("channel", Json::text(input.channel));
    let _ = body.set(
        "target",
        Json::from_pairs(vec![
            ("component", Json::text(input.target_component)),
            ("host", Json::text(input.host)),
            ("install_root", Json::text(input.install_root)),
        ]),
    );
    let _ = body.set("components", Json::array(rows));
    let _ = body.set("downloads", Json::array(downloads));
    let _ = body.set("migrations", Json::array(Vec::new()));
    let _ = body.set(
        "backup",
        Json::from_pairs(vec![
            ("required", Json::bool(false)),
            ("targets", Json::array(Vec::new())),
        ]),
    );
    let _ = body.set(
        "disk_headroom_bytes",
        Json::int(
            staged_bytes
                .saturating_mul(2)
                .saturating_add(DISK_HEADROOM_FLOOR),
        ),
    );
    let _ = body.set("service_interruptions", Json::array(interruptions));
    let _ = body.set(
        "host_reconnect",
        Json::from_pairs(vec![
        ("required", Json::bool(false)),
        (
            "reason",
            Json::text(
                "the update transaction runs inside the per-user install root and needs no host \
                 reconnect: no service registration, no PATH mutation and no elevation",
            ),
        ),
    ]),
    );
    let _ = body.set("bootstrap_changes", Json::array(Vec::new()));
    let _ = body.set(
        "rollback",
        Json::from_pairs(vec![
        ("supported", Json::bool(true)),
        ("restores_previous_versions", Json::bool(true)),
        (
            "limits",
            Json::text_array(&[
                "rollback restores the recorded previous generation and its reported \
                 needs_restart state; how an owning component activates its payload into its \
                 canonical executable path is that component's service lifecycle, owned by \
                 axiom-graphd, not by this distribution layer",
                "user data, the graph output root and portable workspace state are never \
                 modified by this transaction, so they are not part of the rollback set",
                "generations retained beyond the active and previous one are pruned when a new \
                 generation is recorded; a rollback target older than the retained previous \
                 generation is not available",
            ]),
        ),
    ]),
    );
    let _ = body.set(
        "trust",
        Json::from_pairs(vec![
            (
                "metadata_version",
                Json::int(input.manifest.trust.metadata_version),
            ),
            (
                "metadata_expiry",
                Json::text(&input.manifest.trust.metadata_expiry.format()),
            ),
            ("trust_root", Json::text(&input.manifest.trust.trust_root)),
            (
                "signature_present",
                Json::bool(input.manifest.trust.signature_artifact.is_some()),
            ),
        ]),
    );

    let computed = digest(&body).ok_or_else(|| {
        Refusal::new(
            super::error::Class::IoInternal,
            "plan_digest_unavailable",
            "the plan body could not be canonicalised for its digest",
        )
    })?;
    let _ = body.set(
        "approval",
        match input.approved_by {
            Some(approver) => Json::from_pairs(vec![
                ("state", Json::text("approved")),
                ("approved_digest", Json::text(&computed)),
                ("approved_by", Json::text(approver)),
                ("approved_at", Json::text(&created.format())),
            ]),
            None => Json::from_pairs(vec![
                ("state", Json::text("unapproved")),
                ("approved_digest", Json::null()),
                ("approved_by", Json::null()),
                ("approved_at", Json::null()),
            ]),
        },
    );
    let _ = body.set("plan_digest", Json::text(&computed));

    Ok(Built {
        plan: body,
        digest: computed,
        planned,
        excluded,
        needs_restart,
    })
}

fn component_row(
    component: &str,
    installed_version: &Option<String>,
    installed_revision: &Option<String>,
    version: &str,
    revision: &str,
    artifact_sha256: &str,
    action: &str,
) -> Json {
    let optional = |value: &Option<String>| match value {
        Some(text) => Json::text(text),
        None => Json::null(),
    };
    Json::from_pairs(vec![
        ("component", Json::text(component)),
        ("installed_version", optional(installed_version)),
        ("installed_revision", optional(installed_revision)),
        ("target_version", Json::text(version)),
        ("target_revision", Json::text(revision)),
        ("artifact_sha256", Json::text(artifact_sha256)),
        ("action", Json::text(action)),
    ])
}

fn download_row(component: &str, artifact: &super::channel::Artifact) -> Json {
    Json::from_pairs(vec![
        (
            "artifact",
            Json::text(&format!(
                "{}:{}:{}",
                component, artifact.platform, artifact.class
            )),
        ),
        ("url", Json::text(&artifact.url)),
        ("sha256", Json::text(&artifact.sha256)),
        ("size_bytes", Json::int(artifact.size_bytes)),
    ])
}
