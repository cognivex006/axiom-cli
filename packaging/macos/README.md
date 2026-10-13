# Intel Mac release packaging

Compose CLI, core, MCP, skills and runtime inputs from explicit owner revisions and SHA-256 values. `Build-EngineCandidate.py` verifies the skills owner manifest and all declared files before converting them to the engine bundle. `Attach-McpRuntime.py` binds the Python runtime, wheelhouse, MCP wheel and lock before copying the offline runtime bootstrap into the release set.

`release/build_macintel.py` composes historical release assets and checks their source revisions against immutable version tags. Current distribution composition uses the native release workflow. Packaging a local candidate does not sign, notarize or publish it.

## Candidate lifecycle

The candidate's `runtime/bootstrap.sh` supports installation/removal with `--dry-run` and `--apply --approve-digest <digest>`. Its update and rollback paths bind exact current and offered pointers, ask the installed engine to perform its transaction, then activate the distribution entrypoints and runtime.

Rollback verifies the retained runtime before changing engine state. A pending-intent record and OS lock protect interrupted operations. Recovery accepts only matching owned state, restores the engine through its public rollback interface and then restores the recorded runtime and entrypoints. Changed or unfamiliar state is refused while retaining the intent for inspection.

Native checks are `tests/macos/test_mcp_runtime.py`, `test_distribution.py` and `test_distribution_update.py`; use `--help` for exact local inputs. These checks cover installed queries, watching, service state, hashes, rollback, interruption and retained user data. Follow the [installation guide](../../docs/INSTALL.md) for published workstation installation instead of mixing candidate and released layouts.
