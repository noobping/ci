use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::config::Architecture;
use crate::git::CleanIgnoredMode;

use super::{parse_cleanup_ignored_mode, parse_path_list, run_export_step, run_link_step};
use crate::conditions::{
    evaluate_condition, evaluate_condition_with_probe, executable_exists, ExpressionContext,
};
use crate::containers::{
    generated_native_container_image_name, generated_native_containerfile,
    normalized_rust_components,
};

fn expr_ctx<'a>(
    root: &'a Path,
    env: &'a BTreeMap<String, String>,
    success: bool,
    previous_failed: bool,
) -> ExpressionContext<'a> {
    let empty = Box::leak(Box::new(BTreeMap::new()));
    ExpressionContext {
        event: "manual",
        branch: None,
        root,
        env,
        matrix: empty,
        inputs: empty,
        success,
        previous_failed,
    }
}

fn expr_ctx_with_inputs<'a>(
    root: &'a Path,
    env: &'a BTreeMap<String, String>,
    inputs: &'a BTreeMap<String, String>,
) -> ExpressionContext<'a> {
    let empty = Box::leak(Box::new(BTreeMap::new()));
    ExpressionContext {
        event: "manual",
        branch: None,
        root,
        env,
        matrix: empty,
        inputs,
        success: true,
        previous_failed: false,
    }
}

#[test]
fn native_condition_defaults_to_success() {
    let temp = TempDir::new().expect("tempdir");
    let env = BTreeMap::new();
    assert!(evaluate_condition(
        None,
        &expr_ctx(temp.path(), &env, true, false)
    ));
    assert!(!evaluate_condition(
        None,
        &expr_ctx(temp.path(), &env, false, true)
    ));
}

#[test]
fn native_condition_shorthands_work() {
    let temp = TempDir::new().expect("tempdir");
    let env = BTreeMap::new();
    let failed = expr_ctx(temp.path(), &env, false, true);
    let succeeded = expr_ctx(temp.path(), &env, true, false);

    assert!(evaluate_condition(Some("always"), &failed));
    assert!(evaluate_condition(Some("failure"), &failed));
    assert!(!evaluate_condition(Some("success"), &failed));
    assert!(evaluate_condition(Some("success"), &succeeded));
    assert!(!evaluate_condition(Some("failure"), &succeeded));
    assert!(evaluate_condition(Some("!failure"), &succeeded));
    assert!(evaluate_condition(Some("is success"), &succeeded));
    assert!(evaluate_condition(Some("not failure"), &succeeded));
}

#[test]
fn arch_conditions_match_selected_arch() {
    let temp = TempDir::new().expect("tempdir");
    let mut env = BTreeMap::new();
    env.insert("CI_ARCH".to_string(), "x64".to_string());
    env.insert("TARGET_ARCH".to_string(), "amd64".to_string());
    let ctx = expr_ctx(temp.path(), &env, true, false);

    assert!(evaluate_condition(Some("arch(x64)"), &ctx));
    assert!(evaluate_condition(Some("arch(amd64)"), &ctx));
    assert!(evaluate_condition(Some("arch(linux/amd64)"), &ctx));
    assert!(evaluate_condition(Some("arch(arm64, x64)"), &ctx));
    assert!(evaluate_condition(Some("arch(env.TARGET_ARCH)"), &ctx));
    assert!(!evaluate_condition(Some("arch(arm64)"), &ctx));

    let mut host_env = BTreeMap::new();
    let host_arch = Architecture::host().to_string();
    host_env.insert("CI_ARCH".to_string(), host_arch.clone());
    host_env.insert("CI_HOST_ARCH".to_string(), host_arch);
    let host_ctx = expr_ctx(temp.path(), &host_env, true, false);
    assert!(evaluate_condition(Some("arch(host)"), &host_ctx));
    assert!(evaluate_condition(Some("arch host"), &host_ctx));
    assert!(evaluate_condition(Some("arch(host arch)"), &host_ctx));
    assert!(evaluate_condition(
        Some("arch(env.CI_HOST_ARCH)"),
        &host_ctx
    ));
}

