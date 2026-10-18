"""Generated POSIX bootstrap native PTY frames and exact handoff/refusal behavior."""
import argparse
import hashlib
import http.server
import importlib.util
import io
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import time

from terminal_ux.capture import Capture, activity_frames

ROOT = Path(__file__).resolve().parents[1]


def verify(out):
    assert os.name == "posix" and os.getuid() != 0, "native non-root POSIX user required"
    spec = importlib.util.spec_from_file_location("gen", ROOT / "packaging/oneline/generate_bootstrappers.py")
    gen = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gen)
    fixture = b'''#!/bin/sh
printf 'Proceed? [Y/n] '
IFS= read -r answer
printf 'approved:%s\\n' "$answer"
exit "${AXIOM_UX_FIXTURE_EXIT:-0}"
'''
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w:gz") as tar:
        info = tarfile.TarInfo("axiom-cli"); info.mode = 0o755; info.size = len(fixture)
        tar.addfile(info, io.BytesIO(fixture))
        padding = os.urandom(262144)
        info = tarfile.TarInfo("padding"); info.size = len(padding)
        tar.addfile(info, io.BytesIO(padding))
    body = archive.getvalue()
    scenario = ["known"]
    released = threading.Event()
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            self.send_response(404 if scenario[0] == "network-error" else 200)
            if scenario[0] == "network-error": self.send_header("X-Unsafe-Label", "\x1b[2Juntrusted-header")
            if scenario[0] != "unknown": self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close"); self.end_headers()
            try:
                self.wfile.write(body[:len(body)//2]); self.wfile.flush()
                if scenario[0] == "resize": released.wait(15)
                else: time.sleep(1.4)
                self.wfile.write(body[len(body)//2:])
            except (BrokenPipeError, ConnectionResetError): pass
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    cases = []
    evidence = out.parent / (out.stem + "-frames")
    try:
        with tempfile.TemporaryDirectory(prefix="axiom shell visual ") as directory:
            work = Path(directory)
            for fetcher in ["curl", "wget"]:
                assert shutil.which(fetcher), fetcher + " required for mandatory native matrix"
                tools = work / fetcher; tools.mkdir()
                for program in ["sh", "id", "uname", "mktemp", "tr", "mv", "sed", "awk", "sleep", "stty", "wc", "tar", "cut", "sha256sum", "shasum", "rm", "mkdir", "cat", "dirname", "gzip", fetcher]:
                    resolved = shutil.which(program)
                    if resolved: (tools / program).symlink_to(resolved)
                for program in ["sha256sum", "shasum", "tar"]:
                    resolved = shutil.which(program)
                    if resolved:
                        (tools / program).unlink()
                        phase = "Extracting" if program == "tar" else "Verifying"
                        (tools / program).write_text(f'#!/bin/sh\nif [ "${{AXIOM_UX_DELAY_PHASE:-}}" = "{phase}" ]; then sleep 30; else sleep .4; fi\nexec "{resolved}" "$@"\n')
                        (tools / program).chmod(0o755)
                resolved_fetcher = shutil.which(fetcher)
                (tools / fetcher).unlink()
                (tools / fetcher).write_text('#!/bin/sh\nif [ -n "${AXIOM_UX_FETCH_DELAY:-}" ]; then sleep .6; fi\nexec "' + resolved_fetcher + '" "$@"\n')
                (tools / fetcher).chmod(0o755)
                for branch in ["install", "existing", "legacy"]:
                    for mode in ["known", "delayed-open", "unknown", "slow-measurement", "hash-error", "network-error", "child-seven", "cancel", "cancel-verify", "cancel-extract", "plain", "resize", "piped-script", "redirected", "ci", "off", "dumb"]:
                        if branch != "install" and mode not in ["known", "delayed-open", "unknown", "plain", "resize", "piped-script", "redirected", "ci", "off", "dumb", "cancel", "cancel-verify", "cancel-extract"]: continue
                        home = work / (fetcher + "-" + branch + "-" + mode); home.mkdir()
                        if branch == "existing":
                            root = home / "data/axiom"; (root / "bin").mkdir(parents=True)
                            (root / "installed.json").write_bytes(b"{}")
                            (root / "bin/axm").write_bytes(b"old executable must remain unchanged")
                            (root / "bin/axm").chmod(0o755)
                        elif branch == "legacy":
                            pointer = home / "axiom/installs/ecosystem/current"; pointer.parent.mkdir(parents=True)
                            pointer.write_bytes(b"{}")
                        scenario[0] = mode
                        released.clear()
                        digest = "0" * 64 if mode == "hash-error" else hashlib.sha256(body).hexdigest()
                        sums = "".join(digest + "  axiom-1.2.3-" + suffix + "\n" for suffix in ["windows-x64.zip", "linux-x64.tar.gz", "macos-x64.tar.gz"])
                        name = "install.sh" if branch == "install" else "update.sh"
                        generated = gen.generate("v1.2.3", sums, f"http://127.0.0.1:{server.server_port}")
                        script = home / name; script.write_bytes(generated[name])
                        env = {k: v for k, v in os.environ.items() if not k.startswith("AXIOM_") and k not in ["CI", "NO_COLOR"]}
                        env.update(PATH=str(tools), HOME=str(home), XDG_DATA_HOME=str(home / "data"), TERM="dumb" if mode == "dumb" else "xterm-256color", AXIOM_PROGRESS=mode if mode in ["plain", "off"] else "auto")
                        if mode == "ci": env["CI"] = "1"
                        if mode == "delayed-open": env["AXIOM_UX_FETCH_DELAY"] = "1"
                        if mode == "redirected":
                            result = subprocess.run(["/bin/sh", str(script), "--yes"], env=env, cwd=home, input=b"Y\n", capture_output=True, timeout=25)
                            assert result.returncode == 0 and b"\x1b" not in result.stdout + result.stderr, result
                            assert len(re.findall(rb"axiom (?:install|update): (?:downloading|verifying|extracting)", result.stderr)) == 3, result.stderr
                            cases.append(dict(case=fetcher + "-" + branch + "-" + mode, exit_code=0, automatic_plain=True))
                            continue
                        wc = tools / "wc"
                        if mode == "slow-measurement":
                            wc.unlink()
                            # exec preserves the observer-owned child identity during the stall.
                            wc.write_text('#!/bin/sh\nprintf "%s\\n" "$$" >"$AXIOM_UX_MEASURE_PID"\nexec sleep 30\n')
                            wc.chmod(0o755)
                            env["AXIOM_UX_MEASURE_PID"] = str(home / "measure.pid")
                        if mode in ["cancel-verify", "cancel-extract"]:
                            env["AXIOM_UX_DELAY_PHASE"] = "Verifying" if mode == "cancel-verify" else "Extracting"
                        if mode == "child-seven": env["AXIOM_UX_FIXTURE_EXIT"] = "7"
                        argv = ["/bin/sh", "-c", 'cat "$1" | sh -s --', "axiom-piped-script", str(script)] if mode == "piped-script" else ["/bin/sh", str(script)]
                        c = Capture(argv, env=env, cwd=home)
                        try:
                            if mode == "resize":
                                c.until((b"49%", b"50%"))
                                for width in [45, 35, 90]:
                                    c.resize(width)
                                    deadline = time.monotonic() + 2
                                    while True:
                                        display = c.display()
                                        first = next((i for i,s in enumerate(display) if "[" in s and "]" in s and ("%" in s or "indeterminate" in s)), None)
                                        lines = [s.rstrip() for s in display[first:] if s.strip()] if first is not None else []
                                        if len(lines) == 2 and all(len(s) < width for s in lines): break
                                        assert time.monotonic() < deadline, (fetcher, branch, width, lines)
                                        time.sleep(.025)
                                released.set()
                            if mode.startswith("cancel"):
                                phase = "Downloading" if mode == "cancel" else "Verifying" if mode == "cancel-verify" else "Extracting"
                                c.until(phase.encode()); c.send(b"\x03")
                            elif mode not in ["hash-error", "network-error"]:
                                c.until(b"Proceed? [Y/n]"); c.send(b"Y\r")
                            code = c.wait(25); c.save(evidence, fetcher + "-" + branch + "-" + mode)
                        finally:
                            released.set()
                            c.save(evidence, fetcher + "-" + branch + "-" + mode)
                            if c.alive(): c.close()
                        expected = 130 if mode.startswith("cancel") else 7 if mode == "child-seven" else 1 if mode in ["hash-error", "network-error"] else 0
                        assert code == expected, (fetcher, branch, mode, code, bytes(c.raw))
                        assert not c.screen.cursor.hidden, (mode, bytes(c.raw))
                        assert b"100%" not in c.raw
                        assert b"\x1b[2J" not in c.raw
                        assert b"No such file or directory" not in c.raw, (fetcher, branch, mode, bytes(c.raw))
                        if mode in ["plain", "off", "ci", "dumb"]:
                            assert b"\x1b" not in c.raw and b"elapsed" not in c.raw
                            stage_records = re.findall(rb"axiom (?:install|update): (?:downloading|verifying|extracting)", bytes(c.raw))
                            assert len(stage_records) == (0 if mode == "off" else 3), bytes(c.raw)
                        elif not mode.startswith("cancel"):
                            frames = activity_frames(c.raw)
                            assert (frames or mode == "network-error") and all(len(f["details"]) == 1 for f in frames), (fetcher, branch, mode, frames)
                            # Update's discovery paragraph is a permitted boundary before live work.
                            positions = [tuple(i for i, s in f["lines"] if s[0:1] in "|/-\\" or "[" in s and "]" in s) for f in frames]
                            assert all(len(p) == 2 for p in positions) and (len(set(positions)) == 1 or mode == "network-error" and not positions), frames
                            if mode not in ["network-error", "hash-error"]:
                                assert all(label.encode() in c.raw for label in ["Downloading", "Verifying", "Extracting"])
                                if mode not in ["unknown", "slow-measurement"]: assert b"49%" in c.raw or b"50%" in c.raw, bytes(c.raw)
                            if mode in ["unknown", "slow-measurement"]: assert b"%" not in c.raw and b"ETA" not in c.raw
                            if mode == "slow-measurement":
                                downloading = [f for f in frames if any("Downloading" in line for _, line in f["lines"])]
                                assert len({f["details"][0][1][0] for f in downloading}) > 1, downloading
                                pid = int((home / "measure.pid").read_text())
                                deadline = time.monotonic() + 3
                                while True:
                                    try: os.kill(pid, 0)
                                    except ProcessLookupError: break
                                    assert time.monotonic() < deadline, ("measurement child survived handoff", pid)
                                    time.sleep(.025)
                                wc.unlink(); wc.symlink_to(shutil.which("wc"))
                        if branch == "existing":
                            assert (root / "installed.json").read_bytes() == b"{}"
                            assert (root / "bin/axm").read_bytes() == b"old executable must remain unchanged"
                        cases.append(dict(case=fetcher + "-" + branch + "-" + mode, exit_code=code, native_frames=True, generation_sha256=hashlib.sha256(generated[name]).hexdigest()))
            home = work / "not-installed"; home.mkdir()
            script = home / "update.sh"; script.write_bytes(generated["update.sh"])
            env.update(HOME=str(home), XDG_DATA_HOME=str(home / "data"))
            refused = subprocess.run(["/bin/sh", str(script)], env=env, capture_output=True, timeout=10)
            assert refused.returncode == 1 and b"nothing to update" in refused.stderr
            cases.append(dict(case="no-install-refusal", exit_code=1, no_installation=True))
    finally:
        server.shutdown(); server.server_close()
    return dict(passed=True, platform=sys.platform, cases=cases, scope="Native POSIX generated scripts, real curl/wget and SHA/tar, private bounded delegate. Release transactions and GUI are separate gates.")


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__); ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args(); report = verify(args.out.resolve())
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")
    print("PASS native POSIX curl/wget bootstrap visual state, prompts, failures, cancellation and precise exits")
