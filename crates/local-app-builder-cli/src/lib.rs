//! The `local-app-builder` command line.
//!
//! The binary is a thin shell over [`run`]: it takes the arguments, an [`Env`] (the environment variables and the
//! `PATH` the command may look at) and two writers, and returns the exit code. Nothing here reads the process
//! environment directly, so every path is testable.

mod data_root;
mod doctor;
mod toolchain_command;

pub use data_root::{resolve_data_root, DataRoot, DataRootSource};
pub use doctor::{run_doctor, run_doctor_with, Check, CheckStatus, Report};
pub use toolchain_command::run_toolchain;

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

/// What the command may see of its surroundings.
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: HashMap<String, String>,
    path: Vec<PathBuf>,
}

impl Env {
    /// The real process environment.
    #[must_use]
    pub fn from_process() -> Self {
        let vars: HashMap<String, String> = std::env::vars().collect();
        let path = vars
            .get("PATH")
            .map(|p| std::env::split_paths(p).collect())
            .unwrap_or_default();
        Self { vars, path }
    }

    /// An environment built by hand (tests).
    #[must_use]
    pub fn new(vars: impl IntoIterator<Item = (String, String)>, path: Vec<PathBuf>) -> Self {
        Self {
            vars: vars.into_iter().collect(),
            path,
        }
    }

    /// One variable, if set and non-empty.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// The directories searched for programs.
    #[must_use]
    pub fn path(&self) -> &[PathBuf] {
        &self.path
    }
}

/// The crate's version, which is the CLI's.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
local-app-builder — check this machine and the Node and pnpm toolchain Local App builds run with

USAGE:
    local-app-builder <COMMAND> [OPTIONS]

COMMANDS:
    doctor     Check this machine and the data root
    toolchain  Install or check the Node and pnpm that builds run with
    version    Print the version
    help       Print this message

OPTIONS (doctor, toolchain):
    --data-root <DIR>   Use this data root instead of the default

USAGE (toolchain):
    local-app-builder toolchain status
    local-app-builder toolchain install [--from <DIR>]   (--from: take the pinned archives from a directory, no network)

OPTIONS (doctor):
    --json              Print the report as JSON
";

/// Run one invocation. Returns the process exit code: 0 success, 1 a failed check, 2 a usage error.
pub fn run(args: &[String], env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let Some((command, rest)) = args.split_first() else {
        let _ = err.write_all(USAGE.as_bytes());
        return 2;
    };
    match command.as_str() {
        "version" | "--version" | "-V" => {
            if !rest.is_empty() {
                let _ = writeln!(err, "local-app-builder version takes no arguments");
                return 2;
            }
            let _ = writeln!(out, "local-app-builder {VERSION}");
            0
        }
        "help" | "--help" | "-h" => {
            let _ = out.write_all(USAGE.as_bytes());
            0
        }
        "doctor" => doctor_command(rest, env, out, err),
        "toolchain" => run_toolchain(
            rest,
            env,
            local_app_builder_host::Platform::host().map(local_app_builder_host::Spec::pinned),
            out,
            err,
        ),
        other => {
            let _ = writeln!(err, "local-app-builder: unknown command `{other}`\n");
            let _ = err.write_all(USAGE.as_bytes());
            2
        }
    }
}

fn doctor_command(args: &[String], env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let mut flag_root: Option<String> = None;
    let mut json = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--data-root" => match it.next() {
                Some(v) => flag_root = Some(v.clone()),
                None => {
                    let _ = writeln!(
                        err,
                        "local-app-builder doctor: --data-root needs a directory"
                    );
                    return 2;
                }
            },
            other => {
                let _ = writeln!(err, "local-app-builder doctor: unknown option `{other}`");
                return 2;
            }
        }
    }
    let root = match resolve_data_root(flag_root.as_deref(), env) {
        Ok(root) => root,
        Err(message) => {
            let _ = writeln!(err, "local-app-builder doctor: {message}");
            return 1;
        }
    };
    let report = run_doctor(&root, env);
    let rendered = if json {
        report.to_json()
    } else {
        report.to_text()
    };
    let _ = out.write_all(rendered.as_bytes());
    u8::from(report.has_failure())
}
