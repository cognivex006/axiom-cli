//! Announced, reversible per-user PATH change (ADR-0033 decision 5, task L-003).
//!
//! Only the install's own `bin` is ever added, only to the *user* PATH, and only when the plan the
//! user confirmed says so. The exact change is recorded in `<root>/path-change.json`, so uninstall
//! removes exactly that entry and nothing else, restoring the original value byte for byte when
//! nothing else changed it in between.
//!
//! * Windows: the `Path` value of `HKCU\Environment`, read and written through `advapi32` so the
//!   value type (`REG_EXPAND_SZ` or `REG_SZ`) is preserved, followed by a `WM_SETTINGCHANGE`
//!   broadcast so new terminals see it. The machine PATH (`HKLM`) is never opened.
//! * POSIX: one marked line appended to the login profile of the user's shell
//!   (`~/.zprofile` for zsh, `~/.bash_profile` when it exists for bash, else `~/.profile`).
//!
//! `AXIOM_CLI_TEST_USER_ENV_KEY` (Windows) redirects the registry key below `HKCU` so tests never
//! touch the real user environment; on POSIX tests redirect `HOME`.

use std::path::{Path, PathBuf};

use crate::update::error::Refusal;
use crate::update::json::{self, Json};
use crate::update::state;

/// Record of the change, inside the install root.
pub const RECORD_FILE: &str = "path-change.json";
/// Test seam: registry key below HKCU used instead of `Environment`.
#[cfg_attr(not(windows), allow(dead_code))]
pub const TEST_KEY_ENV: &str = "AXIOM_CLI_TEST_USER_ENV_KEY";
/// The marker comment written above the POSIX profile line.
pub const PROFILE_MARKER: &str = "# Added by axiom-cli install; removed by axiom-cli uninstall";

/// What the plan announces for the PATH.
pub fn plan_block(bin: &Path, modify: bool) -> Json {
    let target = target_description();
    let present = modify && current_contains(bin).unwrap_or(false);
    Json::from_pairs(vec![
        ("scope", Json::text("per-user")),
        ("entry", Json::text(&bin.display().to_string())),
        ("target", Json::text(&target)),
        (
            "action",
            Json::text(if !modify {
                "skipped"
            } else if present {
                "already-present"
            } else {
                "add"
            }),
        ),
    ])
}

/// One plain line for the printed plan.
pub fn plan_line(block: &Json) -> String {
    let entry = block.get("entry").and_then(Json::as_text).unwrap_or("?");
    let target = block.get("target").and_then(Json::as_text).unwrap_or("?");
    match block.get("action").and_then(Json::as_text) {
        Some("add") => format!(
            "PATH: add {entry} to {target} (undo: axiom-cli uninstall; skip: --no-modify-path)"
        ),
        Some("already-present") => format!("PATH: {entry} is already on {target}; unchanged"),
        _ => format!(
            "PATH: unchanged (--no-modify-path); run {entry}{}axiom-cli directly",
            std::path::MAIN_SEPARATOR
        ),
    }
}

fn target_description() -> String {
    if cfg!(windows) {
        format!("the user PATH (HKCU\\{})", windows::key_name())
    } else {
        match posix::profile_path() {
            Some(path) => format!("the login profile {}", path.display()),
            None => "the login profile (HOME unresolved)".to_string(),
        }
    }
}

fn current_contains(bin: &Path) -> Result<bool, Refusal> {
    let entry = bin.display().to_string();
    if cfg!(windows) {
        Ok(windows::read()?
            .map(|(value, _)| split(&value).any(|part| same_entry(part, &entry)))
            .unwrap_or(false))
    } else {
        let Some(profile) = posix::profile_path() else {
            return Ok(false);
        };
        Ok(std::fs::read_to_string(profile)
            .map(|text| text.contains(&posix::line(bin)))
            .unwrap_or(false))
    }
}

fn split(value: &str) -> impl Iterator<Item = &str> {
    value.split(';')
}

fn same_entry(a: &str, b: &str) -> bool {
    let trim = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_string();
    if cfg!(windows) {
        trim(a).eq_ignore_ascii_case(&trim(b))
    } else {
        trim(a) == trim(b)
    }
}

