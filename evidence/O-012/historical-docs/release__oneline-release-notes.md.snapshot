# Axiom release

Install with one command (ADR-0033) and update with one command (ADR-0035). Each script is pinned to this release's tag and archive SHA-256 values, refuses to run elevated, verifies the archive before extracting and then runs `axiom-cli install`, which shows the plan and asks once.

Windows (PowerShell 5.1+, no Administrator):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/install.ps1 | iex"
```

macOS, Linux and WSL2 (not as root):

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/install.sh | sh
```

Update an existing installation (0.1.3 or newer runs its own `axiom-cli update`, from 0.1.5 also installed as `axm`; 0.1.0/0.1.2 is upgraded through the installer; nothing installed stops without change):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/orchex006/axiom-cli/releases/latest/download/update.ps1 | iex"
```

```sh
curl -fsSL https://github.com/orchex006/axiom-cli/releases/latest/download/update.sh | sh
```

Pinned to this version: replace `latest/download` with `download/<tag>`.

Then `axm version`, `axm doctor`, `axm update` and `axm uninstall` (user data is kept); `axm` is the short name of `axiom-cli`. Every asset is listed in `SHA256SUMS`; `channel.json` is the update channel for `axiom-cli update`.

Full guide, covering options, upgrading from 0.1.0/0.1.2 and troubleshooting: https://github.com/orchex006/axiom-cli/blob/main/docs/INSTALL.md

Release maintainers: `releases/latest/download/…` resolves only to the newest release that is neither a draft nor a prerelease. A release meant for the one-line install MUST NOT be marked prerelease; otherwise users need the pinned `releases/download/<tag>/…` form.
