# Axiom CLI

Install and update Axiom, connect your agent, then build and query your code graph.
`axm` manages the runtime and host setup; `axge` runs graph operations; `axmp`
serves the local API/MCP gateway. Agent workflows use `$axsk`.

## Install and update

### Windows (PowerShell 5.1+, no Administrator)

Install:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/install.ps1 | iex"
```

Update:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/update.ps1 | iex"
```

### macOS, Linux and WSL2 (not as root)

Install:

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/install.sh | sh
```

Update:

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/update.sh | sh
```

The install script checks the download's SHA-256, shows what it will install and asks once. At `Proceed? [Y/n]`, press Enter or type `y` to continue, or `n` to cancel. Wait for your shell prompt to return before entering another command. After installing, open a new terminal and check the result with `axm version` and `axm doctor`.

The update script finds your installation and runs its `axm update`, which shows the plan, asks once and keeps the previous version for rollback; it prints `up to date` when nothing is newer. If Axiom is on your PATH, `axm update` does the same. The update script also upgrades a 0.1.0/0.1.2 installation, but first remove the old `AXIOM_*` variables ([Upgrading from 0.1.0 or 0.1.2](docs/INSTALL.md#upgrading-from-010-or-012)). The update script is published from 0.1.4.

**Full guide:** [docs/INSTALL.md](docs/INSTALL.md) covers requirements, options, where files go, update, uninstall, upgrading from 0.1.0/0.1.2 and troubleshooting.

## Connect your agent

Version 0.1.10 adds managed host commands and the unified `$axsk` skill.
For older installations, update the runtime first; immutable 0.1.9 retains its
[manual plugin setup](https://github.com/orchex006/axiom-skills/blob/v0.1.9/marketplace-plugins/README.md).

```powershell
axm skills install codex
axm skills doctor codex
```

The manager uses the verified installed bundle and registers its local plugin
with Codex. Open a new agent session after registration. To include host setup
in the runtime installation plan, use `axm install --host codex`. Previously
managed registrations are included when installing/repairing a newer runtime.
The first managed adapter supports Codex; other hosts retain their documented
manual setup until a verified adapter is available.

A disabled managed plugin stays disabled during update; the manager never silently enables it.

Update only the host plugin with `axm skills update codex`; inspect managed
registration with `axm skills list`; remove it with `axm skills uninstall codex`.
Plans ask for confirmation; automation may use `--yes`. A host failure after
runtime installation reports a partial result and preserves the runtime result.
For a failed host command use `axm skills recover codex`; retained snapshots
support recovery without fetching a moving Git branch.

## Verify and try a workflow

```powershell
axm version
axm doctor
axm skills doctor codex
```

Runtime health, host registration, actual skill discovery and an authenticated
MCP query are separate results. A successful registration does not claim the
agent has loaded a new session or can query your projects yet.

In the agent, select the discovered skill (host namespaces may qualify it):

```text
$axsk build
$axsk watch
$axsk ask Find callers across the registered projects A, B and C.
```

Use the [skills guide](https://github.com/orchex006/axiom-skills/blob/main/docs/GETTING-STARTED.md)
for project registration and MCP credentials, and the
[local gateway guide](https://github.com/orchex006/axiom-mcp/blob/main/docs/local-gateway.md)
for service lifetime and multi-project retention.

## Commands

| Command | What it does |
|---|---|
| `axm --help` | Show available commands and options |
| `axm install --help` | Explain installation options |
| `axm install` | Install or repair Axiom (shows the plan, asks once) |
| `axm version` | Show the installed and available versions |
| `axm doctor` | Check the installation; changes nothing |
| `axm update` | Update to the newest release (asks once; keeps a rollback) |
| `axm uninstall` | Remove the programs and PATH entry; keeps your data |

`install` and `update` operate on the complete pinned release set. They do not accept a component name (`cli`, `axiom-graphd`, `mcp` or `skills`). Agent setup is managed separately with `axm skills`; see the setup section above.

Canonical terminal commands are `axm`, `axge` and `axmp`. Fresh installs create only the three canonical commands. An existing owned terminal `axsk` is updated as a deprecated forwarder during migration. In your agent, `$axsk` invokes workflow instructions. Internal installation helpers stay outside PATH.

Run `axm --help` for every option and exit code. For an older installation, run the update script to migrate to the current command layout.

## Explore the ecosystem

Axiom builds a graph of your code and makes it available to your existing AI tools. Choose the repository that owns the part you want to use:

| Component | What it provides | Where to go |
|---|---|---|
| **[axiom-cli](https://github.com/orchex006/axiom-cli)** — this repository | `axm`, ecosystem installation, updates and removal | [Installation guide](docs/INSTALL.md) · [Releases](https://github.com/orchex006/axiom-cli/releases) |
| **[axiom-graphd](https://github.com/orchex006/axiom-graphd)** | `axge`: source analysis, file watching, indexing and graph publication | [Command reference](https://github.com/orchex006/axiom-graphd/blob/main/docs/reference/axiom-graphd.md) · [Engine documentation](https://github.com/orchex006/axiom-graphd/blob/main/docs/README.md) |
| **[axiom-mcp](https://github.com/orchex006/axiom-mcp)** | MCP gateway for querying the published graph from an AI host, through stdio or HTTP | [Run and configure MCP](https://github.com/orchex006/axiom-mcp#run) · [Tools reference](https://github.com/orchex006/axiom-mcp/blob/main/docs/reference/mcp.md) |
| **[axiom-skills](https://github.com/orchex006/axiom-skills)** | Agent skills, workflow policy, host adapters and bootstrap templates | [Plugin installation](https://github.com/orchex006/axiom-skills/blob/main/marketplace-plugins/README.md) · [Host setup](https://github.com/orchex006/axiom-skills/blob/main/docs/guides/hosts.md) |

The CLI distributes the components together. The engine produces graph data, MCP exposes that data to an AI host, and skills describe how an agent should use it. Component guides own their setup instructions and compatibility details.

## For maintainers

- [Distribution and installers](docs/30-DISTRIBUTION-AND-INSTALLERS.md): how install, PATH, bootstrappers and legacy adoption work, and their test harnesses.
- [CLI argv surface](docs/40-CLI-ARGV-SURFACE.md) and [update channel](docs/50-UPDATE-CHANNEL.md).
- [Container channel](docs/50-CONTAINER-CHANNEL.md), which has its own publication status.
- [Development contract](Development.md) and [Changelog](Changelog.md).

The installation engine is owned by `axiom-graphd`; this repository packages and verifies releases and never forks that engine. The normative distribution contract lives in `axiom-specs`.
