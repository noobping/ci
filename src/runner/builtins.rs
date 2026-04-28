use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::artifacts::ArtifactSession;
use crate::conditions::{interpolate_expressions, ExpressionContext};
use crate::config::ContainerRuntime;
use crate::containers::{sh_single_quote, ContainerBackend};
use crate::error::{CiError, Result};
use crate::runner::AppContext;

use super::action_metadata::strip_action_ref;
use super::cache::{restore_cache, CacheState, PendingCache};
use super::cleanup::{cleanup_repo_paths, parse_cleanup_ignored_mode};
use super::env::{resolve_workdir, run_shell};
use super::file_actions::{run_export_step, run_link_step};
use super::inputs::{input_bool, input_value, parse_bool, parse_path_list};

pub(crate) struct BuiltinStepInvocation<'a, 'b> {
    pub(crate) workflow_name: &'a str,
    pub(crate) default_name: &'a str,
    pub(crate) uses: &'a str,
    pub(crate) with: &'a BTreeMap<String, String>,
    pub(crate) extra: Option<&'a BTreeMap<String, String>>,
    pub(crate) inline_run: Option<String>,
    pub(crate) shell: Option<String>,
    pub(crate) workdir: Option<PathBuf>,
    pub(crate) container_runtime: ContainerRuntime,
    pub(crate) expr: &'b ExpressionContext<'b>,
}

pub(crate) struct BuiltinStepState<'a> {
    pub(crate) artifacts: &'a mut ArtifactSession,
    pub(crate) cache_state: &'a mut CacheState,
}

const EXPORT_ACTION_NAMES: &[&str] = &[
    "export",
    "ci/export",
    "artifact",
    "ci/artifact",
    "artifacts",
    "ci/artifacts",
    "release",
    "ci/release",
    "releases",
    "ci/releases",
    "result",
    "ci/result",
    "results",
    "ci/results",
    "res",
    "ci/res",
    "install",
    "ci/install",
];

const LINK_ACTION_NAMES: &[&str] = &["link", "ci/link", "symlink", "ci/symlink"];
const COMMIT_ACTION_NAMES: &[&str] = &["commit", "ci/commit"];
const SYNC_ACTION_NAMES: &[&str] = &["sync", "ci/sync"];
const PODMAN_ACTION_NAMES: &[&str] = &["podman", "ci/podman", "docker", "ci/docker"];
const COMMIT_PATH_INPUT_KEYS: &[&str] = &[
    "path", "paths", "file", "files", "pattern", "patterns", "source", "sources", "src", "srcs",
    "from", "froms",
];
const REMOTE_SOURCE_INPUT_KEYS: &[&str] = &["source", "src", "from"];
const REMOTE_DESTINATION_INPUT_KEYS: &[&str] =
    &["destination", "destenation", "dest", "dst", "to", "target"];

