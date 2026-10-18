//! Exact production module in controlled owned fixtures; no installation or host service changes.
#![allow(dead_code)]
#[path = "../../src/progress.rs"]
mod progress;
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "--descendant") {
        let root = std::path::PathBuf::from(std::env::var_os("AXIOM_UX_MARKER_DIR").unwrap());
        std::fs::write(root.join("descendant.pid"), std::process::id().to_string()).unwrap();
        loop {
            if std::fs::write(root.join("heartbeat"), "alive").is_err() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        return;
    }
    let mut configured = args.clone();
    configured.insert(0, "install".into());
    if args.iter().any(|a| a == "--worker") {
        configured.extend([
            "--apply".into(),
            "--approve-digest".into(),
            "fixture-approved".into(),
        ]);
    }
    progress::configure(&configured);
    if args.iter().any(|a| a == "--owned-timeout") {
        let phase =
            progress::Progress::waiting("Testing cleanup", "waiting for owned child timeout");
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--worker", "--own-child", "--json"])
            .env("AXIOM_CLI_COMPOSITE_UPDATE", "1");
        let result = progress::bounded_output(&mut command, std::time::Duration::from_secs(2));
        phase.finish(result.is_err());
        progress::dispose();
        assert!(result.is_err_and(|e| e.kind() == std::io::ErrorKind::TimedOut));
        println!("{{\"timeout\":true}}");
        return;
    }
    if args.iter().any(|a| a == "--transfer") {
        use std::io::Write;
        let file = std::env::temp_dir().join(format!("axiom-ux-probe-{}.bin", std::process::id()));
        let mut writer = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
            .unwrap();
        let total = (!args.iter().any(|a| a == "--unknown")).then_some(1_048_576);
        let p = progress::Progress::download(file.clone(), total);
        for _ in 0..4 {
            writer.write_all(&vec![42; 262_144]).unwrap();
            writer.flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(450));
        }
        drop(writer);
        p.finish(true);
        let p = progress::Progress::waiting("Verifying", "verifying downloaded fixture bytes");
        std::thread::sleep(std::time::Duration::from_millis(450));
        let valid = std::fs::read(&file).unwrap() == vec![42; 1_048_576]
            && !args.iter().any(|a| a == "--fail");
        p.finish(valid);
        std::fs::remove_file(file).unwrap();
        progress::dispose();
        println!("{{\"verified\":{valid}}}");
        std::process::exit(if valid { 0 } else { 7 });
    }
    if args.iter().any(|a| a == "--visual") {
        for (label, detail) in [
            (
                "Preparing runtime",
                "preparing pinned Python runtime for Axiom MCP",
            ),
            ("Staging", "staging verified components"),
            ("Applying", "applying approved plan"),
        ] {
            let p = progress::Progress::waiting(label, detail);
            std::thread::sleep(std::time::Duration::from_millis(1200));
            p.finish(true);
        }
        progress::suspend();
        println!("Plan: controlled terminal fixture; no installation changes");
        use std::io::Write;
        print!("Proceed? [Y/n] ");
        std::io::stdout().flush().unwrap();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).unwrap();
        let p = progress::Progress::waiting("Finishing", "finishing verified fixture");
        std::thread::sleep(std::time::Duration::from_millis(1000));
        p.finish(true);
        progress::dispose();
        println!("Fixture complete: {}", answer.trim());
        return;
    }
    if args.iter().any(|a| a == "--worker") {
        let mut descendant = if args.iter().any(|a| a == "--own-child") {
            let root = std::path::PathBuf::from(std::env::var_os("AXIOM_UX_MARKER_DIR").unwrap());
            std::fs::write(root.join("worker.pid"), std::process::id().to_string()).unwrap();
            Some(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .arg("--descendant")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            )
        } else {
            None
        };
        if args.iter().any(|a| a == "--diagnostic") {
            eprintln!("fixture diagnostic: retained\x1b[2J");
        }
        let phase = if descendant.is_some() {
            progress::Progress::waiting(
                "Applying components",
                "applying the controlled worker plan",
            )
        } else {
            progress::Progress::waiting("Preparing MCP runtime", "Fixture runtime")
        };
        std::thread::sleep(std::time::Duration::from_millis(if descendant.is_some() {
            30_000
        } else {
            2200
        }));
        if let Some(child) = descendant.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let success = !args.iter().any(|a| a == "--fail");
        phase.finish(success);
        progress::dispose();
        println!("{{\"worker\":true}}");
        std::process::exit(if success { 0 } else { 7 });
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--worker", "--json"])
        .env("AXIOM_CLI_COMPOSITE_UPDATE", "1");
    if args.iter().any(|a| a == "--fail") {
        command.arg("--fail");
    }
    if args.iter().any(|a| a == "--diagnostic") {
        command.arg("--diagnostic");
    }
    if args.iter().any(|a| a == "--owned") {
        command.arg("--own-child");
    }
    let output =
        progress::install_worker(&mut command, !args.iter().any(|a| a == "--dry-run")).unwrap();
    use std::io::Write;
    std::io::stdout().write_all(&output.stdout).unwrap();
    progress::dispose();
    std::process::exit(output.status.code().unwrap_or(8));
}
