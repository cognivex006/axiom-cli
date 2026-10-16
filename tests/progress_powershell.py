"""Run the actual embedded bootstrap transfer helper on native Windows PowerShell."""
from pathlib import Path
import argparse
import hashlib
import http.server
import json
import os
import re
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]

def verify():
    assert os.name == "nt"
    helpers = [re.search(r"function Receive-AxiomArchive[\s\S]+?(?=\n# Failures)", (ROOT / f"installers/oneline/{name}.ps1.in").read_text(encoding="utf-8")).group() for name in ["install", "update"]]
    assert helpers[0] == helpers[1]
    data = b"x" * 1_048_576
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            self.send_response(200)
            if self.path == "/known": self.send_header("Content-Length", str(len(data)))
            self.send_header("Connection", "close"); self.end_headers()
            self.wfile.write(data[:len(data)//2]); self.wfile.flush(); time.sleep(1.5)
            self.wfile.write(data[len(data)//2:])
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    cases = []
    try:
        with tempfile.TemporaryDirectory(prefix="axiom bootstrap progress ") as temporary:
            root = Path(temporary).resolve()
            ps = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
            # Exercise the actual console handoff with inherited stdin and exact exit codes.
            stub = root / "updater.exe"
            compiler = root / "compile.ps1"
            compiler.write_text("""$ErrorActionPreference='Stop'
Add-Type -OutputType ConsoleApplication -OutputAssembly '""" + str(stub).replace("'", "''") + """' -TypeDefinition @'
using System;
public class Updater {
  public static int Main(string[] args) {
    if (args.Length > 1) {
      Console.WriteLine("Proceed?");
      Console.WriteLine("approval:" + Console.ReadLine());
      System.Threading.Thread.Sleep(600);
    }
    return int.Parse(args[0]);
  }
}
'@
""", encoding="utf-8")
            subprocess.run([str(ps), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(compiler)], check=True, capture_output=True, timeout=30)
            template = (ROOT / "installers/oneline/update.ps1.in").read_text(encoding="utf-8")
            monitor = template.split("    # Keep the installed updater's console/input and single approval unchanged.\n", 1)[1].split("    $global:LASTEXITCODE = $code", 1)[0]
            for code, interactive in [(0, False), (7, False), (0, True), (7, True)]:
                script = root / "handoff.ps1"
                script.write_text("$ErrorActionPreference='Stop'\n$cli='" + str(stub).replace("'", "''") + "'\n$cliArgs=@('" + str(code) + "'" + (",'prompt'" if interactive else "") + ")\n" + monitor + "\nif ($null -eq $code) { throw 'lost child exit code' }\nWrite-Output ('result:' + $code)\n", encoding="utf-8")
                p = subprocess.run([str(ps), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)], input="Y\n", env=dict(os.environ, AXIOM_PROGRESS="plain"), capture_output=True, text=True, timeout=15)
                assert p.returncode == 0 and f"result:{code}" in p.stdout, (p.stdout, p.stderr)
                if interactive: assert "approval:Y" in p.stdout, (p.stdout, p.stderr)
                cases.append(dict(case="console-handoff", child_exit_code=code, inherited_approval_input=interactive, exact_exit_code_preserved=True))
            for mode in ["known", "unknown"]:
                script = root / (mode + ".ps1"); output = root / (mode + ".bin")
                prelude = '''$ErrorActionPreference='Stop'
$script:frames = New-Object 'Collections.Generic.List[object]'
function Write-Progress {
param($Id,$Activity,$Status,[int]$PercentComplete=-1,$CurrentOperation,[switch]$Completed)
$script:frames.Add([pscustomobject]@{percent=$PercentComplete;status=$Status;completed=[bool]$Completed})
}
'''
                script.write_text(prelude + helpers[0] + f"\nReceive-AxiomArchive -Uri 'http://127.0.0.1:{server.server_port}/{mode}' -Destination '{str(output).replace(chr(39), chr(39)*2)}'\n" + "ConvertTo-Json -InputObject @($script:frames.ToArray()) -Depth 4 -Compress\n", encoding="utf-8", newline="\n")
                env = dict(os.environ, AXIOM_PROGRESS="plain")
                ps = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
                p = subprocess.run([str(ps), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)], env=env, capture_output=True, text=True, timeout=30)
                assert p.returncode == 0, p.stdout + p.stderr
                assert hashlib.sha256(output.read_bytes()).digest() == hashlib.sha256(data).digest()
                frames = json.loads(p.stdout); active = [f for f in frames if not f["completed"]]
                assert active
                if mode == "known":
                    assert any(40 <= f["percent"] <= 50 for f in active), active
                    assert any("ETA ~" in (f["status"] or "") for f in active), active
                else: assert all(f["percent"] == -1 for f in active), active
                cases.append(dict(case=mode, native_powershell=True, exact_download_bytes=True, observed_frames=len(active)))
    finally:
        server.shutdown(); server.server_close(); thread.join(timeout=5)
    return dict(passed=True, cases=cases, scope="Native PowerShell transfer and console handoff, private temporary files and loopback server; no installation")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.write_text(json.dumps(verify(), indent=2) + "\n", encoding="utf-8", newline="\n")
    print("PASS native PowerShell byte progress, ETA, inherited approval input and exact child exit codes")
