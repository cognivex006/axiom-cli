//! Human-only progress. Child protocol output remains captured and JSON stays quiet.
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);
static TERMINAL: AtomicBool = AtomicBool::new(false);

pub fn configure(args: &[std::ffi::OsString]) {
    let mode = std::env::var("AXIOM_PROGRESS").unwrap_or_default();
    ENABLED.store(
        !args.iter().any(|a| a == "--json") && mode != "off",
        Ordering::Relaxed,
    );
    TERMINAL.store(
        std::io::stderr().is_terminal() && mode != "plain",
        Ordering::Relaxed,
    );
}

pub struct Progress {
    stop: Option<Sender<bool>>,
    worker: Option<JoinHandle<()>>,
}

impl Progress {
    pub fn waiting(label: &str, detail: &str) -> Self {
        Self::start(label, detail, None)
    }

    pub fn download(path: PathBuf, total: Option<u64>) -> Self {
        Self::start(
            "Downloading",
            "Receiving data; integrity is checked after transfer",
            Some((path, total)),
        )
    }

    fn start(label: &str, detail: &str, file: Option<(PathBuf, Option<u64>)>) -> Self {
        if !ENABLED.load(Ordering::Relaxed) {
            return Self {
                stop: None,
                worker: None,
            };
        }
        let (tx, rx) = mpsc::channel();
        let label = label.to_string();
        let detail = detail.to_string();
        let baseline = file
            .as_ref()
            .and_then(|(p, _)| std::fs::metadata(p).ok())
            .map(|m| (m.len(), m.modified().ok()));
        let interval = if TERMINAL.load(Ordering::Relaxed) {
            1
        } else {
            5
        };
        let worker = thread::spawn(move || {
            let start = Instant::now();
            let mut rendered = false;
            let mut last = Instant::now();
            loop {
                let result = rx.recv_timeout(Duration::from_millis(200));
                let finish = match result {
                    Ok(ok) => Some(ok),
                    Err(mpsc::RecvTimeoutError::Disconnected) => Some(false),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                };
                if finish.is_some() || !rendered || last.elapsed().as_secs() >= interval {
                    // Fast probes remain quiet; long operations get their first activity promptly.
                    if !rendered
                        && finish.is_some()
                        && file.is_none()
                        && start.elapsed().as_millis() < 200
                    {
                        return;
                    }
                    let elapsed = start.elapsed().as_secs_f64();
                    let status = match finish {
                        Some(true) => "Done",
                        Some(false) => "Failed",
                        None => "Working",
                    };
                    let (heading, details) = if let Some((path, total)) = &file {
                        let bytes = std::fs::metadata(path)
                            .map(|m| {
                                if finish != Some(true)
                                    && Some((m.len(), m.modified().ok())) == baseline
                                {
                                    0
                                } else {
                                    m.len()
                                }
                            })
                            .unwrap_or(0);
                        transfer(&label, &detail, bytes, *total, elapsed, finish)
                    } else {
                        (
                            format!("{label} [{status}] elapsed {:.0}s", elapsed),
                            detail.clone(),
                        )
                    };
                    let mut stderr = std::io::stderr().lock();
                    let _ = writeln!(stderr, "{heading}\n  {details}");
                    let _ = stderr.flush();
                    rendered = true;
                    last = Instant::now();
                }
                if finish.is_some() {
                    return;
                }
            }
        });
        Self {
            stop: Some(tx),
            worker: Some(worker),
        }
    }

    pub fn finish(mut self, success: bool) {
        self.end(success);
    }

    fn end(&mut self, success: bool) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(success);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.end(false);
    }
}

fn transfer(
    label: &str,
    detail: &str,
    bytes: u64,
    total: Option<u64>,
    seconds: f64,
    finish: Option<bool>,
) -> (String, String) {
    let status = match finish {
        Some(true) => "Received",
        Some(false) => "Failed",
        None => "Downloading",
    };
    let mb = bytes as f64 / 1_000_000.0;
    let speed = if seconds > 0.0 {
        bytes as f64 / seconds
    } else {
        0.0
    };
    let Some(total) = total.filter(|n| *n > 0) else {
        return (
            format!("{label} [{status}] {mb:.1} MB received; elapsed {seconds:.0}s"),
            format!("{detail}; total size unknown"),
        );
    };
    let complete = finish == Some(true) && bytes == total;
    let percent = if complete {
        100
    } else {
        (bytes as u128 * 100 / total as u128).min(99) as u64
    };
    let bars = (percent / 5) as usize;
    let heading = format!(
        "{label} [{}{}] {percent}% ({status})",
        "=".repeat(bars),
        " ".repeat(20 - bars)
    );
    let eta = if finish.is_none() && speed > 0.0 && seconds >= 1.0 && bytes <= total {
        format!("; ETA ~{:.0}s", (total - bytes) as f64 / speed)
    } else {
        String::new()
    };
    (
        heading,
        format!(
            "{mb:.1}/{:.1} MB; {:.1} MB/s; elapsed {seconds:.0}s{eta}. {detail}",
            total as f64 / 1_000_000.0,
            speed / 1_000_000.0
        ),
    )
}

pub fn capture(command: &mut Command, label: &str, detail: &str) -> std::io::Result<Output> {
    let progress = Progress::waiting(label, detail);
    // This process owns the human display; nested machine workers remain quiet.
    let result = command.env("AXIOM_PROGRESS", "off").output();
    progress.finish(result.as_ref().is_ok_and(|output| output.status.success()));
    result
}

pub fn status(command: &mut Command, label: &str, detail: &str) -> std::io::Result<ExitStatus> {
    let progress = Progress::waiting(label, detail);
    let result = command.status();
    progress.finish(result.as_ref().is_ok_and(|status| status.success()));
    result
}

pub fn during<T, E>(label: &str, detail: &str, run: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let progress = Progress::waiting(label, detail);
    let result = run();
    progress.finish(result.is_ok());
    result
}

#[cfg(test)]
mod tests {
    use super::transfer;
    #[test]
    fn unknown_work_never_claims_percent_or_eta() {
        let (top, detail) = transfer("Download", "Waiting", 100, None, 2.0, None);
        assert!(!top.contains('%') && !detail.contains("ETA"));
        assert!(detail.contains("unknown"));
    }
    #[test]
    fn full_bytes_do_not_claim_completion_before_worker_success() {
        assert!(transfer("D", "", 100, Some(100), 2.0, None)
            .0
            .contains("99%"));
        assert!(transfer("D", "", 100, Some(100), 2.0, Some(true))
            .0
            .contains("100%"));
        assert!(!transfer("D", "", 100, Some(100), 2.0, Some(false))
            .0
            .contains("100%"));
        assert!(!transfer("D", "", 101, Some(100), 2.0, Some(true))
            .0
            .contains("100%"));
    }
}