pub(crate) fn run_builtin_step(
    ctx: &AppContext,
    invocation: &BuiltinStepInvocation<'_, '_>,
    state: &mut BuiltinStepState<'_>,
) -> Result<Option<i32>> {
    let normalized = strip_action_ref(invocation.uses).to_lowercase();
    let mut rendered_with = invocation
        .extra
        .map(|extra| interpolate_map(extra, invocation.expr))
        .unwrap_or_default();
    rendered_with.extend(interpolate_map(invocation.with, invocation.expr));

    match normalized.as_str() {
        "checkout" | "actions/checkout" => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            ctx.git.restore_tracked_files(&ctx.repo)?;
            let wants_submodules = rendered_with
                .get("submodules")
                .map(|value| value == "recursive" || parse_bool(value))
                .unwrap_or(ctx.config.defaults.recursive_checkout);
            if wants_submodules {
                ctx.git.ensure_submodules(&ctx.repo)?;
            }
            Ok(Some(0))
        }
        "submodules" | "ci/submodules" => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            ctx.git.ensure_submodules(&ctx.repo)?;
            Ok(Some(0))
        }
        "cache" | "actions/cache" => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            let key = rendered_with
                .get("key")
                .cloned()
                .unwrap_or_else(|| "default".to_string());
            let paths =
                parse_path_list(rendered_with.get("path").map(String::as_str).unwrap_or(""));
            restore_cache(ctx, &key, &paths)?;
            state.cache_state.pending.push(PendingCache { key, paths });
            Ok(Some(0))
        }
        "upload-artifact" | "actions/upload-artifact" => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            let name = rendered_with
                .get("name")
                .cloned()
                .unwrap_or_else(|| invocation.default_name.to_string());
            let paths =
                parse_path_list(rendered_with.get("path").map(String::as_str).unwrap_or(""));
            let _ = state.artifacts.upload_named_artifact(
                invocation.workflow_name,
                &name,
                &paths,
                false,
            )?;
            Ok(Some(0))
        }
        "download-artifact" | "actions/download-artifact" => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            let name = rendered_with
                .get("name")
                .cloned()
                .unwrap_or_else(|| invocation.default_name.to_string());
            let dest = resolve_workdir(
                invocation.expr.root,
                Some(Path::new(
                    rendered_with.get("path").map(String::as_str).unwrap_or("."),
                )),
            );
            let _ = state
                .artifacts
                .download_named_artifact(&name, &dest, false)?;
            Ok(Some(0))
        }
        name if EXPORT_ACTION_NAMES.contains(&name) => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            Ok(Some(run_export_step(invocation.expr.root, &rendered_with)?))
        }
        name if LINK_ACTION_NAMES.contains(&name) => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            Ok(Some(run_link_step(invocation.expr.root, &rendered_with)?))
        }
        name if COMMIT_ACTION_NAMES.contains(&name) => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            Ok(Some(run_commit_step(ctx, &rendered_with)?))
        }
        name if SYNC_ACTION_NAMES.contains(&name) => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            Ok(Some(run_sync_step(ctx, &rendered_with)?))
        }
        name if PODMAN_ACTION_NAMES.contains(&name) => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            Ok(Some(run_podman_step(ctx, invocation, &rendered_with)?))
        }
        "clean" | "ci/clean" => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            Ok(Some(run_clean_step(
                ctx,
                invocation.expr.root,
                &rendered_with,
                invocation.inline_run.as_deref(),
                invocation
                    .shell
                    .as_deref()
                    .unwrap_or(&ctx.config.defaults.shell),
                invocation
                    .workdir
                    .as_deref()
                    .unwrap_or(invocation.expr.root),
                invocation.expr,
            )?))
        }
        "cleanup" | "ci/cleanup" => {
            ctx.output
                .verbose(format!("running built-in action `{}`", invocation.uses));
            Ok(Some(run_cleanup_step(
                ctx,
                invocation.expr.root,
                rendered_with.get("path").map(String::as_str),
                rendered_with.get("paths").map(String::as_str),
                rendered_with
                    .get("missing-ok")
                    .or_else(|| rendered_with.get("missing_ok"))
                    .map(String::as_str),
                rendered_with
                    .get("ignored")
                    .or_else(|| rendered_with.get("include-ignored"))
                    .or_else(|| rendered_with.get("include_ignored"))
                    .map(String::as_str),
            )?))
        }
        _ => Ok(None),
    }
}

pub(crate) fn interpolate_map(
    values: &BTreeMap<String, String>,
    expr: &ExpressionContext<'_>,
) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), interpolate_expressions(value, expr)))
        .collect()
}

fn run_cleanup_step(
    ctx: &AppContext,
    root: &Path,
    path: Option<&str>,
    paths: Option<&str>,
    missing_ok: Option<&str>,
    include_ignored: Option<&str>,
) -> Result<i32> {
    if path.is_none() && paths.is_none() {
        let ignored = parse_cleanup_ignored_mode("cleanup", include_ignored)?;
        ctx.git.clean_untracked_files(&ctx.repo, ignored)?;
        return Ok(0);
    }

    cleanup_repo_paths(root, path, paths, missing_ok)?;
    Ok(0)
}

