//! Native VT capability and viewport size.
use std::sync::atomic::{AtomicI32, Ordering};
static CANCELLED: AtomicI32 = AtomicI32::new(0);
pub fn cancelled() -> Option<i32> {
    let code = CANCELLED.load(Ordering::Relaxed);
    (code != 0).then_some(code)
}
#[cfg(windows)]
pub struct Cancellation;
#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}
#[cfg(windows)]
unsafe extern "system" fn cancel(kind: u32) -> i32 {
    if kind <= 1 {
        CANCELLED.store(130, Ordering::Relaxed);
        1
    } else {
        0
    }
}
#[cfg(windows)]
impl Cancellation {
    pub fn install() -> Self {
        unsafe {
            SetConsoleCtrlHandler(Some(cancel), 1);
            // CI launchers may pass down the inheritable Ctrl+C-ignore flag.
            // Register first, then restore normal signal delivery for this CLI.
            SetConsoleCtrlHandler(None, 0);
        }
        Self
    }
}
#[cfg(windows)]
impl Drop for Cancellation {
    fn drop(&mut self) {
        unsafe {
            SetConsoleCtrlHandler(Some(cancel), 0);
        }
    }
}
#[cfg(unix)]
pub struct Cancellation(libc::sighandler_t, libc::sighandler_t, libc::sighandler_t);
#[cfg(unix)]
extern "C" fn cancel(signal: i32) {
    CANCELLED.store(128 + signal, Ordering::Relaxed);
}
#[cfg(unix)]
impl Cancellation {
    pub fn install() -> Self {
        unsafe {
            Self(
                libc::signal(libc::SIGINT, cancel as libc::sighandler_t),
                libc::signal(libc::SIGTERM, cancel as libc::sighandler_t),
                libc::signal(libc::SIGHUP, cancel as libc::sighandler_t),
            )
        }
    }
}
#[cfg(unix)]
impl Drop for Cancellation {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, self.0);
            libc::signal(libc::SIGTERM, self.1);
            libc::signal(libc::SIGHUP, self.2);
        }
    }
}
#[cfg(windows)]
mod native {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Info {
        size: [i16; 2],
        cursor: [i16; 2],
        attributes: u16,
        window: [i16; 4],
        max: [i16; 2],
    }
    #[repr(C)]
    #[derive(Default)]
    struct Cursor {
        size: u32,
        visible: i32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(kind: u32) -> *mut c_void;
        fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: *mut c_void, mode: u32) -> i32;
        fn GetConsoleScreenBufferInfo(handle: *mut c_void, info: *mut Info) -> i32;
        fn GetConsoleCursorInfo(handle: *mut c_void, info: *mut Cursor) -> i32;
        fn SetConsoleCursorInfo(handle: *mut c_void, info: *const Cursor) -> i32;
    }
    pub struct Console {
        handle: *mut c_void,
        original: u32,
        cursor_size: u32,
    }
    // Only the renderer thread accesses this process-owned console handle.
    unsafe impl Send for Console {}
    impl Console {
        pub fn open() -> Option<Self> {
            unsafe {
                let handle = GetStdHandle(-12i32 as u32);
                let mut original = 0;
                if GetConsoleMode(handle, &mut original) == 0
                    || SetConsoleMode(handle, original | 4) == 0
                {
                    return None;
                }
                let mut cursor = Cursor::default();
                let cursor_size = if GetConsoleCursorInfo(handle, &mut cursor) != 0 {
                    cursor.size
                } else {
                    25
                };
                Some(Self {
                    handle,
                    original,
                    cursor_size,
                })
            }
        }
        pub fn size(&self) -> Option<(usize, usize)> {
            let mut info = Info::default();
            if unsafe { GetConsoleScreenBufferInfo(self.handle, &mut info) } == 0 {
                return None;
            }
            Some((
                (info.window[2] - info.window[0] + 1) as usize,
                (info.window[3] - info.window[1] + 1) as usize,
            ))
        }
    }
    impl Drop for Console {
        fn drop(&mut self) {
            unsafe {
                SetConsoleCursorInfo(
                    self.handle,
                    &Cursor {
                        size: self.cursor_size,
                        visible: 1,
                    },
                );
                SetConsoleMode(self.handle, self.original);
            }
        }
    }
}
#[cfg(unix)]
mod native {
    pub struct Console;
    impl Console {
        pub fn open() -> Option<Self> {
            let c = Self;
            c.size().map(|_| c)
        }
        pub fn size(&self) -> Option<(usize, usize)> {
            let mut s = libc::winsize {
                ws_row: 0,
                ws_col: 0,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            if unsafe { libc::ioctl(2, libc::TIOCGWINSZ, &mut s) } != 0 {
                return None;
            }
            Some((usize::from(s.ws_col), usize::from(s.ws_row)))
        }
    }
}
pub use native::Console;
