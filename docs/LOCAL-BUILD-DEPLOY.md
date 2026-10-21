# Build and deploy from your own machines

Canonical repository: https://github.com/cognivex006/axiom-cli

GitHub Actions is removed. Run from the repository root with existing pinned
toolchains/dependencies. This entrypoint never installs tools or publishes files.

```text
python tools/local_delivery.py check --out ../axiom-cli-checks
python tools/local_delivery.py build --out ../axiom-cli-build
```

Use a new output directory outside the source checkout each time. Build requires
clean committed source. The report records the actual native platform, source,
commands, exit codes and hashed logs. The specification repository has checks
only; it does not build a software runtime. Rust release binaries are written to
the standard target/release directory. MCP and skills packages go to the output
directory's artifacts folder. A successful source build is not release acceptance.

## Release acceptance and deployment

Build and run the existing native owner/runtime suites separately on Windows x64,
Linux x64 and Mac Intel. Keep exact immutable source revisions and native reports;
containers and WSL cannot stand in for another OS. CLI composition consumes checked
same-version owner releases; use release/compose_012.py after owner acceptance.
The former Actions candidate collector is legacy-only; machine release builds do
not call it. Native acceptance, terminal captures and rollback checks still apply.

Use the existing owner release collector to assemble the three native lanes:
graphd release/collect_assets.py, MCP release/collect_portable_assets.py, CLI
release/collect_distribution.py; skills uses release/build_github_release.py.
Each collector receives actual native outputs, not fabricated report files.
After CLI collection, release/oneline_assets.py generates bootstrap scripts and
channel metadata from the checked archive hashes. All tools are local scripts;
invoke each with --help for its explicit input paths and immutable source SHA.

Before any authorized publication, validate the full final directory:

```text
python tools/local_delivery.py verify-assets --assets ../checked-assets --source-revision <40-character-commit>
```

This verifies every inventory byte and the source binding; native acceptance and
review remain separate. Record version, source, checksums, reports and limitations.
Create a new immutable release tag only after all required lanes pass. Upload only
the checked collector directory to cognivex006/axiom-cli using gh release create
with --verify-tag and the reviewed release notes. Never overwrite an existing tag,
upload unchecked globs, or silently reuse a build from the old owner.

## Current migration state

Latest real cached component release: 0.1.15. Its original bytes contain legacy
download URLs and are retained locally as historical evidence. Rebuilt packages
with the new owner on every required native platform are pending; no new release
or tag is published by this source migration. The specification has no current
software release. Only actual new-machine results can establish readiness here.

Dependency audit is required: pre-provision cargo-deny 0.20.2 on the build machine; the local check plan invokes it without installing it. Missing tooling is a failed gate.
