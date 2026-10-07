//! `local-app-builder doctor`: what this machine and the data root look like, before anything is built.
//!
//! A check fails only when something would stop every later command (the data root cannot be written). A toolchain
//! that is not installed is a warning: the commands that need it say so when they run. Node and pnpm on `PATH` are not
//! looked at, because a build never uses them (see `local_app_builder_host::Toolchains`).

use crate::{DataRoot, Env};
use std::fmt::Write as _;
use local_app_builder_host::{Platform, Spec, Status, Toolchains};
use std::path::Path;

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    /// Fine.
    Ok,
    /// Worth knowing; later commands may refuse.
    Warn,
    /// Nothing will work until this is fixed.
    Fail,
}

impl CheckStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// A stable identifier (`data-root`, `toolchain`, …).
    pub id: &'static str,
    /// The outcome.
    pub status: CheckStatus,
    /// What was found, for a person.
    pub detail: String,
}

/// Everything `doctor` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The checks, in the order they ran.
    pub checks: Vec<Check>,
}

impl Report {
    /// True when any check failed.
    #[must_use]
    pub fn has_failure(&self) -> bool {
        self.checks.iter().any(|c| c.status == CheckStatus::Fail)
    }

    /// The report for a terminal.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for c in &self.checks {
            let _ = writeln!(s, "{:<5} {:<12} {}", c.status.label(), c.id, c.detail);
        }
        s
    }

    /// The report as one JSON object: `{"ok":bool,"checks":[{"id","status","detail"}]}`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut s = format!("{{\"ok\":{},\"checks\":[", !self.has_failure());
        for (i, c) in self.checks.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(
                s,
                "{{\"id\":{},\"status\":{},\"detail\":{}}}",
                json_string(c.id),
                json_string(c.status.label()),
                json_string(&c.detail)
            );
        }
        s.push_str("]}\n");
        s
    }
}

