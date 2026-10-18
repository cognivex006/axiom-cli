"""Exercise live download diagnostics through the real CLI and a bounded local HTTP peer."""
from __future__ import annotations
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time

from terminal_ux.capture import Capture, activity_frames

ROOT = Path(__file__).resolve().parents[1]

def verify(cli, out):
    cases = []
    with tempfile.TemporaryDirectory(prefix="axiom progress ") as temporary:
        work = Path(temporary).resolve()
        payload = b"x" * (2 * 1024 * 1024)
        lane = "windows-x64" if os.name == "nt" else ("macos-x64" if sys.platform == "darwin" else "linux-x64")
        extension = "zip" if os.name == "nt" else "tar.gz"
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), http.server.BaseHTTPRequestHandler)
        server.daemon_threads = True
        released = threading.Event()
        unknown = [False]
        automatic = [False]
        doc = json.dumps(dict(manifest_version=1, channel="stable", release=dict(version="0.1.11", tag="v0.1.11", archives=[dict(platform=lane, url=f"http://127.0.0.1:{server.server_port}/v0.1.11/axiom-0.1.11-{lane}.{extension}", sha256="0" * 64, size_bytes=len(payload))]))).encode()

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_GET(self):
                archive = self.path.endswith(extension)
                body = payload if archive else doc + (b" " * 131_072 if unknown[0] else b"")
                self.send_response(200)
                if not unknown[0]:
                    self.send_header("Content-Length", str(len(body)))
                self.send_header("Connection", "close")
                self.end_headers()
                slow = archive or unknown[0]
                if slow:
                    half = len(body) // 2
                    self.wfile.write(body[:half]); self.wfile.flush()
                    if automatic[0]:
                        time.sleep(1.4)
                    else:
                        released.wait(15)
                    self.wfile.write(body[half:])
                else:
                    self.wfile.write(body)

        server.RequestHandlerClass = Handler
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        try:
            modes = ["human", "unknown", "json", "off"]
            modes.append("terminal")
            for mode in modes:
                home = work / mode; home.mkdir()
                manifest = (ROOT / "channels/stable.json").read_bytes()
                (home / "recorded-manifest.json").write_bytes(manifest)
                record = dict(schema_version=1, channel="stable", channel_manifest_sha256=hashlib.sha256(manifest).hexdigest(), current_generation="fixture", previous_generation=None, installed_at="2026-10-16T00:00:00Z", components=[dict(component="axiom-graphd", version="0.1.9")])
                original = json.dumps(record).encode(); (home / "installed.json").write_bytes(original)
                env = {k:v for k,v in os.environ.items() if not k.startswith("AXIOM_")}
                # The existing CI seam applies only to this owned synthetic root; no install occurs.
                env.update(AXIOM_CLI_INSTALL_ROOT=str(home), AXIOM_CLI_TEST_ALLOW_ELEVATED="1", AXIOM_PROGRESS="off" if mode == "off" else "" if mode == "terminal" else "plain")
                unknown[0] = mode == "unknown"; automatic[0] = mode in ["json", "off"]; released.clear()
                words = [str(cli), "update", "--channel", f"http://127.0.0.1:{server.server_port}/channel?private-token-do-not-log"]
                words.append("--dry-run" if mode == "unknown" else "--yes")
                if mode == "json": words.append("--json")
                observed = False
                if mode == "terminal":
                    env.pop("CI", None); env["TERM"] = "xterm-256color"
                    c = Capture(words, env=env, cwd=home)
                    try:
                        c.until(b"50%", timeout=15)
                        assert c.alive()
                        time.sleep(.5)
                        frames = activity_frames(c.raw)
                        transfer = [f for f in frames if any("50%" in text for _, text in f["lines"])]
                        assert transfer and all(len(f["lines"]) == 2 for f in transfer), transfer
                        assert len({text[0] for f in transfer for _, text in f["details"]}) >= 2
                        observed = True; released.set()
                        code = c.wait(25); c.save(out.parent / (out.stem + "-frames"), mode)
                    finally:
                        released.set()
                        if c.alive(): c.close()
                    stdout = ""; stderr = bytes(c.raw).decode(errors="replace")
                    assert "100%" not in stderr and not c.screen.cursor.hidden
                else:
                    proc = subprocess.Popen(words, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    lines = []; seen = queue.Queue()
                    def read():
                        for line in proc.stderr:
                            lines.append(line); seen.put(line)
                    reader = threading.Thread(target=read, daemon=True); reader.start()
                    try:
                        if mode in ["human", "unknown"]:
                            deadline = time.monotonic() + 13
                            while time.monotonic() < deadline:
                                line = seen.get(timeout=max(.1, deadline-time.monotonic()))
                                if "downloading" in line and (mode == "unknown" or extension in line):
                                    assert proc.poll() is None
                                    observed = True; break
                            assert observed, lines
                            released.set()
                        code = proc.wait(timeout=25); stdout = proc.stdout.read(); reader.join(timeout=5)
                    finally:
                        released.set()
                        if proc.poll() is None: proc.kill(); proc.wait(timeout=5)
                    stderr = "".join(lines)
                    assert "\x1b" not in stderr and "elapsed" not in stderr
                assert code == (0 if mode == "unknown" else 2), (mode, code, stdout, stderr)
                assert (home / "installed.json").read_bytes() == original
                assert "private-token-do-not-log" not in stderr
                if mode in ["human", "terminal"]:
                    assert "approved plan pins" in stdout + stderr, (stdout, stderr)
                if mode == "unknown": assert "%" not in stderr and "ETA" not in stderr
                if mode == "json": json.loads(stdout); assert stderr == "", stderr
                if mode == "off": assert "downloading" not in stderr and "ETA" not in stderr
                cases.append(dict(case=mode, exit_code=code, live_progress_before_completion=observed, approved_state_preserved=True))
        finally:
            released.set(); server.shutdown(); server.server_close(); thread.join(timeout=5)
    return dict(passed=True, platform=sys.platform, cases=cases, scope="Real CLI, bounded local HTTP transfer and private synthetic state; no real user installation or remote release changes")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = verify(args.cli.resolve(), args.out.resolve())
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    print("PASS live known/unknown downloads, JSON/quiet output and integrity refusal")
