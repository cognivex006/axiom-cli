//! Uses the exact production progress module; no installation or external process is touched.
#![allow(dead_code)]
#[path = "../src/progress.rs"]
mod progress;
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    progress::configure(&args);
    if args.iter().any(|a| a == "--worker") {
        let phase = progress::Progress::waiting("Preparing MCP runtime", "Fixture runtime");
        std::thread::sleep(std::time::Duration::from_millis(2200));
        let success = !args.iter().any(|a| a == "--fail");
        phase.finish(success);
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
    let output =
        progress::install_worker(&mut command, !args.iter().any(|a| a == "--dry-run")).unwrap();
    use std::io::Write;
    std::io::stdout().write_all(&output.stdout).unwrap();
    std::process::exit(output.status.code().unwrap_or(8));
}
