//! The `local-app-builder` command line.
//!
//! The binary is a thin shell over [`run`]: it takes the arguments, an [`Env`] (the environment variables and the
//! `PATH` the command may look at) and two writers, and returns the exit code. Nothing here reads the process
//! environment directly, so every path is testable.

mod data_root;
mod doctor;
mod lease;
mod local_host;
mod mcp_backend;
mod mcp_protocol;
mod mcp_stdio;
mod toolchain_command;
mod writer_lock;

pub use data_root::{resolve_data_root, DataRoot, DataRootSource};
pub use doctor::{run_doctor, run_doctor_with, Check, CheckStatus, Report};
pub use lease::{Lease, LeaseError, Use, LEASE_IDLE, LEASE_WAIT};
pub use local_host::{within_call, ApprovalSink, CatalogBundle, HostConfig, LocalHost};
pub use mcp_backend::{LocalAppBackend, MAX_PLAN_BYTES, SERVED_READ, SERVED_WRITE};
pub use mcp_protocol::{
    Approval, ApprovalRequest, Approver, CallContext, CallError, ClientLink, LinkError, NoApprover, Reply, ServerIdentity,
    Session, ToolBackend, ToolResult, ToolSpec, APPROVAL_TIMEOUT, LEGACY_VERSION, MODERN_VERSION,
    SUPPORTED_VERSIONS,
};
pub use mcp_stdio::{serve, DRAIN_GRACE, MAX_MESSAGE_BYTES};
pub use toolchain_command::run_toolchain;
pub use writer_lock::{Attempt as WriterAttempt, Holder as WriterHolder, WriterLock, WRITER_LOCK_FILE};

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

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
local-app-builder — build and run Local Apps outside LingXi

USAGE:
    local-app-builder <COMMAND> [OPTIONS]

COMMANDS:
    mcp        Serve the Local App tools to an MCP client over stdio
    doctor     Check this machine and the data root
    toolchain  Install or check the Node and pnpm that builds run with
    version    Print the version
    help       Print this message

OPTIONS (mcp, doctor, toolchain):
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
        "mcp" => mcp_command(rest, env, err),
        "toolchain" => run_toolchain(rest, env, local_app_builder_host::Platform::host().map(local_app_builder_host::Spec::pinned), out, err),
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
                    let _ = writeln!(err, "local-app-builder doctor: --data-root needs a directory");
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
    let rendered = if json { report.to_json() } else { report.to_text() };
    let _ = out.write_all(rendered.as_bytes());
    u8::from(report.has_failure())
}

/// How builds run on this machine: the pinned toolchain under the data root, checked on first use. A machine with no
/// pinned toolchain (not a Mac) has no executor, and builds say so.
fn host_config(root: &std::path::Path, env: &Env) -> HostConfig {
    let executor = local_app_builder_host::Platform::host().ok().map(|platform| {
        let mut private = vec![root.to_path_buf()];
        private.extend(env.var("HOME").map(PathBuf::from));
        Arc::new(local_app_builder_host::ProvisionedExecutor::new(
            local_app_builder_host::Toolchains::in_data_root(root),
            local_app_builder_host::Spec::pinned(platform),
            private,
        )) as Arc<dyn local_app_builder_service::host::BuildExecutor>
    });
    HostConfig { executor }
}

/// The instructions an MCP client shows the model. They name the data root, because an app's workspace is a directory
/// under it that the model edits with its own file tools.
fn mcp_instructions(root: &std::path::Path) -> String {
    format!(
        "Local App tools. This server lists Local Apps and reads an app's record, logs, checkpoints and background \
tasks, and it can create an app, prepare it from a plan the person approves, install its dependencies and build it. \
Approval is asked of the person through the client (MCP elicitation); a client that cannot ask cannot approve, and \
then the action is not done. It cannot run an app or drive its screen yet. The data root is {}: an app's workspace is \
<data root>/<the `workspaceRel` that LocalAppGet reports>, and its LINGXI.md there is the app's own contract.",
        root.display()
    )
}

fn mcp_command(args: &[String], env: &Env, err: &mut dyn Write) -> u8 {
    let mut flag_root: Option<String> = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--data-root" => match it.next() {
                Some(v) => flag_root = Some(v.clone()),
                None => {
                    let _ = writeln!(err, "local-app-builder mcp: --data-root needs a directory");
                    return 2;
                }
            },
            other => {
                let _ = writeln!(err, "local-app-builder mcp: unknown option `{other}`");
                return 2;
            }
        }
    }
    let root = match resolve_data_root(flag_root.as_deref(), env) {
        Ok(root) => root,
        Err(message) => {
            let _ = writeln!(err, "local-app-builder mcp: {message}");
            return 1;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = writeln!(err, "local-app-builder mcp: cannot start the async runtime: {error}");
            return 1;
        }
    };
    let result = runtime.block_on(async {
        let backend = LocalAppBackend::open(&root.path, host_config(&root.path, env)).await?;
        let identity = ServerIdentity {
            name: "local-app-builder".into(),
            version: VERSION.into(),
            instructions: Some(mcp_instructions(&root.path)),
        };
        let session = std::sync::Arc::new(Session::new(std::sync::Arc::new(backend), identity));
        serve(session, tokio::io::stdin(), tokio::io::stdout()).await.map_err(|error| format!("stdio failed: {error}"))
    });
    match result {
        Ok(()) => 0,
        Err(message) => {
            let _ = writeln!(err, "local-app-builder mcp: {message}");
            1
        }
    }
}
