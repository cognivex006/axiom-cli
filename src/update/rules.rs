//! The frozen identifier patterns, transcribed from the owner documents.
//!
//! Every pattern here is a transcription of a pattern that `axiom-specs` owns, so the
//! distributed CLI accepts exactly what the contract accepts and refuses exactly what it
//! refuses. The transcription is pinned by unit tests against the published vectors.
//!
//! Sources:
//!
//! - `contracts/schemas/update-plan.schema.json` - `SEMVER`, `DIGEST`, `REVISION`,
//!   `TIMESTAMP`, `RELATIVE_PATH`, `PLAN_ID`, `https://` download URLs
//! - `tools/update_plan_contract.py` - `PLACEHOLDER`
//! - `compatibility/platform-matrix.json` `distribution.update_channel.forbidden_pins`

/// Revision pins a channel MUST NOT resolve (platform matrix `forbidden_pins`).
pub const FORBIDDEN_PINS: [&str; 6] = ["main", "master", "develop", "latest", "HEAD", "*"];

/// Placeholder markers that make a version, root or URL undeclared rather than usable.
pub const PLACEHOLDERS: [&str; 2] = ["replace", "fill_me"];

/// `(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?`
pub fn is_semver(text: &str) -> bool {
    let (core, prerelease, build) = split_semver(text);
    if !core.iter().all(|part| is_numeric_identifier(part)) {
        return false;
    }
    if let Some(value) = prerelease {
        if value.is_empty() || !value.split('.').all(is_identifier) {
            return false;
        }
    }
    if let Some(value) = build {
        if value.is_empty() || !value.split('.').all(is_identifier) {
            return false;
        }
    }
    true
}

/// Split a SemVer string into `(major, minor, patch)`, an optional prerelease and an optional
/// build section. A string with more than three dot-separated core fields yields empty core
/// fields, which no numeric identifier check can accept.
fn split_semver(text: &str) -> ([&str; 3], Option<&str>, Option<&str>) {
    let cut = text.find(['-', '+']).unwrap_or(text.len());
    let (core, rest) = text.split_at(cut);
    let (prerelease, build) = if let Some(without_dash) = rest.strip_prefix('-') {
        match without_dash.split_once('+') {
            Some((left, right)) => (Some(left), Some(right)),
            None => (Some(without_dash), None),
        }
    } else if let Some(without_plus) = rest.strip_prefix('+') {
        (None, Some(without_plus))
    } else {
        (None, None)
    };
    let mut parts = core.split('.');
    let first = parts.next().unwrap_or("");
    let second = parts.next().unwrap_or("");
    let third = parts.next().unwrap_or("");
    if parts.next().is_some() {
        return (["", "", ""], None, None);
    }
    ([first, second, third], prerelease, build)
}

/// `0|[1-9]\d*`
fn is_numeric_identifier(text: &str) -> bool {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    text == "0" || !text.starts_with('0')
}

