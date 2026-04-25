use std::process::Command;

#[test]
fn schema_command_prints_workflow_schema_without_repo() {
    let output = Command::new(env!("CARGO_BIN_EXE_ci"))
        .args(["schema", "workflow"])
        .output()
        .expect("run ci schema workflow");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("schema output is utf-8");
    assert!(stdout.contains("\"title\": \"ci native workflow\""));
    assert!(stdout.contains("\"container\""));
}
