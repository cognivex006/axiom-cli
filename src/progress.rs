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

struct Renderer {
    terminal: bool,
    width: usize,
    started: bool,
    last_heading: String,
}

impl Renderer {
    fn new(terminal: bool, _detail: String) -> Self {
        Self {
            terminal,
            width: 0,
            started: false,
            last_heading: String::new(),
        }
    }

    fn render(
        &mut self,
        output: &mut impl Write,
        heading: &str,
        details: &str,
        inline: &str,
        finished: bool,
    ) -> std::io::Result<()> {
        if self.terminal {
            let width = inline.chars().count();
            write!(
                output,
                "\r{inline}{}",
                " ".repeat(self.width.saturating_sub(width))
            )?;
            self.width = width;
            if finished {
                writeln!(output)?;
            }
        } else if finished || !self.started || heading != self.last_heading {
            // A pipe is a log, not a terminal: elapsed-only heartbeats add no information.
            writeln!(output, "{inline}")?;
        }
        self.started = true;
        self.last_heading = heading.to_string();
        let _ = details;
        output.flush()
    }
}

pub fn configure(args: &[std::ffi::OsString]) {
    let mode = std::env::var("AXIOM_PROGRESS").unwrap_or_default();
    ENABLED.store(
        (!args.iter().any(|a| a == "--json")
            || (std::env::var("AXIOM_CLI_COMPOSITE_UPDATE").as_deref() == Ok("1")
                && std::env::var("AXIOM_CLI_PROGRESS_RELAY").as_deref() == Ok("1")))
            && mode != "off",
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
        let terminal = TERMINAL.load(Ordering::Relaxed);
        let interval = if terminal { 1 } else { 5 };
        let worker = thread::spawn(move || {
            let mut renderer = Renderer::new(terminal, detail.clone());
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
                    // A measured first transfer frame can include a meaningful rate/ETA.
                    if file.is_some() && finish.is_none() && start.elapsed().as_secs_f64() < 1.0 {
                        continue;
                    }
                    let elapsed = start.elapsed().as_secs_f64();
                    let status = match finish {
                        Some(true) => "Done",
                        Some(false) => "Failed",
                        None => "Working",
                    };
                    let (heading, details, inline) = if let Some((path, total)) = &file {
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
                        let heading = format!("{label} [{status}]");
                        let inline = if terminal || finish.is_some() {
                            format!("{heading} elapsed {elapsed:.0}s")
                        } else {
                            heading.clone()
                        };
                        (heading, detail.clone(), inline)
                    };
                    let mut stderr = std::io::stderr().lock();
                    let _ =
                        renderer.render(&mut stderr, &heading, &details, &inline, finish.is_some());
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
) -> (String, String, String) {
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
            format!("{label} [{status}] {bytes} bytes received"),
            format!("{detail}; total size unknown"),
            format!(
                "{label} [{status}] {mb:.1} MB received; total size unknown; elapsed {seconds:.0}s"
            ),
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
        format!(
            "{label} {percent}% | {mb:.1}/{:.1} MB | {:.1} MB/s{eta}{}",
            total as f64 / 1_000_000.0,
            speed / 1_000_000.0,
            match finish {
                Some(true) => " [Received]",
                Some(false) => " [Failed]",
                None => "",
            }
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

/// The verified install worker owns live stage diagnostics; its JSON stdout stays captured.
/// Relaying is opt-in only for a human update's apply worker, never its dry-run or JSON caller.
pub fn install_worker(command: &mut Command, applying: bool) -> std::io::Result<Output> {
    let live = applying && ENABLED.load(Ordering::Relaxed);
    if live {
        command.env("AXIOM_CLI_PROGRESS_RELAY", "1");
        command.stderr(std::process::Stdio::inherit());
    } else {
        command.env("AXIOM_PROGRESS", "off");
        command.env_remove("AXIOM_CLI_PROGRESS_RELAY");
        command.stderr(std::process::Stdio::piped());
    }
    command.output()
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
    use super::{transfer, Renderer};
    #[test]
    fn terminal_repaints_one_line_and_clears_shorter_frames() {
        let mut output = Vec::new();
        let mut renderer = Renderer::new(true, "Preparing runtime".to_string());
        renderer
            .render(&mut output, "", "", "Working 100s", false)
            .unwrap();
        renderer
            .render(&mut output, "", "", "Working 9s", false)
            .unwrap();
        renderer.render(&mut output, "", "", "Done", true).unwrap();
        assert_eq!(output, b"\rWorking 100s\rWorking 9s  \rDone      \n");
    }
    #[test]
    fn failed_terminal_frame_finishes_before_following_diagnostics() {
        let mut output = Vec::new();
        let mut renderer = Renderer::new(true, "Receiving data".to_string());
        let (heading, details, inline) =
            transfer("Downloading", "", 50, Some(100), 2.0, Some(false));
        renderer
            .render(&mut output, &heading, &details, &inline, true)
            .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.ends_with(" [Failed]\n"));
        assert!(!text.contains("100%"));
    }
    #[test]
    fn redirected_output_has_readable_lines_without_cursor_controls() {
        let mut output = Vec::new();
        let mut renderer = Renderer::new(false, "Ignored".to_string());
        renderer
            .render(
                &mut output,
                "Working",
                "Preparing runtime",
                "Working",
                false,
            )
            .unwrap();
        renderer
            .render(&mut output, "Done", "Prepared runtime", "Done", true)
            .unwrap();
        assert_eq!(output, b"Working\nDone\n");
    }
    #[test]
    fn redirected_heartbeats_are_coalesced_but_failure_is_retained() {
        let mut output = Vec::new();
        let mut renderer = Renderer::new(false, String::new());
        for _ in 0..30 {
            renderer
                .render(
                    &mut output,
                    "Preparing [Working]",
                    "",
                    "Preparing [Working]",
                    false,
                )
                .unwrap();
        }
        renderer
            .render(
                &mut output,
                "Preparing [Failed]",
                "",
                "Preparing [Failed] elapsed 30s",
                true,
            )
            .unwrap();
        assert_eq!(
            output,
            b"Preparing [Working]\nPreparing [Failed] elapsed 30s\n"
        );
    }
    #[test]
    fn inline_transfer_retains_measured_eta_without_the_long_explanation() {
        let (_, _, inline) = transfer(
            "Downloading",
            "Integrity checked after transfer",
            1_000_000,
            Some(2_000_000),
            2.0,
            None,
        );
        assert!(
            inline.contains("50%") && inline.contains("0.5 MB/s") && inline.contains("ETA ~2s")
        );
        assert!(!inline.contains("Integrity"));
        assert!(inline.len() < 80);
    }
    #[test]
    fn unknown_work_never_claims_percent_or_eta() {
        let (top, detail, inline) = transfer("Download", "Waiting", 100, None, 2.0, None);
        assert!(!top.contains('%') && !detail.contains("ETA"));
        assert!(!inline.contains('%') && !inline.contains("ETA"));
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