fn json_string(value: &str) -> String {
    let mut s = String::with_capacity(value.len() + 2);
    s.push('"');
    for ch in value.chars() {
        match ch {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(s, "\\u{:04x}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
    s
}

/// Run every check against `root`. `_env` is kept for the checks that will read the environment.
#[must_use]
pub fn run_doctor(root: &DataRoot, env: &Env) -> Report {
    run_doctor_with(root, env, Platform::host().map(Spec::pinned))
}

/// [`run_doctor`] with the toolchain to look for given (or why there is none for this machine).
#[must_use]
pub fn run_doctor_with(root: &DataRoot, _env: &Env, spec: Result<Spec, String>) -> Report {
    let checks = vec![
        Check {
            id: "version",
            status: CheckStatus::Ok,
            detail: format!("local-app-builder {}", crate::VERSION),
        },
        Check {
            id: "platform",
            status: CheckStatus::Ok,
            detail: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        },
        check_data_root(root),
        check_toolchain(root, spec),
    ];
    Report { checks }
}

fn check_toolchain(root: &DataRoot, spec: Result<Spec, String>) -> Check {
    let id = "toolchain";
    let spec = match spec {
        Ok(spec) => spec,
        Err(reason) => return Check { id, status: CheckStatus::Warn, detail: reason },
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return Check { id, status: CheckStatus::Warn, detail: format!("cannot check: {error}") },
    };
    let toolchains = Toolchains::in_data_root(&root.path);
    let (status, detail) = match runtime.block_on(toolchains.status(&spec)) {
        Status::Ready(receipt) => (
            CheckStatus::Ok,
            format!("{} is installed and verified (node {}, pnpm {})", spec.key, receipt.node.version, receipt.pnpm.version),
        ),
        Status::NotInstalled => (
            CheckStatus::Warn,
            format!("{} is not installed; `local-app-builder toolchain install` provides it, and builds need it", spec.key),
        ),
        Status::Damaged(why) => (CheckStatus::Warn, format!("{} is damaged ({why}); `local-app-builder toolchain install` replaces it", spec.key)),
    };
    Check { id, status, detail }
}

fn check_data_root(root: &DataRoot) -> Check {
    let shown = root.path.display();
    let source = root.source.label();
    let (status, detail) = match std::fs::metadata(&root.path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (CheckStatus::Ok, format!("{shown} ({source}) does not exist yet; it is created on first use"))
        }
        Err(e) => (CheckStatus::Fail, format!("{shown} ({source}) cannot be read: {e}")),
        Ok(m) if !m.is_dir() => (CheckStatus::Fail, format!("{shown} ({source}) is not a directory")),
        Ok(_) => match probe_writable(&root.path) {
            Ok(()) => (CheckStatus::Ok, format!("{shown} ({source}) is writable")),
            Err(e) => (CheckStatus::Fail, format!("{shown} ({source}) is not writable: {e}")),
        },
    };
    Check { id: "data-root", status, detail }
}

/// Create and remove one file. The name carries the process id so two `doctor` runs do not trip each other.
fn probe_writable(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join(format!(".doctor-probe-{}", std::process::id()));
    std::fs::write(&probe, b"")?;
    std::fs::remove_file(&probe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataRootSource;

    fn root(path: &Path) -> DataRoot {
        DataRoot { path: path.to_path_buf(), source: DataRootSource::Flag }
    }

    fn find<'a>(r: &'a Report, id: &str) -> &'a Check {
        r.checks.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn a_missing_root_is_fine_and_is_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        let r = run_doctor(&root(&missing), &Env::default());
        assert_eq!(find(&r, "data-root").status, CheckStatus::Ok);
        assert!(!missing.exists(), "doctor must not create the data root");
        assert!(!r.has_failure());
    }

    #[test]
    fn a_writable_root_passes_and_leaves_no_probe_behind() {
        let dir = tempfile::tempdir().unwrap();
        let r = run_doctor(&root(dir.path()), &Env::default());
        assert_eq!(find(&r, "data-root").status, CheckStatus::Ok);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_file_where_the_root_should_be_fails() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        let r = run_doctor(&root(&file), &Env::default());
        assert_eq!(find(&r, "data-root").status, CheckStatus::Fail);
        assert!(r.has_failure());
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_root_fails() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ro = dir.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
        // A superuser can write anywhere; only assert when the OS actually refuses.
        let can_write = std::fs::write(ro.join("t"), b"").is_ok();
        let r = run_doctor(&root(&ro), &Env::default());
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
        if !can_write {
            assert_eq!(find(&r, "data-root").status, CheckStatus::Fail);
        }
    }

    #[test]
    fn a_missing_toolchain_is_a_warning_that_says_how_to_get_it_and_creates_nothing() {
        let data = tempfile::tempdir().unwrap();
        let r = run_doctor_with(&root(data.path()), &Env::default(), Ok(Spec::pinned(Platform::DarwinArm64)));
        let check = find(&r, "toolchain");
        assert_eq!(check.status, CheckStatus::Warn);
        assert!(check.detail.contains("pnpm@12.5.1/node@26.9.0") && check.detail.contains("local-app-builder toolchain install"), "{}", check.detail);
        assert!(!r.has_failure());
        assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 0, "doctor must not create anything");
    }

    #[test]
    fn node_and_pnpm_on_path_are_not_a_toolchain() {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        for tool in ["node", "pnpm"] {
            std::fs::write(bin.path().join(tool), "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(bin.path().join(tool), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = Env::new([], vec![bin.path().to_path_buf()]);
        let data = tempfile::tempdir().unwrap();
        let r = run_doctor_with(&root(data.path()), &env, Ok(Spec::pinned(Platform::DarwinArm64)));
        assert_eq!(find(&r, "toolchain").status, CheckStatus::Warn);
        assert!(r.checks.iter().all(|c| !c.id.starts_with("tool.")));
    }

    #[test]
    fn a_machine_without_pins_says_so_in_the_toolchain_check() {
        let data = tempfile::tempdir().unwrap();
        let r = run_doctor_with(&root(data.path()), &Env::default(), Err("no pinned toolchain for linux-x86_64".into()));
        assert_eq!(find(&r, "toolchain").status, CheckStatus::Warn);
        assert!(find(&r, "toolchain").detail.contains("no pinned toolchain"));
    }

    #[test]
    fn json_escapes_and_reports_the_overall_verdict() {
        let r = Report {
            checks: vec![Check { id: "x", status: CheckStatus::Fail, detail: "a \"b\"\n\\".into() }],
        };
        assert_eq!(
            r.to_json(),
            "{\"ok\":false,\"checks\":[{\"id\":\"x\",\"status\":\"fail\",\"detail\":\"a \\\"b\\\"\\n\\\\\"}]}\n"
        );
    }
}
