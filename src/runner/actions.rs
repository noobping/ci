use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::actions::{ActionRunStep, ActionStep, ActionUsesStep, ActionsJob, ActionsWorkflow};
use crate::artifacts::ArtifactSession;
use crate::conditions::{evaluate_condition, interpolate_expressions, ExpressionContext};
use crate::config::{Architecture, ContainerRuntime};
use crate::containers::{container_platform, ContainerBackend, ContainerShellSpec};
use crate::error::{CiError, Result};
use crate::git::{command_exists, sanitize_component};
use crate::runner::{AppContext, RunInvocation};
use crate::workflow::ResolvedWorkflow;

use super::action_metadata::{
    load_action_metadata, parse_remote_action, ActionMetadataStep, ActionRuns,
};
use super::builtins::{run_builtin_step, BuiltinStepInvocation, BuiltinStepState};
use super::cache::{save_pending_caches, CacheState};
use super::env::{merged_env, resolve_workdir, run_shell};
use super::jobs::order_jobs;

#[derive(Clone, Copy)]
struct StepStatus {
    success: bool,
    previous_failed: bool,
}

struct ActionsJobExecution<'a> {
    workflow: &'a ActionsWorkflow,
    resolved: &'a ResolvedWorkflow,
    job: &'a ActionsJob,
    arch: &'a Architecture,
    matrix: &'a BTreeMap<String, String>,
    container_runtime: ContainerRuntime,
    base_env: &'a BTreeMap<String, String>,
    backend: Option<&'a ContainerBackend>,
    artifacts: &'a mut ArtifactSession,
    cache_state: &'a mut CacheState,
}

pub(crate) fn run_actions_workflow(
    ctx: &AppContext,
    invocation: &RunInvocation,
    resolved: &ResolvedWorkflow,
    workflow: &ActionsWorkflow,
    base_env: &BTreeMap<String, String>,
    artifacts: &mut ArtifactSession,
) -> Result<i32> {
    let jobs = order_jobs(&workflow.jobs)?;
    let mut cache_state = CacheState::default();
    let empty_inputs = BTreeMap::new();

    for job in jobs {
        for matrix in if job.matrix.is_empty() {
            vec![BTreeMap::new()]
        } else {
            job.matrix.clone()
        } {
            let env = merged_env(base_env, &workflow.env, &job.env);
            let expr = ExpressionContext {
                event: &invocation.event,
                branch: invocation.branch.as_deref(),
                root: &ctx.repo.root,
                env: &env,
                matrix: &matrix,
                inputs: &empty_inputs,
                success: true,
                previous_failed: false,
            };
            if !evaluate_condition(job.if_condition.as_deref(), &expr) {
                ctx.output
                    .verbose(format!("skipping job `{}` due to condition", job.name));
                continue;
            }

            let backend = if job.container.is_some() || !job.services.is_empty() {
                Some(ContainerBackend::detect(invocation.container_runtime)?)
            } else {
                None
            };

            let mut services = Vec::new();
            let platform = container_platform(resolved, &invocation.arch);
            if let Some(backend) = &backend {
                for (name, service) in &job.services {
                    let container_name = format!(
                        "ci-{}-{}-{}",
                        sanitize_component(&workflow.name),
                        sanitize_component(&job.id),
                        sanitize_component(name)
                    );
                    backend.start_service(&container_name, service, Some(&platform))?;
                    services.push(container_name);
                }
            }

            let mut execution = ActionsJobExecution {
                workflow,
                resolved,
                job: &job,
                arch: &invocation.arch,
                matrix: &matrix,
                container_runtime: invocation.container_runtime,
                base_env,
                backend: backend.as_ref(),
                artifacts,
                cache_state: &mut cache_state,
            };
            let status = run_actions_job(ctx, &mut execution)?;

            if let Some(backend) = &backend {
                for service in services {
                    let _ = backend.stop_container(&service);
                }
            }

            save_pending_caches(ctx, &cache_state)?;
            cache_state.pending.clear();

            if status != 0 && !job.continue_on_error {
                return Ok(status);
            }
        }
    }

    Ok(0)
}

