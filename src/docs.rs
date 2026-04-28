use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use clap::CommandFactory;
use clap_complete::generate;
use clap_mangen::Man;

use crate::cli::{Cli, CompletionArgs, CompletionShell, ManArgs};
use crate::error::Result;

pub fn cmd_completion(args: &CompletionArgs) -> Result<i32> {
    let mut command = Cli::command();
    let name = command.get_name().to_string();

    match &args.output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(path)?;
            generate(shell(args.shell), &mut command, name, &mut file);
        }
        None => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            generate(shell(args.shell), &mut command, name, &mut handle);
        }
    }

    Ok(0)
}

pub fn cmd_man(args: &ManArgs) -> Result<i32> {
    let command = Cli::command();

    match &args.dir {
        Some(dir) => {
            fs::create_dir_all(dir)?;
            write_man_tree(command, "ci", dir)?;
        }
        None => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            render_man(command, "ci", &mut handle)?;
        }
    }

    Ok(0)
}

fn shell(shell: CompletionShell) -> clap_complete::Shell {
    match shell {
        CompletionShell::Bash => clap_complete::Shell::Bash,
    }
}

fn write_man_tree(command: clap::Command, page_name: &str, dir: &Path) -> Result<()> {
    let subcommands = command.get_subcommands().cloned().collect::<Vec<_>>();
    let path = dir.join(format!("{page_name}.1"));
    let mut file = File::create(path)?;
    render_man(command, page_name, &mut file)?;

    for subcommand in subcommands {
        let sub_name = format!("{page_name}-{}", subcommand.get_name());
        write_man_tree(subcommand, &sub_name, dir)?;
    }

    Ok(())
}

fn render_man(command: clap::Command, page_name: &str, output: &mut dyn Write) -> Result<()> {
    Man::new(command).title(page_name).render(&mut *output)?;
    if let Some(extra) = man_extra(page_name) {
        output.write_all(extra.as_bytes())?;
    }
    Ok(())
}

