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
ci run
ci run build
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

## Workflow sources

Executable scripts:

```sh
.ci/build.sh
```

YAML-ish workflow files:

```yaml
name: build
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
- `if: exists(cargo)`: true when a bare command exists on `PATH`.
- `if: exists(path:Cargo.toml)`: true when a repo-relative or absolute file/directory path exists.
- `if: exists(env:HOME)`: true when a workflow/step env var is set, or when the host environment provides it.
- `if: missing(cargo)`: inverse existence check.

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

Native `.ci/*.yml` steps can also use built-in `uses:` values:

- `checkout`: restore tracked files to `HEAD`, with optional `with.submodules: true|recursive`
- `submodules`: force `git submodule update --init --recursive`
- `cache`: restore and save cache paths using `with.key` and `with.path`
- `upload-artifact`: store artifacts using `with.name` and `with.path`
- `download-artifact`: restore artifacts using `with.name` and optional `with.path`
- `cleanup`: remove untracked files by default; `with.ignored: true` maps to `git clean -fdx`, `with.ignored: only` maps to `git clean -fdX`, or remove repo-relative files/directories listed in `with.path` or `with.paths`

```yaml
steps:
  - uses: checkout
  - uses: cleanup
  - uses: cleanup
    with:
      ignored: only
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

Supported defaults include shell, silent output, fail-fast, container runtime, git mode/image, recursive checkout, default branch allowlist, artifact store, and actions cache.

Example:

```yaml
defaults:
  silent: true
```

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

Reusable workflows, hosted-runner-only permissions/secrets/OIDC flows, and other non-local features fail explicitly.

## Install modes

### Link mode

```sh
ci install --mode link
```

Creates `.git/ci/run` as a symlink to the currently running `ci` binary.

### Copy mode

```sh
ci install --mode copy
```

Copies the currently running `ci` binary into `.git/ci/run`.

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
