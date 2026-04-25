# ci

`ci` is a small Git-native CI runner.

It installs itself into a local or bare Git repository, behaves like Git hooks, runs workflows from `.ci`, and can also execute GitHub/Gitea workflow files locally.

No daemon. No server. No web UI. If Git can run a hook, Git can run `ci`.

## Build

```sh
cargo build --release
```

## Basic usage

```sh
ci list
ci list --porcelain
ci run
ci run build
ci build
ci run --arch x64,arm64 build
ci run --event pre-push
ci install --mode link --hooks pre-commit,pre-push
ci status
ci explain pre-push
ci completion bash
ci man --dir ./target/man
ci clean --mode move --dest ./ci-artifacts
ci update
ci uninstall --restore
```

## Commands

`ci` keeps the core commands from the original MVP and adds:

- `status`: validate repo/config/hooks/runtimes/cache/store state
- `explain`: show why an event or workflow matched
- `clean`: export or keep recorded artifacts from managed run manifests
- `completion`: generate shell completion scripts
- `man`: generate `man1` pages from the current CLI

If the first command does not match a built-in command, `ci` treats it as a workflow name. For example, `ci build` is equivalent to `ci run build`.

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

If a repository has no workflows, `ci` auto-detects a basic one. A Rust project with `Cargo.toml` gets a generated `build` workflow that runs `cargo build`; when host `cargo` is not available, that generated workflow uses the Rust container automatically.

Executable scripts:

```sh
.ci/build.sh
```

YAML-ish workflow files:

```yaml
name: build
defaults:
  container:
    type: rust
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
  - name: Test
    run: cargo test --all
  - name: Build
    run: cargo build --release
```

Native `.ci/*.yml` steps also support first-class conditions:

- `if: success` or `if: success()`: run when the current step path is still successful. This is the default when `if` is omitted.
- `if: failure` or `if: failure()`: run after the previous executed step failed.
- `if: always` or `if: always()`: run regardless of the previous step result.
- `if: arch(x64)`: true when the selected execution architecture matches; aliases such as `amd64` and `linux/amd64` are normalized, and comma-separated values are accepted.
- `if: exists(cargo)`: true when a repo-relative path exists, or when a bare command exists on `PATH`.
- `if: exists(path:Cargo.toml)`: true when a repo-relative or absolute file/directory path exists.
- `if: exists(file:Cargo.toml)` / `if: exists(dir:src)`: true only for files or directories.
- `if: exists(cmd:cargo)`: true when an executable command exists; `command:`, `exe:`, and `executable:` are aliases.
- `if: exists(env:USE_DEBUG)`: true when a workflow/step env var is set, or when the host environment provides it.
- `if: missing(cargo)`: inverse existence check. The same optional target prefixes work with `missing(...)`.
- When a workflow/container default is set, native `run:` steps use that container by default. Use `container: false` on a step that intentionally targets the host, such as installing files under `~`.

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
- `export`: copy `source`/`src` paths to `destination`/`dest`; multiple sources use the destination as a directory, while a single source can use an exact file path; set `replace: true` or `overwrite: true` to replace an existing target
- `commit`: stage paths and create a commit with `message`/`msg`
- `sync`: pull and push the current branch, or use `mirror: true` with `source`/`src` and `destination`/`dest` remotes
- `clean`: run `git clean -fd` by default; `ignored: true` maps to `git clean -fdx`, `ignored: only` maps to `git clean -fdX`, `purge: true` runs `git fetch --all --prune`, `cargo: true` runs `cargo clean`, `path` or `paths` removes repo-relative targets, and native `.ci/*.yml` steps may extend the cleanup with an inline `run:` block

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
  - use: sync
    strategy: rebase
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

Container workflows:

```text
.ci/rust/Containerfile
.ci/rust/Dockerfile
```

GitHub/Gitea workflow files:

```text
.github/workflows/self-host.yml
.gitea/workflows/self-host.yml
```

`ci` prefers `podman`, then falls back to `docker`. Git commands can use the host binary or fall back to `docker.io/alpine/git:latest` in `auto`/`alias` mode.

## Config

Optional config lives in:

```text
.ci/config.yml
```

Supported defaults include shell, silent output, fail-fast, architecture, container settings, container runtime, git mode/image, recursive checkout, default branch allowlist, artifact store, and actions cache.

Example:

```yaml
silent: true
container:
  type: rust
  arch:
    - x64
    - arm64
  components:
    - cargo-fmt
    - cargo-clippy
  packages:
    - htop
```

In `.ci/config.yml`, default fields can be written directly at the top level; wrapping them in `defaults:` is still accepted. In workflow files, `defaults:` can set workflow defaults such as `container`, `execution`, `branches`, `artifacts`, and `env`; direct workflow fields override those defaults.

`--arch` accepts comma-separated values and can be repeated, so `--arch x64,arm64` and `--arch x64 --arch arm64` are equivalent. `arch` accepts either one value or a YAML list and is also used as the default `container.arch` when the container arch list is omitted. Architecture is an execution setting. Native YAML workflows can run inside a generated container with config-level `container`, workflow `defaults.container`, or workflow-level `container`; workflow-level settings override the defaults. Use `-c`/`--container` to force a generated container for native workflows, or `-C`/`--no-container` to ignore configured native containers and run native steps on the host. `container.type: rust` uses the official Rust image, while omitted/`auto` detects Rust projects and otherwise uses a general Debian image. Rust containers support `components`, installed with `rustup component add`; `cargo-fmt` maps to `rustfmt` and `cargo-clippy` maps to `clippy`. When a container workflow, native container workflow, or action does not set `container.platform`, `ci` maps the selected arch to a podman/docker platform such as `linux/amd64` or `linux/arm64`.

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

## Install modes

### Link mode

```sh
ci install --mode link
```

Creates an arch-specific symlink such as `.git/ci/run.x64` to the currently running `ci` binary. Managed hooks choose `.git/ci/run.x64`, `.git/ci/run.arm64`, or another matching runner from `uname -m`, with `.git/ci/run` kept as a legacy fallback.

### Copy mode

```sh
ci install --mode copy
```

Copies the currently running `ci` binary into an arch-specific path such as `.git/ci/run.x64`.

## Update

```sh
ci update
```

For link mode, this refreshes the symlink. For copy mode, this copies the current binary again.

## Status and explain

```sh
ci status
ci explain build
ci explain pre-push
```

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
ci man --dir ./target/man
```

Install them locally:

```sh
mkdir -p ~/.local/share/man/man1
ci man --dir ~/.local/share/man/man1
```

## Remove

```sh
ci uninstall
```

Only removes hooks that contain the `managed-by: ci` marker.

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

or be invoked directly as a Git hook. Installed hooks are small shell wrappers that call:

```sh
../ci/run hook <hook-name> "$@"
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
ci clean --mode move --dest ./ci-artifacts
```

Build outputs can also be copied during a workflow:

```yaml
steps:
  - use: export
    src: target/release/ci
    if: exists(src)
    dest: dist/ci
    replace: true
```
