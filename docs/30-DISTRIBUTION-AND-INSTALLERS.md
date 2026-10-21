# Distribution and installers

For installation commands, start with [Installing Axiom](INSTALL.md). This guide explains the mechanisms behind those commands.

## Components and ownership

Canonical terminal commands are `axm` (runtime and host manager), `axge` (graph engine) and `axmp` (local API/MCP gateway). `$axsk` is the agent workflow skill. The old terminal `axsk` is retained as a deprecated forwarder during migration. The release also contains a portable skills bundle. The private installation engine lives in `<root>/bin/graphd-engine/`; raw component names in release archives are package identities, not additional installed commands.

The distribution verifies inputs, presents the plan, provisions the MCP runtime and records the installed release. The graph engine owns ecosystem placement, activation, service lifecycle and removal. Calls use program and argv rather than an interpolated shell command.

`axm install` installs the release's complete pinned component set. It does not accept a component name: `axm install cli` and `axm install axiom-graphd` are unsupported. The development `axm skills install codex` adapter registers verified local content; host discovery remains a separate [skills verification step](https://github.com/cognivex006/axiom-skills/blob/main/marketplace-plugins/README.md).

## Release archives and bootstrappers

Releases provide Windows x64, Linux x64 and Intel Mac archives. WSL2 uses the Linux archive. Apple silicon and Linux arm64 are not available through the current one-line installer. Container images have a separate publication path.

The one-line scripts are generated from `installers/oneline/` by `packaging/oneline/generate_bootstrappers.py`. Each script embeds a version tag, archive names and SHA-256 values. The `latest/download` URL selects a script; the script downloads its own pinned archive, verifies it before extraction and invokes the release CLI. Unsupported hosts, elevation, download errors, hash mismatches and extraction failures stop the bootstrap.

Generation is deterministic: the same tag and checksum input produce the same script bytes. `release/oneline_assets.py` writes the scripts, pinned `channel.json` and updated `SHA256SUMS` for collected release archives.

## Installation transaction

From an extracted release, `axm install --dry-run` discovers the adjacent `channel.json`, verifies the artifact lengths and hashes and reports the plan. The interactive command asks `Proceed? [Y/n]`; `--yes` approves that printed plan. Automation can use `--apply --approve-digest <sha256>`.

Installation refuses a foreign nonempty root or missing required engine input, provisions the release's hash-bound MCP runtime, assembles the skills bundle and asks the engine to plan and apply placement. It places the three canonical commands in `<root>/bin` (and retains an existing owned axsk compatibility slot during migration) and the private engine under `<root>/bin/graphd-engine/`, retains generation payloads and writes `installed.json` atomically after activation. A crash before the final record may require retry or recovery; it is not proof of a successful install.

| Platform | Default root |
|---|---|
| Windows | `%LOCALAPPDATA%\Axiom` |
| Linux and macOS | `$XDG_DATA_HOME/axiom`, otherwise `~/.local/share/axiom` |

The installed record stores component versions and hashes, bin digests, the generation and channel source. `version`, `doctor`, `update` and `uninstall` read that record.

## Confirmation and PATH

Without a terminal and without `--yes`, interactive mutation stops with exit `4`. Declining confirmation returns `5`. `--dry-run` never changes the host. The explicit `--apply` workflow requires the exact plan digest and does not perform the interactive PATH addition.

The interactive install announces a per-user PATH change. Windows writes `HKCU\Environment` while preserving the registry value type. POSIX appends a marked entry to the login profile (`.zprofile`, `.bash_profile` or `.profile`). `--no-modify-path` skips it. `path-change.json` records the exact appended text or registry change so removal can preserve unrelated user edits. Existing records remain valid when installer comment wording changes.

## Legacy installations

`doctor` reports recognized older layouts, executables that shadow the current CLI and environment overrides. A legacy CLI store at the selected root can be adopted in place, with its old executable backed up. A bootstrap root elsewhere is left alongside the new installation unless `--adopt <path>` explicitly selects it. Project files, graph output and user data are retained.

For pre-generation 0.1.0 and 0.1.2 CLI stores, adoption verifies the original install manifest's executable name, installed location, size and SHA-256. The old manifest and `cli/` store remain intact. An existing backup must match the recorded bytes; incomplete or ambiguous records and linked paths are refused. Version 0.1.9 corrects this 0.1.8 adoption failure.

Remove leftover `AXIOM_CLI_INSTALL_ROOT`, `AXIOM_ENGINE_BIN` and `AXIOM_HOME` settings from the old installation procedure before upgrading. See [migration instructions](INSTALL.md#upgrading-from-010-or-012).

## Update and removal

The update script finds the installed root and runs its update command. It can migrate recognized older layouts through the installer. With nothing installed, it prints the installation command. It refuses elevation and conflicting legacy overrides.

[Update and rollback](50-UPDATE-CHANNEL.md) explains the transaction and its distinct automation paths. Uninstall delegates removal to the engine, removes owned CLI/bin files and the recorded PATH addition, and retains user data. Windows renames a running executable before removal because the active image cannot delete itself.

## Verification

The Rust suite checks parsing, plans, integrity, activation, refusal and recovery boundaries. Bootstrap tests verify deterministic output and download/elevation/hash refusals. Fresh native CI exercises release installation on Windows, Linux and Intel Mac. Real WSL verification is recorded separately from native Windows execution; containers do not establish native macOS or Windows behavior.

Contributor commands and retained evidence belong in [Development](../Development.md). A passing installation check does not establish project registration, graph generation or AI-host configuration.

Updates retain the previous recorded channel manifest alongside the previous installed record. Failed updates and explicit rollbacks restore both together; a changed retained manifest is refused before restoring the installation.
