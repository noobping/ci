use std::path::Path;

use serde_yaml::Value;

use super::{
    default_container_config, merge_config_files, validate_config_keys, ConfigFile, ContainerType,
    InstallMode,
};

#[test]
fn defaults_arch_accepts_single_value_or_list() {
    let single: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  arch: amd64
"#,
    )
    .expect("parse single arch");
    assert_eq!(
        single
            .defaults
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["x64"]
    );

    let many: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  arch:
    - x86_64
    - aarch64
"#,
    )
    .expect("parse arch list");
    assert_eq!(
        many.defaults
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["x64", "arm64"]
    );
}

#[test]
fn container_config_accepts_type_arch_packages_and_components() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
workflows:
  build:
    container:
      type: rust
      arch:
        - amd64
        - aarch64
      packages:
        - htop
      components:
        - cargo-fmt
      readonly: true
      env:
        RUST_LOG: debug
      volumes:
        - ~/.cache/ci:/cache
"#,
    )
    .expect("parse container config");
    let container = &file.workflows.get("build").expect("workflow").container;

    assert_eq!(container.kind, Some(ContainerType::Rust));
    assert_eq!(
        container
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["x64", "arm64"]
    );
    assert_eq!(container.packages, vec!["htop"]);
    assert_eq!(container.components, vec!["cargo-fmt"]);
    assert_eq!(container.readonly, Some(true));
    assert_eq!(
        container.env.get("RUST_LOG").map(String::as_str),
        Some("debug")
    );
    assert_eq!(container.volumes, vec!["~/.cache/ci:/cache"]);
}

#[test]
fn container_readonly_accepts_yaml_aliases_and_overrides_defaults() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  container:
    read-only: true
workflows:
  build:
    container:
      read_only: false
"#,
    )
    .expect("parse readonly aliases");
    let defaults = file.root_defaults.merge(&file.defaults);
    let container = default_container_config(&defaults)
        .merge(&file.workflows.get("build").expect("workflow").container);

    assert_eq!(container.readonly, Some(false));
}

#[test]
fn tech_stack_aliases_become_default_container_type() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  tech-stack: node
"#,
    )
    .expect("parse tech stack");
    let container = default_container_config(&file.defaults);

    assert_eq!(container.kind, Some(ContainerType::Node));

    let root: ConfigFile = serde_yaml::from_str(
        r#"
type: golang
"#,
    )
    .expect("parse root tech stack");
    let defaults = root.root_defaults.merge(&root.defaults);

    assert_eq!(
        default_container_config(&defaults).kind,
        Some(ContainerType::Go)
    );
}

#[test]
fn quiet_default_merges_like_silent_default() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
quiet: true
defaults:
  silent: true
"#,
    )
    .expect("parse quiet and silent defaults");
    let defaults = file.root_defaults.merge(&file.defaults);

    assert_eq!(defaults.quiet, Some(true));
    assert_eq!(defaults.silent, Some(true));
}

#[test]
fn defaults_arch_becomes_default_container_arch() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  arch:
    - amd64
    - aarch64
"#,
    )
    .expect("parse defaults");
    let container = default_container_config(&file.defaults);

    assert_eq!(
        container
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["x64", "arm64"]
    );
}

#[test]
fn config_file_accepts_default_fields_at_root() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
container:
  type: rust
  arch:
    - amd64
    - aarch64
  components:
    - cargo-fmt
"#,
    )
    .expect("parse root defaults");
    let defaults = file.root_defaults.merge(&file.defaults);

    assert_eq!(defaults.container.kind, Some(ContainerType::Rust));
    assert_eq!(
        defaults
            .container
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["x64", "arm64"]
    );
    assert_eq!(defaults.container.components, vec!["cargo-fmt"]);
}

#[test]
fn explicit_defaults_override_root_default_shorthand() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
container:
  arch: amd64
defaults:
  container:
    arch: aarch64
"#,
    )
    .expect("parse mixed defaults");
    let defaults = file.root_defaults.merge(&file.defaults);

    assert_eq!(
        defaults
            .container
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["arm64"]
    );
}

#[test]
fn defaults_container_arch_overrides_defaults_arch_for_containers() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  arch: amd64
  container:
    arch: aarch64
"#,
    )
    .expect("parse defaults");
    let container = default_container_config(&file.defaults);

    assert_eq!(
        container
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["arm64"]
    );
}

#[test]
fn config_validation_rejects_unknown_keys() {
    let value: Value = serde_yaml::from_str(
        r#"
defaults:
  contaner:
    type: rust
"#,
    )
    .expect("parse yaml value");

    let err = validate_config_keys(&value, Path::new(".ci/config.yml"))
        .expect_err("unknown key should be rejected");

    assert!(err.to_string().contains("unknown key `contaner`"));
}

#[test]
fn config_accepts_default_install_mode() {
    let file: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  install-mode: copy
"#,
    )
    .expect("parse install mode");

    assert_eq!(file.defaults.install_mode, Some(InstallMode::Copy));
}

#[test]
fn layered_config_merges_from_system_to_user_to_project() {
    let system: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  install-mode: link
  shell: /bin/system-sh
hooks:
  pre-push:
    container:
      type: rust
"#,
    )
    .expect("parse system config");
    let user: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  install-mode: copy
hooks:
  pre-push:
    container:
      arch: arm64
"#,
    )
    .expect("parse user config");
    let project: ConfigFile = serde_yaml::from_str(
        r#"
defaults:
  shell: /bin/project-sh
"#,
    )
    .expect("parse project config");

    let merged = merge_config_files([system, user, project]);
    let defaults = merged.root_defaults.merge(&merged.defaults);
    let pre_push = merged.hooks.get("pre-push").expect("pre-push hook");

    assert_eq!(defaults.install_mode, Some(InstallMode::Copy));
    assert_eq!(defaults.shell.as_deref(), Some("/bin/project-sh"));
    assert_eq!(pre_push.container.kind, Some(ContainerType::Rust));
    assert_eq!(
        pre_push
            .container
            .arch
            .to_vec()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        vec!["arm64"]
    );
}