/// `[0-9A-Za-z-]+` per dot-separated identifier.
fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// `^[0-9a-f]{64}$` - a sha256 digest, lowercase hex on purpose: an uppercase digest is a
/// different string and could smuggle a case-only difference past a comparison.
pub fn is_digest64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `^[0-9a-f]{40}$` - a lowercase 40-hex commit revision.
pub fn is_revision40(text: &str) -> bool {
    text.len() == 40
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `^[a-z][a-z0-9-]{0,62}$`
pub fn is_plan_id(text: &str) -> bool {
    let mut bytes = text.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    text.len() <= 63
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `^(?!/)(?!.*\\)(?!.*(?:^|/)\.\.(?:/|$))(?![A-Za-z]:).+$`
///
/// A relative path that cannot escape its root: no absolute prefix, no backslash, no `..`
/// segment, no `C:` drive prefix. This is the ZipSlip defence the transaction contract
/// requires before any archive member is written.
pub fn is_relative_path(text: &str) -> bool {
    if text.is_empty() || text.starts_with('/') || text.contains('\\') {
        return false;
    }
    let mut bytes = text.bytes();
    if let (Some(first), Some(second)) = (bytes.next(), bytes.next()) {
        if first.is_ascii_alphabetic() && second == b':' {
            return false;
        }
    }
    for segment in text.split('/') {
        if segment == ".." {
            return false;
        }
    }
    true
}

/// A download URL must be `https://`; plain `http://`, `file://` and a bare path are refused
/// so a plan can never point acquisition at a mutable local file.
pub fn is_https_url(text: &str) -> bool {
    text.starts_with("https://") && text.len() > "https://".len()
}

/// `PLACEHOLDER = REPLACE|FILL_ME`, case-insensitive.
pub fn contains_placeholder(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    PLACEHOLDERS.iter().any(|marker| lowered.contains(marker))
}

/// The forbidden pins a token would resolve if it were used as a version or revision.
///
/// A pin is refused as a whole token; `latest` inside an unrelated word is not a pin.
pub fn forbidden_pin_in(text: &str) -> Option<&'static str> {
    let trimmed = text.trim();
    FORBIDDEN_PINS
        .iter()
        .copied()
        .find(|pin| trimmed.eq_ignore_ascii_case(pin))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_accepts_the_owner_declared_versions_and_refuses_pep440() {
        for accepted in [
            "0.0.0-dev",
            "0.1.0-draft.1",
            "0.1.0",
            "1.2.3",
            "10.20.30",
            "1.0.0-alpha.1",
            "1.0.0+build.7",
            "0.0.0-dev0",
        ] {
            assert!(is_semver(accepted), "`{accepted}` must be accepted");
        }
        for refused in [
            "",
            "0.0.0.dev0",
            "0.1",
            "1",
            "v1.2.3",
            "01.2.3",
            "1.02.3",
            "1.2.03",
            "latest",
            "main",
            "*",
            "1.2.3-",
            "1.2.3+",
            "1.2.3 alpha",
            "1.2.3-a_b",
        ] {
            assert!(!is_semver(refused), "`{refused}` must be refused");
        }
    }

    #[test]
    fn digest_and_revision_are_lowercase_hex_only() {
        assert!(is_digest64(&"a1".repeat(32)));
        assert!(!is_digest64(&"A1".repeat(32)));
        assert!(!is_digest64(&"a".repeat(63)));
        assert!(!is_digest64(&"a".repeat(65)));
        assert!(!is_digest64(&format!("{}g", "a".repeat(63))));
        assert!(is_revision40(&"0f".repeat(20)));
        assert!(!is_revision40(&"0F".repeat(20)));
        assert!(!is_revision40(&"0f".repeat(19)));
    }

    #[test]
    fn plan_ids_freeze_the_schema_pattern() {
        assert!(is_plan_id("update-stable-to-0-1-0"));
        assert!(is_plan_id("a"));
        assert!(is_plan_id(&format!("a{}", "b".repeat(62))));
        assert!(!is_plan_id(""));
        assert!(!is_plan_id(&format!("a{}", "b".repeat(63))));
        assert!(!is_plan_id("Update-stable"));
        assert!(!is_plan_id("1update"));
        assert!(!is_plan_id("update_stable"));
        // A trailing hyphen is inside the frozen character class, so the schema accepts it.
        // Verified against the reference matcher as well as the schema text.
        assert!(is_plan_id("update-stable-"));
    }

    #[test]
    fn relative_paths_cannot_escape_a_root() {
        for accepted in [
            "bin/axiom-cli/bin/axiom-cli.exe",
            "generation.json",
            "a/b/c.txt",
            "a/..b/c",
            "a/b..",
        ] {
            assert!(is_relative_path(accepted), "`{accepted}` must be accepted");
        }
        for refused in [
            "",
            "/etc/passwd",
            "..\\..\\windows",
            "a/../../b",
            "..",
            "../a",
            "a/..",
            "C:/windows",
            "c:x",
            "a\\b",
        ] {
            assert!(!is_relative_path(refused), "`{refused}` must be refused");
        }
    }

    #[test]
    fn urls_must_be_https() {
        assert!(is_https_url("https://github.com/cognivex006/axiom-cli"));
        assert!(!is_https_url("http://github.com/cognivex006/axiom-cli"));
        assert!(!is_https_url("https://"));
        assert!(!is_https_url("file:///c:/tmp/artifact.bin"));
        assert!(!is_https_url("c:/tmp/artifact.bin"));
        assert!(!is_https_url(""));
    }

    #[test]
    fn placeholders_and_pins_are_detected() {
        assert!(contains_placeholder("REPLACE_ME"));
        assert!(contains_placeholder("fill_me"));
        assert!(!contains_placeholder("0.1.0-draft.1"));
        for pin in FORBIDDEN_PINS {
            assert_eq!(forbidden_pin_in(pin), Some(pin));
            assert_eq!(forbidden_pin_in(&pin.to_ascii_uppercase()), Some(pin));
        }
        assert_eq!(forbidden_pin_in(" latest "), Some("latest"));
        assert_eq!(forbidden_pin_in("latest-build"), None);
        assert_eq!(forbidden_pin_in("0.1.0"), None);
    }
}
