# Linux x64 (including WSL2) and macOS arm64 — axiom-cli

Owner: axiom-cli. Task: `J-008`. This guide is the owner-side runbook for the two
**design-complete / test-later** delivery tiers. The normative rules live in
`axiom-specs/contracts/axiom-cli-distribution-contract.md`;
this file does not restate a shared schema.

## Tier position

Both targets are mandatory native targets in the platform matrix, but neither is
in the finish-first tier.

| Target | Tier | `certified` | Evidence |
|---|---|---|---|
| `linux-x64` | design-complete / test-later | `false` | one WSL2 execution record (see below); no native distro package run |
| `wsl2-linux-x64` | design-complete / test-later | `false` | recorded as `linux-x64` evidence, never as Windows evidence |
| `macos-arm64` | design-complete / test-later | `false` | empty — the artifact is **not built** here, leg `not_run` |

Neither target may be presented as finish-first. A cross-compile and a container
run are not runtime evidence for either target.

## What ships

| Path | Role |
|---|---|
| `installers/linux/AxiomCli.Linux.Common.sh` | shared POSIX-sh helpers: host facts, envelope builder, lock, systemd probe |
| `installers/linux/Install-AxiomCli.sh` | transactional per-user install/update |
| `installers/linux/Uninstall-AxiomCli.sh` | owned-only removal, `--purge` gated |
| `packaging/linux/Build-ReleaseSet.sh` | builds a release set from a real Linux x64 ELF artifact |
| `packaging/linux/release-set.schema.json` | documents the line-oriented release-set shape |
| `packaging/linux/install-result.schema.json` | Linux profile of the shared 33-key `install-result` envelope |
| `packaging/macos-arm64/Build-ReleaseSet.sh` | the one macOS recipe, parameterised by `--arch arm64|x64` |
| `packaging/macos-arm64/README.md` | the honest "recipe present, artifact NOT BUILT" record |
| `tests/linux/Invoke-AxiomCliLinuxDistributionTests.sh` | the executable Linux legs (L01–L20) |
| `tests/linux_distribution.rs` | static drift guards for the POSIX shell, the key set and the macOS record |

## Linux x64 install / update / uninstall

### Properties the path guarantees

- **glibc target.** The artifact is a dynamically linked ELF that requires glibc;
  the reference artifact's highest required symbol version is `GLIBC_2.34`, so it
  runs on Debian 12 (glibc 2.36) and Ubuntu 26.04 (glibc 2.43). `ldd` reports only
  `libgcc_s.so.1`, `libc.so.6` and the dynamic loader. No musl build is produced.
- **Per-user, no root.** Every path is derived from the invoking user's own
  `XDG_DATA_HOME` / `HOME`; the scripts never invoke `sudo`, `doas` or `su`, and
  the envelope always reports `elevation_required: false`.
- **No Bash, Docker or symlinks.** The scripts are `#!/bin/sh` and parse under
  `dash`; native behavior uses a real copy of the executable, never a symlink.
  The only host tools required are coreutils (`sha256sum`, `cp`, `mv`, `mkdir`,
  `chmod`, `sed`, `head`, `tail`, `wc`, `od`) and `tar`/`gzip` for archives; a
  missing prerequisite is refused by name, never guessed.
