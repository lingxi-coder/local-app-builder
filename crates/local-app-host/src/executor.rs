//! Runs the service's isolated commands on this Mac.
//!
//! [`LocalExecutor`] is the service's `BuildExecutor` without a guest. It translates the command's guest paths to
//! host paths ([`crate::path_map`]), starts it under a seatbelt profile ([`crate::seatbelt`]) in its own process
//! group with only the environment the service gave it, and watches it until it exits, runs out of time, runs out of
//! memory ([`crate::watchdog`]) or is dropped.
//!
//! What the receipt says is what happened. `network_policy_enforced` is true only because the command was started
//! under a profile that applies the policy; when the profile cannot be applied the command is **not run** and the
//! error says so, because the service never accepts an unisolated build. `memory_limit_enforced` is true only when the
//! watchdog read the group's memory at the start and the command was stopped if it passed the ceiling; it is a sampled
//! limit, so a command can overshoot by what it allocates between two samples.
//!
//! A limit the executor cannot enforce (anything but memory) is refused rather than ignored: a command that was asked
//! to be bounded and was not is worse than one that did not run.

use crate::path_map::{PathMap, Toolchain};
use crate::seatbelt::{Policy, Profile, SANDBOX_EXEC};
use crate::watchdog::{kill_group, kill_group_blocking, sample, SAMPLE_INTERVAL};
use async_trait::async_trait;
use local_app_contracts::execution::{CommandOutcome, Enforcement, IsolatedCommand, ResourceLimits};
use local_app_service::host::BuildExecutor;
use std::collections::BTreeMap;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::Instant;

/// Exit code reported for a command that was stopped because it ran too long (the one `timeout(1)` uses).
pub const TIMED_OUT_EXIT_CODE: i32 = 124;
/// Exit code reported for a command that was killed (`128 + SIGKILL`).
pub const KILLED_EXIT_CODE: i32 = 137;
/// How much of each output stream is kept; when a command writes more, the end is kept.
pub const OUTPUT_CAP_BYTES: usize = 2 * 1024 * 1024;

/// Where this executor finds what it runs, and what it keeps a command away from.
#[derive(Debug, Clone)]
pub struct LocalExecutorConfig {
    /// The programs a command may run.
    pub toolchain: Toolchain,
    /// The directory holding the whole toolchain (its `bin` and the libraries the programs load); readable.
    pub toolchain_root: PathBuf,
    /// Roots whose contents a command cannot read except under its mounts and the toolchain: the data root, so one
    /// app's build cannot read another's, and the home directory.
    pub private_roots: Vec<PathBuf>,
    /// How often memory is sampled.
    pub sample_interval: Duration,
}

impl LocalExecutorConfig {
    /// The usual configuration: `toolchain_root` holds `bin/`, and the data root and home are private.
    #[must_use]
    pub fn new(toolchain_root: PathBuf, private_roots: Vec<PathBuf>) -> Self {
        Self {
            toolchain: Toolchain { guest_bin: "/usr/bin".into(), host_bin: toolchain_root.join("bin") },
            toolchain_root,
            private_roots,
            sample_interval: SAMPLE_INTERVAL,
        }
    }
}

/// The service's build executor on a Mac.
#[derive(Debug, Clone)]
pub struct LocalExecutor {
    config: LocalExecutorConfig,
}

impl LocalExecutor {
    /// An executor for `config`.
    #[must_use]
    pub fn new(config: LocalExecutorConfig) -> Self {
        Self { config }
    }
}

/// Kills the process group when dropped, so a cancelled run leaves nothing behind.
struct GroupGuard {
    pgid: u32,
    armed: bool,
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if self.armed {
            kill_group_blocking(self.pgid);
        }
    }
}

/// The end of a stream, bounded.
async fn read_tail(mut stream: impl AsyncRead + Unpin) -> String {
    let mut kept: Vec<u8> = Vec::new();
    let mut dropped: u64 = 0;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                kept.extend_from_slice(&chunk[..n]);
                if kept.len() > OUTPUT_CAP_BYTES {
                    let excess = kept.len() - OUTPUT_CAP_BYTES;
                    kept.drain(..excess);
                    dropped += excess as u64;
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&kept).into_owned();
    if dropped > 0 {
        format!("[{dropped} bytes of earlier output were dropped]\n{text}")
    } else {
        text
    }
}

fn unsupported_limits(limits: ResourceLimits) -> Option<&'static str> {
    if limits.max_cpu_seconds.is_some() {
        Some("max_cpu_seconds")
    } else if limits.max_processes.is_some() {
        Some("max_processes")
    } else if limits.max_open_files.is_some() {
        Some("max_open_files")
    } else {
        None
    }
}

/// Everything about a command that can be decided before anything runs.
struct Plan {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    profile: Profile,
}

