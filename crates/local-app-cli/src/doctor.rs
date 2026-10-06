//! `local-app doctor`: what this machine and the data root look like, before anything is built.
//!
//! A check fails only when something would stop every later command (the data root cannot be written). A missing
//! tool is a warning: the commands that need it say so when they run.

use crate::{DataRoot, Env};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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
    /// A stable identifier (`data-root`, `tool.node`, …).
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

/// Run every check against `root` and `env`.
#[must_use]
pub fn run_doctor(root: &DataRoot, env: &Env) -> Report {
    let mut checks = vec![
        Check {
            id: "version",
            status: CheckStatus::Ok,
            detail: format!("local-app {}", crate::VERSION),
        },
        Check {
            id: "platform",
            status: CheckStatus::Ok,
            detail: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        },
        check_data_root(root),
    ];
    for tool in ["node", "pnpm"] {
        checks.push(check_tool(tool, env));
    }
    Report { checks }
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

fn check_tool(name: &'static str, env: &Env) -> Check {
    let id = match name {
        "node" => "tool.node",
        _ => "tool.pnpm",
    };
    match find_on_path(name, env.path()) {
        Some(path) => Check { id, status: CheckStatus::Ok, detail: format!("{name} found at {}", path.display()) },
        None => Check { id, status: CheckStatus::Warn, detail: format!("{name} was not found on PATH") },
    }
}

fn find_on_path(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(name)).find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
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

    #[cfg(unix)]
    #[test]
    fn a_tool_is_found_only_when_it_is_an_executable_file() {
        use std::os::unix::fs::PermissionsExt;
        let bin = tempfile::tempdir().unwrap();
        let node = bin.path().join("node");
        std::fs::write(&node, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pnpm = bin.path().join("pnpm");
        std::fs::write(&pnpm, "not executable").unwrap();
        std::fs::set_permissions(&pnpm, std::fs::Permissions::from_mode(0o644)).unwrap();
        let env = Env::new([], vec![bin.path().to_path_buf()]);
        let data = tempfile::tempdir().unwrap();
        let r = run_doctor(&root(data.path()), &env);
        assert_eq!(find(&r, "tool.node").status, CheckStatus::Ok);
        assert_eq!(find(&r, "tool.pnpm").status, CheckStatus::Warn);
        assert!(!r.has_failure(), "a missing tool is a warning, not a failure");
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