fn run_actions_job(ctx: &AppContext, execution: &mut ActionsJobExecution<'_>) -> Result<i32> {
    let mut success = true;
    let mut previous_failed = false;
    for step in &execution.job.steps {
        let status = StepStatus {
            success,
            previous_failed,
        };
        let exit_code = match step {
            ActionStep::Run(step) => run_actions_run_step(ctx, execution, step, status)?,
            ActionStep::Uses(step) => run_actions_uses_step(ctx, execution, step, status)?,
        };
        success = exit_code == 0;
        previous_failed = exit_code != 0;
        if exit_code != 0 && !step_continue_on_error(step) {
            return Ok(exit_code);
        }
    }
    Ok(0)
}

fn step_continue_on_error(step: &ActionStep) -> bool {
    match step {
        ActionStep::Run(step) => step.continue_on_error,
        ActionStep::Uses(step) => step.continue_on_error,
    }
}

fn run_actions_run_step(
    ctx: &AppContext,
    execution: &ActionsJobExecution<'_>,
    step: &ActionRunStep,
    status: StepStatus,
) -> Result<i32> {
    let merged = merged_env(
        &merged_env(
            execution.base_env,
            &execution.workflow.env,
            &execution.job.env,
        ),
        &BTreeMap::new(),
        &step.env,
    );
    let expr = ExpressionContext {
        event: execution
            .base_env
            .get("CI_EVENT")
            .map(String::as_str)
            .unwrap_or("manual"),
        branch: execution.base_env.get("CI_BRANCH").map(String::as_str),
        root: &ctx.repo.root,
        env: &merged,
        matrix: execution.matrix,
        inputs: &BTreeMap::new(),
        success: status.success,
        previous_failed: status.previous_failed,
    };
    if !evaluate_condition(step.if_condition.as_deref(), &expr) {
        ctx.output.verbose(format!(
            "skipping action step `{}` due to condition",
            step.name
        ));
        return Ok(0);
    }

    ctx.output.info(format!("--> {}", step.name));

    let shell = step
        .shell
        .as_deref()
        .or(execution.job.defaults.shell.as_deref())
        .or(execution.workflow.defaults.shell.as_deref())
        .unwrap_or(&ctx.config.defaults.shell);
    let script = interpolate_expressions(&step.run, &expr);
    let workdir = resolve_workdir(
        &ctx.repo.root,
        step.working_directory
            .as_deref()
            .map(Path::new)
            .or_else(|| {
                execution
                    .job
                    .defaults
                    .working_directory
                    .as_deref()
                    .map(Path::new)
            })
            .or_else(|| {
                execution
                    .workflow
                    .defaults
                    .working_directory
                    .as_deref()
                    .map(Path::new)
            })
            .or(execution.resolved.execution.workspace.as_deref()),
    );
    ctx.output.verbose_at(
        2,
        format!(
            "running action step `{}` with shell `{shell}` in {}",
            step.name,
            workdir.display()
        ),
    );
    ctx.output.verbose_at(
        3,
        format!(
            "action step `{}` environment contains {} variable(s)",
            step.name,
            merged.len()
        ),
    );

    if let Some(container) = execution.job.container.as_ref() {
        let platform = container_platform(execution.resolved, execution.arch);
        ctx.output.verbose_at(
            2,
            format!(
                "action step `{}` container image `{}` on {platform}",
                step.name, container.image
            ),
        );
        execution
            .backend
            .ok_or_else(|| CiError::Message("container runtime was not initialised".to_string()))?
            .run_shell(&ContainerShellSpec {
                image: &container.image,
                repo_root: &ctx.repo.root,
                shell,
                script: &script,
                env: &merged,
                workdir: &workdir,
                platform: Some(&platform),
                options: container.options.as_deref(),
                extra_volumes: &[],
                cache_mounts: &[],
                container_workdir: None,
                readonly: execution.resolved.container.readonly.unwrap_or(false),
            })
    } else {
        run_shell(shell, &script, &workdir, &merged)
    }
}

