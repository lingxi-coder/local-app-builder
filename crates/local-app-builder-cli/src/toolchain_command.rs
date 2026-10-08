//! `local-app-builder toolchain`: provide, and check, the Node and pnpm builds run with.
//!
//! ```text
//! local-app-builder toolchain status  [--data-root DIR]
//! local-app-builder toolchain install [--data-root DIR] [--from DIR]
//! ```
//!
//! Installing is a command of its own, not something the first build does on the way: it downloads about 80 MB, and
//! a person should see that happen. `--from DIR` takes the same pinned archives from a directory instead of the
//! network, for a machine that has none.

use crate::{resolve_data_root, Env};
use local_app_builder_host::{Source, Spec, Status, Toolchains};
use std::io::Write;
use std::path::PathBuf;

/// Run `toolchain <args…>` for `spec`. Returns the exit code: 0 success (for `status`: the toolchain is ready), 1 a
/// failure or a toolchain that is not ready, 2 a usage error.
pub fn run_toolchain(
    args: &[String],
    env: &Env,
    spec: Result<Spec, String>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let Some((action, rest)) = args.split_first() else {
        let _ = writeln!(
            err,
            "local-app-builder toolchain: expected `status` or `install`"
        );
        return 2;
    };
    if !matches!(action.as_str(), "status" | "install") {
        let _ = writeln!(err, "local-app-builder toolchain: unknown action `{action}` (expected `status` or `install`)");
        return 2;
    }
    let mut flag_root: Option<String> = None;
    let mut from: Option<PathBuf> = None;
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match (arg.as_str(), action.as_str()) {
            ("--data-root", _) => match it.next() {
                Some(v) => flag_root = Some(v.clone()),
                None => {
                    let _ = writeln!(
                        err,
                        "local-app-builder toolchain: --data-root needs a directory"
                    );
                    return 2;
                }
            },
            ("--from", "install") => match it.next() {
                Some(v) => from = Some(PathBuf::from(v)),
                None => {
                    let _ = writeln!(err, "local-app-builder toolchain: --from needs a directory");
                    return 2;
                }
            },
            (other, _) => {
                let _ = writeln!(
                    err,
                    "local-app-builder toolchain {action}: unknown option `{other}`"
                );
                return 2;
            }
        }
    }
    let root = match resolve_data_root(flag_root.as_deref(), env) {
        Ok(root) => root,
        Err(message) => {
            let _ = writeln!(err, "local-app-builder toolchain: {message}");
            return 1;
        }
    };
    let spec = match spec {
        Ok(spec) => spec,
        Err(reason) => {
            let _ = writeln!(err, "local-app-builder toolchain: {reason}");
            return 1;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = writeln!(
                err,
                "local-app-builder toolchain: cannot start the async runtime: {error}"
            );
            return 1;
        }
    };
    let toolchains = Toolchains::in_data_root(&root.path);
    if action == "status" {
        return runtime.block_on(status(&toolchains, &spec, out));
    }
    let source = from.map_or(Source::Network, Source::Directory);
    let progress = |line: &str| {
        // Progress goes to stderr from a plain closure; a failed write must not fail the install.
        let _ = writeln!(std::io::stderr(), "local-app-builder toolchain: {line}");
    };
    match runtime.block_on(toolchains.install(&spec, &source, &progress)) {
        Ok(receipt) => {
            let _ = writeln!(
                out,
                "{} is installed at {} (node {}, pnpm {})",
                spec.key,
                toolchains.dir(&spec).display(),
                receipt.node.version,
                receipt.pnpm.version
            );
            0
        }
        Err(error) => {
            let _ = writeln!(err, "local-app-builder toolchain: {error}");
            1
        }
    }
}

async fn status(toolchains: &Toolchains, spec: &Spec, out: &mut dyn Write) -> u8 {
    match toolchains.status(spec).await {
        Status::Ready(receipt) => {
            let _ = writeln!(
                out,
                "ready: {} at {}\n  node {} (sha256 {})\n  pnpm {} (sha256 {})",
                spec.key,
                toolchains.dir(spec).display(),
                receipt.node.version,
                receipt.node.binary_sha256,
                receipt.pnpm.version,
                receipt.pnpm.binary_sha256
            );
            0
        }
        state => {
            let headline = match &state {
                Status::Damaged(why) => format!("damaged: {} ({why})", spec.key),
                _ => format!("not installed: {}", spec.key),
            };
            let _ = writeln!(out, "{headline}");
            let _ = writeln!(out, "  `local-app-builder toolchain install` downloads and verifies it. To install without network access, put these files in a directory and add `--from DIR`:");
            for artifact in [&spec.node, &spec.pnpm] {
                let _ = writeln!(
                    out,
                    "    {}  ({} bytes, sha256 {})\n      from {}",
                    artifact.file_name(),
                    artifact.size,
                    artifact.sha256,
                    artifact.url
                );
            }
            1
        }
    }
}
