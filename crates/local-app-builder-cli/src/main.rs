//! The `local-app-builder` binary. All behaviour lives in the library so it can be tested without a process.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = local_app_builder_cli::Env::from_process();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let code = local_app_builder_cli::run(&args, &env, &mut out, &mut err);
    let _ = out.flush();
    ExitCode::from(code)
}