impl LocalExecutor {
    fn plan(&self, command: &IsolatedCommand) -> Result<Plan, String> {
        if let Some(limit) = unsupported_limits(command.limits) {
            return Err(format!("unsupported_limit: this host cannot enforce {limit}, and a limit it cannot enforce is not ignored"));
        }
        let map = PathMap::new(&command.mounts, &self.config.toolchain)?;
        let program = map.program(&command.command)?;
        let guest_cwd = command.cwd.as_deref().ok_or("the command has no working directory")?;
        let cwd = map.directory(guest_cwd)?;

        let mut leftovers = Vec::new();
        let args: Vec<String> = command
            .args
            .iter()
            .map(|arg| {
                let translated = map.text(arg);
                leftovers.extend(map.mentions_guest_root(&translated).map(|_| arg.clone()));
                translated
            })
            .collect();
        let mut env = BTreeMap::new();
        for (key, value) in &command.env {
            let translated = if key == "PATH" { host_path(&self.config.toolchain) } else { map.text(value) };
            leftovers.extend(map.mentions_guest_root(&translated).map(|_| format!("{key}={value}")));
            env.insert(key.clone(), translated);
        }
        if let Some(first) = leftovers.first() {
            return Err(format!(
                "{first} names a guest path no mount covers, so there is nothing on this machine for it to mean"
            ));
        }

        let mut writable = Vec::new();
        let mut readable = vec![canonical(&self.config.toolchain_root)?];
        for mount in &command.mounts {
            let host = canonical(&mount.host_path)?;
            if mount.read_only {
                readable.push(host);
            } else {
                writable.push(host);
            }
        }
        let private = self.config.private_roots.iter().filter_map(|root| std::fs::canonicalize(root).ok()).collect();
        let profile = Profile::compile(&Policy { network: Some(command.network), writable, readable, private });
        Ok(Plan { program, args, cwd, env, profile })
    }
}

fn host_path(toolchain: &Toolchain) -> String {
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", toolchain.host_bin.display())
}

fn canonical(path: &std::path::Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))
}

#[async_trait]
impl BuildExecutor for LocalExecutor {
    async fn run(&self, command: IsolatedCommand) -> Result<CommandOutcome, String> {
        if !std::path::Path::new(SANDBOX_EXEC).is_file() {
            return Err(format!(
                "sandbox_unavailable: {SANDBOX_EXEC} is missing, and a command is never run without its isolation"
            ));
        }
        let plan = self.plan(&command)?;

        let mut child = Command::new(SANDBOX_EXEC)
            .args(plan.profile.arguments())
            .arg(&plan.program)
            .args(&plan.args)
            .current_dir(&plan.cwd)
            .env_clear()
            .envs(&plan.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|error| format!("start {}: {error}", plan.program.display()))?;
        let pgid = child.id().ok_or("the command ended before it could be watched")?;
        let mut guard = GroupGuard { pgid, armed: true };
        let stdout = tokio::spawn(read_tail(child.stdout.take().ok_or("no stdout pipe")?));
        let stderr = tokio::spawn(read_tail(child.stderr.take().ok_or("no stderr pipe")?));

        let limit_kib = command.limits.max_memory_mb.map(|mb| u64::from(mb) * 1024);
        if limit_kib.is_some() && sample(pgid).await.is_none() {
            kill_group(pgid).await;
            let _ = child.wait().await;
            guard.armed = false;
            return Err("watchdog_unavailable: the resident memory of the command could not be read (`ps` failed), so a \
                        memory limit cannot be enforced and the command was stopped"
                .into());
        }

        let deadline = command.timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
        let mut ticker = tokio::time::interval(self.config.sample_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut timed_out = false;
        let mut over_limit: Option<u64> = None;
        let mut blind_samples = 0_u32;
        let status = loop {
            tokio::select! {
                status = child.wait() => break status.map_err(|error| format!("wait for the command: {error}"))?,
                () = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                    timed_out = true;
                    kill_group(pgid).await;
                }
                _ = ticker.tick(), if limit_kib.is_some() && over_limit.is_none() => {
                    match sample(pgid).await {
                        Some(rss) => {
                            blind_samples = 0;
                            if rss > limit_kib.unwrap_or(u64::MAX) {
                                over_limit = Some(rss);
                                kill_group(pgid).await;
                            }
                        }
                        None => {
                            blind_samples += 1;
                            if blind_samples >= 4 {
                                kill_group(pgid).await;
                                let _ = child.wait().await;
                                guard.armed = false;
                                return Err("watchdog_unavailable: the resident memory of the command could not be read \
                                            for several samples in a row, and the command was stopped".into());
                            }
                        }
                    }
                }
            }
            if timed_out {
                // The group has been told to die; collect the leader.
                break child.wait().await.map_err(|error| format!("wait for the command: {error}"))?;
            }
        };
        // Whatever the command left running in its group dies with it.
        kill_group(pgid).await;
        guard.armed = false;

        let collect = |handle: tokio::task::JoinHandle<String>| async move {
            match tokio::time::timeout(Duration::from_secs(2), handle).await {
                Ok(Ok(text)) => text,
                _ => String::new(),
            }
        };
        let out = collect(stdout).await;
        let mut err = collect(stderr).await;

        let mut exit_code = status.code().unwrap_or_else(|| status.signal().map_or(-1, |signal| 128 + signal));
        if timed_out {
            exit_code = TIMED_OUT_EXIT_CODE;
        }
        if let Some(rss) = over_limit {
            exit_code = KILLED_EXIT_CODE;
            let limit_mb = command.limits.max_memory_mb.unwrap_or(0);
            if !err.is_empty() && !err.ends_with('\n') {
                err.push('\n');
            }
            err.push_str(&format!(
                "resource_limit_exceeded: the command's processes held {} MiB resident, over the {limit_mb} MiB limit, and were killed\n",
                rss / 1024
            ));
        }
        if exit_code == 71 && err.contains("sandbox_apply") {
            return Err(format!(
                "sandbox_unavailable: the sandbox could not be applied ({}); the command was not run",
                err.trim()
            ));
        }
        Ok(CommandOutcome {
            stdout: out,
            stderr: err,
            exit_code,
            timed_out,
            cancelled: false,
            enforcement: Enforcement { network_policy_enforced: true, memory_limit_enforced: limit_kib.is_some() },
        })
    }
}
