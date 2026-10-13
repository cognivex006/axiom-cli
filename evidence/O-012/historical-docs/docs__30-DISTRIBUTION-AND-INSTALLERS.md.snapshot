# Distribution and installers — axiom-cli

Owner: axiom-cli. **Maintainer guide.** To install Axiom, follow [INSTALL](INSTALL.md); this page explains how that works and how it is tested. The normative rules live in `axiom-specs/contracts/axiom-cli-distribution-contract.md`; this guide is the owner-side runbook and must not restate a shared schema or protocol.

## Scope

`axiom-cli` owns what a human or an agent obtains and runs: the distributed entrypoint, the native installer scripts, the transactional install/update/uninstall, the published container image and the update channel manifest. It does not own the installation engine, the analyzers, the store, the daemon lifecycle or the bootstrap engine — those stay in `axiom-graphd`, and this repository invokes them through their documented argv surface.

## Entrypoint and verbs

One executable, `axiom-cli` (`axiom-cli.exe` on Windows), is the single distribution entrypoint on every platform. It exposes `install`, `update`, `doctor`, `version` and `uninstall`, and delegates engine work to the installed `axiom-graphd` programs. The engine CLI `axiom` shipped in the core release stays owned by `axiom-graphd`; `axiom-cli` installs, verifies and updates it rather than duplicating any of its behaviour. All five verbs dispatch to real work, and a verb refuses with a typed exit code and reason instead of starting a partial operation when a required prerequisite, digest, pin or platform declaration is missing; `NotReady` (`4`) is one such refusal, not the universal answer. Invocation is program plus argv; no verb requires a shell string, Bash or elevation.

## Delivery platforms

| Platform | Artifact class | Executable |
|---|---|---|
| windows-x64 | archive containing `.exe` | `axiom-cli.exe` |
| linux-x64 | archive preserving the executable bit | `axiom-cli` |
| macos-x64 | archive preserving the executable bit | `axiom-cli` |
| macos-arm64 | archive preserving the executable bit | `axiom-cli` |
| container-linux-x64 | OCI image `ghcr.io/orchex006/axiom-cli` | `axiom-cli` |

The container image is a delivery channel for Linux-family automation. It is never native runtime evidence and never certifies a host OS target.

## Release tiers

Finish-first, release-blocking now: Windows x64, macOS x64 (Intel) and the GitHub Container Registry image.

Design-complete, verification later: Linux x64 including the WSL2 lane, and macOS arm64. These carry the same declared contract but stay explicitly unverified until a native run is recorded. A WSL2 run is Linux evidence and is never Windows evidence.

## Install

Installing is transactional and reversible. Verify release provenance and the per-artifact SHA256 before unpacking; unpack into a user-writable install directory; never require root, never bypass Gatekeeper and never mutate the machine PATH. The only hosted script is the versioned one-line bootstrapper below, which ADR-0033 permits because it embeds its release's exact tag and SHA-256 values and contains no installation logic. Then delegate to the engine to register the solution, bind logical repositories to native absolute paths and apply managed bootstrap with an approved plan digest. Re-running install must be idempotent.

The user-facing steps are in [INSTALL](INSTALL.md). The sections below describe each mechanism in the order a one-line install exercises it.

### First install through `axiom-cli` (ADR-0033, L-002)

Run from an extracted release, `axiom-cli install --dry-run` resolves the `channel.json` beside the executable (no `--from`, no environment variable), verifies every artifact and prints the plan; `--apply --approve-digest <plan digest>` then:

1. refuses a nonempty root that holds entries no Axiom install writes (`6`), and a missing engine (`3`), before changing anything;
2. provisions the MCP runtime with the release's own provisioner and pinned inputs (`Provision-McpRuntime.ps1` + `runtime-input.json` on Windows; `runtime/provision.py` + `runtime/manifest.json` with the host `python3` on POSIX), whose digests are bound into the plan;
3. assembles the engine bundle and delegates placement to `axiom` (`install plan --bundle` / `install apply`);
4. places `axiom-cli`, `axiom` and `axiom-graphd` side by side in `<root>/bin` (idempotent, digest-checked);
5. copies the verified artifacts into `generations/<id>/`, records the manifest bytes and writes `installed.json` last and atomically, with every component version and SHA-256 and the `bin` digests.

