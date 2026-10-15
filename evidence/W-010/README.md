# Legacy CLI-store adoption regression

The reported unchanged 0.1.2 executable matches its original install-manifest SHA-256. The 0.1.8 guard consults only generation ownership and refuses before the existing CLI-store backup runs. The regression first failed with `legacy_command_foreign`; see the red-test output.

The fix recognizes only a single manifest-owned CLI artifact from the supported 0.1.0/0.1.2 store. Schema, kind, version, component, root, name, installed path, size and SHA-256 are checked. Other legacy command slots still need generation ownership. Linked ownership paths and conflicting backups are refused. The manifest and old CLI store are preserved, with old command bytes retained in both the legacy backup and recorded private retirement slot.

Native Windows tests run in isolated temporary fixtures. The CLI integration regression uses the existing native engine stand-in and redirected test environment; it proves wrapper lifecycle composition, not a published production install. The real installed user roots are read only. Source changes do not modify immutable 0.1.8 tags or assets. Ordinary non-elevated installation on the user's account has not been rerun. The POSIX symlink boundary test requires Linux/macOS CI. Logs replace machine-local path prefixes with descriptive placeholders.

Local checks: format, clippy, all Rust tests, release build, public documentation validator and Python public-doc/bootstrapper/release tests. Python platform-specific skips remain reported in the logs. Independent CI results and delivery revisions are recorded separately after source push. Security-sensitive migration review and explicit release authority remain prerequisites to any release.
