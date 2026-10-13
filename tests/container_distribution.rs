//! Pins the `axiom-cli` container delivery channel (task J-006).
//!
//! The assertions read the shipped assets directly instead of importing a shared table,
//! so a drift between the Dockerfile, the entrypoint, the publication workflow and the
//! owner documentation is detectable rather than impossible. Every expectation here is an
//! independent statement of a rule owned by `axiom-specs`:
//! `contracts/axiom-cli-distribution-contract.md` section 5 and
//! `compatibility/platform-matrix.json` `distribution.container`.
//!
//! These checks are static on purpose: they must pass on a host with no Docker daemon, so
//! they guard the definition. The runtime proof (build, digest, verb exit codes) is
//! recorded as evidence for J-006 instead of being asserted here.

use std::fs;
use std::path::PathBuf;

/// Base images pinned in `containers/Dockerfile`, restated here independently.
const TAG_BUILDER: &str = "rust:1.85-slim-bookworm";
const TAG_RUNTIME: &str = "debian:bookworm-slim";
const DIGEST_BUILDER: &str =
    "sha256:9f841bbe9e7d8e37ceb96ed907265a3a0df7f44e3737d0b100e7907a679acb36";
const DIGEST_RUNTIME: &str =
    "sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "`{relative}` must exist and be readable UTF-8: {error}",
            relative = relative,
            error = error
        )
    })
}

fn dockerfile() -> String {
    read("containers/Dockerfile")
}

fn entrypoint() -> String {
    read("containers/entrypoint.sh")
}

fn workflow() -> String {
    read(".github/workflows/publish-container.yml")
}

/// Lines belonging to the block introduced by `marker`, de-indented, stopping at the first
/// line that is no longer more indented than the marker itself.
fn indented_block(text: &str, marker: &str) -> Vec<String> {
    let mut collected: Vec<String> = Vec::new();
    let mut base: Option<usize> = None;
    for line in text.lines() {
        match base {
            Some(indentation) => {
                if line.trim().is_empty() {
                    continue;
                }
                if line.len() - line.trim_start().len() <= indentation {
                    break;
                }
                collected.push(line.trim().to_string());
            }
            None => {
                if line.trim() == marker {
                    base = Some(line.len() - line.trim_start().len());
                }
            }
        }
    }
    collected
}

/// Executable lines of a shell script: comments and blanks dropped, so a guard cannot be
/// tripped by prose in a doc comment.
fn executable_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

fn is_lower_hex(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
}

#[test]
fn every_base_image_is_pinned_by_digest() {
    let text = dockerfile();
    let mut pinned: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("FROM ") {
            continue;
        }
        let reference = trimmed
            .split_whitespace()
            .nth(1)
            .unwrap_or_else(|| panic!("`{trimmed}` must name a base image"));
        let digest = reference
            .split_once("@sha256:")
            .unwrap_or_else(|| panic!("base image `{reference}` must be pinned by digest"))
            .1;
        assert!(
            is_lower_hex(digest) && digest.len() == 64,
            "base image `{reference}` must carry a 64-character lowercase hex digest, got `{digest}`"
        );
        assert!(
            !reference.contains(":latest"),
            "base image `{reference}` must not float on a tag"
        );
        pinned.push(reference.to_string());
    }
    let expected = vec![
        format!("{TAG_BUILDER}@{DIGEST_BUILDER}"),
        format!("{TAG_RUNTIME}@{DIGEST_RUNTIME}"),
    ];
    assert_eq!(
        pinned, expected,
        "the builder and runtime stages must be the two pinned digests"
    );
}

