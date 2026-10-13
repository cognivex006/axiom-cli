# Linux, WSL2 and Apple silicon

## Availability

The current one-line installer supports Linux x64, WSL2 with Linux x64, Windows x64 and Intel Mac. Apple silicon and Linux arm64 are not available through that installer. A packaging recipe alone does not establish a released artifact.

## Linux and WSL2

Use the [installation guide](INSTALL.md) for the normal workflow. Linux release archives carry the CLI, core executables, MCP/skills and verified per-user runtime inputs. The low-level POSIX installer scripts are under `installers/linux/`; `packaging/linux/Build-ReleaseSet.sh` checks native ELF/x86_64 inputs before describing a release set.

The per-user service path uses `systemd --user` when a reachable user manager exists. With no manager it reports degraded service state rather than pretending registration succeeded. Explicitly requiring `--service systemd-user` when unavailable is a refusal and rolls entrypoint placement back.

WSL2 uses Linux artifacts but records its kernel, distribution and service-manager behavior separately. Run installer and runtime checks as a normal Linux user. Root may prepare an explicitly selected isolated test distro, but is not a product runtime requirement. See [native WSL2 verification](wsl-lifecycle.md).

## Apple silicon recipe

`packaging/macos-arm64/Build-ReleaseSet.sh` is shared by both Mac architectures. On a native macOS host:

```sh
sh packaging/macos-arm64/Build-ReleaseSet.sh \
  --arch arm64 --out-dir out/macos-arm64 --build --json
```

`--build` targets `aarch64-apple-darwin`; `--arch x64` selects `x86_64-apple-darwin`. A prebuilt binary can be supplied with `--cli-binary`. The recipe checks the Mach-O architecture against the requested target, obtains the version from explicit input or `VERSION`, and refuses a non-Darwin host.

The recipe's `--service launchd-user` path remains unavailable; it must not be confused with the separately implemented Intel Mac engine-owned LaunchAgent. No Apple silicon native lifecycle execution or downloadable arm64 archive is claimed here. See [recipe details](../packaging/macos-arm64/README.md).
