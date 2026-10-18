"""Native PS5.1 visual-state tests of the helpers embedded by the real generator."""
from pathlib import Path
import argparse
import hashlib
import http.server
import importlib.util
import json
import os
import subprocess
import tempfile
import threading
import time

from terminal_ux.capture import Capture, activity_frames

ROOT = Path(__file__).resolve().parents[1]


def verify(out):
    assert os.name == "nt", "native Windows required"
    spec = importlib.util.spec_from_file_location("gen", ROOT / "packaging/oneline/generate_bootstrappers.py")
    gen = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gen)
    payload = b"x" * 1_048_576
    released = threading.Event()
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass
        def do_GET(self):
            self.send_response(500 if self.path == "/error" else 200)
            if self.path != "/unknown":
                self.send_header("Content-Length", str(len(payload)))
            self.send_header("Connection", "close")
            self.end_headers()
            try:
                self.wfile.write(payload[:len(payload)//2])
                self.wfile.flush()
                if self.path == "/resize": released.wait(15)
                else: time.sleep(1.6)
                self.wfile.write(payload[len(payload)//2:])
            except (BrokenPipeError, ConnectionResetError):
                pass
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    sums = "".join("0" * 64 + "  axiom-1.2.3-" + suffix + "\n" for suffix in ["windows-x64.zip", "linux-x64.tar.gz", "macos-x64.tar.gz"])
    generated = gen.generate("v1.2.3", sums)
    cases = []
    evidence = out.parent / (out.stem + "-frames")
    try:
        with tempfile.TemporaryDirectory(prefix="axiom native visual ") as directory:
            work = Path(directory)
            ps = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
            for action in ["install", "update"]:
                script_text = generated[action + ".ps1"].decode()
                helper = script_text.split("# BEGIN AXIOM TERMINAL RENDERER\n", 1)[1].split("# END AXIOM TERMINAL RENDERER", 1)[0]
                for mode in ["known", "unknown", "error", "plain", "off", "ci", "dumb", "no-color", "resize", "redirected"]:
                    script = work / "probe.ps1"
                    destination = work / "archive.bin"
                    destination.unlink(missing_ok=True)
                    released.clear()
                    endpoint = mode if mode in ["known", "unknown", "error", "resize"] else "known"
                    source = f"$ErrorActionPreference='Stop'\n$AxiomAction='{action}'\n" + helper
                    source += f"""
try {{
    $ui.Begin('Downloading','downloading v1.2.3 (axiom-1.2.3-windows-x64.zip)')
    Receive-AxiomArchive -Uri 'http://127.0.0.1:{server.server_port}/{endpoint}' -Destination '{str(destination).replace(chr(39), chr(39)*2)}'
    $ui.Begin('Verifying','verifying archive SHA-256')
    Start-Sleep -Milliseconds 400
    $ui.Begin('Extracting','extracting verified archive')
    Start-Sleep -Milliseconds 400
}} finally {{ $ui.Dispose() }}
Write-Output 'fixture complete'
"""
                    script.write_text(source, encoding="utf-8", newline="\n")
                    env = {k: v for k, v in os.environ.items() if not k.startswith("AXIOM_") and k.upper() not in ["CI", "PSMODULEPATH", "PSMODULEANALYSISCACHEPATH", "NO_COLOR"]}
                    env.update(TERM="dumb" if mode == "dumb" else "xterm-256color", AXIOM_PROGRESS=mode if mode in ["plain", "off"] else "auto")
                    env["PSModuleAnalysisCachePath"] = str(work / "module-cache")
                    env["PSModulePath"] = str(Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/Modules")
                    if mode == "ci":
                        env["CI"] = "1"
                    if mode == "no-color":
                        env["NO_COLOR"] = "1"
                    argv = [str(ps), "-NoProfile", "-File", str(script)]
                    if mode in ["plain", "off", "ci", "dumb", "redirected"]:
                        result = subprocess.run(argv, env=env, capture_output=True, timeout=30)
                        assert result.returncode == 0, result.stderr
                        assert b"\x1b" not in result.stderr
                        assert len(result.stderr.splitlines()) == (0 if mode == "off" else 3)
                        cases.append(dict(case=action + "-" + mode, exit_code=0, deterministic_plain=True))
                        continue
                    c = Capture(argv, env=env, cwd=work)
                    try:
                        if mode == "resize":
                            c.until(tuple(f"{n}%".encode() for n in range(1, 51)))
                            for width in [45, 35, 90]:
                                c.resize(width)
                                deadline = time.monotonic() + 2
                                while True:
                                    lines = [s.rstrip() for s in c.display() if s.strip()]
                                    if len(lines) == 2 and all(len(s) < width for s in lines): break
                                    assert time.monotonic() < deadline, (action, width, lines)
                                    time.sleep(.025)
                            released.set()
                        code = c.wait(30)
                        c.save(evidence, action + "-" + mode)
                    finally:
                        released.set()
                        c.save(evidence, action + "-" + mode)
                        if c.alive():
                            c.close()
                    assert code == (1 if mode == "error" else 0), (mode, code, bytes(c.raw))
                    frames = activity_frames(c.raw)
                    assert frames and all(len(f["lines"]) == 2 for f in frames), frames
                    assert len({tuple(i for i, _ in f["lines"]) for f in frames}) == 1
                    assert len({s[0] for f in frames for _, s in f["details"]}) >= 2
                    assert b"100%" not in c.raw
                    if mode == "unknown":
                        assert b"%" not in c.raw and b"ETA" not in c.raw
                    elif mode != "error":
                        assert any(f"{n}%".encode() in c.raw for n in range(40, 51)), bytes(c.raw)
                    if mode != "error":
                        assert hashlib.sha256(destination.read_bytes()).digest() == hashlib.sha256(payload).digest()
                        assert all(label.encode() in c.raw for label in ["Downloading", "Verifying", "Extracting"])
                    cases.append(dict(case=action + "-" + mode, exit_code=code, visual_frames=len(frames), fixed_two_rows=True, rotating_spinner=True))
                # Exercise the generated bootstrap's own finally/exit contract,
                # rather than inferring cancellation from the renderer helper.
                home = work / (action + "-cancel-home"); home.mkdir()
                data = home / "data"; data.mkdir()
                if action == "update":
                    installed = data / "Axiom"; (installed / "bin").mkdir(parents=True)
                    (installed / "installed.json").write_bytes(b"{}")
                    (installed / "bin/axm.exe").write_bytes(b"owned unused marker")
                sums_ok = "".join(hashlib.sha256(payload).hexdigest() + "  axiom-1.2.3-" + suffix + "\n" for suffix in ["windows-x64.zip", "linux-x64.tar.gz", "macos-x64.tar.gz"])
                full = gen.generate("v1.2.3", sums_ok, f"http://127.0.0.1:{server.server_port}")[action + ".ps1"]
                target = home / (action + ".ps1"); target.write_bytes(full)
                env.update(HOME=str(home), USERPROFILE=str(home), LOCALAPPDATA=str(data), AXIOM_CLI_TEST_ALLOW_ELEVATED="1", AXIOM_PROGRESS="auto")
                env["PATH"] = os.pathsep.join(p for p in env["PATH"].split(os.pathsep) if not any((Path(p) / n).is_file() for n in ["axm.exe", "axiom-cli.exe"]))
                for phase in ["Downloading", "Verifying", "Extracting"]:
                    wrapper = home / "wrapper.ps1"
                    block = ""
                    if phase == "Verifying":
                        block = "function Get-FileHash { param($LiteralPath,$Algorithm) Start-Sleep -Seconds 10; Microsoft.PowerShell.Utility\\Get-FileHash @PSBoundParameters }\n"
                    elif phase == "Extracting":
                        block = "function Expand-Archive { param($LiteralPath,$DestinationPath) Start-Sleep -Seconds 10; Microsoft.PowerShell.Archive\\Expand-Archive @PSBoundParameters }\n"
                    wrapper.write_text(block + "& '" + str(target).replace("'", "''") + "' -Yes\nexit $LASTEXITCODE\n", encoding="utf-8", newline="\n")
                    c = Capture([str(ps), "-NoProfile", "-File", str(wrapper)], env=env, cwd=home)
                    try:
                        c.until(phase.encode(), timeout=30); c.send(b"\x03")
                        code = c.wait(20); c.save(evidence, action + "-cancel-" + phase.lower())
                    finally:
                        if c.alive(): c.close()
                    assert code == 130 and not c.screen.cursor.hidden, (action, phase, code, bytes(c.raw))
                    cases.append(dict(case=action + "-cancel-" + phase.lower(), exit_code=code, generated_finally=True, controlled_phase_delay=phase != "Downloading"))
                # IEX shares the caller's scope: failed initialization must restore
                # its preference and leave the original error catchable.
                wrapper.write_text("function Add-Type { throw 'controlled-renderer-init-failure' }\n$ProgressPreference='Continue'\n$caught=$null\ntry { Invoke-Expression ([IO.File]::ReadAllText('" + str(target).replace("'", "''") + "')) } catch { $caught=$_.Exception.Message }\nif ($ProgressPreference -ne 'Continue' -or $caught -notlike '*controlled-renderer-init-failure*') { throw 'initialization cleanup failed' }\nWrite-Output 'initializer-error-retained; preference-restored'\n", encoding="utf-8", newline="\n")
                result = subprocess.run([str(ps), "-NoProfile", "-File", str(wrapper)], env=env, capture_output=True, timeout=30)
                assert result.returncode == 0 and b"initializer-error-retained; preference-restored" in result.stdout, result
                cases.append(dict(case=action + "-initializer-failure", original_error_retained=True, preference_restored=True))
    finally:
        server.shutdown()
        server.server_close()
    return dict(passed=True, platform="native Windows PowerShell 5.1 / ConPTY", cases=cases, scope="Generated embedded helper with bounded loopback transfer and controlled verification/extraction durations; no installation. GUI, full bootstrap/upgrade and cancellation gates are separate.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = verify(args.out.resolve())
    args.out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
    print("PASS native PS5.1 generated-helper visual frames, measured/unknown bytes and plain modes")