fn man_extra(page_name: &str) -> Option<&'static str> {
    match page_name {
        "ci" => Some(
            r#"
.SH OVERVIEW
ci discovers workflows from .ci, .github/workflows, and .gitea/workflows.
Native workflows may be YAML files, executable files, or Containerfile/Dockerfile workflows.
If no workflow exists, ci can auto-detect a basic build workflow for common stacks.
.SH FILES
.TP
.B /etc/ci.yml
System settings and defaults.
.TP
.B ~/.config/ci/config.yml
User settings and defaults.
.TP
.B .ci/config.yml
Project settings and defaults.
.TP
.B .ci/*.yml
Native YAML workflows.
.TP
.B .ci/**/workflow.yml
Metadata for directory workflows.
.TP
.B .ci/**/Containerfile
Container build workflow.
.TP
.B .github/workflows/*.yml
GitHub Actions style workflows.
.TP
.B .gitea/workflows/*.yml
Gitea Actions style workflows.
.PP
By default, .github and .gitea workflows are discovered only for bare repositories. Set other_workflows: true or false in config to override this while keeping native .ci workflows enabled.
.SH EXAMPLES
.EX
ci list
ci build
ci build -- --dry-run
ci run -e pre-push build
ci run --arch x64,arm64 --tech rust build
ci explain build --arch x64 --tech rust
ci schema workflow
ci install -m link -H pre-commit,pre-push
ci man -d ~/.local/share/man/man1
.EE
.SH PRECEDENCE
CLI flags override workflow fields, workflow fields override workflow defaults, workflow defaults override project config, project config overrides user config, user config overrides system config, and config overrides auto-detection. Values under policy or locked are applied after normal config and CLI flags; system policy is strongest, then user policy, then project policy.
--config replaces normal config discovery but keeps system and user policy/locked sections.
.SH OUTPUT
--verbose or -v shows command traces. Repeat it as -vv and -vvv for extra runner detail.
--quiet or -q hides normal output, warnings, and workflow script output, while still showing critical errors.
Quiet mode passes --quiet to supported runner-owned commands, and falls back to --silent for commands that use that spelling.
.P
Output levels:
.IP "Quiet"
Enabled by -q, --quiet, or quiet: true. Hides normal output, warnings, verbose logs, and workflow script output. Critical ci errors are still shown.
.IP "Normal"
The default. Shows normal info, warnings, critical errors, and workflow script output.
.IP "Verbose 1"
Enabled by -v or --verbose. Adds command traces and runner verbose messages; asks supported runner-owned commands to be verbose.
.IP "Verbose 2"
Enabled by -vv. Adds runner detail such as Git execution mode, step shell/workdir, container image/platform, and package container build paths.
.IP "Verbose 3+"
Enabled by -vvv. Adds deeper detail such as Git working directory, environment variable counts, and generated container base/tag info. Higher levels currently behave like -vvv.
.SH GIT
git_mode accepts auto, host, flatpak, custom, or alias. auto detects Flatpak and uses flatpak-spawn --host git when available, then falls back to host git or the configured Git container image. custom requires git_command; git_command may be a command string or YAML list. container_runtime auto prefers podman, then flatpak-spawn --host podman inside Flatpak, then docker.
.SH BUILD ARGUMENTS
Unknown top-level commands are treated as workflow names, so ci build is equivalent to ci run build.
Arguments after the build workflow name are forwarded to the detected build step. Use -- before build-command flags when a flag name overlaps with ci, for example ci build -- --dry-run.
.SH CONDITIONS
exists(value) checks for a repo-relative path or command. Parentheses are optional, so exists value, has value, and is value are accepted too. missing(value) is the inverse, and not(value) is an alias. Word forms such as is exists(value), not exists value, and not missing value are also accepted.
.SH NATIVE WORKFLOW EXAMPLE
.EX
name: build
on:
  - manual
  - pre-push
tech: rust
container:
  arch: [x64, arm64]
  components: [cargo-fmt, cargo-clippy]
  env:
    RUST_BACKTRACE: "1"
steps:
  - run: cargo fmt --check
    readonly: true
  - run: cargo test --all
  - run: cargo build --release
  - use: export
    if: arch(host) and exists(from)
    container: false
    from: target/release/ci
    to: ~/.local/bin/ci.${{ env.CI_ARCH }}
    replace: true
.EE
.SH STEP CONTAINERS
.EX
steps:
  - name: Node lint
    container: docker.io/library/node:22-bookworm-slim
    run: npm test
  - name: Tool check
    container:
      file: .ci/tools.Containerfile
      image: localhost/my-project-tools
      env:
        TOOL_MODE: strict
      volumes:
        - /tmp:/tmp/ci-tools
      packages:
        - shellcheck
    run: tool check
.EE
Step-level packages and Rust components build a generated image for that step, using the step image as the base image. Set container: false on a step to run it on the host. ci run --no-container disables workflow and step containers for native YAML workflows.
.SH PODMAN ACTION EXAMPLE
.EX
steps:
  - name: Run Butane
    use: podman
    shell: bash
    run: |
      podman run --rm -v "$PWD:/work:Z" -w /work quay.io/coreos/butane:release --help
.EE
The podman action runs an inline Bash script with a podman function backed by the selected container runtime. With Docker, Podman SELinux relabel volume options such as :Z and :z are ignored.
.SH SETTINGS EXAMPLE
.EX
quiet: true
tech: rust
arch: [x64, arm64]
container:
  packages: [htop]
  components: [cargo-fmt, cargo-clippy]
  volumes: ["~/.cache/my-project:/cache"]
branches:
  allow: [main, develop]
.EE
.SH CONTAINERFILE EXAMPLE
.EX
# .ci/image/Containerfile
FROM docker.io/library/rust:latest
WORKDIR /work
COPY . .
RUN cargo test --all
RUN cargo build --release
.EE
.SH SYSTEMD USER SERVICE EXAMPLE
.EX
[Service]
Type=oneshot
WorkingDirectory=%h/Projects/my-project
ExecStart=%h/.local/bin/ci --quiet run build
.EE
"#,
        ),
        "ci-run" => Some(
            r#"
.SH EXAMPLES
.EX
ci run
ci run build
ci build
ci run build --no-default-features
ci build --features sqlite
ci build -- --dry-run
ci build --no-dry-run -- --dry-run
ci run -a
ci run -e pre-push build
ci run --arch x64,arm64 build
ci run --tech node build
ci run --container build
ci run --no-container build
.EE
.SH NOTES
Unknown top-level commands are treated as workflow names, so ci build is equivalent to ci run build.
Arguments after the build workflow name are forwarded to the detected build step. Native YAML build workflows first look for a step named build, then for a recognizable build command such as cargo build, and append the arguments to that step.
Known ci options keep their ci meaning before --. Put build-command flags after -- when a flag name overlaps with ci.
The space-joined forwarded argument string is available to scripts as CI_WORKFLOW_ARGS.
Use --container to force native workflows into containers, and --no-container to run them on the host. In Flatpak, container_runtime auto can use host podman through flatpak-spawn --host.
Native containers use stack-aware dependency cache mounts under .git/ci/container-cache.
A native run step can set container to an image string or to a map with image, file, packages, components, platform, env, volumes, workdir, and readonly.
"#,
        ),
        "ci-list" => Some(
            r#"
.SH EXAMPLES
.EX
ci list
ci list --porcelain
ci list | cut -f1
.EE
.SH OUTPUT
Porcelain output is tab-separated: name, provider, kind, and path.
"#,
        ),
        "ci-install" => Some(
            r#"
.SH EXAMPLES
.EX
ci install -m link -H pre-commit,pre-push
ci install -m copy -H pre-push
ci install -B
.EE
.SH NOTES
Link mode creates architecture-specific runners such as .git/ci/run.x64.
Link mode always links to the current ci executable and removes other managed run.<arch> files.
Copy mode copies the current ci binary into the repository. Hooks are direct symlinks with one runner and small selector scripts with multiple runners.
The install_mode config default accepts link or copy and is used when --mode/-m is omitted. policy.install_mode or locked.install_mode overrides --mode/-m.
"#,
        ),
        "ci-uninstall" => Some(
            r#"
.SH EXAMPLES
.EX
ci uninstall
ci uninstall -r
ci uninstall -k
.EE
.SH NOTES
Only hooks containing the managed-by: ci marker or pointing at managed .git/ci/run targets are removed automatically.
"#,
        ),
        "ci-update" => Some(
            r#"
.SH EXAMPLES
.EX
ci update
ci update --all
ci update --all ~/Projects
ci update ~/Projects/myproject
ci update -r ~/Projects
ci update --recursive ~/Projects
ci update -s ./target/release/ci
.EE
.SH NOTES
For link installs this refreshes links. For copy installs this copies the current or selected binary again.
Use ci update PATH to update another repository. Use --all or -a to update Git repositories directly in the current directory or path argument, without descending further. Use -r or --recursive to search recursively.
"#,
        ),
        "ci-other" => Some(
            r#"
.SH EXAMPLES
.EX
ci other
.EE
.SH OUTPUT
Prints the current ci binary, the host-architecture runner installed under .git/ci, both content hashes, and a status of same, update-needed, or missing.
"#,
        ),
        "ci-hook" => Some(
            r#"
.SH EXAMPLES
.EX
ci hook pre-commit
ci hook pre-push origin git@example.com:repo.git
.EE
.SH NOTES
Installed Git hooks call ci hook <hook-name> "$@".
"#,
        ),
        "ci-status" => Some(
            r#"
.SH EXAMPLES
.EX
ci status
ci doctor
.EE
.SH NOTES
Reports repository, config, hook, container runtime, Git, Node, and artifact state.
"#,
        ),
        "ci-explain" => Some(
            r#"
.SH EXAMPLES
.EX
ci explain build
ci explain pre-push
.EE
.SH NOTES
Use explain when a workflow did not run and you need to see event, branch, arch, container, and step condition reasons.
"#,
        ),
        "ci-schema" => Some(
            r#"
.SH EXAMPLES
.EX
ci schema
ci schema config
ci schema workflow
.EE
.SH NOTES
Prints JSON Schema for editor integration and external validation tooling.
"#,
        ),
        "ci-clean" => Some(
            r#"
.SH EXAMPLES
.EX
ci clean
ci clean build
ci clean -m move -d ./ci-artifacts
ci clean -r 123 -n
.EE
.SH NOTES
Exports or keeps recorded artifacts from .git/ci artifacts and run manifests.
"#,
        ),
        "ci-init" => Some(
            r#"
.SH EXAMPLES
.EX
ci init
ci init --force
.EE
.SH NOTES
Creates .ci/build.yml. Existing files are kept unless --force is used.
"#,
        ),
        "ci-completion" => Some(
            r#"
.SH EXAMPLES
.EX
ci completion bash
ci completion bash --output ~/.local/share/bash-completion/completions/ci
.EE
"#,
        ),
        "ci-man" => Some(
            r#"
.SH EXAMPLES
.EX
ci man
ci man -d ./target/man
ci man -d ~/.local/share/man/man1
.EE
.SH NOTES
When --dir is set, ci writes ci.1 and one page per subcommand.
"#,
        ),
        "ci-self" => Some(
            r#"
.SH EXAMPLES
.EX
ci self
.EE
"#,
        ),
        _ => None,
    }
}
