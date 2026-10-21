//! The channel manifest (`channels/stable.json`), owned by `axiom-cli`.
//!
//! The distribution contract freezes one rule above all others here: the installed release
//! records the manifest it came from, and versions resolve **only** from that recorded
//! manifest. This module therefore parses and validates a manifest document; it never
//! discovers one, never follows a branch tip, a tag alias, a network `latest`, `HEAD` or `*`,
//! and never invents a version that the owning repository has not declared.
//!
//! A version the owner has not declared in SemVer is recorded as `undeclared` together with
//! the source of the owner's declaration, and it can never be planned for. That is how the
//! distribution stays honest about `axiom-mcp`, whose owner declares a PEP 440 version.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::json::{self, Json};
use super::rules;
use super::sha256;
use super::time::Stamp;

/// Hosts a component artifact can target, from `update-plan.schema.json` `target.host`.
pub const HOSTS: [&str; 4] = ["windows-x64", "linux-x64", "macos-arm64", "macos-x64"];

/// The five components the distribution contract names.
pub const COMPONENTS: [&str; 5] = ["axiom-graphd", "axiom-mcp", "axiom", "axiom-cli", "skills"];

/// Artifact classes from `axiom-cli-distribution-contract.md` section 3.
pub const ARTIFACT_CLASSES: [&str; 2] = ["per-user-installer", "oci-image"];

/// Channel names accepted by `update-plan.schema.json` `channel`.
pub const CHANNEL_NAMES: [&str; 2] = ["stable", "prerelease"];

/// A refusal with a stable machine reason and a human message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelError {
    /// Stable reason token, used in evidence and in the JSON envelope.
    pub reason: String,
    /// Human-readable explanation.
    pub message: String,
}

impl ChannelError {
    /// Build a refusal.
    pub fn new(reason: impl Into<String>, message: impl Into<String>) -> ChannelError {
        ChannelError {
            reason: reason.into(),
            message: message.into(),
        }
    }
}

/// Owner-declared trust metadata for the channel.
#[derive(Clone, Debug)]
pub struct Trust {
    /// Monotonic metadata version, `>= 1`.
    pub metadata_version: i64,
    /// When this metadata stops being valid.
    pub metadata_expiry: Stamp,
    /// The pinned 64-hex trust root digest.
    pub trust_root: String,
    /// Digest of the detached signature artifact, when the owner published one.
    pub signature_artifact: Option<String>,
}

/// One published artifact for one component on one platform.
#[derive(Clone, Debug)]
pub struct Artifact {
    /// Target host id.
    pub platform: String,
    /// Artifact class id.
    pub class: String,
    /// `https://` location the owner publishes it at.
    pub url: String,
    /// Content digest, verified before the artifact is used.
    pub sha256: String,
    /// Declared byte length, verified together with the digest.
    pub size_bytes: i64,
}

/// An owner-declared post-swap health probe for one component.
///
/// A component whose payload is only bytes cannot be shown to work by hashing it again, so the
/// owner may declare a probe that is executed as a program plus argv once the new generation is
/// active. The probe is part of the recorded channel manifest, exactly like an artifact digest,
/// so it is the owner's declaration and not a local convention. A probe is optional: when it is
/// absent the health check is the payload digest re-verification alone.
#[derive(Clone, Debug)]
pub struct Health {
    /// Program to execute. An absolute path is allowed: a probe is a host tool, not a payload.
    pub program: String,
    /// Arguments, passed as argv - never joined into a shell string.
    pub args: Vec<String>,
    /// The exit code the probe must produce for the generation to count as healthy.
    pub expect_exit: i64,
}

