//! Test-only native counterpart of the existing shell engine fixtures.
use std::io::Write;

fn main() {
    let program = std::env::current_exe().unwrap();
    let body = std::fs::read_to_string(program.with_extension("fixture")).unwrap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args
        .iter()
        .position(|v| v == "--out")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_default();
    let phase = if args.first().map(String::as_str) == Some("install") {
        args.get(1).map(String::as_str).unwrap_or("")
    } else {
        ""
    };
    let selected = if body.contains("case \"$1 $2\"") {
        let marker = format!("\"install {phase}\")");
        body.split(&marker)
            .nth(1)
            .unwrap_or("")
            .split(";;")
            .next()
            .unwrap_or("")
    } else {
        body.as_str()
    };
    let mut exit = 0;
    for line in selected.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("printf '") {
            if let Some(end) = value.find('\'') {
                let template = &value[..end];
                let tail = &value[end + 1..];
                let data = template
                    .replace("\\n", "\n")
                    .replace("%s", &out.replace('\\', "\\\\").replace('"', "\\\""));
                if tail.contains("> \"$out\"") {
                    std::fs::write(&out, data).unwrap();
                } else if tail.contains(">&2") {
                    eprint!("{data}");
                } else {
                    print!("{data}");
                }
            }
        } else if let Some(value) = line.strip_prefix("sleep ") {
            std::thread::sleep(std::time::Duration::from_secs_f64(value.parse().unwrap()));
        } else if let Some(value) = line.strip_prefix("exit ") {
            exit = value.parse().unwrap_or(90);
            break;
        } else if line.starts_with("echo ") {
            if line.contains(">&2") {
                eprintln!("fixture error");
            } else {
                println!("fixture non JSON");
            }
        }
    }
    std::io::stdout().flush().unwrap();
    std::process::exit(exit);
}
