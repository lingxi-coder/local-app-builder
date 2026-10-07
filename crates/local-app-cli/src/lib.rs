//! The `local-app` command line.
//!
//! The binary is a thin shell over [`run`]: it takes the arguments, an [`Env`] (the environment variables and the
//! `PATH` the command may look at) and two writers, and returns the exit code. Nothing here reads the process
//! environment directly, so every path is testable.

mod data_root;
mod doctor;
mod mcp_backend;
mod mcp_protocol;
mod mcp_stdio;
mod open_lock;
mod writer_lock;

pub use data_root::{resolve_data_root, DataRoot, DataRootSource};
pub use doctor::{run_doctor, Check, CheckStatus, Report};
pub use mcp_backend::{LocalAppBackend, UnsupportedHost, UNSUPPORTED};
pub use mcp_protocol::{
    CallError, Reply, ServerIdentity, Session, ToolBackend, ToolResult, ToolSpec, LEGACY_VERSION, MODERN_VERSION,
    SUPPORTED_VERSIONS,
};
pub use mcp_stdio::{serve, DRAIN_GRACE, MAX_MESSAGE_BYTES};
pub use open_lock::{OpenLock, OPEN_LOCK_FILE, OPEN_LOCK_TIMEOUT};
pub use writer_lock::{Attempt as WriterAttempt, Holder as WriterHolder, WriterLock, WRITER_LOCK_FILE};

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
        Self { vars: vars.into_iter().collect(), path }
    }

    /// One variable, if set and non-empty.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str).filter(|v| !v.is_empty())
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
local-app — build and run Local Apps outside LingXi

USAGE:
    local-app <COMMAND> [OPTIONS]

COMMANDS:
    mcp        Serve the Local App tools to an MCP client over stdio
    doctor     Check this machine and the data root
    version    Print the version
    help       Print this message

OPTIONS (mcp, doctor):
    --data-root <DIR>   Use this data root instead of the default

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
                let _ = writeln!(err, "local-app version takes no arguments");
                return 2;
            }
            let _ = writeln!(out, "local-app {VERSION}");
            0
        }
        "help" | "--help" | "-h" => {
            let _ = out.write_all(USAGE.as_bytes());
            0
        }
        "doctor" => doctor_command(rest, env, out, err),
        "mcp" => mcp_command(rest, env, err),
        other => {
            let _ = writeln!(err, "local-app: unknown command `{other}`\n");
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
                    let _ = writeln!(err, "local-app doctor: --data-root needs a directory");
                    return 2;
                }
            },
            other => {
                let _ = writeln!(err, "local-app doctor: unknown option `{other}`");
                return 2;
            }
        }
    }
    let root = match resolve_data_root(flag_root.as_deref(), env) {
        Ok(root) => root,
        Err(message) => {
            let _ = writeln!(err, "local-app doctor: {message}");
            return 1;
        }
    };
    let report = run_doctor(&root, env);
    let rendered = if json { report.to_json() } else { report.to_text() };
    let _ = out.write_all(rendered.as_bytes());
    u8::from(report.has_failure())
}

/// The instructions an MCP client shows the model.
const MCP_INSTRUCTIONS: &str = "Local App tools. This server can list and read Local Apps; it cannot build, run, \
or change them yet. A tool that needs a runtime, a screen or the person's approval answers \
`unsupported_on_this_host`.";

fn mcp_command(args: &[String], env: &Env, err: &mut dyn Write) -> u8 {
    let mut flag_root: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--data-root" => match it.next() {
                Some(v) => flag_root = Some(v.clone()),
                None => {
                    let _ = writeln!(err, "local-app mcp: --data-root needs a directory");
                    return 2;
                }
            },
            other => {
                let _ = writeln!(err, "local-app mcp: unknown option `{other}`");
                return 2;
            }
        }
    }
    let root = match resolve_data_root(flag_root.as_deref(), env) {
        Ok(root) => root,
        Err(message) => {
            let _ = writeln!(err, "local-app mcp: {message}");
            return 1;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = writeln!(err, "local-app mcp: cannot start the async runtime: {error}");
            return 1;
        }
    };
    let result = runtime.block_on(async {
        // Held only while the store opens: see `open_lock`.
        let lock_root = root.path.clone();
        let lock = tokio::task::spawn_blocking(move || OpenLock::acquire(&lock_root, OPEN_LOCK_TIMEOUT))
            .await
            .map_err(|error| format!("waiting for the data root: {error}"))??;
        let backend = LocalAppBackend::open(&root.path).await;
        drop(lock);
        let backend = backend?;
        let identity = ServerIdentity {
            name: "local-app".into(),
            version: VERSION.into(),
            instructions: Some(MCP_INSTRUCTIONS.into()),
        };
        let session = std::sync::Arc::new(Session::new(std::sync::Arc::new(backend), identity));
        serve(session, tokio::io::stdin(), tokio::io::stdout()).await.map_err(|error| format!("stdio failed: {error}"))
    });
    match result {
        Ok(()) => 0,
        Err(message) => {
            let _ = writeln!(err, "local-app mcp: {message}");
            1
        }
    }
}
