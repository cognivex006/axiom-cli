//! The single interactive confirmation of ADR-0033 decision 4 (task L-003).
//!
//! A bare `install` or `uninstall` prints its plan and asks once. The answer is turned into the
//! plan's own canonical digest by the caller, so the approval stays bound to exactly the plan that
//! was printed (docs/16 section 7). Automation says `--yes` (or `AXIOM_INSTALL_YES=1`); without it
//! and without a terminal the caller prints the plan, changes nothing and exits `4`.

use std::io::{BufRead, IsTerminal, Write};

/// Environment equivalent of `--yes`.
pub const YES_ENV: &str = "AXIOM_INSTALL_YES";
/// Test seam: treat a piped stdin as the terminal so a harness can answer the real prompt.
pub const PROMPT_STDIN_TEST_ENV: &str = "AXIOM_CLI_TEST_PROMPT_STDIN";

/// How the single confirmation was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Approved by `--yes` / `AXIOM_INSTALL_YES=1`.
    PreApproved,
    /// Approved at the prompt; the plan was already printed.
    Confirmed,
    /// The prompt was answered with anything other than yes.
    Declined,
    /// No terminal to ask (or `--json`), and no `--yes`.
    NoTerminal,
}

impl Decision {
    /// Token recorded in the report.
    pub fn name(self) -> &'static str {
        match self {
            Decision::PreApproved => "pre-approved",
            Decision::Confirmed => "confirmed",
            Decision::Declined => "declined",
            Decision::NoTerminal => "no-terminal",
        }
    }
}

/// Whether `AXIOM_INSTALL_YES` asks for a pre-approved run.
pub fn yes_from_env() -> bool {
    matches!(
        std::env::var(YES_ENV).ok().as_deref().map(str::trim),
        Some("1") | Some("true") | Some("yes")
    )
}

/// Decide once. Only an interactive terminal on stdin is ever prompted.
pub fn decide(yes: bool, json: bool, plan: &[String]) -> Decision {
    crate::progress::suspend();
    if yes || yes_from_env() {
        return Decision::PreApproved;
    }
    let prompt_seam = std::env::var_os(PROMPT_STDIN_TEST_ENV).is_some();
    if json || !(std::io::stdin().is_terminal() || prompt_seam) {
        return Decision::NoTerminal;
    }
    let mut out = std::io::stdout();
    for line in plan {
        let _ = writeln!(out, "{line}");
    }
    let _ = write!(out, "Proceed? [Y/n] ");
    let _ = out.flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return Decision::Declined;
    }
    if accepts(&answer) {
        Decision::Confirmed
    } else {
        Decision::Declined
    }
}

/// `Y` is the default: an empty answer, `y` or `yes` accepts; anything else declines.
pub fn accepts(answer: &str) -> bool {
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_answer_accepts_and_anything_else_declines() {
        for yes in ["", "\n", "y", "Y\r\n", "yes", " YES "] {
            assert!(accepts(yes), "{yes:?}");
        }
        for no in ["n", "no", "N\n", "q", "yess", "0"] {
            assert!(!accepts(no), "{no:?}");
        }
    }

    #[test]
    fn json_without_yes_never_prompts() {
        if !yes_from_env() {
            assert_eq!(decide(false, true, &[]), Decision::NoTerminal);
        }
        assert_eq!(decide(true, true, &[]), Decision::PreApproved);
    }
}
