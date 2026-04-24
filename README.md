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
ci clean --mode move --dest ./ci-artifacts
ci update
ci uninstall --restore
```

## Commands

`ci` keeps the core commands from the original MVP and adds:

- `status`: validate repo/config/hooks/runtimes/cache/store state
- `explain`: show why an event or workflow matched
- `clean`: export or keep recorded artifacts from managed run manifests

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
  - name: Format
    run: cargo fmt --check
  - name: Test
    run: cargo test --all
  - name: Build
    run: cargo build --release
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

Supported defaults include shell, fail-fast, container runtime, git mode/image, recursive checkout, default branch allowlist, artifact store, and actions cache.

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

Creates `.git/ci/ci` as a symlink to the currently running `ci` binary.

### Copy mode

```sh
ci install --mode copy
```

Copies the currently running `ci` binary into `.git/ci/ci`.

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
../ci/ci hook <hook-name> "$@"
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
