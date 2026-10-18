"""Native regression for live verified-worker stderr, captured JSON and bootstrap refusal."""
from pathlib import Path
import argparse,hashlib,http.server,importlib.util,io,json,os,queue,subprocess,sys,tarfile,tempfile,threading,time,zipfile
ROOT=Path(__file__).resolve().parents[1]

def verify(evidence=None):
    cases=[]
    with tempfile.TemporaryDirectory(prefix="axiom progress parity ") as directory:
        temp=Path(directory); exe=temp/("worker.exe" if os.name=="nt" else "worker")
        subprocess.run(["cargo", "+1.85.0", "build", "--locked", "--example", "terminal-ux-probe"], cwd=ROOT, check=True, capture_output=True, timeout=180)
        import shutil
        shutil.copy2(ROOT / "target/debug/examples" / ("terminal-ux-probe.exe" if os.name == "nt" else "terminal-ux-probe"), exe)
        for mode in ["plain","json","off","dry-run","failed","diagnostic"]:
            env={k:v for k,v in os.environ.items() if not k.startswith("AXIOM_") and k.upper() not in ["PSMODULEPATH", "PSMODULEANALYSISCACHEPATH"]}
            env["AXIOM_PROGRESS"]="off" if mode=="off" else "" if mode=="terminal" else "plain"
            args=[str(exe)]+({"json":["--json"],"dry-run":["--dry-run"],"failed":["--fail"],"diagnostic":["--diagnostic"]}.get(mode,[]))
            master=slave=None
            if mode=="terminal":
                import pty
                master,slave=pty.openpty()
            proc=subprocess.Popen(args,env=env,stdout=subprocess.PIPE,stderr=slave if slave is not None else subprocess.PIPE)
            if slave is not None:os.close(slave)
            chunks=[];seen=queue.Queue()
            def read():
                while True:
                    try: chunk=os.read(master,4096) if master is not None else proc.stderr.read(1)
                    except OSError:return
                    if not chunk:return
                    chunks.append(chunk);seen.put(chunk)
            reader=threading.Thread(target=read,daemon=True);reader.start()
            live=False
            try:
                if mode in ["plain","failed","terminal"]:
                    sample=b""; deadline=time.monotonic()+1.8
                    while b"Fixture runtime" not in sample and time.monotonic()<deadline:
                        sample+=seen.get(timeout=max(.01,deadline-time.monotonic()))
                    assert b"Fixture runtime" in sample and proc.poll() is None,(mode,sample)
                    live=True
                proc.wait(timeout=10);stdout=proc.stdout.read();reader.join(timeout=3)
            finally:
                if proc.poll() is None:proc.kill();proc.wait(timeout=5)
                if master is not None:os.close(master)
            stderr=b"".join(chunks).decode()
            assert json.loads(stdout)=={"worker":True},(mode,stdout)
            assert proc.returncode==(7 if mode=="failed" else 0),(mode,stderr)
            if mode in ["json","off","dry-run"]:assert stderr=="",(mode,stderr)
            elif mode=="terminal":assert "\rPreparing MCP runtime" in stderr and "[Done]" in stderr,stderr
            else:
                assert "elapsed" not in stderr and "\x1b" not in stderr
                assert "Fixture runtime" in stderr
                if mode == "failed": assert "failed:" in stderr
                if mode == "diagnostic": assert stderr.count("fixture diagnostic: retained[2J") == 1
            cases.append(dict(case="worker-"+mode,live_before_completion=live,exact_exit=True,json_stdout_isolated=True))

        # The bootstrap must run its verified downloaded worker, never the old CLI.
        stub=temp/"bootstrap_stub.rs"
        stub.write_text('fn main(){println!("verified-worker:{}:{}",std::env::var("AXIOM_CLI_INSTALL_ROOT").unwrap_or_default(),std::env::args().skip(1).collect::<Vec<_>>().join(" "));std::process::exit(7)}')
        native=temp/("axiom-cli.exe" if os.name=="nt" else "axiom-cli")
        subprocess.run(["rustc","+1.85.0",str(stub),"-o",str(native)],check=True,capture_output=True,timeout=90)
        data=native.read_bytes(); suffix="windows-x64.zip" if os.name=="nt" else "macos-x64.tar.gz" if sys.platform=="darwin" else "linux-x64.tar.gz"
        archive=io.BytesIO()
        if os.name=="nt":
            with zipfile.ZipFile(archive,"w",zipfile.ZIP_DEFLATED) as z:z.writestr(native.name,data)
        else:
            with tarfile.open(fileobj=archive,mode="w:gz") as z:
                item=tarfile.TarInfo(native.name);item.size=len(data);item.mode=0o755;z.addfile(item,io.BytesIO(data))
        body=archive.getvalue()
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self,*_):pass
            def do_GET(self):
                self.send_response(200);self.send_header("Content-Length",str(len(body)));self.end_headers();self.wfile.write(body)
        server=http.server.ThreadingHTTPServer(("127.0.0.1",0),Handler);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        try:
            spec=importlib.util.spec_from_file_location("bootstrap_generator",ROOT/"packaging/oneline/generate_bootstrappers.py");gen=importlib.util.module_from_spec(spec);spec.loader.exec_module(gen)
            home=temp/"private home";install=home/("data/Axiom" if os.name=="nt" else "data/axiom"); (install/"bin/graphd-engine").mkdir(parents=True)
            record=install/"installed.json";record.write_bytes(b"{}")
            old=install/"bin"/("axm.exe" if os.name=="nt" else "axm");old.write_bytes(b"old-updater-must-not-run");old.chmod(0o755)
            env={k:v for k,v in os.environ.items() if not k.startswith("AXIOM_") and k.upper() not in ["PSMODULEPATH", "PSMODULEANALYSISCACHEPATH"]}
            env.update(HOME=str(home),USERPROFILE=str(home),LOCALAPPDATA=str(home/"data"),XDG_DATA_HOME=str(home/"data"),AXIOM_CLI_TEST_ALLOW_ELEVATED="1",AXIOM_PROGRESS="off")
            if os.name == "nt":
                # PowerShell 7's inherited module path must not hide the native 5.1 modules.
                env["PSModulePath"] = str(Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/Modules")
                env["PSModuleAnalysisCachePath"] = str(temp / "powershell-module-cache")
            # Avoid discovering the caller's installed CLI. Native helpers retain normal PATH.
            env["PATH"]=os.pathsep.join(p for p in env["PATH"].split(os.pathsep) if not any((Path(p)/name).is_file() for name in ["axm","axm.exe","axiom-cli","axiom-cli.exe"]))
            for good in [True,False]:
                sums="\n".join((hashlib.sha256(body).hexdigest() if good and name==suffix else "0"*64)+"  axiom-1.2.3-"+name for name in ["windows-x64.zip","linux-x64.tar.gz","macos-x64.tar.gz"])
                scripts=gen.generate("v1.2.3",sums,f"http://127.0.0.1:{server.server_port}")
                name="update.ps1" if os.name=="nt" else "update.sh";script=temp/name;script.write_bytes(scripts[name])
                if os.name=="nt":words=[str(Path(os.environ["SystemRoot"])/"System32/WindowsPowerShell/v1.0/powershell.exe"),"-NoProfile","-File",str(script),"-Yes"]
                else:words=["sh",str(script),"--yes"]
                result=subprocess.run(words,env=env,capture_output=True,text=True,timeout=90)
                expected_code = 7 if good else 1
                assert result.returncode == expected_code, (result.stdout, result.stderr)
                if good:
                    assert f"verified-worker:{install}:update --yes" in result.stdout,(result.stdout,result.stderr)
                else:assert "SHA-256 mismatch" in result.stdout+result.stderr and "verified-worker:" not in result.stdout,(result.stdout,result.stderr)
                assert record.read_bytes()==b"{}" and old.read_bytes()==b"old-updater-must-not-run"
                cases.append(dict(case="bootstrap-verified-worker" if good else "bootstrap-bad-digest",old_installation_preserved=True))
                if good and os.name == "nt":
                    sys.path.insert(0, str(ROOT / "tests/terminal_ux"))
                    from capture import Capture
                    for action in ["install", "update"]:
                        target = temp / (action + ".ps1"); target.write_bytes(scripts[target.name])
                        wrapper = temp / (action + "-interactive.ps1")
                        wrapper.write_text("$ProgressPreference='Continue'\ntry { Invoke-Expression ([IO.File]::ReadAllText('" + str(target).replace("'", "''") + "')) } catch { Write-Output ('caught:'+$_.Exception.Message) }\nWrite-Output ('interactive-alive:'+$LASTEXITCODE+':'+$ProgressPreference)\n", encoding="utf-8", newline="\n")
                        folder = evidence.parent / (evidence.stem + "-lifecycle") if evidence is not None else temp / "lifecycle"
                        folder.mkdir(parents=True, exist_ok=True)
                        interactive = Capture([words[0], "-NoExit", "-NoProfile", "-File", str(wrapper)], env=dict(env, AXIOM_INSTALL_YES="1"), cwd=temp)
                        try:
                            # The borrowed console must return before we type exit.
                            # Piping exit before -File completes is not an interactive host.
                            interactive.until(b"interactive-alive:", timeout=60)
                            interactive.send(b"exit 0\r")
                            code = interactive.wait(20)
                        finally:
                            interactive.save(folder, action + "-console")
                            if interactive.alive(): interactive.close()
                        output = bytes(interactive.raw).decode("utf-8", errors="replace")
                        if evidence is not None:
                            (folder / (action + ".json")).write_text(json.dumps(dict(exit_code=code, console_output=output, bootstrapper_sha256=hashlib.sha256(scripts[target.name]).hexdigest(), caller_continued="interactive-alive:7:Continue" in output), indent=2) + "\n", encoding="utf-8", newline="\n")
                        assert code == 0 and "interactive-alive:7:Continue" in output, (action, code, output)
                        cases.append(dict(case=action + "-iex-noexit", child_last_exit_code=7, caller_continued=True, preference_restored=True, native_console=True))
        finally:server.shutdown();server.server_close();thread.join(timeout=3)
    return dict(passed=True,platform=sys.platform,cases=cases,scope="Native production progress module and generated bootstrap against private fixture root/local HTTP; no real installed state or live Agent session")

if __name__=="__main__":
    p=argparse.ArgumentParser();p.add_argument("--out",type=Path,required=True);args=p.parse_args();result=verify(args.out.resolve());args.out.write_text(json.dumps(result,indent=2)+"\n",encoding="utf-8");print("PASS native worker progress parity, JSON/off isolation and bootstrap digest refusal")
