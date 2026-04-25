use std::collections::BTreeMap;
use std::path::Path;

use crate::artifacts::ArtifactSession;
use crate::conditions::{
    evaluate_condition, evaluate_condition_with_probe, interpolate_expressions, ExpressionContext,
};
use crate::containers::{
    container_platform, ContainerBackend, ContainerCommandExistsSpec, ContainerShellSpec,
};
use crate::error::{CiError, Result};
use crate::runner::{AppContext, RunInvocation};
use crate::workflow::{NativeStep, ResolvedWorkflow};

use super::builtins::{interpolate_map, run_builtin_step, BuiltinStepInvocation, BuiltinStepState};
use super::cache::{save_pending_caches, CacheState};
use super::env::{merged_env, resolve_workdir, run_shell};
use super::native_container::{
    native_container_cache_mounts, prepare_native_container_image, NativeContainerExecution,
};

pub(crate) fn run_native_yaml_containerized(
    ctx: &AppContext,
    invocation: &RunInvocation,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
    base_env: &BTreeMap<String, String>,
    artifacts: &mut ArtifactSession,
) -> Result<i32> {
    let backend = ContainerBackend::detect(invocation.container_runtime)?;
    let platform = container_platform(resolved, &invocation.arch);
    let image = prepare_native_container_image(ctx, &backend, resolved, steps, &platform)?;
    if image.build_status != 0 {
        return Ok(image.build_status);
    }

    let container = NativeContainerExecution {
        backend: &backend,
        image: image.image,
        platform,
    };
    run_native_yaml(
        ctx,
        invocation,
        resolved,
        steps,
        base_env,
        artifacts,
        Some(&container),
    )
}

pub(crate) fn run_native_yaml(
    ctx: &AppContext,
    _invocation: &RunInvocation,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
    base_env: &BTreeMap<String, String>,
    artifacts: &mut ArtifactSession,
    container: Option<&NativeContainerExecution<'_>>,
) -> Result<i32> {
    let mut previous_failed = false;
    let mut workflow_failure = 0;
    let mut cache_state = CacheState::default();
    let native_cache_mounts = if container.is_some() {
        native_container_cache_mounts(ctx, resolved, steps)?
    } else {
        Vec::new()
    };
    let empty_matrix = BTreeMap::new();
    let empty_inputs = BTreeMap::new();
    for step in steps {
        let step_name = step
            .name
            .as_deref()
            .or(step.uses.as_deref())
            .unwrap_or("run");

        let step_container = container.filter(|_| step.container.unwrap_or(true));
        let mut condition_env = merged_env(base_env, &resolved.env, &step.env);
        if step_container.is_some() {
            condition_env = merged_env(&condition_env, &resolved.container.env, &BTreeMap::new());
        }
        let mut condition_inputs = BTreeMap::new();
        let preliminary_expr = ExpressionContext {
            event: base_env
                .get("CI_EVENT")
                .map(String::as_str)
                .unwrap_or("manual"),
            branch: base_env.get("CI_BRANCH").map(String::as_str),
            root: &ctx.repo.root,
            env: &condition_env,
            matrix: &empty_matrix,
            inputs: &empty_inputs,
            success: workflow_failure == 0 && !previous_failed,
            previous_failed,
        };
        if step.uses.is_some() {
            condition_inputs = native_step_inputs(step, &preliminary_expr);
        }
        let expr = ExpressionContext {
            inputs: if condition_inputs.is_empty() {
                &empty_inputs
            } else {
                &condition_inputs
            },
            ..preliminary_expr
        };
        let should_run = if let Some(container) = step_container {
            let command_probe = |name: &str| {
                container.backend.command_exists(
                    &ContainerCommandExistsSpec {
                        image: &container.image,
                        repo_root: &ctx.repo.root,
                        env: &condition_env,
                        platform: Some(&container.platform),
                    },
                    name,
                )
            };
            evaluate_condition_with_probe(
                step.if_condition.as_deref(),
                &expr,
                Some(&command_probe),
            )?
        } else {
            evaluate_condition(step.if_condition.as_deref(), &expr)
        };
        if !should_run {
            ctx.output
                .verbose(format!("skipping step `{step_name}` due to condition"));
            continue;
        }

        ctx.output.info(format!("--> {step_name}"));

        let status = if step.uses.is_some() {
            run_native_uses_step(
                ctx,
                resolved,
                step,
                step_name,
                &expr,
                artifacts,
                &mut cache_state,
            )?
        } else if let Some(run) = step.run.as_deref() {
            let shell = step
                .shell
                .as_deref()
                .or(resolved.execution.shell.as_deref())
                .unwrap_or(&ctx.config.defaults.shell);
            let script = interpolate_expressions(run, &expr);
            let workdir = resolve_workdir(
                &ctx.repo.root,
                step.working_directory
                    .as_deref()
                    .map(Path::new)
                    .or(resolved.execution.workspace.as_deref()),
            );
            if let Some(container) = step_container {
                container.backend.run_shell(&ContainerShellSpec {
                    image: &container.image,
                    repo_root: &ctx.repo.root,
                    shell,
                    script: &script,
                    env: &condition_env,
                    workdir: &workdir,
                    platform: Some(&container.platform),
                    options: None,
                    extra_volumes: &resolved.container.volumes,
                    cache_mounts: &native_cache_mounts,
                    container_workdir: resolved.container.workdir.as_deref(),
                })?
            } else {
                run_shell(shell, &script, &workdir, &condition_env)?
            }
        } else {
            return Err(CiError::Message(format!(
                "{} native step is missing `run` and `use`",
                resolved.path.display()
            )));
        };
        previous_failed = status != 0;
        if status != 0 && !step.continue_on_error && workflow_failure == 0 {
            workflow_failure = status;
        }
    }
    save_pending_caches(ctx, &cache_state)?;
    Ok(workflow_failure)
}

fn run_native_uses_step(
    ctx: &AppContext,
    resolved: &ResolvedWorkflow,
    step: &NativeStep,
    default_name: &str,
    expr: &ExpressionContext<'_>,
    artifacts: &mut ArtifactSession,
    cache_state: &mut CacheState,
) -> Result<i32> {
    let uses = step.uses.as_deref().ok_or_else(|| {
        CiError::Message(format!(
            "{} native action step is missing `use`",
            resolved.path.display()
        ))
    })?;
    let invocation = BuiltinStepInvocation {
        workflow_name: &resolved.name,
        default_name,
        uses,
        with: &step.with,
        extra: Some(&step.extra),
        inline_run: step.run.clone(),
        shell: Some(
            step.shell
                .as_deref()
                .or(resolved.execution.shell.as_deref())
                .unwrap_or(&ctx.config.defaults.shell)
                .to_string(),
        ),
        workdir: Some(resolve_workdir(
            &ctx.repo.root,
            step.working_directory
                .as_deref()
                .map(Path::new)
                .or(resolved.execution.workspace.as_deref()),
        )),
        expr,
    };
    let mut state = BuiltinStepState {
        artifacts,
        cache_state,
    };
    run_builtin_step(ctx, &invocation, &mut state)?.ok_or_else(|| {
        CiError::Message(format!(
            "{} uses unsupported native action source `{uses}`",
            resolved.path.display()
        ))
    })
}

fn native_step_inputs(step: &NativeStep, expr: &ExpressionContext<'_>) -> BTreeMap<String, String> {
    let mut inputs = interpolate_map(&step.extra, expr);
    inputs.extend(interpolate_map(&step.with, expr));
    inputs
}
