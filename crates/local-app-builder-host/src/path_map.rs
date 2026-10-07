//! From the paths the service speaks to the paths this machine has.
//!
//! The service describes every command in terms of a Linux guest: the program is `/usr/bin/node`, the project is
//! mounted at `/var/lingxi/local-app-build/<id>/<channel>/project`, the dependency store at another fixed path. A Mac
//! has no guest, so the executor runs the command on the host and translates what it was told: each mount's guest
//! path becomes the host directory it stands for, and each program becomes the one the toolchain provides.
//!
//! Translation is by whole path segments and only for what the command names. A guest path that no mount covers is
//! not guessed at: it is reported, because running a command with a path that means nothing here would fail in a way
//! nobody could read.

use local_app_builder_contracts::execution::Mount;
use std::path::{Path, PathBuf};

/// Where the programs a command may run live on this machine, and what the guest calls that directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toolchain {
    /// The directory the guest finds its programs in (`/usr/bin`).
    pub guest_bin: String,
    /// The directory on this machine that holds the same programs.
    pub host_bin: PathBuf,
}

/// The translation for one command.
#[derive(Debug, Clone)]
pub struct PathMap {
    /// `(guest path, canonical host path)`, longest guest path first so the most specific mount wins.
    mounts: Vec<(String, PathBuf)>,
    toolchain: Toolchain,
}

/// Characters that end a path inside a longer string (`--store-dir=/x:/y`, `PATH`-style lists, quoted values).
fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '=' | ':' | ',' | ';' | '"' | '\'' | '(' | ')')
}

impl PathMap {
    /// Build the map for `mounts`. Every mount's host directory must exist; it is canonicalised, because the
    /// sandbox and the programs both see real paths (`/tmp` is `/private/tmp` here).
    ///
    /// # Errors
    /// A mount's host directory is missing, or two mounts claim the same guest path.
    pub fn new(mounts: &[Mount], toolchain: &Toolchain) -> Result<Self, String> {
        let mut entries: Vec<(String, PathBuf)> = Vec::new();
        for mount in mounts {
            let host = std::fs::canonicalize(&mount.host_path)
                .map_err(|error| format!("mount {}: {error}", mount.host_path.display()))?;
            let guest = mount.guest_path.trim_end_matches('/').to_string();
            if guest.is_empty() || !guest.starts_with('/') {
                return Err(format!("mount guest path {:?} is not an absolute path", mount.guest_path));
            }
            if entries.iter().any(|(existing, _)| *existing == guest) {
                return Err(format!("two mounts claim the guest path {guest}"));
            }
            entries.push((guest, host));
        }
        entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        Ok(Self { mounts: entries, toolchain: toolchain.clone() })
    }

    /// The host program a guest program stands for.
    ///
    /// # Errors
    /// The program is not in the toolchain's directory, or the toolchain does not provide it. A command is never
    /// run as some other program that happens to share its name.
    pub fn program(&self, guest: &str) -> Result<PathBuf, String> {
        let prefix = format!("{}/", self.toolchain.guest_bin.trim_end_matches('/'));
        let Some(name) = guest.strip_prefix(&prefix).filter(|n| !n.is_empty() && !n.contains('/')) else {
            return Err(format!("program {guest} is not one the toolchain provides (programs live in {prefix})"));
        };
        let host = self.toolchain.host_bin.join(name);
        if !host.is_file() {
            return Err(format!(
                "the toolchain does not provide {name}: {} is missing (toolchain not installed?)",
                host.display()
            ));
        }
        Ok(host)
    }

    /// `text` with each mounted guest path replaced by its host path. Only whole paths and whole leading segments
    /// are replaced: `/var/x/project` is rewritten in `/var/x/project/src` and in `--dir=/var/x/project`, not in
    /// `/var/x/project-two`.
    #[must_use]
    pub fn text(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.char_indices().peekable();
        let mut previous: Option<char> = None;
        while let Some((index, c)) = chars.next() {
            let at_start_of_path = previous.is_none_or(is_separator);
            if at_start_of_path && c == '/' {
                if let Some((guest, host)) = self.mount_at(&text[index..]) {
                    out.push_str(&host.to_string_lossy());
                    for _ in 1..guest.chars().count() {
                        chars.next();
                    }
                    previous = guest.chars().last();
                    continue;
                }
            }
            out.push(c);
            previous = Some(c);
        }
        out
    }

    fn mount_at(&self, rest: &str) -> Option<(&str, &Path)> {
        self.mounts.iter().find_map(|(guest, host)| {
            let tail = rest.strip_prefix(guest.as_str())?;
            tail.chars().next().is_none_or(|next| next == '/' || is_separator(next)).then_some((guest.as_str(), host.as_path()))
        })
    }

    /// The host directory a guest working directory stands for, which must be inside a mount.
    ///
    /// # Errors
    /// The directory is not inside any mount, or does not exist.
    pub fn directory(&self, guest: &str) -> Result<PathBuf, String> {
        let (_, host) = self.mount_at(guest).filter(|(g, _)| guest.starts_with(g)).ok_or_else(|| {
            format!("working directory {guest} is not inside any mount, so there is nothing on this machine for it to be")
        })?;
        let translated = PathBuf::from(self.text(guest));
        if !translated.starts_with(host) {
            return Err(format!("working directory {guest} resolves outside its mount"));
        }
        let real = std::fs::canonicalize(&translated)
            .map_err(|error| format!("working directory {}: {error}", translated.display()))?;
        if !real.starts_with(host) {
            return Err(format!("working directory {guest} resolves outside its mount through a link"));
        }
        Ok(real)
    }