/// One component entry of the channel.
#[derive(Clone, Debug)]
pub struct Component {
    /// Component id.
    pub component: String,
    /// `true` when the owning repository declared a SemVer version.
    pub declared: bool,
    /// Declared SemVer version, only when `declared`.
    pub version: Option<String>,
    /// Declared 40-hex revision, only when `declared`.
    pub revision: Option<String>,
    /// Where the owner's declaration lives; required when the version is not SemVer.
    pub declared_source: String,
    /// Whether this component must be restarted after it changes.
    pub needs_restart: bool,
    /// Published artifacts, possibly empty while nothing is published.
    pub artifacts: Vec<Artifact>,
    /// Owner-declared post-swap health probe, when the owner published one.
    pub health: Option<Health>,
}

impl Component {
    /// The artifact for one platform and class, when the owner published one.
    pub fn artifact(&self, platform: &str, class: &str) -> Option<&Artifact> {
        self.artifacts
            .iter()
            .find(|item| item.platform == platform && item.class == class)
    }
}

/// A parsed channel manifest.
#[derive(Clone, Debug)]
pub struct Manifest {
    /// Manifest schema version.
    pub manifest_version: i64,
    /// Channel name.
    pub channel: String,
    /// When the owner last updated the manifest.
    pub updated_at: Stamp,
    /// Whether the owner has published the artifact set this manifest describes.
    pub published: bool,
    /// Free-text owner note; may be empty.
    pub note: String,
    /// Trust metadata.
    pub trust: Trust,
    /// Components, in the frozen id order.
    pub components: Vec<Component>,
}

/// A manifest together with the digest of the exact bytes it was parsed from.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The parsed manifest.
    pub manifest: Manifest,
    /// Path the bytes were read from.
    pub path: PathBuf,
    /// sha256 of the bytes on disk, so a caller can compare it with the recorded digest.
    pub sha256: String,
}

impl Manifest {
    /// Parse and validate a manifest document.
    pub fn parse(text: &str) -> Result<Manifest, ChannelError> {
        let value = json::parse(text).map_err(|error| {
            ChannelError::new(
                "channel_manifest_not_json",
                format!("channels/stable.json is not valid JSON: {error}"),
            )
        })?;
        Manifest::from_json(&value)
    }

    /// Read, parse and validate a manifest from disk.
    pub fn load(path: &Path) -> Result<Loaded, ChannelError> {
        let bytes = std::fs::read(path).map_err(|error| {
            ChannelError::new(
                "channel_unreachable",
                format!(
                    "cannot read the channel manifest {}: {error}",
                    path.display()
                ),
            )
        })?;
        let text = String::from_utf8(bytes.clone()).map_err(|_| {
            ChannelError::new(
                "channel_manifest_not_utf8",
                format!("the channel manifest {} is not UTF-8", path.display()),
            )
        })?;
        Ok(Loaded {
            manifest: Manifest::parse(&text)?,
            path: path.to_path_buf(),
            sha256: sha256::digest_hex(&bytes),
        })
    }

    /// The component entry for `component`.
    pub fn component(&self, component: &str) -> Option<&Component> {
        self.components
            .iter()
            .find(|item| item.component == component)
    }

