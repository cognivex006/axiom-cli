# Update channel - axiom-cli

This guide is owned and released with `axiom-cli` under task **J-007**
(`axiom-cli-distribution`, requirements R16, R22, R06). It describes the update path that
`axiom-cli` actually implements today, and it distinguishes what has been executed on a real
host from what has not. The canonical rules stay in `axiom-specs`:
`docs/20-VERSION-CHECK-UPDATE-RELEASE.md`, `contracts/axiom-cli-distribution-contract.md` and
`contracts/schemas/update-plan.schema.json` with `tools/update_plan_contract.py`. Do not fork them
here.

## Path decision (recorded)

The task card proposed `src/update/` and `channels/stable.json`; both were created exactly there.
No path was widened or renamed, and nothing was added outside `axiom-cli`.

```text
src/update/mod.rs        the `update` verb surface and the subcommand vocabulary
src/update/apply.rs      check / plan / apply / rollback and the whole transaction
src/update/channel.rs    the channel manifest: parse, validate, digest, reject placeholders
src/update/plan.rs       plan construction, canonical digest and the mirrored contract checks
src/update/generation.rs one immutable generation: its record and its payload
src/update/journal.rs    the transaction journal and crash recovery
src/update/fetch.rs      artifact acquisition from the local, isolated cache
src/update/sha256.rs     the in-repo SHA-256 used for every digest
src/update/health.rs     the post-swap health probe
src/update/state.rs      the install root, its record and its lock
src/update/rules.rs      shared rules: digests, SemVer, relative paths, forbidden pins
src/update/json.rs       the single-JSON-object envelope writer and a canonical JSON writer
src/update/report.rs     report and exit-code vocabulary
src/update/error.rs      the refusal classes that map to the canonical exit codes
src/update/time.rs       UTC timestamps
channels/stable.json     the channel manifest this repository publishes
tests/update_channel.rs  the acceptance and boundary suite (23 legs)
```

## One-command update (ADR-0033, L-005)

```text
axiom-cli update [--yes] [--dry-run] [--channel <url|path>]
```

A bare `update` is the human path for an installation made by the one-line installer (or by
`axiom-cli install`, which records `channel_source` in `installed.json`):

1. read the installed core version (`axiom-graphd`; MCP and skills version independently) and the
   recorded channel source (default
   `https://github.com/orchex006/axiom-cli/releases/latest/download/channel.json`);
2. fetch the channel manifest; its `release` block must pin `version`, `tag` = `v<version>` and,
   per platform, an exact archive `url` under that tag with `sha256` and `size_bytes`. A tag alias
   (`latest`), a branch tip, `*` or a `releases/latest` archive URL is refused (`2`);
3. answer `up to date` (`0`) when the offer is not newer; otherwise build a plan that pins the
   version, archive URL, SHA-256 and the manifest digest, print it and ask once (`--yes`
   pre-approves; no terminal and no `--yes`: exit `4`, nothing changed; declined: `5`);
4. re-fetch the manifest and refuse if its digest changed after planning (`6`), download the
   archive with the OS `curl`, verify length and SHA-256 before use (`2` on mismatch), extract it
   with the OS `tar` into `staging/update`;
5. hand the extracted release to its own `axiom-cli install` in engine-update mode, so the graphd
   engine runs `update plan` / `update apply` (keeping its rollback journal), the MCP runtime of the
   new release is provisioned, and the new `bin` and `installed.json` are recorded with the old
   generation as `previous_generation`;
