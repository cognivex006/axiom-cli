"""Native PTY/ConPTY capture and explicit DEC-register screen reconstruction."""
from __future__ import annotations

import copy
import os
import queue
import socket
import subprocess
import threading
import time
from pathlib import Path

import pyte


class Screen(pyte.Screen):
    """DEC save/restore is persistent; pyte's default implementation pops a stack."""

    def save_cursor(self):
        self.savepoints.clear()
        super().save_cursor()

    def restore_cursor(self):
        point = copy.deepcopy(self.savepoints[-1]) if self.savepoints else None
        super().restore_cursor()
        if point is not None:
            self.savepoints.append(point)


class Capture:
    def __init__(self, argv, *, env, cwd, columns=90, rows=24):
        self.raw = bytearray()
        self.chunks = queue.Queue()
        self.screen = Screen(columns, rows)
        self.stream = pyte.ByteStream(self.screen)
        self.frames = []
        self.closed = False
        if os.name == "nt":
            from winpty import PtyProcess
            self.proc = PtyProcess.spawn([str(a) for a in argv], env=env, cwd=str(cwd), dimensions=(rows, columns), backend="0")
            self.proc.fileobj.settimeout(0.1)
        else:
            import fcntl
            import pty
            import struct
            import termios
            self.master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
            self.proc = subprocess.Popen(argv, env=env, cwd=cwd, stdin=slave, stdout=slave, stderr=slave, start_new_session=True, preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0))
            os.close(slave)
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def alive(self):
        return self.proc.isalive() if os.name == "nt" else self.proc.poll() is None

    def _read(self):
        quiet = time.monotonic()
        while not self.closed:
            try:
                data = self.proc.fileobj.recv(65536) if os.name == "nt" else os.read(self.master, 65536)
            except socket.timeout:
                if not self.alive() and time.monotonic() - quiet > 0.3:
                    break
                continue
            except OSError:
                break
            if not data:
                break
            quiet = time.monotonic()
            self.raw.extend(data)
            self.stream.feed(data)
            self.frames.append(dict(at=quiet, lines=[s.rstrip() for s in self.screen.display], cursor=[self.screen.cursor.x, self.screen.cursor.y], hidden=self.screen.cursor.hidden))
            self.chunks.put(data)
            if b"\x1b[6n" in data:
                self.send(b"\x1b[1;1R")

    def send(self, data):
        if os.name == "nt":
            self.proc.write(data.decode("utf-8"))
        else:
            os.write(self.master, data)

    def until(self, needle, timeout=30):
        needles = needle if isinstance(needle, tuple) else (needle,)
        deadline = time.monotonic() + timeout
        while not any(item in self.raw for item in needles):
            if time.monotonic() >= deadline or not self.alive():
                raise AssertionError((needle, bytes(self.raw[-4096:]).decode(errors="replace")))
            time.sleep(0.025)

    def resize(self, columns, rows=24):
        if os.name == "nt":
            self.proc.setwinsize(rows, columns)
        else:
            import fcntl
            import struct
            import termios
            fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        self.screen.resize(rows, columns)

    def wait(self, timeout=30):
        deadline = time.monotonic() + timeout
        while self.alive():
            if time.monotonic() >= deadline:
                self.close()
                raise TimeoutError("owned terminal fixture timed out")
            time.sleep(0.05)
        self.thread.join(2)
        code = self.proc.exitstatus if os.name == "nt" else self.proc.returncode
        self.close()
        return code

    def close(self):
        self.closed = True
        if os.name == "nt":
            # winpty marks dead children closed before closing its socket.
            self.proc.closed = False
            self.proc.close(force=True)
        else:
            if self.proc.poll() is None:
                self.proc.kill()
                self.proc.wait()
            os.close(self.master)

    def save(self, directory: Path, name: str):
        import json
        directory.mkdir(parents=True, exist_ok=True)
        (directory / (name + ".vt")).write_bytes(self.raw)
        (directory / (name + ".json")).write_text(json.dumps(dict(frames=self.frames, final_frame=self.screen.display), indent=2) + "\n", encoding="utf-8", newline="\n")


def activity_frames(raw, columns=90, rows=24):
    """Inspect completed two-row repaints, including all intermediate stage changes."""
    screen = Screen(columns, rows)
    stream = pyte.ByteStream(screen)
    frames = []
    chunks = bytes(raw).split(b"\x1b8")
    for index, chunk in enumerate(chunks):
        stream.feed(chunk)
        if index < len(chunks) - 1:
            stream.feed(b"\x1b8")
        lines = [(i, text.rstrip()) for i, text in enumerate(screen.display) if text.strip()]
        details = [(i, text) for i, text in lines if len(text) > 2 and text[0] in "|/-\\" and " axiom " in text]
        if details:
            frames.append(dict(lines=lines, details=details))
    return frames