fn run_actions_uses_step(
    ctx: &AppContext,
    execution: &mut ActionsJobExecution<'_>,
    step: &ActionUsesStep,
    status: StepStatus,
) -> Result<i32> {
    let empty_env = BTreeMap::new();
    let merged = merged_env(
        &merged_env(
            execution.base_env,
            &execution.workflow.env,
            &execution.job.env,
        ),
        &empty_env,
        &step.env,
    );
    let expr = ExpressionContext {
        event: execution
            .base_env
            .get("CI_EVENT")
            .map(String::as_str)
            .unwrap_or("manual"),
        branch: execution.base_env.get("CI_BRANCH").map(String::as_str),
        root: &ctx.repo.root,
        env: &merged,
        matrix: execution.matrix,
        inputs: &step.with,
        success: status.success,
        previous_failed: status.previous_failed,
    };
    if !evaluate_condition(step.if_condition.as_deref(), &expr) {
        ctx.output.verbose(format!(
            "skipping action step `{}` due to condition",
            step.name
        ));
        return Ok(0);
    }

    ctx.output.info(format!("--> {}", step.name));

    let invocation = BuiltinStepInvocation {
        workflow_name: &execution.workflow.name,
        default_name: &step.name,
        uses: &step.uses,
        with: &step.with,
        extra: None,
        inline_run: None,
        shell: Some(
            execution
                .job
                .defaults
                .shell
                .as_deref()
                .or(execution.workflow.defaults.shell.as_deref())
                .unwrap_or(&ctx.config.defaults.shell)
                .to_string(),
        ),
        workdir: Some(resolve_workdir(
            &ctx.repo.root,
            step.working_directory
                .as_deref()
                .map(Path::new)
                .or_else(|| {
                    execution
                        .job
                        .defaults
                        .working_directory
                        .as_deref()
                        .map(Path::new)
                })
                .or_else(|| {
                    execution
                        .workflow
                        .defaults
                        .working_directory
                        .as_deref()
                        .map(Path::new)
                })
                .or(execution.resolved.execution.workspace.as_deref()),
        )),
        container_runtime: execution.container_runtime,
        expr: &expr,
    };
    let mut state = BuiltinStepState {
        artifacts: execution.artifacts,
        cache_state: execution.cache_state,
    };
    if let Some(exit_code) = run_builtin_step(ctx, &invocation, &mut state)? {
        return Ok(exit_code);
    }

    if step.uses.starts_with("docker://") {
        let image = step.uses.trim_start_matches("docker://");
        ctx.output
            .verbose(format!("running docker action `{}`", step.uses));
        let platform = container_platform(execution.resolved, execution.arch);
        return execution
            .backend
            .ok_or_else(|| {
                CiError::Message("docker actions require a container runtime".to_string())
            })?
            .run_shell(&ContainerShellSpec {
                image,
                repo_root: &ctx.repo.root,
                shell: &ctx.config.defaults.shell,
                script: "true",
                env: &merged,
                workdir: &ctx.repo.root,
                platform: Some(&platform),
                options: None,
                extra_volumes: &[],
                cache_mounts: &[],
                container_workdir: None,
                readonly: execution.resolved.container.readonly.unwrap_or(false),
            });
    }

    let platform = container_platform(execution.resolved, execution.arch);
    if step.uses.starts_with("./") {
        let dir = ctx.repo.root.join(step.uses.trim_start_matches("./"));
        ctx.output.verbose(format!(
            "running local action `{}` from {}",
            step.uses,
            dir.display()
        ));
        return run_local_action(
            ctx,
            execution.matrix,
            &merged,
            execution.backend,
            &dir,
            &step.with,
            &platform,
        );
    }

    ctx.output
        .verbose(format!("resolving remote action `{}`", step.uses));
    let remote = parse_remote_action(&step.uses)?;
    let repo = ctx.git.clone_action_repo(
        &ctx.repo.actions_cache,
        execution.workflow.remote_base(),
        &remote.owner,
        &remote.repo,
        &remote.reference,
    )?;
    let dir = if remote.subpath.is_empty() {
        repo
    } else {
        repo.join(remote.subpath)
    };
    run_local_action(
        ctx,
        execution.matrix,
        &merged,
        execution.backend,
        &dir,
        &step.with,
        &platform,
    )
}

