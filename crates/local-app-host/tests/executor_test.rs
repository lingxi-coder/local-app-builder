//! The executor against real processes under the real sandbox.
//!
//! The "toolchain" here is a directory with a `node` that evaluates the shell code it is given, and a `perl` that is
//! the system's: what is under test is the executor's translation, isolation and watching, not Node. Each isolation
//! test states what it must *not* be able to do and checks that the thing really did not happen on disk, so that a
//! sandbox that quietly stopped working fails here rather than in someone's build.
#![cfg(target_os = "macos")]

use local_app_contracts::execution::{
    CommandOutcome, IsolatedCommand, Mount, MountKind, NetworkPolicy, ResourceLimits,
};
use local_app_host::{LocalExecutor, LocalExecutorConfig};
use local_app_service::host::BuildExecutor;
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PROJECT: &str = "/var/lingxi/local-app-build/app-a/store/project";
const STORE: &str = "/var/lingxi/local-app-dependency-store";

struct World {
    _dir: tempfile::TempDir,
    data: PathBuf,
    project: PathBuf,
    store: PathBuf,
    other: PathBuf,
    executor: LocalExecutor,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let data = base.join("data");
    let project = data.join("apps/a/project");
    let store = data.join("store");
    let other = data.join("apps/b");
    let toolchain = base.join("toolchain");
    for d in [&project, &store, &other, &toolchain.join("bin")] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(other.join("secret.txt"), "other app's data").unwrap();
    std::fs::write(project.join("input.txt"), "project data").unwrap();
    let node = toolchain.join("bin/node");
    std::fs::write(&node, "#!/bin/sh\neval \"$1\"\n").unwrap();
    std::fs::set_permissions(&node, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("/usr/bin/perl", toolchain.join("bin/perl")).unwrap();
    let mut config = LocalExecutorConfig::new(toolchain, vec![data.clone()]);
    config.sample_interval = Duration::from_millis(50);
    World { _dir: dir, data, project, store, other, executor: LocalExecutor::new(config) }
}

fn command(program: &str, args: &[&str]) -> IsolatedCommand {
    IsolatedCommand {
        command: format!("/usr/bin/{program}"),
        args: args.iter().map(ToString::to_string).collect(),
        cwd: Some(PROJECT.into()),
        env: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
        timeout_ms: Some(30_000),
        network: NetworkPolicy::Disabled,
        limits: ResourceLimits::default(),
        mounts: Vec::new(),
    }
}

impl World {
    fn mounted(&self, mut command: IsolatedCommand) -> IsolatedCommand {
        command.mounts = vec![
            Mount { host_path: self.project.clone(), guest_path: PROJECT.into(), read_only: false, kind: MountKind::Project },
            Mount { host_path: self.store.clone(), guest_path: STORE.into(), read_only: true, kind: MountKind::DependencyStore },
        ];
        command
    }

