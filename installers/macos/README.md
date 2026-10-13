# Intel Mac installer adapters

`Install-AxiomCli.sh` verifies a local release set and delegates approved placement to the `axiom` engine. It uses program/argv and does not implement another ecosystem installer or service protocol.

`Manage-Entrypoints.sh` is the older narrow bootstrap for CLI/engine entrypoints in `~/.local/bin`. It records owned bytes and a shell PATH block, refuses unowned conflicts or edited blocks, and preserves user data on removal. It does not provision the MCP runtime or register the daemon.

Full local candidates use their packaged `runtime/bootstrap.sh` for approved install, update, rollback and recovery. Runtime and engine pointer changes remain coordinated; a missing or corrupt retained runtime refuses rollback rather than inventing a replacement.

For current user installation, use [Installing Axiom](../../docs/INSTALL.md). For composition and native transaction tests, see [Intel Mac packaging](../../packaging/macos/README.md).
