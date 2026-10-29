//! Landlock re-exec helper: `synthesis-sandbox-exec '<policy json>' -- <program> [args...]`.
//! Applies the policy to itself (fail closed if the kernel cannot enforce it), then replaces
//! itself with the program. Exit 126: sandbox could not be applied; 127: exec failed.

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use std::os::unix::process::CommandExt;

    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (policy, program, rest) = match synthesis_sandbox::parse_helper_args(&args) {
        Ok(v) => v,
        Err(e) => return fail(126, &e),
    };
    if let Err(e) = synthesis_sandbox::restrict_current_process(&policy) {
        return fail(126, &e);
    }
    let err = std::process::Command::new(program).args(rest).exec();
    fail(127, &err)
}

#[cfg(target_os = "linux")]
fn fail(code: u8, e: &dyn std::fmt::Display) -> std::process::ExitCode {
    eprintln!("synthesis-sandbox-exec: {e}");
    std::process::ExitCode::from(code)
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("synthesis-sandbox-exec: Landlock is Linux-only");
    std::process::ExitCode::from(126)
}
