"""Native installer regression with verified published 0.1.2 CLI bytes and legacy records.

Called only from the existing fresh hosted-runner harness. All account directories and
the Windows PATH registry destination are isolated; production user installs are not used.
The legacy manifest is a fixture matching the reported pre-generation store format.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import urllib.request
import uuid
import zipfile


def digest(data):
    return hashlib.sha256(data).hexdigest()


def snapshot(root):
    return {p.relative_to(root).as_posix(): digest(p.read_bytes())
            for p in root.rglob("*") if p.is_file()}


def verify(lane, command, base_env):
    windows = lane == "windows-x64"
    name = "axiom-cli.exe" if windows else "axiom-cli"
    extension = "zip" if windows else "tar.gz"
    asset = f"axiom-0.1.2-{lane}.{extension}"
    base = "https://github.com/orchex006/axiom-cli/releases/download/v0.1.2/"
    with urllib.request.urlopen(base + "SHA256SUMS", timeout=90) as response:
        sums = response.read().decode()
    expected = next(line.split("  ", 1)[0] for line in sums.splitlines()
                    if line.split("  ", 1)[-1] == asset)
    with urllib.request.urlopen(base + asset, timeout=180) as response:
        archive_bytes = response.read()
    if digest(archive_bytes) != expected:
        raise ValueError("published legacy archive checksum mismatch")
    cases = []
    key = r"Software\AxiomCliTest\LegacyStore-" + uuid.uuid4().hex
    with tempfile.TemporaryDirectory(prefix="axiom-legacy-store-") as temporary:
        work = Path(temporary).resolve()
        archive_path = work / asset
        archive_path.write_bytes(archive_bytes)
        if windows:
            with zipfile.ZipFile(archive_path) as archive:
                members = [p for p in archive.namelist() if Path(p).name == name]
                if len(members) != 1:
                    raise ValueError("ambiguous published legacy CLI member")
                old_cli = archive.read(members[0])
        else:
            with tarfile.open(archive_path) as archive:
                members = [p for p in archive.getmembers() if p.isfile() and Path(p.name).name == name]
                if len(members) != 1:
                    raise ValueError("ambiguous published legacy CLI member")
                old_cli = archive.extractfile(members[0]).read()
        home = work / "home"
        data = home / "data"
        store = data / ("Axiom" if windows else "axiom")
        (store / "bin").mkdir(parents=True)
        (store / "cli").mkdir()
        exe = store / "bin" / name
        exe.write_bytes(old_cli)
        exe.chmod(0o755)
        manifest = dict(schema_version=1, document_kind="axiom-cli-install-manifest",
                        release_version="0.1.2", components=[dict(component="axiom-cli",
                        installed_version="0.1.2", install_root=str(store), artifacts=[dict(
                            name=name, installed_path=str(exe), size_bytes=len(old_cli),
                            sha256=digest(old_cli))])])
        manifest_path = store / "install-manifest.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        state_path = store / "cli/state.json"
        state_path.write_text('{"document_kind":"axiom-cli-install-state","release_version":"0.1.2"}', encoding="utf-8")
        legacy = home / "axiom"
        pointer = legacy / "installs/ecosystem/current"
        pointer.parent.mkdir(parents=True)
        pointer.write_text('{"activated":[{"component":"axiom-graphd","version":"0.1.2"}]}', encoding="utf-8")
        (legacy / "notes.txt").write_text("user-owned bootstrap notes", encoding="utf-8")
        legacy_before = snapshot(legacy)
        keep = {p: p.read_bytes() for p in [manifest_path, state_path]}
        env = dict(base_env, HOME=str(home), USERPROFILE=str(home), LOCALAPPDATA=str(data),
                   XDG_DATA_HOME=str(data), XDG_CONFIG_HOME=str(home / ".config"),
                   AXIOM_CLI_TEST_USER_ENV_KEY=key)
        # Exclude another installation from discovery and PATH while retaining system tools.
        env["PATH"] = os.pathsep.join(p for p in env.get("PATH", "").split(os.pathsep)
                                      if not any((Path(p) / n).is_file()
                                                 for n in ["axm", "axm.exe", "axiom-cli", "axiom-cli.exe"]))
        def run(argv):
            return subprocess.run([str(a) for a in argv], env=env, capture_output=True,
                                  text=True, stdin=subprocess.DEVNULL, timeout=1200)
        def case(label, passed):
            cases.append(dict(case=label, passed=bool(passed)))
            if not passed:
                raise AssertionError(label)
        try:
            exe.write_bytes(old_cli + b"modified")
            before = snapshot(store)
            refused = run(command)
            case("altered legacy executable refused without store changes",
                 refused.returncode != 0 and "unowned, linked or modified" in refused.stdout + refused.stderr
                 and snapshot(store) == before and snapshot(legacy) == legacy_before)
            exe.write_bytes(old_cli)
            installed = run(command)
            if installed.returncode:
                raise AssertionError("legacy-store install failed: " + installed.stdout[-2500:] + installed.stderr[-1500:])
            case("published legacy CLI adopted through current one-line installer", (store / "installed.json").is_file())
            case("old metadata preserved", all(p.read_bytes() == value for p, value in keep.items()))
            case("old executable preserved in both backup locations",
                 (store / "legacy/cli-store-0.1.2" / name).read_bytes() == old_cli
                 and (store / "bin/.internal" / ("retired-axiom-cli.exe" if windows else "retired-axiom-cli")).read_bytes() == old_cli
                 and not exe.exists())
            case("separate bootstrap root and user notes preserved", snapshot(legacy) == legacy_before)
            axm = store / "bin" / ("axm.exe" if windows else "axm")
            case("adopted installation doctor passes", run([axm, "doctor"]).returncode == 0)
            case("only canonical public commands placed",
                 {p.stem for p in (store / "bin").iterdir() if p.is_file()} == {"axm", "axge", "axmp"})
        finally:
            if windows:
                import winreg
                try:
                    winreg.DeleteKey(winreg.HKEY_CURRENT_USER, key)
                except FileNotFoundError:
                    pass
    return dict(lane=lane, previous_version="0.1.2", old_archive_sha256=expected,
                old_cli_sha256=digest(old_cli), cases=cases, passed=all(c["passed"] for c in cases),
                scope="Published old CLI bytes with a manifest-shaped isolated legacy store; current candidate bootstrapper and actual engine/runtime. No real user installation mutation.")
