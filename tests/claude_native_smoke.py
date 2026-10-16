"""Verify managed registration with a real Claude Code client in a private profile.

This proves native plugin registration, not skill discovery in an AI session or graph access.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def dump(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8", newline="\n")


def verify(cli: Path, client: Path, bundle: Path | None = None):
    cases = []
    with tempfile.TemporaryDirectory(prefix="axiom private Claude ") as temporary:
        work = Path(temporary).resolve()
        profile = work / "profile"
        config = profile / ".claude"
        profile.mkdir()
        config.mkdir()
        root = work / "installation"
        root.mkdir()
        channel = json.loads((ROOT / "channels/stable.json").read_bytes())
        selected = next(c for c in channel["components"] if c["component"] == "skills")
        real_bundle = json.loads((bundle / "bundle.json").read_bytes()) if bundle else None
        if real_bundle:
            selected.update(version=real_bundle["version"], revision=real_bundle["revision"])
        manifest = json.dumps(channel).encode()
        version, revision = selected["version"], selected["revision"]
        (root / "recorded-manifest.json").write_bytes(manifest)
        dump(root / "installed.json", dict(schema_version=1, channel="stable",
             channel_manifest_sha256=hashlib.sha256(manifest).hexdigest(),
             current_generation="private-native-host-fixture", previous_generation=None,
             installed_at="2026-10-16T00:00:00Z"))
        source = root / "installs/ecosystem/skills" / version
        if bundle:
            # Only copy manifest-declared public payload, refusing links and escaping names.
            assert real_bundle is not None
            for row in real_bundle["entries"]:
                name = row["path"]
                assert name and not any(c in name for c in "\\:")
                assert not name.startswith("/") and all(p not in ["", ".", ".."] for p in name.split("/"))
                original = bundle
                for part in name.split("/"):
                    original = original / part
                    assert not original.is_symlink() and not original.is_junction()
                raw = original.read_bytes()
                assert len(raw) == row["size_bytes"] and hashlib.sha256(raw).hexdigest() == row["sha256"]
                target = source / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(raw)
            (source / "bundle.json").write_bytes((bundle / "bundle.json").read_bytes())
        for name, value in {
            ".agents/plugins/marketplace.json": dict(name="axiom-skills", plugins=[dict(name="axiom", source=dict(source="local", path="./plugins/axiom"))]),
            ".claude-plugin/marketplace.json": dict(name="axiom-skills", owner=dict(name="Axiom fixture"), plugins=[dict(name="axiom", source="./plugins/axiom")]),
            "plugins/axiom/.claude-plugin/plugin.json": dict(name="axiom", version=version, description="Private local Axiom registration verification", author=dict(name="Axiom fixture")),
        }.items():
            if not bundle:
                dump(source / name, value)
        skill = source / "plugins/axiom/skills/axsk/SKILL.md"
        if not bundle:
            skill.parent.mkdir(parents=True)
            skill.write_text("---\nname: axsk\ndescription: Private registration verification.\n---\nUse scoped graph evidence.\n", encoding="utf-8")
        entries = [dict(path=p.relative_to(source).as_posix(), kind="reference",
                   sha256=hashlib.sha256(p.read_bytes()).hexdigest(), size_bytes=p.stat().st_size)
                   for p in source.rglob("*") if p.is_file()]
        if not bundle:
            dump(source / "bundle.json", dict(schema_version=1, component="skills", version=version, revision=revision, entries=entries))
        dump(source.parent / "current", dict(schema_version=1, component="skills", version=version,
             revision=revision, manifest_sha256=hashlib.sha256((source / "bundle.json").read_bytes()).hexdigest()))
        host_bin = work / "client bin"
        host_bin.mkdir()
        # Copy the real native client so PATH cannot select another installation or wrapper.
        executable = host_bin / ("claude.exe" if os.name == "nt" else "claude")
        shutil.copyfile(client, executable)
        if os.name != "nt": executable.chmod(0o755)
        env = {k: v for k, v in os.environ.items() if not k.startswith(("AXIOM_", "CLAUDE_", "ANTHROPIC_"))}
        env.update(AXIOM_CLI_INSTALL_ROOT=str(root), CLAUDE_CONFIG_DIR=str(config),
                   HOME=str(profile), USERPROFILE=str(profile),
                   APPDATA=str(profile / "roaming"), LOCALAPPDATA=str(profile / "local"),
                   XDG_CONFIG_HOME=str(profile / ".config"), XDG_CACHE_HOME=str(profile / ".cache"),
                   CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1", DISABLE_AUTOUPDATER="1",
                   PATH=str(host_bin) + os.pathsep + os.environ.get("PATH", ""), PYTHONUTF8="1")
        native_version = subprocess.run([str(executable), "--version"], env=env,
                                       cwd=work, capture_output=True, text=True, timeout=20)
        assert native_version.returncode == 0
        for words in [["plugin", "marketplace", "list", "--json"], ["plugin", "list", "--json"]]:
            initial = subprocess.run([str(executable), *words], env=env, cwd=work,
                                     capture_output=True, text=True, timeout=30)
            assert initial.returncode == 0 and json.loads(initial.stdout) == [], "private Claude profile must start empty"
        def call(action, *options, expected=0):
            result = subprocess.run([str(cli), "--json", "skills", action, "claude", *options],
                                    env=env, cwd=work, capture_output=True, text=True, timeout=180)
            assert result.returncode == expected, (action, result.returncode, result.stdout, result.stderr)
            assert len(result.stdout.splitlines()) == 1 and result.stderr == ""
            doc = json.loads(result.stdout)
            cases.append(dict(action=action, exit_code=result.returncode))
            return doc
        call("install", "--dry-run")
        assert not (root / "host-integrations").exists()
        call("install", "--yes")
        receipt = root / "host-integrations/claude.json"
        assert receipt.is_file()
        before = receipt.read_bytes()
        config_files = [config / "settings.json", config / "plugins/known_marketplaces.json",
                        config / "plugins/installed_plugins.json"]
        config_before = {p: p.read_bytes() if p.exists() else None for p in config_files}
        doctor = call("doctor")
        assert doctor["details"]["host_discovery"] == "new_session_required_not_verified"
        assert doctor["details"]["mcp_query"] == "not_checked"
        assert receipt.read_bytes() == before
        assert {p: p.read_bytes() if p.exists() else None for p in config_files} == config_before
        native = subprocess.run([str(executable), "plugin", "list", "--json"], env=env,
                                cwd=work, capture_output=True, text=True, timeout=30)
        assert native.returncode == 0
        plugins = json.loads(native.stdout)
        own = [p for p in plugins if p["id"] == "axiom@axiom-skills" and p["scope"] == "user"]
        assert len(own) == 1 and own[0]["enabled"] is True
        cache = Path(own[0].get("readFromFolder") or own[0]["installPath"])
        assert cache.resolve().is_relative_to(work)
        cached_skill = cache / "skills/axsk/SKILL.md"
        trusted = cached_skill.read_bytes()
        cached_skill.write_text("human cache edit", encoding="utf-8")
        call("update", "--yes", expected=6)
        assert cached_skill.read_text() == "human cache edit" and receipt.read_bytes() == before
        cached_skill.write_bytes(trusted)
        call("update", "--yes")
        call("doctor")
        call("uninstall", "--yes")
        assert not receipt.exists()
        result = dict(passed=True, native_client=native_version.stdout.strip(),
                      client_sha256=hashlib.sha256(client.read_bytes()).hexdigest(),
                      cli_sha256=hashlib.sha256(cli.read_bytes()).hexdigest(), cases=cases,
                      private_profile=True, runtime_bundle="verified delivered payload in synthetic state" if bundle else "synthetic digest-bound local fixture",
                      bundle_manifest_sha256=hashlib.sha256((source / "bundle.json").read_bytes()).hexdigest(),
                      doctor_preserves_host_registration_config=True,
                      scope="Actual native Claude Code registration/doctor/update/uninstall and modified-cache refusal; no real user profile, AI session, credentials or graph query")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--bundle", type=Path, help="optional already-verified installed skills payload")
    args = parser.parse_args()
    dump(args.out, verify(args.cli.resolve(), args.client.resolve(), args.bundle.resolve() if args.bundle else None))
    print("PASS actual Claude Code lifecycle in a private profile; AI-session discovery and graph query unverified")
