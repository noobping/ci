mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use common::assertions::{assert_failure, assert_success, ci_command, output, stderr, stdout};
use common::fake_podman::path_with_fake_bin;
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

fn write_native_and_github_workflows(repo: &TestRepo) {
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - run: echo native
"#,
    );
    repo.write(
        ".github/workflows/release.yml",
        r#"
name: Release
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: cargo build --release
"#,
    );
}

#[test]
fn non_bare_repos_disable_other_workflows_by_default() {
    let repo = TestRepo::new();
    write_native_and_github_workflows(&repo);

    let mut command = repo.ci();
    command.args(["list", "--porcelain"]);
    let output = assert_success(output(command));
    let stdout = stdout(&output);

    assert!(stdout.contains("build\tnative\tyaml\t"));
    assert!(!stdout.contains("Release\tgithub-actions"));
}

#[test]
fn bare_repos_enable_other_workflows_by_default() {
    let repo = TestRepo::new_bare();
    write_native_and_github_workflows(&repo);

    let mut command = repo.ci();
    command.args(["list", "--porcelain"]);
    let output = assert_success(output(command));
    let stdout = stdout(&output);

    assert!(stdout.contains("build\tnative\tyaml\t"));
    assert!(stdout.contains("Release\tgithub-actions"));
}

#[test]
fn config_can_override_other_workflow_discovery_default() {
    let repo = TestRepo::new();
    repo.write(".ci/config.yml", "other_workflows: true\n");
    write_native_and_github_workflows(&repo);

    let mut command = repo.ci();
    command.args(["list", "--porcelain"]);
    let output = assert_success(output(command));

    assert!(stdout(&output).contains("Release\tgithub-actions"));
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
fn quiet_config_hides_all_output_but_keeps_step_effects() {
    let repo = TestRepo::new();
    repo.write(".ci/config.yml", "quiet: true\n");
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - name: quiet step
    run: |
      printf quiet > quiet.txt
      printf stdout
      printf stderr >&2
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "build"]);
    let output = assert_success(output(command));

    assert_eq!(repo.read("quiet.txt"), "quiet");
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).is_empty());
}

#[test]
fn quiet_cli_shows_critical_errors_but_hides_parse_errors() {
    let repo = TestRepo::new();
    repo.write(".ci/config.yml", "defaults:\n  contaner:\n    type: rust\n");

    let mut critical = repo.ci();
    critical.args(["-q", "status"]);
    let critical = assert_failure(output(critical), 2);
    assert!(stdout(&critical).is_empty());
    assert!(stderr(&critical).contains("unknown key `contaner`"));

    let mut parse_error = repo.ci();
    parse_error.args(["-q", "--definitely-not-a-ci-option"]);
    let parse_error = assert_failure(output(parse_error), 2);
    assert!(stdout(&parse_error).is_empty());
    assert!(stderr(&parse_error).is_empty());
}

#[test]
fn run_build_forwards_args_to_detected_build_step() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake cargo dir");
    let bin_dir = fake.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create fake bin dir");
    let cargo = bin_dir.join("cargo");
    fs::write(
        &cargo,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CARGO_ARGS_OUT\"\n",
    )
    .expect("write fake cargo");
    let mut permissions = fs::metadata(&cargo).expect("cargo metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&cargo, permissions).expect("chmod fake cargo");
    let args_out = fake.path().join("cargo.args");

    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
needs: check
steps:
  - name: test
    run: printf test > test.txt
  - name: Build
    run: cargo build --release
"#,
    );
    repo.write(
        ".ci/check.yml",
        r#"
on: [manual]
steps:
  - name: check
    run: printf '%s' "$CI_WORKFLOW_ARGS" > check.args
"#,
    );

    let mut command = repo.ci();
    command
        .env("PATH", path_with_fake_bin(&bin_dir))
        .env("CARGO_ARGS_OUT", &args_out)
        .args([
            "run",
            "build",
            "--no-default-features",
            "--features",
            "sqlite",
        ]);
    assert_success(output(command));

    assert_eq!(
        fs::read_to_string(args_out).expect("read cargo args"),
        "build\n--release\n--no-default-features\n--features\nsqlite\n"
    );
    assert_eq!(repo.read("test.txt"), "test");
    assert_eq!(repo.read("check.args"), "");
}

