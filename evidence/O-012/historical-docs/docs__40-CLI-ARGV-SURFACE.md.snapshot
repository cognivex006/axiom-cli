# CLI argv surface — axiom-cli

Owner: axiom-cli. Frozen by task `J-003`. The normative rules stay in
`axiom-specs/contracts/axiom-cli-distribution-contract.md`
section 2 and `axiom-specs/docs/16-CLI-AND-CONTROL-API.md` sections 1, 6
and 7 of the pinned `axiom-specs` revision. This guide is the owner-side description of the
implemented surface and must not restate a shared schema as an alternative edition.

## One executable, five verbs

`axiom-cli` (`axiom-cli.exe` on Windows) is the single distribution entrypoint. It exposes
`install`, `update`, `doctor`, `version` and `uninstall`. Every documented verb is reachable from
argv and listed by `--help`; the usage text and the dispatcher are pinned against each other by
`tests/argv_surface.rs`, so removing a verb from either one alone fails the suite.

The entrypoint is the distribution layer, and it owns real logic: it resolves the release set,
verifies every artifact's length and sha256, builds and seals the canonical approval plan, reads and
validates the channel manifest, and runs the transactional update. The installation engine,
bootstrap rules, service lifecycle and per-component placement stay owned by `axiom-graphd` and are
invoked through their published argv surface; this repository re-implements none of them.
`install` and `uninstall` hand placement and removal to the engine and report its result (L-002).

## Invocation

```
axiom-cli [GLOBAL OPTIONS] <verb> [verb options]
```

| Verb | Options |
|---|---|
| `install` | `[--yes] [--no-modify-path] [--adopt <path>]` \| `--dry-run` \| `--apply`, `--approve-digest <sha256>`, `--from <path>` |
| `update` | `[--yes] [--dry-run] [--channel <url\|path>]` \| `check [--all]` \| `plan --to <version> [--out <file>]` \| `apply --plan <file> --approve-digest <sha256>` \| `rollback --transaction <id> [--approve-digest <sha256>]` |
| `doctor` | `[--all]` |
| `version` | `[--all]` |
| `uninstall` | `[--yes]` \| `--dry-run` \| `--apply`, `--approve-digest <sha256>`, `[--purge-data]` |

A bare `install`, `uninstall` or `update` is the interactive human path (ADR-0033, L-003/L-005). It
prints the plan and asks `Proceed? [Y/n]` once, and the answer approves exactly that plan's canonical
digest. `--yes` (`-y`, or `AXIOM_INSTALL_YES=1`) approves without a prompt. Without a terminal (or
with `--json`) and without `--yes`, it prints the plan, changes nothing and exits `4`
(`confirmation_required`), so an argv-only consumer can never mistake an unperformed install for a
real one. A declined prompt exits `5`. `--dry-run` (token `"dry-run"`) reports the plan and changes
nothing; `--apply` (token `"apply"`) runs the transaction and requires `--approve-digest`. These two
automation modes never touch the PATH.

Global options: `-h`/`--help` (exit 0), `--json` (machine-readable mode) and `--verbose`
(diagnostics on stderr). `--json` may appear before or after the verb.

Invocation is program plus argv. No verb builds a shell string, requires Bash, WSL, Docker,
elevation or Node.js, or performs a silent auto-update. The only PATH change is the interactive
install's per-user entry, which is shown in the plan, skippable with `--no-modify-path` and removed
by uninstall. `install` and `uninstall` refuse to run elevated (`6` `elevated_refused`).

## Machine-readable mode

`--json` writes exactly one JSON object to stdout and leaves diagnostics on stderr:

```json
{"code":0,"status":"ok","message":"...","retryable":false,"details":{"engine_owner":"axiom-graphd","installed":false,"available_source":"unresolved"},"request_id":"..."}
```

`details.engine_owner` names the repository that owns the installation engine (`axiom-graphd`), so a
caller can see the ownership boundary without parsing the message. A refusal uses the same shape;
only `code`, `status`, `message` and `details` change.