/// Apply the announced change and record it. Returns the record.
pub fn apply(root: &Path, block: &Json) -> Result<Json, Refusal> {
    if block.get("action").and_then(Json::as_text) != Some("add") {
        return Ok(block.clone());
    }
    let bin = PathBuf::from(block.get("entry").and_then(Json::as_text).unwrap_or(""));
    let record = if cfg!(windows) {
        windows::add(&bin)?
    } else {
        posix::add(&bin)?
    };
    let path = root.join(RECORD_FILE);
    state::write_atomic(
        &path.with_extension("json.tmp"),
        &path,
        json::canonical_text(&record).as_bytes(),
    )?;
    Ok(record)
}

/// Take ownership of a PATH entry a legacy installer added, so uninstall removes it.
pub fn adopt(root: &Path, block: &Json) -> Result<(), Refusal> {
    let path = root.join(RECORD_FILE);
    if path.exists() || !cfg!(windows) {
        return Ok(());
    }
    let entry = block.get("entry").and_then(Json::as_text).unwrap_or("");
    let record = Json::from_pairs(vec![
        ("platform", Json::text("windows")),
        ("key", Json::text(&format!("HKCU\\{}", windows::key_name()))),
        ("entry", Json::text(entry)),
        ("adopted", Json::bool(true)),
        ("value_existed", Json::bool(true)),
        ("separator_added", Json::bool(true)),
    ]);
    state::write_atomic(
        &path.with_extension("json.tmp"),
        &path,
        json::canonical_text(&record).as_bytes(),
    )
}

