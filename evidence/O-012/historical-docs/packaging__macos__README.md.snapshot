# macOS x64 packaging

`Build-ReleaseSet.sh` is the J-005 entrypoint for a native Intel macOS build.
It delegates to the shared, architecture-validating macOS recipe with
`--arch x64`; it never cross-builds or labels another architecture as x64.

```sh
sh packaging/macos/Build-ReleaseSet.sh --out-dir out/macos-x64 --build --json
```

The recipe records no signing or notarization assertion. An unsigned artifact
is reported as unsigned; any Gatekeeper or quarantine action remains an
explicit operator action and is not performed by the packager.

For a complete Intel Mac entrypoint candidate, provide the two binaries from
the same pinned core revision:

```sh
sh packaging/macos/Build-ReleaseSet.sh --out-dir out/macos-x64 \
  --cli-binary target/x86_64-apple-darwin/release/axiom-cli \
  --engine-cli /path/to/pinned/axiom --engine-daemon /path/to/pinned/axiom-graphd \
  --core-version 0.1.0 --core-revision <40-hex-core-commit> --json
```

The packager verifies each input is executable x86_64 Mach-O, copies it into
the release set and records its byte count, SHA-256 and declared version. The
core revision argument is the operator's immutable source pin and must match
the build being supplied. `python3` is used only on the packaging host; it is
not a clean-user installation prerequisite. This candidate manifest is not a
signed or published channel manifest: it explicitly declares `unsigned` and
`not_notarized`, and the candidate installer accepts only those declarations.

For a local engine-install candidate, add a pinned MCP wheel and an
engine-format skills bundle after building the entrypoint set:

```sh
python3 packaging/macos/Build-EngineCandidate.py \
  --release-set out/macos-x64 \
  --mcp-wheel /path/to/axiom_mcp-0.1.0-py3-none-any.whl \
  --mcp-sha256 <verified-64-hex-digest> \
  --mcp-revision <40-hex-owner-commit> \
  --skills-bundle /path/to/engine-format-skills
```

The composer checks every declared core, wheel and skills payload byte, then
writes a local `channel.json` with `published: false` and the repository's
development trust root. It does not replace a channel or artifact that already
exists. `axiom-cli install --from out/macos-x64` can then resolve that candidate
locally. A clean user still needs the K-002 versioned Python runtime before
engine apply; the composer does not install Python or pass an MCP executable to
the engine. Full owner skills, final update/rollback and signing remain separate
release requirements. `test_engine_candidate.py` prepares Python only in its
temporary test HOME and can exercise an isolated LaunchAgent with `--service`.

## K-104 candidate MCP runtime

`Provision-McpRuntime.py` accepts only local, SHA-256 pinned CPython 3.13,
wheelhouse, MCP wheel and dependency lock inputs. For this Mac Intel candidate,
the interpreter is uv managed CPython 3.13.15 (`BUILD` 20260901), repacked as a
regular-file tarball so the archive has no link targets. The interpreter source
choice is candidate-only; the archive digest and exact bytes are recorded in
`evidence/K-104/`. The wheel and lock come from the K-103 handoff. All pip
installs use `--no-index`; the user host does not fetch packages or need a
compiler. The bootstrap caller extracts the verified interpreter and invokes
this script with that interpreter by absolute path; no developer venv or shell
activation is used.

The tool writes `mcp-runtime/versions/<generation>` below the chosen per-user
install root and activates it through `current.json` only after entrypoint and
version checks pass. The previous generation remains available for `rollback`.
`remove` deletes owned generations and pointers while preserving sibling user
data. `axiom-cli` verifies the active Python and launcher hashes before adding
that runtime's `venv/bin` to the installation engine's PATH for plan/apply.
This candidate tool does not select or publish a production download URL.

Run `<candidate-python3.13> packaging/macos/Provision-McpRuntime.py --help` for its argument
surface. The native transaction proof is
`tests/macos/test_mcp_runtime.py --help`; pass the exact archived inputs named
by the K-104 handoff.