## Exit vocabulary

The five verbs share the canonical CLI exit-code set owned by `axiom-specs`
`docs/16-CLI-AND-CONTROL-API.md` section 6. This repository defines no private space.

| Code | Meaning |
|---|---|
| `0` | success |
| `2` | validation |
| `3` | not found |
| `4` | not-ready / stale |
| `5` | authorization |
| `6` | conflict |
| `7` | timeout / busy |
| `8` | I/O or internal |
| `9` | incompatible |
| `10` | lock unavailable |
| `20` | partial multi-repository operation |

An unknown verb, a missing option value and an invalid flag combination return `2`.

## Engine handoff and current refusals

Every documented verb now dispatches to real work; `NotReady` is a conditional refusal, not the
universal answer. No verb returns `0` when it could not complete, and no refusal is an empty success
envelope. What remains is the engine handoff:

- `install --apply` (L-002): assembles the engine bundle, provisions the release's MCP runtime,
  delegates placement to the engine (`install plan --bundle` / `install apply`), then places
  `axiom-cli`, `axiom` and `axiom-graphd` in `<root>/bin` and writes `installed.json` last. Refusals:
  `3` `engine_not_found` when no engine binary is present (nothing placed), `6`
  `install_root_foreign` for a nonempty root holding non-Axiom entries, `2` `artifact_unverified:*`
  for a tampered artifact (before any placement), and the engine's own code when it refuses.
- `uninstall --apply`: invokes the engine's removal plan embedded in the approved uninstall plan,
  removes the owned `bin` files and the recorded PATH entry, and keeps user data. Refusals: `3`
  `nothing_installed` when no `installed.json` is recorded and `3` `engine_not_found`.
- `install --dry-run` or `install --apply` with no release set: `4` `no_release_set`.

An engine refusal means nothing was installed, updated or removed. The engine stays owned by
`axiom-graphd` (`I-003`/`I-004`); this layer resolves and verifies the release set and reports the
refusal rather than placing bytes itself.

## Engine discovery

The engine binary is resolved, most explicit first, from:

1. `AXIOM_ENGINE_BIN`, an absolute or relative path to the engine executable;
2. the directory of the running `axiom-cli` executable, which is where a release set places the
   engine beside the distribution entrypoint;
3. `PATH`.

The program name is `axiom` (`axiom.exe` on Windows). A missing engine is `3` `engine_not_found`
with a `searched[]` list naming every place that was tried, so the operator knows which binary to
place. The engine is invoked as program plus argv, never through a shell string.

## `--from <path>` and the local release set

`install ... --from <path>` resolves a release set locally instead of from a recorded channel
manifest:

- if `<path>` is a directory, the manifest is discovered in that directory, preferring
  `channel.json`, then `stable.json`, then `manifest.json`; a directory with none of them refuses
  `3` `release_set_manifest_missing`;
- if `<path>` is a file, that file is read as the manifest;
- artifact bytes are resolved from that same directory;
- `--from` is **not** approval. A mutating `--apply` still requires `--approve-digest <sha256>` bound
  to the canonical plan digest.

Without `--from`, the release set comes from the channel manifest the installed release recorded, or
from `AXIOM_CLI_CHANNEL_MANIFEST` while nothing is installed yet; otherwise the verb answers `4`
`no_release_set`.

## Environment overrides

| Variable | Effect |
|---|---|
| `AXIOM_ENGINE_BIN` | Explicit path to the `axiom-graphd` engine binary; overrides sibling and `PATH` discovery. |
| `AXIOM_CLI_INSTALL_ROOT` | The install root. Otherwise the per-user data location (`%LOCALAPPDATA%\Axiom` on Windows, else `$XDG_DATA_HOME/axiom`, else `$HOME/.local/share/axiom`). |
| `AXIOM_CLI_CHANNEL_MANIFEST` | A channel manifest to read while nothing is installed yet. |
| `AXIOM_CLI_ARTIFACT_CACHE` | The local artifact cache `install` and `update` verify bytes against; acquisition never contacts the network. |