#[test]
fn build_separator_forwards_known_ci_flag_to_build_step() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake cargo dir");
    let bin_dir = fake.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("create fake bin dir");
    let cargo = bin_dir.join("cargo");
    fs::write(
        &cargo,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CARGO_ARGS_OUT\"\n",
    )
    .expect("write fake cargo");
    let mut permissions = fs::metadata(&cargo).expect("cargo metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&cargo, permissions).expect("chmod fake cargo");
    let args_out = fake.path().join("cargo.args");

    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - name: build
    run: cargo build --release
  - run: printf done > ran.txt
"#,
    );

    let mut command = repo.ci();
    command
        .env("PATH", path_with_fake_bin(&bin_dir))
        .env("CARGO_ARGS_OUT", &args_out)
        .args(["build", "--no-dry-run", "--", "--dry-run"]);
    assert_success(output(command));

    assert_eq!(
        fs::read_to_string(args_out).expect("read cargo args"),
        "build\n--release\n--dry-run\n"
    );
    assert_eq!(repo.read("ran.txt"), "done");
}

#[test]
fn run_rejects_forwarded_args_for_non_build_workflow() {
    let repo = TestRepo::new();
    repo.write(
        ".ci/check.yml",
        r#"
on: [manual]
steps:
  - run: true
"#,
    );

    let mut command = repo.ci();
    command.args(["run", "check", "--some-arg"]);
    assert_failure(output(command), 2);
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
fn status_can_use_custom_git_command_from_config() {
    let repo = TestRepo::new();
    let temp = TempDir::new().expect("create temp dir");
    let log = temp.path().join("git.log");
    let wrapper = temp.path().join("git-wrapper");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec git \"$@\"\n",
            log.display()
        ),
    )
    .expect("write git wrapper");
    let mut permissions = fs::metadata(&wrapper)
        .expect("wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper, permissions).expect("make wrapper executable");
    repo.write(
        ".ci/config.yml",
        &format!(
            "git-mode: custom\ngit-command:\n  - {}\n",
            wrapper.display()
        ),
    );

    let mut command = repo.ci();
    command.arg("status");
    let output = assert_success(output(command));
    let stdout = stdout(&output);

    assert!(stdout.contains("git mode: custom"));
    assert!(stdout.contains(&format!("git command: {}", wrapper.display())));
    let log = fs::read_to_string(log).expect("read git wrapper log");
    assert!(log.contains("rev-parse --abbrev-ref HEAD"));
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
    repo.write(
        ".ci/pre-push.yml",
        r#"
on: [pre-push]
steps:
  - run: printf single > hooked.txt
"#,
    );

    let mut install = repo.ci();
    install.args(["install", "--mode", "copy", "--hooks", "pre-push"]);
    assert_success(output(install));

    let host_arch = host_runner_suffix();
    let hook_path = repo.path().join(".git/hooks/pre-push");
    assert!(hook_path.exists());
    assert_eq!(
        fs::read_link(&hook_path).expect("read hook symlink"),
        std::path::PathBuf::from(format!("../ci/run.{host_arch}"))
    );
    assert!(!repo.path().join(".git/ci/hook").exists());
    assert!(repo
        .path()
        .join(format!(".git/ci/run.{host_arch}"))
        .exists());
    let mut hook_command = Command::new(&hook_path);
    hook_command.current_dir(repo.path());
    assert_success(output(hook_command));
    assert_eq!(repo.read("hooked.txt"), "single");

    let mut uninstall = repo.ci();
    uninstall.args(["uninstall"]);
    assert_success(output(uninstall));

    assert!(!repo.path().join(".git/hooks/pre-push").exists());
}

