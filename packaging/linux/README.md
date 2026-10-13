# Linux MCP runtime packaging

`Provision-McpRuntime.py` accepts local Python runtime, wheelhouse, MCP wheel and dependency-lock inputs with explicit SHA-256 values. It provisions offline into `mcp-runtime/versions/<generation>`, verifies the installed entrypoint and writes `current.json` after validation. Prior generations remain available for rollback.

The retained Linux candidate uses CPython 3.13.15 for x86_64. Inputs are regular-file archives bound to the owner wheel and lock. The installed flow needs no developer virtual environment or compiler and does not download Python dependencies.

```sh
python3 packaging/linux/Provision-McpRuntime.py --help
python3 tests/linux/test_mcp_runtime.py --help
```

Removal deletes owned generations and pointers while preserving sibling user data. Local candidate composition and runtime checks are separate from public release publication. See [Linux and WSL2 delivery](../../docs/60-LINUX-AND-MACOS-ARM64.md).
