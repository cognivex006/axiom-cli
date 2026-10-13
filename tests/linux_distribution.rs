//! Static drift guards for the Linux x64 / WSL2 and macOS arm64 distribution
//! tiers (task J-008).
//!
//! These tests read the shipped installer, packaging and schema sources and
//! assert the properties the distribution contract fixes for these tiers. They
//! deliberately do not execute a shell: the executable legs live in
//! `tests/linux/Invoke-AxiomCliLinuxDistributionTests.sh`, which is run by hand
//! on a real Linux host because it needs a Linux-native filesystem. Everything
//! here is cheap, deterministic and runs on any host, so a regression in the
//! POSIX shell, the key set or the "not built" record is caught immediately.
//!
//! The canonical 33-key list is written out independently of both the schema
//! and the installer on purpose. If it were read from one of them, deleting a
//! key from that one artefact could not be detected.

use std::fs;
use std::path::PathBuf;

/// The per-user installer, its shared library, the uninstaller and the release
/// set builder must all be runnable with the platform `/bin/sh` alone.
const LINUX_SOURCES: [&str; 5] = [
    "installers/linux/AxiomCli.Linux.Common.sh",
    "installers/linux/Install-AxiomCli.sh",
    "installers/linux/Uninstall-AxiomCli.sh",
    "packaging/linux/Build-ReleaseSet.sh",
    "packaging/macos-arm64/Build-ReleaseSet.sh",
];

/// The `install-result` envelope key set, in canonical order, owned by
/// `axiom-specs/contracts/axiom-cli-distribution-contract.md` and shared with
/// the Windows profile (`packaging/install-result.schema.json`).
const CANONICAL_ENVELOPE_KEYS: [&str; 33] = [
    "schema_version",
    "spec_version",
    "envelope_kind",
    "operation",
    "outcome",
    "exit_code",
    "status",
    "message",
    "retryable",
    "platform",
    "host",
    "install_root",
    "bin_dir",
    "dry_run",
    "mutated",
    "plan_digest",
    "approved_digest",
    "transaction_id",
    "interrupted_install_recovered",
    "release_set",
    "artifacts",
    "unverified_artifacts",
    "components",
    "refusals",
    "path_rule",
    "service_registration",
    "preserved",
    "removed",
    "elevation_required",
    "shell",
    "limitations",
    "generated_at",
    "request_id",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

/// Extract the string members of the first JSON `"required": [ ... ]` array.
fn first_required_array(json: &str) -> Vec<String> {
    let start = json
        .find("\"required\"")
        .expect("the schema declares required");
    let open = json[start..].find('[').expect("required is an array") + start;
    let close = json[open..].find(']').expect("the array is closed") + open;
    json[open + 1..close]
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

#[test]
fn every_linux_and_macos_script_is_posix_sh_without_forbidden_dependencies() {
    for source in LINUX_SOURCES {
        let text = read(source);
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some("#!/bin/sh"),
            "{source} must start with a POSIX /bin/sh shebang"
        );
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            let line_number = index + 1;
            for forbidden in ["sudo", "doas", "su", "docker", "curl", "wget", "bash"] {
                let invokes = trimmed
                    .strip_prefix(forbidden)
                    .is_some_and(|rest| rest.starts_with(char::is_whitespace));
                assert!(
                    !invokes,
                    "{source}:{line_number} invokes `{forbidden}`; a documented command must \
                     not require elevation, a container runtime or Bash on a supported host"
                );
            }
            assert!(
                !line.contains("ln -s"),
                "{source}:{line_number} creates a symlink; native behavior must not depend on one"
            );
        }
    }
}

#[test]
fn the_installer_emits_exactly_the_canonical_envelope_keys_in_order() {
    let installer = read("installers/linux/Install-AxiomCli.sh");
    let emitted: Vec<String> = installer
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("ac_put ")?;
            let key = rest.split_whitespace().next()?;
            if key.is_empty() {
                None
            } else {
                Some(key.to_string())
            }
        })
        .collect();
    assert_eq!(
        emitted,
        CANONICAL_ENVELOPE_KEYS.to_vec(),
        "the Linux installer must emit the shared install-result envelope key set, in order"
    );
}

#[test]
fn the_linux_schema_requires_exactly_the_canonical_envelope_keys() {
    let schema = read("packaging/linux/install-result.schema.json");
    assert_eq!(
        first_required_array(&schema),
        CANONICAL_ENVELOPE_KEYS.to_vec(),
        "the Linux profile must not diverge from the shared 33-key envelope"
    );
    assert!(
        schema.contains("\"shell\": { \"const\": \"linux-posix-sh\" }"),
        "the Linux profile must pin the shell identifier the installer reports"
    );
}

#[test]
fn the_installer_targets_glibc_and_registers_a_user_service_or_degrades_explicitly() {
    let common = read("installers/linux/AxiomCli.Linux.Common.sh");
    assert!(
        common.contains("GNU_LIBC_VERSION"),
        "the host facts must record the glibc version, because the Linux tier targets glibc"
    );
    assert!(
        common.contains("systemctl --user show-environment"),
        "the systemd-user probe must ask the user manager, not merely look for the binary"
    );
    for precondition in ["/run/systemd/system is absent", "does not answer"] {
        assert!(
            common.contains(precondition),
            "the degradation reason must name the exact failing precondition `{precondition}`"
        );
    }

    let installer = read("installers/linux/Install-AxiomCli.sh");
    assert!(
        installer.contains("ac_systemd_user_reason"),
        "the installer must report why the user service manager is unavailable"
    );
    assert!(
        installer.contains("\"systemd-user-available\""),
        "an explicit --service systemd-user request must be refused, never silently skipped"
    );
    assert!(
        installer.contains("--approve-digest"),
        "a mutating transaction must require an approval digest"
    );
    assert!(
        installer.contains("ac_acquire_lock") && installer.contains("ac_release_lock"),
        "install must be transactional: it takes and releases a lock"
    );

    let uninstaller = read("installers/linux/Uninstall-AxiomCli.sh");
    assert!(
        uninstaller.contains("--purge"),
        "uninstall must preserve user state unless --purge is explicitly approved"
    );
}

#[test]
fn the_macos_recipe_is_arch_parameterised_and_never_cross_builds() {
    let recipe = read("packaging/macos-arm64/Build-ReleaseSet.sh");
    for needle in [
        "--arch",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "arm64",
        "x64",
        "cputype",
    ] {
        assert!(
            recipe.contains(needle),
            "the macOS recipe must be arch-parameterised and record each architecture: `{needle}`"
        );
    }
    assert!(
        recipe.contains("Darwin") && recipe.contains("not_run"),
        "the macOS recipe must refuse to run off a Darwin host and record the leg as not_run"
    );
    assert!(
        recipe.contains("launchd-user"),
        "the macOS recipe must answer NotReady for the launcher it does not build yet"
    );
}

#[test]
fn the_macos_arm64_documentation_does_not_claim_an_available_release() {
    let readme = read("packaging/macos-arm64/README.md");
    assert!(
        readme.contains("released Apple silicon archive") && readme.contains("not established"),
        "the arm64 recipe must not be presented as an available verified release"
    );
    assert!(
        readme.contains("NOT_RUN"),
        "off-platform execution must be documented as unrun, never a fabricated pass"
    );
}
