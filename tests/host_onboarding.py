"""Execute native host-adapter transactions against a private protocol fixture."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def dump(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")


def run(cli: Path, host_name="codex"):
    checks = []
    with tempfile.TemporaryDirectory(prefix="axiom host adapter ") as temporary:
        temp = Path(temporary)
        root, bin_dir = temp / "installation ; with spaces", temp / "host bin"
        bin_dir.mkdir()
        host = bin_dir / (host_name + (".exe" if os.name == "nt" else ""))
        subprocess.run(["rustc", "--edition=2021", str(ROOT / "tests/host_client_stub.rs"),
                        "-o", str(host)], check=True, capture_output=True)
        state_file = host.with_suffix(".state.json")
        channel = (ROOT / "channels/stable.json").read_bytes()
        selected = next(x for x in json.loads(channel)["components"] if x["component"] == "skills")
        version, revision = selected["version"], selected["revision"]
        root.mkdir()
        (root / "recorded-manifest.json").write_bytes(channel)
        dump(root / "installed.json", dict(schema_version=1, channel="stable",
             channel_manifest_sha256=hashlib.sha256(channel).hexdigest(),
             current_generation="host-fixture", previous_generation=None,
             installed_at="2026-10-16T00:00:00Z"))
        source = root / "installs/ecosystem/skills" / version
        files = {".agents/plugins/marketplace.json": dict(name="axiom-skills", plugins=[dict(name="axiom", source=dict(source="local", path="./plugins/axiom"))]),
                 "plugins/axiom/.codex-plugin/plugin.json": dict(name="axiom", version=version, skills="./skills/"),
                 ".claude-plugin/marketplace.json": dict(name="axiom-skills", plugins=[dict(name="axiom", source="./plugins/axiom")]),
                 "plugins/axiom/.claude-plugin/plugin.json": dict(name="axiom", version=version)}
        for name, value in files.items():
            dump(source / name, value)
        skill = source / "plugins/axiom/skills/axsk/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text("---\nname: axsk\ndescription: Fixture graph workflow.\n---\nUse registered scoped graph evidence.\n")

        def publish():
            entries = [dict(path=p.relative_to(source).as_posix(), kind="reference",
                       sha256=hashlib.sha256(p.read_bytes()).hexdigest(), size_bytes=p.stat().st_size)
                       for p in source.rglob("*") if p.is_file() and p.name != "bundle.json"]
            manifest = dict(schema_version=1, component="skills", version=version, revision=revision, entries=entries)
            dump(source / "bundle.json", manifest)
            digest = hashlib.sha256((source / "bundle.json").read_bytes()).hexdigest()
            dump(source.parent / "current", dict(schema_version=1, component="skills", version=version,
                 revision=revision, manifest_sha256=digest))
            return digest

        digest = publish()
        env = {k: v for k, v in os.environ.items() if not k.startswith("AXIOM_")}
        # Never allow fallback to a real installed client, even if fixture lookup fails.
        env.update(AXIOM_CLI_INSTALL_ROOT=str(root), PATH=str(bin_dir))

        def call(words, expected=0):
            p = subprocess.run([str(cli), "--json", "skills", *words], env=env,
                               capture_output=True, text=True, timeout=90)
            assert p.returncode == expected, (words, p.returncode, p.stdout, p.stderr)
            value = json.loads(p.stdout)
            assert len(p.stdout.splitlines()) == 1
            checks.append(dict(arguments=words, exit_code=p.returncode))
            return value

        # Host selection belongs to the approval digest; preview never installs.
        release = temp / "release"
        release.mkdir()
        declared = json.loads(channel)
        artifact = release / "cli-fixture.bin"
        artifact.write_bytes(b"verified fixture artifact")
        target = "windows-x64" if os.name == "nt" else ("macos-x64" if sys.platform == "darwin" else "linux-x64")
        for row in declared["components"]:
            if row["component"] == "axiom-cli":
                row["artifacts"] = [dict(platform=target, **{"class": "per-user-installer"},
                    url="https://fixtures.invalid/cli-fixture.bin",
                    sha256=hashlib.sha256(artifact.read_bytes()).hexdigest(), size_bytes=artifact.stat().st_size)]
        dump(release / "channel.json", declared)
        plans = []
        for extra in [[], ["--host", host_name]]:
            preview = subprocess.run([str(cli), "--json", "install", "--dry-run", "--from", str(release), *extra],
                                     env=env, capture_output=True, text=True, timeout=30)
            assert preview.returncode == 0, preview.stdout + preview.stderr
            plans.append(json.loads(preview.stdout)["details"])
        assert plans[0]["plan_digest"] != plans[1]["plan_digest"]
        # Slow native metadata calls must not each create a user-facing progress stage.
        human = subprocess.run(
            [str(cli), "install", "--dry-run", "--from", str(release), "--host", host_name],
            env={**env, "AXIOM_HOST_FIXTURE_QUERY_DELAY_MS": "350", "AXIOM_PROGRESS": "plain"},
            capture_output=True, text=True, timeout=30,
        )
        assert human.returncode == 0, human.stdout + human.stderr
        assert "Waiting for" not in human.stderr and "[Done]" not in human.stderr, human.stderr
        assert "published=" not in human.stdout and "release set:" not in human.stdout, human.stdout
        assert "component size:" in human.stdout and "plan digest:" in human.stdout, human.stdout
        checks.append(dict(case="slow-metadata-quiet-concise-plan", exit_code=0))
        assert plans[1]["plan"]["host_setup"]["host"] == host_name
        assert plans[1]["plan"]["host_setup"]["version"] == version
        assert not state_file.exists() and not (root / "host-integrations").exists()
        checks.append(dict(case="combined host selection is approval-bound; dry run is read-only", exit_code=0))

        call(["install", host_name, "--dry-run"])
        assert not state_file.exists() and not (root / "host-integrations").exists()
        call(["install", host_name], 4)
        assert not state_file.exists()
        (root / "update.lock").write_text("another transaction")
        call(["install", host_name, "--yes"], 10)
        (root / "update.lock").unlink()
        call(["install", host_name, "--yes"])
        before = state_file.read_bytes()
        doctor = call(["doctor", host_name])
        assert doctor["details"]["host_discovery"] == "new_session_required_not_verified"
        assert doctor["details"]["mcp_query"] == "not_checked"
        call(["list"])
        assert state_file.read_bytes() == before
        disabled = json.loads(before)
        disabled["installed"][0]["enabled"] = False
        dump(state_file, disabled)
        disabled_bytes = state_file.read_bytes()
        skipped = call(["update", host_name, "--yes"])
        assert skipped["status"] == "skipped_disabled"
        assert call(["doctor", host_name])["details"]["host_discovery"] == "disabled"
        call(["install", host_name, "--yes"], 4)
        assert state_file.read_bytes() == disabled_bytes
        state_file.write_bytes(before)
        receipt = root / f"host-integrations/{host_name}.json"
        old = json.loads(receipt.read_text())
        if host_name == "claude":
            cache_skill = host.with_suffix(".cache") / "skills/axsk/SKILL.md"
            cache_bytes = cache_skill.read_bytes()
            cache_skill.write_text("user modified client cache")
            for action in ["doctor", "update", "uninstall"]:
                call([action, host_name, "--yes"], 6)
            assert cache_skill.read_text() == "user modified client cache"
            cache_skill.write_bytes(cache_bytes)
            competing = json.loads(state_file.read_bytes())
            competing["installed"][0]["scope"] = "project"
            dump(state_file, competing)
            call(["update", host_name, "--yes"], 6)
            state_file.write_bytes(before)

        owned_skill = Path(old["marketplace_root"]) / "plugins/axiom/skills/axsk/SKILL.md"
        trusted = owned_skill.read_bytes()
        owned_skill.write_text("human changes")
        call(["update", host_name, "--yes"], 6)
        assert owned_skill.read_text() == "human changes" and state_file.read_bytes() == before
        owned_skill.write_bytes(trusted)
        skill.write_bytes(skill.read_bytes() + b"\nNew release content.\n")
        assert publish() != digest
        call(["update", host_name, "--yes"])
        assert json.loads(receipt.read_text())["marketplace_root"] != old["marketplace_root"]
        assert Path(old["marketplace_root"]).exists()
        call(["uninstall", host_name, "--yes"])
        assert not receipt.exists()
        host.with_suffix(".fail-add").write_text("fail once")
        call(["install", host_name, "--yes"], 20)
        assert receipt.with_suffix(".pending.json").exists()
        call(["recover", host_name, "--yes"])
        assert not receipt.exists() and not receipt.with_suffix(".pending.json").exists()
        call(["install", host_name, "--yes"])
        call(["uninstall", host_name, "--yes"])
        foreign = temp / "human marketplace"; foreign.mkdir()
        dump(state_file, dict(marketplaces=[dict(name="axiom-skills", root=str(foreign))], installed=[]))
        before = state_file.read_bytes()
        call(["install", host_name, "--yes"], 6)
        assert state_file.read_bytes() == before
        call(["list"])
        assert state_file.read_bytes() == before
        call(["install", "unknown", "--yes"], 2)
        assert state_file.read_bytes() == before
        # Both native protocols participate in one approved runtime plan.
        dump(state_file, dict(marketplaces=[], installed=[]))
        other = "claude" if host_name == "codex" else "codex"
        other_client = bin_dir / (other + (".exe" if os.name == "nt" else ""))
        shutil.copyfile(host, other_client)
        if os.name != "nt": other_client.chmod(0o755)
        call(["install", host_name, "--yes"])
        call(["install", other, "--yes"])
        both = subprocess.run([str(cli), "--json", "install", "--dry-run", "--from", str(release)],
                              env=env, capture_output=True, text=True, timeout=30)
        assert both.returncode == 0, both.stdout + both.stderr
        combined = json.loads(both.stdout)["details"]
        assert [p["host"] for p in combined["plan"]["host_setups"]] == ["codex", "claude"]
        assert combined["plan"]["host_setup"] == combined["plan"]["host_setups"][0]
        single = subprocess.run([str(cli), "--json", "install", "--dry-run", "--from", str(release), "--host", other],
                                env=env, capture_output=True, text=True, timeout=30)
        assert single.returncode == 0, single.stdout + single.stderr
        one = json.loads(single.stdout)["details"]
        assert "host_setups" not in one["plan"] and combined["plan_digest"] != one["plan_digest"]
        call(["uninstall", host_name, "--yes"])
        assert (root / f"host-integrations/{other}.json").is_file()
        call(["doctor", other])
        call(["uninstall", other, "--yes"])
        return dict(passed=True, platform=os.name, host_execution="native protocol fixture, not an actual AI-client session", host=host_name, checks=checks)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = dict(passed=True, hosts=[run(args.cli.resolve(), host) for host in ["codex", "claude"]])
    dump(args.out, result)
    print("PASS native host adapter lifecycle and preservation boundaries")