    fn from_json(value: &Json) -> Result<Manifest, ChannelError> {
        let object = value.as_object().ok_or_else(|| {
            ChannelError::new(
                "channel_manifest_not_object",
                "the channel manifest must be a JSON object",
            )
        })?;
        reject_undeclared_keys(
            object,
            &[
                "manifest_version",
                "channel",
                "updated_at",
                "published",
                "note",
                "trust",
                "components",
            ],
        )?;
        let manifest_version = object
            .get("manifest_version")
            .and_then(Json::as_int)
            .ok_or_else(|| {
                ChannelError::new(
                    "missing_field:manifest_version",
                    "the channel manifest requires `manifest_version`",
                )
            })?;
        if manifest_version != 1 {
            return Err(ChannelError::new(
                "unsupported_manifest_version",
                format!(
                    "channel manifest_version {manifest_version} is not supported; this CLI reads version 1"
                ),
            ));
        }
        let channel = text_field(object, "channel")?;
        if !CHANNEL_NAMES.contains(&channel.as_str()) {
            return Err(ChannelError::new(
                "unsupported_channel_name",
                format!("channel name `{channel}` is not one of {CHANNEL_NAMES:?}"),
            ));
        }
        if let Some(pin) = rules::forbidden_pin_in(&channel) {
            return Err(ChannelError::new(
                format!("forbidden_pin:{pin}"),
                format!("the channel name `{channel}` is a forbidden pin"),
            ));
        }
        let updated_at = parse_stamp(object, "updated_at")?;
        let published = object
            .get("published")
            .and_then(Json::as_bool)
            .ok_or_else(|| {
                ChannelError::new(
                    "missing_field:published",
                    "the channel manifest requires boolean `published`",
                )
            })?;
        let note = object
            .get("note")
            .and_then(Json::as_text)
            .unwrap_or("")
            .to_string();
        let trust = parse_trust(object.get("trust"))?;
        let components = parse_components(object.get("components"))?;
        let manifest = Manifest {
            manifest_version,
            channel,
            updated_at,
            published,
            note,
            trust,
            components,
        };
        manifest.reject_placeholders()?;
        Ok(manifest)
    }

