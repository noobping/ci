
# Design

## Notes

help me with this idea.

make a rust programm called ci. it is a cli tool that can link (to the installed location of itself) or copy itself into a bare git repo or local git repo. that way it can run the workflows in the .ci folder. so, this ci tool is a ci pipeline. it is compettable with all git hooks. if it talks like a duck, it's a duck. make it a propper linux/unix tool with clear flags. so that it is user frendly. also, how do i update and remove the ci tool from a git repo?

a workflow can be a executable (like a script) or a yml file or a containerfile/dockerfile. it can be in the .ci dir or in a subdir. like project/.ci/workflowname/Containerfile

add the help, verbose, silent flags. try to build the rust ci app with the ci app.

is there anything else you can add or make it more clear?

---------

make a rust programm called ci. it is a cli tool that can link (to the installed location of itself) or copy itself into a bare git repo or local git repo. that way it can run the workflows in the .ci folder. so, this ci tool is a ci pipeline. it is compettable with all git hooks. if it talks like a duck, it's a duck. make it a propper linux/unix tool with clear flags. so that it is user frendly. also, how do i update and remove the ci tool from a git repo?

a workflow can be a executable (like a script) or a yml file or a containerfile/dockerfile. it can be in the .ci dir or in a subdir. like project/.ci/workflowname/Containerfile

if git is not installed thwn use a container with git. like as alias for git.

you can configure this in the config or with flags. git mode host, auto and alias. alias uses a container with git. by default from dockerhub.

by default, work only on main, master, develop and development branches. this can be set in workflows, config and flags

recursive checkout (for subrepos) by default

add support for github / gitea actions

support artifacts. so that the project can be cleaned. the artifects are kept or moved/renamed. you may call this the clean option with keep and move

use podman multi arch support

rename the `doctor` command to `status`

## Idea

The idea: **a tiny self-installing CI runner that behaves like Git hooks when placed inside a repo**.

> `ci` can be installed once, then linked or copied into a repo’s Git hook system.
> When Git calls `pre-commit`, `pre-push`, `post-receive`, etc., the same `ci` binary detects which hook name invoked it and runs matching workflows from `.ci`.

That gives you the “if it talks like a duck, it’s a duck” behavior.

## Concept

Program name:

```bash
ci
```

Purpose:

```text
A small Unix-style CI runner for Git repositories.

It installs itself into local or bare Git repos, runs workflows from .ci,
and is compatible with Git hooks by behaving like the hook that invoked it.
```

Example project layout:

```text
project/
├── .git/
├── .ci/
│   ├── build.sh
│   ├── test.yml
│   ├── pre-commit.yml
│   └── docker-build/
│       └── Containerfile
├── src/
└── Cargo.toml
```

For a bare repo:

```text
repo.git/
├── hooks/
├── ci/
│   └── ci
└── .ci/
    ├── pre-receive.yml
    └── deploy.sh
```

For local repos, workflows usually live in:

```text
project/.ci
```

For bare repos, I would support both:

```text
repo.git/.ci
```

and optionally a checked-out temporary worktree when running server-side hooks.

---

## Core CLI design

```bash
ci [GLOBAL FLAGS] <COMMAND>
```

Global flags:

```bash
-h, --help          Show help
-V, --version       Show version
-v, --verbose       Increase verbosity, can be repeated
-q, --quiet         Less output
    --silent        No output except fatal errors
    --repo <PATH>   Git repository path, defaults to current directory
    --ci-dir <DIR>  Workflow directory, defaults to .ci
    --color <WHEN>  auto, always, never
```

Suggested commands:

```bash
ci run [WORKFLOW]
ci list
ci install
ci uninstall
ci update
ci hook <HOOK>
ci doctor
ci self
```

---

## Important commands

### Run workflows manually

```bash
ci run
ci run build
ci run test
ci run pre-commit
ci run docker-build
```

Meaning:

```text
Run all workflows, or one named workflow, from .ci.
```

Useful flags:

```bash
ci run build --verbose
ci run test --silent
ci run --event pre-push
ci run --all
ci run --dry-run
```

---

### List workflows

```bash
ci list
```

Example output:

```text
build.sh                  executable
test.yml                  yaml
pre-commit.yml            yaml, hook: pre-commit
docker-build/Containerfile container
```

---

### Install into a repo

```bash
ci install
```

Default behavior:

```text
Install ci into the current Git repo using symlinks.
```

More explicit:

```bash
ci install --repo .
ci install --repo /path/to/project
ci install --repo /srv/git/project.git --bare
```

Installation modes:

```bash
ci install --mode link
ci install --mode copy
```

I would make `link` the default.

#### Link mode