#[test]
fn oci_label_set_is_complete() {
    let text = dockerfile();
    let required: [(&str, &str); 4] = [
        (
            "org.opencontainers.image.source",
            "org.opencontainers.image.source=\"https://github.com/orchex006/axiom-cli\"",
        ),
        (
            "org.opencontainers.image.revision",
            "org.opencontainers.image.revision=\"${AXIOM_REVISION}\"",
        ),
        (
            "org.opencontainers.image.version",
            "org.opencontainers.image.version=\"${AXIOM_VERSION}\"",
        ),
        (
            "org.opencontainers.image.licenses",
            "org.opencontainers.image.licenses=\"UNLICENSED\"",
        ),
    ];
    for (label, expectation) in required {
        assert!(
            text.contains(expectation),
            "the image must carry `{label}` as `{expectation}`"
        );
    }
    assert!(
        text.contains("LABEL "),
        "the OCI labels must be declared with LABEL"
    );
    for label in [
        "org.opencontainers.image.title",
        "org.opencontainers.image.description",
        "org.opencontainers.image.url",
        "org.opencontainers.image.documentation",
        "org.opencontainers.image.created",
    ] {
        assert!(
            text.contains(label),
            "the OCI label set must include `{label}`"
        );
    }
}

#[test]
fn runtime_user_is_non_root_and_the_revision_is_not_invented() {
    let text = dockerfile();
    assert!(
        text.contains("useradd --uid 10001"),
        "the runtime user must be created with an explicit non-root uid"
    );
    let user_lines: Vec<String> = text
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| line.starts_with("USER "))
        .collect();
    assert_eq!(
        user_lines,
        vec!["USER 10001:10001".to_string()],
        "the image must switch to the non-root runtime user"
    );
    assert!(
        text.contains("ARG SOURCE_DATE_EPOCH"),
        "the Dockerfile must declare SOURCE_DATE_EPOCH so the image timestamp can be deterministic"
    );
    assert!(
        text.contains("ARG AXIOM_REVISION=unknown"),
        "a bare local build must default the revision label to `unknown` instead of inventing one"
    );
}

#[test]
fn dockerfile_version_default_matches_the_version_file() {
    let version = read("VERSION").trim().to_string();
    assert!(
        !version.is_empty(),
        "VERSION must not be empty; the image version is derived from it"
    );
    let text = dockerfile();
    let declared = text
        .lines()
        .map(|line| line.trim().to_string())
        .find_map(|line| line.strip_prefix("ARG AXIOM_VERSION=").map(str::to_string))
        .expect("the Dockerfile must declare `ARG AXIOM_VERSION=`");
    assert_eq!(
        declared, version,
        "the Dockerfile version default must track VERSION so the image cannot silently mislabel itself"
    );
}

#[test]
fn the_container_is_declared_a_non_native_channel() {
    let text = dockerfile();
    assert!(
        text.contains("io.orchex006.axiom.native-evidence=\"false\""),
        "the image must label itself as non-native evidence"
    );
    assert!(
        text.contains("io.orchex006.axiom.channel=\"container-linux-x64\""),
        "the image must label itself with its delivery platform id"
    );
    assert!(
        text.contains("NOT native runtime evidence"),
        "the Dockerfile must state that a container run is not native runtime evidence"
    );
    assert!(
        text.contains("prerequisite for a native installation path"),
        "the Dockerfile must state that the container is not a native prerequisite"
    );
    assert!(
        workflow().contains("NOT native runtime evidence"),
        "the publication workflow must repeat the non-native-evidence statement"
    );
    assert!(
        entrypoint().contains("native evidence"),
        "the entrypoint must document that a container run is not native evidence"
    );
}

#[test]
fn entrypoint_forwards_the_argv_vector_unchanged() {
    let text = entrypoint();
    assert!(
        text.starts_with("#!/bin/sh"),
        "the entrypoint must be a POSIX shell script"
    );
    assert!(
        !text.contains('\r'),
        "the entrypoint must use LF endings to run on Linux"
    );
    assert!(
        text.contains("exec /usr/local/bin/axiom-cli \"$@\""),
        "the entrypoint must exec the binary with the argv vector verbatim"
    );
    assert!(
        text.contains("set -eu"),
        "the entrypoint must fail fast rather than continue after an error"
    );
    let code = executable_lines(&text).join("\n");
    for forbidden in ["eval", "sh -c", "$*", "$(", "`"] {
        assert!(
            !code.contains(forbidden),
            "the entrypoint must not use `{forbidden}`: argv reaches the binary unmodified"
        );
    }
    assert_eq!(
        executable_lines(&text).len(),
        2,
        "the entrypoint must do nothing beyond failing fast and exec'ing the binary"
    );
}

