# macOS architecture packaging

`Build-ReleaseSet.sh` is a shared POSIX recipe for `--arch arm64` and `--arch x64`. It runs only on macOS and checks each Mach-O input's architecture before packaging. It records the selected version from `--release-version` or `VERSION`, and refuses to invent one.

```sh
sh packaging/macos-arm64/Build-ReleaseSet.sh \
  --arch arm64 --out-dir out/macos-arm64 --build --json
```

`--build` runs the locked Rust release build for `aarch64-apple-darwin` or `x86_64-apple-darwin`. Supply `--cli-binary FILE` to package an existing binary. The recipe writes under the selected output directory and does not elevate.

The recipe is present; a released Apple silicon archive and native arm64 lifecycle verification are not established. Off-platform execution returns not-ready with a `NOT_RUN` record. `--service launchd-user` is unavailable in this recipe. Intel Mac's current engine-owned service path is described in the [distribution guide](../../docs/31-MACOS-X64-DISTRIBUTION.md).
