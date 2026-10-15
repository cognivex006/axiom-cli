# Installing Axiom

One command installs Axiom for your user account:

- `axm`: Main CLI / Orchestrator.
- `axsk`: Skills Manager.
- `axge`: Graph Engine.
- `axmp`: MCP Server / Gateway.

You do not need Administrator or root rights, and you do not copy any checksums by hand.

`axm install` installs the complete pinned release set. It does not accept component names such as `cli` or `axiom-graphd`. `axm update` updates the installed release set; it also does not accept a component name.

Only the four commands above are installed on PATH. The installation engine is private. The installed skills payload still needs [plugin registration in your AI host](https://github.com/orchex006/axiom-skills/blob/main/marketplace-plugins/README.md).

- [Before you start](#before-you-start)
- [Install](#install)
- [Check that it worked](#check-that-it-worked)
- [Install options](#install-options)
- [Where files go](#where-files-go)
- [Update](#update)
- [Uninstall](#uninstall)
- [Upgrading from 0.1.0 or 0.1.2](#upgrading-from-010-or-012)
- [Troubleshooting](#troubleshooting)

## Before you start

| Platform | You need | Status |
|---|---|---|
| Windows 10/11 x64 | Windows PowerShell 5.1 (built in) | Supported and tested |
| Linux x64, including WSL2 | `curl` or `wget`, `sha256sum` or `shasum`, `tar`, `python3` | Supported and tested |
| macOS Intel (x64) | Same as Linux | Published; native installation checks run in CI |
| macOS Apple silicon, Linux arm64 | — | Not available yet |

- **Run as your normal user.** The installer refuses to run as Administrator (Windows) or root (`sudo`).
- **Upgrading?** If you installed 0.1.0 or 0.1.2 before, read [Upgrading from 0.1.0 or 0.1.2](#upgrading-from-010-or-012) first.

## Install

### Windows (PowerShell 5.1+, no Administrator)

Open a normal PowerShell window (not "Run as administrator") and run:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/install.ps1 | iex"
```

To update later, run the [update command](#update):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/update.ps1 | iex"
```

### macOS, Linux and WSL2 (not as root)

Open a terminal and run:

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/install.sh | sh
```

To update later, run the [update command](#update):

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/update.sh | sh
```

### What happens

1. The script downloads the release archive for your system from GitHub.
2. It checks the archive's SHA-256 before unpacking. If the check fails, it stops, and nothing on your machine has changed.
3. `axm install` shows a plan and asks once:

   ```text
   channel: stable
     axiom-graphd 0.1.3 - artifact declared
     axiom-mcp 0.1.3 - artifact declared
     skills 0.1.3 - ...
     mcp runtime - provisioned from the release
   install root: C:\Users\you\AppData\Local\Axiom (bin: C:\Users\you\AppData\Local\Axiom\bin)
   size: 4.4 MB of verified artifacts
   PATH: add C:\Users\you\AppData\Local\Axiom\bin to the user PATH (undo: axm uninstall; skip: --no-modify-path)
   Proceed? [Y/n]
   ```

4. Press Enter (or type `Y`). Axiom installs and adds its `bin` folder to **your** PATH.

**Open a new terminal** afterwards, because the terminal you installed from does not see the new PATH yet.

## Check that it worked

In a new terminal:

```sh
axm version
```

The first line shows the installed version, for example `installed: 0.1.3 channel=stable ...`.

```sh
axm doctor
```

The exit code is `0` when the reported installation checks pass. Project registration, graph freshness, MCP permissions and host plugin discovery require separate setup. SQLite is checked through the graph engine's actual connection/query probe; its version is informational. A missing or unusable driver reports a failure and a repair command. Prebuilt installations need no Rust compiler or separate system SQLite installation.

## Install options

| Environment variable (any system) | Linux / macOS option | Effect |
|---|---|---|
| `AXIOM_INSTALL_YES=1` | `--yes` | Install without asking (for scripts and CI) |
| `AXIOM_NO_MODIFY_PATH=1` | `--no-modify-path` | Do not change your PATH |
| `AXIOM_VERSION=0.1.3` | `--version 0.1.3` | Install a specific version |

The Windows one-line command cannot take options, so use the environment variables there:

```powershell
# Windows: set the environment variable, then run the normal command
$env:AXIOM_INSTALL_YES = '1'
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/install.ps1 | iex"
```

```sh
# Linux / macOS: put the options after `sh -s --`
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/install.sh | sh -s -- --yes
```

To pin a version in the URL itself, replace `latest/download` with `download/v0.1.3`.

If you run without a terminal (for example in CI) and do not pass `--yes`, the installer prints the plan, changes nothing and exits with code `4`.

## Where files go

| System | Install folder |
|---|---|
| Windows | `%LOCALAPPDATA%\Axiom` |
| Linux / macOS | `$XDG_DATA_HOME/axiom`, or `~/.local/share/axiom` if that variable is not set |

Inside the install folder:

| Path | Contents |
|---|---|
| `bin/` | `axm`, `axsk`, `axge`, `axmp`; private helpers are under `bin/graphd-engine/` |
| `mcp-runtime/` | The Python runtime for the MCP server |
| `generations/` | The installed release, kept so an update can roll back |
| `installed.json` | What is installed (read by `version`, `doctor`, `update`, `uninstall`) |
| `path-change.json` | The exact PATH change, so uninstall can undo it |

The PATH change applies to your user only:

- **Windows:** the user `Path` value under `HKCU\Environment`. The system PATH is never changed.
- **Linux / macOS:** one marked `export PATH=...` line in `~/.zprofile` (zsh), `~/.bash_profile` (bash, if it exists), or `~/.profile`.

## Update

Use the update command for your system (published from 0.1.4):

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/update.ps1 | iex"
```

```sh
# Linux, WSL2 or macOS
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/update.sh | sh
```

The update script decides what to do from what is on your machine. It changes nothing by itself:

| Found | What the update script does |
|---|---|
| Axiom 0.1.3 or newer | Runs that installation's own `axm update`. |
| Axiom 0.1.0 or 0.1.2 | Downloads the release, checks its SHA-256 and runs the installer, which upgrades the old installation ([details](#upgrading-from-010-or-012)). |
| Nothing | Stops and prints the install command. It never installs from scratch. |
| Old `AXIOM_CLI_INSTALL_ROOT`, `AXIOM_ENGINE_BIN` or `AXIOM_HOME` variables | Stops, names them and shows how to remove them. |

If Axiom is on your PATH, you can also run it directly. The same command works on every system:

```sh
axm update
```

If a newer release exists, `update` shows the plan and asks once. It then downloads and verifies the release and keeps the previous version so it can roll back. If nothing is newer, it prints `up to date`.

- `axm update --dry-run`: show the plan only.
- `axm update --yes`: update without asking. For the update script, set `AXIOM_INSTALL_YES=1`, or use `sh -s -- --yes` on Linux / macOS.
- `axm update rollback --transaction previous`: go back to the version before the last update.

Use an update command, not the install command, to move an existing installation to a newer release.

## Uninstall

```sh
axm uninstall
```

`uninstall` shows the plan and asks once. It removes the Axiom programs, the background service registration and the PATH entry it added. **Your data, workspaces and graph output are kept.**

On Windows, the running program (`axm.exe`) cannot delete itself, so it is renamed to `<name>.exe.uninstalled`. You can delete that file afterwards.

## Upgrading from 0.1.0 or 0.1.2

The 0.1.2 instructions used two scripts and asked you to set environment variables. Those variables now get in the way, so remove them first.

**1. Remove the old environment variables** (Windows, in PowerShell):

```powershell
foreach ($name in 'AXIOM_CLI_INSTALL_ROOT', 'AXIOM_ENGINE_BIN', 'AXIOM_HOME') {
    [Environment]::SetEnvironmentVariable($name, $null, 'User')
}
```

On Linux or macOS, delete any `export AXIOM_CLI_INSTALL_ROOT=...`, `AXIOM_ENGINE_BIN` or `AXIOM_HOME` lines from your shell profile. Then **open a new terminal**.

Why: `AXIOM_CLI_INSTALL_ROOT` changes where Axiom installs, and `AXIOM_ENGINE_BIN` makes `axm` keep running the old 0.1.2 engine.

**2. Run the [update](#update) command** (from 0.1.4) **or the [install](#install) command.** Both reach the same installer, which says what it found and what it will do:

| Found | What the installer does |
|---|---|
| An old `axiom-cli` (0.1.0 or 0.1.2) in the default install folder, `%LOCALAPPDATA%\Axiom` | Checks recorded ownership before migration. An executable without a verified ownership record is preserved and installation stops with its path; move that file aside before retrying. |
| A 0.1.2 bootstrap folder elsewhere (for example `%USERPROFILE%\axiom`) | Leaves it untouched and installs to the default folder next to it. |

Most people should keep this default. If you would rather turn the 0.1.2 bootstrap folder into your install, do this instead of the one-line command:

1. Download `axiom-<version>-windows-x64.zip` and `SHA256SUMS` from the [latest release](https://github.com/orchex006/axiom-cli/releases/latest).
2. Check the archive: the output of `Get-FileHash <zip> -Algorithm SHA256` must match the zip's line in `SHA256SUMS`.
3. Extract the zip to a new folder. In that folder, run:

   ```powershell
   .\axm.exe install --adopt "<path to the 0.1.2 bootstrap folder>"
   ```

**3. Run `axm doctor`.** It lists any old layouts, any older `axiom-cli` that is still earlier on your PATH, and any leftover variables. `doctor` only reports and never changes anything.

## Troubleshooting

| What you see | Cause | Fix |
|---|---|---|
| `axm` is not recognized after installing | The terminal was opened before the PATH changed, or the installation is older than 0.1.5 (no `axm` yet) | Open a new terminal. If you used `--no-modify-path`, run `<install folder>/bin/axm` directly. On an older installation, run the [update command](#update) once. |
| `axm` shows old help text, or answers `NotReady` to every command | An older `axm` comes first on your PATH | Find it with `Get-Command axm -All` (Windows) or `which -a axm`. `axm doctor` also names it. Remove the old folder from your PATH. |
| `axm version` says `installed: no` | A leftover `AXIOM_CLI_INSTALL_ROOT` points to another folder | Remove the variable ([Upgrading](#upgrading-from-010-or-012), step 1) and open a new terminal. |
| `refusing to run elevated` or `refusing to run as root` | The terminal runs as Administrator, or you used `sudo` | Run it again from a normal-user terminal. |
| `axm engine update: leftover variables from the 0.1.2 procedure are set` | `AXIOM_CLI_INSTALL_ROOT`, `AXIOM_ENGINE_BIN` or `AXIOM_HOME` is still set | Remove them ([Upgrading](#upgrading-from-010-or-012), step 1), open a new terminal and run the update command again. |
| `axm engine update: Axiom is not installed for this user, so there is nothing to update` | Nothing is installed (or it was uninstalled) | Use the [install](#install) command. |
| `download failed ...` or `SHA-256 mismatch ...; nothing was changed` | Network or proxy problem, or a damaged download | Try again. Nothing was installed. |
| `a host python3 is required` (Linux / macOS) | `python3` is not installed | Install it with your package manager, for example `sudo apt install python3`, then run the install command again as your normal user. |
| `unsupported platform` or `unsupported architecture` | arm64 is not published yet | See [Before you start](#before-you-start). |
| The installer exits with code `4` without asking | No terminal is attached (CI or a pipe) | Set `AXIOM_INSTALL_YES=1`, or add `--yes` on Linux / macOS. |
| Exit code `5` | You answered `n` | Run it again and answer `Y`. |
| `the install root ... holds entries no Axiom install writes` (exit code `6`) | The install folder contains files that Axiom did not create | Remove a leftover `AXIOM_CLI_INSTALL_ROOT`, or move those files out. |

If repair from the installed CLI reports a missing skills bundle, rerun the [installation URL](#install) to supply verified release content. An `up to date` message means no upgrade was applied; it does not establish that every advanced update path is ready.

For the meaning of every exit code, run `axm --help`. For details on how installation, updates and verification work, see [Distribution and installers](30-DISTRIBUTION-AND-INSTALLERS.md).

## Repair and inspect an installation

`axm install` installs the complete pinned ecosystem or repairs that set. It includes the CLI, graph engine, MCP runtime and skills bundle; it does not take a component name such as `cli` or `axiom-graphd`. Review the plan and answer `Y`, or use `axm install --yes` to approve it directly.

An existing installation can supply its verified active skills bundle for repair even after the downloaded archive has been removed. If that bundle is missing or corrupt, reinstall from the release installer to obtain verified payloads.

`axm version` summarizes the running CLI and installed graph engine, MCP and skills. Use `axge` for graph work, `axsk` for skills and `axmp` for MCP. To inspect the live update offer without applying it, run `axm update --dry-run`. The advanced `axm update check` examines recorded channel metadata; it does not fetch the latest public offer.

`axm doctor` prints a short summary and actionable findings. Use `axm doctor --verbose` for paths, transaction details and prerequisite evidence, or `--json` for automation. Project graph registration and host plugin discovery remain separate checks.

The current command layout migrates recorded, unchanged legacy entrypoints into private storage. If an old executable was edited or is unowned, installation stops and names the file; move it aside and retry. Verified previous generation payloads remain available for rollback. Existing published releases retain their original layout until you install a release containing this change.

Version 0.1.8 has a migration defect: an unchanged 0.1.0 or 0.1.2 CLI store can fail with `axiom-cli.exe is unowned, linked or modified` because ownership is recorded in the older `install-manifest.json`. A matching SHA-256 in that manifest identifies this case. The pending fix validates the recorded executable name, location, size and SHA-256, preserves `cli/` and the manifest, and keeps a verified backup before retiring the old command. Modified files, linked ownership inputs and conflicting backups still stop adoption. This fix requires a newer release; retrying the published 0.1.8 installer uses the same code.

For an installation made by an older command layout, the update URL verifies and runs the current release CLI before starting the update transaction. This avoids the previous updater's obsolete executable health checks. Current installations use their own `axm update`; both paths retain the normal approval and rollback behavior.
