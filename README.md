# ci

`ci` is a tiny Git-native CI runner.

It installs itself into a local or bare Git repository, behaves like Git hooks, and runs workflows from `.ci`.

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
ci install --mode link --hooks pre-commit,pre-push
ci update
ci uninstall --restore
```

## Workflow types

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

`ci` will try `podman` first, then `docker`.

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

