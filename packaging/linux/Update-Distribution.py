#!/usr/bin/env python3
"""Coordinate an unsigned local Linux candidate update with the installed engine.

The engine owns its ecosystem transaction. This coordinator binds its approval
to the verified candidate and the exact installed CLI/engine generation, moves
the distribution entrypoint only after engine activation, and restores both on
a failed second phase. It is for local container candidates, not a release
channel or a signature verifier.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tempfile


HEX = re.compile(r"[0-9a-f]{64}\Z")


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def require(path: Path, expected: str) -> None:
    if not HEX.fullmatch(str(expected)) or path.is_symlink() or not path.is_file():
        raise ValueError(f"missing or unsafe candidate input: {path.name}")
    if any(parent.is_symlink() for parent in path.parents):
        raise ValueError(f"linked candidate input ancestor: {path.name}")
    if digest(path) != expected:
        raise ValueError(f"candidate input changed: {path.name}")


def candidate(release: Path, files_path: Path, runtime_input: Path) -> dict:
    if release.is_symlink() or not release.is_dir():
        raise ValueError("candidate release directory is missing or linked")
    if files_path.is_symlink() or runtime_input.is_symlink():
        raise ValueError("candidate manifests cannot be links")
    files = json.loads(files_path.read_text())
    rows = files.get("files", [])
    if files.get("task_id") not in ("K-405", "K-406") or not rows:
        raise ValueError("candidate file manifest is incompatible")
    names: set[str] = set()
    for row in rows:
        name = row["path"]
        relative = PurePosixPath(name)
        if (
            relative.is_absolute()
            or not relative.parts
            or ".." in relative.parts
            or name in names
        ):
            raise ValueError("candidate contains an unsafe or duplicate path")
        names.add(name)
        path = release.joinpath(*relative.parts)
        require(path, row["sha256"])
        if path.stat().st_size != row["size_bytes"]:
            raise ValueError(f"candidate size changed: {name}")
    observed = {
        p.relative_to(release).as_posix() for p in release.rglob("*") if p.is_file()
    }
    if observed != names:
        raise ValueError("candidate membership differs from manifest")
    receipt = json.loads((release / "candidate-inputs.json").read_text())
    if receipt != files["inputs"] or receipt["lane"] != "container-linux-x64":
        raise ValueError("candidate input receipt differs")
    require(runtime_input, receipt["runtime_input_sha256"])
    runtime = json.loads(runtime_input.read_text())
    if (
        runtime["task_id"] != "K-404"
        or runtime["runtime"]["sha256"] != receipt["runtime_archive_sha256"]
        or runtime["wheelhouse"]["sha256"] != receipt["wheelhouse_archive_sha256"]
    ):
        raise ValueError("runtime receipt differs")
    release_set = json.loads((release / "release-set.json").read_text())
    channel = json.loads((release / "channel.json").read_text())
    if (
        release_set["platform"] != "linux-x64"
        or release_set["arch"] != "x86_64"
        or release_set["signing"] != "unsigned"
        or channel["published"] is not False
        or [row["name"] for row in release_set["artifacts"]]
        != ["axiom-cli", "axiom", "axiom-graphd"]
    ):
        raise ValueError("candidate platform, component set or authority differs")
    for row in release_set["artifacts"]:
        require(release / row["name"], row["sha256"])
    core = release_set["artifacts"][1:]
    if (
        core[0]["version"] != core[1]["version"]
        or core[0]["revision"] != core[1]["revision"]
    ):
        raise ValueError("core versions or source revisions disagree")
    return {
        "files_sha256": digest(files_path),
        "release_set_sha256": digest(release / "release-set.json"),
        "channel_sha256": digest(release / "channel.json"),
        "runtime_input_sha256": digest(runtime_input),
        "cli_sha256": release_set["artifacts"][0]["sha256"],
        "core_sha256": core[1]["sha256"],
        "core_version": core[1]["version"],
        "core_revision": core[1]["revision"],
    }


def kit_manifest(path: Path, args: argparse.Namespace) -> str:
    if path.is_symlink() or not path.is_file():
        raise ValueError("candidate kit manifest missing or linked")
    body = json.loads(path.read_text())
    if body.get("task_id") != "K-406" or body.get("published") is not False:
        raise ValueError("candidate kit identity or authority differs")
    directory = path.parent
    expected = {
        "Update-Distribution.py": Path(__file__),
        "Install-AxiomCli.sh": args.cli_installer,
        "AxiomCli.Linux.Common.sh": args.cli_installer.parent
        / "AxiomCli.Linux.Common.sh",
        "candidate-files.json": args.files,
        "runtime-input.json": args.runtime_input,
        "cli-release/release-set.json": args.cli_installer.parent
        / "cli-release/release-set.json",
        "cli-release/axiom-cli": args.cli_installer.parent / "cli-release/axiom-cli",
    }
    for name, local in expected.items():
        if local.resolve() != (directory / name).resolve():
            raise ValueError("candidate kit path differs: " + name)
        require(local, body["files"][name])
    return digest(path)


def marker(path: Path) -> dict[str, str]:
    if path.is_symlink() or not path.is_file():
        raise ValueError("owned CLI marker is missing or linked")
    rows = [line.split("=", 1) for line in path.read_text().splitlines()]
    if any(len(row) != 2 for row in rows):
        raise ValueError("owned CLI marker is malformed")
    result = dict(rows)
    if len(result) != len(rows) or set(result) != {
        "version",
        "plan_digest",
        "install_root",
        "bin_dir",
        "state_dir",
        "generation",
        "entrypoint_sha256",
        "release_set_sha256",
        "installed_at",
    }:
        raise ValueError("owned CLI marker fields differ")
    return result


def installed(home: Path, root: Path) -> dict:
    cli_root = home / ".local/share/axiom-cli"
    owned = cli_root / ".axiom-cli-owner"
    value = marker(owned)
    cli = home / ".local/bin/axiom-cli"
    generation = Path(value["generation"])
    if (
        value["install_root"] != str(cli_root)
        or value["bin_dir"] != str(cli.parent)
        or not generation.is_relative_to(cli_root / "generations")
        or generation.is_symlink()
    ):
        raise ValueError("CLI owner record points outside its install root")
    require(cli, value["entrypoint_sha256"])
    require(generation / "axiom-cli", value["entrypoint_sha256"])
    engine = root / "installs/ecosystem/current"
    runtime = root / "mcp-runtime/current.json"
    skills = root / "installs/ecosystem/skills/current"
    for path in (engine, runtime, skills):
        if path.is_symlink() or not path.is_file():
            raise ValueError(
                f"installed ecosystem pointer missing or linked: {path.name}"
            )
    active = json.loads(engine.read_text())
    rows = {row["component"]: row for row in active["activated"]}
    if set(rows) != {"axiom-graphd", "axiom-mcp"}:
        raise ValueError("installed ecosystem component set differs")
    daemon = Path(rows["axiom-graphd"]["destination"])
    if not daemon.is_relative_to(root / "installs/ecosystem"):
        raise ValueError("installed daemon escapes owned root")
    require(daemon, rows["axiom-graphd"]["sha256"])
    mcp = json.loads(runtime.read_text())
    if mcp.get("wheel_sha256") != rows["axiom-mcp"]["sha256"]:
        raise ValueError("active MCP wheel and engine pointer disagree")
    return {
        "cli_sha256": digest(cli),
        "cli_marker_sha256": digest(owned),
        "cli_generation": str(generation),
        "cli_version": value["version"],
        "core_version": rows["axiom-graphd"]["version"],
        "core_revision": active.get("source_revision"),
        "core_sha256": rows["axiom-graphd"]["sha256"],
        "engine_pointer_sha256": digest(engine),
        "runtime_pointer_sha256": digest(runtime),
        "skills_pointer_sha256": digest(skills),
        "mcp_wheel_sha256": mcp["wheel_sha256"],
    }


def seal(checked: dict, before: dict, kit_sha256: str) -> str:
    body = {
        "schema_version": 1,
        "operation": "container-distribution-update",
        "candidate": checked,
        "before": before,
        "kit_manifest_sha256": kit_sha256,
    }
    return hashlib.sha256(
        json.dumps(body, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def compatible(checked: dict, before: dict) -> None:
    old = tuple(int(part) for part in before["core_version"].split("."))
    new = tuple(int(part) for part in checked["core_version"].split("."))
    if (
        new <= old
        or new[0] != old[0]
        or checked["core_sha256"] == before["core_sha256"]
    ):
        raise ValueError("candidate core is not a newer compatible generation")
    if checked["cli_sha256"] == before["cli_sha256"]:
        raise ValueError("candidate does not self-update the CLI")


def call(argv: list[str], env: dict[str, str]) -> dict:
    result = subprocess.run(argv, env=env, capture_output=True, text=True, timeout=120)
    try:
        body = json.loads(result.stdout)
    except json.JSONDecodeError:
        body = {"stdout": result.stdout[-1000:], "stderr": result.stderr[-1000:]}
    if result.returncode:
        raise ValueError(
            f"{Path(argv[0]).name} exited {result.returncode}: {json.dumps(body)[-1000:]}"
        )
    return body


def environment(home: Path, root: Path) -> dict[str, str]:
    value = json.loads((root / "mcp-runtime/current.json").read_text())
    bin_dir = root / "mcp-runtime/versions" / value["generation"] / "venv/bin"
    if not bin_dir.is_dir() or bin_dir.is_symlink():
        raise ValueError("active MCP runtime bin is missing or linked")
    return {
        "HOME": str(home),
        "PATH": f"{bin_dir}:/usr/bin:/bin:/usr/sbin:/sbin",
        "AXIOM_HOME": str(root),
        "AXIOM_CLI_INSTALL_ROOT": str(root),
        "AXIOM_CLI_COMPOSITE_UPDATE": "1",
    }


def atomic_bytes(path: Path, body: bytes, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix=".k406-", dir=path.parent)
    try:
        os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(body)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


@contextmanager
def update_lock(root: Path):
    path = root / "distribution-update.lock"
    descriptor = os.open(
        path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600
    )
    try:
        os.write(descriptor, f"pid={os.getpid()}\n".encode())
        os.fsync(descriptor)
        yield
    finally:
        os.close(descriptor)
        path.unlink()


def restore_cli(home: Path, before: dict, marker_bytes: bytes) -> None:
    source = Path(before["cli_generation"]) / "axiom-cli"
    require(source, before["cli_sha256"])
    cli = home / ".local/bin/axiom-cli"
    atomic_bytes(cli, source.read_bytes(), 0o755)
    owned = home / ".local/share/axiom-cli/.axiom-cli-owner"
    atomic_bytes(owned, marker_bytes)
    if digest(owned) != before["cli_marker_sha256"]:
        raise ValueError("restored CLI marker differs")


def engine_rollback(
    release: Path, root: Path, env: dict[str, str], transaction: str
) -> dict:
    argv = [
        str(release / "axiom"),
        "update",
        "rollback",
        "--transaction",
        transaction,
        "--json",
    ]
    return call(argv, env)


def cli_install(release: Path, home: Path, script: Path, env: dict[str, str]) -> dict:
    base = [
        "/bin/sh",
        str(script),
        "--release-set",
        str(script.parent / "cli-release/release-set.json"),
        "--install-root",
        str(home / ".local/share/axiom-cli"),
        "--bin-dir",
        str(home / ".local/bin"),
        "--service",
        "none",
    ]
    planned = call(base + ["--plan"], env)
    approved = planned["plan_digest"]
    return call(base + ["--apply", "--approve-digest", approved], env)


def apply(
    args: argparse.Namespace,
    checked: dict,
    before: dict,
    approved: str,
    kit_sha256: str,
) -> dict:
    digest_value = seal(checked, before, kit_sha256)
    if approved != digest_value:
        raise ValueError("stale or incorrect approval digest")
    root, home, release = args.root, args.home, args.release
    env = environment(home, root)
    pending = root / "distribution-update-pending.json"
    receipt = root / "distribution-update.json"
    marker_path = home / ".local/share/axiom-cli/.axiom-cli-owner"
    with update_lock(root):
        current = installed(home, root)
        if current != before or pending.exists() or receipt.exists():
            raise ValueError("installed generation changed or an update already exists")
        marker_bytes = marker_path.read_bytes()
        atomic_bytes(
            pending,
            (
                json.dumps({"before": before, "candidate": checked}, sort_keys=True)
                + "\n"
            ).encode(),
        )
        transaction = None
        events: list[dict] = []
        try:
            if args.fail_at == "download":
                raise ValueError("injected artifact acquisition failure")
            if args.fail_at == "stage":
                raise ValueError("injected staging failure")
            plan = call(
                [
                    str(release / "axiom-cli"),
                    "install",
                    "--from",
                    str(release),
                    "--dry-run",
                    "--json",
                ],
                env,
            )
            result = call(
                [
                    str(release / "axiom-cli"),
                    "install",
                    "--from",
                    str(release),
                    "--apply",
                    "--approve-digest",
                    plan["details"]["plan_digest"],
                    "--json",
                ],
                env,
            )
            events.append({"phase": "engine-update", "result": result})
            transaction = result.get("details", {}).get("engine_transaction")
            if not transaction:
                raise ValueError("engine update returned no rollback transaction")
            if args.fail_at in ("activation", "service-restart"):
                raise ValueError(
                    f"injected {args.fail_at} failure after engine activation"
                )
            entry = cli_install(release, home, args.cli_installer, env)
            events.append({"phase": "cli-self-update", "result": entry})
            after = installed(home, root)
            if (
                after["engine_pointer_sha256"] == before["engine_pointer_sha256"]
                or after["cli_sha256"] == before["cli_sha256"]
            ):
                raise ValueError("update did not activate both engine and CLI")
            if args.fail_at == "post-entrypoint":
                raise ValueError("injected post-entrypoint health failure")
            record = {
                "schema_version": 1,
                "candidate": checked,
                "before": before,
                "after": after,
                "engine_transaction": transaction,
                "approval_digest": digest_value,
                "marker_before_hex": marker_bytes.hex(),
            }
            atomic_bytes(receipt, (json.dumps(record, sort_keys=True) + "\n").encode())
            pending.unlink()
            return {
                "status": "updated",
                "plan_digest": digest_value,
                "before": before,
                "after": after,
                "engine_transaction": transaction,
                "events": events,
                "certified": False,
            }
        except Exception as error:
            failures = []
            if transaction:
                try:
                    engine_rollback(release, root, env, transaction)
                except Exception as rollback_error:
                    failures.append("engine: " + str(rollback_error))
            try:
                if digest(home / ".local/bin/axiom-cli") != before["cli_sha256"]:
                    restore_cli(home, before, marker_bytes)
            except Exception as rollback_error:
                failures.append("cli: " + str(rollback_error))
            try:
                if installed(home, root) != before:
                    failures.append("installed state differs from approved A")
            except Exception as rollback_error:
                failures.append("state: " + str(rollback_error))
            if failures:
                raise ValueError(
                    str(error) + "; rollback incomplete: " + "; ".join(failures)
                ) from error
            pending.unlink()
            raise


def rollback(args: argparse.Namespace) -> dict:
    root, home, release = args.root, args.home, args.release
    receipt = root / "distribution-update.json"
    if receipt.is_symlink() or not receipt.is_file():
        raise ValueError("no owned update receipt")
    value = json.loads(receipt.read_text())
    before, after = value["before"], value["after"]
    env = environment(home, root)
    with update_lock(root):
        if installed(home, root) != after:
            raise ValueError("installed B differs from the update receipt")
        marker_bytes = bytes.fromhex(value["marker_before_hex"])
        result = engine_rollback(release, root, env, value["engine_transaction"])
        restore_cli(home, before, marker_bytes)
        restored = installed(home, root)
        if restored != before:
            raise ValueError("rollback did not restore approved A")
        receipt.unlink()
        return {
            "status": "rolled_back",
            "before": after,
            "after": restored,
            "engine": result,
            "certified": False,
        }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "plan", "apply", "rollback"))
    parser.add_argument("--release", required=True, type=Path)
    parser.add_argument("--files", required=True, type=Path)
    parser.add_argument("--runtime-input", required=True, type=Path)
    parser.add_argument("--home", required=True, type=Path)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--cli-installer", required=True, type=Path)
    parser.add_argument("--kit-manifest", required=True, type=Path)
    parser.add_argument("--to", default="")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--plan-file", type=Path)
    parser.add_argument("--approve-digest", default="")
    parser.add_argument(
        "--fail-at",
        choices=(
            "download",
            "stage",
            "activation",
            "service-restart",
            "post-entrypoint",
        ),
    )
    args = parser.parse_args()
    try:
        kit_sha256 = kit_manifest(args.kit_manifest, args)
        if args.action == "rollback":
            candidate(args.release, args.files, args.runtime_input)
            print(json.dumps(rollback(args), sort_keys=True))
            return 0
        checked = candidate(args.release, args.files, args.runtime_input)
        before = installed(args.home, args.root)
        if (
            args.action == "check"
            and checked["core_sha256"] == before["core_sha256"]
            and checked["cli_sha256"] == before["cli_sha256"]
        ):
            print(
                json.dumps(
                    {"status": "up_to_date", "installed": before, "certified": False},
                    sort_keys=True,
                )
            )
            return 0
        compatible(checked, before)
        if args.to and args.to != checked["core_version"]:
            raise ValueError("requested version differs from candidate")
        plan_digest = seal(checked, before, kit_sha256)
        plan = {
            "status": "planned",
            "plan_digest": plan_digest,
            "before": before,
            "candidate": checked,
            "kit_manifest_sha256": kit_sha256,
            "certified": False,
        }
        if args.action in ("check", "plan"):
            if args.action == "plan" and args.out:
                atomic_bytes(
                    args.out, (json.dumps(plan, sort_keys=True) + "\n").encode()
                )
            print(json.dumps(plan, sort_keys=True))
            return 0
        if (
            not args.plan_file
            or args.plan_file.is_symlink()
            or not args.plan_file.is_file()
        ):
            raise ValueError("approved plan file missing or linked")
        if json.loads(args.plan_file.read_text()) != plan:
            raise ValueError(
                "approved plan differs from current candidate or installation"
            )
        print(
            json.dumps(
                apply(args, checked, before, args.approve_digest, kit_sha256),
                sort_keys=True,
            )
        )
        return 0
    except (
        OSError,
        ValueError,
        KeyError,
        IndexError,
        subprocess.TimeoutExpired,
    ) as error:
        print(
            json.dumps(
                {"status": "refused", "reason": str(error), "certified": False},
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
