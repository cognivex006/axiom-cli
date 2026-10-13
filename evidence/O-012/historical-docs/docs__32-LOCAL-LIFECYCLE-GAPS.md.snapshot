# Local lifecycle status

Reviewed 2026-09-22 against the current feature working trees. Native scope is
Mac x64; no released distribution or other platform is certified.

## Verified implementation

The engine composes persistent `serve`, repository-root graph publication,
exact-generation solution catalogs, and SIGINT/SIGTERM shutdown. MCP queries
the exact catalog vector and refuses missing members instead of using newer
project pointers.

The native combined fixture installs real graphd bytes, registers its per-user
LaunchAgent, queries actual analyzed source through the installed MCP wheel,
observes a source edit, updates the engine/skills pointers, checks the running
service, rolls both pointers back byte-for-byte, and removes the owned service
and runtime while preserving user data. The two graphd generations deliberately
use the same runnable development build in different version directories; this
is transaction evidence, not two certified releases.

The canonical uninstall plan/apply interface is declared in ADR-0014. The
distribution wrapper embeds the engine plan before computing its own approval
digest. Apply invokes that exact plan and only clears an unchanged distribution
marker. It exposes no user-data purge. Service identity, ownership verification,
file removal and recovery remain in the engine.

Evidence: `axiom-graphd/evidence/local-lifecycle-20260922/combined-native-lifecycle.json`
and its adjacent reproducible Python script; distribution-only proof and full CLI
gates are under `evidence/J-005/local-lifecycle-20260922/`.

## Remaining J-005 acceptance work

The scoped fixture supplies the distribution CLI and installation-engine CLI
from their build directories and uses an already prepared CPython dependency
environment. It does not prove a clean-machine installation of those entrypoints
or provision a dedicated versioned MCP virtual environment. The current macOS
wrappers do not place `axiom-cli` itself or manage the user's shell PATH. These
are local engineering items, not reasons to wait for a server or another OS.

`axiom-cli update` still addresses its separate delivery generation store. The
new local `axiom update plan --to <version> --bundle <dir> --out <file>` path
updates the actual engine ecosystem. An adapter is still required before the
distribution update channel can claim the same end-to-end behavior.

## Deferred release evidence

Windows x64, Linux x64 and macOS arm64 have not been run on the final bytes.
Released-core fixtures, signed/notarized artifacts and final release-gate evidence
remain unavailable. None is inferred from the local development tests.
