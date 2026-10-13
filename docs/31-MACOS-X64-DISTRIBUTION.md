# Intel Mac distribution

Intel Macs use the `macos-x64` release archive and the [one-line installer](INSTALL.md#macos-linux-and-wsl2-not-as-root). Apple silicon is a separate target and is not available through that installer.

The CLI verifies native x86_64 Mach-O inputs and invokes `axiom` for approved ecosystem placement and removal. The graph engine owns the per-user `com.axiom.axiom-graphd` LaunchAgent; distribution adapters do not implement a second service protocol.

## Verification and placement

A declared engine artifact must have the expected size and SHA-256 and be an executable regular file. An invalid declared artifact stops installation rather than falling back to another engine on PATH. The engine's approved plan declares core read/execute permissions, and activation enforces them.

Current installation places the owned entrypoints in `<root>/bin` and uses a hash-bound per-user MCP runtime. Earlier test archives used a separate `runtime/bootstrap.sh` workflow and `~/.local/bin` entrypoints. Follow the instructions for the version you downloaded; do not mix those layouts.

## Native checks and limits

Native Intel Mac CI exercises the one-line install, version and installation diagnostics, update checks, uninstall and reinstall. Local lifecycle checks also cover launchd registration/removal, installed MCP loading, generation activation, rollback and retained user files.

An isolated test home is not a fresh operating-system account. An update check against the same version does not prove a newer-version upgrade. Native binaries are unsigned and not notarized; packaging does not silently change Gatekeeper or quarantine settings.

See [macOS packaging](../packaging/macos/README.md) for candidate composition and [update and rollback](50-UPDATE-CHANNEL.md) for transaction boundaries.