    /// Run shell code through the toolchain's `node`.
    async fn sh(&self, code: &str, rest: &[&str]) -> CommandOutcome {
        let mut args = vec![code];
        args.extend_from_slice(rest);
        self.executor.run(self.mounted(command("node", &args))).await.expect("the command ran")
    }
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("/bin/kill").args(["-0", pid.trim()]).output().is_ok_and(|o| o.status.success())
}

fn wait_for(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            if !text.trim().is_empty() {
                return text;
            }
        }
        assert!(Instant::now() < deadline, "{} never appeared", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test]
async fn guest_paths_in_the_program_arguments_and_environment_mean_host_paths() {
    let w = world();
    let mut cmd = w.mounted(command("node", &["pwd; echo \"$2\"; echo \"$HOME\"; cat input.txt", &format!("--store-dir={STORE}")]));
    cmd.env.insert("HOME".into(), format!("{PROJECT}/.state/home"));
    let outcome = w.executor.run(cmd).await.unwrap();
    assert_eq!(outcome.exit_code, 0, "{outcome:?}");
    let lines: Vec<&str> = outcome.stdout.lines().collect();
    assert_eq!(lines[0], w.project.to_str().unwrap());
    assert_eq!(lines[1], format!("--store-dir={}", w.store.display()));
    assert_eq!(lines[2], format!("{}/.state/home", w.project.display()));
    assert_eq!(lines[3], "project data");
    assert!(!outcome.timed_out && !outcome.cancelled);
}

#[tokio::test]
async fn the_command_sees_only_the_environment_it_was_given() {
    let w = world();
    std::env::set_var("LOCAL_APP_HOST_TEST_LEAK", "leaked");
    let outcome = w.sh("echo \"[${LOCAL_APP_HOST_TEST_LEAK:-unset}] [${PATH}]\"", &[]).await;
    assert_eq!(outcome.exit_code, 0, "{outcome:?}");
    let line = outcome.stdout.trim();
    assert!(line.starts_with("[unset] ["), "{line}");
    // The guest's PATH is replaced by the toolchain's, so `#!/usr/bin/env node` finds the right node and not whatever
    // else this Mac has installed.
    assert!(line.contains("/toolchain/bin:/usr/bin:/bin"), "{line}");
}

#[tokio::test]
async fn a_nonzero_exit_is_an_outcome_not_an_error() {
    let w = world();
    let outcome = w.sh("echo out; echo err >&2; exit 3", &[]).await;
    assert_eq!((outcome.exit_code, outcome.stdout.trim(), outcome.stderr.trim()), (3, "out", "err"));
    assert!(outcome.enforcement.network_policy_enforced);
    assert!(!outcome.enforcement.memory_limit_enforced, "no memory limit was asked for");
}

#[tokio::test]
async fn a_command_cannot_write_outside_its_project() {
    let w = world();
    let outside = std::env::temp_dir().join(format!("local-app-host-escape-{}", std::process::id()));
    let code = format!("echo ok > out.txt; echo bad > '{}/evil.txt'; echo bad > '{}'; echo bad > /etc/local-app-host-evil", w.other.display(), outside.display());
    let outcome = w.sh(&code, &[]).await;
    assert_ne!(outcome.exit_code, 0, "the last write must have failed: {outcome:?}");
    assert_eq!(std::fs::read_to_string(w.project.join("out.txt")).unwrap().trim(), "ok", "the project is writable");
    assert!(!w.other.join("evil.txt").exists(), "wrote into another app");
    assert!(!outside.exists(), "wrote outside the data root");
    assert!(!Path::new("/etc/local-app-host-evil").exists());
    assert!(outcome.stderr.matches("not permitted").count() >= 3, "{}", outcome.stderr);
}

#[tokio::test]
async fn a_read_only_mount_cannot_be_written() {
    let w = world();
    let outcome = w.sh(&format!("echo x > {STORE}/blocked"), &[]).await;
    assert_ne!(outcome.exit_code, 0);
    assert!(!w.store.join("blocked").exists());
}

#[tokio::test]
async fn a_command_cannot_read_another_apps_files_or_list_them() {
    let w = world();
    let secret = w.other.join("secret.txt");
    let outcome = w.sh(&format!("cat '{0}' && echo LEAKED; ls '{1}' && echo LISTED; cat input.txt; ls /usr/bin >/dev/null && echo system-ok", secret.display(), w.other.display()), &[]).await;
    assert!(!outcome.stdout.contains("LEAKED") && !outcome.stdout.contains("other app's data"), "{outcome:?}");
    assert!(!outcome.stdout.contains("LISTED"), "{outcome:?}");
    assert!(outcome.stdout.contains("project data"), "the project is readable: {outcome:?}");
    assert!(outcome.stdout.contains("system-ok"), "the system is readable: {outcome:?}");
    // Looking at a path is allowed (programs resolve the real path of their own script); reading it is not.
    let outcome = w.sh(&format!("test -e '{}' && echo exists", secret.display()), &[]).await;
    assert!(outcome.stdout.contains("exists"), "{outcome:?}");
}

#[tokio::test]
async fn the_network_policy_is_enforced_for_each_policy() {
    let w = world();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let probe = "/usr/bin/nc -z -w 2 127.0.0.1 \"$2\" && echo connected || echo refused";
    let run = |network| {
        let mut cmd = w.mounted(command("node", &[probe, &port]));
        cmd.network = network;
        let executor = &w.executor;
        async move { executor.run(cmd).await.unwrap() }
    };
    let disabled = run(NetworkPolicy::Disabled).await;
    assert_eq!(disabled.stdout.trim(), "refused", "{disabled:?}");
    assert!(disabled.enforcement.network_policy_enforced);
    assert_eq!(run(NetworkPolicy::LoopbackOnly).await.stdout.trim(), "connected");
    assert_eq!(run(NetworkPolicy::Allowed).await.stdout.trim(), "connected");
    drop(listener);
}

#[tokio::test]
async fn a_command_that_runs_too_long_is_killed_with_everything_it_started() {
    let w = world();
    let mut cmd = w.mounted(command("node", &["sleep 60 & echo $! > child.pid; wait"]));
    cmd.timeout_ms = Some(3_000); // well past sandbox start-up when many tests start one at once
    let started = Instant::now();
    let outcome = w.executor.run(cmd).await.unwrap();
    assert!(outcome.timed_out, "{outcome:?}");
    assert_eq!(outcome.exit_code, local_app_host::TIMED_OUT_EXIT_CODE);
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid = std::fs::read_to_string(w.project.join("child.pid")).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(!alive(&pid), "the grandchild {pid} outlived the command");
}

#[tokio::test]
async fn a_command_that_exits_leaving_a_child_behind_does_not_leave_it_running() {
    // A build tool that backgrounds a helper and returns: the helper holds the output pipes open, so without the
    // group being killed at exit the run would wait for it and then leave it behind.
    let w = world();
    let started = Instant::now();
    let outcome = w.sh("sleep 60 & echo $! > child.pid; echo launched", &[]).await;
    assert_eq!((outcome.exit_code, outcome.stdout.trim()), (0, "launched"), "{outcome:?}");
    assert!(started.elapsed() < Duration::from_secs(2), "the run waited for the orphan: {:?}", started.elapsed());
    let pid = std::fs::read_to_string(w.project.join("child.pid")).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(!alive(&pid), "the child {pid} outlived the command that started it");
}

#[tokio::test]
async fn a_command_that_passes_its_memory_limit_is_killed_and_says_why() {
    let w = world();
    let mut cmd = w.mounted(command("perl", &["-e", "$x = 'a' x (400*1024*1024); sleep 60"]));
    cmd.limits = ResourceLimits { max_memory_mb: Some(128), ..ResourceLimits::default() };
    let started = Instant::now();
    let outcome = w.executor.run(cmd).await.unwrap();
    assert_eq!(outcome.exit_code, local_app_host::KILLED_EXIT_CODE, "{outcome:?}");
    assert!(outcome.stderr.contains("resource_limit_exceeded"), "{}", outcome.stderr);
    assert!(outcome.enforcement.memory_limit_enforced);
    assert!(!outcome.timed_out);
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test]
async fn a_command_under_its_memory_limit_runs_to_the_end_and_is_still_marked_enforced() {
    let w = world();
    let mut cmd = w.mounted(command("perl", &["-e", "$x = 'a' x (8*1024*1024); print 'done'"]));
    cmd.limits = ResourceLimits { max_memory_mb: Some(512), ..ResourceLimits::default() };
    let outcome = w.executor.run(cmd).await.unwrap();
    assert_eq!((outcome.exit_code, outcome.stdout.as_str()), (0, "done"), "{outcome:?}");
    assert!(outcome.enforcement.memory_limit_enforced);
}

#[tokio::test]
async fn dropping_the_run_stops_the_command_and_its_children() {
    let w = std::sync::Arc::new(world());
    let task = {
        let w = w.clone();
        tokio::spawn(async move { w.sh("sleep 60 & echo $! > child.pid; wait", &[]).await })
    };
    let pid = tokio::task::spawn_blocking({
        let path = w.project.join("child.pid");
        move || wait_for(&path)
    })
    .await
    .unwrap();
    assert!(alive(&pid));
    task.abort();
    let _ = task.await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(&pid) {
        assert!(Instant::now() < deadline, "the child {pid} survived the cancelled run");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_command_that_the_host_cannot_make_safe_is_refused_before_anything_runs() {
    let w = world();
    let marker = w.project.join("ran");
    let touch = format!("touch '{}'", marker.display());
    let refuse = |cmd: IsolatedCommand| {
        let executor = &w.executor;
        async move { executor.run(cmd).await.unwrap_err() }
    };

    let mut limited = w.mounted(command("node", &[&touch]));
    limited.limits.max_cpu_seconds = Some(5);
    assert!(refuse(limited).await.contains("unsupported_limit"));

    let mut other_program = w.mounted(command("node", &[&touch]));
    other_program.command = "/bin/sh".into();
    assert!(refuse(other_program).await.contains("not one the toolchain provides"));

    let unmounted = w.mounted(command("node", &[&touch, "/var/lingxi/local-app-build/other/store/project/x"]));
    assert!(refuse(unmounted).await.contains("no mount covers"));

    let mut elsewhere = w.mounted(command("node", &[&touch]));
    elsewhere.cwd = Some("/var/lingxi/somewhere".into());
    assert!(refuse(elsewhere).await.contains("not inside any mount"));

    let missing = w.mounted(command("pnpm", &[&touch]));
    assert!(refuse(missing).await.contains("does not provide pnpm"));

    assert!(!marker.exists(), "a refused command must not have run");
}

#[tokio::test]
async fn output_beyond_the_cap_keeps_the_end_and_says_what_was_dropped() {
    let w = world();
    let outcome = w.sh("yes abcdefghijklmnopqrstuvwxyz | head -c 3500000; echo; echo THE-END", &[]).await;
    assert_eq!(outcome.exit_code, 0);
    assert!(outcome.stdout.starts_with('['), "{}", &outcome.stdout[..60.min(outcome.stdout.len())]);
    assert!(outcome.stdout.contains("bytes of earlier output were dropped"));
    assert!(outcome.stdout.trim_end().ends_with("THE-END"));
    assert!(outcome.stdout.len() <= local_app_host::OUTPUT_CAP_BYTES + 100);
    let _ = &w.data;
}
