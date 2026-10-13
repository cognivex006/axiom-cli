# Axiom release

## Installation

Install the CLI, graph engine, MCP runtime and skills bundle for your user account. The scripts verify this release's archive SHA-256 before extraction; installation shows its plan and asks once.

Windows (PowerShell 5.1+):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/install.ps1 | iex"
```

macOS Intel, Linux x64 and WSL2:

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/install.sh | sh
```

## Update

Run `axm update`, or use the release's update script:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/update.ps1 | iex"
```

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/update.sh | sh
```

The script uses an existing installation's update command; recognized older layouts upgrade through the installer. With nothing installed, it prints the installation command. Check the reported installed/offered version and any refusal before assuming an upgrade succeeded.

## Commands and setup

`axm` and `axiom-cli` run the same program. `axiom` is the graph engine's separate operations CLI. Use `axm version`, `axm doctor` and `axm --help` to inspect the installation. `axm uninstall` retains user data.

`install` manages the complete pinned component set; it does not accept a component name. Project registration, MCP permissions and host plugin discovery need their own setup. See the [installation guide](https://github.com/orchex006/axiom-cli/blob/main/docs/INSTALL.md) and [skills/plugin guide](https://github.com/orchex006/axiom-skills/blob/main/marketplace-plugins/README.md).

Every release asset is listed in `SHA256SUMS`. `channel.json` pins archive downloads; use `download/<tag>` instead of `latest/download` to select a particular release. Apple silicon and Linux arm64 are unavailable through these scripts.

## Changes

See [Changelog](https://github.com/orchex006/axiom-cli/blob/main/Changelog.md) for versioned New Features, Bug Fixes and known limitations. Release maintainers should include the selected version's changes and tag-to-tag comparison in the published notes.
