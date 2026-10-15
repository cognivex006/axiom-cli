"""Exercise a real previous-release upgrade through the verified current updater."""
from __future__ import annotations
import argparse
import functools
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
VERSION = (ROOT / "VERSION").read_text().strip()
PREVIOUS = "0.1.7"


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify(artifacts, lane, output, previous_version=PREVIOUS):
    extension = "zip" if lane == "windows-x64" else "tar.gz"
    suffix = ".exe" if lane == "windows-x64" else ""
    current = artifacts / f"axiom-{VERSION}-{lane}.{extension}"
    if not current.is_file():
        raise ValueError("current checked archive is missing")
    composer = load("upgrade_composer", ROOT / "release/compose_012.py")
    generator = load("upgrade_generator", ROOT / "packaging/oneline/generate_bootstrappers.py")
    cases = []
    registry_key = r"Software\AxiomCliTest\UpgradePrevious-" + uuid.uuid4().hex
    with tempfile.TemporaryDirectory(prefix="axiom-upgrade-previous-") as temporary:
        work = Path(temporary).resolve()
        home = work / "home"
        home.mkdir()
        data = home / "data"
        root = data / ("Axiom" if suffix else "axiom")
        old_name = f"axiom-{previous_version}-{lane}.{extension}"
        base = f"https://github.com/orchex006/axiom-cli/releases/download/v{previous_version}"
        sums = urllib.request.urlopen(base + "/SHA256SUMS", timeout=90).read().decode()
        expected = next(line.split("  ", 1)[0] for line in sums.splitlines() if line.split("  ", 1)[-1] == old_name)
        old_archive = work / old_name
        urllib.request.urlretrieve(base + "/" + old_name, old_archive)
        if sha(old_archive) != expected:
            raise ValueError("previous release archive digest mismatch")
        previous = work / "previous"
        composer.unpack(old_archive, previous)
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
        env.update(HOME=str(home), USERPROFILE=str(home), XDG_DATA_HOME=str(data), LOCALAPPDATA=str(data),
                   AXIOM_CLI_TEST_USER_ENV_KEY=registry_key)
        # Bootstrap discovery prefers commands on PATH. Keep it inside this fixture even
        # when the developer already has a real installation or an earlier CI case ran.
        names = ("axm", "axiom-cli", "axm.exe", "axiom-cli.exe")
        entries = [entry for entry in env.get("PATH", "").split(os.pathsep)
                   if entry and not any((Path(entry) / name).is_file() for name in names)]
        env["PATH"] = os.pathsep.join([str(root / "bin"), *entries])
        if suffix and os.environ.get("GITHUB_ACTIONS") == "true":
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
        workspace = work / "user-project"
        workspace.mkdir()
        sentinel = workspace / "notes.txt"
        sentinel.write_text("user project data")
        try:
            command = [shutil.which("pwsh") or "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script, "-Yes"] if suffix else ["sh", script, "--yes"]
            run("verified current updater migrates previous release", command)
            axm = root / "bin" / ("axm" + suffix)
            version = run("current component versions", [axm, "version"]).stdout
            if version.count(VERSION) != 4:
                raise ValueError("the four installed components must report the new version: " + version)
            run("doctor after upgrade", [axm, "doctor"])
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
            if sentinel.read_text() != "user project data":
                raise ValueError("user data changed")
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
                                 "passed": True, "scope": "Actual immutable previous release to exact checked candidate, then rollback; isolated user directories and Windows test registry key"}, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--platform", required=True, choices=["windows-x64", "linux-x64", "macos-x64"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--previous", default=PREVIOUS, choices=["0.1.6", "0.1.7"])
    args = parser.parse_args()
    verify(args.artifacts.resolve(), args.platform, args.out.resolve(), args.previous)
