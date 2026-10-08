//! `local-app-builder toolchain` through the library entry points, with small fake archives for the offline path.
#![cfg(unix)]

use flate2::write::GzEncoder;
use flate2::Compression;
use local_app_builder_cli::{run_toolchain, Env};
use local_app_builder_host::{Artifact, Spec};
use sha2::{Digest, Sha256};
use std::path::Path;

fn tgz(path: &str, content: &[u8]) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    let mut header = tar::Header::new_gnu();
    header.set_path(path).unwrap();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append(&header, content).unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn artifact(url: &str, entry: &str, version: &str) -> (Artifact, Vec<u8>) {
    let archive = tgz(entry, format!("#!/bin/sh\necho {version}\n").as_bytes());
    let sha256 = Sha256::digest(&archive)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (
        Artifact {
            url: url.into(),
            sha256,
            size: archive.len() as u64,
            entry: entry.into(),
        },
        archive,
    )
}

fn fake() -> (Spec, Vec<u8>, Vec<u8>) {
    let (node, node_archive) = artifact(
        "https://example.invalid/dist/node-fake.tar.gz",
        "node-fake/bin/node",
        "v26.9.0",
    );
    let (pnpm, pnpm_archive) = artifact(
        "https://example.invalid/pnpm-fake.tgz",
        "package/pnpm",
        "12.5.1",
    );
    (
        Spec {
            key: "pnpm@12.5.1/node@26.9.0".into(),
            node_version: "v26.9.0".into(),
            pnpm_version: "12.5.1".into(),
            node,
            pnpm,
        },
        node_archive,
        pnpm_archive,
    )
}

fn invoke(args: &[&str], spec: Result<Spec, String>) -> (u8, String, String) {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_toolchain(&args, &Env::default(), spec, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn data(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

#[test]
fn status_of_nothing_installed_names_the_archives_for_an_offline_install() {
    let root = tempfile::tempdir().unwrap();
    let (spec, _, _) = fake();
    let (code, out, _) = invoke(
        &["status", "--data-root", &data(root.path())],
        Ok(spec.clone()),
    );
    assert_eq!(code, 1);
    assert!(
        out.starts_with("not installed: pnpm@12.5.1/node@26.9.0"),
        "{out}"
    );
    for artifact in [&spec.node, &spec.pnpm] {
        assert!(
            out.contains(artifact.file_name())
                && out.contains(&artifact.sha256)
                && out.contains(&artifact.url),
            "{out}"
        );
    }
    assert!(out.contains("--from DIR"));
    assert_eq!(
        std::fs::read_dir(root.path()).unwrap().count(),
        0,
        "status must not create anything"
    );
}

#[test]
fn an_offline_install_then_status_is_ready() {
    let root = tempfile::tempdir().unwrap();
    let archives = tempfile::tempdir().unwrap();
    let (spec, node, pnpm) = fake();
    std::fs::write(archives.path().join("node-fake.tar.gz"), node).unwrap();
    std::fs::write(archives.path().join("pnpm-fake.tgz"), pnpm).unwrap();

    let (code, out, err) = invoke(
        &[
            "install",
            "--data-root",
            &data(root.path()),
            "--from",
            &data(archives.path()),
        ],
        Ok(spec.clone()),
    );
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("is installed at")
            && out.contains("toolchains/pnpm@12.5.1/node@26.9.0")
            && out.contains("node v26.9.0, pnpm 12.5.1"),
        "{out}"
    );

    let (code, out, _) = invoke(&["status", "--data-root", &data(root.path())], Ok(spec));
    assert_eq!(code, 0);
    assert!(
        out.starts_with("ready: pnpm@12.5.1/node@26.9.0") && out.contains("sha256"),
        "{out}"
    );
}

#[test]
fn a_missing_archive_fails_with_the_file_name_and_installs_nothing() {
    let root = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let (spec, _, _) = fake();
    let (code, out, err) = invoke(
        &[
            "install",
            "--data-root",
            &data(root.path()),
            "--from",
            &data(empty.path()),
        ],
        Ok(spec),
    );
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert!(
        err.contains("node-fake.tar.gz") && err.contains("must be in that directory"),
        "{err}"
    );
    assert!(!root.path().join("toolchains/pnpm@12.5.1").exists());
}

#[test]
fn usage_errors_are_exit_code_two_and_a_machine_without_pins_is_one() {
    let (spec, _, _) = fake();
    for args in [
        vec![],
        vec!["fetch"],
        vec!["status", "--bogus"],
        vec!["install", "--from"],
        vec!["status", "--from", "x"],
        vec!["status", "--data-root"],
    ] {
        let (code, out, err) = invoke(&args, Ok(spec.clone()));
        assert_eq!(code, 2, "{args:?}: {err}");
        assert!(out.is_empty() && !err.is_empty(), "{args:?}");
    }
    let root = tempfile::tempdir().unwrap();
    let (code, _, err) = invoke(
        &["status", "--data-root", &data(root.path())],
        Err("no pinned toolchain for linux-x86_64".into()),
    );
    assert_eq!(code, 1);
    assert!(err.contains("no pinned toolchain"));
}

#[cfg(target_os = "macos")]
#[test]
fn the_real_entry_point_asks_for_the_real_pins() {
    use local_app_builder_cli::run;

    let root = tempfile::tempdir().unwrap();
    let args: Vec<String> = ["toolchain", "status", "--data-root", &data(root.path())]
        .map(String::from)
        .to_vec();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&args, &Env::default(), &mut out, &mut err);
    let out = String::from_utf8(out).unwrap();
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("pnpm@12.5.1/node@26.9.0")
            && out.contains("https://nodejs.org/dist/v26.9.0/")
            && out.contains("https://registry.npmjs.org/@pnpm/exe."),
        "{out}"
    );

    let args: Vec<String> = ["doctor", "--data-root", &data(root.path())]
        .map(String::from)
        .to_vec();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    run(&args, &Env::default(), &mut out, &mut err);
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.lines().any(|l| l.starts_with("warn")
            && l.contains("toolchain")
            && l.contains("not installed")),
        "{out}"
    );
}
