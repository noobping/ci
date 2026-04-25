use std::fs;

use tempfile::TempDir;

use crate::config::ContainerType;
use crate::workflow::WorkflowSource;

use super::generated_default_workflows;

#[test]
fn generated_default_rust_build_uses_container_when_cargo_is_missing() {
    let temp = TempDir::new().expect("tempdir");
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("cargo");
    let ci_dir = temp.path().join(".ci");

    let workflows = generated_default_workflows(temp.path(), &ci_dir, None, &|_| false);

    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].name, "build");
    match &workflows[0].source {
        WorkflowSource::NativeYaml(native) => {
            assert_eq!(
                native.metadata.on.to_vec(),
                vec!["manual".to_string(), "pre-push".to_string()]
            );
            assert_eq!(native.metadata.tech_stack, Some(ContainerType::Rust));
            assert_eq!(native.metadata.container.kind, Some(ContainerType::Rust));
            assert_eq!(native.steps.len(), 1);
            assert_eq!(native.steps[0].run.as_deref(), Some("cargo build"));
        }
        _ => panic!("expected native workflow"),
    }
}

#[test]
fn generated_default_rust_build_uses_host_when_cargo_is_available() {
    let temp = TempDir::new().expect("tempdir");
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("cargo");

    let workflows =
        generated_default_workflows(temp.path(), &temp.path().join(".ci"), None, &|_| true);

    match &workflows[0].source {
        WorkflowSource::NativeYaml(native) => {
            assert_eq!(native.metadata.container.kind, None);
        }
        _ => panic!("expected native workflow"),
    }
}

#[test]
fn generated_default_node_build_uses_node_stack() {
    let temp = TempDir::new().expect("tempdir");
    fs::write(temp.path().join("package.json"), "{\"scripts\":{}}").expect("package");

    let workflows =
        generated_default_workflows(temp.path(), &temp.path().join(".ci"), None, &|tool| {
            tool != "npm"
        });

    match &workflows[0].source {
        WorkflowSource::NativeYaml(native) => {
            assert_eq!(native.metadata.container.kind, Some(ContainerType::Node));
            assert_eq!(
                native.steps[0].run.as_deref(),
                Some("npm install && npm run build --if-present")
            );
        }
        _ => panic!("expected native workflow"),
    }
}

#[test]
fn generated_default_build_honors_requested_stack() {
    let temp = TempDir::new().expect("tempdir");

    let workflows = generated_default_workflows(
        temp.path(),
        &temp.path().join(".ci"),
        Some(ContainerType::Go),
        &|_| false,
    );

    match &workflows[0].source {
        WorkflowSource::NativeYaml(native) => {
            assert_eq!(native.metadata.container.kind, Some(ContainerType::Go));
            assert_eq!(native.steps[0].run.as_deref(), Some("go build ./..."));
        }
        _ => panic!("expected native workflow"),
    }
}