The root is `%LOCALAPPDATA%\Axiom` on Windows and `$XDG_DATA_HOME/axiom`, else `~/.local/share/axiom`, on POSIX. `version`, `doctor`, `update` and `uninstall` read that one `installed.json`; the engine is found as a sibling in `bin`. A crash before `installed.json` is written leaves an unfinished install that the next run repeats. The real-host harness is `tests/l002_first_install.py`.

### One confirmation and the user PATH (ADR-0033, L-003)

A bare `axiom-cli install` is the human path. It prints the plan (version, components, root, the PATH change and the size) and asks `Proceed? [Y/n]` once; the answer approves exactly that plan's canonical digest. `--yes` (or `AXIOM_INSTALL_YES=1`) approves without a prompt. Without a terminal (or with `--json`) and without `--yes` it prints the plan, changes nothing and exits `4`; a declined prompt exits `5`. `--dry-run` and `--apply --approve-digest` are unchanged for automation and never touch the PATH.

The interactive plan adds only `<root>/bin`, only to the user PATH, and says so before asking:

- Windows: the `Path` value of `HKCU\Environment`, keeping its registry type (`REG_EXPAND_SZ` stays expandable) and broadcasting the change to new terminals. The machine PATH is never opened.
- POSIX: one marked `export PATH=...` line appended to the login profile (`~/.zprofile` for zsh, `~/.bash_profile` when it exists for bash, else `~/.profile`).

`--no-modify-path` skips it. The exact change is recorded in `<root>/path-change.json`; `axiom-cli uninstall` removes exactly that entry (byte-for-byte restoration when nothing else edited the value), removes the owned `bin` files whose digests still match `installed.json`, and keeps user data. On Windows the running `bin\axiom-cli.exe` cannot delete itself, so it is moved aside as `axiom-cli.exe.uninstalled`. The real-host harness is `tests/l003_confirm_and_path.py`; it redirects the PATH target to `HKCU\Software\AxiomCliTest` (or a work `HOME`) so the operator's real PATH is never changed.

### One-line bootstrappers (ADR-0033, L-004)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/install.ps1 | iex"
```

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/install.sh | sh
```