#[test]
fn other_reports_installed_runner_hash_status() {
    let repo = TestRepo::new();
    let host_arch = host_runner_suffix();

    let mut missing = repo.ci();
    missing.args(["other"]);
    let missing = assert_success(output(missing));
    let missing_stdout = stdout(&missing);
    assert!(missing_stdout.contains("status: missing"));
    assert!(missing_stdout.contains(&format!(".git/ci/run.{host_arch}")));

    let mut install = repo.ci();
    install.args(["install", "--mode", "copy", "--hooks", "pre-push"]);
    assert_success(output(install));

    let mut same = repo.ci();
    same.args(["other"]);
    let same = assert_success(output(same));
    let same_stdout = stdout(&same);
    assert!(same_stdout.contains("installed: copy"));
    assert!(same_stdout.contains("current hash: "));
    assert!(same_stdout.contains("installed hash: "));
    assert!(same_stdout.contains("status: same"));

    fs::write(
        repo.path().join(format!(".git/ci/run.{host_arch}")),
        "older runner",
    )
    .expect("overwrite installed runner");
    let mut update_needed = repo.ci();
    update_needed.args(["other"]);
    let update_needed = assert_success(output(update_needed));
    assert!(stdout(&update_needed).contains("status: update-needed"));
}

#[test]
fn update_all_updates_installed_runners_under_directory() {
    let parent = TempDir::new().expect("create parent dir");
    let repo_one = parent.path().join("one");
    let repo_two = parent.path().join("two");
    let repo_skip = parent.path().join("skip");
    let repo_nested = parent.path().join("group").join("nested");
    init_git_repo(&repo_one);
    init_git_repo(&repo_two);
    init_git_repo(&repo_skip);
    init_git_repo(&repo_nested);

    for repo in [&repo_one, &repo_two, &repo_nested] {
        let mut install = ci_command();
        install.args([
            "--repo",
            repo.to_str().expect("repo path"),
            "install",
            "--mode",
            "copy",
            "--hooks",
            "pre-push",
        ]);
        assert_success(output(install));
        fs::write(
            repo.join(format!(".git/ci/run.{}", host_runner_suffix())),
            "old runner",
        )
        .expect("overwrite installed runner");
    }

    let mut update = ci_command();
    update.args([
        "update",
        "--all",
        parent.path().to_str().expect("parent path"),
    ]);
    let update = assert_success(output(update));
    let update_stdout = stdout(&update);
    assert!(update_stdout.contains("Updated 2 ci installation(s); skipped 1; failed 0."));

    let current = fs::read(env!("CARGO_BIN_EXE_ci")).expect("read current test binary");
    for repo in [&repo_one, &repo_two] {
        let installed = fs::read(repo.join(format!(".git/ci/run.{}", host_runner_suffix())))
            .expect("read updated runner");
        assert_eq!(installed, current);
    }
    let nested = fs::read(repo_nested.join(format!(".git/ci/run.{}", host_runner_suffix())))
        .expect("read nested runner");
    assert_eq!(nested, b"old runner");
}

#[test]
fn update_recursive_updates_installed_runners_under_directory() {
    let parent = TempDir::new().expect("create parent dir");
    let repo_one = parent.path().join("one");
    let repo_nested = parent.path().join("group").join("nested");
    init_git_repo(&repo_one);
    init_git_repo(&repo_nested);

    for repo in [&repo_one, &repo_nested] {
        let mut install = ci_command();
        install.args([
            "--repo",
            repo.to_str().expect("repo path"),
            "install",
            "--mode",
            "copy",
            "--hooks",
            "pre-push",
        ]);
        assert_success(output(install));
        fs::write(
            repo.join(format!(".git/ci/run.{}", host_runner_suffix())),
            "old runner",
        )
        .expect("overwrite installed runner");
    }

    let mut update = ci_command();
    update.args([
        "update",
        "--recursive",
        parent.path().to_str().expect("parent path"),
    ]);
    let update = assert_success(output(update));
    let update_stdout = stdout(&update);
    assert!(update_stdout.contains("Updated 2 ci installation(s); skipped 0; failed 0."));

    let current = fs::read(env!("CARGO_BIN_EXE_ci")).expect("read current test binary");
    for repo in [&repo_one, &repo_nested] {
        let installed = fs::read(repo.join(format!(".git/ci/run.{}", host_runner_suffix())))
            .expect("read updated runner");
        assert_eq!(installed, current);
    }
}