These overrides apply to `install`, `uninstall`, `doctor` and `version` as well as `update`.

## Module map

```text
src/cli.rs        argv parse, verb dispatch, global options and the shared envelope
src/lifecycle.rs  install/uninstall release-set resolution, plan build, seal() and approval
src/doctor.rs     the doctor checks and their worst-code exit
src/version.rs    the version report
src/engine.rs     discovery of, and delegation to, the axiom-graphd engine
src/target.rs     delivery platform, tier and compiled-in host id
src/update/       the update channel: check / plan / apply / rollback
```

## Per-verb exit behaviour

| Verb | Behaviour | Exit codes today |
|---|---|---|
| `install` (bare) | Interactive (L-003): prints version, components, root, PATH change and size, asks `Proceed? [Y/n]` once, applies that plan's digest; `--yes` / `AXIOM_INSTALL_YES=1` skips the prompt; `--no-modify-path` skips the PATH change | `0` installed; `4` `confirmation_required` (no terminal, no `--yes`, nothing changed); `5` `confirmation_declined`; install refusals as `--apply` |
| `install --dry-run` | Resolves the release set, verifies artifacts, changes nothing | `0` resolved; `4` `no_release_set`; `2` validation |
| `install --apply` | Requires `--approve-digest`; verifies, provisions the MCP runtime, delegates placement to the engine, fills `<root>/bin` and writes `installed.json` | `2` missing or invalid digest / unverified artifact; `3` `engine_not_found`; `4` `no_release_set`; `6` conflict on a stale or mismatched approval or a foreign root |
| `uninstall` (bare) | Interactive: prints what is removed and kept, asks once (`--yes` skips); removes the engine install, the owned `bin` files and exactly the recorded PATH entry, keeps user data | `0`; `4` `confirmation_required`; `5` `confirmation_declined`; uninstall refusals as `--apply` |
| `uninstall --dry-run` | Reports the removal plan, changes nothing | `0`; `2` |
| `uninstall --apply` | Requires `--approve-digest`; runs the embedded engine removal plan, removes owned `bin` files and the recorded PATH entry, keeps user data | `3` `nothing_installed` / `engine_not_found`; `6` conflict on a stale or mismatched approval |
| `update` | `check` / `plan` / `apply` / `rollback` through the update channel | see `docs/50-UPDATE-CHANNEL.md` |
| `doctor` | Runs target, install root, integrity, prerequisite and engine checks | `0` clean; `4` a required check is unverified; `2` an installed generation fails integrity |
| `version` | Reports installed and available versions | always `0`; `installed:false` and `available:null` with nothing installed |
| any | Unknown verb, missing option value, invalid flag combination | `2` |

## Plan approval

A mutating `install`, `update` or `uninstall` transaction accepts `--approve-digest <sha256>` as its
approval artifact, bound to the canonical plan digest, exactly as `docs/16-CLI-AND-CONTROL-API.md`
section 7 requires. Supplying `--plan` or `--from` alone is not approval; a digest that is not 64 hex
characters is rejected as validation. At the CLI level, `uninstall --purge-data` additionally
requires `--apply` and an explicit approval; it is a different plan from a plain uninstall, so it is
a different digest, and an uninstall never recursively deletes `.axiom`.

## Building and testing

The binary builds with no third-party dependencies and no network fetch:

```
cargo build --release --offline
cargo test --offline
```

The pinned toolchain records `rust-version = "1.85"` in `Cargo.toml`; `Cargo.lock` pins the resolved
graph. The `J-003` slice's recorded run was Windows x64 with `rustc 1.98.1`; no evidence exists for
the lifecycle verbs added later. The current development host is `macos-x64`, which is
finish-first by declaration but not `certified`. No platform is certified by this document.
