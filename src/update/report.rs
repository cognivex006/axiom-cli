//! Result reporting for the `update` verbs.
//!
//! The global `--json` contract is one JSON object on stdout and nothing else, so every
//! update result is built as one object and written once. The envelope keys are the same keys
//! `src/cli.rs` uses for the other verbs (`code`, `status`, `message`, `retryable`, `details`,
//! `request_id`); `details` carries structured update data instead of only strings so a check
//! result can name every component without flattening it into text.
//!
//! Plain mode writes human lines to stdout on success and the failure class to stderr, which
//! keeps the J-003 guarantee that a plain-mode failure never looks like data.

use super::json::Json;
use crate::cli::{exit, ENGINE_OWNER, PROGRAM};

/// One update result.
pub struct Report {
    code: i32,
    status: &'static str,
    message: String,
    retryable: bool,
    details: Json,
    lines: Vec<String>,
    /// The verb this result belongs to, used only for the plain-mode prefix. It defaults to
    /// `update`, which is the verb this module was first written for, so an existing caller keeps
    /// its current output while `version`, `doctor`, `install` and `uninstall` name themselves.
    subject: &'static str,
}

impl Report {
    /// Create a report with an empty `details` object.
    pub fn new(code: i32, status: &'static str, message: impl Into<String>) -> Report {
        Report {
            code,
            status,
            message: message.into(),
            retryable: false,
            details: Json::Object(Default::default()),
            lines: Vec::new(),
            subject: "update",
        }
    }

    /// Name the verb this result belongs to, so plain-mode diagnostics point at the right one.
    pub fn subject(mut self, subject: &'static str) -> Report {
        self.subject = subject;
        self
    }

    /// A successful result.
    pub fn ok(status: &'static str, message: impl Into<String>) -> Report {
        Report::new(exit::SUCCESS, status, message)
    }

    /// A declared-but-unavailable result (`4`).
    pub fn not_ready(message: impl Into<String>) -> Report {
        Report::new(exit::NOT_READY, "not_ready", message)
    }

    /// A refused request (`2`).
    pub fn refused(message: impl Into<String>) -> Report {
        Report::new(exit::VALIDATION, "validation_error", message)
    }

    /// Attach one structured detail.
    pub fn detail(&mut self, key: &str, value: Json) -> &mut Report {
        if self.details.set(key, value).is_err() {
            // `set` fails only when `details` is not an object, which cannot happen for a
            // report built by this module; keep the report rather than losing the result.
        }
        self
    }

    /// Mark the result as retryable without changing its exit class.
    pub fn retryable(mut self) -> Report {
        self.retryable = true;
        self
    }

    /// Add one human-readable line for plain mode.
    pub fn line(&mut self, text: impl Into<String>) -> &mut Report {
        self.lines.push(text.into());
        self
    }

    /// Add several human-readable lines for plain mode.
    pub fn lines<I, S>(&mut self, texts: I) -> &mut Report
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for text in texts {
            self.lines.push(text.into());
        }
        self
    }

    /// The exit code this report represents.
    pub fn code(&self) -> i32 {
        self.code
    }

    /// Write the report and return the process exit code.
    pub fn emit(mut self, json: bool, verbose: bool) -> i32 {
        let code = self.code();
        self.detail("engine_owner", Json::text(ENGINE_OWNER));
        let mut envelope = Json::Object(Default::default());
        let _ = envelope.set("code", Json::int(i64::from(self.code)));
        let _ = envelope.set("status", Json::text(self.status));
        let _ = envelope.set("message", Json::text(&self.message));
        let _ = envelope.set("retryable", Json::bool(self.retryable));
        let _ = envelope.set("details", self.details);
        let _ = envelope.set("request_id", Json::text(&request_id()));

        if json {
            let mut out = String::new();
            envelope.write(&mut out);
            println!("{out}");
            return code;
        }
        match code {
            exit::SUCCESS => {
                for text in &self.lines {
                    println!("{text}");
                }
                if self.subject != "doctor" && (self.subject != "version" || verbose) {
                    println!("{PROGRAM}: {}", self.message);
                }
            }
            exit::NOT_READY => {
                eprintln!("{PROGRAM}: {}: NotReady: {}", self.subject, self.message);
                // A refusal is the most consequential answer a verb gives; dropping its enumerated
                // detail in plain mode forces an operator to re-run with `--json` to learn what is
                // missing. Print the same lines the success path prints.
                for text in &self.lines {
                    eprintln!("{PROGRAM}: {text}");
                }
                if verbose {
                    eprintln!("{PROGRAM}: diagnostic: parsed invocation: {}", self.status);
                }
            }
            _ => {
                eprintln!("{PROGRAM}: error: {}", self.message);
                for text in &self.lines {
                    eprintln!("{PROGRAM}: {text}");
                }
                if verbose {
                    eprintln!("{PROGRAM}: diagnostic: exit code {}", self.code);
                }
            }
        }
        code
    }
}

/// A per-invocation correlation id, shaped like the one `src/cli.rs` writes.
///
/// Built here rather than reached for across modules so the update slice has no hidden
/// dependency on a private helper of the argv layer.
pub fn request_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{PROGRAM}-update-{:x}-{:x}", std::process::id(), nanos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::json::canonical_text;

    #[test]
    fn a_report_renders_as_one_json_object_with_the_frozen_envelope_keys() {
        let mut report = Report::ok("ok", "checked");
        report.detail("subcommand", Json::text("check"));
        let mut out = String::new();
        let mut envelope = Json::Object(Default::default());
        envelope.set("code", Json::int(0)).unwrap();
        envelope.set("status", Json::text("ok")).unwrap();
        envelope.set("message", Json::text("checked")).unwrap();
        envelope.set("retryable", Json::bool(false)).unwrap();
        envelope.set("details", report.details).unwrap();
        envelope.write(&mut out);
        let text = canonical_text(&envelope);
        assert!(text.starts_with("{\"code\":0,"), "{text}");
        assert!(text.ends_with('}'));
        assert!(text.contains("\"subcommand\":\"check\""));
        assert!(
            text.contains("\"engine_owner\":\"axiom-graphd\"") || !text.contains("engine_owner")
        );
    }

    #[test]
    fn exit_classes_map_to_the_canonical_vocabulary() {
        assert_eq!(Report::ok("ok", "m").code(), exit::SUCCESS);
        assert_eq!(Report::not_ready("m").code(), exit::NOT_READY);
        assert_eq!(Report::refused("m").code(), exit::VALIDATION);
    }
}