#[test]
fn workflow_builds_with_buildx_and_publishes_immutable_tags_only() {
    let text = workflow();
    assert!(
        text.contains("docker/setup-buildx-action"),
        "the publication workflow must build with buildx"
    );
    assert!(
        text.contains("docker/build-push-action"),
        "the publication workflow must publish through build-push-action"
    );
    assert!(
        text.contains("platforms=\"linux/amd64\""),
        "the finish-first tier starts from linux/amd64"
    );
    assert!(
        text.contains("platforms=\"${platforms},linux/arm64\""),
        "linux/arm64 must be reachable but opt-in"
    );
    assert!(
        text.contains("publish_arm64") && text.contains("${{ inputs.publish_arm64 }}"),
        "linux/arm64 must be gated behind an explicit input so it is never an unverified placeholder"
    );
    assert!(
        text.contains("ghcr.io/orchex006/axiom-cli"),
        "the workflow must target the contract image reference"
    );

    let tags = indented_block(&text, "tags: |");
    assert!(
        !tags.is_empty(),
        "the build step must declare an explicit tag list"
    );
    assert!(
        !tags.iter().any(|line| line.contains("latest")),
        "a floating tag must not exist: {tags:?}"
    );
    assert!(
        tags.iter()
            .any(|line| line.contains(":${{ steps.version.outputs.version }}")),
        "the image must carry its immutable version tag: {tags:?}"
    );
    assert!(
        tags.iter()
            .any(|line| line.contains(":git-${{ steps.version.outputs.short_sha }}")),
        "the image must carry a commit-derived tag: {tags:?}"
    );
}

#[test]
fn workflow_records_the_digest_and_keeps_publication_opt_in() {
    let text = workflow();
    assert!(
        text.contains("${{ steps.build.outputs.digest }}"),
        "the workflow must read the digest buildx produced"
    );
    assert!(
        text.contains("immutable_reference=${{ env.IMAGE }}@$DIGEST"),
        "the workflow must record the immutable digest reference"
    );
    assert!(
        text.contains("push: ${{ steps.mode.outputs.push == 'true' }}"),
        "pushing must be decided by the resolved mode, not hard-coded"
    );
    assert!(
        text.contains("default: false"),
        "a manual run must default to not pushing"
    );
    assert!(
        text.contains("packages: write"),
        "publishing needs the packages scope"
    );
    assert!(
        text.contains("contents: read"),
        "the workflow must not ask for write access to repository contents"
    );
    assert!(
        text.contains("SOURCE_DATE_EPOCH=${{ steps.version.outputs.epoch }}"),
        "the workflow must pass the commit time as SOURCE_DATE_EPOCH"
    );
    assert!(
        text.contains("distribution.container.published_image_digest"),
        "the workflow must say where the digest is recorded"
    );
    for forbidden in ["docker push", "force", "git tag"] {
        assert!(
            !text.contains(forbidden),
            "the workflow must not contain `{forbidden}`"
        );
    }
}

#[test]
fn owner_documentation_covers_the_channel_and_the_air_gapped_variant() {
    let doc = read("docs/50-CONTAINER-CHANNEL.md");
    assert!(
        doc.contains(TAG_BUILDER) && doc.contains(TAG_RUNTIME),
        "the channel doc must name both base images"
    );
    assert!(
        doc.contains(DIGEST_BUILDER) && doc.contains(DIGEST_RUNTIME),
        "the channel doc must record both pinned base digests"
    );
    assert!(
        doc.contains("air-gapped"),
        "the channel doc must describe the offline or air-gapped variant"
    );
    assert!(
        doc.contains("not\nnative runtime evidence") || doc.contains("not native runtime evidence"),
        "the channel doc must state that a container run is not native runtime evidence"
    );
    assert!(
        read("docs/README.md").contains("50-CONTAINER-CHANNEL.md"),
        "the documentation index must link the container channel guide"
    );
    assert!(
        read("README.md").contains("50-CONTAINER-CHANNEL.md"),
        "the repository README must link the container channel guide"
    );
}
