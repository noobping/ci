
![License](https://img.shields.io/badge/license-MIT-blue.svg)
[![Release](https://github.com/noobping/ci/actions/workflows/release.yml/badge.svg)](https://github.com/noobping/ci/actions/workflows/release.yml)

# ci

`ci` is a small Git-native CI runner and build tool.

Install it into a local or bare Git repository to run like Git hooks, or use it directly for builds. It runs workflows from `.ci`, and can also execute GitHub/Gitea workflow files locally.

No daemon. No server. No web UI.

`ci` uses what the host already has: Podman, Docker, and Git when available. If Git is missing but Podman or Docker is present, it can run Git from a (configurable) container image instead.

If Git can run a hook, Git can run `ci`.

## Build from source

```sh
cargo build --release
```

## Commands

Core commands:

- `run`: run one or more workflows.
- `list`: list discovered workflows.
- `install`: install managed Git hooks.
- `uninstall`: remove managed Git hooks.
- `update`: refresh the installed runner.
- `hook`: run as a Git hook entrypoint.
- `status`: validate repo, config, hooks, runtimes, cache, and store state.
- `explain`: show why an event or workflow matched.
- `schema`: print JSON Schema for config and workflow files.
- `clean`: export or keep recorded artifacts from run manifests.
- `completion`: generate shell completion scripts.
- `man`: generate `man1` pages.
- `init`: create `.ci/build.yml`.
- `self`: print information about the current `ci` binary.
- `other`: compare the installed repository runner with the current `ci` binary.

If the first command does not match a built-in command, `ci` treats it as a workflow name. For example, `ci build` is equivalent to `ci run build`.

Arguments after the `build` workflow name are forwarded to the detected build step:

```sh
ci run build --no-default-features
ci build --features sqlite
ci build -- --dry-run
ci build --no-dry-run -- --dry-run
```

For native YAML build workflows, `ci` looks for a step named `build`, then for a recognizable build command such as `cargo build`, and appends the arguments to that step. Known `ci` options keep their `ci` meaning before `--`, so `ci build --dry-run` previews the workflow. Put build-command flags after `--` when a flag name overlaps with `ci`. The space-joined argument string is also available to scripts as `CI_WORKFLOW_ARGS`.

Global output controls:

```sh
ci --verbose build
ci -vv build
ci -vvv build
ci --quiet build
```

`--verbose`/`-v` shows command traces; repeat it as `-vv` and `-vvv` for extra runner detail. `--quiet`/`-q` hides normal output, warnings, and workflow script output, but still shows critical errors. Verbose and quiet modes are passed to supported runner-owned commands, such as Git; quiet mode uses `--quiet` when supported and falls back to `--silent` for commands that use that spelling.

Output levels:

| Level | How to enable | Behavior |
| --- | --- | --- |
| Quiet | `-q`, `--quiet`, or `quiet: true` | Hides normal output, warnings, verbose logs, and workflow script output. Critical `ci` errors are still shown. |
| Normal | default | Shows normal info, warnings, critical errors, and workflow script output. |
| Verbose 1 | `-v`, `--verbose` | Adds command traces and runner verbose messages; asks supported runner-owned commands to be verbose. |
| Verbose 2 | `-vv` | Adds runner detail such as Git execution mode, step shell/workdir, container image/platform, and package container build paths. |
| Verbose 3+ | `-vvv` | Adds deeper detail such as Git working directory, environment variable counts, and generated container base/tag info. Higher levels currently behave like `-vvv`. |

Configuration precedence is:

```text
CLI flags > workflow fields > workflow defaults > project config > user config > system config > auto-detect
```

## Script-friendly list output

`ci list` keeps the aligned human-readable layout when writing to a terminal.

When `stdout` is redirected or piped, `ci list` automatically switches to porcelain output:

```sh
ci list | cut -f1
ci list > workflows.tsv
```

Porcelain output is tab-separated:

```text
name<TAB>provider<TAB>kind<TAB>path
```

Use `--porcelain` to force that format on a terminal, or `--no-porcelain` to keep the aligned layout even when piping or redirecting output.

## Workflow sources

If a repository has no workflows, `ci` auto-detects a basic `build` workflow for supported stacks. Rust, Node.js, Go, Python packages, Maven, Gradle, and .NET projects get a default build command; when the host tool is not available, that generated workflow uses the matching container automatically.

Choose a stack explicitly when auto-detection is not enough:

```sh
ci build --tech rust
ci build -t node
ci build --type golang
ci build --tech-stack py
```

Workflow sources:

- Native YAML: `.ci/build.yml`, `.ci/test.yaml`
- Native executable: `.ci/build.sh`
- Directory metadata: `.ci/release/workflow.yml`
- Containerfile workflow: `.ci/image/Containerfile`
- GitHub Actions: `.github/workflows/*.yml`
- Gitea Actions: `.gitea/workflows/*.yml`

Executable scripts:

```sh
.ci/build.sh
```

Native YAML workflow files:

```yaml
name: build
defaults:
  tech: rust
  container:
    arch:
      - x64
      - arm64
    components:
      - cargo-fmt
      - cargo-clippy
    packages:
      - htop
on:
  - manual
  - pre-push
steps:
  - name: Checkout
    uses: checkout
    with:
      submodules: recursive
  - name: Format
    run: cargo fmt --check
    readonly: true
  - name: Test
    run: cargo test --all
  - name: Build
    run: cargo build --release
```

Native `.ci/*.yml` steps also support first-class conditions:

- `if: success` or `if: success()`: run when the current step path is still successful. This is the default when `if` is omitted.
- `if: failure` or `if: failure()`: run after the previous executed step failed.
- `if: always` or `if: always()`: run regardless of the previous step result.
- `if: arch(x64)` or `if: arch x64`: true when the selected execution architecture matches; aliases such as `amd64` and `linux/amd64` are normalized, comma-separated values are accepted, and `arch(host)` matches only the host machine's native architecture.
- `if: exists(cargo)` or `if: exists cargo`: true when a repo-relative path exists, or when a bare command exists on `PATH`; `has(...)` and `is(...)` are aliases, and their parentheses are optional too.
- `if: exists(path:Cargo.toml)`: true when a repo-relative or absolute file/directory path exists.
- `if: exists(file:Cargo.toml)` / `if: exists(dir:src)`: true only for files or directories.
- `if: exists(cmd:cargo)`: true when an executable command exists; `command:`, `exe:`, and `executable:` are aliases.
- `if: exists(env:USE_DEBUG)`: true when a workflow/step env var is set, or when the host environment provides it.
- `if: missing(cargo)` or `if: missing cargo`: inverse existence check; `not(...)` is an alias. The same optional target prefixes work with `missing(...)`.
- `if: is exists(Cargo.toml)` / `if: not missing Cargo.toml`: word forms are accepted for readable existence checks.
- When a workflow/container default is set, native `run:` steps use that container by default. Use `container: false` on a step that intentionally targets the host, such as installing files under `~`.

Native workflows can require other workflows with `needs:`. Dependencies run before the selected workflow, even when they would not otherwise match the current event. `requires:`, `depends:`, and `dependencies:` are accepted aliases.

```yaml
name: release-bin
on: [post-receive, manual]
needs: build
steps:
  - run: install -Dm0755 "dist/ci-linux-$CI_ARCH" "public/ci-linux-$CI_ARCH"
```

This repository uses that to keep the workflows small:

- `check`: format, lint, and test
- `build`: depends on `check`, builds release binaries, and writes `dist/ci-linux-$CI_ARCH`
- `install-local`: depends on `build` and installs the host binary under `~/.local/bin`
- `aur`: depends on `build`, generates `.SRCINFO`, and commits AUR metadata when available

To add a fallback step after a failure and still let the workflow recover, mark the failing step with `continue-on-error: true`.

```yaml
steps:
  - name: Build with host cargo
    run: cargo build --release
    continue-on-error: true
  - name: Build with toolbox cargo
    if: "failure && exists(flatpak-spawn) && exists(toolbox)"
    run: flatpak-spawn --host toolbox run cargo build --release
```

Native `.ci/*.yml` steps can also use built-in `uses:` or `use:` action sources. `name:` is only the display name, so built-ins can still have custom labels.

- `checkout`: restore tracked files to `HEAD`, with optional `with.submodules: true|recursive`
- `submodules`: force `git submodule update --init --recursive`
- `cache`: restore and save cache paths using `with.key` and `with.path`
- `upload-artifact`: store artifacts using `with.name` and `with.path`
- `download-artifact`: restore artifacts using `with.name` and optional `with.path`
- `export`: copy `source`/`src`/`from` paths to `destination`/`dest`/`to`; multiple sources use the destination as a directory, while a single source can use an exact file path; set `replace: true` or `overwrite: true` to replace an existing target
- `link`: create symlinks from `source`/`src`/`from` to `destination`/`dest`/`to`; multiple sources use the destination as a directory, while a single source can use an exact link path; set `replace: true` or `overwrite: true` to replace an existing target
- `commit`: stage paths and create a commit with `message`/`msg`; without paths it stages all changes with `git add -A`; staged paths may use `path`, `paths`, `file`, `files`, `pattern`, `patterns`, `source`, `src`, or `from`; set `staged: true` to commit only already staged changes
- `sync`: pull and push the current branch, or use `mirror: true` with `source`/`src`/`from` and `destination`/`dest`/`to` remotes
- `clean`: run `git clean -fd` by default; `ignored: true` maps to `git clean -fdx`, `ignored: only` maps to `git clean -fdX`, `purge: true` runs `git fetch --all --prune`, `cargo: true` runs `cargo clean`, `path` or `paths` removes repo-relative targets, and native `.ci/*.yml` steps may extend the cleanup with an inline `run:` block
- `podman`: run an inline Bash script with a `podman` function backed by the selected container runtime; auto/runtime selection still prefers Podman, then Docker, and Docker runs strip Podman SELinux relabel mount options such as `:Z`/`:z`

`export` handles files and build outputs. `commit` and `sync` are separate repository actions.

```yaml
steps:
  - use: checkout
  - name: Clean ignored build outputs
    use: clean
  - use: clean
    ignored: only
  - use: export
    src:
      - target/release/ci
      - README.md
    dest: dist
    replace: true
  - use: commit
    message: "ci: update generated outputs"
  - name: Generate AUR metadata
    run: makepkg --printsrcinfo > .SRCINFO
  - use: commit
    paths:
      - PKGBUILD
      - .SRCINFO
    message: "aur: update package metadata"
  - use: sync
    strategy: rebase
  - name: Run Butane in a container
    use: podman
    shell: bash
    run: |
      podman run --rm -v "$PWD:/work:Z" -w /work quay.io/coreos/butane:release --help
  - use: clean
    purge: true
    cargo: true
    run: |
      rm -rf dist coverage
  - name: Use local cargo
    if: exists(cargo)
    run: cargo test
  - name: Fallback to toolbox cargo
    if: missing(cargo)
    run: toolbox run cargo test
  - name: Only if a file exists
    if: exists(path:Cargo.toml)
    run: cat Cargo.toml
  - name: Only if HOME is set
    if: exists(env:HOME)
    run: printf '%s\n' "$HOME"
```

Containerfile workflow and GitHub/Gitea examples are covered below. `ci` prefers `podman`, then `flatpak-spawn --host podman` when running inside Flatpak, then falls back to `docker`. Git commands can use the host binary, `flatpak-spawn --host git`, a custom command wrapper, or fall back to `docker.io/alpine/git:latest` in `auto`/`alias` mode.

## Config

Optional config lives in:

```text
/etc/ci.yml
/etc/ci/config.yml
~/.config/ci/config.yml
.ci/config.yml
```

The same `.yaml` filenames are also accepted. Config is loaded in order from system, user, then project config, so project config wins. `--config path/to/file.yml` uses that file for normal config while still keeping system/user `policy` and `locked` sections.

Supported defaults include shell, quiet output, fail-fast, tech stack, architecture, container settings, container runtime, git mode/command/image, default install mode, recursive checkout, default branch allowlist, artifact store, and actions cache. `quiet: true` hides normal output, warnings, and workflow script output, while still showing critical errors. By default, `.github/workflows` and `.gitea/workflows` are discovered only in bare repositories; set `other_workflows: true` or `other_workflows: false` to override that while keeping native `.ci` workflows enabled.

Example:

```yaml
quiet: true
fail_fast: true
tech: rust
arch:
  - x64
  - arm64

container:
  arch:
    - x64
    - arm64
  components:
    - cargo-fmt
    - cargo-clippy
  packages:
    - htop
  env:
    RUST_BACKTRACE: "1"
  volumes:
    - ~/.cache/my-project:/cache

git_mode: auto
git_image: docker.io/alpine/git:latest
install_mode: copy
recursive_checkout: true
other_workflows: false

locked:
  install_mode: copy

branches:
  allow:
    - main
    - develop

workflows:
  build:
    branches:
      allow:
        - main
        - develop

hooks:
  pre-push:
    branches:
      allow:
        - main
```

In `.ci/config.yml`, default fields can be written directly at the top level; wrapping them in `defaults:` is still accepted. In workflow files, `defaults:` can set workflow defaults such as `tech`, `container`, `execution`, `branches`, `artifacts`, and `env`; direct workflow fields override those defaults. Unknown YAML keys are rejected so misspelled fields fail early.

`git_mode` accepts `auto`, `host`, `flatpak`, `custom`, or `alias`. In `auto` mode, `ci` detects Flatpak and uses `flatpak-spawn --host git` when available, then falls back to host `git` or the configured Git container image. Use `git_mode: custom` with `git_command` to force a wrapper; `git-command: "flatpak-spawn --host git"` and YAML lists are both accepted. The `--git-command` flag accepts the same command string.

```yaml
git_mode: custom
git_command:
  - flatpak-spawn
  - --host
  - git
```

`install_mode` accepts `link` or `copy` and is used when `ci install` is run without `--mode`/`-m`; the CLI flag wins over normal config. Put defaults under `policy:` or `locked:` to apply them after normal config and CLI flags. For policy values, system config is strongest, then user config, then project config, so `/etc/ci.yml` can enforce a managed default such as:

```yaml
locked:
  install_mode: copy
```

`--arch` accepts comma-separated values and can be repeated, so `--arch x64,arm64` and `--arch x64 --arch arm64` are equivalent. `arch` accepts either one value or a YAML list and is also used as the default `container.arch` when the container arch list is omitted. Architecture is an execution setting. The selected execution architecture is exposed as `CI_ARCH`; the host machine architecture is exposed as `CI_HOST_ARCH`. Native YAML workflows can run inside a generated container with config-level `container`, workflow `defaults.container`, workflow-level `container`, or a selected tech stack; workflow-level settings override the defaults. `container_runtime: auto` prefers direct Podman, then host Podman through `flatpak-spawn --host` inside Flatpak, then Docker. Explicit `podman` and `docker` settings also use the matching host runtime through `flatpak-spawn --host` when needed. Use `-c`/`--container` to force a generated container for native workflows, or `-C`/`--no-container` to ignore configured native containers and run native steps on the host. `tech`, `type`, `tech-stack`, and `container.type` accept `auto`, `general`, `rust`, `node`, `go`, `python`, `maven`, `gradle`, and `dotnet`; common aliases such as `npm`, `js`, `golang`, `py`, and `.net` are accepted. Omitted/`auto` detects the stack from project files and step commands, then falls back to a general Debian image. Rust containers support `components`, installed with `rustup component add`; `cargo-fmt` maps to `rustfmt` and `cargo-clippy` maps to `clippy`. Container `env`, `volumes`, `workdir`, and `readonly` are passed to native container runs. `container.readonly: true` mounts `/work` read-only; a native step can override it with `readonly: false` or opt into it with `readonly: true`. `read-only` and `read_only` are accepted aliases. When a container workflow, native container workflow, or action does not set `container.platform`, `ci` maps the selected arch to a podman/docker platform such as `linux/amd64` or `linux/arm64`.

Print JSON Schema for editor integration or validation tooling:

```sh
ci schema config
ci schema workflow
```

Workflow-local defaults:

```yaml
name: build
defaults:
  tech: node
  container:
    packages:
      - git
  env:
    NODE_ENV: production

steps:
  - run: npm ci
  - run: npm run build --if-present
```

## Tech Stacks And Containers

`tech`, `type`, `tech-stack`, and `container.type` accept:

- `auto`
- `general`
- `rust`
- `node`
- `go`
- `python`
- `maven`
- `gradle`
- `dotnet`

Common aliases such as `npm`, `js`, `golang`, `py`, `.net`, and `csharp` are accepted.

Default images:

- Rust: `docker.io/library/rust:latest`
- Node.js: `docker.io/library/node:22-bookworm-slim`
- Go: `docker.io/library/golang:latest`
- Python: `docker.io/library/python:3`
- Maven: `docker.io/library/maven:latest`
- Gradle: `docker.io/library/gradle:latest`
- .NET: `mcr.microsoft.com/dotnet/sdk:latest`
- General: `docker.io/library/debian:stable-slim`

Force or disable containers for native workflows:

```sh
ci run --container build
ci run --no-container build
ci run -c build
ci run -C build
```

Run for multiple container architectures:

```yaml
container:
  arch:
    - x64
    - arm64
```

If `container.arch` is set and no `--arch` override is provided, native container workflows run once per listed architecture.

Mount `/work` read-only for check-only container steps:

```yaml
container:
  image: docker.io/library/rust:latest
steps:
  - run: cargo fmt --check
    readonly: true
  - run: cargo build --release
    readonly: false
```

Use `container.readonly: true` to make read-only the workflow default. Step-level `readonly` overrides it.

Native containers use stack-aware cache mounts under `.git/ci/container-cache` for common dependency caches such as Cargo, npm, Go modules, pip, Maven, Gradle, and NuGet. Build output paths such as `target/` stay in the repository mount so later `export` steps can see them.

Run a single native `run:` step in its own image:

```yaml
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
```

Step-level `packages` and Rust `components` build a generated image for that step, using the step `image` as the base image. `container: false` still keeps a step on the host. `ci run --no-container` disables workflow and step containers for native YAML workflows.

## Containerfile Examples

Use a custom image for native workflow steps:

```Dockerfile
# Containerfile.ci
FROM docker.io/library/rust:latest
RUN rustup component add rustfmt clippy
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config \
    && rm -rf /var/lib/apt/lists/*
```

```sh
podman build -t localhost/my-project-ci -f Containerfile.ci .
```

```yaml
name: build
container:
  image: localhost/my-project-ci
  arch:
    - x64
    - arm64

steps:
  - run: cargo fmt --check
  - run: cargo test --all
  - run: cargo build --release
```

Use a Containerfile as a workflow:

```Dockerfile
# .ci/image/Containerfile
FROM docker.io/library/rust:latest
WORKDIR /work
COPY . .
RUN cargo test --all
RUN cargo build --release
```

Optional metadata for that workflow:

```yaml
# .ci/image/workflow.yml
on:
  - manual
  - pre-push
container:
  arch:
    - x64
    - arm64
```

Run it:

```sh
ci run image
```

Containerfile workflows build with the repository root as context. Put checks in `RUN` instructions or in the image entrypoint.

## Actions compatibility

`ci` discovers `.github/workflows/*.yml` and `.gitea/workflows/*.yml` and runs a broad local subset including:

- `on`, `env`, `defaults.run`
- `jobs`, `needs`, `strategy.matrix`
- `if`, `working-directory`, `continue-on-error`
- `job.container`, `services`
- `steps.run`, `steps.uses`

Built-in shims exist for:

- `actions/checkout`
- `actions/cache`
- `actions/upload-artifact`
- `actions/download-artifact`

Example:

```yaml
name: self-host
on:
  - push
jobs:
  test:
    runs-on: local
    container: docker.io/library/rust:latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --all
```

## Systemd Integration

`ci` does not need a daemon, but systemd can run it on a timer.

`~/.config/systemd/user/my-project-ci.service`:

```ini
[Unit]
Description=Run ci build for my-project

[Service]
Type=oneshot
WorkingDirectory=%h/Projects/my-project
ExecStart=%h/.local/bin/ci --quiet run build
```

`~/.config/systemd/user/my-project-ci.timer`:

```ini
[Unit]
Description=Run ci build for my-project periodically

[Timer]
OnBootSec=5min
OnUnitActiveSec=1h
Persistent=true

[Install]
WantedBy=timers.target
```

Enable it:

```sh
systemctl --user daemon-reload
systemctl --user enable --now my-project-ci.timer
```

For a system service, use an absolute `WorkingDirectory` and an absolute `ExecStart` path.

## Environment

Workflow steps receive:

- `CI=true`
- `CI_TOOL=ci`
- `CI_EVENT`
- `CI_HOOK`
- `CI_ARCH`
- `CI_HOST_ARCH`
- `CI_PLATFORM`
- `CI_REPO`
- `CI_GIT_DIR`
- `CI_WORKFLOW`
- `CI_WORKFLOW_PATH`
- `CI_WORKFLOW_DIR`
- `CI_RUN_ID`
- `CI_PROVIDER`
- `CI_HOOK_ARGS`
- `CI_BRANCH` when available

GitHub/Gitea compatibility variables such as `GITHUB_ACTIONS`, `GITHUB_REF`, `GITHUB_REF_NAME`, `GITEA_REF`, and `GITEA_REF_NAME` are also populated when applicable.

Use expression syntax in YAML inputs:

```yaml
to: ~/.local/bin/ci.${{ env.CI_ARCH }}
```

Use shell syntax inside `run:`:

```yaml
run: echo "$CI_ARCH"
```

## Install modes

### Link mode

```sh
ci install -m link
```

Creates an arch-specific symlink such as `.git/ci/run.x64` to the currently running `ci` binary. Link mode is a single-current-executable install: managed hooks are direct symlinks to that runner, and reinstalling with link mode removes other managed `run.<arch>` files before rewriting hooks back to the current machine.

`--source` is for copy installs; link mode always links the repository runner to the current `ci` executable.

### Copy mode

```sh
ci install -m copy
```

Copies the currently running `ci` binary into an arch-specific path such as `.git/ci/run.x64`.

When no `--source` is set, copy mode installs only the current machine's architecture. That means you can run the same install once on an x64 machine and once on an arm64 machine to populate both `.git/ci/run.x64` and `.git/ci/run.arm64` without extra flags.

With one installed architecture, managed hooks are direct symlinks such as `.git/hooks/pre-push -> ../ci/run.x64`. When copy mode installs or detects multiple `.git/ci/run.<arch>` binaries, managed hooks become small scripts that select the matching runner from `uname -m`.

Copy installs can use per-architecture sources:

```sh
ci --arch x64,arm64 install -m copy -s 'dist/ci-linux-{arch}'
```

That installs `dist/ci-linux-x64` to `.git/ci/run.x64` and `dist/ci-linux-arm64` to `.git/ci/run.arm64`.

## Update

```sh
ci update
ci update ~/Projects/myproject
ci update -a
ci update --all ~/Projects
ci update -r ~/Projects
ci update --recursive
ci other
```

For link mode, this refreshes the runner symlink. For copy mode, this copies the current binary again. `ci update -s 'dist/ci-linux-{arch}'` uses the same per-architecture source template as install. Managed hooks are refreshed as direct symlinks when one runner is installed, or selector scripts when multiple runners are installed.

Use `ci update PATH` to update another repository directly. Use `ci update --all [PATH]` to update Git repositories directly in that directory, without descending further. Use `ci update -r [PATH]` or `ci update --recursive [PATH]` to search recursively. Repositories without an installed `.git/ci/run...` binary are skipped.

`ci other` prints the host-architecture runner installed under `.git/ci`, hashes it against the currently running `ci`, and reports `same`, `update-needed`, or `missing`.

## Status and explain

```sh
ci status
ci explain build --arch x64 --tech rust
ci explain pre-push
```

`ci explain` prints the selected architectures, container type/image/platform, matching reason, and native step conditions. `ci status` checks the configured architecture list and warns when a non-host architecture may need binfmt/qemu support.

## Completion and man pages

Generate bash completion to stdout:

```sh
ci completion bash
```

Install bash completion locally:

```sh
mkdir -p ~/.local/share/bash-completion/completions
ci completion bash --output ~/.local/share/bash-completion/completions/ci
```

Generate `man1` pages:

```sh
ci man -d ./target/man
```

Install them locally:

```sh
mkdir -p ~/.local/share/man/man1
ci man -d ~/.local/share/man/man1
```

`ci man --dir`/`-d` writes `ci.1` and one page per subcommand.

## Remove

```sh
ci uninstall
```

Only removes hooks that contain the `managed-by: ci` marker or point at a managed `.git/ci/run...` target.

Use:

```sh
ci uninstall --restore
```

to restore backed-up hooks named `hook-name.ci-backup`.

## Hook behavior

`ci` can run as:

```sh
ci hook pre-commit
```

or be invoked directly as a Git hook.

With one installed runner, installed hooks are symlinks to the runner:

```sh
.git/hooks/pre-push -> ../ci/run.x64
```

With multiple installed runners, each hook is a small managed script that selects the arch-specific runner and calls:

```sh
../ci/run.$ci_arch hook <hook-name> "$@"
```

## Artifacts

Successful runs are recorded under:

```text
.git/ci/runs/
.git/ci/artifacts/
```

Native workflows can declare:

```yaml
artifacts:
  paths:
    - target/release/ci
  mode: keep
```

Artifacts can later be exported with:

```sh
ci clean -m move -d ./ci-artifacts
```

Build outputs can also be copied during a workflow:

```yaml
steps:
  - use: export
    if: exists(from)
    from: target/release/ci
    to: dist/ci
    replace: true
```

## Troubleshooting

Inspect discovery and matching:

```sh
ci list
ci explain build
ci explain pre-push
ci status
```

Use `--verbose` for command traces, `-vv`/`-vvv` for extra runner detail, and `--quiet` for low-noise hooks or timers:

```sh
ci --verbose build
ci -vv build
ci --quiet run --event pre-push build
```