    fn reject_placeholders(&self) -> Result<(), ChannelError> {
        let mut fields: Vec<String> = vec![self.trust.trust_root.clone()];
        if let Some(signature) = &self.trust.signature_artifact {
            fields.push(signature.clone());
        }
        for component in &self.components {
            if let Some(version) = &component.version {
                fields.push(version.clone());
            }
            if let Some(revision) = &component.revision {
                fields.push(revision.clone());
            }
            fields.push(component.declared_source.clone());
            for artifact in &component.artifacts {
                fields.push(artifact.url.clone());
                fields.push(artifact.sha256.clone());
            }
        }
        for field in fields {
            if rules::contains_placeholder(&field) {
                return Err(ChannelError::new(
                    "unresolved_placeholder",
                    format!(
                        "the channel manifest still carries an unresolved placeholder in `{field}`"
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn reject_undeclared_keys(
    object: &BTreeMap<String, Json>,
    allowed: &[&str],
) -> Result<(), ChannelError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ChannelError::new(
                format!("undeclared_field:{key}"),
                format!("the channel manifest declares an unknown field `{key}`"),
            ));
        }
    }
    Ok(())
}

fn text_field(object: &BTreeMap<String, Json>, key: &str) -> Result<String, ChannelError> {
    object
        .get(key)
        .and_then(Json::as_text)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ChannelError::new(
                format!("missing_field:{key}"),
                format!("the channel manifest requires `{key}`"),
            )
        })
}

fn parse_stamp(object: &BTreeMap<String, Json>, key: &str) -> Result<Stamp, ChannelError> {
    let text = text_field(object, key)?;
    Stamp::parse(&text)
        .map_err(|error| ChannelError::new(format!("invalid_timestamp:{key}"), error))
}

fn parse_trust(value: Option<&Json>) -> Result<Trust, ChannelError> {
    let object = value.and_then(Json::as_object).ok_or_else(|| {
        ChannelError::new(
            "missing_field:trust",
            "the channel manifest requires a `trust` object",
        )
    })?;
    reject_undeclared_keys(
        object,
        &[
            "metadata_version",
            "metadata_expiry",
            "trust_root",
            "signature_artifact",
        ],
    )?;
    let metadata_version = object
        .get("metadata_version")
        .and_then(Json::as_int)
        .ok_or_else(|| {
            ChannelError::new(
                "missing_field:trust.metadata_version",
                "`trust.metadata_version` is required",
            )
        })?;
    if metadata_version < 1 {
        return Err(ChannelError::new(
            "invalid_trust_metadata_version",
            format!("trust metadata_version {metadata_version} is below 1"),
        ));
    }
    let metadata_expiry = parse_stamp(object, "metadata_expiry")?;
    let trust_root = match object.get("trust_root") {
        Some(Json::Text(root)) => root.clone(),
        Some(Json::Null) | None => {
            return Err(ChannelError::new(
                "unsigned_channel",
                "the channel manifest pins no trust_root, so the channel is unsigned and MUST NOT be resolved",
            ))
        }
        Some(other) => {
            return Err(ChannelError::new(
                "invalid_trust_root",
                format!(
                    "`trust.trust_root` must be a 64-hex digest, found {}",
                    other.kind()
                ),
            ))
        }
    };
    if !rules::is_digest64(&trust_root) {
        return Err(ChannelError::new(
            "invalid_trust_root",
            format!(
                "`trust.trust_root` must be a 64-character lowercase hex digest, got {} characters",
                trust_root.len()
            ),
        ));
    }
    let signature_artifact = match object.get("signature_artifact") {
        None | Some(Json::Null) => None,
        Some(Json::Text(digest)) => {
            if !rules::is_digest64(digest) {
                return Err(ChannelError::new(
                    "invalid_signature_artifact",
                    "`trust.signature_artifact` must be a 64-character lowercase hex digest",
                ));
            }
            Some(digest.clone())
        }
        Some(other) => {
            return Err(ChannelError::new(
                "invalid_signature_artifact",
                format!(
                    "`trust.signature_artifact` must be a digest or null, found {}",
                    other.kind()
                ),
            ))
        }
    };
    Ok(Trust {
        metadata_version,
        metadata_expiry,
        trust_root,
        signature_artifact,
    })
}

fn parse_components(value: Option<&Json>) -> Result<Vec<Component>, ChannelError> {
    let items = value.and_then(Json::as_array).ok_or_else(|| {
        ChannelError::new(
            "missing_field:components",
            "the channel manifest requires a `components` array",
        )
    })?;
    if items.is_empty() {
        return Err(ChannelError::new(
            "empty_component_set",
            "the channel manifest declares no component, so nothing can be resolved",
        ));
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut components = Vec::new();
    for item in items {
        let object = item.as_object().ok_or_else(|| {
            ChannelError::new(
                "component_not_object",
                "every component entry must be a JSON object",
            )
        })?;
        reject_undeclared_keys(
            object,
            &[
                "component",
                "status",
                "version",
                "revision",
                "declared_source",
                "needs_restart",
                "artifacts",
                "health",
            ],
        )?;
        let component = text_field(object, "component")?;
        if !COMPONENTS.contains(&component.as_str()) {
            return Err(ChannelError::new(
                "unknown_component",
                format!(
                    "`{component}` is not one of the five distribution components {COMPONENTS:?}"
                ),
            ));
        }
        if !seen.insert(component.clone()) {
            return Err(ChannelError::new(
                "duplicate_component",
                format!("the channel manifest declares `{component}` more than once"),
            ));
        }
        let status = text_field(object, "status")?;
        let version = opt_text(object.get("version"));
        let revision = opt_text(object.get("revision"));
        let declared_source = text_field(object, "declared_source")?;
        let needs_restart = object
            .get("needs_restart")
            .and_then(Json::as_bool)
            .ok_or_else(|| {
                ChannelError::new(
                    format!("missing_field:{component}.needs_restart"),
                    format!("component `{component}` must declare `needs_restart`"),
                )
            })?;
        let (declared, version, revision) = match status.as_str() {
            "declared" => {
                let version = version.ok_or_else(|| {
                    ChannelError::new(
                        format!("missing_field:{component}.version"),
                        format!("component `{component}` is declared but carries no version"),
                    )
                })?;
                let revision = revision.ok_or_else(|| {
                    ChannelError::new(
                        format!("missing_field:{component}.revision"),
                        format!("component `{component}` is declared but carries no revision"),
                    )
                })?;
                if let Some(pin) = rules::forbidden_pin_in(&version) {
                    return Err(ChannelError::new(
                        format!("forbidden_pin:{pin}"),
                        format!(
                            "component `{component}` resolves version `{version}`, which is a forbidden pin"
                        ),
                    ));
                }
                if let Some(pin) = rules::forbidden_pin_in(&revision) {
                    return Err(ChannelError::new(
                        format!("forbidden_pin:{pin}"),
                        format!(
                            "component `{component}` resolves revision `{revision}`, which is a forbidden pin"
                        ),
                    ));
                }
                if !rules::is_semver(&version) {
                    return Err(ChannelError::new(
                        format!("invalid_semver:{component}"),
                        format!(
                            "component `{component}` declares `{version}`, which is not SemVer; a version whose owner does not declare SemVer is `undeclared` and is never normalised here"
                        ),
                    ));
                }
                if !rules::is_revision40(&revision) {
                    return Err(ChannelError::new(
                        format!("invalid_revision:{component}"),
                        format!(
                            "component `{component}` declares revision `{revision}`, which is not 40 lowercase hex characters"
                        ),
                    ));
                }
                (true, Some(version), Some(revision))
            }
            "undeclared" => {
                if version.is_some() || revision.is_some() {
                    return Err(ChannelError::new(
                        format!("undeclared_component_carries_version:{component}"),
                        format!(
                            "component `{component}` is undeclared but still carries a version or revision; the distribution never invents one"
                        ),
                    ));
                }
                (false, None, None)
            }
            other => {
                return Err(ChannelError::new(
                    format!("unsupported_component_status:{other}"),
                    format!(
                        "component `{component}` declares status `{other}`, expected `declared` or `undeclared`"
                    ),
                ))
            }
        };
        let artifacts = parse_artifacts(&component, object.get("artifacts"))?;
        let health = parse_health(&component, object.get("health"))?;
        components.push(Component {
            component,
            declared,
            version,
            revision,
            declared_source,
            needs_restart,
            artifacts,
            health,
        });
    }
    Ok(components)
}

fn parse_health(component: &str, value: Option<&Json>) -> Result<Option<Health>, ChannelError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let object = value.as_object().ok_or_else(|| {
        ChannelError::new(
            format!("health_not_object:{component}"),
            format!("component `{component}` must declare `health` as a JSON object"),
        )
    })?;
    reject_undeclared_keys(object, &["program", "args", "expect_exit"])?;
    let program = text_field(object, "program")?;
    if rules::contains_placeholder(&program) {
        return Err(ChannelError::new(
            "unresolved_placeholder",
            format!("component `{component}` declares a health probe with a placeholder program"),
        ));
    }
    let args = match object.get("args") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::Array(items)) => {
            let mut collected = Vec::new();
            for item in items {
                let text = item.as_text().ok_or_else(|| {
                    ChannelError::new(
                        format!("health_args_not_text:{component}"),
                        format!("every `health.args` entry of `{component}` must be a string"),
                    )
                })?;
                collected.push(text.to_string());
            }
            collected
        }
        Some(_) => {
            return Err(ChannelError::new(
                format!("health_args_not_array:{component}"),
                format!("component `{component}` must declare `health.args` as an array"),
            ))
        }
    };
    let expect_exit = object
        .get("expect_exit")
        .and_then(Json::as_int)
        .unwrap_or(0);
    Ok(Some(Health {
        program,
        args,
        expect_exit,
    }))
}

