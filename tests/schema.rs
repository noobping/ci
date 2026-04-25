mod common;

use common::assertions::{assert_failure, assert_success, ci_command, output, stderr, stdout};

#[test]
fn schema_command_prints_workflow_schema_without_repo() {
    let mut command = ci_command();
    command.args(["schema", "workflow"]);
    let output = assert_success(output(command));

    let stdout = stdout(&output);
    assert!(stdout.contains("\"title\": \"ci native workflow\""));
    assert!(stdout.contains("\"container\""));
}

#[test]
fn schema_command_rejects_unknown_subject_without_repo() {
    let mut command = ci_command();
    command.args(["schema", "wat"]);
    let output = assert_failure(output(command), 2);

    assert!(stderr(&output).contains("unknown schema `wat`"));
}