/// Remove exactly the recorded change. Absent record: nothing to do.
pub fn revert(root: &Path) -> Result<Option<Json>, Refusal> {
    let path = root.join(RECORD_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let record = json::parse(&std::fs::read_to_string(&path).map_err(|error| {
        Refusal::io(
            "path_record_unreadable",
            &path.display().to_string(),
            &error,
        )
    })?)
    .map_err(|error| Refusal::validation("path_record_invalid", error))?;
    let outcome = if cfg!(windows) {
        windows::remove(&record)?
    } else {
        posix::remove(&record)?
    };
    std::fs::remove_file(&path)
        .map_err(|error| Refusal::io("path_record_remove", &path.display().to_string(), &error))?;
    Ok(Some(outcome))
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::c_void;

    type Hkey = *mut c_void;
    const HKEY_CURRENT_USER: Hkey = 0x8000_0001u32 as usize as Hkey;
    const KEY_READ: u32 = 0x20019;
    const KEY_WRITE: u32 = 0x20006;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            key: Hkey,
            sub: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            sam: u32,
            security: *const c_void,
            result: *mut Hkey,
            disposition: *mut u32,
        ) -> i32;
        fn RegOpenKeyExW(
            key: Hkey,
            sub: *const u16,
            options: u32,
            sam: u32,
            result: *mut Hkey,
        ) -> i32;
        fn RegQueryValueExW(
            key: Hkey,
            name: *const u16,
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            size: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            key: Hkey,
            name: *const u16,
            reserved: u32,
            kind: u32,
            data: *const u8,
            size: u32,
        ) -> i32;
        fn RegDeleteValueW(key: Hkey, name: *const u16) -> i32;
        fn RegCloseKey(key: Hkey) -> i32;
    }
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            hwnd: isize,
            msg: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn key_name() -> String {
        std::env::var(TEST_KEY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Environment".to_string())
    }

    struct Key(Hkey);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn open(write: bool) -> Result<Option<Key>, Refusal> {
        let name = wide(&key_name());
        let mut key: Hkey = std::ptr::null_mut();
        let status = unsafe {
            if write {
                let mut disposition = 0u32;
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    name.as_ptr(),
                    0,
                    std::ptr::null(),
                    0,
                    KEY_READ | KEY_WRITE,
                    std::ptr::null(),
                    &mut key,
                    &mut disposition,
                )
            } else {
                RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, KEY_READ, &mut key)
            }
        };
        match status {
            0 => Ok(Some(Key(key))),
            ERROR_FILE_NOT_FOUND if !write => Ok(None),
            code => Err(Refusal::io(
                "user_path_registry",
                &format!("HKCU\\{}", key_name()),
                &std::io::Error::from_raw_os_error(code),
            )),
        }
    }

    /// The user `Path` value and its registry type, or `None` when absent.
    pub fn read() -> Result<Option<(String, u32)>, Refusal> {
        let Some(key) = open(false)? else {
            return Ok(None);
        };
        let name = wide("Path");
        let (mut kind, mut size) = (0u32, 0u32);
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(Refusal::io(
                "user_path_read",
                "HKCU Path",
                &std::io::Error::from_raw_os_error(status),
            ));
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut bytes = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut kind,
                buffer.as_mut_ptr() as *mut u8,
                &mut bytes,
            )
        };
        if status != 0 {
            return Err(Refusal::io(
                "user_path_read",
                "HKCU Path",
                &std::io::Error::from_raw_os_error(status),
            ));
        }
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Err(Refusal::validation(
                "user_path_type",
                format!("the user Path value has registry type {kind}; only REG_SZ and REG_EXPAND_SZ are changed"),
            ));
        }
        let units = (bytes as usize / 2).min(buffer.len());
        let mut text = String::from_utf16_lossy(&buffer[..units]);
        while text.ends_with('\0') {
            text.pop();
        }
        Ok(Some((text, kind)))
    }

    fn write(value: Option<(&str, u32)>) -> Result<(), Refusal> {
        let key = open(true)?.expect("created");
        let name = wide("Path");
        let status = match value {
            Some((text, kind)) => {
                let data = wide(text);
                unsafe {
                    RegSetValueExW(
                        key.0,
                        name.as_ptr(),
                        0,
                        kind,
                        data.as_ptr() as *const u8,
                        (data.len() * 2) as u32,
                    )
                }
            }
            None => unsafe { RegDeleteValueW(key.0, name.as_ptr()) },
        };
        if status != 0 {
            return Err(Refusal::io(
                "user_path_write",
                "HKCU Path",
                &std::io::Error::from_raw_os_error(status),
            ));
        }
        let environment = wide("Environment");
        let mut result = 0usize;
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                environment.as_ptr(),
                SMTO_ABORTIFHUNG,
                2000,
                &mut result,
            )
        };
        Ok(())
    }

    pub fn add(bin: &Path) -> Result<Json, Refusal> {
        let entry = bin.display().to_string();
        let original = read()?;
        let (new, kind) = match &original {
            Some((value, kind)) if value.is_empty() => (entry.clone(), *kind),
            Some((value, kind)) if value.ends_with(';') => (format!("{value}{entry}"), *kind),
            Some((value, kind)) => (format!("{value};{entry}"), *kind),
            None => (entry.clone(), REG_EXPAND_SZ),
        };
        let separator_added =
            matches!(&original, Some((value, _)) if !value.is_empty() && !value.ends_with(';'));
        write(Some((&new, kind)))?;
        Ok(Json::from_pairs(vec![
            ("platform", Json::text("windows")),
            ("separator_added", Json::bool(separator_added)),
            ("key", Json::text(&format!("HKCU\\{}", key_name()))),
            ("entry", Json::text(&entry)),
            ("value_existed", Json::bool(original.is_some())),
            ("value_type", Json::int(kind as i64)),
            (
                "original_sha256",
                Json::text(&crate::update::sha256::hex(&crate::update::sha256::digest(
                    original.as_ref().map(|(v, _)| v.as_bytes()).unwrap_or(b""),
                ))),
            ),
            ("written", Json::text(&new)),
        ]))
    }

    pub fn remove(record: &Json) -> Result<Json, Refusal> {
        let entry = record
            .get("entry")
            .and_then(Json::as_text)
            .unwrap_or("")
            .to_string();
        let existed = record
            .get("value_existed")
            .and_then(Json::as_bool)
            .unwrap_or(true);
        let Some((value, kind)) = read()? else {
            return Ok(Json::from_pairs(vec![(
                "action",
                Json::text("already-absent"),
            )]));
        };
        let separator = record
            .get("separator_added")
            .and_then(Json::as_bool)
            .unwrap_or(true);
        let restored = remove_entry(&value, &entry, separator);
        let action = match restored {
            None => "not-present",
            Some(ref text) if text.is_empty() && !existed => {
                write(None)?;
                "value-deleted"
            }
            Some(ref text) => {
                write(Some((text, kind)))?;
                "entry-removed"
            }
        };
        Ok(Json::from_pairs(vec![
            ("action", Json::text(action)),
            ("entry", Json::text(&entry)),
        ]))
    }

    /// Remove exactly one occurrence of `entry`, undoing the separator `add` introduced
    /// (`separator_added`), so an untouched value is restored byte for byte.
    pub fn remove_entry(value: &str, entry: &str, separator_added: bool) -> Option<String> {
        if value == entry {
            return Some(String::new());
        }
        let suffix = if separator_added {
            format!(";{entry}")
        } else {
            entry.to_string()
        };
        if let Some(prefix) = value.strip_suffix(&suffix) {
            if separator_added || prefix.ends_with(';') {
                return Some(prefix.to_string());
            }
        }
        let parts: Vec<&str> = value.split(';').collect();
        let index = parts
            .iter()
            .position(|part| super::same_entry(part, entry))?;
        let mut kept = parts;
        kept.remove(index);
        Some(kept.join(";"))
    }
}