#[test]
fn update_path_updates_single_installed_repo() {
    let parent = TempDir::new().expect("create parent dir");
    let repo = parent.path().join("repo");
    init_git_repo(&repo);

    let mut install = ci_command();
    install.args([
        "--repo",
        repo.to_str().expect("repo path"),
        "install",
        "--mode",
        "copy",
        "--hooks",
        "pre-push",
    ]);
    assert_success(output(install));
    fs::write(
        repo.join(format!(".git/ci/run.{}", host_runner_suffix())),
        "old runner",
    )
    .expect("overwrite installed runner");

    let mut update = ci_command();
    update.args(["update", repo.to_str().expect("repo path")]);
    let update = assert_success(output(update));
    assert!(stdout(&update).contains("Updated 1 ci installation(s); skipped 0; failed 0."));

    let current = fs::read(env!("CARGO_BIN_EXE_ci")).expect("read current test binary");
    let installed =
        fs::read(repo.join(format!(".git/ci/run.{}", host_runner_suffix()))).expect("read runner");
    assert_eq!(installed, current);
}

#[test]
fn install_uses_configured_default_mode_when_mode_flag_is_omitted() {
    let repo = TestRepo::new();
    let host_arch = host_runner_suffix();
    repo.write(".ci/config.yml", "install-mode: copy\n");

    let mut install = repo.ci();
    install.args(["install", "--hooks", "pre-push"]);
    assert_success(output(install));

    assert!(
        !fs::symlink_metadata(repo.path().join(format!(".git/ci/run.{host_arch}")))
            .expect("runner metadata")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(repo.path().join(".git/hooks/pre-push")).expect("read hook symlink"),
        std::path::PathBuf::from(format!("../ci/run.{host_arch}"))
    );
}

#[test]
fn locked_install_mode_overrides_mode_flag() {
    let repo = TestRepo::new();
    let host_arch = host_runner_suffix();
    repo.write(
        ".ci/config.yml",
        r#"
install-mode: copy
locked:
  install-mode: link
"#,
    );

    let mut install = repo.ci();
    install.args(["install", "--mode", "copy", "--hooks", "pre-push"]);
    assert_success(output(install));

    assert!(
        fs::symlink_metadata(repo.path().join(format!(".git/ci/run.{host_arch}")))
            .expect("runner metadata")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(repo.path().join(".git/hooks/pre-push")).expect("read hook symlink"),
        std::path::PathBuf::from(format!("../ci/run.{host_arch}"))
    );
}

#[test]
fn copy_install_can_use_per_arch_sources() {
    let repo = TestRepo::new();
    repo.write("dist/ci-linux-x64", "x64");
    repo.write("dist/ci-linux-arm64", "arm64");
    let source = repo.path().join("dist/ci-linux-{arch}");

    let mut install = repo.ci();
    install.args([
        "--arch",
        "x64,arm64",
        "install",
        "--mode",
        "copy",
        "--source",
        source.to_str().expect("source path"),
        "--hooks",
        "pre-push",
    ]);
    assert_success(output(install));

    assert_eq!(repo.read(".git/ci/run.x64"), "x64");
    assert_eq!(repo.read(".git/ci/run.arm64"), "arm64");
    let hook_path = repo.path().join(".git/hooks/pre-push");
    assert!(!fs::symlink_metadata(&hook_path)
        .expect("hook metadata")
        .file_type()
        .is_symlink());
    let hook = repo.read(".git/hooks/pre-push");
    assert!(hook.contains("ci_hook=$(basename \"$0\")"));
    assert!(hook.contains("run.$ci_arch"));
}

