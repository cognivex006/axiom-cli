# macOS arm64 packaging — axiom-cli

Owner: `axiom-cli`. Task `J-008` owns the **arm64** leg of the macOS tier; the
**x64** macOS delivery is task `J-005`. Both legs are produced by the **same
recipe**: `packaging/macos-arm64/Build-ReleaseSet.sh`, a single POSIX-sh script
parameterised by `--arch arm64|x64`.

## State of this delivery

| Item | State |
|---|---|
| Recipe | **present** (`Build-ReleaseSet.sh`, shared by both macOS arches) |
| Build host required | macOS (`uname -s` = `Darwin`) |
| macOS arm64 artifact | **NOT BUILT** — the current `macos-x64` host can run the recipe, but no run has been recorded |
| macOS x64 artifact | **NOT BUILT here** — owned by `J-005` |
| Target certification | `certified: false`, `evidence: []` |
| Evidence status | `not_run` |

Nothing in this directory claims a macOS artifact exists. Running the recipe on
a non-macOS host is refused with exit `4` (not-ready) and a `NOT_RUN` record, so
an off-platform cross-compile can never be mistaken for macOS evidence. The
platform matrix keeps `macos-arm64` in the `design-complete-test-later` tier and
forbids presenting it as finish-first.

## Why the artifact is not built here

The `J-008` execution host was Windows 11 x64 with WSL2 and Docker, which cannot
produce *evidence* for a macOS target: a cross-compiled Mach-O is not a native
execution record and a container image is never native evidence either. The
current development host is `macos-x64`, so the shared macOS recipe is now
runnable natively — but no run has been recorded yet, so the honest record is
still `not_run` with `certified: false` and empty evidence. The contract
(`axiom-specs/contracts/axiom-cli-distribution-contract.md` §8) and the platform
matrix (`compatibility/platform-matrix.json`,
`distribution.rules.tier2_may_be_presented_as_finish_first = false`) both require
a native run before a target may be called `verified`.

## The recipe

On a native macOS host:

```sh
# arm64 (this card)
sh packaging/macos-arm64/Build-ReleaseSet.sh \
    --arch arm64 --out-dir out/macos-arm64 --build --json

# x64 (task J-005), same recipe, different --arch
sh packaging/macos-arm64/Build-ReleaseSet.sh \
    --arch x64 --out-dir out/macos-x64 --build --json
```

`--build` runs `cargo build --release --locked --target <triple>`; the triples
are `aarch64-apple-darwin` for `arm64` and `x86_64-apple-darwin` for `x64`.
Supply `--cli-binary FILE` instead of `--build` to package a prebuilt binary.

## Guarantees the recipe enforces

- **Architecture is recorded per artifact.** Every artifact entry carries its own
  `arch`. The script reads the Mach-O `cputype` from the artifact header and
  refuses (exit `9`) when it does not match `--arch`, so an arm64 release set and
  an x64 release set cannot silently claim each other.
- **No invented version.** The release version comes from `--release-version` or
  the repository `VERSION` file; absent both, the script refuses rather than
  guessing.
- **No off-platform artifact.** Only `Darwin` proceeds past the host gate.
- **No elevation.** The script writes only under `--out-dir` and never invokes
  `sudo`.
- **No launchd until it exists.** `--service launchd-user` returns exit `4`
  (not-ready): the launcher is not built, so no unit is declared.
```