#[test]
fn exists_conditions_work_with_explicit_paths() {
    let temp = TempDir::new().expect("tempdir");
    let tool = temp.path().join("tool");
    fs::write(&tool, "#!/bin/sh\nexit 0\n").expect("write tool");

    let mut permissions = fs::metadata(&tool).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&tool, permissions).expect("chmod");

    let tool = tool.display().to_string();
    let env = BTreeMap::new();
    let condition = format!("exists('{tool}')");
    let missing = format!("missing('{tool}-missing')");

    assert!(evaluate_condition(
        Some(&condition),
        &expr_ctx(temp.path(), &env, true, false)
    ));
    assert!(evaluate_condition(
        Some(&missing),
        &expr_ctx(temp.path(), &env, true, false)
    ));
    assert!(evaluate_condition(
        Some(&format!("exists(cmd:'{tool}')")),
        &expr_ctx(temp.path(), &env, true, false)
    ));
    assert!(evaluate_condition(
        Some(&format!("missing(command:'{tool}-missing')")),
        &expr_ctx(temp.path(), &env, true, false)
    ));
    assert!(executable_exists(&tool));
    assert!(!executable_exists(&format!("{tool}-missing")));
}

#[test]
fn exists_checks_repo_relative_files_and_env_paths() {
    let temp = TempDir::new().expect("tempdir");
    fs::create_dir_all(temp.path().join("target")).expect("create dir");
    fs::write(temp.path().join("marker.txt"), "ok").expect("write file");
    fs::write(temp.path().join("host"), "ok").expect("write host file");

    let mut env = BTreeMap::new();
    env.insert("BUILD_DIR".to_string(), "target".to_string());
    env.insert("HOME".to_string(), "/tmp/test-home".to_string());

    let ctx = expr_ctx(temp.path(), &env, true, false);
    assert!(evaluate_condition(Some("exists(target)"), &ctx));
    assert!(evaluate_condition(Some("exists target"), &ctx));
    assert!(evaluate_condition(Some("exists(env.BUILD_DIR)"), &ctx));
    assert!(evaluate_condition(Some("exists(marker.txt)"), &ctx));
    assert!(evaluate_condition(Some("is(host)"), &ctx));
    assert!(evaluate_condition(Some("is host"), &ctx));
    assert!(evaluate_condition(Some("is marker.txt"), &ctx));
    assert!(evaluate_condition(Some("exists(path:marker.txt)"), &ctx));
    assert!(evaluate_condition(Some("exists(path:env.BUILD_DIR)"), &ctx));
    assert!(evaluate_condition(Some("exists(file:marker.txt)"), &ctx));
    assert!(evaluate_condition(Some("has(file:marker.txt)"), &ctx));
    assert!(evaluate_condition(Some("has file:marker.txt"), &ctx));
    assert!(evaluate_condition(Some("is(file:marker.txt)"), &ctx));
    assert!(evaluate_condition(Some("is file:marker.txt"), &ctx));
    assert!(evaluate_condition(Some("is exists(file:marker.txt)"), &ctx));
    assert!(evaluate_condition(Some("is exists file:marker.txt"), &ctx));
    assert!(evaluate_condition(Some("missing(file:target)"), &ctx));
    assert!(evaluate_condition(Some("missing file:target"), &ctx));
    assert!(evaluate_condition(Some("not(file:target)"), &ctx));
    assert!(evaluate_condition(Some("not file:target"), &ctx));
    assert!(evaluate_condition(Some("is missing(file:target)"), &ctx));
    assert!(evaluate_condition(Some("is missing file:target"), &ctx));
    assert!(evaluate_condition(Some("not exists(file:target)"), &ctx));
    assert!(evaluate_condition(Some("not exists file:target"), &ctx));
    assert!(evaluate_condition(
        Some("not missing(file:marker.txt)"),
        &ctx
    ));
    assert!(evaluate_condition(
        Some("not missing file:marker.txt"),
        &ctx
    ));
    assert!(evaluate_condition(Some("exists(dir:target)"), &ctx));
    assert!(evaluate_condition(Some("exists(directory:target)"), &ctx));
    assert!(evaluate_condition(Some("missing(dir:marker.txt)"), &ctx));
    assert!(evaluate_condition(Some("exists(env:HOME)"), &ctx));
    assert!(evaluate_condition(
        Some("missing(env:NOT_SET_FOR_TEST)"),
        &ctx
    ));
    assert!(evaluate_condition(Some("missing(dist)"), &ctx));
}

