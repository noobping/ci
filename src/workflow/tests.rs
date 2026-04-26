use std::path::Path;

use serde_yaml::Value;

use super::{validate_native_workflow_keys, NativeWorkflowFile};

#[test]
fn native_clean_step_accepts_inline_run_and_top_level_options() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
steps:
  - name: Fresh clean
    use: clean
    cargo: true
    purge: true
    ignored: only
    run: cargo sweep -i
"#,
    )
    .expect("parse workflow");
    let step = file
        .steps
        .into_iter()
        .next()
        .expect("step")
        .into_step(Path::new(".ci/build.yml"), 0)
        .expect("valid step");

    assert_eq!(step.name.as_deref(), Some("Fresh clean"));
    assert_eq!(step.uses.as_deref(), Some("clean"));
    assert_eq!(step.run.as_deref(), Some("cargo sweep -i"));
    assert_eq!(step.extra.get("cargo").map(String::as_str), Some("true"));
    assert_eq!(step.extra.get("purge").map(String::as_str), Some("true"));
    assert_eq!(step.extra.get("ignored").map(String::as_str), Some("only"));
}

#[test]
fn native_non_clean_action_rejects_run_with_use() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
steps:
  - use: checkout
    run: git status
"#,
    )
    .expect("parse workflow");
    let err = file
        .steps
        .into_iter()
        .next()
        .expect("step")
        .into_step(Path::new(".ci/build.yml"), 0)
        .expect_err("step should be rejected");

    assert!(err
        .to_string()
        .contains("must define exactly one of `run` or `use`"));
}

#[test]
fn native_step_rejects_multiple_action_source_aliases() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
steps:
  - use: clean
    uses: checkout
"#,
    )
    .expect("parse workflow");
    let err = file
        .steps
        .into_iter()
        .next()
        .expect("step")
        .into_step(Path::new(".ci/build.yml"), 0)
        .expect_err("step should be rejected");

    assert!(err
        .to_string()
        .contains("must define only one of `use` or `uses`"));
}

#[test]
fn native_step_accepts_container_override() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
steps:
  - name: Install locally
    container: false
    read-only: true
    run: ci completion bash
"#,
    )
    .expect("parse workflow");
    let step = file
        .steps
        .into_iter()
        .next()
        .expect("step")
        .into_step(Path::new(".ci/build.yml"), 0)
        .expect("valid step");

    assert_eq!(step.container, Some(false));
    assert_eq!(step.readonly, Some(true));
}

#[test]
fn native_step_accepts_container_image_shorthand() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
steps:
  - name: Node lint
    container: docker.io/library/node:22-bookworm-slim
    run: npm test
"#,
    )
    .expect("parse workflow");
    let step = file
        .steps
        .into_iter()
        .next()
        .expect("step")
        .into_step(Path::new(".ci/build.yml"), 0)
        .expect("valid step");
    let container = step.container_config.expect("step container config");

    assert_eq!(step.container, Some(true));
    assert_eq!(
        container.image.as_deref(),
        Some("docker.io/library/node:22-bookworm-slim")
    );
}

#[test]
fn native_step_accepts_container_file_config() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
steps:
  - name: Tool check
    container:
      container-file: .ci/tools.Containerfile
      image: localhost/project-tools
      platform: linux/arm64
      working-directory: /work/tooling
      read-only: true
      env:
        TOOL_MODE: strict
      volumes:
        - /tmp:/tmp/ci-tools
      packages:
        - shellcheck
      components:
        - cargo-fmt
    run: tool check
"#,
    )
    .expect("parse workflow");
    let step = file
        .steps
        .into_iter()
        .next()
        .expect("step")
        .into_step(Path::new(".ci/build.yml"), 0)
        .expect("valid step");
    let container = step.container_config.expect("step container config");

    assert_eq!(step.container, Some(true));
    assert_eq!(container.file.as_deref(), Some(".ci/tools.Containerfile"));
    assert_eq!(container.image.as_deref(), Some("localhost/project-tools"));
    assert_eq!(container.platform.as_deref(), Some("linux/arm64"));
    assert_eq!(container.workdir.as_deref(), Some("/work/tooling"));
    assert_eq!(container.readonly, Some(true));
    assert_eq!(
        container.env.get("TOOL_MODE").map(String::as_str),
        Some("strict")
    );
    assert_eq!(container.volumes, vec!["/tmp:/tmp/ci-tools"]);
    assert_eq!(container.packages, vec!["shellcheck"]);
    assert_eq!(container.components, vec!["cargo-fmt"]);
}

#[test]
fn native_workflow_parses_container_arch_aliases() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
container:
  arch:
    - amd64
    - aarch64
steps:
  - run: echo ok
"#,
    )
    .expect("parse workflow");
    let arch = file
        .container
        .arch
        .to_vec()
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();

    assert_eq!(arch, vec!["x64", "arm64"]);
}

#[test]
fn native_workflow_accepts_top_level_tech_stack_aliases() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
tech-stack: node
steps:
  - run: npm run build
"#,
    )
    .expect("parse workflow");
    let metadata = file.metadata();

    assert_eq!(
        metadata.tech_stack,
        Some(crate::config::ContainerType::Node)
    );
}

#[test]
fn native_workflow_parses_dependencies() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
needs:
  - check
  - build
steps:
  - run: echo ok
"#,
    )
    .expect("parse workflow");

    assert_eq!(file.needs.to_vec(), vec!["check", "build"]);

    let alias: NativeWorkflowFile = serde_yaml::from_str(
        r#"
depends: check
steps:
  - run: echo ok
"#,
    )
    .expect("parse workflow");

    assert_eq!(alias.needs.to_vec(), vec!["check"]);
}

#[test]
fn native_workflow_defaults_merge_under_direct_fields() {
    let file: NativeWorkflowFile = serde_yaml::from_str(
        r#"
defaults:
  container:
    type: rust
    arch: amd64
    components:
      - cargo-fmt
container:
  arch: aarch64
steps:
  - run: echo ok
"#,
    )
    .expect("parse workflow");
    let metadata = file.metadata();

    assert_eq!(
        metadata
            .container
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["arm64"]
    );
    assert_eq!(metadata.container.components, vec!["cargo-fmt"]);
}

#[test]
fn native_workflow_validation_rejects_unknown_top_level_keys() {
    let value: Value = serde_yaml::from_str(
        r#"
contaner:
  type: rust
steps:
  - run: echo ok
"#,
    )
    .expect("parse yaml value");

    let err = validate_native_workflow_keys(&value, Path::new(".ci/build.yml"))
        .expect_err("unknown key should be rejected");

    assert!(err.to_string().contains("unknown key `contaner`"));
}
