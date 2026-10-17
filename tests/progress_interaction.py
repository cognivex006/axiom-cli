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

ROOT = Path(__file__).resolve().parents[1]

def verify(cli):
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
            if os.name != "nt":
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
                master = slave = None
                if mode == "terminal":
                    import pty
                    master, slave = pty.openpty()
                proc = subprocess.Popen(words, env=env, stdout=subprocess.PIPE,
                                        stderr=slave if slave is not None else subprocess.PIPE, text=True)
                if slave is not None:
                    os.close(slave)
                lines = []; seen = queue.Queue()
                def read():
                    if master is not None:
                        while True:
                            try:
                                chunk = os.read(master, 4096)
                            except OSError:
                                return
                            if not chunk:
                                return
                            text = chunk.decode("utf-8", errors="replace")
                            lines.append(text); seen.put(text)
                    for line in proc.stderr:
                        lines.append(line); seen.put(line)
                reader = threading.Thread(target=read, daemon=True); reader.start()
                observed = False
                try:
                    if mode in ["human", "terminal", "unknown"]:
                        deadline = time.monotonic() + 13
                        while time.monotonic() < deadline:
                            try:
                                line = seen.get(timeout=max(0.1, deadline - time.monotonic()))
                            except queue.Empty:
                                raise AssertionError((mode, lines, proc.poll())) from None
                            if (mode in ["human", "terminal"] and "ETA ~" in line) or (mode == "unknown" and "total size unknown" in line):
                                assert proc.poll() is None, "progress must appear while transfer is blocked"
                                observed = True; break
                        assert observed, lines
                        released.set()
                    code = proc.wait(timeout=25)
                    stdout = proc.stdout.read()
                    reader.join(timeout=5)
                finally:
                    released.set()
                    if proc.poll() is None: proc.kill(); proc.wait(timeout=5)
                    if master is not None: os.close(master)
                stderr = "".join(lines)
                assert code == (0 if mode == "unknown" else 2), (mode, code, stdout, stderr)
                assert (home / "installed.json").read_bytes() == original
                assert "private-token-do-not-log" not in stderr and "\x1b" not in stderr
                if mode in ["human", "terminal"]:
                    assert any(f"{n}%" in stderr for n in range(40, 51)) and "100%" in stderr and "MB/s" in stderr, stderr
                    assert "approved plan pins" in stdout + stderr, (stdout, stderr)
                if mode == "terminal":
                    assert stderr.count("\rDownloading") >= 3, stderr
                    assert "Receiving data; integrity is checked after transfer" not in stderr, stderr
                if mode == "unknown": assert "%" not in stderr and "ETA" not in stderr, stderr
                if mode == "json": json.loads(stdout); assert stderr == "", stderr
                if mode == "off": assert "MB received" not in stderr and "ETA" not in stderr
                cases.append(dict(case=mode, exit_code=code, live_progress_before_completion=observed, approved_state_preserved=True))
        finally:
            released.set(); server.shutdown(); server.server_close(); thread.join(timeout=5)
    return dict(passed=True, platform=sys.platform, cases=cases, scope="Real CLI, bounded local HTTP transfer and private synthetic state; no real user installation or remote release changes")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = verify(args.cli.resolve())
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    print("PASS live known/unknown downloads, JSON/quiet output and integrity refusal")