#[test]
fn exists_conditions_can_probe_container_commands() {
    let temp = TempDir::new().expect("tempdir");
    let env = BTreeMap::new();
    let ctx = expr_ctx(temp.path(), &env, true, false);
    let probe = |name: &str| -> crate::error::Result<bool> {
        Ok(matches!(name, "definitely-ci-probe-tool" | "toolbox"))
    };

    assert!(evaluate_condition_with_probe(
        Some("exists(definitely-ci-probe-tool)"),
        &ctx,
        Some(&probe)
    )
    .expect("evaluate exists"));
    assert!(evaluate_condition_with_probe(
        Some("exists(cmd:definitely-ci-probe-tool)"),
        &ctx,
        Some(&probe)
    )
    .expect("evaluate command exists"));
    assert!(
        evaluate_condition_with_probe(Some("exists(executable:toolbox)"), &ctx, Some(&probe))
            .expect("evaluate executable exists")
    );
    assert!(!evaluate_condition_with_probe(
        Some("missing(definitely-ci-probe-tool)"),
        &ctx,
        Some(&probe)
    )
    .expect("evaluate missing"));
    assert!(evaluate_condition_with_probe(
        Some("missing(definitely-ci-probe-tool-missing)"),
        &ctx,
        Some(&probe)
    )
    .expect("evaluate absent"));
    assert!(!evaluate_condition_with_probe(
        Some("missing(definitely-ci-probe-tool) and exists(toolbox)"),
        &ctx,
        Some(&probe)
    )
    .expect("evaluate word and"));
    assert!(evaluate_condition_with_probe(
        Some("missing(definitely-ci-probe-tool-missing) and exists(toolbox)"),
        &ctx,
        Some(&probe)
    )
    .expect("evaluate fallback condition"));
    assert!(evaluate_condition_with_probe(
        Some("exists(definitely-ci-probe-tool) or exists(nope)"),
        &ctx,
        Some(&probe)
    )
    .expect("evaluate word or"));
}

#[test]
fn exists_conditions_can_reference_action_inputs() {
    let temp = TempDir::new().expect("tempdir");
    fs::create_dir_all(temp.path().join("target/release")).expect("create dir");
    fs::write(temp.path().join("target/release/ci"), "bin").expect("write file");

    let env = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    inputs.insert("src".to_string(), "target/release/ci".to_string());

    let ctx = expr_ctx_with_inputs(temp.path(), &env, &inputs);
    assert!(evaluate_condition(Some("exists(src)"), &ctx));
    assert!(evaluate_condition(Some("exists(inputs.src)"), &ctx));
}

#[test]
fn cleanup_ignored_mode_parses_supported_values() {
    assert_eq!(
        parse_cleanup_ignored_mode("cleanup", None).expect("default ignored mode"),
        CleanIgnoredMode::Exclude
    );
    assert_eq!(
        parse_cleanup_ignored_mode("cleanup", Some("true")).expect("include ignored"),
        CleanIgnoredMode::Include
    );
    assert_eq!(
        parse_cleanup_ignored_mode("cleanup", Some("only")).expect("ignored only"),
        CleanIgnoredMode::Only
    );
    assert!(parse_cleanup_ignored_mode("cleanup", Some("maybe")).is_err());
}

#[test]
fn path_list_accepts_yaml_sequence_text() {
    assert_eq!(
        parse_path_list("- target/release/ci\n- dist/app.tar.gz"),
        vec!["target/release/ci", "dist/app.tar.gz"]
    );
}

#[test]
fn rust_component_aliases_normalize_to_rustup_components() {
    assert_eq!(
        normalized_rust_components(&[
            "cargo-fmt".to_string(),
            "cargo-clippy".to_string(),
            "rust-src".to_string(),
            "rustfmt".to_string(),
        ])
        .expect("normalize components"),
        vec!["rustfmt", "clippy", "rust-src"]
    );
}

#[test]
fn generated_containerfile_installs_components_before_packages() {
    let content = generated_native_containerfile(
        "docker.io/library/rust:latest",
        &["htop".to_string()],
        &["rustfmt".to_string(), "clippy".to_string()],
    );

    assert!(content.contains("RUN rustup component add 'rustfmt' 'clippy'"));
    assert!(content.contains("apt-get install -y --no-install-recommends 'htop'"));
    assert!(
        content
            .find("rustup component add")
            .expect("components line")
            < content.find("apt-get install").expect("package line")
    );
}

#[test]
fn generated_container_image_name_uses_localhost_reference() {
    assert_eq!(
        generated_native_container_image_name("build", "linux/amd64"),
        "localhost/ci-build-linux-amd64:latest"
    );
}

