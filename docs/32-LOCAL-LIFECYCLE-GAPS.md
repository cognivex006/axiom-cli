# Installation and readiness

`axm` manages the installed release. `axiom` manages graph projects and services; MCP connects graph queries to an AI host, and skills/plugins provide agent instructions.

## What the installation commands establish

- `install` verifies and places the release's pinned component set.
- `version` reports the installation record and available release information.
- `doctor` checks installation integrity, prerequisites, locks, interrupted transactions and engine discovery.
- `update` verifies a release offer and applies an approved transaction, retaining rollback state.
- `uninstall` removes owned programs and service entrypoints while retaining user data.

A successful `doctor` result is limited to the checks it ran. It does not establish that a project is registered, graph data is current, an MCP caller has permissions or an AI host has discovered its skills.

## After installation

Use the [graph engine guide](https://github.com/orchex006/axiom-graphd/blob/main/docs/README.md) to register and analyze a project, the [MCP guide](https://github.com/orchex006/axiom-mcp#run) to configure transport and permissions, and the [skills/plugin guide](https://github.com/orchex006/axiom-skills/blob/main/marketplace-plugins/README.md) to install into your host.

## Troubleshooting boundaries

A plan is not an applied transaction. If confirmation is declined, a digest is stale or required skills/runtime content is absent, the CLI should report the refusal rather than claim installation succeeded. On releases where installed repair cannot find its original skills bundle, rerun the verified installation URL to supply the release content.

Use `axm doctor --verbose` or `--json` when reporting a problem, and include `axm version`. Keep release archives, candidate update kits and project/host configuration distinct. Apple silicon, Linux arm64 and external host runtime behavior require their own verification; current x64 checks do not prove them.
