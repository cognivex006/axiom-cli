# Linux x64 MCP runtime candidate

`Provision-McpRuntime.py` accepts only local runtime, wheelhouse, MCP wheel and
dependency lock inputs with explicit SHA-256 values. K-404 uses uv managed
CPython 3.13.15 for Linux x86_64, repacked as regular files. The input choice
and exact archive digest are in `evidence/K-404/runtime-input.json`. The wheel
and lock are the K-403 bytes. Provisioning installs offline into
`mcp-runtime/versions/<generation>`, checks the installed entrypoint and
publishes `current.json` after validation. A prior generation remains for
rollback. `remove` deletes only owned generations and pointers and preserves
sibling user data. No developer venv, compiler, or package download is used
by the installed user flow.

Run `python3 packaging/linux/Provision-McpRuntime.py --help` for the input
surface. The native proof is `tests/linux/test_mcp_runtime.py --help` with the
four K-404 candidate inputs. K-405 composes this runtime with the CLI, core and
skills distribution; the candidate here is unsigned and unpublished.
