mod common;

use std::fs;
use std::process::Command;

use common::assertions::{assert_failure, assert_success, ci_command, output, stderr, stdout};
use common::repo::TestRepo;
use tempfile::TempDir;

#[test]
fn bootstrap_commands_do_not_require_a_repository() {
    let mut schema = ci_command();
    schema.args(["schema", "all"]);
    let schema = assert_success(output(schema));
    assert!(stdout(&schema).contains("\"workflow\""));

    let mut completion = ci_command();
    completion.args(["completion", "bash"]);
    let completion = assert_success(output(completion));
    assert!(stdout(&completion).contains("_ci()"));

    let man_dir = TempDir::new().expect("create man dir");
    let mut man = ci_command();
    man.arg("man").arg("--dir").arg(man_dir.path());
    assert_success(output(man));
    assert!(man_dir.path().join("ci.1").exists());
    assert!(man_dir.path().join("ci-schema.1").exists());
}

#[test]
fn list_auto_detects_default_rust_build_workflow() {
    let repo = TestRepo::new();
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );

    let mut command = repo.ci();
    command.args(["list", "--porcelain"]);
    let output = assert_success(output(command));
    let stdout = stdout(&output);

    assert!(stdout.starts_with("build\tnative\tyaml\t"));
    assert!(stdout.contains(".ci/build.yml"));
}

#[test]
fn init_detects_rust_and_writes_container_ready_workflow() {
    let repo = TestRepo::new();
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );

    let mut command = repo.ci();
    command.args(["init", "--force"]);
    assert_success(output(command));

    let build = repo.read(".ci/build.yml");
    assert!(build.contains("tech: rust"));
    assert!(build.contains("cargo fmt --check"));
    assert!(build.contains("cargo clippy --all-targets -- -D warnings"));
    assert!(build.contains("components: [cargo-fmt, cargo-clippy]"));
}

#[test]
fn run_executes_native_steps_and_skips_false_conditions() {
    let repo = TestRepo::new();
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - name: writes marker
    run: printf ok > marker.txt
  - name: skipped
    if: false
    run: printf bad > skipped.txt
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "build"]);
    assert_success(output(command));

    assert_eq!(repo.read("marker.txt"), "ok");
    assert!(!repo.exists("skipped.txt"));
}

#[test]
fn quiet_config_hides_info_but_keeps_step_effects() {
    let repo = TestRepo::new();
    repo.write(".ci/config.yml", "quiet: true\n");
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - name: quiet step
    run: printf quiet > quiet.txt
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "build"]);
    let output = assert_success(output(command));

    assert_eq!(repo.read("quiet.txt"), "quiet");
    assert!(!stdout(&output).contains("INFO"));
}

#[test]
fn explain_reports_arch_platform_and_skipped_steps() {
    let repo = TestRepo::new();
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: rust
steps:
  - name: host-only
    if: arch(x64)
    run: echo host
  - name: selected
    if: arch(arm64)
    run: echo selected
"#,
    );

    let mut command = repo.ci();
    command.args(["explain", "build", "--arch", "arm64", "--tech", "rust"]);
    let output = assert_success(output(command));
    let stdout = stdout(&output);

    assert!(stdout.contains("Precedence: CLI flags > workflow fields"));
    assert!(stdout.contains("arches: arm64"));
    assert!(stdout.contains("platform(arm64): linux/arm64"));
    assert!(stdout.contains("SKIP host-only"));
    assert!(stdout.contains("OK   selected"));
}

#[test]
fn unknown_config_key_fails_with_schema_hint() {
    let repo = TestRepo::new();
    repo.write(".ci/config.yml", "defaults:\n  contaner:\n    type: rust\n");

    let mut command = repo.ci();
    command.arg("status");
    let output = assert_failure(output(command), 2);

    assert!(stderr(&output).contains("unknown key `contaner`"));
    assert!(stderr(&output).contains("ci schema config"));
}

#[test]
fn unknown_workflow_key_fails_with_schema_hint() {
    let repo = TestRepo::new();
    repo.write(
        ".ci/build.yml",
        r#"
contaner:
  type: rust
steps:
  - run: echo ok
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "build"]);
    let output = assert_failure(output(command), 2);

    assert!(stderr(&output).contains("unknown key `contaner`"));
    assert!(stderr(&output).contains("ci schema workflow"));
}

