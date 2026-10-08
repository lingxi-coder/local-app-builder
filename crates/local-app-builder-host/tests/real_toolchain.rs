//! The real thing: the pinned Node and pnpm, downloaded and verified, running a real template's frozen install and
//! build inside the sandbox.
//!
//! Opt-in, because it downloads about 80 MB and then the template's packages:
//!
//! ```text
//! LOCAL_APP_REAL_TOOLCHAIN=1 cargo test -p local-app-builder-host --test real_toolchain -- --nocapture
//! ```
//!
//! `LOCAL_APP_REAL_TOOLCHAIN_DATA=<dir>` keeps the data root (toolchain and package store) between runs, so only the
//! first one downloads. It does what the service does for an app, command for command: the dependency install with
//! the network allowed and `--frozen-lockfile --ignore-scripts`, then the fixed Vite build with the network disabled
//! and a memory limit; and builds twice, because the builds must be reproducible (P1).
#![cfg(target_os = "macos")]

use local_app_builder_contracts::execution::{
    IsolatedCommand, Mount, MountKind, NetworkPolicy, ResourceLimits,
};
use local_app_builder_host::{
    LocalExecutor, LocalExecutorConfig, Platform, Source, Spec, Status, Toolchains,
};
use local_app_builder_service::host::BuildExecutor;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const PROJECT: &str = "/var/lingxi/local-app-build/real-app/store/project";
const STORE: &str = "/var/lingxi/local-app-dependency-store";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn digest_tree(dir: &Path) -> String {
    let mut files = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            if entry.path().is_dir() {
                walk(&entry.path(), out);
            } else {
                out.push(entry.path());
            }
        }
    }
    walk(dir, &mut files);
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.strip_prefix(dir).unwrap().to_string_lossy().as_bytes());
        hasher.update(std::fs::read(&file).unwrap());
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn env(state: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("PATH".into(), "/usr/bin:/bin".into());
    env.insert("CI".into(), "1".into());
    env.insert("NODE_ENV".into(), "production".into());
    for (key, dir) in [
        ("HOME", "home"),
        ("TMPDIR", "tmp"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
        ("XDG_CACHE_HOME", "xdg-cache"),
        ("XDG_CONFIG_HOME", "xdg-config"),
        ("XDG_DATA_HOME", "xdg-data"),
        ("PNPM_HOME", "pnpm-home"),
        ("COREPACK_HOME", "corepack"),
    ] {
        env.insert(key.into(), format!("{state}/{dir}"));
    }
    env
}

#[tokio::test]
async fn the_pinned_toolchain_installs_and_builds_a_real_template_in_the_sandbox() {
    if std::env::var_os("LOCAL_APP_REAL_TOOLCHAIN").is_none() {
        eprintln!("skipped: set LOCAL_APP_REAL_TOOLCHAIN=1 to download the real toolchain and build a real template");
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let data = std::env::var_os("LOCAL_APP_REAL_TOOLCHAIN_DATA")
        .map_or_else(|| scratch.path().join("data"), PathBuf::from);
    std::fs::create_dir_all(&data).unwrap();
    let data = std::fs::canonicalize(&data).unwrap();

    // 1. The real toolchain, from the network, against the pins in the source.
    let spec = Spec::pinned(Platform::host().unwrap());
    let toolchains = Toolchains::in_data_root(&data);
    let started = Instant::now();
    let receipt = toolchains
        .install(&spec, &Source::Network, &|line| {
            eprintln!("  toolchain: {line}")
        })
        .await
        .expect("install");
    eprintln!(
        "toolchain ready in {:?}: node {} pnpm {}",
        started.elapsed(),
        receipt.node.version,
        receipt.pnpm.version
    );
    assert!(matches!(toolchains.status(&spec).await, Status::Ready(_)));

    // 2. A project laid out as the service lays an app out.
    let project = data.join("apps/real-app/workspace");
    let _ = std::fs::remove_dir_all(&project);
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../plugins/lingxi-local-app/assets/templates/react-dom/r4"),
        &project,
    );
    let state = format!("{PROJECT}/.lingxi-build-state");
    for dir in [
        "home",
        "tmp",
        "xdg-cache",
        "xdg-config",
        "xdg-data",
        "pnpm-home",
        "corepack",
    ] {
        std::fs::create_dir_all(project.join(".lingxi-build-state").join(dir)).unwrap();
    }
    let store = data.join("dependency-store");
    std::fs::create_dir_all(&store).unwrap();
    let mounts = vec![
        Mount {
            host_path: project.clone(),
            guest_path: PROJECT.into(),
            read_only: false,
            kind: MountKind::Project,
        },
        Mount {
            host_path: store,
            guest_path: STORE.into(),
            read_only: false,
            kind: MountKind::DependencyStore,
        },
    ];
    let mut config = LocalExecutorConfig::new(
        toolchains.dir(&spec),
        vec![data.clone(), PathBuf::from(std::env::var("HOME").unwrap())],
    );
    config.sample_interval = std::time::Duration::from_millis(250);
    let executor = LocalExecutor::new(config);

    // 3. `pnpm install --frozen-lockfile`, as the service builds the command: the network allowed, scripts ignored.
    let install = IsolatedCommand {
        command: "/usr/bin/pnpm".into(),
        args: [
            "install",
            "--frozen-lockfile",
            "--ignore-scripts",
            "--prefer-offline",
            "--store-dir",
            STORE,
            "--reporter=append-only",
        ]
        .map(String::from)
        .to_vec(),
        cwd: Some(PROJECT.into()),
        env: env(&state),
        timeout_ms: Some(600_000),
        network: NetworkPolicy::Allowed,
        limits: ResourceLimits {
            max_memory_mb: Some(4096),
            ..ResourceLimits::default()
        },
        mounts: mounts.clone(),
    };
    let started = Instant::now();
    let outcome = executor.run(install).await.expect("the install ran");
    eprintln!(
        "pnpm install: exit {} in {:?}\n{}",
        outcome.exit_code,
        started.elapsed(),
        outcome
            .stdout
            .lines()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(
        outcome.exit_code, 0,
        "stderr: {}\nstdout: {}",
        outcome.stderr, outcome.stdout
    );
    outcome
        .enforcement
        .ensure_for(
            NetworkPolicy::Allowed,
            ResourceLimits {
                max_memory_mb: Some(4096),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    assert!(
        project.join("node_modules/vite/bin/vite.js").is_file(),
        "the install produced no vite"
    );

    // 4. The fixed Vite build, offline, twice.
    let build = || IsolatedCommand {
        command: "/usr/bin/node".into(),
        args: [
            "--max-old-space-size=3072".to_string(),
            format!("{PROJECT}/node_modules/vite/bin/vite.js"),
            "build".into(),
            "--outDir".into(),
            "dist".into(),
            "--emptyOutDir".into(),
            "--config".into(),
            "vite.config.mjs".into(),
        ]
        .to_vec(),
        cwd: Some(PROJECT.into()),
        env: env(&state),
        timeout_ms: Some(600_000),
        network: NetworkPolicy::Disabled,
        limits: ResourceLimits {
            max_memory_mb: Some(4096),
            ..ResourceLimits::default()
        },
        mounts: mounts.clone(),
    };
    let mut digests = Vec::new();
    for round in 1..=2 {
        let started = Instant::now();
        let outcome = executor.run(build()).await.expect("the build ran");
        eprintln!(
            "build #{round}: exit {} in {:?}",
            outcome.exit_code,
            started.elapsed()
        );
        assert_eq!(
            outcome.exit_code, 0,
            "stderr: {}\nstdout: {}",
            outcome.stderr, outcome.stdout
        );
        assert!(
            outcome.enforcement.network_policy_enforced
                && outcome.enforcement.memory_limit_enforced
        );
        assert!(
            project.join("dist/index.html").is_file(),
            "no dist/index.html"
        );
        digests.push(digest_tree(&project.join("dist")));
    }
    eprintln!("dist digest: {}", digests[0]);
    assert_eq!(
        digests[0], digests[1],
        "two builds of the same input must be byte-identical"
    );

    // 5. The real Node is confined, not just the shell scripts the unit tests use: no network, no reading another
    //    app's files, no writing outside the project.
    let other = data.join("apps/other-app");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("secret.txt"), "other app").unwrap();
    let probe = |code: String, network| {
        let mut command = build();
        command.args = vec!["-e".into(), code];
        command.network = network;
        command.limits = ResourceLimits::default();
        let executor = &executor;
        async move { executor.run(command).await.expect("the probe ran") }
    };
    let result = probe("fetch('https://example.com').then(() => console.log('REACHED'), (e) => console.log('BLOCKED ' + (e.cause && e.cause.code)))".into(), NetworkPolicy::Disabled).await;
    assert!(
        result.stdout.contains("BLOCKED") && !result.stdout.contains("REACHED"),
        "{result:?}"
    );
    let result = probe(format!("try {{ console.log('READ ' + require('fs').readFileSync('{}', 'utf8')) }} catch (e) {{ console.log('DENIED ' + e.code) }}", other.join("secret.txt").display()), NetworkPolicy::Disabled).await;
    assert!(
        result.stdout.contains("DENIED EPERM") && !result.stdout.contains("READ"),
        "{result:?}"
    );
    let escape = data.join("escaped.txt");
    let result = probe(format!("try {{ require('fs').writeFileSync('{}', 'x'); console.log('WROTE') }} catch (e) {{ console.log('DENIED ' + e.code) }}", escape.display()), NetworkPolicy::Disabled).await;
    assert!(
        result.stdout.contains("DENIED EPERM") && !escape.exists(),
        "{result:?}"
    );
}
