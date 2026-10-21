# Update and rollback

For the usual upgrade, run `axm update` or the [update script](INSTALL.md#update). This guide explains version selection, transaction approval and the advanced update interfaces.

## One-command update

```sh
axm update --dry-run
axm update
axm update --yes
axm update rollback --transaction previous
```

The installed release records its channel source. The default public source is `https://github.com/cognivex006/axiom-cli/releases/latest/download/channel.json`. That endpoint selects metadata; every offered archive must still pin an exact version tag, URL, SHA-256 and byte length. A floating archive URL, branch tip or wildcard is refused.

The command reads the installed core version and the offer. When the offer is not newer it reports `up to date`; that result does not prove an upgrade was performed. Otherwise it reports the version, download, size and plan digest and asks once. Without a terminal use `--yes`, or use `--dry-run` to inspect without mutation.

Before applying, it rechecks the manifest digest and refuses a changed offer. It downloads and verifies the archive before extraction, then asks the new release's installer and graph engine to update the ecosystem. The transaction retains the previous generation, MCP runtime pointer, entrypoints and installation record for rollback. A failed health check attempts restoration and reports failure; retained records are needed if recovery itself cannot complete.

## Advanced automation

The explicit update interfaces are:

```text
axm update check [--all]
axm update plan --to <version> [--out <file>]
axm update apply --plan <file> --approve-digest <sha256>
axm update rollback --transaction <id> [--approve-digest <sha256>]
```

These are distinct paths, not interchangeable spellings for the one-command network update. The older distribution-only coordinator consumes its recorded channel and local verified artifact cache. `check --all` may refuse an active engine ecosystem when no compatible composite kit is configured, even if the bare command reports the current version is up to date. Do not interpret that refusal as an applied update.

Local composite kits selected by `AXIOM_CLI_COMPOSITE_KIT` bind exact candidate inputs and delegate activation/rollback to the installed engine. They are local test/automation inputs rather than proof of a published update feed. Windows uses the installed runtime's native `Scripts` path and moves a running CLI image aside before replacement.

## Verification and approvals

An apply plan binds versions, artifact hashes, installed state and transaction intent. A missing, malformed, changed or stale approval is refused before activation. The coordinator lock prevents concurrent mutation. Journals describe interrupted operations; recovery accepts only state matching the owned records rather than overwriting unfamiliar files.

The generation swap and health probe form the activation boundary. Retained generation payloads support rollback, and modified user files are outside the rollback set. Component service activation stays owned by the graph engine.

## Source and trust

`channel.json` shipped next to release archives supplies pinned download metadata. Historical archives also contain local candidate manifests with `published: false`; those are offline payload descriptions, not signed public update channels. The checked-in `channels/stable.json` is a development seed and must not be presented as a production signing root.

Checksums establish byte integrity against the selected metadata. They do not establish code signing, notarization or certification. Source revisions and release asset checksums are available in the release's source/checksum records.

## Failure and recovery

| Result | Action |
|---|---|
| `up to date` | Compare the reported installed and offered versions; no upgrade was applied |
| Changed channel or stale approval | Regenerate and review the plan |
| Missing or modified artifact | Restore verified release content; do not bypass the hash check |
| Composite activation unavailable | Use a compatible engine-owned update path or report the limitation |
| Lock busy | Wait for the current operation; inspect interrupted state before retrying |
| Rollback target unavailable | Restore a verified release; a removed generation cannot be invented |

A same-version install/update smoke check does not prove upgrades between different releases. See [command reference](40-CLI-ARGV-SURFACE.md) for exit codes and [installation troubleshooting](INSTALL.md#troubleshooting) for legacy overrides.