## K-105 composed per-user candidate

Build the three-binary release set, then use `Build-EngineCandidate.py` with the
K-101 extracted `owner-source.tar`, `--skills-revision` and
`--skills-manifest-sha256`. Verify the source tar SHA against the K-101 handoff
before extraction. The composer checks the owner manifest's SHA and every
declared payload byte; the CLI converts that verified source to the engine
skills bundle. Then run `Attach-McpRuntime.py` with the K-104
`runtime-input.json`, its `--runtime-input-sha256`, and all four local artifacts.
It copies the runtime, wheelhouse, MCP wheel and lock, plus the per-user
bootstrap, into `runtime/`. The complete set remains unpublished, unsigned and
not notarized. The packaging host uses Python; the installed user flow does not.

The input revisions and SHA-256 values for this candidate are recorded in
`evidence/K-105/owner-handoff.json`. Use
`sh <release-set>/runtime/bootstrap.sh install --release-set <release-set>
--dry-run`, review its plan digest, then repeat with `--apply --approve-digest
<digest>`. The script runs the pinned CPython from a temporary extraction,
installs dependencies without network access, verifies installed executable
hashes and starts the user LaunchAgent. Use `uninstall` with the same dry-run
and approval flow. It removes owned executable and service entrypoints while
preserving local bindings and user files. Empty skills version directories left
by engine removal are pruned; any edited file remains. The native proof is
`tests/macos/test_distribution.py`.

## K-106 composite update candidate

`runtime/bootstrap.sh update --release-set <B> --dry-run` reports the digest
of the verified B candidate and the exact installed A pointers. Re-run with
`--apply --approve-digest <digest>` to ask the installed engine for its own
`update plan` and `update apply` transaction, then activate the distribution
entrypoints. An owned update receipt records the engine transaction and both
sets of pointer hashes. `runtime/bootstrap.sh rollback --release-set <B>` has
the same dry-run and approval pattern and asks the engine to restore its
retained A generation. The legacy distribution-only `axiom-cli update` verbs
refuse while an engine ecosystem pointer exists.

The runtime rollback target is verified with `provision.py rollback --dry-run`
before the engine pointer changes; the coordinator also binds the retained
runtime pointer bytes to the approved A state. This preflight rejects known
missing or corrupted A runtime inputs but cannot prevent a later I/O failure
or process death during rollback.

The coordinator writes `distribution-update-pending.json` before mutation and
uses an OS lock released on process exit. It refuses another operation while
an interrupted update is pending. `runtime/bootstrap.sh recover --release-set
<B>` gives that pending intent a dry-run approval digest. Apply discovers the
one new outer engine journal, checks its A pointer and B core identity, then
invokes the engine's public rollback transaction before restoring the pinned
A runtime and entrypoints. Foreign or changed state is refused while retaining
the intent. The native process-death harness exercises recovery and retry;
engine tests cover `prepared` before and between pointer moves.
Explicit rollback also writes the pending record before the engine pointer
moves and retains it if rollback fails or is interrupted.

A/B must carry distinct, source-pinned CLI and core artifacts with a new core
version. The native proof is `tests/macos/test_distribution_update.py` with
`--candidate-a`, `--candidate-b` and `--out`. It checks installed query and
watcher behavior, owned service status, exact activated hashes, rollback,
failure boundaries and preserved user data. The K-106 branch uses the owner's
source-pinned local B 0.1.1 core and records native A/B results in
`evidence/K-106/`. This candidate path does not sign, notarize or publish a
release.

## 0.1.1 MacIntel GitHub test release

`release/build_macintel.py` composes checksum-verified owner release assets,
checks each source revision against its immutable `v0.1.1` tag, and includes
the recorded CPython runtime and offline wheelhouse. The runtime attachment
and installer now select the wheel from the validated MCP version, supporting
both retained 0.1.0 candidates and new 0.1.1 inputs. See
`release/v0.1.1-notes.md` for download and install instructions.
