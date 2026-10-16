# Development — axiom-cli

## Pinned governance

The canonical workflow is Development.md in `axiom-specs` at the exact revision in `spec.lock.json`. A missing or unverified pin blocks implementation; do not treat `spec.lock.example.json` as valid authorization.

## Before work

Read the selected task, the distribution contract and the platform matrix, verify completed dependencies, claim a single owner, record branch/base revision/dirty tree, explicit commit and push authority, allowed files and tests. Preserve unrelated changes. A–G and J letters do not override task `repo` ownership.

## During work

Implement one bounded task; coordinate public changes through spec ADR/RFC first. Include positive, negative and failure-boundary tests. The distributed executable is a thin wrapper over the `axiom-graphd` installation engine; never fork or re-implement that engine here. Installer and update code must be transactional: verify a digest before use, swap atomically, keep a rollback artifact, and self-update `axiom-cli` inside the same transaction. Native behavior must not depend on Bash, WSL, Docker, administrator privileges or symlinks. Executable launches use program and argv, not an interpolated shell string. Version-pinned one-line bootstrappers verify archive checksums before delegating to the CLI. No hidden PATH mutation or silent auto-update.

## Completion

Attach real test output/hashes, acceptance mapping, changed-file review, compatibility and rollback notes, updated local docs and Changelog.md. Record which native targets were actually executed and which stayed unverified; a container image is never native runtime evidence and a WSL2 run is never Windows evidence. Complete the canonical six preflight and eight completion checks; spec tooling validates evidence consistency, but independent CI reruns remain required.

## Branch/release policy

Follow the pinned canonical governance. Do not invent a remote, push credentials or release version. `axiom-cli` is the canonical distribution repository; the daemon plus CLI share one core release, while MCP and skills version independently. Documentation follows its component version. No production action is authorized by copying this seed.

## Public documentation and release notes

Public guides, help, installer explanations and release notes describe commands and user-visible behavior. Keep internal task/spec identifiers, private evidence paths, completion checklists and function-tracking labels in internal records. Versions, exit codes and links to public issues or pull requests are appropriate where they explain the change.

Changelog entries use one Unreleased section followed by descending versions. Group changes under New Features, Bug Fixes, Documentation, Chores or Known Limitations when applicable. Describe what a user can now do or which failure was corrected. Published notes are rendered from the selected version's entry with installation instructions and a tag comparison. Never present a candidate, unrun platform or same-version smoke check as a completed release upgrade.

Required editorial checks:

```sh
python3 release/check_public_docs.py
python3 -m unittest tests.test_public_docs tests.test_bootstrappers -v
```

The public Changelog stays free of internal identifiers. Canonical task completion retains a separate internal changelog receipt and historical snapshots. Published version tags, assets and historical evidence remain immutable; documentation edits do not imply a new release.

## Agent onboarding and local gateway development

The 2026-10-16 owner request authorizes the approved agent onboarding implementation on scoped feature branches. Canonical terminal names are axm/axge/axmp; axsk is the agent skill. Host lifecycle is managed once by axm. Gateway changes retain authenticated multi-project scope and verified generations, isolated process ownership, bounded retention and configurable three-hour idle shutdown. Version remains unchanged and this feature work is unpublished; independent detailed shared-contract/security review remains required before integration/publication.

## Authorized 0.1.10 release

The user explicitly requested merge and release as 0.1.10 on 2026-10-16. This release prepares the previously verified onboarding/gateway source and release documentation. This supersedes the earlier feature-only restriction. Main integration, new release/v0.1.10 branch, immutable v0.1.10 tag and publication are authorized after independent shared-contract review and required owner/native checks. Older tags/assets and real installed user state remain preserved.

## Authorized 0.1.11 patch release

The user explicitly requested all reported fixes and merge/release as 0.1.11 on 2026-10-16. This supersedes prior feature-only restrictions. Preserve installed user state and immutable historical Releases/tags. Main integration, new release branches/tags and publication require independent changed-contract review, required owner/native checks, exact source/artifact binding and actual previous-release upgrade/rollback.