#[test]
fn copy_install_without_source_uses_host_arch_only() {
    let repo = TestRepo::new();
    let host_arch = host_runner_suffix();

    let mut install = repo.ci();
    install.args([
        "--arch",
        "x64,arm64",
        "install",
        "--mode",
        "copy",
        "--hooks",
        "pre-push",
    ]);
    assert_success(output(install));

    assert!(repo.exists(&format!(".git/ci/run.{host_arch}")));
    if host_arch != "x64" {
        assert!(!repo.exists(".git/ci/run.x64"));
    }
    if host_arch != "arm64" {
        assert!(!repo.exists(".git/ci/run.arm64"));
    }
    assert_eq!(
        fs::read_link(repo.path().join(".git/hooks/pre-push")).expect("read hook symlink"),
        std::path::PathBuf::from(format!("../ci/run.{host_arch}"))
    );
}

#[test]
fn copy_install_adds_host_arch_to_existing_runner_and_switches_hooks_to_script() {
    let repo = TestRepo::new();
    let host_arch = host_runner_suffix();
    let other_arch = if host_arch == "x64" { "arm64" } else { "x64" };
    repo.write(&format!(".git/ci/run.{other_arch}"), "other");
    repo.write(
        ".ci/pre-push.yml",
        r#"
on: [pre-push]
steps:
  - run: printf script > hooked.txt
"#,
    );

    let mut install = repo.ci();
    install.args(["install", "--mode", "copy", "--hooks", "pre-push"]);
    assert_success(output(install));

    assert!(repo.exists(&format!(".git/ci/run.{host_arch}")));
    assert!(repo.exists(&format!(".git/ci/run.{other_arch}")));
    let hook_path = repo.path().join(".git/hooks/pre-push");
    assert!(!fs::symlink_metadata(&hook_path)
        .expect("hook metadata")
        .file_type()
        .is_symlink());
    let hook = repo.read(".git/hooks/pre-push");
    assert!(hook.contains("run.$ci_arch"));
    let mut hook_command = Command::new(&hook_path);
    hook_command.current_dir(repo.path());
    assert_success(output(hook_command));
    assert_eq!(repo.read("hooked.txt"), "script");
}

#[test]
fn link_install_uses_current_host_runner_even_with_source_template() {
    let repo = TestRepo::new();
    repo.write("dist/ci-linux-x64", "x64");
    repo.write("dist/ci-linux-arm64", "arm64");
    let source = repo.path().join("dist/ci-linux-{arch}");
    let host_arch = host_runner_suffix();
    let stale_arch = if host_arch == "x64" { "arm64" } else { "x64" };
    repo.write(&format!(".git/ci/run.{stale_arch}"), "stale");

    let mut install = repo.ci();
    install.args([
        "--arch",
        "x64,arm64",
        "install",
        "--mode",
        "link",
        "--source",
        source.to_str().expect("source path"),
        "--hooks",
        "pre-push",
    ]);
    assert_success(output(install));

    assert!(
        fs::symlink_metadata(repo.path().join(format!(".git/ci/run.{host_arch}")))
            .expect("runner metadata")
            .file_type()
            .is_symlink()
    );
    if host_arch != "x64" {
        assert!(!repo.exists(".git/ci/run.x64"));
    }
    if host_arch != "arm64" {
        assert!(!repo.exists(".git/ci/run.arm64"));
    }
    assert_eq!(
        fs::read_link(repo.path().join(".git/hooks/pre-push")).expect("read hook symlink"),
        std::path::PathBuf::from(format!("../ci/run.{host_arch}"))
    );
}

fn host_runner_suffix() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" | "amd64" => "x64",
        "aarch64" | "arm64" => "arm64",
        other => other,
    }
}

fn init_git_repo(path: &Path) {
    fs::create_dir_all(path).expect("create repo dir");
    run_setup_ok(Command::new("git").arg("init").arg(path));
    run_setup_ok(Command::new("git").arg("-C").arg(path).args([
        "config",
        "user.email",
        "ci@example.test",
    ]));
    run_setup_ok(
        Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["config", "user.name", "ci tests"]),
    );
    run_setup_ok(Command::new("git").arg("-C").arg(path).args([
        "commit",
        "--allow-empty",
        "-m",
        "initial",
    ]));
}

fn run_setup_ok(command: &mut Command) {
    let output = command.output().expect("run setup command");
    assert!(
        output.status.success(),
        "setup command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