`installers/oneline/install.ps1.in` and `install.sh.in` are templates; `packaging/oneline/generate_bootstrappers.py --tag vX.Y.Z --sums <SHA256SUMS> --out <dir>` fills them with that release's tag, archive names and SHA-256 values. The output is deterministic (LF, no timestamps), so the same inputs give byte-identical scripts. The `latest` URL only selects which immutable script runs; the scripts download only their own tag's archive (or, with a version pin, that tag's own script) and never resolve `latest`.

Each script refuses an elevated run (Administrator / root), an unsupported OS or architecture, a download failure, a SHA-256 mismatch (checked before extraction) and an archive that cannot be extracted, each before any change. It extracts into a temporary directory that is always removed, then runs `axiom-cli install` (which prints the plan and asks once). Options: `-Yes` / `--yes`, `-NoModifyPath` / `--no-modify-path`, `-Version` / `--version X.Y.Z`; under `irm | iex` use `AXIOM_INSTALL_YES=1`, `AXIOM_NO_MODIFY_PATH=1` or `AXIOM_VERSION`, or `& ([scriptblock]::Create((irm <url>))) -Yes`. `install.sh` reads the confirmation from `/dev/tty` because `curl | sh` occupies stdin. The Windows script uses only Windows PowerShell 5.1; failures `throw` rather than `exit` so `irm | iex` never closes the user's session. `axiom-cli install`/`uninstall` also refuse to run elevated themselves.

The real-host harness `tests/l004_bootstrappers.py` serves staged releases from a local HTTP server laid out like GitHub release downloads and runs the exact one-liner through Windows PowerShell; `tests/test_bootstrappers.py` covers the generator.

### Short command `axm` (ADR-0036, L-016)

`axm` is the short name of `axiom-cli`. `layout::with_short_command` adds `bin/axm` (`axm.exe`) as a second `BinSource` with the same verified bytes and SHA-256 as `axiom-cli`, so `place_bin`, the `installed.json` bin records, the generation payload, update, `update rollback` and uninstall handle it like every other owned executable. There is no symlink, `.cmd` shim or shell alias. A running `axm.exe` is moved aside on Windows exactly like `axiom-cli.exe`, and an `axm` running from `<root>/bin` resolves that root itself. The update scripts accept either name on PATH. Installations before 0.1.5 get `axm` from their next update or install.

### One-line update bootstrappers (ADR-0035, L-013)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/update.ps1 | iex"
```

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/update.sh | sh
```

`installers/oneline/update.ps1.in` and `update.sh.in` are generated by the same `generate_bootstrappers.py` run, with the same tag, archive names and SHA-256 values, and are published by `release/oneline_assets.py` next to the install scripts. They contain no update logic and do the following, in order:

1. Refuse an elevated run, and refuse while `AXIOM_CLI_INSTALL_ROOT`, `AXIOM_ENGINE_BIN` or `AXIOM_HOME` is set. On Windows the process and the user environment are both checked. The script names each variable and prints the removal command; it never removes one itself.
2. Look for an `installed.json` with `bin/axiom-cli` in the root of the `axiom-cli` that PATH resolves, then in the default root, then in `%USERPROFILE%\axiom` (`~/axiom`). If one is found, run that root's own `axiom-cli update` (passing `--yes` when given) and pass its exit code through.
3. Otherwise, for a 0.1.0/0.1.2 layout (an `install-manifest.json` or `bin/axiom-cli` in the default root, or `installs/ecosystem/current` under `~/axiom`), make exactly the install bootstrapper's verified hand-off to `axiom-cli install`.
4. Otherwise, change nothing and print the pinned install command.

There is no version option. `update.sh` reads the confirmation from `/dev/tty` like `install.sh`. Tests: `tests/test_bootstrappers.py` (generation, and `update.sh` against fixture homes) and the fresh-runner steps in `tests/l007_fresh_runner.py` (installed, leftover variable, nothing installed, 0.1.2 layout).

### Legacy installations (ADR-0033, L-006)

`axiom-cli doctor` names, without changing anything: a J-004 **CLI store** (`bin/axiom-cli`, `cli/`, `install-manifest.json`, 0.1.0 or 0.1.2), a 0.1.2 **bootstrap root** (`installs/ecosystem/current`, the nested `mcp-runtime/mcp-runtime` of the 0.1.2 Windows bootstrap, no `installed.json`), any Axiom-like directory it cannot classify (`unknown`, never modified), every other `axiom-cli` on PATH with its version and whether it shadows this install, and leftover `AXIOM_CLI_INSTALL_ROOT` / `AXIOM_ENGINE_BIN` / `AXIOM_HOME` variables. It inspects the target root, the platform default root, `%USERPROFILE%\axiom` (`~/axiom`) and any root those variables name.

`axiom-cli install` shows each in its plan:

- a CLI store at the target root is adopted in place: its `bin/axiom-cli` is backed up to `legacy/cli-store-<version>/`, `cli/` and `install-manifest.json` are kept, and a PATH entry the old installer added becomes owned by this install (uninstall removes it);
- a bootstrap root elsewhere is left untouched (side by side); `axiom-cli install --adopt <path>` adopts it in place instead: the engine reports `already-installed`, the nested MCP runtime is reused where it is (its venv holds absolute paths), user data and graph output are kept, and `bin` + `installed.json` are written there;
- an `axiom-cli` running from an installed `<root>/bin` resolves that root itself, so an adopted non-default root needs no environment variable.

The real-host harness `tests/l006_adopt_legacy.py` rebuilds the 2026-10-06 Windows state (fixture CLI store first on PATH, a real 0.1.2 bootstrap root made by the published `bootstrap_windows.ps1`, leftover variables).

## Update

One-command update (ADR-0033, L-005): `axiom-cli update` checks the recorded channel, shows the plan, asks once, applies through the engine's update transaction and keeps a rollback generation; see [50-UPDATE-CHANNEL](50-UPDATE-CHANNEL.md#one-command-update-adr-0033-l-005).

The older J-007 subcommands (`check`, `plan`, `apply`, `rollback`) live in `src/update/` with the channel manifest in `channels/stable.json`; [50-UPDATE-CHANNEL](50-UPDATE-CHANNEL.md) records what runs today and what does not. For those subcommands:

- `update check`, `update plan`, `update apply` and `update rollback` resolve a version only from the channel manifest the installed release recorded; a branch tip, a tag alias, a network `latest` and an unsigned channel are refused.
- Every artifact's length and sha256 are verified before it is used, and `axiom-cli` self-updates inside the same transaction under one approval digest.
- The swap keeps the previous generation until the new one passes its health probe; a failed verification or health check restores the previous generation, and an interrupted apply is recovered from the journal. Every changed component is reported under `needs_restart`.
- An update is never applied without an explicit approved request.

The engine implements local ecosystem activation, owned-service coordination and rollback (ADR-0014). The bare one-command `update` uses that engine transaction. The J-007 subcommands still use a separate delivery generation store; for an installation with an active engine pointer they refuse with `composite_activation_not_ready` unless an explicit local kit is supplied (see 50-UPDATE-CHANNEL).

## Uninstall

A bare `axiom-cli uninstall` prints what it removes and keeps, asks once (`--yes` skips the prompt), removes the engine install, the owned `bin` files and exactly the recorded PATH entry, and keeps user data (L-003).

Uninstall removes only owned executables and startup entries by default. Workspace data, checkpoints and private state require a separate, explicit deletion approval. Never recursively delete `.axiom`.

`axiom-cli uninstall --dry-run` embeds the canonical engine removal plan in its outer approval document. `--apply --approve-digest <digest>` invokes that exact engine plan, then clears only an unchanged distribution marker. The engine verifies ownership, stops its owned service, preserves edited/unowned files and writes retryable removal evidence. The wrapper refuses `--purge-data`; native Mac x64 evidence is recorded under `evidence/J-005/local-lifecycle-20260922/`.

## Dependencies

Installation dependencies and their version manifest are owned by the distribution contract. This seed deliberately invents no version: an entry whose manifest does not exist yet is recorded as undeclared pending the owning task, not guessed. Known prerequisite classes are the Rust toolchain for the core and the CLI, a pinned Python interpreter range for the gateway and the SQLite driver. Node.js, WSL, Docker, Bash and elevation are explicitly not required on a supported host.

## Evidence gate

Record, per target: the actual OS/architecture, the artifact digest, the installer and update transcript, the rollback result and the unverified remainder. Compiling for a target, publishing a container image or passing spec unit tests does not certify any platform. No platform is certified by this guide.

## Windows x64 per-user install (card J-004 — historical)

> **Historical maintainer runbook (0.1.0 era).** Users do not follow these steps; they use the one-line install in [INSTALL](INSTALL.md). `Install-AxiomCli.ps1` installs only the CLI store, which `axiom-cli install` now detects and adopts (L-006). Keep this section to reproduce the J-004 evidence.

This section is the owner-side runbook for the first native target that was implemented and actually
executed. Sources: `installers/windows/`, `packaging/windows/` and `tests/windows/` in this
repository. The card proposed exactly these paths; no path change was needed. Everything here runs in
the Windows PowerShell that ships with Windows — no WSL, Bash, Docker, Node.js or elevation.

### Files

| Path | Role |
|---|---|
| `packaging/windows/Build-ReleaseSet.ps1` | Assembles a `release-set.json` whose sha256 is the approval digest for the whole transaction. |
| `packaging/install-result.schema.json` | Draft 2020-12 schema for the machine-readable install-result envelope. |
| `installers/windows/AxiomCli.Windows.Common.ps1` | Shared helpers: canonical JSON, sha256, per-user PATH rules, transaction lock, host info. |
| `installers/windows/Install-AxiomCli.ps1` | Transactional per-user install, repair and recovery. |
| `installers/windows/Uninstall-AxiomCli.ps1` | Transactional removal that preserves user data unless a separate purge plan is approved. |
| `tests/windows/Invoke-AxiomCliWindowsDistributionTests.ps1` | The executed acceptance harness (32 legs) that produces `evidence/J-004/`. |

### Step 1 — assemble the release set and obtain the approval digest

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\Build-ReleaseSet.ps1 `
  -OutDir .\_out\release-set -CliExe .\target\release\axiom-cli.exe -Version 0.1.0 -Json
```

Writes `<_out>\release-set.json` plus the carried artifact bytes. The report is a release-set
assembly report, not an install-result envelope.

### Step 2 — plan (never mutates)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File installers\windows\Install-AxiomCli.ps1 `
  -ReleaseSet .\_out\release-set -Json
```

Exit `0`, `outcome = planned`, `mutated = false`, nothing created. The sha256 of `release-set.json` is
the approval digest.

### Step 3 — install with the approval digest

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File installers\windows\Install-AxiomCli.ps1 `
  -ReleaseSet .\_out\release-set -Apply -ApproveDigest <sha256-of-release-set.json> -Json
```

Exit `0` with `outcome = installed` and `elevation_required = false`.

### Step 4 — uninstall (preserves user data)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File installers\windows\Uninstall-AxiomCli.ps1 -InstallRoot <root> -Json
powershell -NoProfile -ExecutionPolicy Bypass -File installers\windows\Uninstall-AxiomCli.ps1 `
  -InstallRoot <root> -Apply -ApproveDigest <sha256-of-uninstall-plan> -Json
```

`-PurgeData` removes the install root as well. It is a different plan, so it is a different digest and
a different approval; a purge is never implied by a plain uninstall.

### Layout and scope

Default install root is `%LOCALAPPDATA%\Axiom` (per-user). Under it: `bin\axiom-cli.exe`, the
versioned `cli\generations\<version>\`, `cli\staging\` and `cli\rollback\` artifacts,
`cli\journal.json` (interrupted-transaction recovery), `cli\state.json`, `cli\transaction.lock` and
`install-manifest.json`. Only `HKCU\Environment\Path` (user scope) is changed; the machine-wide PATH is
snapshotted before and after every transaction and a change throws.

### Exit codes (PowerShell installer)

This table is the Windows PowerShell installer's exit mapping, not the `axiom-cli` binary's. `0`
success, `2` validation or missing approval, `3` not found, `5` approval mismatch, `6` conflict,
`8` I/O failure with rollback, `9` digest mismatch or refused downgrade, `10` transaction lock
unavailable. A mutating run that fails rolls back and reports `rolled_back`.

The CLI's own per-verb exit behaviour (including the engine handoff) is tabulated in
[docs/40-CLI-ARGV-SURFACE.md](40-CLI-ARGV-SURFACE.md).

### Reproduce the evidence

```powershell
powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File tests\windows\Invoke-AxiomCliWindowsDistributionTests.ps1
```

Records `evidence/J-004/`: `j004-run-transcript.txt`, `j004-step-records.json`,
`j004-artifact-digests.txt`, `j004-schema-validation.txt`, `envelopes\` (install-result envelopes,
validated against the schema) and `release-set-reports\`.

The harness is self-cleaning: the legs that deliberately leave an installation behind
(`L07b` forced unowned install, `L08b` allowed downgrade, `L09` interrupted-install recovery) are
uninstalled by explicit `L17a-c` cleanup legs, and `L17-cleanup-path-hygiene` asserts that no
`_j004-run` per-user PATH entry survives. Run it twice in a row: the second run proves the cleanup is
idempotent and a previously leaked PATH entry cannot be re-masked by the pre-run PATH guard.

### Verified vs unverified on Windows x64

Verified by the executed harness on Windows 11 Pro 10.0.26200 x64: per-user install, idempotent
re-run, interrupted-install recovery, digest-mismatch refusal, unowned-entrypoint conflict,
downgrade refusal, held-lock refusal, uninstall, purge behind a separate approval, scheduled-task
registration and removal at `Limited` run level, user-PATH add/remove with an unchanged machine PATH,
and schema validation of every captured install-result envelope.

Unverified and never reported as installed: the `axiom-graphd` core artifacts (`axiom-graphd.exe`,
`axiom.exe`) because the pinned core release is not built, so the release set declares them and records
them under `unverified_artifacts`; `axiom-mcp` and the skills versions because no version source is
available to this release; and every non-Windows target. `windows-x64` remains `certified: false` in
the platform matrix; a Windows run is not certification.