```bash
ci install --mode link
```

Creates:

```text
.git/ci/run -> /usr/local/bin/ci
.git/hooks/pre-commit -> ../ci/run
.git/hooks/pre-push -> ../ci/run
.git/hooks/post-merge -> ../ci/run
...
```

For bare repos:

```text
repo.git/ci/run -> /usr/local/bin/ci
repo.git/hooks/pre-receive -> ../ci/run
repo.git/hooks/post-receive -> ../ci/run
...
```

#### Copy mode

```bash
ci install --mode copy
```

Creates:

```text
.git/ci/run
.git/hooks/pre-commit -> ../ci/run
.git/hooks/pre-push -> ../ci/run
```

In copy mode, the repo has its own private copy of the binary.

That is nice for reproducibility, but updates need to be explicit.

---

## Git hook compatibility

This is the most important feature.

Git expects hook files like:

```text
.git/hooks/pre-commit
.git/hooks/pre-push
.git/hooks/post-merge
```

Each hook must be executable.

Your `ci` tool should support two ways of being called:

```bash
ci hook pre-commit
```

and also:

```bash
.git/hooks/pre-commit
```

When called as `pre-commit`, the binary should inspect `argv[0]`.

Pseudo-logic:

```rust
let invoked_as = basename(argv[0]);

if invoked_as is a known git hook {
    run_hook(invoked_as, remaining_args);
} else {
    parse_normal_cli_args();
}
```

So this works:

```bash
.git/hooks/pre-commit
```

and behaves as:

```bash
ci hook pre-commit
```

That is the duck part. Beautifully Unix-y.

---

## Supported Git hooks

You can support all standard Git hooks.

Client-side:

```text
applypatch-msg
pre-applypatch
post-applypatch
pre-commit
pre-merge-commit
prepare-commit-msg
commit-msg
post-commit
pre-rebase
post-checkout
post-merge
pre-push
pre-auto-gc
post-rewrite
sendemail-validate
fsmonitor-watchman
p4-changelist
p4-prepare-changelist
p4-post-changelist
p4-pre-submit
post-index-change
```

Server-side:

```text
pre-receive
update
proc-receive
post-receive
post-update
reference-transaction
push-to-checkout
pre-auto-gc
```

Install flags:

```bash
ci install --hooks all
ci install --hooks client
ci install --hooks server
ci install --hooks pre-commit,pre-push
```

For a local repo, default to client hooks.

For a bare repo, default to server hooks.

---

## Workflow discovery

The `.ci` directory should support:

```text
.ci/build.sh
.ci/test.yml
.ci/lint
.ci/pre-commit.yml
.ci/workflow-name/Containerfile
.ci/workflow-name/Dockerfile
.ci/workflow-name/workflow.yml
```

Recommended discovery rules:

| File type        | Behavior                                                     |
| ---------------- | ------------------------------------------------------------ |
| Executable file  | Run directly                                                 |
| `.yml` / `.yaml` | Parse as workflow file                                       |
| `Containerfile`  | Build and run container workflow                             |
| `Dockerfile`     | Same as Containerfile                                        |
| Directory        | Look for `workflow.yml`, executable files, or container file |

---

## Executable workflow example

```bash
# .ci/build.sh
#!/usr/bin/env bash
set -euo pipefail

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo build --release
```

Then:

```bash
chmod +x .ci/build.sh
ci run build
```

---

## YAML workflow example

```yaml
# .ci/test.yml
name: test

on:
  - pre-commit
  - pre-push
  - manual

steps:
  - name: Format
    run: cargo fmt --check

  - name: Lint
    run: cargo clippy --all-targets -- -D warnings

  - name: Test
    run: cargo test --all
```

Then:

```bash
ci run test
```

Or automatically:

```bash
git commit
```

if installed into `pre-commit`.

---

## Container workflow example

```text
.ci/rust/Containerfile
```

```dockerfile
FROM rust:latest

WORKDIR /work
COPY . .

RUN cargo test --all
```

Run with:

```bash
ci run rust
```

The tool can detect:

```bash
podman
docker
```

Prefer `podman` on Linux if available, then fallback to `docker`.

Possible flags:

```bash
ci run rust --container-runtime podman
ci run rust --container-runtime docker
ci run rust --no-container-cache
```

---

## Hook-to-workflow mapping

When Git calls:

```bash
pre-commit
```

`ci` should run workflows matching one of these:

```text
.ci/pre-commit
.ci/pre-commit.sh
.ci/pre-commit.yml
.ci/* where YAML has on: [pre-commit]
```

For example:

```yaml
name: rust-checks

on:
  - pre-commit

steps:
  - run: cargo fmt --check
  - run: cargo test
```