fn run_local_action(
    ctx: &AppContext,
    matrix: &BTreeMap<String, String>,
    base_env: &BTreeMap<String, String>,
    backend: Option<&ContainerBackend>,
    dir: &Path,
    inputs: &BTreeMap<String, String>,
    platform: &str,
) -> Result<i32> {
    let meta = load_action_metadata(dir)?;
    match meta.runs {
        ActionRuns::Composite { steps } => {
            let mut success = true;
            let mut previous_failed = false;
            for step in steps {
                match step {
                    ActionMetadataStep::Run(step) => {
                        let expr = ExpressionContext {
                            event: base_env
                                .get("CI_EVENT")
                                .map(String::as_str)
                                .unwrap_or("manual"),
                            branch: base_env.get("CI_BRANCH").map(String::as_str),
                            root: dir,
                            env: base_env,
                            matrix,
                            inputs,
                            success,
                            previous_failed,
                        };
                        if !evaluate_condition(step.if_condition.as_deref(), &expr) {
                            continue;
                        }
                        let shell = step.shell.as_deref().unwrap_or(&ctx.config.defaults.shell);
                        let script = interpolate_expressions(&step.run, &expr);
                        let workdir = resolve_workdir(
                            dir,
                            step.working_directory
                                .as_deref()
                                .map(Path::new)
                                .or(Some(Path::new("."))),
                        );
                        let status = run_shell(shell, &script, &workdir, base_env)?;
                        success = status == 0;
                        previous_failed = status != 0;
                        if status != 0 && !step.continue_on_error {
                            return Ok(status);
                        }
                    }
                    ActionMetadataStep::Uses => {
                        return Err(CiError::Message(format!(
                            "composite action {} contains nested `uses`, which is not supported yet",
                            dir.display()
                        )));
                    }
                }
            }
            Ok(0)
        }
        ActionRuns::Docker {
            image,
            dockerfile,
            entrypoint,
            args,
        } => {
            let backend = backend.ok_or_else(|| {
                CiError::Message("docker actions require a container runtime".to_string())
            })?;
            let image = if let Some(image) = image {
                image
            } else {
                let dockerfile = dockerfile
                    .as_ref()
                    .map(|path| dir.join(path))
                    .unwrap_or_else(|| dir.join("Dockerfile"));
                let tag = format!(
                    "ci-action-{}",
                    sanitize_component(
                        dir.file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or("action")
                    )
                );
                let build_status = backend.build(&dockerfile, dir, &tag, Some(platform))?;
                if build_status != 0 {
                    return Ok(build_status);
                }
                tag
            };

            backend.run_action_container(
                &image,
                dir,
                base_env,
                entrypoint.as_deref(),
                &args.unwrap_or_default(),
                Some(platform),
            )
        }
        ActionRuns::Node { main } => {
            let path = dir.join(main);
            if command_exists("node") {
                let mut command = Command::new("node");
                command
                    .arg(&path)
                    .current_dir(dir)
                    .envs(base_env)
                    .stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
                Ok(command.status()?.code().unwrap_or(1))
            } else {
                let backend = backend.ok_or_else(|| {
                    CiError::Message("js actions require node or a container runtime".to_string())
                })?;
                backend.run_shell(&ContainerShellSpec {
                    image: &ctx.config.defaults.node_image,
                    repo_root: dir,
                    shell: &ctx.config.defaults.shell,
                    script: &format!(
                        "node {}",
                        path.file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or("index.js")
                    ),
                    env: base_env,
                    workdir: dir,
                    platform: Some(platform),
                    options: None,
                    extra_volumes: &[],
                    cache_mounts: &[],
                    container_workdir: None,
                    readonly: false,
                })
            }
        }
    }
}
