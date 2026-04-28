mod common;

use tempfile::TempDir;

use common::assertions::{assert_failure, assert_success, output, stderr};
use common::fake_podman::{
    make_fake_docker, make_fake_flatpak_spawn, make_fake_podman, make_fake_shell,
    path_with_fake_bin, path_with_fake_bins,
};
use common::repo::TestRepo;

fn default_platform() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "linux/amd64".to_string(),
        "aarch64" => "linux/arm64".to_string(),
        arch => format!("linux/{arch}"),
    }
}

fn platform_slug(platform: &str) -> String {
    platform
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[test]
fn native_container_run_uses_platform_env_volumes_and_cache_mounts() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: rust
container:
  image: localhost/fake-rust
  arch: arm64
  env:
    FROM_CONTAINER: yes
  volumes:
    - /tmp:/tmp/ci-extra
steps:
  - name: container step
    run: printf "$FROM_CONTAINER" > container.txt
  - name: host step
    container: false
    run: printf host > host.txt
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    assert_success(output(command));

    assert_eq!(repo.read("container.txt"), "yes");
    assert_eq!(repo.read("host.txt"), "host");
    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    assert!(log.contains("--platform linux/arm64"));
    assert!(log.contains("-e FROM_CONTAINER=yes"));
    assert!(log.contains("/tmp:/tmp/ci-extra"));
    assert!(log.contains("/usr/local/cargo/registry"));
    assert!(log.contains("/usr/local/cargo/git"));
}

#[test]
fn podman_action_runs_through_docker_and_strips_selinux_volume_labels() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake docker dir");
    let fake_bin = make_fake_docker(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
execution:
  shell: bash
steps:
  - name: podman compat
    use: podman
    run: |
      podman run --rm -v "$PWD:/work:Z" -w /work fake-image /bin/sh -c 'printf docker > out.txt'
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "docker",
        "build",
    ]);
    assert_success(output(command));

    assert_eq!(repo.read("out.txt"), "docker");
    let log = std::fs::read_to_string(fake.path().join("docker.log")).expect("read docker log");
    assert!(log.contains("run --rm"));
    assert!(log.contains(":/work"));
    assert!(
        !log.contains(":/work:Z"),
        "docker fallback should strip SELinux relabel options from volume mounts: {log}"
    );
}

#[test]
fn podman_action_keeps_selinux_volume_labels_for_podman() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
execution:
  shell: bash
steps:
  - name: podman native
    use: podman
    run: |
      podman run --rm -v "$PWD:/work:Z" -w /work fake-image /bin/sh -c 'printf podman > out.txt'
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    assert_success(output(command));

    assert_eq!(repo.read("out.txt"), "podman");
    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    assert!(log.contains(":/work:Z"));
}

#[test]
fn auto_runtime_uses_flatpak_host_podman_when_inside_flatpak() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let host_bin = make_fake_podman(fake.path());
    let flatpak_bin = make_fake_flatpak_spawn(fake.path(), &host_bin);
    let sh_bin = make_fake_shell(fake.path());
    let git = String::from_utf8(
        std::process::Command::new("sh")
            .arg("-c")
            .arg("command -v git")
            .output()
            .expect("find host git")
            .stdout,
    )
    .expect("git path utf8");
    let git = git.trim();
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: rust
container:
  image: localhost/fake-rust
steps:
  - name: container step
    run: printf flatpak-host > container.txt
"#,
    );

    let mut command = repo.ci();
    command
        .env("FLATPAK_ID", "dev.test.CI")
        .env("PATH", path_with_fake_bins(&[flatpak_bin, sh_bin]))
        .args([
            "--git-mode",
            "custom",
            "--git-command",
            git,
            "run",
            "--no-recursive-checkout",
            "build",
        ]);
    assert_success(output(command));

    assert_eq!(repo.read("container.txt"), "flatpak-host");
    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    assert!(log.contains("run --rm"));
}

#[test]
fn native_container_readonly_mount_can_be_global_or_step_override() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: rust
container:
  image: localhost/fake-rust
  readonly: true
steps:
  - name: readonly step
    run: "true"
  - name: writable step
    readonly: false
    run: "true"
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    assert_success(output(command));

    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    assert!(log.contains(":/work:z,ro"));
    assert!(log.contains(":/work:z -w /work"));
}

#[test]
fn container_readonly_alone_enables_native_container() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
container:
  readonly: true
steps:
  - run: "true"
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    assert_success(output(command));

    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    assert!(log.contains(":/work:z,ro"));
    assert!(log.contains("docker.io/library/debian:stable-slim"));
}

#[test]
fn native_container_builds_generated_image_for_packages_and_components() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: rust
container:
  arch: x64
  packages: [pkg-config]
  components: [cargo-fmt, cargo-clippy]