#[test]
fn export_single_file_uses_exact_destination_path() {
    let temp = TempDir::new().expect("tempdir");
    fs::create_dir_all(temp.path().join("target/release")).expect("create target");
    fs::write(temp.path().join("target/release/ci"), "bin").expect("write source");

    let mut inputs = BTreeMap::new();
    inputs.insert("from".to_string(), "target/release/ci".to_string());
    inputs.insert("to".to_string(), "dist/ci".to_string());

    assert_eq!(
        run_export_step(temp.path(), &inputs).expect("export should succeed"),
        0
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/ci")).expect("read exported file"),
        "bin"
    );
}

#[test]
fn export_existing_target_requires_replace_or_overwrite() {
    let temp = TempDir::new().expect("tempdir");
    fs::create_dir_all(temp.path().join("target/release")).expect("create target");
    fs::create_dir_all(temp.path().join("dist")).expect("create dist");
    fs::write(temp.path().join("target/release/ci"), "new").expect("write source");
    fs::write(temp.path().join("dist/ci"), "old").expect("write existing");

    let mut inputs = BTreeMap::new();
    inputs.insert("src".to_string(), "target/release/ci".to_string());
    inputs.insert("dest".to_string(), "dist/ci".to_string());

    let err = run_export_step(temp.path(), &inputs).expect_err("export should fail");
    assert!(err
        .to_string()
        .contains("set `replace: true` or `overwrite: true`"));
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/ci")).expect("read existing file"),
        "old"
    );

    inputs.insert("overwrite".to_string(), "true".to_string());
    assert_eq!(
        run_export_step(temp.path(), &inputs).expect("export should overwrite"),
        0
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/ci")).expect("read overwritten file"),
        "new"
    );
}

#[test]
fn export_multiple_sources_uses_destination_as_directory() {
    let temp = TempDir::new().expect("tempdir");
    fs::write(temp.path().join("one.txt"), "one").expect("write source one");
    fs::write(temp.path().join("two.txt"), "two").expect("write source two");

    let mut inputs = BTreeMap::new();
    inputs.insert("source".to_string(), "one.txt\ntwo.txt".to_string());
    inputs.insert("destination".to_string(), "out".to_string());

    assert_eq!(
        run_export_step(temp.path(), &inputs).expect("export should succeed"),
        0
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("out/one.txt")).expect("read one"),
        "one"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("out/two.txt")).expect("read two"),
        "two"
    );
}

#[test]
fn link_single_file_uses_relative_symlink_target() {
    let temp = TempDir::new().expect("tempdir");
    fs::write(temp.path().join("ci.x64"), "bin").expect("write source");

    let mut inputs = BTreeMap::new();
    inputs.insert("from".to_string(), "ci.x64".to_string());
    inputs.insert("to".to_string(), "bin/ci".to_string());

    assert_eq!(
        run_link_step(temp.path(), &inputs).expect("link should succeed"),
        0
    );
    let link = temp.path().join("bin/ci");
    assert!(fs::symlink_metadata(&link)
        .expect("link metadata")
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&link).expect("read link"),
        PathBuf::from("../ci.x64")
    );
    assert_eq!(fs::read_to_string(&link).expect("read linked file"), "bin");
}

#[test]
fn link_existing_target_requires_replace_or_overwrite() {
    let temp = TempDir::new().expect("tempdir");
    fs::write(temp.path().join("ci.x64"), "new").expect("write source");
    fs::write(temp.path().join("ci"), "old").expect("write existing");

    let mut inputs = BTreeMap::new();
    inputs.insert("src".to_string(), "ci.x64".to_string());
    inputs.insert("dest".to_string(), "ci".to_string());

    let err = run_link_step(temp.path(), &inputs).expect_err("link should fail");
    assert!(err
        .to_string()
        .contains("set `replace: true` or `overwrite: true`"));
    assert_eq!(
        fs::read_to_string(temp.path().join("ci")).expect("read existing"),
        "old"
    );

    inputs.insert("replace".to_string(), "true".to_string());
    assert_eq!(
        run_link_step(temp.path(), &inputs).expect("link should replace"),
        0
    );
    assert_eq!(
        fs::read_link(temp.path().join("ci")).expect("read replacement link"),
        PathBuf::from("ci.x64")
    );
}
