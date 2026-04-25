use std::fs;

use super::{load_actions_workflow, ActionStep, ActionsProvider};

#[test]
fn workflow_and_job_permissions_are_ignored() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("permissions.yml");
    fs::write(
        &path,
        r#"
name: permissions
on: push
permissions:
  contents: read
jobs:
  build:
    continue-on-error: "${{ matrix.stream.name == 'next' }}"
    permissions:
      packages: write
    steps:
      - run: echo ok
        continue-on-error: "${{ false }}"
"#,
    )
    .expect("write workflow");

    let workflow = load_actions_workflow(&path, ActionsProvider::GitHub).expect("load workflow");

    assert_eq!(workflow.name, "permissions");
    assert_eq!(workflow.jobs.len(), 1);
    assert_eq!(workflow.jobs[0].id, "build");
    assert!(!workflow.jobs[0].continue_on_error);
    match &workflow.jobs[0].steps[0] {
        ActionStep::Run(step) => assert!(!step.continue_on_error),
        _ => panic!("expected run step"),
    }
}
