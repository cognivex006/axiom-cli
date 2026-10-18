//! Retained process ownership, bounded capture and cancellation cleanup.
use std::io::{Read, Result};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
static CHILDREN: OnceLock<Mutex<Vec<Weak<Mutex<Process>>>>> = OnceLock::new();
#[cfg(unix)]
static INHERITED: AtomicBool = AtomicBool::new(false);
const GROUP_ENV: &str = "AXIOM_CLI_OWNED_PROCESS_GROUP";

pub fn configure_inheritance(private_worker: bool) {
    #[cfg(unix)]
    {
        let raw = std::env::var(GROUP_ENV).ok();
        let parent = unsafe { libc::getppid() };
        let group = unsafe { libc::getpgrp() };
        let parent_group = unsafe { libc::getpgid(parent) };
        let valid = raw.as_deref().is_some_and(|value| {
            valid_inheritance(
                value,
                private_worker,
                parent,
                std::process::id() as i32,
                group,
                parent_group,
            )
        });
        INHERITED.store(valid, Ordering::Relaxed);
        if !valid {
            std::env::remove_var(GROUP_ENV);
        }
    }
    #[cfg(windows)]
    {
        let _ = private_worker;
        std::env::remove_var(GROUP_ENV);
    }
}
#[cfg(unix)]
fn valid_inheritance(
    raw: &str,
    private_worker: bool,
    parent: i32,
    pid: i32,
    group: i32,
    parent_group: i32,
) -> bool {
    if !private_worker {
        return false;
    }
    let fields: Vec<_> = raw.split(':').collect();
    if fields.len() != 4 || fields[0] != "v1" {
        return false;
    }
    let inherited = fields[3].parse::<i32>().ok();
    fields[1].parse().ok() == Some(parent)
        && fields[2].parse().ok() == Some(parent_group)
        && parent > 0
        && parent_group > 0
        && (inherited == Some(0) && group == pid && group != parent_group
            || inherited == Some(group) && group == parent_group)
}
struct Process {
    child: Child,
    _job: Job,
    exit: Option<ExitStatus>,
    detached: bool,
}
impl Process {
    fn poll(&mut self) -> Result<Option<ExitStatus>> {
        if self.exit.is_some() {
            return Ok(self.exit);
        }
        #[cfg(unix)]
        {
            // Keep this child waitable until its process group is closed. Its
            // PID cannot be reused while the child/zombie is still retained.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            } != 0
            {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self._job.group = None;
                    self.detached = true;
                }
                return Err(error);
            }
            if info.si_signo == 0 {
                return Ok(None);
            }
            self._job.terminate();
            self.exit = Some(self.child.wait()?);
        }
        #[cfg(windows)]
        {
            self.exit = self.child.try_wait()?;
        }
        Ok(self.exit)
    }
}
pub struct Owned(Arc<Mutex<Process>>);
impl Owned {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        // Cancellation cannot finish its ownership snapshot between spawn and
        // registration. This lock never covers transaction work or pipe reads.
        let mut children = CHILDREN
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap();
        if super::terminal::cancelled().is_some() {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x00000004);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            if !INHERITED.load(Ordering::Relaxed) {
                command.process_group(0);
            }
        }
        #[cfg(unix)]
        {
            let group = unsafe { libc::getpgrp() };
            let inherited = if INHERITED.load(Ordering::Relaxed) {
                group
            } else {
                0
            };
            command.env(
                GROUP_ENV,
                format!("v1:{}:{group}:{inherited}", std::process::id()),
            );
        }
        #[cfg(windows)]
        command.env_remove(GROUP_ENV);
        let mut child = command.spawn()?;
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        #[cfg(windows)]
        if let Err(error) = resume_owned(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let this = Self(Arc::new(Mutex::new(Process {
            child,
            _job: job,
            exit: None,
            detached: false,
        })));
        children.retain(|child| child.strong_count() > 0);
        children.push(Arc::downgrade(&this.0));
        Ok(this)
    }
    pub fn id(&self) -> u32 {
        self.0.lock().unwrap().child.id()
    }
    pub fn stdout(&self) -> std::process::ChildStdout {
        self.0.lock().unwrap().child.stdout.take().unwrap()
    }
    pub fn stderr(&self) -> std::process::ChildStderr {
        self.0.lock().unwrap().child.stderr.take().unwrap()
    }
    pub fn wait_abort(
        &self,
        timeout: Option<Duration>,
        aborted: Option<&AtomicBool>,
    ) -> Result<ExitStatus> {
        let deadline = timeout.and_then(|duration| Instant::now().checked_add(duration));
        loop {
            if let Some(status) = self.0.lock().unwrap().poll()? {
                return Ok(status);
            }
            if aborted.is_some_and(|a| a.load(Ordering::Relaxed)) {
                return Err(std::io::Error::other("worker stream failed"));
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "owned worker exceeded its time budget",
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        let mut process = self.0.lock().unwrap();
        process._job.terminate();
        if process.exit.is_none() && !process.detached {
            let _ = process.child.kill();
            process.exit = process.child.wait().ok();
        }
    }
}
pub fn cancel_all(code: i32) {
    let children: Vec<_> = CHILDREN
        .get()
        .and_then(|c| c.lock().ok())
        .map(|c| c.iter().filter_map(Weak::upgrade).collect())
        .unwrap_or_default();
    for child in &children {
        let process = child.lock().unwrap();
        process._job.interrupt(code);
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        let running = children
            .iter()
            .any(|c| c.lock().unwrap().poll().ok().flatten().is_none());
        if !running || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    for child in children {
        let mut process = child.lock().unwrap();
        process._job.terminate();
        if process.exit.is_none() && !process.detached {
            let _ = process.child.kill();
            process.exit = process.child.wait().ok();
        }
    }
}
pub fn read_bounded(mut source: impl Read) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = [0; 16384];
    loop {
        let n = source.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        if output.len() + n <= OUTPUT_LIMIT {
            output.extend_from_slice(&buffer[..n]);
        } else {
            return Err(std::io::Error::other("worker output exceeded 4 MiB"));
        }
    }
    Ok(output)
}
pub fn capture(command: &mut Command, timeout: Option<Duration>) -> Result<Output> {
    let child = Owned::spawn(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    let stdout = child.stdout();
    let stderr = child.stderr();
    let failed = Arc::new(AtomicBool::new(false));
    let out_failed = failed.clone();
    let err_failed = failed.clone();
    let out = thread::spawn(move || {
        let result = read_bounded(stdout);
        if result.is_err() {
            out_failed.store(true, Ordering::Relaxed);
        }
        result
    });
    let err = thread::spawn(move || {
        let result = read_bounded(stderr);
        if result.is_err() {
            err_failed.store(true, Ordering::Relaxed);
        }
        result
    });
    let status = child.wait_abort(timeout, Some(&failed));
    drop(child); // Close the entire owned family before joining either pipe reader.
    let stdout = out
        .join()
        .map_err(|_| std::io::Error::other("worker stdout reader failed"))?;
    let stderr = err
        .join()
        .map_err(|_| std::io::Error::other("worker stderr reader failed"))?;
    Ok(Output {
        status: status?,
        stdout: stdout?,
        stderr: stderr?,
    })
}

#[cfg(unix)]
struct Job {
    group: Option<i32>,
}
#[cfg(unix)]
impl Job {
    fn attach(child: &Child) -> Result<Self> {
        Ok(Self {
            group: if INHERITED.load(Ordering::Relaxed) {
                None
            } else {
                Some(child.id() as i32)
            },
        })
    }
    fn interrupt(&self, code: i32) {
        if let Some(group) = self.group {
            unsafe {
                libc::kill(-group, code - 128);
            }
        }
    }
    fn terminate(&mut self) {
        if let Some(group) = self.group.take() {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    }
}

#[cfg(windows)]
struct Job(*mut std::ffi::c_void);
#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    pub struct Limits {
        pub user_time: i64,
        pub job_time: i64,
        pub flags: u32,
        pub min_working: usize,
        pub max_working: usize,
        pub active: u32,
        pub affinity: usize,
        pub priority: u32,
        pub scheduling: u32,
        pub io: [u64; 6],
        pub process_memory: usize,
        pub job_memory: usize,
        pub peak_process: usize,
        pub peak_job: usize,
    }
    #[repr(C)]
    #[derive(Default)]
    pub struct ThreadEntry {
        pub size: u32,
        pub usage: u32,
        pub id: u32,
        pub owner: u32,
        pub priority: i32,
        pub delta: i32,
        pub flags: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> *mut c_void;
        pub fn SetInformationJobObject(
            job: *mut c_void,
            kind: u32,
            info: *const Limits,
            size: u32,
        ) -> i32;
        pub fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        pub fn TerminateJobObject(job: *mut c_void, code: u32) -> i32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
        pub fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        pub fn Thread32First(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
        pub fn Thread32Next(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
        pub fn OpenThread(access: u32, inherit: i32, tid: u32) -> *mut c_void;
        pub fn GetProcessIdOfThread(thread: *mut c_void) -> u32;
        pub fn ResumeThread(thread: *mut c_void) -> u32;
    }
}

#[cfg(windows)]
fn resume_owned(child: &Child) -> Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    unsafe {
        let raw = windows::CreateToolhelp32Snapshot(4, 0);
        if raw == -1isize as *mut std::ffi::c_void {
            return Err(std::io::Error::last_os_error());
        }
        let snapshot = OwnedHandle::from_raw_handle(raw);
        let mut entry = windows::ThreadEntry {
            size: std::mem::size_of::<windows::ThreadEntry>() as u32,
            ..Default::default()
        };
        let mut found = windows::Thread32First(snapshot.as_raw_handle(), &mut entry);
        while found != 0 {
            if entry.owner == child.id() {
                let raw = windows::OpenThread(0x0002 | 0x0800, 0, entry.id);
                if raw.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let thread = OwnedHandle::from_raw_handle(raw);
                if windows::GetProcessIdOfThread(thread.as_raw_handle()) != child.id() {
                    return Err(std::io::Error::other(
                        "owned primary thread identity changed",
                    ));
                }
                let count = windows::ResumeThread(thread.as_raw_handle());
                if count == u32::MAX {
                    return Err(std::io::Error::last_os_error());
                }
                if count != 1 {
                    return Err(std::io::Error::other(
                        "unexpected primary thread suspension state",
                    ));
                }
                return Ok(());
            }
            found = windows::Thread32Next(snapshot.as_raw_handle(), &mut entry);
        }
        Err(std::io::Error::other(
            "owned suspended primary thread was not found",
        ))
    }
}
#[cfg(windows)]
impl Job {
    fn attach(child: &Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            let job = windows::CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let limits = windows::Limits {
                flags: 0x2000,
                ..Default::default()
            };
            if windows::SetInformationJobObject(
                job,
                9,
                &limits,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || windows::AssignProcessToJobObject(job, child.as_raw_handle()) == 0
            {
                let error = std::io::Error::last_os_error();
                windows::CloseHandle(job);
                return Err(error);
            }
            Ok(Self(job))
        }
    }
    fn interrupt(&self, _code: i32) {} // Console Ctrl+C is already broadcast to the family.
    fn terminate(&self) {
        unsafe {
            windows::TerminateJobObject(self.0, 130);
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn ambient_or_stale_group_markers_cannot_disable_public_ownership() {
        assert!(!valid_inheritance("1", true, 10, 12, 12, 42));
        assert!(!valid_inheritance("v1:10:42:0", false, 10, 12, 12, 42));
        assert!(!valid_inheritance("v1:9:42:0", true, 10, 12, 12, 42));
        assert!(valid_inheritance("v1:10:42:0", true, 10, 12, 12, 42));
        assert!(valid_inheritance("v1:10:12:12", true, 10, 20, 12, 12));
    }
    #[test]
    fn bounded_output_rejects_flood_without_growing_forever() {
        assert_eq!(
            read_bounded(&b"real diagnostic"[..]).unwrap(),
            b"real diagnostic"
        );
        assert!(read_bounded(std::io::repeat(b'x').take((OUTPUT_LIMIT + 1) as u64)).is_err());
    }
    #[test]
    fn stream_failure_closes_and_reaps_the_owned_child() {
        #[cfg(windows)]
        let mut command = {
            let mut c = Command::new(
                std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                    .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            );
            c.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
            c
        };
        #[cfg(unix)]
        let mut command = {
            let mut c = Command::new("/bin/sh");
            c.args(["-c", "sleep 30"]);
            c
        };
        let child = Owned::spawn(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .unwrap();
        let retained = child.0.clone();
        let failed = AtomicBool::new(true);
        assert!(child
            .wait_abort(Some(Duration::from_secs(5)), Some(&failed))
            .is_err());
        drop(child);
        assert!(retained.lock().unwrap().exit.is_some());
    }
}
