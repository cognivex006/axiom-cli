"""Native Windows retained-runtime repair proof in an isolated user profile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import uuid
import zipfile


def verify(archive, output):
    cases = []
    with tempfile.TemporaryDirectory(prefix="axiom-runtime-reinstall-") as temporary:
        work = Path(temporary).resolve()
        release = work / "release"
        with zipfile.ZipFile(archive) as z:
            z.extractall(release)
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
        data = work / "data"
        home = work / "home"
        home.mkdir()
        root = data / "Axiom"
        registry_key = r"Software\AxiomCliTest\RuntimeRepair-" + uuid.uuid4().hex
        env.update(HOME=str(home), USERPROFILE=str(home), LOCALAPPDATA=str(data),
                   AXIOM_CLI_TEST_USER_ENV_KEY=registry_key)
        if (os.environ.get("GITHUB_ACTIONS") == "true"
                or os.environ.get("AXIOM_CLI_TEST_ALLOW_ELEVATED") == "1"):
            env["AXIOM_CLI_TEST_ALLOW_ELEVATED"] = "1"

        def run(name, argv, expected=0):
            result = subprocess.run([str(x) for x in argv], env=env, cwd=work,
                stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=900)
            cases.append({"case": name, "exit_code": result.returncode,
                "expected_exit_code": expected,
                "output": (result.stdout + result.stderr).replace(str(work), "<isolated>")})
            if result.returncode != expected:
                raise AssertionError(name + ": " + cases[-1]["output"][-2500:])
            print(name + ": passed", flush=True)
            return result

        installer = release / "axiom-cli.exe"
        command = [installer, "install", "--from", release, "--yes", "--no-modify-path", "--json"]
        passed = False
        try:
            run("fresh install", command)
            axm = root / "bin/axm.exe"
            run("fresh doctor", [axm, "doctor"])
            sentinel = root / "state/human-notes.txt"
            sentinel.parent.mkdir(exist_ok=True)
            sentinel.write_text("preserve user data")
            current = root / "mcp-runtime/current.json"
            pointer = current.read_bytes()
            key = json.loads(pointer)["generation"]
            owner = root / "mcp-runtime/versions" / key / "owner.json"
            owner_bytes = owner.read_bytes()
            run("uninstall", [axm, "uninstall", "--yes", "--json"])
            run("reinstall after uninstall", command)
            run("doctor after reinstall", [axm, "doctor"])
            # A previous failed uninstall/update can leave a complete generation
            # but no active pointer. Reproduce that exact boundary explicitly.
            current.unlink()
            run("recover retained runtime without current pointer", command)
            assert owner.read_bytes() == owner_bytes, "retained generation was overwritten"
            run("same-release install repeated", command)
            record_path = root / "installed.json"
            manifest_path = root / "recorded-manifest.json"
            manifest = json.loads(manifest_path.read_bytes())
            for component in manifest["components"]:
                if component["component"] == "skills":
                    component["version"] = "0.1.10"
                    component["revision"] = "b" * 40
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            record = json.loads(record_path.read_bytes())
            record["channel_manifest_sha256"] = hashlib.sha256(manifest_path.read_bytes()).hexdigest()
            record_path.write_text(json.dumps(record), encoding="utf-8")
            run("diagnose mismatched release metadata", [axm, "version"], 2)
            run("repair inconsistent skills and recorded release", command)
            run("version after repair", [axm, "version"])
            run("doctor after repair", [axm, "doctor"])
            result = json.loads(run("actual MCP version", [axm, "mcp", "version", "--json"]).stdout)
            assert result["version"] == "0.1.11"
            assert sentinel.read_text() == "preserve user data"
            passed = True
        finally:
            import winreg
            try:
                winreg.DeleteKey(winreg.HKEY_CURRENT_USER, registry_key)
            except FileNotFoundError:
                pass
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps({"platform": "windows-x64", "execution": "native",
                "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
                "passed": passed, "cases": cases}, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    verify(args.archive.resolve(), args.out.resolve())
