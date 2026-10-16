#!/usr/bin/env python3
"""M-001 native Mac Intel published bootstrap smoke test, with a real terminal.

On a fresh GitHub-hosted Mac Intel account, omit --work to use its actual HOME.
For local debugging, --work creates a NEW isolated home; its evidence explicitly
does not claim a clean OS account. No AXIOM_* prompt seam or staged asset is used.
The report can be assessed independently by M-002 after M-001 is complete.
"""
from __future__ import annotations

import argparse
import errno
import hashlib
import json
import os
import platform
import pty
import re
import select
import shlex
import shutil
import signal
import subprocess
import time
from pathlib import Path

BASE = "https://github.com/orchex006/axiom-cli/releases"
PROMPT = b"Proceed? [Y/n]"


def fetch(url):
    return subprocess.run(["curl", "-fsSL", "--retry", "2", "--max-time", "300", url],
                          capture_output=True, check=True, timeout=330).stdout


def terminal(argv, env, timeout=1800):
    """Answer only after observing the prompt; never use an AXIOM_* test seam."""
    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(str(argv[0]), [str(a) for a in argv], env)
    output = bytearray()
    answered = False
    deadline = time.monotonic() + timeout
    timed_out = False
    try:
        while True:
            if time.monotonic() >= deadline:
                timed_out = True
                os.killpg(pid, signal.SIGKILL)
                break
            if not select.select([fd], [], [], 1)[0]:
                continue
            try:
                chunk = os.read(fd, 65536)
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            output.extend(chunk)
            if PROMPT in output and not answered:
                os.write(fd, b"y\n")
                answered = True
        _, status = os.waitpid(pid, 0)
    finally:
        os.close(fd)
    code = os.waitstatus_to_exitcode(status)
    text = output.decode("utf-8", errors="replace").replace("\r\n", "\n")
    return code, text, answered, timed_out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Darwin" or platform.machine() != "x86_64" or os.getuid() == 0:
        parser.error("requires native non-root macOS Intel")
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith("AXIOM")}
    fresh_runner = (os.environ.get("GITHUB_ACTIONS") == "true" and
                    os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted")
    if args.work:
        if args.work.exists():
            parser.error("--work must be new; existing files are preserved")
        args.work.mkdir(parents=True)
        env.update(HOME=str(args.work.resolve()), XDG_DATA_HOME=str(args.work.resolve() / "data"),
                   SHELL="/bin/sh", PATH="/usr/bin:/bin:/usr/sbin:/sbin")
    elif not fresh_runner:
        parser.error("actual HOME may only be used on a fresh GitHub-hosted runner; use --work locally")
    home = Path(env["HOME"])
    root = Path(env.get("XDG_DATA_HOME") or home / ".local/share") / "axiom"
    if root.exists() or (home / "axiom").exists() or any(shutil.which(n, path=env["PATH"]) for n in ("axiom-cli", "axm")):
        parser.error("existing Axiom state found; clean-account precondition failed, nothing was changed")
    script_url = BASE + "/latest/download/install.sh"
    script = fetch(script_url)
    tag = re.search(rb"^AXIOM_TAG='(v[0-9]+\.[0-9]+\.[0-9]+)'$", script, re.M).group(1).decode()
    asset = f"axiom-{tag[1:]}-macos-x64.tar.gz"
    sums_bytes = fetch(f"{BASE}/download/{tag}/SHA256SUMS")
    sums = dict((name.lstrip("*"), digest) for digest, name in
                (line.split(None, 1) for line in sums_bytes.decode().splitlines()))
    downloaded_sha = hashlib.sha256(fetch(f"{BASE}/download/{tag}/{asset}")).hexdigest()
    hashes = {"install.sh": hashlib.sha256(script).hexdigest(), asset: downloaded_sha}
    script_digest = re.search(rb"'Darwin x86_64'\).*?expected='([0-9a-f]{64})'", script).group(1).decode()
    hashes_match = all(sums.get(name) == digest for name, digest in hashes.items()) and script_digest == downloaded_sha
    if not hashes_match:
        parser.error("published script/archive/embedded digest disagree with SHA256SUMS; nothing installed")
    modern_commands = tuple(map(int, tag[1:].split("."))) >= (0, 1, 7)
    cli = root / "bin" / ("axm" if modern_commands else "axiom-cli")
    oneliner = ["sh", "-c", f"curl -fsSL {script_url} | sh"]
    steps = []

    def step(name, argv, check=None, prompts=None):
        try:
            code, text, answered, timed_out = terminal(argv, env)
        except OSError as error:
            code, text, answered, timed_out = 127, str(error), False, False
        count = text.count(PROMPT.decode())
        passed = code == 0 and not timed_out and (check is None or check(text)) and (prompts is None or count == prompts)
        steps.append({"step": name, "command": shlex.join([str(a) for a in argv]), "exit_code": code,
                      "passed": passed, "prompts": count, "answered": answered, "timed_out": timed_out,
                      "output": text})
        print(f"[{'ok' if passed else 'FAIL'}] {name} exit={code}, prompts={count}", flush=True)
        if not passed:
            print(text[-2000:], flush=True)
        return passed

    installed = step("published one-liner", oneliner,
                     check=lambda text: f"downloading {tag} ({asset})" in text, prompts=1)
    if installed:
        step("version", [cli, "version"], check=lambda text: (
            all(re.search(r"^Axiom\s+" + label + r"\s+" + command + r"\s+" + re.escape(tag[1:]) + r"\s*$", text, re.M)
                for label, command in (("CLI", "axm"), ("Skills", r"\$?axsk"), ("Graph Engine", "axge"), ("MCP", "axmp")))
            if modern_commands else f"installed: {tag[1:]}" in text))
        step("doctor exit 0", [cli, "doctor"])
        step("update", [cli, "update"], check=lambda text: "up to date" in text)
        removed = step("uninstall", [cli, "uninstall"], prompts=1)
        if removed:
            steps.append({"step": "owned binaries removed", "passed": not cli.exists()})
            step("reinstall", oneliner, prompts=1)
            if cli.exists():
                step("doctor after reinstall", [cli, "doctor"])
                step("cleanup uninstall", [cli, "uninstall"], prompts=1)
    document = {"task_id": "M-001", "lane": "macos-x64", "execution": "native", "asset_source": "published",
                "clean_user_account": fresh_runner and args.work is None,
                "account_kind": "fresh-github-hosted" if fresh_runner and args.work is None else "isolated-home-existing-user",
                "manual_steps": [], "release_tag": tag, "published_url": script_url,
                "asset_hashes": hashes, "published_sha256sums": {name: sums[name] for name in hashes},
                "sha256sums_sha256": hashlib.sha256(sums_bytes).hexdigest(),
                "asset_hashes_match_sha256sums": hashes_match,
                "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(), "uid": os.getuid()},
                "environment": {"axiom_variables_required": [], "axiom_variables_set": [], "prompt_method": "native-pty",
                                "home_redirected": args.work is not None, "elevated": False},
                "run_url": "{}/{}/actions/runs/{}".format(os.environ.get("GITHUB_SERVER_URL", ""),
                             os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("GITHUB_RUN_ID", "local")),
                "source_revision": os.environ.get("GITHUB_SHA"), "steps": steps,
                "passed": hashes_match and installed and all(s["passed"] for s in steps)}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(document, indent=2) + "\n")
    return 0 if document["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
