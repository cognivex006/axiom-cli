"""Source-bound native renderer frames, prompts, modes, resizing and cancellation."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from capture import Capture, Screen, activity_frames


def verify(probe, out):
    cases = []
    # Cursor queries are a PTY protocol, not read-chunk boundaries. A wrong
    # response can move ConPTY over ordinary output during resize.
    import pyte
    screen = Screen(90, 24)
    replies = []
    screen.write_process_input = replies.append
    stream = pyte.ByteStream(screen)
    stream.feed(b"\x1b[4;7H\x1b[")
    assert not replies
    stream.feed(b"6nabc\x1b[6n\x1b[2;3H\x1b[6ntrailing")
    assert replies == ["\x1b[4;7R", "\x1b[4;10R", "\x1b[2;3R"], replies
    cases.append(dict(case="split-and-coalesced-cursor-queries", replies_at_query_position=True))
    with tempfile.TemporaryDirectory(prefix="axiom terminal acceptance ") as directory:
        work = Path(directory)
        base = {k: v for k, v in os.environ.items() if not k.startswith("AXIOM_") and k not in ["CI", "NO_COLOR"]}
        base.update(TERM="xterm-256color", TEMP=str(work), TMP=str(work), TMPDIR=str(work))
        for mode in ["apply-cancel", "apply-cancel-json", "apply-timeout"] + (["apply-term"] if os.name != "nt" else ["apply-cancel-inherited-ignore"]):
            marker = work / mode; marker.mkdir()
            argv = [str(probe), "install", "--owned-timeout" if mode == "apply-timeout" else "--owned"]
            if mode == "apply-cancel-json": argv.append("--json")
            if mode == "apply-cancel-inherited-ignore": argv.append("--inherited-ignore")
            c = Capture(argv, env=dict(base, AXIOM_UX_MARKER_DIR=str(marker)), cwd=work)
            handles = []
            try:
                deadline = time.monotonic() + 8
                while not all((marker / name).exists() for name in ["worker.pid", "descendant.pid"]):
                    assert c.alive() and time.monotonic() < deadline, (mode, bytes(c.raw))
                    time.sleep(.025)
                pids = [int((marker / name).read_text()) for name in ["worker.pid", "descendant.pid"]]
                if os.name == "nt":
                    import ctypes
                    native = ctypes.WinDLL("kernel32", use_last_error=True)
                    native.OpenProcess.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
                    native.OpenProcess.restype = ctypes.c_void_p
                    native.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
                    native.CloseHandle.argtypes = [ctypes.c_void_p]
                    handles = [native.OpenProcess(0x100000, False, pid) for pid in pids]
                    assert all(handles), (mode, pids)
                if mode == "apply-term":
                    import signal
                    os.kill(c.proc.pid, signal.SIGTERM)
                elif mode != "apply-timeout": c.send(b"\x03")
                code = c.wait(15); c.save(out.parent / (out.stem + "-frames"), mode)
                assert code == (0 if mode == "apply-timeout" else 143 if mode == "apply-term" else 130), (mode, code, bytes(c.raw))
                assert not c.screen.cursor.hidden
                if os.name == "nt":
                    assert all(native.WaitForSingleObject(handle, 2000) == 0 for handle in handles), (mode, pids)
                else:
                    deadline = time.monotonic() + 3
                    for pid in pids:
                        while True:
                            try: os.kill(pid, 0)
                            except ProcessLookupError: break
                            assert time.monotonic() < deadline, (mode, "owned descendant remains", pid)
                            time.sleep(.05)
                cases.append(dict(case=mode, exit_code=code, worker_and_grandchild_exited=True, retained_native_identity=True))
            finally:
                c.save(out.parent / (out.stem + "-frames"), mode)
                if os.name == "nt":
                    for handle in handles:
                        if handle: native.CloseHandle(handle)
                if c.alive(): c.close()
        for mode in ["known", "unknown", "failed", "no-color", "resize", "cancel", "prompt"]:
            argv = [str(probe), "install", "--visual" if mode == "prompt" else "--transfer"]
            if mode == "unknown": argv.append("--unknown")
            if mode == "failed": argv.append("--fail")
            if mode == "resize": argv.extend(["--resize", "--preface"])
            env = dict(base)
            if mode == "no-color": env["NO_COLOR"] = "1"
            c = Capture(argv, env=env, cwd=work)
            try:
                c.until(b"axiom install:")
                if mode == "prompt":
                    c.until(b"Proceed? [Y/n]")
                    assert not c.screen.cursor.hidden
                    c.send(b"Y\r")
                elif mode == "resize":
                    for width in [45, 90, 35, 70, 45, 90]:
                        c.resize(width)
                        deadline = time.monotonic() + 2
                        while True:
                            display = c.display()
                            assert all(any(f"Existing output sentinel {n}" in line for line in display) for n in range(1,4)), display
                            first = next((i for i,s in enumerate(display) if "[" in s and "]" in s and ("%" in s or "indeterminate" in s)), None)
                            nonempty = [s.rstrip() for s in display[first:] if s.strip()] if first is not None else []
                            if len(nonempty) == 2 and all(len(s) < width for s in nonempty): break
                            assert time.monotonic() < deadline, (width, nonempty)
                            time.sleep(.025)
                        assert "indeterminate" in nonempty[0] or "%" in nonempty[0], nonempty
                        time.sleep(.15)
                elif mode == "cancel":
                    c.send(b"\x03")
                code = c.wait(20)
                c.save(out.parent / (out.stem + "-frames"), mode)
            finally:
                c.save(out.parent / (out.stem + "-frames"), mode)
                if c.alive(): c.close()
            assert code == (130 if mode == "cancel" else 7 if mode == "failed" else 0), (mode, code, bytes(c.raw))
            assert not c.screen.cursor.hidden, (mode, bytes(c.raw))
            if mode == "resize":
                assert all(any(f"Existing output sentinel {n}" in line for line in c.display()) for n in range(1,4)), c.display()
            if mode == "prompt":
                display = "\n".join(c.display())
                assert "Plan: controlled terminal fixture" in display and "Proceed? [Y/n]" in display and "Fixture complete: Y" in display, display
            if mode not in ["resize", "cancel"]:
                frames = activity_frames(c.raw)
                before_prompt = [f for f in frames if not any("Plan:" in s for _, s in f["lines"])]
                assert before_prompt and all(len(f["lines"]) == 2 for f in before_prompt), before_prompt
                assert len({tuple(i for i, _ in f["lines"]) for f in before_prompt}) == 1
                assert len({s[0] for f in before_prompt for _, s in f["details"]}) >= 3
                if mode not in ["unknown", "prompt"]:
                    assert b"99%" in c.raw and b"100%" not in c.raw
                if mode == "unknown": assert b"%" not in c.raw and b"ETA" not in c.raw
            cases.append(dict(case=mode, exit_code=code, cursor_restored=True, native_capture=True))
        for mode in ["plain", "off", "json", "ci", "dumb", "stdout-only", "stderr-only"]:
            argv = [str(probe), "install", "--transfer"]
            env = dict(base, AXIOM_PROGRESS=mode if mode in ["plain", "off"] else "auto")
            if mode == "json": argv.append("--json")
            if mode == "ci": env["CI"] = "1"
            if mode == "dumb": env["TERM"] = "dumb"
            transport_negotiation = False
            # One redirected stream must disable live output even if the other is a PTY.
            if mode in ["stdout-only", "stderr-only"] and os.name == "nt":
                target = work / (mode + ".log")
                c = Capture([str(probe), "--redirect-stdout" if mode == "stdout-only" else "--redirect-stderr"], env=dict(env, AXIOM_UX_REDIRECT_FILE=str(target)), cwd=work)
                try:
                    code = c.wait(15)
                finally:
                    c.save(out.parent / (out.stem + "-frames"), mode)
                    if c.alive(): c.close()
                machine = target.read_bytes()
                assert b"\x1b" not in machine, (mode, machine)
                terminal = bytes(c.raw)
                # ConPTY itself emits a leading capability/focus/Win32-input
                # negotiation, also reproduced with plain cmd echo (no Axiom).
                # Never strip arbitrary ANSI, a middle sequence, or file output.
                preambles = [b"\x1b[1t", b"\x1b[c", b"\x1b[?1004h", b"\x1b[?9001h"]
                while any(terminal.startswith(prefix) for prefix in preambles):
                    prefix = next(p for p in preambles if terminal.startswith(p))
                    terminal = terminal[len(prefix):]
                    transport_negotiation = True
                data = terminal + machine
                assert code == 0
            elif mode in ["stdout-only", "stderr-only"]:
                import pty
                master, slave = pty.openpty()
                try:
                    result = subprocess.run(argv, env=env, stdout=subprocess.PIPE if mode == "stdout-only" else slave, stderr=slave if mode == "stdout-only" else subprocess.PIPE, timeout=15)
                    os.close(slave)
                    captured = os.read(master, 65536)
                finally: os.close(master)
                data = captured + (result.stdout or b"") + (result.stderr or b"")
            else:
                result = subprocess.run(argv, env=env, capture_output=True, timeout=15)
                data = result.stdout + result.stderr
            assert (code if os.name == "nt" and mode in ["stdout-only", "stderr-only"] else result.returncode) == 0 and b"\x1b" not in data and b"elapsed" not in data, (mode, data)
            if mode in ["json", "off"]: assert data == b'{"verified":true}\n' or data == b'{"verified":true}\r\n', data
            cases.append(dict(case=mode, exit_code=0, controls_absent=True, single_stream_tty_exercised=(mode in ["stdout-only", "stderr-only"]), conpty_transport_preamble=transport_negotiation))
    return dict(passed=True, platform=sys.platform, cases=cases, scope="Production Rust module in a source-built fixture; not release upgrade or GUI acceptance")


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--probe", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    args = ap.parse_args()
    report = verify(args.probe.resolve(), args.out.resolve())
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")
    print("PASS native two-row frames, transfer truth, prompt, resize, cancellation and progress modes")