#[test]
fn missing_workflow_returns_not_found_code() {
    let repo = TestRepo::new();

    let mut command = repo.ci();
    command.args(["run", "missing"]);
    let output = output(command);

    assert_eq!(output.status.code(), Some(127));
}

#[test]
fn export_and_link_actions_handle_from_to_aliases() {
    let repo = TestRepo::new();
    repo.write("target/release/ci", "bin");
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - use: export
    from: target/release/ci
    to: dist/ci.x64
    replace: true
  - use: link
    from: dist/ci.x64
    to: dist/ci
    replace: true
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "build"]);
    assert_success(output(command));

    assert_eq!(repo.read("dist/ci.x64"), "bin");
    assert_eq!(
        fs::read_link(repo.path().join("dist/ci")).unwrap(),
        std::path::PathBuf::from("ci.x64")
    );
}

#[test]
fn commit_action_stages_generated_pattern_before_committing() {
    let repo = TestRepo::new();
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - run: |
      mkdir -p generated
      printf one > generated/one.txt
      printf two > generated/two.log
      printf skip > skip.txt
  - use: commit
    patterns: generated/*.txt
    message: "ci: commit generated text"
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "build"]);
    assert_success(output(command));

    let tree = assert_success(
        Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["ls-tree", "-r", "--name-only", "HEAD"])
            .output()
            .expect("list HEAD tree"),
    );
    let tree = stdout(&tree);

    assert!(tree.contains("generated/one.txt"));
    assert!(!tree.contains("generated/two.log"));
    assert!(!tree.contains("skip.txt"));
}

#[test]
fn workflow_needs_run_dependencies_before_selected_workflow() {
    let repo = TestRepo::new();
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - run: |
      printf build >> order.txt
      printf built > built.txt
"#,
    );
    repo.write(
        ".ci/release.yml",
        r#"
on: [manual]
needs: build
steps:
  - run: |
      test -f built.txt
      printf release >> order.txt
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "release"]);
    assert_success(output(command));

    assert_eq!(repo.read("order.txt"), "buildrelease");
}

#[test]
fn status_reports_generated_workflows_and_architecture_diagnostics() {
    let repo = TestRepo::new();
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write(".ci/config.yml", "arch: [x64, arm64]\n");

    let mut command = repo.ci();
    command.arg("status");
    let output = assert_success(output(command));
    let stdout = stdout(&output);

    assert!(stdout.contains("Arch:       x64,arm64"));
    assert!(stdout.contains("found 1 workflow(s)"));
    assert!(stdout.contains("container arch"));
}

#[test]
fn dry_run_reports_selected_workflow_without_running_steps() {
    let repo = TestRepo::new();
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - run: printf no > dry-run-created.txt
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "--dry-run", "build"]);
    let output = assert_success(output(command));

    assert!(stdout(&output).contains("would run build"));
    assert!(!repo.exists("dry-run-created.txt"));
}

#[test]
fn continue_on_error_allows_workflow_to_recover() {
    let repo = TestRepo::new();
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - name: tolerated failure
    run: exit 7
    continue-on-error: true
  - name: recovery
    if: failure
    run: printf recovered > recovered.txt
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "build"]);
    assert_success(output(command));

    assert_eq!(repo.read("recovered.txt"), "recovered");
}

#[test]
fn install_and_uninstall_manage_hooks_and_runner_binary() {
    let repo = TestRepo::new();

    let mut install = repo.ci();
    install.args(["install", "--mode", "copy", "--hooks", "pre-push"]);
    assert_success(output(install));

    assert!(repo.path().join(".git/hooks/pre-push").exists());
    assert!(
        repo.path().join(".git/ci/run.x64").exists()
            || repo.path().join(".git/ci/run.arm64").exists()
    );

    let mut uninstall = repo.ci();
    uninstall.args(["uninstall"]);
    assert_success(output(uninstall));

    assert!(!repo.path().join(".git/hooks/pre-push").exists());
}
