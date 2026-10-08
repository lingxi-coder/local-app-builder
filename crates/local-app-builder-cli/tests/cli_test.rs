//! The command line as a process: argument handling, exit codes, and what goes to which stream.

use std::process::Command;

fn local_app() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_local-app-builder"));
    c.env_remove("LOCAL_APP_BUILDER_DATA_ROOT");
    c
}

#[test]
fn version_prints_the_crate_version_and_exits_zero() {
    let out = local_app().arg("version").output().unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("local-app-builder {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(out.stderr.is_empty());
}

#[test]
fn no_arguments_is_a_usage_error_on_stderr() {
    let out = local_app().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("USAGE"));
}

#[test]
fn an_unknown_command_names_itself_and_exits_two() {
    let out = local_app().arg("frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("frobnicate"));
}

#[test]
fn doctor_json_on_a_fresh_root_is_ok_and_does_not_create_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    let out = local_app()
        .args(["doctor", "--json", "--data-root"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("{\"ok\":true,\"checks\":["), "{text}");
    assert!(!root.exists());
}

#[test]
fn doctor_exits_one_when_the_root_is_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    std::fs::write(&file, "x").unwrap();
    let out = local_app()
        .args(["doctor", "--data-root"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("fail"));
}

#[test]
fn doctor_refuses_a_relative_root_and_a_missing_value() {
    let out = local_app()
        .args(["doctor", "--data-root", "rel"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("absolute"));
    let out = local_app()
        .args(["doctor", "--data-root"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn doctor_reads_the_root_from_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    let out = local_app()
        .arg("doctor")
        .env("LOCAL_APP_BUILDER_DATA_ROOT", dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("LOCAL_APP_BUILDER_DATA_ROOT"));
}