Manual-only workflows:

```yaml
on:
  - manual
```

Run with:

```bash
ci run rust-checks
```

---

## Updating the tool in a repo

This depends on install mode.

### If installed with link mode

```bash
ci install --mode link
```

The repo points to the installed system binary.

So updating is usually just:

```bash
cargo install --path .
```

or:

```bash
install -Dm755 target/release/ci ~/.local/bin/ci
```

The repo uses the new version automatically.

Still, provide:

```bash
ci update
```

Behavior:

```text
Refresh hook links and verify they still point to the current ci binary.
```

Useful:

```bash
ci update --repo .
ci update --repo /srv/git/project.git --bare
ci update --source ~/.local/bin/ci
```

### If installed with copy mode

```bash
ci install --mode copy
```

The repo has its own copy:

```text
.git/ci/run
```

Update with:

```bash
ci update
```

Behavior:

```text
Copy the currently running ci binary into the repo again.
```

Or explicitly:

```bash
ci update --source target/release/ci
```

For all repos later, you could eventually support:

```bash
ci update --all-known-repos
```

but I would not put that in the first version.

---

## Removing from a repo

Command:

```bash
ci uninstall
```

or alias:

```bash
ci remove
```

Behavior:

```text
Remove ci-managed hook files and .git/ci/run.
Do not delete user-owned hooks unless they were generated by ci.
```

Important: every installed hook should contain a marker.

For wrapper scripts:

```bash
#!/usr/bin/env sh
# managed-by: ci
exec "$(dirname "$0")/../ci/run" hook pre-commit "$@"
```

Then uninstall can safely remove only hooks containing:

```text
managed-by: ci
```

Commands:

```bash
ci uninstall
ci uninstall --repo .
ci uninstall --repo /srv/git/project.git --bare
ci uninstall --hooks pre-commit,pre-push
ci uninstall --keep-binary
ci uninstall --dry-run
```

If there was a previous hook, do this during install:

```text
.git/hooks/pre-commit          generated by ci
.git/hooks/pre-commit.ci-backup previous user hook
```

Then uninstall can restore it:

```bash
ci uninstall --restore
```

That is very friendly behavior.

---

## “Build the Rust ci app with the ci app”

Nice bootstrap goal.

You can dogfood it like this:

```text
.ci/build.yml
```

```yaml
name: build

on:
  - manual
  - pre-push

steps:
  - name: Format
    run: cargo fmt --check

  - name: Lint
    run: cargo clippy --all-targets -- -D warnings

  - name: Test
    run: cargo test --all

  - name: Build
    run: cargo build --release
```

Then:

```bash
cargo build --release
./target/release/ci run build
./target/release/ci install --mode link --hooks pre-push
```

After that:

```bash
git push
```

runs the `ci` workflow through the Git hook.

That is a great self-hosting story.

---

## Suggested help output

```text
ci - small Git-native CI runner

Usage:
  ci [OPTIONS] <COMMAND>

Commands:
  run        Run workflows from .ci
  list       List discovered workflows
  install    Install ci into a Git repository
  uninstall  Remove ci from a Git repository
  update     Update ci installation in a Git repository
  hook       Run workflows for a Git hook
  doctor     Check repository and ci installation
  self       Show information about this ci binary
  help       Show help for a command

Options:
  -h, --help             Show help
  -V, --version          Show version
  -v, --verbose          Increase verbosity
  -q, --quiet            Reduce output
      --silent           Suppress all non-error output
      --repo <PATH>      Repository path
      --ci-dir <DIR>     Workflow directory [default: .ci]
      --color <WHEN>     auto, always, never [default: auto]
```

---

## Suggested `install` help

```text
Usage:
  ci install [OPTIONS]

Options:
      --repo <PATH>        Git repository path [default: .]
      --mode <MODE>        install mode: link or copy [default: link]
      --hooks <HOOKS>      all, client, server, or comma-separated list
      --bare              Treat repo as a bare Git repository
      --force             Replace existing ci-managed hooks
      --backup-existing   Backup existing hooks before replacing
      --dry-run           Show what would be installed
  -h, --help              Show help
```

---

## Suggested `run` help

```text
Usage:
  ci run [OPTIONS] [WORKFLOW]

Arguments:
  [WORKFLOW]              Workflow name to run

Options:
      --event <EVENT>     Event or hook name, e.g. pre-commit
      --all               Run all workflows
      --dry-run           Show what would run
      --fail-fast         Stop after first failure
      --keep-going        Continue after failures
      --container-runtime <RUNTIME>
                           podman, docker, or auto
  -h, --help              Show help
```

---

## Rust project structure

