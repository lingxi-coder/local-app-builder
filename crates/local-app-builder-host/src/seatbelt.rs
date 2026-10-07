//! The macOS sandbox profile a build runs under.
//!
//! The service asks for isolation and then refuses to believe a host that did not provide it (see
//! `local_app_builder_contracts::execution::Enforcement`), so what is claimed here has to be what the profile does. Each
//! clause below was run against a real process before it was written down (see the tests):
//!
//! * **Network.** `Disabled` denies every network operation, which includes local sockets. `LoopbackOnly` denies them
//!   and allows only the loopback interface. `Allowed` adds nothing.
//! * **Writes.** Denied everywhere, then allowed under the mounts that are not read-only and on `/dev/null`. A build
//!   cannot write outside its project.
//! * **Reads.** The roots in `private` (the data root, the home directory) cannot have their *contents* read, except
//!   under the mounts and the toolchain. Looking at a path (`stat`) is still allowed, because a program resolving the
//!   real path of its own script looks at every directory above it; listing a directory or opening a file is not. So
//!   another app's files are unreadable and their names are unlistable, while the names of the directories on the way
//!   to the project remain knowable.
//! * Everything else is left as the system has it: a profile that starts from "deny" has to enumerate what every
//!   program needs from the system, and one it missed would show up as a build failing for no readable reason.
//!
//! Rules are evaluated in order and a later one overrides an earlier one, which is how the allowances below can sit
//! inside a denied root.
//!
//! The paths travel as profile parameters (`-D name=value`, read with `(param "name")`), never spliced into the
//! profile text, so a path cannot change what the profile says.
//!
//! `sandbox-exec` is marked deprecated by Apple and is what this uses: it is on every macOS and takes a profile string.
//! Moving to the `libsandbox` call it wraps is a later, contained change in this file.

use local_app_builder_contracts::execution::NetworkPolicy;
use std::path::PathBuf;

/// The program that applies a profile and then runs a command.
pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// What a command may touch.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// The network the command may use.
    pub network: Option<NetworkPolicy>,
    /// Directories the command may write under (canonical).
    pub writable: Vec<PathBuf>,
    /// Directories the command may read under, beyond what the system allows everywhere (canonical).
    pub readable: Vec<PathBuf>,
    /// Roots whose contents are off limits except under `writable` and `readable` (canonical).
    pub private: Vec<PathBuf>,
}

/// A profile and the parameters it reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The profile source.
    pub text: String,
    /// `(name, value)` pairs, passed as `-D name=value`.
    pub params: Vec<(String, String)>,
}

impl Profile {
    /// Compile `policy`.
    #[must_use]
    pub fn compile(policy: &Policy) -> Self {
        let mut params = Vec::new();
        let mut group = |prefix: &str, paths: &[PathBuf]| -> String {
            paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let name = format!("{prefix}{index}");
                    params.push((name.clone(), path.to_string_lossy().into_owned()));
                    format!("(subpath (param \"{name}\"))")
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let writable = group("W", &policy.writable);
        let readable = group("R", &policy.readable);
        let private = group("P", &policy.private);

        let mut text = String::from("(version 1)\n(allow default)\n");
        match policy.network {
            Some(NetworkPolicy::Disabled) => text.push_str("(deny network*)\n"),
            Some(NetworkPolicy::LoopbackOnly) => {
                text.push_str("(deny network*)\n(allow network* (remote ip \"localhost:*\"))\n");
            }
            Some(NetworkPolicy::Allowed) | None => {}
        }
        text.push_str("(deny file-write*)\n");
        text.push_str("(allow file-write* (literal \"/dev/null\")");
        if !writable.is_empty() {
            text.push(' ');
            text.push_str(&writable);
        }
        text.push_str(")\n");
        if !private.is_empty() {
            text.push_str(&format!("(deny file-read-data {private})\n"));
            let allowed = [readable.as_str(), writable.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
            if !allowed.is_empty() {
                text.push_str(&format!("(allow file-read-data {allowed})\n"));
            }
        }
        Self { text, params }
    }

    /// The arguments that make `sandbox-exec` apply this profile, to be followed by the command and its arguments.
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        let mut args = vec!["-p".to_string(), self.text.clone()];
        for (name, value) in &self.params {
            args.push("-D".into());
            args.push(format!("{name}={value}"));
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy {
            network: Some(NetworkPolicy::Disabled),
            writable: vec![PathBuf::from("/data/apps/a/project")],
            readable: vec![PathBuf::from("/data/toolchains/t")],
            private: vec![PathBuf::from("/data"), PathBuf::from("/Users/u")],
        }
    }

    #[test]
    fn paths_are_parameters_and_never_part_of_the_profile_text() {
        let nasty = PathBuf::from("/data/\"))(allow network*)(\\x");
        let profile = Profile::compile(&Policy { writable: vec![nasty.clone()], ..policy() });
        assert!(!profile.text.contains("allow network"), "{}", profile.text);
        assert!(!profile.text.contains("/data"), "a path leaked into the profile text: {}", profile.text);
        assert!(profile.params.iter().any(|(_, v)| *v == nasty.to_string_lossy()));
    }

    #[test]
    fn each_network_policy_compiles_to_what_it_says() {
        let text = |network| Profile::compile(&Policy { network, ..policy() }).text;
        assert!(text(Some(NetworkPolicy::Disabled)).contains("(deny network*)"));
        assert!(!text(Some(NetworkPolicy::Disabled)).contains("allow network"));
        let loopback = text(Some(NetworkPolicy::LoopbackOnly));
        assert!(loopback.contains("(deny network*)") && loopback.contains("(allow network* (remote ip \"localhost:*\"))"));
        assert!(!text(Some(NetworkPolicy::Allowed)).contains("network"));
    }

    #[test]
    fn the_allowances_come_after_the_denials_they_open_a_hole_in() {
        let text = Profile::compile(&policy()).text;
        let deny_read = text.find("(deny file-read-data").unwrap();
        let allow_read = text.find("(allow file-read-data").unwrap();
        let deny_write = text.find("(deny file-write*)").unwrap();
        let allow_write = text.find("(allow file-write*").unwrap();
        assert!(deny_read < allow_read && deny_write < allow_write, "{text}");
        assert!(text.contains("(literal \"/dev/null\")"));
    }

    #[test]
    fn what_may_be_written_may_also_be_read() {
        let profile = Profile::compile(&policy());
        let allow = profile.text.lines().find(|l| l.starts_with("(allow file-read-data")).unwrap();
        assert!(allow.contains("\"W0\"") && allow.contains("\"R0\""), "{allow}");
    }

    #[test]
    fn arguments_carry_the_profile_then_every_parameter() {
        let args = Profile::compile(&policy()).arguments();
        assert_eq!(args[0], "-p");
        assert_eq!(args.iter().filter(|a| *a == "-D").count(), 4);
        assert!(args.contains(&"P1=/Users/u".to_string()));
    }
}
