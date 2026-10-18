"""Generated POSIX bootstrap native PTY frames and exact handoff/refusal behavior."""
import argparse
import hashlib
import http.server
import importlib.util
import io
import json
import os
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
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            self.send_response(404 if scenario[0] == "network-error" else 200)
            if scenario[0] == "network-error": self.send_header("X-Unsafe-Label", "\x1b[2Juntrusted-header")
            if scenario[0] != "unknown": self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close"); self.end_headers()
            try:
                self.wfile.write(body[:len(body)//2]); self.wfile.flush(); time.sleep(1.4)
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
                        (tools / program).write_text(f'#!/bin/sh\nsleep .4\nexec "{resolved}" "$@"\n')
                        (tools / program).chmod(0o755)
                for branch in ["install", "existing", "legacy"]:
                    for mode in ["known", "unknown", "slow-measurement", "hash-error", "network-error", "child-seven", "cancel", "plain"]:
                        if branch != "install" and mode not in ["known", "unknown"]: continue
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
                        digest = "0" * 64 if mode == "hash-error" else hashlib.sha256(body).hexdigest()
                        sums = "".join(digest + "  axiom-1.2.3-" + suffix + "\n" for suffix in ["windows-x64.zip", "linux-x64.tar.gz", "macos-x64.tar.gz"])
                        name = "install.sh" if branch == "install" else "update.sh"
                        generated = gen.generate("v1.2.3", sums, f"http://127.0.0.1:{server.server_port}")
                        script = home / name; script.write_bytes(generated[name])
                        env = {k: v for k, v in os.environ.items() if not k.startswith("AXIOM_") and k not in ["CI", "NO_COLOR"]}
                        env.update(PATH=str(tools), HOME=str(home), XDG_DATA_HOME=str(home / "data"), TERM="xterm-256color", AXIOM_PROGRESS="plain" if mode == "plain" else "auto")
                        wc = tools / "wc"
                        if mode == "slow-measurement":
                            wc.unlink()
                            # exec preserves the observer-owned child identity during the stall.
                            wc.write_text('#!/bin/sh\nexec sleep 30\n')
                            wc.chmod(0o755)
                        if mode == "child-seven": env["AXIOM_UX_FIXTURE_EXIT"] = "7"
                        c = Capture(["/bin/sh", str(script)], env=env, cwd=home)
                        try:
                            if mode == "cancel":
                                c.until(b"axiom install: downloading"); c.send(b"\x03")
                            elif mode not in ["hash-error", "network-error"]:
                                c.until(b"Proceed? [Y/n]"); c.send(b"Y\r")
                            code = c.wait(25); c.save(evidence, fetcher + "-" + branch + "-" + mode)
                        finally:
                            if c.alive(): c.close()
                        expected = 130 if mode == "cancel" else 7 if mode == "child-seven" else 1 if mode in ["hash-error", "network-error"] else 0
                        assert code == expected, (fetcher, branch, mode, code, bytes(c.raw))
                        assert not c.screen.cursor.hidden, (mode, bytes(c.raw))
                        assert b"100%" not in c.raw
                        assert b"\x1b[2J" not in c.raw
                        if mode == "plain":
                            assert b"\x1b" not in c.raw and b"elapsed" not in c.raw
                        elif mode != "cancel":
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
