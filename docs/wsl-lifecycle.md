# Native WSL2 lifecycle verification

Use an isolated WSL2 distro and a normal non-root user. Bootstrap from verified Linux archives without a prepared development virtual environment. A complete run checks installation, MCP runtime provisioning, queries, file watching, restart, updates, failure recovery, rollback and removal while preserving project files and user data.

The runtime checks are in `tests/linux/test_mcp_runtime.py`; the lifecycle harness is `tests/linux/test_wsl_lifecycle.py`. Use each script's `--help` for its exact archived inputs and options. The complete lifecycle uses `--full`; resume options are diagnostic and do not replace a fresh run.

Check `systemd --user` where available and report its absence explicitly. Retained inactive payloads after rollback are valid retry inputs, so validation should check their hashes rather than require their deletion.

Record kernel, distribution, UID, source and input hashes, sanitized argv and exit codes. WSL execution is separate from native Windows and container execution. Packaging, signing and release publication are independent outcomes.
