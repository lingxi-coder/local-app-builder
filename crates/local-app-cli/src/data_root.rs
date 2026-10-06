//! Where the CLI keeps its data.
//!
//! The service takes the data root as an injected path (see `local_apps::storage`); choosing it is the host's job,
//! and this is the CLI's rule: `--data-root`, then `LOCAL_APP_DATA_ROOT`, then the platform default.

use crate::Env;
use std::path::PathBuf;

/// Which rule picked the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataRootSource {
    /// The `--data-root` option.
    Flag,
    /// The `LOCAL_APP_DATA_ROOT` variable.
    Variable,
    /// The platform default.
    Default,
}

impl DataRootSource {
    /// A short label for reports.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Flag => "--data-root",
            Self::Variable => "LOCAL_APP_DATA_ROOT",
            Self::Default => "default",
        }
    }
}

/// A chosen data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoot {
    /// The directory (it may not exist yet).
    pub path: PathBuf,
    /// Which rule chose it.
    pub source: DataRootSource,
}

/// Pick the data root. Relative paths are refused: a root that moves with the working directory would put one
/// person's apps in several places.
pub fn resolve_data_root(flag: Option<&str>, env: &Env) -> Result<DataRoot, String> {
    let (raw, source) = if let Some(flag) = flag {
        (PathBuf::from(flag), DataRootSource::Flag)
    } else if let Some(var) = env.var("LOCAL_APP_DATA_ROOT") {
        (PathBuf::from(var), DataRootSource::Variable)
    } else {
        (default_root(env)?, DataRootSource::Default)
    };
    if !raw.is_absolute() {
        return Err(format!("the data root must be an absolute path, got `{}`", raw.display()));
    }
    Ok(DataRoot { path: raw, source })
}

fn default_root(env: &Env) -> Result<PathBuf, String> {
    let home = env.var("HOME").ok_or("HOME is not set, so there is no default data root; pass --data-root")?;
    if cfg!(target_os = "macos") {
        Ok(PathBuf::from(home).join("Library/Application Support/local-app"))
    } else if let Some(xdg) = env.var("XDG_DATA_HOME") {
        Ok(PathBuf::from(xdg).join("local-app"))
    } else {
        Ok(PathBuf::from(home).join(".local/share/local-app"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        Env::new(pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())), vec![])
    }

    #[test]
    fn the_flag_beats_the_variable_beats_the_default() {
        let e = env(&[("HOME", "/home/a"), ("LOCAL_APP_DATA_ROOT", "/var/x")]);
        let r = resolve_data_root(Some("/tmp/flag"), &e).unwrap();
        assert_eq!((r.path, r.source), ("/tmp/flag".into(), DataRootSource::Flag));
        let r = resolve_data_root(None, &e).unwrap();
        assert_eq!((r.path, r.source), ("/var/x".into(), DataRootSource::Variable));
        let r = resolve_data_root(None, &env(&[("HOME", "/home/a")])).unwrap();
        assert_eq!(r.source, DataRootSource::Default);
        assert!(r.path.starts_with("/home/a"));
        assert!(r.path.ends_with("local-app"));
    }

    #[test]
    fn an_empty_variable_is_unset() {
        let r = resolve_data_root(None, &env(&[("HOME", "/h"), ("LOCAL_APP_DATA_ROOT", "")])).unwrap();
        assert_eq!(r.source, DataRootSource::Default);
    }

    #[test]
    fn a_relative_root_is_refused() {
        assert!(resolve_data_root(Some("data"), &env(&[])).unwrap_err().contains("absolute"));
        assert!(resolve_data_root(None, &env(&[("LOCAL_APP_DATA_ROOT", "rel")])).is_err());
    }

    #[test]
    fn no_home_and_no_override_is_an_error_that_names_the_way_out() {
        assert!(resolve_data_root(None, &env(&[])).unwrap_err().contains("--data-root"));
    }
}