6. health-check the new `bin/axiom-cli version` and `bin/axiom version`; on failure roll back the
   engine transaction, the `bin` files (from the previous generation's payload), the MCP runtime
   pointer and `installed.json` byte for byte, and exit `8`.

`axiom-cli update rollback --transaction previous` reverses the last one-command update the same
way. User data, workspaces and the graph output root are never written. The real-host harness is
`tests/l005_one_command_update.py`; `--channel` accepts a local path or `http://127.0.0.1` for
tests and `https://` otherwise.

## The four verbs

For an installation with an active `installs/ecosystem/current` engine pointer
and no explicit local kit, distribution update verbs refuse with
`composite_activation_not_ready` (exit 4). The J-007 distribution generation
store and the installed engine ecosystem have separate activation pointers;
moving either one alone could expose an incompatible CLI/core/MCP set. K-003
introduced this guard. K-406 adds an explicit unsigned container candidate
path: when `AXIOM_CLI_COMPOSITE_KIT` names a manifest-checked local kit, the
public verbs call the coordinator that binds both activation steps to one
approval and delegates ecosystem update/rollback to `axiom`. The guard stays
in force when no kit is named. The existing J-007 fixture store remains
testable where no engine ecosystem is installed.

```text
axiom-cli update check   [--json] [--all]
axiom-cli update plan    --to <semver> [--out <file>] [--json]
axiom-cli update apply   --plan <file> --approve-digest <sha256> [--json]
axiom-cli update rollback --transaction previous|<id> [--json]
```

`check` and `plan` are read-only. `apply` is the only mutating verb. `rollback` moves the active
pointer and never downloads.

Exit codes follow the frozen vocabulary of `docs/40-CLI-ARGV-SURFACE.md`: `0` success, `2`
validation refusal, `3` not found, `4` not ready, `6` conflict, `8` I/O or internal, `9`
incompatible, `10` lock unavailable. `--json` writes exactly one JSON object on stdout and keeps
diagnostics on stderr; the short machine token is `details.reason_code` and the full stated reason
is `details.reason`.

## Where versions come from

On the update path, versions are resolved **only** from the channel manifest that the installed
release recorded. (Installation and `version` can additionally read a manifest named by `--from` or
`AXIOM_CLI_CHANNEL_MANIFEST`; see `docs/40-CLI-ARGV-SURFACE.md`.) The update path never resolves a
branch tip, a tag alias, a network `latest`, or an unsigned channel:

- a version equal to `main`, `master`, `develop`, `latest`, `HEAD` or `*` is refused with
  `forbidden_pin:<value>`;
- a manifest without a `trust_root`, or without a `signature_artifact`, is refused with
  `unsigned_channel`;
- a manifest whose `metadata_expiry` has passed is refused with `channel_metadata_expired` as
  incompatible (`9`), and the same check runs again at `apply` time so a window that closes
  between approval and application cannot be exploited;
- the recorded manifest bytes are re-hashed and compared with `installed.json`; a mismatch is
  `recorded_manifest_digest_mismatch`.

## Verification, swap and rollback

For every artifact the plan names, the cache copy is re-hashed and its length is compared with the
manifest **before** the bytes are used. A mismatch is `artifact_digest_mismatch` and nothing is
swapped. An artifact that cannot be obtained stays unverified and the verb answers not-ready (`4`,
`artifact_unreachable`) rather than reporting an applied update.

The swap keeps the previous generation until the new one is verified:

```text
recover an interrupted transaction
-> take the coordinator lock
-> read the recorded channel manifest of the installed release
-> refuse unless the plan agrees with it, component by component
-> stage, then verify length and sha256 of every artifact before it is used
-> write the journal, then rename the staging tree into generations/<id>
-> swap installed.json atomically, keeping the previous generation
-> health check the new generation
-> on failure: restore the previous generation and report a rollback
-> on success: finalise the journal, prune older generations, release the lock
```

A generation that fails its own health probe is **not** promoted to the retained rollback target,
because the retained target must be a generation that was verified. `rollback --transaction
previous` therefore answers `no_previous_generation` (`6`) instead of re-activating bytes that just
failed. Every changed component is reported per component under `needs_restart`.
## Install root layout

The update layer owns one install root and nothing else. It never writes into the workspace, the
graph output root or portable state:

```text
<install-root>/
  installed.json              the release record: schema_version, channel,
                              channel_manifest_sha256, current_generation,
                              previous_generation, installed_at
  recorded-manifest.json      the exact channel manifest bytes the installed release recorded;
                              re-hashed and compared with installed.json on every verb
  update.lock                 the coordinator lock; a stale lock refuses with
                              lock_unavailable (10) instead of proceeding
  generations/<id>/           one immutable generation: generation.json + payload/**
  staging/<transaction>/      the transaction's staging tree, renamed into place on commit
  journal/<transaction>.json  the transaction journal; the next run recovers it
```

Generation ids look like `g-<utc-stamp>-<hex>` and transaction ids like `t-<utc-stamp>-<hex>`;
both are lowercase. A generation is only ever added whole. The active pointer in `installed.json`
is the single mutable value the swap replaces, and `previous_generation` is what rollback returns
to.

## The trust root is a development seed

`channels/stable.json` publishes no release and offers no artifact: `published` is `false` and
every component's `artifacts` is empty, because publication is closed for this wave. Nothing is
invented: a component whose owning repository does not declare a SemVer version is recorded as
`undeclared` with `version` and `revision` `null` (today `axiom-mcp`, whose `pyproject.toml`
declares the PEP 440 `0.0.0.dev0`, which is not SemVer).

`trust.trust_root` is the SHA-256 of this ASCII preimage, written here so it can be re-derived:

```text
axiom-cli dev channel trust root v1 (development seed; not a release signing root)
sha256 = 23e4845c8e76c7cf8642b2c47f759d401345424309aec6b3bf4ad8413b40ee7a
```

It is **not** a release signing root, and `trust.signature_artifact` is `null`, so the manifest is
an unsigned channel: `update plan` refuses it with `unsigned_channel` (2). Replacing the seed with
a real signed manifest, `published: true` and real artifact entries is a later, publication-open
task.
## Versions, digests and the local-only artifact cache

Three environment overrides exist so a fixture can be driven end to end without touching a real
install root. They are not update-only: `AXIOM_CLI_INSTALL_ROOT` and `AXIOM_CLI_CHANNEL_MANIFEST`
also govern `install`, `uninstall`, `doctor` and `version`, and `AXIOM_CLI_ARTIFACT_CACHE` is the
cache `install` verifies against. They are the documented way to run the distribution path by hand
(the full override table is in `docs/40-CLI-ARGV-SURFACE.md`):

```text
AXIOM_CLI_INSTALL_ROOT       the install root; otherwise the per-user data location
                             (Windows %LOCALAPPDATA%\Axiom, else $XDG_DATA_HOME/axiom,
                             else $HOME/.local/share/axiom)
AXIOM_CLI_CHANNEL_MANIFEST   a channel manifest to read while nothing is installed yet
AXIOM_CLI_ARTIFACT_CACHE     the local artifact cache used for offline acquisition
```

Acquisition is deliberately local-only, which is what makes the offline case testable honestly.
This layer never contacts a remote, opens a socket or follows a redirect:

- an artifact URL is keyed by its last path segment and resolved under
  `AXIOM_CLI_ARTIFACT_CACHE`, or from a `file://` location;
- an `https://` artifact that is not present in the cache answers `artifact_unreachable` (4);
- a copy whose length or sha256 differs from the manifest is `artifact_digest_mismatch` (2), and
  the bad copy is deleted rather than left where a later step could mistake it for a verified one;
- the active version is read only from the channel manifest the installed release recorded, so a
  branch tip, a tag alias and a network `latest` are structurally unreachable.

## Windows atomicity limits (stated, not assumed)

The swap is an atomic pointer change plus a directory rename inside one install root. That has two
honest limits on Windows:

- a rename cannot replace an executable that is currently running. The active pointer is still
  swapped atomically; taking effect needs the affected process to restart, which is why every
  changed component is reported under `needs_restart` rather than being reported live;
- this layer moves the pointer and the generations. The engine that owns a component's canonical
  executable path and its restart stays in `axiom-graphd` (I-003/I-004) and is not exercised here.

The path needs no administrator rights, no symlink privilege, no Bash and no WSL: it is a per-user
directory the user can write. Locking is a single file (`update.lock`), and the journal is what
makes an interrupted apply recoverable instead of half-applied.

The K-306 unsigned Windows composite candidate uses an explicit local kit and
the installed MCP Python. For its distribution entrypoint, the Windows
installer first renames the owned running image into its rollback area, then
moves the verified B image into the vacant entrypoint path. The coordinator
records the previous CLI state and uses the graphd-owned engine transaction
for ecosystem rollback. This candidate path is selected only when
`AXIOM_CLI_COMPOSITE_KIT` names an exact manifest-checked kit; it does not
publish or trust a remote release.

For the K-010 Windows 0.1.2 candidate, the kit verifies the exact K-309 core
manifest before it can update K-305 A. The new core can register a per-user
Windows service. When that service is installed after the update, rollback to
0.1.1 first requires `axiom service uninstall` from the installed B engine.
The coordinator refuses an automatic rollback while the service remains,
because A cannot run its newer Task Scheduler action. This refusal leaves the
installed B pointers, task and user data unchanged.
## What is verified, and what is not

Verified on this host: the whole `update check|plan|apply|rollback` machinery, against local
on-disk fixtures, on **windows-x64**, using the real release binary. Positive, negative and
boundary legs are recorded with their exit codes under `evidence/J-007/`.

This is the `J-007` recorded capture (`windows-x64`). The current development host is `macos-x64`;
no native run of the lifecycle verbs added later is recorded there, and no target is `certified`.

Explicitly **unverified**:

- `linux-x64`, `macos-arm64` and `macos-x64`: no native run exists. A WSL2 run is Linux evidence
  and is never Windows evidence, and no platform is certified by this document;
- real channel resolution: the trust root above is a development seed and the manifest is
  unsigned, so resolving against a published, signed channel stays unverified by construction.

## Reproducing the evidence

```text
cargo build --release --locked
cargo test --locked --all-targets            # 89 unit + 14 argv_surface + 56 cli_verbs + 9 container_distribution + 6 linux_distribution + 24 update_channel
```

The acceptance transcript was produced by a scratch harness under `_w10ev/`, which is never
committed and rebuilds and deletes its own fixtures under `_w10ev/fx/` on each run:

```text
pwsh -NoProfile -File '_w10ev/run-ac2.ps1'   # requires target\release\axiom-cli.exe
```

Deleting that directory does not affect the committed evidence, which is the verbatim captured
output under `evidence/J-007/`.