```text
ci/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── git.rs
│   ├── install.rs
│   ├── hooks.rs
│   ├── workflow.rs
│   ├── runner.rs
│   ├── container.rs
│   ├── output.rs
│   └── error.rs
└── .ci/
    └── build.yml
```

Good crates for this:

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "1"
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
walkdir = "2"
tracing = "0.1"
tracing-subscriber = "0.3"
```

For a proper Unix tool, keep it boring and predictable. Boring is good here. Boring is how tools become dependable little gremlins.

---

## Important implementation details

### Find the current binary

Use:

```rust
std::env::current_exe()
```

That gives the path to the running `ci` binary.

For install:

```text
link mode: symlink repo/.git/ci/run -> current_exe
copy mode: copy current_exe -> repo/.git/ci/run
```

### Find Git directory

Use Git itself instead of guessing:

```bash
git -C <repo> rev-parse --git-dir
git -C <repo> rev-parse --is-bare-repository
git -C <repo> rev-parse --show-toplevel
```

This avoids a lot of weird edge cases with worktrees, submodules, and bare repos.

### Do not clobber user hooks

Never blindly overwrite:

```text
.git/hooks/pre-commit
```

unless:

```bash
--force
```

or the file contains:

```text
managed-by: ci
```

This one matters. People get very salty when tools eat their hooks. Correctly so.

---

## Exit codes

Use clear exit codes:

```text
0   success
1   workflow failed
2   invalid usage
3   internal error
126 workflow found but not executable
127 workflow not found
130 interrupted
```

---

## Extra features worth adding

### `ci doctor`

Very useful.

```bash
ci doctor
```

Checks:

```text
- Is this a Git repo?
- Is .ci present?
- Which workflows were found?
- Is ci installed into hooks?
- Are hook files executable?
- Are symlinks valid?
- Is podman/docker available?
- Are YAML workflows valid?
```

Example:

```text
Repository: /home/nick/project
Git dir:    /home/nick/project/.git
CI dir:     /home/nick/project/.ci

OK  found 3 workflows
OK  pre-commit hook installed
OK  pre-push hook installed
WARN docker-build requires podman or docker
```

### `ci init`

Creates a starter `.ci` folder:

```bash
ci init
```

Maybe:

```text
.ci/build.yml
```

with Rust defaults if `Cargo.toml` exists.

### `ci explain`

Shows why a workflow ran:

```bash
ci explain pre-commit
```

Output:

```text
Hook: pre-commit

Matched workflows:
- .ci/pre-commit.yml because filename matches hook
- .ci/build.yml because on includes pre-commit
```

This is great for debugging.

### Locking

Prevent two CI runs from fighting:

```bash
ci run --lock
```

Uses:

```text
.git/ci/lock
```

### Environment variables

Set these for workflows:

```text
CI=true
CI_TOOL=ci
CI_EVENT=pre-commit
CI_REPO=/path/to/repo
CI_GIT_DIR=/path/to/repo/.git
CI_WORKFLOW=build
CI_WORKFLOW_DIR=/path/to/repo/.ci
```

For Git hook args:

```text
CI_HOOK=pre-push
CI_HOOK_ARGS=...
```

### Config file

Optional later:

```text
.ci/config.yml
```

Example:

```yaml
default:
  fail_fast: true
  shell: /usr/bin/env bash
  container_runtime: auto

hooks:
  pre-commit:
    workflows:
      - fmt
      - test
```

---

## A clean MVP

I would build the first version like this:

1. `ci run`
2. discover executable files and YAML files in `.ci`
3. `ci list`
4. `ci install --mode link|copy`
5. support `pre-commit`, `pre-push`, `pre-receive`, `post-receive`
6. `ci uninstall`
7. `ci update`
8. `ci doctor`

Then add container support.

That avoids the classic trap of making a CI system so flexible it becomes a haunted YAML castle.

---

## One possible command flow

Create project workflow:

```bash
mkdir -p .ci
cat > .ci/build.yml <<'EOF'
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
EOF
```

Build `ci`:

```bash
cargo build --release
```

Run itself:

```bash
./target/release/ci run build
```

Install into repo:

```bash
./target/release/ci install --mode link --hooks pre-push
```

Update later:

```bash
ci update
```

Remove later:

```bash
ci uninstall --restore
```

---

## Clear positioning

I would describe the project like this:

```text
ci is a Git-native CI runner.

It does not need a server.
It does not need a daemon.
It does not need a web UI.

It runs workflows from .ci locally or from Git hooks.
Install it into a repository with symlinks or by copying the binary.
If Git can run a hook, Git can run ci.
```

Tiny, sharp, very Unix.