fn run_clean_step(
    ctx: &AppContext,
    root: &Path,
    rendered_with: &BTreeMap<String, String>,
    inline_run: Option<&str>,
    shell: &str,
    workdir: &Path,
    expr: &ExpressionContext<'_>,
) -> Result<i32> {
    if rendered_with
        .get("purge")
        .map(|value| parse_bool(value))
        .unwrap_or(false)
    {
        ctx.git.fetch_prune(&ctx.repo)?;
    }

    let ignored = parse_cleanup_ignored_mode(
        "clean",
        rendered_with
            .get("ignored")
            .or_else(|| rendered_with.get("include-ignored"))
            .or_else(|| rendered_with.get("include_ignored"))
            .map(String::as_str),
    )?;
    ctx.git.clean_untracked_files(&ctx.repo, ignored)?;

    if rendered_with
        .get("cargo")
        .map(|value| parse_bool(value))
        .unwrap_or(false)
    {
        let command = if ctx.output.is_quiet() {
            "cargo clean --quiet"
        } else {
            "cargo clean"
        };
        let status = run_shell(shell, command, workdir, expr.env)?;
        if status != 0 {
            return Ok(status);
        }
    }

    let path = rendered_with.get("path").map(String::as_str);
    let paths = rendered_with.get("paths").map(String::as_str);
    if path.is_some() || paths.is_some() {
        cleanup_repo_paths(
            root,
            path,
            paths,
            rendered_with
                .get("missing-ok")
                .or_else(|| rendered_with.get("missing_ok"))
                .map(String::as_str),
        )?;
    }

    if let Some(script) = inline_run {
        let script = interpolate_expressions(script, expr);
        let status = run_shell(shell, &script, workdir, expr.env)?;
        if status != 0 {
            return Ok(status);
        }
    }

    Ok(0)
}

fn run_podman_step(
    ctx: &AppContext,
    invocation: &BuiltinStepInvocation<'_, '_>,
    rendered_with: &BTreeMap<String, String>,
) -> Result<i32> {
    let shell = invocation
        .shell
        .as_deref()
        .unwrap_or(&ctx.config.defaults.shell);
    if !shell_looks_like_bash(shell) {
        return Err(CiError::Usage(
            "podman action requires a bash-compatible shell; set `execution.shell: bash`"
                .to_string(),
        ));
    }

    let backend = ContainerBackend::detect(invocation.container_runtime)?;
    let script = invocation
        .inline_run
        .as_deref()
        .map(|script| interpolate_expressions(script, invocation.expr))
        .or_else(|| {
            input_value(rendered_with, &["args", "arg", "command", "cmd"])
                .map(|args| format!("podman {args}"))
        })
        .ok_or_else(|| {
            CiError::Usage("podman action requires inline `run` or `with.args`".to_string())
        })?;
    let wrapped = format!("{}\n{}", podman_shell_prelude(&backend), script);
    let workdir = invocation
        .workdir
        .as_deref()
        .unwrap_or(invocation.expr.root);
    run_shell(shell, &wrapped, workdir, invocation.expr.env)
}

fn shell_looks_like_bash(shell: &str) -> bool {
    shell
        .split_whitespace()
        .next()
        .and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .map(|value| value == "bash" || value == "env")
        .unwrap_or(false)
        || shell.contains("bash")
}