- **systemd-user when available, explicit degradation when not.** If the release
  set declares a user service, install writes and enables
  `~/.config/systemd/user/<unit>` when a `systemd --user` manager answers
  `systemctl --user show-environment`. When it does not, the install still
  succeeds and the `service_registration.reason` names the exact failing
  precondition (for example "systemd is not the running init in this environment
  (/run/systemd/system is absent)"). With `--service systemd-user` an unreachable
  user manager is a refusal (exit `4`, check `systemd-user-available`) and the
  install rolls its entrypoint back instead of silently skipping registration.
- **One machine-readable envelope.** Every invocation writes exactly one JSON
  `install-result` object to stdout and diagnostics to stderr, carrying the shared
  33-key envelope (`schema_version` … `request_id`), with `shell:
  "linux-posix-sh"` and `platform: "linux-x64"`.

### Exit codes

The canonical vocabulary of `axiom-specs/docs/16-CLI-AND-CONTROL-API.md` section 6:
`0` success, `2` validation, `3` not found, `4` not-ready, `5` authorization,
`6` conflict, `8` I/O, `9` incompatible, `10` lock unavailable, `20` partial.

### Replayable walkthrough

Every command below is documented in terms of `sh` (never `bash`) and never needs
root. `WS` is the workspace root that contains this repository.

```sh
# 0. One-time: obtain a Linux x64 artifact. On this host it was built in a
#    pinned Debian container and copied out (see "Reference artifact" below).

# 1. Build a release set from the real artifact (refuses a non-ELF/x86-64 input).
sh installers/linux/Build-ReleaseSet.sh \
    --out-dir /tmp/axiom-release --cli-binary /tmp/axiom-cli \
    --release-version 0.0.0-dev

# 2. Plan alone creates nothing and prints the envelope with its plan digest.
sh installers/linux/Install-AxiomCli.sh \
    --release-set /tmp/axiom-release/release-set.json \
    --install-root "$HOME/.local/share/axiom-cli" \
    --bin-dir      "$HOME/.local/bin" \
    --state-dir    "$HOME/.local/state/axiom" \
    --plan

# 3. Apply only with the approved plan digest printed by step 2.
sh installers/linux/Install-AxiomCli.sh \
    --release-set /tmp/axiom-release/release-set.json \
    --install-root "$HOME/.local/share/axiom-cli" \
    --bin-dir      "$HOME/.local/bin" \
    --state-dir    "$HOME/.local/state/axiom" \
    --apply --approve-digest <plan-digest>

# 4. Update is the same apply with a newer release set. Re-applying the same
#    release set is idempotent (outcome already_installed); an older release set
#    is refused (exit 9, check no-downgrade). The previous generation is kept.

# 5. Uninstall removes only what this installer owns. User state is preserved
#    unless deletion is separately approved with --purge.
sh installers/linux/Uninstall-AxiomCli.sh \
    --install-root "$HOME/.local/share/axiom-cli" \
    --bin-dir      "$HOME/.local/bin" \
    --state-dir    "$HOME/.local/state/axiom" \
    --plan
sh installers/linux/Uninstall-AxiomCli.sh \
    --install-root "$HOME/.local/share/axiom-cli" \
    --bin-dir      "$HOME/.local/bin" \
    --state-dir    "$HOME/.local/state/axiom" \
    --apply --approve-digest <plan-digest>
```

Observed layout after a successful install (Debian 12, user install root):

```text
~/.local/share/axiom-cli/.axiom-cli-owner
~/.local/share/axiom-cli/generations/0.0.0-dev-<plan-prefix>/axiom-cli
~/.local/bin/axiom-cli                      # mode 0755, digest == artifact digest
~/.local/state/axiom                        # created, preserved by install
```

## WSL2 lane

A WSL2 run is executed **as the Linux target** and is recorded as `linux-x64`
evidence. It is never recorded as Windows evidence, and the envelope reports
`platform: "linux-x64"`, `host.os: "linux"` and `host.wsl: true` with the real
`host.wsl_distro_name`.

```powershell
# From Windows PowerShell, run the Linux harness inside the distro.
wsl -d Debian -- sh /mnt/d/<path>/tests/linux/Invoke-AxiomCliLinuxDistributionTests.sh \
    --cli-binary /home/<user>/axiom-cli --scratch /home/<user>/j008-scratch
```

The run's own output records the kernel and distribution:

```text
Linux <host> 6.18.33.2-microsoft-standard-WSL2 #1 SMP PREEMPT_DYNAMIC ... x86_64 GNU/Linux
Debian GNU/Linux 12 (bookworm)   ID=debian   VERSION_ID=12   glibc 2.36   systemd absent
Ubuntu 26.04.1 LTS               ID=ubuntu   VERSION_ID=26.04 glibc 2.43  systemd PID 1
```

The same harness produces the two distinct service outcomes: on a distro without
a user manager the install degrades explicitly (`kind: "none"`), and on a distro
where systemd is init the unit is written, enabled and later removed
(`kind: "systemd-user"`, `registered: true`).

## macOS arm64

macOS arm64 artifacts are built from the **same recipe** as macOS x64. The recipe
is `packaging/macos-arm64/Build-ReleaseSet.sh`, parameterised by
`--arch arm64|x64`; every artifact entry records its own `arch` and the script
reads the Mach-O `cputype` back and refuses when it does not match, so the two
architectures cannot claim each other. `--service launchd-user` answers
not-ready (exit `4`) because the launcher is not built; the script proceeds only
on a `Darwin` host and otherwise prints a `NOT_RUN` record.

**This host cannot build or execute a macOS artifact.** The recipe is documented,
the artifact is **not built**, the target stays `certified: false` with empty
evidence and the leg is recorded `not_run`. See
[packaging/macos-arm64/README.md](../packaging/macos-arm64/README.md).

## Evidence

- Executed: the Linux x64 path (install / idempotent update / downgrade refusal /
  transactional recovery / uninstall / purge) on Debian 12 and Ubuntu 26.04 under
  WSL2, including the systemd-user registration leg. Recorded under
  `evidence/J-008/`.
- Not run: macOS arm64 (the current `macos-x64` host can run the shared recipe, but
  no macOS run is recorded), `linux/arm64`, signing and attestation, and image
  publication. The update channel is implemented (`J-007`) with publication closed;
  `channels/stable.json` is owned by `J-007`, and no version is invented here.

The Linux installer scripts never report a faked success, and the `axiom-cli` binary dispatches all
five verbs: `install --apply` and `uninstall --apply` refuse at the `axiom-graphd` engine handoff
rather than placing or removing bytes themselves (see
[docs/40-CLI-ARGV-SURFACE.md](40-CLI-ARGV-SURFACE.md)). The `--service launchd-user` refusal (macOS,
exit `4`) and the `--service systemd-user` refusal on an unreachable user manager (Linux, exit `4`)
stay as stated above.