    /// Whether `text` still names a place under the guest roots after translation, which would mean a mount the
    /// service forgot to declare.
    #[must_use]
    pub fn mentions_guest_root(&self, text: &str) -> Option<&'static str> {
        [local_app_builder_contracts::guest_paths::LOCAL_APP_BUILD_ROOT, local_app_builder_contracts::guest_paths::LOCAL_APP_DEPENDENCY_STORE]
            .into_iter()
            .find(|root| text.contains(root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use local_app_builder_contracts::execution::MountKind;

    struct Fixture {
        _dir: tempfile::TempDir,
        project: PathBuf,
        store: PathBuf,
        bin: PathBuf,
        map: PathMap,
    }

    const PROJECT: &str = "/var/lingxi/local-app-build/app-one/store/project";
    const STORE: &str = "/var/lingxi/local-app-dependency-store";

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let project = std::fs::canonicalize(dir.path()).unwrap().join("project");
        let store = std::fs::canonicalize(dir.path()).unwrap().join("store");
        let bin = std::fs::canonicalize(dir.path()).unwrap().join("bin");
        for d in [&project, &store, &bin] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(project.join("sub")).unwrap();
        std::fs::write(bin.join("node"), b"#!/bin/sh\n").unwrap();
        let mounts = [
            Mount { host_path: project.clone(), guest_path: PROJECT.into(), read_only: false, kind: MountKind::Project },
            Mount { host_path: store.clone(), guest_path: STORE.into(), read_only: true, kind: MountKind::DependencyStore },
        ];
        let map = PathMap::new(&mounts, &Toolchain { guest_bin: "/usr/bin".into(), host_bin: bin.clone() }).unwrap();
        Fixture { _dir: dir, project, store, bin, map }
    }

    #[test]
    fn a_mounted_guest_path_becomes_its_host_directory_inside_any_string() {
        let f = fixture();
        let host = f.project.to_string_lossy().into_owned();
        assert_eq!(f.map.text(PROJECT), host);
        assert_eq!(f.map.text(&format!("{PROJECT}/node_modules/vite/bin/vite.js")), format!("{host}/node_modules/vite/bin/vite.js"));
        assert_eq!(f.map.text(&format!("--dir={PROJECT}")), format!("--dir={host}"));
        assert_eq!(f.map.text(&format!("{STORE}:{PROJECT}/x")), format!("{}:{host}/x", f.store.display()));
        assert_eq!(f.map.text("--max-old-space-size=3072"), "--max-old-space-size=3072");
        assert_eq!(f.map.text("vite.config.mjs"), "vite.config.mjs");
    }

    #[test]
    fn only_whole_segments_are_rewritten() {
        let f = fixture();
        for untouched in [format!("{PROJECT}-two"), format!("{PROJECT}x/y"), format!("x{PROJECT}"), "/var/lingxi/local-app-build".to_string()] {
            assert_eq!(f.map.text(&untouched), untouched, "{untouched}");
        }
    }

    #[test]
    fn a_program_is_the_toolchains_and_nothing_else_is_ever_run() {
        let f = fixture();
        assert_eq!(f.map.program("/usr/bin/node").unwrap(), f.bin.join("node"));
        for (guest, why) in [
            ("/usr/bin/pnpm", "does not provide pnpm"),
            ("/bin/sh", "not one the toolchain provides"),
            ("/usr/bin/../bin/sh", "not one the toolchain provides"),
            ("/usr/bin/", "not one the toolchain provides"),
            ("node", "not one the toolchain provides"),
        ] {
            let error = f.map.program(guest).unwrap_err();
            assert!(error.contains(why), "{guest}: {error}");
        }
    }

    #[test]
    fn a_working_directory_must_be_inside_a_mount_and_stay_there() {
        let f = fixture();
        assert_eq!(f.map.directory(PROJECT).unwrap(), f.project);
        assert_eq!(f.map.directory(&format!("{PROJECT}/sub")).unwrap(), f.project.join("sub"));
        assert!(f.map.directory("/var/lingxi/elsewhere").unwrap_err().contains("not inside any mount"));
        assert!(f.map.directory(&format!("{PROJECT}/missing")).is_err());
        // A link that leaves the mount is not a way out of it.
        let outside = f.project.parent().unwrap().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, f.project.join("escape")).unwrap();
        let error = f.map.directory(&format!("{PROJECT}/escape")).unwrap_err();
        assert!(error.contains("outside its mount"), "{error}");
    }

    #[test]
    fn the_guest_roots_are_recognised_so_an_undeclared_mount_is_not_run_blind() {
        let f = fixture();
        assert!(f.map.mentions_guest_root("/var/lingxi/local-app-build/other/store/project/x").is_some());
        assert!(f.map.mentions_guest_root(&f.map.text(PROJECT)).is_none());
    }

    #[test]
    fn a_missing_host_directory_or_a_duplicate_guest_path_is_refused_up_front() {
        let toolchain = Toolchain { guest_bin: "/usr/bin".into(), host_bin: PathBuf::from("/nonexistent") };
        let missing = Mount {
            host_path: PathBuf::from("/definitely/not/here"),
            guest_path: PROJECT.into(),
            read_only: false,
            kind: MountKind::Project,
        };
        assert!(PathMap::new(&[missing], &toolchain).unwrap_err().contains("/definitely/not/here"));
        let dir = tempfile::tempdir().unwrap();
        let twice = |kind| Mount { host_path: dir.path().to_path_buf(), guest_path: PROJECT.into(), read_only: false, kind };
        let error = PathMap::new(&[twice(MountKind::Project), twice(MountKind::DependencyStore)], &toolchain).unwrap_err();
        assert!(error.contains("two mounts claim"), "{error}");
    }
}
