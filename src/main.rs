//! `axiom-cli` process entry.
//!
//! Thin on purpose: collect argv, hand it to [`cli::run`], exit with the code `cli`
//! returns. All parsing, the exit vocabulary and the `NotReady` contract live in
//! `src/cli.rs`.
//!
//! Invocation is program plus argv. Nothing here builds a shell string, reads a shell
//! profile or mutates `PATH`.

mod bundle;
mod cli;
mod commands;
mod confirm;
mod doctor;
mod elevation;
mod engine;
mod layout;
mod legacy;
mod lifecycle;
mod pathenv;
mod runtime;
mod target;
mod update;
mod version;

fn main() {
    let arguments: Vec<_> = std::env::args_os().collect();
    let code = commands::run(&arguments).unwrap_or_else(|| cli::run(arguments));
    std::process::exit(code);
}