fn parse_artifacts(component: &str, value: Option<&Json>) -> Result<Vec<Artifact>, ChannelError> {
    let items = value.and_then(Json::as_array).ok_or_else(|| {
        ChannelError::new(
            format!("missing_field:{component}.artifacts"),
            format!("component `{component}` must declare an `artifacts` array, possibly empty"),
        )
    })?;
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut artifacts = Vec::new();
    for item in items {
        let object = item.as_object().ok_or_else(|| {
            ChannelError::new(
                "artifact_not_object",
                "every artifact entry must be a JSON object",
            )
        })?;
        reject_undeclared_keys(
            object,
            &["platform", "class", "url", "sha256", "size_bytes"],
        )?;
        let platform = text_field(object, "platform")?;
        if !HOSTS.contains(&platform.as_str()) {
            return Err(ChannelError::new(
                format!("unknown_platform:{platform}"),
                format!("artifact platform `{platform}` is not one of {HOSTS:?}"),
            ));
        }
        let class = text_field(object, "class")?;
        if !ARTIFACT_CLASSES.contains(&class.as_str()) {
            return Err(ChannelError::new(
                format!("unknown_artifact_class:{class}"),
                format!("artifact class `{class}` is not one of {ARTIFACT_CLASSES:?}"),
            ));
        }
        if !seen.insert((platform.clone(), class.clone())) {
            return Err(ChannelError::new(
                "duplicate_artifact",
                format!("component `{component}` declares {platform}/{class} more than once"),
            ));
        }
        let url = text_field(object, "url")?;
        if let Some(pin) = rules::forbidden_pin_in(&url) {
            return Err(ChannelError::new(
                format!("forbidden_pin:{pin}"),
                format!("artifact URL `{url}` is a forbidden pin, not an immutable location"),
            ));
        }
        if !rules::is_https_url(&url) {
            return Err(ChannelError::new(
                format!("insecure_artifact_url:{component}"),
                format!("artifact URL `{url}` must be an absolute https:// location"),
            ));
        }
        let sha256 = text_field(object, "sha256")?;
        if !rules::is_digest64(&sha256) {
            return Err(ChannelError::new(
                format!("invalid_artifact_digest:{component}"),
                format!("artifact {platform}/{class} of `{component}` must carry a 64-hex sha256"),
            ));
        }
        let size_bytes = object
            .get("size_bytes")
            .and_then(Json::as_int)
            .ok_or_else(|| {
                ChannelError::new(
                    format!("missing_field:{component}.size_bytes"),
                    format!(
                        "artifact {platform}/{class} of `{component}` must declare `size_bytes`"
                    ),
                )
            })?;
        if size_bytes < 1 {
            return Err(ChannelError::new(
                format!("invalid_artifact_size:{component}"),
                format!(
                    "artifact {platform}/{class} of `{component}` declares size_bytes {size_bytes}, which must be at least 1"
                ),
            ));
        }
        artifacts.push(Artifact {
            platform,
            class,
            url,
            sha256,
            size_bytes,
        });
    }
    Ok(artifacts)
}