fn podman_shell_prelude(backend: &ContainerBackend) -> String {
    let command = backend
        .command_tokens()
        .iter()
        .map(|part| sh_single_quote(part))
        .collect::<Vec<_>>()
        .join(" ");
    let runtime = backend.runtime_name();
    format!(
        r#"__ci_container_runtime={runtime}
__ci_container_command=({command})

__ci_strip_selinux_volume_label() {{
  local spec="$1"
  if [[ "$__ci_container_runtime" != docker || "$spec" != *:*:* ]]; then
    printf '%s' "$spec"
    return
  fi

  local prefix="${{spec%:*}}"
  local opts="${{spec##*:}}"
  local out=()
  local opt
  IFS=',' read -ra __ci_volume_opts <<< "$opts"
  for opt in "${{__ci_volume_opts[@]}}"; do
    case "$opt" in
      z|Z|label=*|relabel=*) ;;
      *) out+=("$opt") ;;
    esac
  done

  if ((${{#out[@]}} == 0)); then
    printf '%s' "$prefix"
  else
    local joined="${{out[0]}}"
    local index
    for ((index = 1; index < ${{#out[@]}}; index++)); do
      joined="${{joined}},${{out[$index]}}"
    done
    printf '%s:%s' "$prefix" "$joined"
  fi
}}

__ci_strip_docker_transport() {{
  local value="$1"
  if [[ "$__ci_container_runtime" == docker && "$value" == docker://* ]]; then
    printf '%s' "${{value#docker://}}"
  else
    printf '%s' "$value"
  fi
}}

__ci_podman_compat() {{
  local args=()
  while (($#)); do
    case "$1" in
      -v|--volume)
        args+=("$1")
        shift
        if (($#)); then
          args+=("$(__ci_strip_selinux_volume_label "$1")")
          shift
        fi
        ;;
      -v=*|--volume=*)
        args+=("${{1%%=*}}=$(__ci_strip_selinux_volume_label "${{1#*=}}")")
        shift
        ;;
      --mount)
        args+=("$1")
        shift
        if (($#)); then
          args+=("${{1//,relabel=private/}}")
          args[-1]="${{args[-1]//,relabel=shared/}}"
          shift
        fi
        ;;
      --mount=*)
        local mount_value="${{1#*=}}"
        mount_value="${{mount_value//,relabel=private/}}"
        mount_value="${{mount_value//,relabel=shared/}}"
        args+=("--mount=$mount_value")
        shift
        ;;
      --userns=keep-id)
        if [[ "$__ci_container_runtime" != docker ]]; then
          args+=("$1")
        fi
        shift
        ;;
      --userns)
        if [[ "$__ci_container_runtime" == docker && "${{2:-}}" == keep-id ]]; then
          shift 2
        else
          args+=("$1")
          shift
          if (($#)); then
            args+=("$1")
            shift
          fi
        fi
        ;;
      --tls-verify|--tls-verify=true|--tls-verify=false)
        if [[ "$__ci_container_runtime" != docker ]]; then
          args+=("$1")
        fi
        shift
        ;;
      --tls-verify=*)
        if [[ "$__ci_container_runtime" != docker ]]; then
          args+=("$1")
        fi
        shift
        ;;
      --security-opt=label=*)
        if [[ "$__ci_container_runtime" != docker ]]; then
          args+=("$1")
        fi
        shift
        ;;
      --security-opt)
        if [[ "$__ci_container_runtime" == docker && "${{2:-}}" == label=* ]]; then
          shift 2
        else
          args+=("$1")
          shift
          if (($#)); then
            args+=("$1")
            shift
          fi
        fi
        ;;
      docker://*)
        args+=("$(__ci_strip_docker_transport "$1")")
        shift
        ;;
      *)
        args+=("$1")
        shift
        ;;
    esac
  done

  command "${{__ci_container_command[@]}}" "${{args[@]}}"
}}

podman() {{
  __ci_podman_compat "$@"
}}

docker() {{
  __ci_podman_compat "$@"
}}
"#
    )
}

fn run_commit_step(ctx: &AppContext, rendered_with: &BTreeMap<String, String>) -> Result<i32> {
    let paths = input_value(rendered_with, COMMIT_PATH_INPUT_KEYS)
        .map(parse_path_list)
        .unwrap_or_default();
    let staged_only = input_bool(
        rendered_with,
        &["staged", "staged-only", "staged_only"],
        false,
    );
    let add_all = input_bool(rendered_with, &["all"], paths.is_empty() && !staged_only);

    if !staged_only {
        let mut args = vec!["add".to_string()];
        if add_all {
            args.push("-A".to_string());
        } else if !paths.is_empty() {
            args.push("--".to_string());
            args.extend(paths);
        }

        if args.len() > 1 {
            let status = git_status(ctx, &args)?;
            if status != 0 {
                return Ok(status);
            }
        }
    }

    let allow_empty = input_bool(
        rendered_with,
        &["allow-empty", "allow_empty", "empty"],
        false,
    );
    if !allow_empty {
        let status = ctx
            .git
            .status_in_dir(&ctx.repo.root, &["diff", "--cached", "--quiet"])?;
        if status == 0 {
            ctx.output.info("No changes to commit.");
            return Ok(0);
        }
    }

    let message = input_value(rendered_with, &["message", "msg", "summary"])
        .unwrap_or("ci: automated changes");
    let mut args = vec!["commit".to_string(), "-m".to_string(), message.to_string()];
    if allow_empty {
        args.push("--allow-empty".to_string());
    }
    if input_bool(rendered_with, &["signoff", "sign-off", "signed-off"], false) {
        args.push("--signoff".to_string());
    }
    if let Some(author) = input_value(rendered_with, &["author"]) {
        args.push("--author".to_string());
        args.push(author.to_string());
    }

    git_status(ctx, &args)
}

fn run_sync_step(ctx: &AppContext, rendered_with: &BTreeMap<String, String>) -> Result<i32> {
    let remote = input_value(rendered_with, &["remote"]).unwrap_or("origin");
    let source_remote = input_value(rendered_with, REMOTE_SOURCE_INPUT_KEYS).unwrap_or(remote);
    let destination_remote =
        input_value(rendered_with, REMOTE_DESTINATION_INPUT_KEYS).unwrap_or(remote);

    if input_bool(rendered_with, &["mirror"], false) {
        let fetch_status = git_status(
            ctx,
            &[
                "fetch".to_string(),
                "--prune".to_string(),
                source_remote.to_string(),
            ],
        )?;
        if fetch_status != 0 {
            return Ok(fetch_status);
        }
        return git_status(
            ctx,
            &[
                "push".to_string(),
                "--mirror".to_string(),
                destination_remote.to_string(),
            ],
        );
    }

    let branch = input_value(rendered_with, &["branch", "ref"])
        .map(ToOwned::to_owned)
        .or_else(|| ctx.repo.branch.clone())
        .or_else(|| ctx.git.current_branch(&ctx.repo).ok().flatten());

    if input_bool(rendered_with, &["prune"], false) {
        let status = git_status(
            ctx,
            &[
                "fetch".to_string(),
                "--prune".to_string(),
                source_remote.to_string(),
            ],
        )?;
        if status != 0 {
            return Ok(status);
        }
    }

    if input_bool(rendered_with, &["pull"], true) {
        let strategy = sync_pull_strategy(rendered_with);
        if strategy != "none" {
            let mut args = vec!["pull".to_string()];
            match strategy.as_str() {
                "ff-only" | "ff" => args.push("--ff-only".to_string()),
                "rebase" => args.push("--rebase".to_string()),
                "merge" => args.push("--no-rebase".to_string()),
                other => {
                    return Err(CiError::Usage(format!(
                        "sync strategy must be `ff-only`, `rebase`, `merge`, or `none`; got `{other}`"
                    )));
                }
            }
            args.push(source_remote.to_string());
            if let Some(branch) = branch.as_ref() {
                args.push(branch.clone());
            }
            let status = git_status(ctx, &args)?;
            if status != 0 {
                return Ok(status);
            }
        }
    }

    if input_bool(rendered_with, &["push"], true) {
        let mut args = vec!["push".to_string()];
        if input_bool(
            rendered_with,
            &["follow-tags", "follow_tags", "tags"],
            false,
        ) {
            args.push("--follow-tags".to_string());
        }
        args.push(destination_remote.to_string());
        args.push(
            branch
                .as_ref()
                .map(|branch| format!("HEAD:{branch}"))
                .unwrap_or_else(|| "HEAD".to_string()),
        );
        let status = git_status(ctx, &args)?;
        if status != 0 {
            return Ok(status);
        }
    }

    Ok(0)
}

fn sync_pull_strategy(values: &BTreeMap<String, String>) -> String {
    input_value(values, &["strategy", "pull-strategy", "pull_strategy"])
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| {
            if input_bool(values, &["automerge", "auto-merge", "merge"], false) {
                "merge".to_string()
            } else {
                "ff-only".to_string()
            }
        })
}

fn git_status(ctx: &AppContext, args: &[String]) -> Result<i32> {
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    ctx.git.status_in_dir(&ctx.repo.root, &args)
}
