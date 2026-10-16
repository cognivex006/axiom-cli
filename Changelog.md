# Changelog

Changes are listed by release, newest first. For downloads, see [GitHub Releases](https://github.com/orchex006/axiom-cli/releases).

## Unreleased

### Bug Fixes

- Keep preserved older installation files informational in doctor; identify actual PATH and environment warnings separately.

### New Features

- Add managed Codex skills install/update/doctor/list/uninstall/recovery through axm; bind optional agent setup to the installation plan and preserve partial results. Support local gateway supervision and explain deprecated terminal axsk.

## 0.1.9

### Bug Fixes

- Restore adoption of unchanged 0.1.0 and 0.1.2 CLI stores by checking their original install manifest. Preserve the old executable and metadata, and refuse modified files or conflicting backups.
- Verify adoption through the native one-line installer using published 0.1.2 CLI bytes and a legacy store fixture. Check updates and exact rollback from 0.1.8.

## 0.1.8

### New Features

- Ship two default workflow skills: axge for graph work and axm for ecosystem lifecycle, with focused references and optional legacy packages.
- Include incremental graph build, instance-wide polling watch and verified selected JSON/HTML export with measured engine-cycle diagnostics.

### Bug Fixes

- Align native packaging with the approved workflow specification while retaining previous owner input pins for repair and migration.

- Keep graph publications correct after the last source file is removed or recreated, retain the valid graph after malformed saves, and return repository-relative source locations.
- Verify upgrades and exact rollback from both the previous canonical command layout and the older legacy layout. Check current public command names in the published Mac Intel smoke test.

### Known Limitations

- Analysis covers declarations and containment; complete method call graphs are not claimed. Phase, queue and source observation timings remain unavailable. Project-only builds may leave the full solution catalog pending.
- Mac ARM and unrun AI host runtime behavior remain unverified.


## 0.1.7

### New Features

- Provide canonical native commands `axm`, `axsk`, `axge` and `axmp` with owned, verified placement. Route skills to the engine and MCP to its provisioned runtime without a shell.

### Bug Fixes

- Remove verified commands added during an upgrade when rolling back to an older layout. Refuse changed or linked files before rollback and retain the previous manifest and generation.

- Upgrade older command layouts through a checksum-verified current CLI, so the previous updater does not apply its obsolete executable health checks after migration. Both shell and PowerShell keep the normal single approval and preserve the detected install root.

- Restore the exact recorded channel manifest with the previous generation after a failed update or explicit rollback, keeping doctor integrity checks valid. Refuse changed rollback snapshots before mutation.

- Make the shell updater prefer the installation selected by `axm` when a separate legacy CLI also exists on PATH, matching PowerShell.

- Make doctor report actual SQLite and MCP runtime readiness with concise results and actionable repair steps. SQLite version numbers are informational; verbose mode keeps paths and scope without parser trace noise.
- Place the private installation engine in `bin/graphd-engine/`. Install and update migrate verified older layouts while retaining owned rollback files and protecting modified or foreign files.

- Install only `axm`, `axsk`, `axge` and `axmp` on PATH. Keep installation helpers private and migrate only unchanged, recorded legacy executables; preserve verified rollback payloads.

- Show four component/command/version rows in `axm version`; remove the redundant installed summary, legacy command labels, channel hint and success footer.

## 0.1.6

### New Features

- Include ten agent skills, with `graph-build` for initial indexing and `graph-watch` for automatic graph refresh on source saves through the graph daemon.
- Align the CLI, graph engine, MCP and skills at version 0.1.6. Publish refreshed shell and PowerShell installation and update scripts with pinned downloads and checksums.

### Bug Fixes

- Read the pinned package version in Windows/Linux runtime bootstrap instead of an older hard-coded MCP version. This keeps fresh installation consistent with the release payload.
- Repair an installed ecosystem using its verified active skills bundle when the original release archive is no longer present.
- Show a short diagnostic summary with clear missing prerequisites and next steps. Retain detailed checks with `doctor --verbose` or `--json`.
- Show the CLI, graph engine, MCP and skills versions separately. Explain that `axiom` and `axiom-graphd` belong to the same engine release and distinguish recorded versions from an update check.

### Documentation

- Rewrite CLI guides and release notes around commands, supported platforms and user-visible changes. Remove internal development identifiers from public documentation and generated installer descriptions.
- Add direct links to graph engine commands, MCP setup and host plugin installation. Explain the installation confirmation and the separate project/AI-host setup steps.

### Bug Fixes

- Remove internal specification references from help, diagnostic explanations and newly written shell-profile comments.
- Identify the bundled SQLite check by its owning component in JSON diagnostic details.

### Chores

- Add an automated check for internal identifiers in public documents, installer templates, generated release metadata and CLI text.
- Extend fresh-runner installation checks to Intel Macs, including update checks, uninstall and reinstall. These checks do not establish upgrades between different released versions.

Full Changelog: [v0.1.5…v0.1.6](https://github.com/orchex006/axiom-cli/compare/v0.1.5...v0.1.6)

## 0.1.5

### New Features

- Add `axm` as the short command for `axiom-cli`. Both names run the same program and support the same commands; installation, update, rollback and uninstall manage both entrypoints.
- Bundle core, MCP and skills version 0.1.5. MCP and agent skills can expose the installation manager's commands.

Full Changelog: [v0.1.4…v0.1.5](https://github.com/orchex006/axiom-cli/compare/v0.1.4...v0.1.5)

## 0.1.4

### New Features

- Add `update.sh` and `update.ps1` to release downloads. They find an existing installation and run its update command; older installations upgrade through the verified installer. With nothing installed, they print the installation command.
- Stop with clear instructions when legacy environment overrides would select the wrong installation.

Full Changelog: [v0.1.3…v0.1.4](https://github.com/orchex006/axiom-cli/compare/v0.1.3...v0.1.4)

## 0.1.3

### New Features

- Install Axiom with a single shell or PowerShell command. Release scripts pin the archive version and SHA-256, verify downloads before extraction and refuse elevated execution.
- Show the installation or removal plan and ask for confirmation once. Add the installation's `bin` directory to the user PATH, with `--no-modify-path` to skip that change.
- Add one-command updates with version and checksum verification, a retained previous generation and `update rollback --transaction previous`.
- Detect legacy installation layouts, older executables on PATH and leftover environment overrides. Support in-place adoption of recognized legacy roots while preserving user data.

### Bug Fixes

- Place CLI and core executables together and write the installation record after activation, so version and diagnostic commands can discover the installed release.
- Avoid nesting the MCP runtime directory during legacy Windows installation.
- Treat Rust as a build-time dependency for installed releases and recognize the bundled Python runtime. Report an unknown bundled SQLite version as a non-blocking finding.

### Documentation

- Add the installation guide with options, update, removal, legacy migration and troubleshooting.

Full Changelog: [v0.1.2…v0.1.3](https://github.com/orchex006/axiom-cli/compare/v0.1.2...v0.1.3)

## 0.1.2

### New Features

- Provide Windows x64, Linux x64 and Intel Mac distribution archives containing the CLI, graph engine, portable MCP and skills, and pinned Python/dependency inputs. WSL2 uses the Linux archive.
- Keep installation and activation under the graph engine's transaction, with digest approval and retained generations for rollback.

### Known Limitations

- These test-release archives use the older explicit bootstrap and approval workflow. Follow the [historical release instructions](release/v0.1.2-notes.md); newer releases provide the one-line installer.

Full Changelog: [v0.1.1…v0.1.2](https://github.com/orchex006/axiom-cli/compare/v0.1.1...v0.1.2)

## 0.1.1

### New Features

- Publish an Intel Mac test archive with CLI, graph engine, MCP, skills and an offline Python runtime. Verify each downloadable asset with `SHA256SUMS`.

### Known Limitations

- Native binaries are unsigned and not notarized. The embedded channel describes a local offline payload; it is not a signed automatic update feed. See the [release instructions](release/v0.1.1-notes.md).

Full Changelog: [v0.1.0…v0.1.1](https://github.com/orchex006/axiom-cli/compare/v0.1.0...v0.1.1)

## 0.1.0

### New Features

- Introduce the experimental Windows distribution entrypoint, per-user install/removal scripts and approval-bound plans.
- Define `install`, `update`, `doctor`, `version` and `uninstall`, with structured JSON reports and stable exit codes.

### Known Limitations

- This initial release predates the complete ecosystem installer. Use the [current installation guide](docs/INSTALL.md) for a new installation or migration.
