//! One human display owner; opted-in verified workers emit bounded semantic records.
#[path = "progress/child.rs"]
mod child;
#[path = "progress/terminal.rs"]
mod terminal;
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

#[derive(Clone)]
struct Stage {
    id: String,
    label: String,
    detail: String,
    file: Option<(PathBuf, Option<u64>)>,
    done: Option<bool>,
    received: Option<u64>,
}
enum Event {
    Begin(Stage),
    End(String, bool),
    Measure(String, Option<u64>),
    Suspend(mpsc::Sender<()>),
    Dispose(mpsc::Sender<()>),
}
struct Session {
    tx: mpsc::SyncSender<Event>,
    control: mpsc::Sender<Event>,
    worker: thread::JoinHandle<()>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancellation: thread::JoinHandle<()>,
    human: bool,
}
static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
static RELAY: OnceLock<bool> = OnceLock::new();
static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn clip(text: &str, columns: usize) -> String {
    let safe: String = text.chars().take(512).filter(|c| !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200b}' | '\u{200e}' | '\u{200f}')).collect();
    let mut end = 0;
    for (index, character) in safe.char_indices() {
        let next = index + character.len_utf8();
        if safe[..next].width() > columns {
            break;
        }
        end = next;
    }
    safe[..end].to_string()
}
struct Renderer {
    console: Option<terminal::Console>,
    reserved: bool,
    frame: usize,
    action: String,
    fallback: String,
    painted: Option<Instant>,
}
impl Renderer {
    fn begin(action: String, live: bool) -> Self {
        Self {
            console: if live {
                terminal::Console::open()
            } else {
                None
            },
            reserved: false,
            frame: 0,
            action,
            fallback: String::new(),
            painted: None,
        }
    }
    fn update(&mut self, s: &Stage) -> std::io::Result<()> {
        if self.reserved
            && self
                .painted
                .is_some_and(|t| t.elapsed() < Duration::from_millis(125))
        {
            return Ok(());
        }
        let Some((cols, rows)) = self.console.as_ref().and_then(|c| c.size()) else {
            return Ok(());
        };
        if cols < 32 || rows < 3 {
            self.suspend()?;
            if self.fallback != s.detail {
                let _ = writeln!(
                    std::io::stderr(),
                    "axiom {}: {}",
                    self.action,
                    clip(&s.detail, 2048)
                );
                self.fallback = s.detail.clone();
            }
            return Ok(());
        }
        let w = cols - 1;
        let reserve = if self.reserved {
            ""
        } else {
            "\x1b[?25l\r\n\r\n\x1b[2A\x1b7"
        };
        let percent = s.file.as_ref().and_then(|(_, total)| {
            let bytes = s.received?;
            total
                .filter(|n| *n > 0)
                .map(|n| (bytes as u128 * 100 / n as u128).min(99) as usize)
        });
        let bw = (w / 3).clamp(3, 40);
        let status = match s.done {
            Some(false) => "failed".to_string(),
            Some(true) if s.file.is_none() => "done".to_string(),
            _ => percent
                .map(|p| format!("{p}%"))
                .unwrap_or_else(|| "indeterminate".into()),
        };
        let filled = percent.map(|p| p * bw / 100);
        let bar: String = (0..bw)
            .map(|i| {
                if filled.is_some_and(|n| i < n) || filled.is_none() && i == self.frame % bw {
                    '='
                } else {
                    '-'
                }
            })
            .collect();
        let label = clip(&s.label, w.saturating_sub(bw + status.len() + 5));
        let top = format!("{label}  [{bar}] {status}");
        let spinner = ['|', '/', '-', '\\'][self.frame % 4];
        let detail = clip(&format!("{spinner} axiom {}: {}", self.action, s.detail), w);
        let frame = format!("{reserve}\x1b8\r\x1b[2K{top}\r\n\x1b[2K{detail}\x1b8");
        let mut out = std::io::stderr().lock();
        // Record ownership before writing so a partial write is cleared on failure.
        self.reserved = true;
        out.write_all(frame.as_bytes())?;
        self.painted = Some(Instant::now());
        self.frame = self.frame.wrapping_add(1);
        out.flush()
    }
    fn suspend(&mut self) -> std::io::Result<()> {
        if self.reserved {
            self.reserved = false;
            self.painted = None;
            let mut out = std::io::stderr().lock();
            write!(out, "\x1b8\r\x1b[2K\r\n\x1b[2K\x1b8\x1b[?25h")?;
            out.flush()?;
        }
        Ok(())
    }
    fn complete(&mut self, s: &Stage) {
        let _ = self.update(s);
    }
    fn fail(&mut self, s: &Stage) {
        let _ = self.update(s);
    }
    fn dispose(&mut self) {
        let _ = self.suspend();
        self.console.take();
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        self.dispose();
    }
}
pub fn configure(args: &[std::ffi::OsString]) {
    let private_worker = std::env::var("AXIOM_CLI_COMPOSITE_UPDATE").as_deref() == Ok("1")
        && args.iter().any(|a| a == "install")
        && args.iter().any(|a| a == "--json")
        && (args.iter().any(|a| a == "--dry-run")
            || args.iter().any(|a| a == "--apply") && args.iter().any(|a| a == "--approve-digest"));
    child::configure_inheritance(private_worker);
    // Routed transports/engine commands own their console and signal lifecycle.
    let root_verb = args.iter().filter_map(|a| a.to_str()).find(|a| {
        matches!(
            *a,
            "mcp"
                | "engine"
                | "install"
                | "update"
                | "uninstall"
                | "doctor"
                | "version"
                | "skills"
                | "help"
        )
    });
    if matches!(root_verb, Some("mcp" | "engine")) {
        return;
    }
    let action = args
        .iter()
        .find_map(|a| {
            a.to_str()
                .filter(|a| matches!(*a, "install" | "update" | "uninstall"))
        })
        .unwrap_or("setup")
        .to_string();
    let relay = private_worker && std::env::var("AXIOM_CLI_PROGRESS_EVENTS").as_deref() == Ok("1");
    let _ = RELAY.set(relay);
    let mode = std::env::var("AXIOM_PROGRESS").unwrap_or_default();
    let human = !relay && mode != "off" && !args.iter().any(|a| a == "--json");
    let live = human
        && mode != "plain"
        && std::env::var_os("CI").is_none()
        && std::env::var("TERM").as_deref() != Ok("dumb")
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal();
    let (tx, rx) = mpsc::sync_channel(64);
    // Only trusted lifecycle callers can enqueue controls; worker records cannot.
    // A full activity queue must never block prompt handoff or cancellation.
    let (control, controls) = mpsc::channel();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancellation_stop = stop.clone();
    let cancellation_control = control.clone();
    let cancellation_handler = terminal::Cancellation::install();
    let cancellation = thread::spawn(move || {
        let _handler = cancellation_handler;
        while !cancellation_stop.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(code) = terminal::cancelled() {
                let (ack, received) = mpsc::channel();
                let _ = cancellation_control.send(Event::Dispose(ack));
                let _ = received.recv_timeout(Duration::from_millis(250));
                child::cancel_all(code);
                std::process::exit(code);
            }
            thread::sleep(Duration::from_millis(25));
        }
    });
    let measure: std::sync::Arc<Mutex<Option<(String, PathBuf)>>> =
        std::sync::Arc::new(Mutex::new(None));
    let source = measure.clone();
    let measure_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = measure_stop.clone();
    let updates = tx.clone();
    let meter = thread::spawn(move || {
        while !stopping.load(std::sync::atomic::Ordering::Relaxed) {
            let current = source.lock().unwrap().clone();
            if let Some((id, path)) = current {
                // Filesystem metadata may block on a user-provided mount. It never
                // runs on the UI thread or holds the transaction/renderer lock.
                let bytes = std::fs::metadata(path).map(|m| m.len()).ok();
                let _ = updates.try_send(Event::Measure(id, bytes));
            }
            thread::sleep(Duration::from_millis(125));
        }
    });
    let worker = thread::spawn(move || {
        let mut renderer = Renderer::begin(action, live);
        let mut stage: Option<Stage> = None;
        loop {
            let event = match controls.try_recv() {
                Ok(event) => Ok(event),
                Err(_) => rx.recv_timeout(Duration::from_millis(125)),
            };
            match event {
                Ok(Event::Begin(s)) => {
                    if !human {
                        continue;
                    }
                    if renderer.console.is_none()
                        && !stage
                            .as_ref()
                            .is_some_and(|old| old.detail == s.detail && old.label == s.label)
                    {
                        let _ = writeln!(
                            std::io::stderr(),
                            "axiom {}: {}",
                            renderer.action,
                            clip(&s.detail, 2048)
                        );
                    }
                    *measure.lock().unwrap() = if renderer.console.is_some() {
                        s.file
                            .as_ref()
                            .map(|(path, _)| (s.id.clone(), path.clone()))
                    } else {
                        None
                    };
                    stage = Some(s);
                }
                Ok(Event::Measure(id, bytes)) => {
                    if let Some(s) = stage.as_mut().filter(|s| s.id == id) {
                        s.received = bytes;
                    }
                }
                Ok(Event::End(id, ok)) => {
                    if !human {
                        continue;
                    }
                    if let Some(s) = stage.as_mut().filter(|s| s.id == id) {
                        *measure.lock().unwrap() = None;
                        s.done = Some(ok);
                        if !ok && renderer.console.is_none() {
                            let _ = writeln!(
                                std::io::stderr(),
                                "axiom {}: failed: {}",
                                renderer.action,
                                clip(&s.detail, 2048)
                            );
                        }
                        if ok {
                            renderer.complete(s);
                        } else {
                            renderer.fail(s);
                        }
                    }
                }
                Ok(Event::Suspend(ack)) => {
                    // Activity queued before the handoff cannot repaint over a prompt.
                    while rx.try_recv().is_ok() {}
                    *measure.lock().unwrap() = None;
                    let _ = renderer.suspend();
                    stage = None;
                    let _ = ack.send(());
                }
                Ok(Event::Dispose(ack)) => {
                    while rx.try_recv().is_ok() {}
                    renderer.dispose();
                    let _ = ack.send(());
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(s) = &stage {
                if renderer.update(s).is_err() {
                    renderer.dispose();
                    *measure.lock().unwrap() = None;
                }
            }
        }
        measure_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        // A blocked mount cannot delay the transaction or shutdown. The process
        // owns at most one measurement worker, and process exit closes its I/O.
        if meter.is_finished() {
            let _ = meter.join();
        }
    });
    *SESSION.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(Session {
        tx,
        control,
        worker,
        stop,
        cancellation,
        human,
    });
}
fn send(event: Event) {
    if let Some(s) = SESSION.get().and_then(|s| s.lock().ok()) {
        if let Some(s) = &*s {
            if matches!(event, Event::Suspend(_) | Event::Dispose(_)) {
                let _ = s.control.send(event);
            } else {
                let _ = s.tx.try_send(event);
            }
        }
    }
}
pub fn suspend() {
    if SESSION.get().is_none() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    send(Event::Suspend(tx));
    let _ = rx.recv_timeout(Duration::from_secs(2));
}
pub fn dispose() {
    if let Some(s) = SESSION
        .get()
        .and_then(|s| s.lock().ok())
        .and_then(|mut s| s.take())
    {
        let (tx, rx) = mpsc::channel();
        let _ = s.control.send(Event::Dispose(tx));
        let acknowledged = rx.recv_timeout(Duration::from_secs(2)).is_ok();
        s.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = s.cancellation.join();
        if acknowledged || s.worker.is_finished() {
            let _ = s.worker.join();
        }
    }
}
fn hex(s: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut end = s.len().min(2048);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end]
        .bytes()
        .flat_map(|b| {
            [
                DIGITS[usize::from(b >> 4)] as char,
                DIGITS[usize::from(b & 15)] as char,
            ]
        })
        .collect()
}
fn bounded(s: &str) -> &str {
    let mut end = s.len().min(2048);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
fn unhex(s: &str) -> Option<String> {
    if s.len() > 4096 || s.len() % 2 != 0 {
        return None;
    }
    String::from_utf8(
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
            .collect::<Option<Vec<_>>>()?,
    )
    .ok()
}
fn record(kind: &str, s: &Stage) {
    let (path, total) = s
        .file
        .as_ref()
        .and_then(|(p, n)| {
            p.to_str()
                .filter(|p| p.len() <= 2048)
                .map(|p| (p.to_string(), n.unwrap_or(0)))
        })
        .unwrap_or_default();
    eprintln!(
        "AXIOM_PROGRESS_V1\t{kind}\t{}\t{}\t{}\t{}\t{total}",
        s.id,
        hex(&s.label),
        hex(&s.detail),
        hex(&path)
    );
}
pub struct Progress {
    stage: Option<Stage>,
}
impl Progress {
    pub fn waiting(label: &str, detail: &str) -> Self {
        Self::start(label, detail, None)
    }
    pub fn download(path: PathBuf, total: Option<u64>) -> Self {
        let detail = format!(
            "downloading {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        Self::start("Downloading", &detail, Some((path, total)))
    }
    fn start(label: &str, detail: &str, file: Option<(PathBuf, Option<u64>)>) -> Self {
        let stage = Stage {
            id: format!(
                "{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            label: bounded(label).into(),
            detail: bounded(detail).into(),
            file: file.filter(|(path, _)| path.to_str().is_some_and(|path| path.len() <= 2048)),
            done: None,
            received: None,
        };
        if RELAY.get() == Some(&true) {
            record("begin", &stage);
        } else {
            send(Event::Begin(stage.clone()));
        }
        Self { stage: Some(stage) }
    }
    pub fn finish(mut self, success: bool) {
        self.end(success);
    }
    fn end(&mut self, ok: bool) {
        if let Some(s) = self.stage.take() {
            if RELAY.get() == Some(&true) {
                record(if ok { "complete" } else { "fail" }, &s);
            } else {
                send(Event::End(s.id, ok));
            }
        }
    }
}
impl Drop for Progress {
    fn drop(&mut self) {
        self.end(false);
    }
}
fn parse(line: &str) -> Option<Event> {
    if line.len() > 16384 {
        return None;
    }
    let body = line.strip_suffix('\n')?;
    let f: Vec<_> = body
        .strip_suffix('\r')
        .unwrap_or(body)
        .split('\t')
        .collect();
    if f.len() != 7
        || f[0] != "AXIOM_PROGRESS_V1"
        || f[2].len() > 64
        || f[2].is_empty()
        || !f[2].bytes().all(|c| c.is_ascii_digit() || c == b'-')
        || f[6].is_empty()
        || !f[6].bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let (pid, sequence) = f[2].split_once('-')?;
    let _: u32 = pid.parse().ok()?;
    let _: u64 = sequence.parse().ok()?;
    let label = unhex(f[3])?;
    let detail = unhex(f[4])?;
    let path = unhex(f[5])?;
    let total: u64 = f[6].parse().ok()?;
    match f[1] {
        "begin" => Some(Event::Begin(Stage {
            id: f[2].into(),
            label,
            detail,
            file: if path.is_empty() {
                None
            } else {
                Some((path.into(), (total > 0).then_some(total)))
            },
            done: None,
            received: None,
        })),
        "complete" | "fail" => Some(Event::End(f[2].into(), f[1] == "complete")),
        _ => None,
    }
}
pub fn capture(command: &mut Command, label: &str, detail: &str) -> std::io::Result<Output> {
    let p = Progress::waiting(label, detail);
    let result = child::capture(
        command
            .env("AXIOM_PROGRESS", "off")
            .env_remove("AXIOM_CLI_PROGRESS_EVENTS"),
        None,
    );
    p.finish(result.as_ref().is_ok_and(|o| o.status.success()));
    result
}
pub fn install_worker(command: &mut Command, applying: bool) -> std::io::Result<Output> {
    let human = SESSION
        .get()
        .and_then(|s| s.lock().ok())
        .is_some_and(|s| s.as_ref().is_some_and(|s| s.human));
    if !applying || !human {
        let result = child::capture(
            command
                .env("AXIOM_PROGRESS", "off")
                .env_remove("AXIOM_CLI_PROGRESS_EVENTS"),
            None,
        );
        if applying {
            if let Ok(output) = &result {
                diagnostic(&output.stderr);
            }
        }
        return result;
    }
    let p = Progress::waiting("Installing", "applying the approved component plan");
    let child = child::Owned::spawn(
        command
            .env("AXIOM_CLI_PROGRESS_EVENTS", "1")
            .stdin(Stdio::null()) // Approved machine apply only; public confirmation stays inherited.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    let stdout = child.stdout();
    let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let output_failed = failed.clone();
    let reader = thread::spawn(move || {
        let result = child::read_bounded(stdout);
        if result.is_err() {
            output_failed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        result
    });
    let stderr = child.stderr();
    let pid = child.id();
    let abort = failed.clone();
    let errors = thread::spawn(move || {
        let result = relay_stderr(std::io::BufReader::new(stderr), pid);
        if result.is_err() {
            abort.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        result
    });
    let status = child.wait_abort(None, Some(&failed));
    drop(child);
    let stdout = reader
        .join()
        .map_err(|_| std::io::Error::other("worker stdout reader failed"))?;
    let stderr = errors
        .join()
        .map_err(|_| std::io::Error::other("worker stderr reader failed"))?;
    p.finish(status.as_ref().is_ok_and(|s| s.success()) && stdout.is_ok() && stderr.is_ok());
    Ok(Output {
        status: status?,
        stdout: stdout?,
        stderr: stderr?,
    })
}

fn relay_stderr(mut error: impl BufRead, worker_pid: u32) -> std::io::Result<Vec<u8>> {
    let mut stderr = Vec::new();
    let mut last_sequence: Option<u64> = None;
    let mut active_id: Option<String> = None;
    let mut discarding = false;
    let mut overflow = false;
    loop {
        let mut bytes = Vec::new();
        let n = error.by_ref().take(16385).read_until(b'\n', &mut bytes)?;
        if n == 0 {
            break;
        }
        if discarding {
            discarding = !bytes.ends_with(b"\n");
            continue;
        }
        if let Some(event) = bytes
            .ends_with(b"\n")
            .then(|| std::str::from_utf8(&bytes).ok().and_then(parse))
            .flatten()
        {
            let valid = match &event {
                Event::Begin(s) => {
                    let (pid, sequence) = s.id.split_once('-').unwrap();
                    let sequence: u64 = sequence.parse().unwrap();
                    if pid.parse::<u32>().ok() == Some(worker_pid)
                        && last_sequence.is_none_or(|n| sequence > n)
                    {
                        last_sequence = Some(sequence);
                        active_id = Some(s.id.clone());
                        true
                    } else {
                        false
                    }
                }
                Event::End(id, _) => {
                    if active_id.as_ref() == Some(id) {
                        active_id = None;
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };
            if valid {
                send(event);
            }
        } else if bytes.starts_with(b"AXIOM_PROGRESS_") {
            suspend();
            let message = b"axiom: ignored invalid worker progress record\n";
            diagnostic(message);
            if stderr.len() + message.len() <= 4 * 1024 * 1024 {
                stderr.extend_from_slice(message);
            } else {
                overflow = true;
            }
            discarding = !bytes.ends_with(b"\n");
        } else {
            suspend();
            if stderr.len() + bytes.len() <= 4 * 1024 * 1024 {
                diagnostic(&bytes);
                stderr.extend_from_slice(&bytes);
            } else {
                overflow = true;
            }
        }
        if overflow {
            return Err(std::io::Error::other("worker stderr exceeded 4 MiB"));
        }
    }
    if overflow {
        Err(std::io::Error::other("worker stderr exceeded 4 MiB"))
    } else {
        Ok(stderr)
    }
}

pub fn bounded_output(command: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    child::capture(command, Some(timeout))
}

pub fn worker_output(command: &mut Command) -> std::io::Result<Output> {
    child::capture(command, None)
}
pub fn diagnostics(bytes: &[u8]) {
    suspend();
    diagnostic(bytes);
}

fn diagnostic(bytes: &[u8]) {
    // Restore real worker diagnostics even when the transaction consumes stdout only.
    // Preserve text/newlines while refusing terminal-control or bidi instructions.
    let safe = diagnostic_text(bytes, RELAY.get() == Some(&true));
    let _ = std::io::stderr().lock().write_all(safe.as_bytes());
}
fn diagnostic_text(bytes: &[u8], relay: bool) -> String {
    let safe: String = String::from_utf8_lossy(bytes).chars().filter(|c| *c == '\n' || *c == '\t' || !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}')).collect();
    if relay {
        safe.split_inclusive('\n')
            .map(|line| {
                if line.starts_with("AXIOM_PROGRESS_") {
                    format!("diagnostic: {line}")
                } else {
                    line.to_string()
                }
            })
            .collect()
    } else {
        safe
    }
}
pub fn status(command: &mut Command, label: &str, detail: &str) -> std::io::Result<ExitStatus> {
    let output = capture(command, label, detail)?;
    if !output.status.success() {
        diagnostics(&output.stderr);
    }
    Ok(output.status)
}
pub fn during<T, E>(label: &str, detail: &str, run: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let p = Progress::waiting(label, detail);
    let result = run();
    p.finish(result.is_ok());
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clips_visual_columns_and_controls() {
        assert_eq!(clip("界界ab\n\u{202e}", 5), "界界a");
        assert_eq!(clip("e\u{301}x", 1), "e\u{301}");
        assert_eq!(clip("❤️❤️", 2), "❤️");
    }
    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_never_turn_into_a_different_measurement_target() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/fixture-\xff".to_vec()));
        let progress = Progress::download(path, Some(100));
        assert!(progress.stage.as_ref().unwrap().file.is_none());
    }
    #[test]
    fn rejects_untrusted_event_records() {
        let valid = "AXIOM_PROGRESS_V1\tbegin\t123-0\t4c\t446574\t\t0\n";
        assert!(
            matches!(parse(valid), Some(Event::Begin(s)) if s.label == "L" && s.detail == "Det" && s.file.is_none())
        );
        for s in [
            valid.replace("V1", "V2"),
            valid.replace("begin", "verify"),
            valid.replace("4c", "ff"),
            valid.replace("4c", "g0"),
            valid.replace("4c", "f"),
            valid.replace("123-0", "123--1"),
            valid.replace("123-0", "4294967296-0"),
            valid.replace("\t0\n", "\t+1\n"),
            valid.replace("\t0\n", "\t18446744073709551616\n"),
            valid.replace("4c", &"61".repeat(2049)),
            valid.replace('\n', ""),
            valid.replace('\n', "\r\r\n"),
        ] {
            assert!(parse(&s).is_none(), "{s}");
        }
    }
    #[test]
    fn diagnostics_cannot_spoof_a_worker_event_or_inject_controls() {
        let data = b"real error\nAXIOM_PROGRESS_V1\tbegin\n\x1b[2Junsafe";
        let safe = diagnostic_text(data, true);
        assert!(safe.starts_with("real error\ndiagnostic: AXIOM_PROGRESS_V1"));
        assert!(!safe.contains('\x1b'));
    }
    #[test]
    fn stderr_read_errors_are_not_silently_treated_as_eof() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fixture read failure"))
            }
        }
        impl BufRead for Broken {
            fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
                Err(std::io::Error::other("fixture read failure"))
            }
            fn consume(&mut self, _: usize) {}
        }
        assert!(relay_stderr(Broken, 123).is_err());
    }
}