fn opt_text(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const CLI_REV: &str = "7ac376b5e5e0f52f4fe3af7fb97971d220277d38";
    const MCP_REV: &str = "cebd159e12b283e3638feffc4cecfa033070cf66";

    fn digest() -> String {
        "aa".repeat(32)
    }

    fn document(components: &str) -> String {
        format!(
            r#"{{"manifest_version":1,"channel":"stable","updated_at":"2026-09-20T00:00:00Z","published":true,"note":"","trust":{{"metadata_version":1,"metadata_expiry":"2027-09-20T00:00:00Z","trust_root":"{root}","signature_artifact":null}},"components":{components}}}"#,
            root = ROOT,
            components = components
        )
    }

    #[test]
    fn a_declared_component_with_a_published_artifact_parses() {
        let text = document(&format!(
            r#"[{{"component":"axiom-cli","status":"declared","version":"0.0.0-dev","revision":"{rev}","declared_source":"axiom-cli/VERSION","needs_restart":true,"artifacts":[{{"platform":"windows-x64","class":"per-user-installer","url":"https://github.com/cognivex006/axiom-cli/releases/download/x/a.zip","sha256":"{sha}","size_bytes":1}}]}}]"#,
            rev = CLI_REV,
            sha = digest()
        ));
        let manifest = Manifest::parse(&text).expect("a well-formed channel must parse");
        assert_eq!(manifest.channel, "stable");
        assert!(manifest.published);
        assert_eq!(manifest.trust.trust_root, ROOT);
        let component = manifest
            .component("axiom-cli")
            .expect("axiom-cli is declared");
        assert!(component.declared);
        assert!(component.needs_restart);
        assert_eq!(component.artifacts.len(), 1);
        assert!(component
            .artifact("windows-x64", "per-user-installer")
            .is_some());
        assert!(component
            .artifact("linux-x64", "per-user-installer")
            .is_none());
    }

    #[test]
    fn a_pep440_version_is_not_normalised_into_semver() {
        let text = document(&format!(
            r#"[{{"component":"axiom-mcp","status":"declared","version":"0.0.0.dev0","revision":"{rev}","declared_source":"axiom-mcp/pyproject.toml","needs_restart":true,"artifacts":[]}}]"#,
            rev = MCP_REV
        ));
        let error = Manifest::parse(&text).expect_err("a PEP 440 version is not SemVer");
        assert_eq!(error.reason, "invalid_semver:axiom-mcp");
    }

    #[test]
    fn an_undeclared_component_is_recorded_with_its_source_and_carries_no_version() {
        let text = document(
            r#"[{"component":"axiom-mcp","status":"undeclared","version":null,"revision":null,"declared_source":"axiom-mcp/pyproject.toml = 0.0.0.dev0 (PEP 440)","needs_restart":true,"artifacts":[]}]"#,
        );
        let manifest = Manifest::parse(&text).expect("an undeclared component is recordable");
        let component = manifest
            .component("axiom-mcp")
            .expect("axiom-mcp is listed");
        assert!(!component.declared);
        assert!(component.version.is_none());
        assert!(component.revision.is_none());
        assert!(component.declared_source.contains("PEP 440"));
    }

    #[test]
    fn a_forbidden_pin_is_refused_as_a_version() {
        for token in ["latest", "main", "*", "HEAD"] {
            let text = document(&format!(
                r#"[{{"component":"axiom-cli","status":"declared","version":"{token}","revision":"{rev}","declared_source":"s","needs_restart":false,"artifacts":[]}}]"#,
                token = token,
                rev = CLI_REV
            ));
            let error = Manifest::parse(&text).expect_err("a forbidden pin must be refused");
            assert_eq!(error.reason, format!("forbidden_pin:{token}"));
        }
    }

    #[test]
    fn a_forbidden_pin_is_refused_as_a_revision() {
        let text = document(
            r#"[{"component":"axiom-cli","status":"declared","version":"0.0.0-dev","revision":"develop","declared_source":"s","needs_restart":false,"artifacts":[]}]"#,
        );
        let error = Manifest::parse(&text).expect_err("a forbidden revision must be refused");
        assert_eq!(error.reason, "forbidden_pin:develop");
    }

    #[test]
    fn a_forbidden_pin_is_refused_as_an_artifact_url() {
        let text = document(&format!(
            r#"[{{"component":"axiom-cli","status":"declared","version":"0.0.0-dev","revision":"{rev}","declared_source":"s","needs_restart":false,"artifacts":[{{"platform":"windows-x64","class":"per-user-installer","url":"latest","sha256":"{sha}","size_bytes":1}}]}}]"#,
            rev = CLI_REV,
            sha = digest()
        ));
        let error = Manifest::parse(&text).expect_err("a forbidden URL must be refused");
        assert_eq!(error.reason, "forbidden_pin:latest");
    }

    #[test]
    fn a_non_https_artifact_url_is_refused() {
        let text = document(&format!(
            r#"[{{"component":"axiom-cli","status":"declared","version":"0.0.0-dev","revision":"{rev}","declared_source":"s","needs_restart":false,"artifacts":[{{"platform":"windows-x64","class":"per-user-installer","url":"http://example.invalid/a.zip","sha256":"{sha}","size_bytes":1}}]}}]"#,
            rev = CLI_REV,
            sha = digest()
        ));
        let error = Manifest::parse(&text).expect_err("http must be refused");
        assert_eq!(error.reason, "insecure_artifact_url:axiom-cli");
    }

    #[test]
    fn an_unsigned_channel_is_refused() {
        let text = document(
            r#"[{"component":"axiom-cli","status":"undeclared","version":null,"revision":null,"declared_source":"s","needs_restart":false,"artifacts":[]}]"#,
        )
        .replace(&format!("\"trust_root\":\"{ROOT}\""), "\"trust_root\":null");
        let error = Manifest::parse(&text).expect_err("an unsigned channel must be refused");
        assert_eq!(error.reason, "unsigned_channel");
    }

    #[test]
    fn an_unresolved_placeholder_is_refused() {
        let text = document(
            r#"[{"component":"axiom-cli","status":"declared","version":"0.0.0-dev","revision":"REPLACE_ME","declared_source":"s","needs_restart":false,"artifacts":[]}]"#,
        );
        let error = Manifest::parse(&text).expect_err("a placeholder revision is not a revision");
        assert_eq!(error.reason, "invalid_revision:axiom-cli");
    }

    #[test]
    fn an_undeclared_component_cannot_smuggle_a_version() {
        let text = document(
            r#"[{"component":"axiom-mcp","status":"undeclared","version":"0.0.0-dev","revision":null,"declared_source":"s","needs_restart":true,"artifacts":[]}]"#,
        );
        let error = Manifest::parse(&text).expect_err("undeclared must not carry a version");
        assert_eq!(
            error.reason,
            "undeclared_component_carries_version:axiom-mcp"
        );
    }

    #[test]
    fn structural_defects_are_named() {
        let cases: Vec<(&str, &str)> = vec![
            (
                r#"{"manifest_version":2,"channel":"stable","updated_at":"2026-09-20T00:00:00Z","published":true,"note":"","trust":{},"components":[]}"#,
                "unsupported_manifest_version",
            ),
            (
                r#"{"manifest_version":1,"channel":"nightly","updated_at":"2026-09-20T00:00:00Z","published":true,"note":"","trust":{},"components":[]}"#,
                "unsupported_channel_name",
            ),
            ("[]", "channel_manifest_not_object"),
            ("not json", "channel_manifest_not_json"),
            ("{}", "missing_field:manifest_version"),
            (
                r#"{"manifest_version":1,"channel":"stable","updated_at":"2026-09-20T00:00:00Z","published":true,"note":"","trust":{},"components":[{"component":"axiom-cli","status":"undeclared","version":null,"revision":null,"declared_source":"s","needs_restart":false,"artifacts":[]}],"extra":1}"#,
                "undeclared_field:extra",
            ),
        ];
        for (text, reason) in cases {
            let error = Manifest::parse(text).expect_err("a defective manifest must be refused");
            assert_eq!(error.reason, reason, "{text}");
        }
    }

    #[test]
    fn an_empty_component_set_cannot_resolve_anything() {
        let text = document("[]");
        assert_eq!(
            Manifest::parse(&text)
                .expect_err("nothing to resolve")
                .reason,
            "empty_component_set"
        );
    }

    #[test]
    fn duplicate_components_are_refused() {
        let one = format!(
            r#"{{"component":"axiom-cli","status":"declared","version":"0.0.0-dev","revision":"{rev}","declared_source":"s","needs_restart":false,"artifacts":[]}}"#,
            rev = CLI_REV
        );
        let text = document(&format!("[{one},{one}]"));
        assert_eq!(
            Manifest::parse(&text)
                .expect_err("duplicates must be refused")
                .reason,
            "duplicate_component"
        );
    }

    #[test]
    fn a_missing_needs_restart_is_refused() {
        let text = document(
            r#"[{"component":"axiom-cli","status":"undeclared","version":null,"revision":null,"declared_source":"s","artifacts":[]}]"#,
        );
        assert_eq!(
            Manifest::parse(&text)
                .expect_err("needs_restart is required")
                .reason,
            "missing_field:axiom-cli.needs_restart"
        );
    }
}
