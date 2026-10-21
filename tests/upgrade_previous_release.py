"""Exercise a real previous-release upgrade through the verified current updater."""
from __future__ import annotations
import argparse
import functools
import hashlib
import http.server
import importlib.util
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import tarfile
import threading
import urllib.request
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = (ROOT / "VERSION").read_text().strip()
PREVIOUS = "0.1.12"


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify(artifacts, lane, output, previous_version=PREVIOUS, allow_elevated_fixture=False):
    extension = "zip" if lane == "windows-x64" else "tar.gz"
    suffix = ".exe" if lane == "windows-x64" else ""
    current = artifacts / f"axiom-{VERSION}-{lane}.{extension}"
    if not current.is_file():
        raise ValueError("current checked archive is missing")
    composer = load("upgrade_composer", ROOT / "release/compose_012.py")
    if suffix:
        with zipfile.ZipFile(current) as archive:
            binding = json.loads(archive.read("DISTRIBUTION-FILES.json"))
    else:
        with tarfile.open(current) as archive:
            binding = json.loads(archive.extractfile("DISTRIBUTION-FILES.json").read())
    if binding.get("version") != VERSION or binding.get("platform") != lane or not re.fullmatch(r"[0-9a-f]{40}", binding.get("source_revision", "")):
        raise ValueError("candidate source/version/platform binding is invalid")
    cases = []
    registry_key = r"Software\AxiomCliTest\UpgradePrevious-" + uuid.uuid4().hex
    with tempfile.TemporaryDirectory(prefix="axiom-upgrade-previous-") as temporary:
        work = Path(temporary).resolve()
        # Freeze generator inputs before the long transaction. Record the exact
        # bytes used, rather than silently reading a changing development tree.
        snapshot = work / "generator-source"
        generation_sources = {}
        for relative in ["packaging/oneline/generate_bootstrappers.py", *["installers/oneline/" + name for name in ["install.ps1.in", "update.ps1.in", "install.sh.in", "update.sh.in", "progress.ps1", "progress.sh"]]]:
            raw = (ROOT / relative).read_bytes().replace(b"\r\n", b"\n")
            target = snapshot / relative; target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(raw)
            generation_sources[relative] = hashlib.sha256(raw).hexdigest()
        generator = load("upgrade_generator", snapshot / "packaging/oneline/generate_bootstrappers.py")
        home = work / "home"
        home.mkdir()
        data = home / "data"
        root = data / ("Axiom" if suffix else "axiom")
        old_name = f"axiom-{previous_version}-{lane}.{extension}"
        base = f"https://github.com/cognivex006/axiom-cli/releases/download/v{previous_version}"
        sums = urllib.request.urlopen(base + "/SHA256SUMS", timeout=90).read().decode()
        expected = next(line.split("  ", 1)[0] for line in sums.splitlines() if line.split("  ", 1)[-1] == old_name)
        old_archive = work / old_name
        urllib.request.urlretrieve(base + "/" + old_name, old_archive)
        if sha(old_archive) != expected:
            raise ValueError("previous release archive digest mismatch")
        previous = work / "previous"
        composer.unpack(old_archive, previous)
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM") and (not suffix or k.upper() != "PSMODULEPATH")}
        env.update(HOME=str(home), USERPROFILE=str(home), XDG_DATA_HOME=str(data), LOCALAPPDATA=str(data),
                   APPDATA=str(home / "roaming"), CODEX_HOME=str(home / ".codex"), CLAUDE_CONFIG_DIR=str(home / ".claude"),
                   AXIOM_CLI_TEST_USER_ENV_KEY=registry_key)
        # Bootstrap discovery prefers commands on PATH. Keep it inside this fixture even
        # when the developer already has a real installation or an earlier CI case ran.
        names = ("axm", "axiom-cli", "axm.exe", "axiom-cli.exe")
        entries = [entry for entry in env.get("PATH", "").split(os.pathsep)
                   if entry and not any((Path(entry) / name).is_file() for name in names)]
        env["PATH"] = os.pathsep.join([str(root / "bin"), *entries])
        elevation_seam = bool(suffix and (os.environ.get("GITHUB_ACTIONS") == "true" or allow_elevated_fixture))
        if elevation_seam:
            env["AXIOM_CLI_TEST_ALLOW_ELEVATED"] = "1"

        def run(name, argv, expected_code=0):
            result = subprocess.run([str(a) for a in argv], env=env, cwd=work, stdin=subprocess.DEVNULL,
                                    capture_output=True, text=True, timeout=1200)
            cases.append({"case": name, "exit_code": result.returncode, "expected_exit_code": expected_code,
                          "stdout": result.stdout.replace(str(work), "<isolated>"),
                          "stderr": result.stderr.replace(str(work), "<isolated>")})
            if result.returncode != expected_code:
                raise ValueError(name + ": " + (result.stdout + result.stderr)[-3000:])
            return result

        run("install actual immutable previous release", [previous / ("axiom-cli" + suffix), "install", "--yes", "--no-modify-path"])
        record_path = root / "installed.json"
        original = json.loads(record_path.read_text())
        generation = original["current_generation"]
        original_manifest = (root / "recorded-manifest.json").read_bytes()
        command_paths = {name: root / "bin" / (name + suffix)
                         for name in ("axm", "axiom-cli", "axsk", "axge", "axmp")}
        original_commands = {name: sha(path) if path.is_file() else None
                             for name, path in command_paths.items()}
        old_command = next(command_paths[name] for name in ("axm", "axiom-cli")
                           if original_commands[name] is not None)
        served = work / "server"
        tag_root = served / ("v" + VERSION)
        tag_root.mkdir(parents=True)
        shutil.copyfile(current, tag_root / current.name)
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(served))
        handler.log_message = lambda *a, **k: None
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        address = f"http://127.0.0.1:{server.server_port}"
        channel = {"channel": "stable", "manifest_version": 1, "release": {
            "version": VERSION, "tag": "v" + VERSION, "archives": [{"platform": lane,
            "url": f"{address}/v{VERSION}/{current.name}", "sha256": sha(current), "size_bytes": current.stat().st_size}]}}
        (tag_root / "channel.json").write_text(json.dumps(channel))
        # Redirect discovery only; all previous release binaries, digests and layout stay intact.
        original["channel_source"] = f"{address}/v{VERSION}/channel.json"
        record_path.write_text(json.dumps(original))
        before = record_path.read_bytes()
        checksums = "".join(f"{sha(current) if platform == lane else '0' * 64}  axiom-{VERSION}-{platform}.{ext}\n"
                            for platform, ext in [("windows-x64", "zip"), ("linux-x64", "tar.gz"), ("macos-x64", "tar.gz")])
        scripts = generator.generate("v" + VERSION, checksums, address)
        script = tag_root / ("update.ps1" if suffix else "update.sh")
        script.write_bytes(scripts[script.name])
        bootstrapper_sha256 = sha(script)
        workspace = work / "user-project"
        workspace.mkdir()
        sentinel = workspace / "notes.txt"
        sentinel.write_text("user project data")
        try:
            command = [Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script, "-Yes"] if suffix else ["sh", script, "--yes"]
            upgraded = run("verified current updater migrates previous release", command)
            if not any(phase in upgraded.stderr.casefold() for phase in ["preparing the pinned python runtime", "staging verified axiom", "applying components with the verified graph engine"]):
                raise ValueError("update hid its actual install stages: " + upgraded.stderr)
            if "Installing new release [Working]" in upgraded.stderr or "updater active; elapsed" in upgraded.stderr:
                raise ValueError("wrapper heartbeat obscured the worker stages")
            axm = root / "bin" / ("axm" + suffix)
            version = run("current component versions", [axm, "version"]).stdout
            if version.count(VERSION) != 4:
                raise ValueError("the four installed components must report the new version: " + version)
            run("doctor after upgrade", [axm, "doctor"])
            mcp = json.loads(run("actual MCP runtime after upgrade", [axm, "mcp", "version", "--json"]).stdout)
            if mcp["version"] != VERSION:
                raise ValueError("upgraded command still launches an older MCP runtime")
            if not (root / "bin/graphd-engine" / ("axiom" + suffix)).is_file():
                raise ValueError("named engine runtime missing")
            if not (root / "generations" / generation).is_dir():
                raise ValueError("previous generation was not retained")
            run("rollback to previous release", [axm, "update", "rollback", "--transaction", "previous"])
            if record_path.read_bytes() != before or (root / "recorded-manifest.json").read_bytes() != original_manifest:
                raise ValueError("rollback did not restore the exact previous records")
            run("previous release executable restored", [old_command, "version", "--json"])
            for name, path in command_paths.items():
                restored = sha(path) if path.is_file() else None
                if restored != original_commands[name]:
                    raise ValueError("previous command bytes or absence not restored: " + name)
            run("retry current update after rollback with retained runtime", command)
            run("doctor after retained-runtime retry", [axm, "doctor"])
            run("rollback retried update", [axm, "update", "rollback", "--transaction", "previous"])
            run("previous version after second rollback", [old_command, "version", "--json"])
            if sentinel.read_text() != "user project data":
                raise ValueError("user data changed")
            if previous_version == "0.1.10":
                repair = work / "verified-current-repair"
                composer.unpack(current, repair)
                repair_root = work / "independent-repair-install"
                saved = {key: env.get(key) for key in ("AXIOM_CLI_INSTALL_ROOT", "AXIOM_HOME")}
                env.update(AXIOM_CLI_INSTALL_ROOT=str(repair_root), AXIOM_HOME=str(repair_root))
                try:
                    run("independent previous installation for repair",
                        [previous / ("axiom-cli" + suffix), "install", "--from", previous, "--yes", "--no-modify-path"])
                    run("direct install repair refreshes existing runtime",
                        [repair / ("axiom-cli" + suffix), "install", "--from", repair, "--yes", "--no-modify-path"])
                    mcp = json.loads(run("actual MCP runtime after direct repair", [repair_root / "bin" / ("axm" + suffix), "mcp", "version", "--json"]).stdout)
                    if mcp["version"] != VERSION:
                        raise ValueError("direct repair reused the previous MCP runtime")
                finally:
                    for key, value in saved.items():
                        if value is None: env.pop(key, None)
                        else: env[key] = value
                if sentinel.read_text() != "user project data":
                    raise ValueError("direct repair changed user data")
        finally:
            server.shutdown()
            if suffix:
                import winreg
                try:
                    winreg.DeleteKey(winreg.HKEY_CURRENT_USER, registry_key)
                except FileNotFoundError:
                    pass
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps({"platform": lane, "previous_version": previous_version, "version": VERSION,
                                 "execution": "native", "candidate_sha256": sha(current), "cases": cases,
                                 "passed": True, "source_revision": binding["source_revision"], "bootstrapper_sha256": bootstrapper_sha256, "generation_sources": generation_sources,
                                 "elevation_test_seam_used": elevation_seam, "scope": "Actual immutable previous release to exact checked candidate, then rollback; isolated user directories and Windows test registry key"}, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--platform", required=True, choices=["windows-x64", "linux-x64", "macos-x64"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--previous", default=PREVIOUS, choices=["0.1.6", "0.1.7", "0.1.8", "0.1.9", "0.1.10", "0.1.11", "0.1.12", "0.1.14"])
    parser.add_argument("--allow-elevated-fixture", action="store_true", help="allow only this isolated Windows fixture to use the existing CI elevation seam; not non-elevated-user evidence")
    args = parser.parse_args()
    verify(args.artifacts.resolve(), args.platform, args.out.resolve(), args.previous, args.allow_elevated_fixture)
