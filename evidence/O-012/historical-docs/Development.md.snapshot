# Development — axiom-cli

## Pinned governance

The canonical workflow is Development.md in `axiom-specs` at the exact revision in `spec.lock.json`. A missing or unverified pin blocks implementation; do not treat `spec.lock.example.json` as valid authorization.

## Before work

Read the selected task, the distribution contract and the platform matrix, verify completed dependencies, claim a single owner, record branch/base revision/dirty tree, explicit commit and push authority, allowed files and tests. Preserve unrelated changes. A–G and J letters do not override task `repo` ownership.

## During work

Implement one bounded task; coordinate public changes through spec ADR/RFC first. Include positive, negative and failure-boundary tests. The distributed executable is a thin wrapper over the `axiom-graphd` installation engine; never fork or re-implement that engine here. Installer and update code must be transactional: verify a digest before use, swap atomically, keep a rollback artifact, and self-update `axiom-cli` inside the same transaction. Native behavior must not depend on Bash, WSL, Docker, administrator privileges or symlinks. Executable launches use program and argv, not an interpolated shell string. No `curl | sh`, no hidden PATH mutation and no silent auto-update.

## Completion

Attach real test output/hashes, acceptance mapping, changed-file review, compatibility and rollback notes, updated local docs and Changelog.md. Record which native targets were actually executed and which stayed unverified; a container image is never native runtime evidence and a WSL2 run is never Windows evidence. Complete the canonical six preflight and eight completion checks; spec tooling validates evidence consistency, but independent CI reruns remain required.

## Branch/release policy

Follow the pinned canonical governance. Do not invent a remote, push credentials or release version. `axiom-cli` is the canonical distribution repository; the daemon plus CLI share one core release, while MCP and skills version independently. Documentation follows its component version. No production action is authorized by copying this seed.

## K-305 Windows x64 distribution candidate

The Windows release set carries the CLI, both native core executables, the
K-303 MCP wheel, the K-301 skills owner archive and the K-304 pinned runtime
inputs and provisioner scripts. `Build-ReleaseSet.ps1 -CoreCandidateManifest`
checks the exact K-302 binary and archive bytes before it describes the core
as a verified unsigned candidate. `Build-EngineCandidate.py` checks every
owner handoff, expands the skills payload and writes a local `channel.json`
for the installed `axiom-cli` entrypoint. This candidate remains unpublished.
The set includes its Windows install/uninstall/common scripts and K-304
provisioner, so the final harness runs from candidate and installed paths.
JSON written through `-Out` is the completed envelope and matches stdout.
The installed engine owns its service lifecycle; the native Windows Scheduled
Task acceptance uses a Limited per-user task running the installed daemon.
The current graphd `axiom service install` runtime still routes through a
Unix-only identity path on Windows. The local Task Scheduler harness proves
the installed daemon can run as a Limited per-user task, but the public service
verb remains a K-010/release blocker until its owner corrects it. The separate
Rust update state also makes `axiom-cli version --all` report no recorded
channel installation after this local candidate; inspect the PowerShell
distribution state and engine activation record for the exact installed bytes.

## K-306 Windows x64 composite update candidate

`packaging/windows/Build-UpdateKit.py` binds K-305 candidate A, the K-308
native core 0.1.2 B archive, and one CLI 0.1.2 build to an unsigned local kit.
`AXIOM_CLI_COMPOSITE_KIT` opts the public `axiom-cli update` verbs into the
manifest-checked Windows coordinator. It runs with the pinned installed MCP
Python under `venv/Scripts`, delegates ecosystem update/rollback to the graphd
engine, and applies the CLI release set with the Windows installer. Approval
binds exact kit and installed A bytes. The installer renames its owned active
entrypoint before placing B because a running Windows executable cannot be
overwritten. The coordinator retains a receipt and handles an interrupted
rollback only when the observed A/B bytes match it exactly. Candidate and
native Scheduled Task evidence are under `evidence/K-306/`; signing,
publication, and the graphd public Windows service verb remain separate gates.

## K-010 Windows final distribution lifecycle candidate

The K-010 kit keeps K-305 0.1.1 A immutable and packages the exact K-309
service-capable 0.1.2 core as B. Its candidate manifest, release set, channel
and update receipt carry K-010 identity and K-309 source/archive hashes.
`axiom-cli` still delegates the engine and Windows service lifecycle to the
graphd-owned `axiom` executable. If a user registers the new service after
updating from A, rollback to the older service-incompatible core refuses before
changing either pointer until `axiom service uninstall` removes the owned B
task. The final installed native lifecycle evidence belongs under
`evidence/K-010/`; it is unsigned local evidence, not release certification.

## K-304 Windows x64 runtime candidate

`packaging/windows/Provision-McpRuntime.ps1` uses the SHA-256-pinned CPython
3.13.15 embeddable archive to run the owner provisioner without a prepared
Python environment. `packaging/windows/runtime-input.json` binds that archive,
pip 26.1.2 and the exact K-303 offline MCP bundle. The owned generation keeps
`venv/Scripts/python.exe` and `axiom-mcp.exe`; the CLI verifies both digests
before passing the Windows PATH to the engine. Provision, rollback and removal
operate only under the selected per-user root. This is a candidate and does not
establish K-305 installation or K-307 query/process acceptance.

## K-404 Linux x64 runtime candidate

`packaging/linux/Provision-McpRuntime.py` provisions the K-403 MCP wheel and
hash-locked dependencies from local archives into a per-user Python 3.13
generation. `src/lifecycle.rs` verifies the active runtime executable hashes
and passes its absolute `venv/bin` to the engine for a `linux-x64` plan/apply.
The candidate inputs, transaction test and SHA-256 evidence are under
`evidence/K-404/`. This local container proof does not establish native WSL2
behavior or release provenance.

## K-405 container distribution candidate

The unsigned Linux x64 candidate is in `evidence/K-405/`. Its archive carries
the CLI and core binaries, MCP wheel, source skills and channel metadata; the
runtime-input receipt binds the separate K-404 Python archive, wheelhouse and
lock. `tests/linux/verify_k405_candidate.py` checks every archive member and
runtime receipt byte. The OCI image keeps uid/gid 10001 and uses an owned
volume. With no systemd user manager, the installed graphd daemon runs in the
foreground and drains on SIGINT. The local test uses the approved per-user
install and uninstall flows and preserves source, bindings and user data.
This is candidate verification; K-406 and K-407 retain update and query gates.

## K-406 local container update candidate

`packaging/linux/Build-UpdateKit.py` copies an exact K-405-format candidate,
the K-404 runtime receipt, Linux installer and update coordinator into an
unsigned local kit. `AXIOM_CLI_COMPOSITE_KIT` opts the public `axiom-cli update`
verbs into that installed-engine path. The coordinator verifies the kit and
every release member, binds the approval digest to the current CLI/engine
generation, and delegates ecosystem activation and rollback to `axiom`.
The distribution layer retains and restores its own CLI entrypoint. Without
the explicit kit, the K-003 guard still refuses split update; a feature-branch
candidate is not a published update channel.

## K-408 WSL2 native verification

Use the real WSL2 lane and exact K-401/402/403 Linux artifact inputs. Runtime user UID must be nonzero; root may only prepare the explicitly approved isolated test distro. The native harness and retained-generation helper live in tests/linux/. Owner Rust checks are recorded separately from WSL execution. No parent-done, foreign-machine path or release/signing gate is added to this local child. K-004 aggregates the completed child; K-409 remains its own installed MCP matrix.
