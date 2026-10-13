# macOS x64 distribution boundary

J-005 packages native x86_64 Mach-O bytes and routes lifecycle requests through
`axiom-cli` to the `axiom` installation engine. The distribution adapters do not
copy payloads, implement a second uninstaller, or invent a service/update
protocol: they pass verified inputs and approval-bound plans to the owning engine
with argv, never an interpolated shell command.

The local macOS x64 lifecycle has been exercised against a temporary home and
install root. The combined run installed the graph daemon and MCP wheel, verified
that query code loaded from the installed wheel, registered the per-user
`com.axiom.axiom-graphd` LaunchAgent, applied and rolled back the engine/skills
pointers, and removed the owned service and runtime while retaining a user file.
It recorded every command with exit code 0; after uninstall, `launchctl` reported
the canonical label absent. See
`axiom-graphd/evidence/local-lifecycle-20260922/combined-native-lifecycle.json`.

The verified local interface uses approval-bound engine plans:

```text
axiom install plan --bundle <verified-local-bundle> --out <engine-plan.json>
axiom install apply --plan <engine-plan.json> --approve-digest <engine-plan-digest>
```

The reviewed engine plan declares executable core rows with `read` and `execute`
permissions. The installer enforces that declared mode after activation. The same
engine owns per-user launchd registration/removal and ownership-scoped uninstall;
the distribution wrapper supplies verified bytes and invokes its documented argv
forms.

K-001 preparation: if the local channel manifest declares an `axiom` artifact
for macOS x64, the distribution verifies its size and SHA-256 with the rest of
the release set and invokes that executable for engine planning and apply. A
declared artifact that is missing, a directory, or not executable is refused;
the install cannot fall back to an unrelated engine on the developer PATH.

The separate clean-user bootstrap now places `axiom-cli` and `axiom` in
`~/.local/bin` from the digest-verified Intel Mac candidate, records both
versions and digests, and owns a narrow shell PATH block. The native test uses
an isolated home, confirms discovery in a login shell, checks idempotent rerun,
unowned-file refusal, injected install/uninstall rollback, human-edited block
refusal and data-preserving removal. This is entrypoint placement only; it does
not provision MCP or register the graph daemon service.

The earlier combined development fixture uses prepared CPython 3.13 dependencies
and the same runnable development graphd bytes in two version directories to
exercise transaction semantics. The new entrypoint test supplies native
candidate binaries from an isolated temporary release set. Neither proves a
dedicated versioned MCP environment, a single final distribution update flow,
signing/notarization, or a published immutable channel. Gatekeeper/quarantine
actions remain explicit operator actions and are never performed silently.