#[cfg(not(windows))]
mod windows {
    use super::*;
    pub fn key_name() -> String {
        "Environment".to_string()
    }
    pub fn read() -> Result<Option<(String, u32)>, Refusal> {
        Ok(None)
    }
    pub fn add(_: &Path) -> Result<Json, Refusal> {
        Err(Refusal::validation(
            "not_windows",
            "the Windows user PATH is not available here",
        ))
    }
    pub fn remove(_: &Json) -> Result<Json, Refusal> {
        Err(Refusal::validation(
            "not_windows",
            "the Windows user PATH is not available here",
        ))
    }
}

mod posix {
    use super::*;

    pub fn home() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
    }

    pub fn profile_path() -> Option<PathBuf> {
        let home = home()?;
        let shell = std::env::var("SHELL").unwrap_or_default();
        Some(if shell.ends_with("/zsh") {
            home.join(".zprofile")
        } else if shell.ends_with("/bash") && home.join(".bash_profile").is_file() {
            home.join(".bash_profile")
        } else {
            home.join(".profile")
        })
    }

    pub fn line(bin: &Path) -> String {
        format!("export PATH=\"{}:$PATH\"", bin.display())
    }

    pub fn add(bin: &Path) -> Result<Json, Refusal> {
        let profile = profile_path().ok_or_else(|| {
            Refusal::not_ready(
                "home_unresolved",
                "HOME is not set, so no login profile can carry the PATH line",
            )
        })?;
        let original = std::fs::read(&profile).ok();
        let mut block = String::new();
        if let Some(bytes) = &original {
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                block.push('\n');
            }
        }
        block.push_str(PROFILE_MARKER);
        block.push('\n');
        block.push_str(&line(bin));
        block.push('\n');
        let mut new = original.clone().unwrap_or_default();
        new.extend_from_slice(block.as_bytes());
        state::write_atomic(&profile.with_extension("axiom-tmp"), &profile, &new)?;
        Ok(Json::from_pairs(vec![
            ("platform", Json::text("posix")),
            ("profile", Json::text(&profile.display().to_string())),
            ("entry", Json::text(&bin.display().to_string())),
            ("profile_existed", Json::bool(original.is_some())),
            ("appended", Json::text(&block)),
        ]))
    }

    pub fn remove(record: &Json) -> Result<Json, Refusal> {
        let profile = PathBuf::from(record.get("profile").and_then(Json::as_text).unwrap_or(""));
        let appended = record
            .get("appended")
            .and_then(Json::as_text)
            .unwrap_or("")
            .to_string();
        let existed = record
            .get("profile_existed")
            .and_then(Json::as_bool)
            .unwrap_or(true);
        let Ok(bytes) = std::fs::read(&profile) else {
            return Ok(Json::from_pairs(vec![(
                "action",
                Json::text("already-absent"),
            )]));
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let restored = if let Some(prefix) = text.strip_suffix(&appended) {
            prefix.to_string()
        } else if let Some(index) = text.find(&appended) {
            format!("{}{}", &text[..index], &text[index + appended.len()..])
        } else {
            return Ok(Json::from_pairs(vec![(
                "action",
                Json::text("not-present"),
            )]));
        };
        if restored.is_empty() && !existed {
            std::fs::remove_file(&profile).map_err(|error| {
                Refusal::io("profile_remove", &profile.display().to_string(), &error)
            })?;
            return Ok(Json::from_pairs(vec![(
                "action",
                Json::text("profile-deleted"),
            )]));
        }
        state::write_atomic(
            &profile.with_extension("axiom-tmp"),
            &profile,
            restored.as_bytes(),
        )?;
        Ok(Json::from_pairs(vec![(
            "action",
            Json::text("line-removed"),
        )]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn removing_restores_the_value_add_produced() {
        use super::windows::remove_entry;
        let entry = r"C:\Users\u\AppData\Local\Axiom\bin";
        for original in ["", r"%USERPROFILE%\bin", r"C:\a;C:\b;", r"C:\a;C:\b"] {
            let added = if original.is_empty() {
                entry.to_string()
            } else if original.ends_with(';') {
                format!("{original}{entry}")
            } else {
                format!("{original};{entry}")
            };
            assert_eq!(
                remove_entry(
                    &added,
                    entry,
                    !original.is_empty() && !original.ends_with(';')
                )
                .as_deref(),
                Some(original),
                "{original}"
            );
        }
        assert_eq!(
            remove_entry(&format!(r"C:\x;{entry};C:\y"), entry, true).as_deref(),
            Some(r"C:\x;C:\y")
        );
        assert_eq!(remove_entry(r"C:\x", entry, true), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_registry_round_trip_is_byte_identical_under_a_test_key() {
        let key = format!("Software\\AxiomCliTest\\Env-{}", std::process::id());
        std::env::set_var(TEST_KEY_ENV, &key);
        let root = std::env::temp_dir().join(format!("axiom-path-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // Seed a REG_EXPAND_SZ value with an unexpanded variable.
        let seeded = windows::add(Path::new(r"%USERPROFILE%\tools")).unwrap();
        assert_eq!(
            seeded.get("value_existed").and_then(Json::as_bool),
            Some(false)
        );
        let before = windows::read().unwrap().unwrap();
        let bin = root.join("bin");
        let block = plan_block(&bin, true);
        assert_eq!(block.get("action").and_then(Json::as_text), Some("add"));
        apply(&root, &block).unwrap();
        let during = windows::read().unwrap().unwrap();
        assert!(during.0.ends_with(&bin.display().to_string()));
        assert_eq!(
            plan_block(&bin, true).get("action").and_then(Json::as_text),
            Some("already-present")
        );
        revert(&root).unwrap().unwrap();
        assert_eq!(windows::read().unwrap().unwrap(), before);
        assert!(!root.join(RECORD_FILE).exists());
        std::env::remove_var(TEST_KEY_ENV);
    }

    #[cfg(not(windows))]
    #[test]
    fn the_profile_round_trip_is_byte_identical() {
        let home = std::env::temp_dir().join(format!("axiom-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("SHELL", "/bin/sh");
        std::fs::write(home.join(".profile"), b"umask 022").unwrap();
        let root = home.join("root");
        std::fs::create_dir_all(&root).unwrap();
        apply(&root, &plan_block(&root.join("bin"), true)).unwrap();
        assert!(std::fs::read_to_string(home.join(".profile"))
            .unwrap()
            .contains(PROFILE_MARKER));
        revert(&root).unwrap();
        assert_eq!(std::fs::read(home.join(".profile")).unwrap(), b"umask 022");
    }

    #[cfg(not(windows))]
    #[test]
    fn removal_accepts_a_recorded_legacy_profile_comment() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("axiom-profile-legacy-{unique}"));
        std::fs::create_dir_all(&root).unwrap();
        let profile = root.join(".profile");
        let legacy = "# Added by axiom-cli install (ADR-0033); removed by axiom-cli uninstall\nexport PATH=owned-bin:$PATH\n";
        std::fs::write(&profile, format!("umask 022\n{legacy}# user edit\n")).unwrap();
        let record = Json::from_pairs(vec![
            ("profile", Json::text(&profile.display().to_string())),
            ("appended", Json::text(legacy)),
            ("profile_existed", Json::bool(true)),
        ]);
        posix::remove(&record).unwrap();
        assert_eq!(
            std::fs::read_to_string(&profile).unwrap(),
            "umask 022\n# user edit\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_skipped_change_writes_nothing() {
        let root = std::env::temp_dir().join(format!("axiom-path-skip-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let block = plan_block(&root.join("bin"), false);
        assert_eq!(block.get("action").and_then(Json::as_text), Some("skipped"));
        apply(&root, &block).unwrap();
        assert!(!root.join(RECORD_FILE).exists());
        assert!(revert(&root).unwrap().is_none());
    }
}
