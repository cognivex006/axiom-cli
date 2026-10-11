# Axiom CLI

`axm` (short for `axiom-cli`) installs, checks, updates and removes the Axiom ecosystem on your machine:

- `axm` / `axiom-cli` itself (two names for the same program).
- The graph engine, `axiom` and `axiom-graphd`.
- The MCP server and the skills bundle.

**Start here:** [Install and update](#install-and-update) · [Commands](#commands) · [Use Axiom after installation](#use-axiom-after-installation) · [Troubleshooting](docs/INSTALL.md#troubleshooting)

## Explore the ecosystem

Axiom builds a graph of your code and makes it available to your existing AI tools. Choose the repository that owns the part you want to use:

| Component | What it provides | Where to go |
|---|---|---|
| **[axiom-cli](https://github.com/orchex006/axiom-cli)** — this repository | `axm` / `axiom-cli`, ecosystem installation, updates and removal | [Installation guide](docs/INSTALL.md) · [Releases](https://github.com/orchex006/axiom-cli/releases) |
| **[axiom-graphd](https://github.com/orchex006/axiom-graphd)** | Rust graph engine: source analysis, file watching, indexing and graph publication; `axiom-graphd` and the `axiom` operations CLI | [Command reference](https://github.com/orchex006/axiom-graphd/blob/main/docs/reference/axiom-graphd.md) · [Engine documentation](https://github.com/orchex006/axiom-graphd/blob/main/docs/README.md) |
| **[axiom-mcp](https://github.com/orchex006/axiom-mcp)** | MCP gateway for querying the published graph from an AI host, through stdio or HTTP | [Run and configure MCP](https://github.com/orchex006/axiom-mcp#run) · [Tools reference](https://github.com/orchex006/axiom-mcp/blob/main/docs/reference/mcp.md) |
| **[axiom-skills](https://github.com/orchex006/axiom-skills)** | Agent skills, workflow policy, host adapters and bootstrap templates | [Plugin installation](https://github.com/orchex006/axiom-skills/blob/main/marketplace-plugins/README.md) · [Host setup](https://github.com/orchex006/axiom-skills/blob/main/docs/guides/hosts.md) |
| **[axiom-specs](https://github.com/orchex006/axiom-specs)** | Shared architecture, contracts, compatibility decisions and development tasks | [Start here](https://github.com/orchex006/axiom-specs/blob/main/START-HERE.md) · [Task index](https://github.com/orchex006/axiom-specs/blob/main/tasks/INDEX.md) — repository access required |

The CLI distributes the components together. The engine produces graph data, MCP exposes that data to an AI host, and skills describe how an agent should use it. Component guides own their setup instructions and compatibility details.

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

Run `axm --help` for every option and exit code. `axm` is installed from 0.1.5; on 0.1.4 or older use `axiom-cli` (same verbs), or run the update script once to get `axm`.

## Use Axiom after installation

Installing the programs and passing `axm doctor` checks the installation. Project registration, graph generation and AI-host configuration are separate setup steps.

| I want to… | Start with |
|---|---|
| Register a solution, inspect its state, or query graph context and impact | [Graph engine command reference](https://github.com/orchex006/axiom-graphd/blob/main/docs/reference/axiom-graphd.md) and [operations guide](https://github.com/orchex006/axiom-graphd/blob/main/docs/22-OPERATIONS-AND-TROUBLESHOOTING.md) |
| Connect an AI host to the graph through MCP | [MCP startup and credential configuration](https://github.com/orchex006/axiom-mcp#run), then [stdio transport](https://github.com/orchex006/axiom-mcp/blob/main/docs/stdio-transport.md) or [HTTP transport](https://github.com/orchex006/axiom-mcp/blob/main/docs/http-transport.md) |
| Add Axiom skills to Codex, Claude Code, Gemini CLI or Antigravity | [Skills/plugin installation](https://github.com/orchex006/axiom-skills/blob/main/marketplace-plugins/README.md), then [host compatibility and setup](https://github.com/orchex006/axiom-skills/blob/main/docs/guides/hosts.md) |
| Understand the graph-assisted agent workflow | [Agent workflow guide](https://github.com/orchex006/axiom-skills/blob/main/docs/guides/agent-workflow.md) |
| Diagnose installation or update problems | [CLI troubleshooting](docs/INSTALL.md#troubleshooting); include the command, output and `axm version` when reporting a problem |

`axm` manages the installed ecosystem. Graph commands belong to `axiom-graphd`; project bootstrap and host operations belong to `axiom`. Installing the skills plugin does not start an MCP server or install the graph runtime. Follow the linked host guide for the capabilities verified for your host and version.

## For maintainers

- [Distribution and installers](docs/30-DISTRIBUTION-AND-INSTALLERS.md): how install, PATH, bootstrappers and legacy adoption work, and their test harnesses.
- [CLI argv surface](docs/40-CLI-ARGV-SURFACE.md) and [update channel](docs/50-UPDATE-CHANNEL.md).
- [Container channel](docs/50-CONTAINER-CHANNEL.md), which has its own publication status.
- [Development contract](Development.md) and [Changelog](Changelog.md).

The installation engine is owned by `axiom-graphd`; this repository packages and verifies releases and never forks that engine. The normative distribution contract lives in `axiom-specs`.