steps:
  - run: printf built > built.txt
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    assert_success(output(command));

    assert_eq!(repo.read("built.txt"), "built");
    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    assert!(log.contains("build --platform linux/amd64"));
    assert!(log.contains("localhost/ci-build-linux-amd64:latest"));

    let generated = repo
        .path()
        .join(".git/ci/containers/ci-build-linux-amd64.Containerfile");
    let generated = std::fs::read_to_string(generated).expect("read generated Containerfile");
    assert!(generated.contains("FROM docker.io/library/rust:latest"));
    assert!(generated.contains("rustup component add 'rustfmt' 'clippy'"));
    assert!(generated.contains("pkg-config"));
}

#[test]
fn native_step_can_use_own_container_image_without_workflow_container() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - name: host step
    run: printf host > host.txt
  - name: step image
    container:
      image: localhost/step-image
      env:
        STEP_ENV: image
      volumes:
        - /tmp:/tmp/step-extra
    run: printf "$STEP_ENV" > step.txt
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    assert_success(output(command));

    assert_eq!(repo.read("host.txt"), "host");
    assert_eq!(repo.read("step.txt"), "image");
    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    assert!(log.contains("localhost/step-image"));
    assert!(log.contains("-e STEP_ENV=image"));
    assert!(log.contains("/tmp:/tmp/step-extra"));
}

#[test]
fn no_container_override_runs_step_container_on_host() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - name: step image
    container: localhost/step-image
    run: printf host > host-only.txt
"#,
    );

    let mut command = repo.ci();
    command
        .env("PATH", path_with_fake_bin(&fake_bin))
        .args(["run", "--no-container", "build"]);
    assert_success(output(command));

    assert_eq!(repo.read("host-only.txt"), "host");
    assert!(!fake.path().join("podman.log").exists());
}

#[test]
fn native_step_can_build_containerfile_for_single_step() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(".ci/step.Containerfile", "FROM scratch\n");
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
steps:
  - name: file step
    container:
      file: .ci/step.Containerfile
    run: printf file > file.txt
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    assert_success(output(command));

    assert_eq!(repo.read("file.txt"), "file");
    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    let platform = default_platform();
    let platform_slug = platform_slug(&platform);
    assert!(log.contains(&format!("build --platform {platform}")));
    assert!(log.contains(".ci/step.Containerfile"));
    assert!(log.contains(&format!(
        "localhost/ci-build-step-1-file-step-{platform_slug}:latest"
    )));
}

#[test]
fn native_step_can_build_package_container_for_single_step() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: rust
steps:
  - name: package step
    container:
      image: localhost/base-rust
      packages:
        - pkg-config
      components:
        - cargo-fmt
    run: printf packages > packages.txt
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    assert_success(output(command));

    assert_eq!(repo.read("packages.txt"), "packages");
    let log = std::fs::read_to_string(fake.path().join("podman.log")).expect("read podman log");
    let platform = default_platform();
    let platform_slug = platform_slug(&platform);
    assert!(log.contains(&format!("build --platform {platform}")));
    assert!(log.contains(&format!(
        "localhost/ci-build-step-1-package-step-{platform_slug}:latest"
    )));

    let generated = repo.path().join(format!(
        ".git/ci/containers/ci-build-step-1-package-step-{platform_slug}.Containerfile"
    ));
    let generated = std::fs::read_to_string(generated).expect("read generated Containerfile");
    assert!(generated.contains("FROM localhost/base-rust"));
    assert!(generated.contains("rustup component add 'rustfmt'"));
    assert!(generated.contains("pkg-config"));
}

#[test]
fn step_container_components_are_rejected_for_non_rust_stacks() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: node
steps:
  - name: node component
    container:
      image: localhost/base-node
      components: [cargo-fmt]
    run: npm run build
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    let output = assert_failure(output(command), 2);

    assert!(
        stderr(&output).contains("step container.components is only supported for Rust containers")
    );
}

#[test]
fn container_components_are_rejected_for_non_rust_stacks() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: node
container:
  components: [cargo-fmt]
steps:
  - run: npm run build
"#,
    );

    let mut command = repo.ci();
    command.env("PATH", path_with_fake_bin(&fake_bin)).args([
        "run",
        "--container-runtime",
        "podman",
        "build",
    ]);
    let output = assert_failure(output(command), 2);

    assert!(stderr(&output).contains("container.components is only supported for Rust containers"));
}

#[test]
fn no_container_override_runs_configured_container_workflow_on_host() {
    let repo = TestRepo::new();
    let fake = TempDir::new().expect("fake podman dir");
    let fake_bin = make_fake_podman(fake.path());
    repo.write(
        ".ci/build.yml",
        r#"
on: [manual]
tech: rust
container:
  image: localhost/fake-rust
steps:
  - run: printf host > host-only.txt
"#,
    );

    let mut command = repo.ci();
    command
        .env("PATH", path_with_fake_bin(&fake_bin))
        .args(["run", "--no-container", "build"]);
    assert_success(output(command));

    assert_eq!(repo.read("host-only.txt"), "host");
    assert!(!fake.path().join("podman.log").exists());
}
