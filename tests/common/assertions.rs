use std::process::{Command, Output};

pub fn ci_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ci"))
}

pub fn output(mut command: Command) -> Output {
    command.output().expect("run command")
}

pub fn assert_success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "expected success\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub fn assert_failure(output: Output, code: i32) -> Output {
    assert_eq!(
        output.status.code(),
        Some(code),
        "expected exit code {code}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
