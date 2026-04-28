use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::artifacts::ArtifactSession;
use crate::conditions::{
    evaluate_condition, evaluate_condition_with_probe, interpolate_expressions, ExpressionContext,
};
use crate::config::ContainerType;
use crate::containers::{
    container_platform, generated_native_containerfile, normalized_rust_components,
    validate_container_image_ref, validate_container_packages, ContainerBackend,
    ContainerCommandExistsSpec, ContainerShellSpec,
};
use crate::error::{CiError, Result};
use crate::git::sanitize_component;
use crate::runner::{AppContext, ContainerOverride, RunInvocation};
use crate::workflow::{NativeStep, ResolvedWorkflow, StepContainerConfig};

use super::builtins::{interpolate_map, run_builtin_step, BuiltinStepInvocation, BuiltinStepState};
use super::cache::{save_pending_caches, CacheState};
use super::env::{merged_env, resolve_workdir, run_shell};
use super::native_container::{
    native_container_base_image, native_container_cache_mounts, native_container_effective_type,
    prepare_native_container_image, NativeContainerExecution,
};

struct StepContainerExecution<'a> {
    backend: &'a ContainerBackend,
    image: String,
    platform: String,
    env: BTreeMap<String, String>,
    volumes: Vec<String>,
    workdir: Option<String>,
    readonly: Option<bool>,
    cache_mounts: Vec<(PathBuf, String)>,
    build_status: i32,
}

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
    invocation: &RunInvocation,
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
    let step_container_backend =
        if needs_step_container_backend(invocation, steps, container.is_some()) {
            Some(ContainerBackend::detect(invocation.container_runtime)?)
        } else {
            None
        };
    let forwarded_step = detect_build_arg_step(resolved, steps, &invocation.workflow_args)?;
    let empty_matrix = BTreeMap::new();
    let empty_inputs = BTreeMap::new();
    for (index, step) in steps.iter().enumerate() {
        let forwarded_args = forwarded_step
            .filter(|target| *target == index)
            .map(|_| invocation.workflow_args.as_slice());
        let step_name = step
            .name
            .as_deref()
            .or(step.uses.as_deref())
            .unwrap_or("run");

        let step_container = prepare_step_container(
            ctx,
            invocation,
            resolved,
            steps,
            step,
            step_name,
            index,
            container,
            step_container_backend.as_ref(),
            &native_cache_mounts,
        )?;
        let mut condition_env = merged_env(base_env, &resolved.env, &step.env);
        if let Some(container) = &step_container {
            condition_env = merged_env(&condition_env, &container.env, &BTreeMap::new());
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
        let should_run = if let Some(container) = &step_container {
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
            let mut state = BuiltinStepState {
                artifacts,
                cache_state: &mut cache_state,
            };
            run_native_uses_step(
                ctx,
                invocation.container_runtime,
                resolved,
                step,
                &expr,
                forwarded_args,
                &mut state,
            )?
        } else if let Some(run) = step.run.as_deref() {
            let shell = step
                .shell
                .as_deref()
                .or(resolved.execution.shell.as_deref())
                .unwrap_or(&ctx.config.defaults.shell);
            let script = if let Some(args) = forwarded_args {
                append_args_to_build_script(&interpolate_expressions(run, &expr), args)
            } else {
                interpolate_expressions(run, &expr)
            };
            let workdir = resolve_workdir(
                &ctx.repo.root,
                step.working_directory
                    .as_deref()
                    .map(Path::new)
                    .or(resolved.execution.workspace.as_deref()),
            );
            ctx.output.verbose_at(
                2,
                format!(
                    "running step `{step_name}` with shell `{shell}` in {}",
                    workdir.display()
                ),
            );
            ctx.output.verbose_at(
                3,
                format!(
                    "step `{step_name}` environment contains {} variable(s)",
                    condition_env.len()
                ),
            );
            if let Some(container) = &step_container {
                ctx.output.verbose_at(
                    2,
                    format!(
                        "step `{step_name}` container image `{}` on {}",
                        container.image, container.platform
                    ),
                );
            }
            if let Some(container) = step_container {
                if container.build_status != 0 {
                    container.build_status
                } else {
                    container.backend.run_shell(&ContainerShellSpec {
                        image: &container.image,
                        repo_root: &ctx.repo.root,
                        shell,
                        script: &script,
                        env: &condition_env,
                        workdir: &workdir,
                        platform: Some(&container.platform),
                        options: None,
                        extra_volumes: &container.volumes,
                        cache_mounts: &container.cache_mounts,
                        container_workdir: container.workdir.as_deref(),
                        readonly: step.readonly.or(container.readonly).unwrap_or(false),
                    })?
                }
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

fn needs_step_container_backend(
    invocation: &RunInvocation,
    steps: &[NativeStep],
    has_workflow_container: bool,
) -> bool {
    invocation.container_override != ContainerOverride::Disable
        && !has_workflow_container
        && steps.iter().any(|step| {
            step.uses.is_none()
                && step.run.is_some()
                && step.container.unwrap_or(true)
                && step.container_config.is_some()
        })
}

#[allow(clippy::too_many_arguments)]
fn prepare_step_container<'a>(
    ctx: &AppContext,
    invocation: &RunInvocation,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
    step: &NativeStep,
    step_name: &str,
    step_index: usize,
    workflow_container: Option<&NativeContainerExecution<'a>>,
    step_backend: Option<&'a ContainerBackend>,
    native_cache_mounts: &[(PathBuf, String)],
) -> Result<Option<StepContainerExecution<'a>>> {
    if invocation.container_override == ContainerOverride::Disable || step.container == Some(false)
    {
        return Ok(None);
    }

    if let Some(config) = &step.container_config {
        if step.uses.is_some() || step.run.is_none() {
            return Ok(workflow_container.map(|container| {
                workflow_step_container(container, resolved, native_cache_mounts)
            }));
        }

        return prepare_configured_step_container(
            ctx,
            invocation,
            resolved,
            steps,
            step_name,
            step_index,
            workflow_container,
            step_backend,
            native_cache_mounts,
            config,
        )
        .map(Some);
    }

    Ok(workflow_container
        .filter(|_| step.container.unwrap_or(true))
        .map(|container| workflow_step_container(container, resolved, native_cache_mounts)))
}

#[allow(clippy::too_many_arguments)]
fn prepare_configured_step_container<'a>(
    ctx: &AppContext,
    invocation: &RunInvocation,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
    step_name: &str,
    step_index: usize,
    workflow_container: Option<&NativeContainerExecution<'a>>,
    step_backend: Option<&'a ContainerBackend>,
    native_cache_mounts: &[(PathBuf, String)],
    config: &StepContainerConfig,
) -> Result<StepContainerExecution<'a>> {
    let backend = workflow_container
        .map(|container| container.backend)
        .or(step_backend)
        .ok_or_else(|| {
            CiError::Message("container runtime was not initialized for step container".to_string())
        })?;
    let platform = config
        .platform
        .clone()
        .or_else(|| workflow_container.map(|container| container.platform.clone()))
        .unwrap_or_else(|| container_platform(resolved, &invocation.arch));
    let mut env = resolved.container.env.clone();
    env.extend(config.env.clone());
    let mut volumes = resolved.container.volumes.clone();
    volumes.extend(config.volumes.clone());
    let workdir = config
        .workdir
        .clone()
        .or_else(|| resolved.container.workdir.clone());
    let readonly = config.readonly.or(resolved.container.readonly);

    let build_file = config
        .file
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| step_container_file(&ctx.repo.root, value));
    let has_generated_layers = !config.packages.is_empty() || !config.components.is_empty();
    let uses_workflow_image = !has_generated_layers
        && build_file.is_none()
        && config.image.is_none()
        && workflow_container.is_some();
    let generated_image =
        generated_step_container_image_name(&resolved.name, step_name, step_index, &platform);
    let image = if has_generated_layers {
        config
            .image
            .clone()
            .filter(|_| build_file.is_some())
            .unwrap_or(generated_image)
    } else if build_file.is_some() {
        config.image.clone().unwrap_or(generated_image)
    } else {
        config
            .image
            .clone()
            .or_else(|| workflow_container.map(|container| container.image.clone()))
            .ok_or_else(|| {
                CiError::Usage(format!(
                    "{} step `{step_name}` container must set `image` or `file`",
                    resolved.path.display()
                ))
            })?
    };
    validate_container_image_ref(&image)?;

    if !config.components.is_empty()
        && native_container_effective_type(ctx, resolved, steps) != ContainerType::Rust
    {
        return Err(CiError::Usage(
            "step container.components is only supported for Rust containers".to_string(),
        ));
    }
    validate_container_packages(&config.packages)?;
    let components = normalized_rust_components(&config.components)?;

    let build_status = if let Some(file) = build_file {
        if !file.exists() {
            return Err(CiError::NotFound(file.display().to_string()));
        }
        let base_image = if has_generated_layers {
            generated_step_container_base_image_name(
                &resolved.name,
                step_name,
                step_index,
                &platform,
            )
        } else {
            image.clone()
        };
        validate_container_image_ref(&base_image)?;
        ctx.output.verbose(format!(
            "building step `{step_name}` container from {}",
            file.display()
        ));
        let status = backend.build(&file, &ctx.repo.root, &base_image, Some(&platform))?;
        if status != 0 || !has_generated_layers {
            status
        } else {
            build_step_package_image(
                ctx,
                backend,
                resolved,
                step_name,
                step_index,
                &platform,
                &base_image,
                &image,
                &config.packages,
                &components,
            )?
        }
    } else if has_generated_layers {
        let base_image = config
            .image
            .clone()
            .or_else(|| workflow_container.map(|container| container.image.clone()))
            .unwrap_or_else(|| native_container_base_image(ctx, resolved, steps));
        validate_container_image_ref(&base_image)?;
        build_step_package_image(
            ctx,
            backend,
            resolved,
            step_name,
            step_index,
            &platform,
            &base_image,
            &image,
            &config.packages,
            &components,
        )?
    } else {
        0
    };

    Ok(StepContainerExecution {
        backend,
        image,
        platform,
        env,
        volumes,
        workdir,
        readonly,
        cache_mounts: if uses_workflow_image {
            native_cache_mounts.to_vec()
        } else {
            Vec::new()
        },
        build_status,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_step_package_image(
    ctx: &AppContext,
    backend: &ContainerBackend,
    resolved: &ResolvedWorkflow,
    step_name: &str,
    step_index: usize,
    platform: &str,
    base_image: &str,
    image: &str,
    packages: &[String],
    components: &[String],
) -> Result<i32> {
    let file_stem = step_container_file_stem(&resolved.name, step_name, step_index, platform);
    let dir = ctx.repo.state_dir.join("containers");
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{file_stem}.Containerfile"));
    fs::write(
        &file,
        generated_native_containerfile(base_image, packages, components),
    )?;
    ctx.output.verbose(format!(
        "building step `{step_name}` package container from {}",
        file.display()
    ));
    backend.build(&file, &dir, image, Some(platform))
}

fn workflow_step_container<'a>(
    container: &NativeContainerExecution<'a>,
    resolved: &ResolvedWorkflow,
    native_cache_mounts: &[(PathBuf, String)],
) -> StepContainerExecution<'a> {
    StepContainerExecution {
        backend: container.backend,
        image: container.image.clone(),
        platform: container.platform.clone(),
        env: resolved.container.env.clone(),
        volumes: resolved.container.volumes.clone(),
        workdir: resolved.container.workdir.clone(),
        readonly: resolved.container.readonly,
        cache_mounts: native_cache_mounts.to_vec(),
        build_status: 0,
    }
}

fn step_container_file(repo_root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn generated_step_container_image_name(
    workflow_name: &str,
    step_name: &str,
    step_index: usize,
    platform: &str,
) -> String {
    let file_stem = step_container_file_stem(workflow_name, step_name, step_index, platform);
    format!("localhost/{file_stem}:latest")
}

fn generated_step_container_base_image_name(
    workflow_name: &str,
    step_name: &str,
    step_index: usize,
    platform: &str,
) -> String {
    let file_stem = step_container_file_stem(workflow_name, step_name, step_index, platform);
    format!("localhost/{file_stem}-base:latest")
}

fn step_container_file_stem(
    workflow_name: &str,
    step_name: &str,
    step_index: usize,
    platform: &str,
) -> String {
    format!(
        "ci-{}-step-{}-{}-{}",
        sanitize_component(workflow_name),
        step_index + 1,
        sanitize_component(step_name),
        sanitize_component(platform)
    )
}

fn run_native_uses_step(
    ctx: &AppContext,
    container_runtime: crate::config::ContainerRuntime,
    resolved: &ResolvedWorkflow,
    step: &NativeStep,
    expr: &ExpressionContext<'_>,
    forwarded_args: Option<&[String]>,
    state: &mut BuiltinStepState<'_>,
) -> Result<i32> {
    let uses = step.uses.as_deref().ok_or_else(|| {
        CiError::Message(format!(
            "{} native action step is missing `use`",
            resolved.path.display()
        ))
    })?;
    let mut with = step.with.clone();
    if let Some(args) = forwarded_args {
        with.entry("args".to_string())
            .or_insert_with(|| args.join(" "));
    }

    let invocation = BuiltinStepInvocation {
        workflow_name: &resolved.name,
        default_name: step
            .name
            .as_deref()
            .or(step.uses.as_deref())
            .unwrap_or("run"),
        uses,
        with: &with,
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
        container_runtime,
        expr,
    };
    run_builtin_step(ctx, &invocation, state)?.ok_or_else(|| {
        CiError::Message(format!(
            "{} uses unsupported native action source `{uses}`",
            resolved.path.display()
        ))
    })
}

fn detect_build_arg_step(
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
    args: &[String],
) -> Result<Option<usize>> {
    if args.is_empty() {
        return Ok(None);
    }

    if resolved.name != "build" {
        return Err(CiError::Usage(format!(
            "workflow arguments can only be forwarded to the `build` workflow; `{}` is not supported",
            resolved.name
        )));
    }

    let named_build_steps = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| step.name.as_deref().map(is_build_label).unwrap_or(false))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if named_build_steps.len() == 1 {
        return Ok(named_build_steps.first().copied());
    }
    if named_build_steps.len() > 1 {
        return Err(CiError::Usage(format!(
            "{} has multiple steps named `build`; rename the step that should receive workflow arguments",
            resolved.path.display()
        )));
    }

    let build_command_steps = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            step.run
                .as_deref()
                .map(script_contains_build_command)
                .unwrap_or(false)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if build_command_steps.len() == 1 {
        return Ok(build_command_steps.first().copied());
    }
    if build_command_steps.len() > 1 {
        return Err(CiError::Usage(format!(
            "{} has multiple possible build steps; name the intended step `build`",
            resolved.path.display()
        )));
    }

    if steps.len() == 1 {
        return Ok(Some(0));
    }

    Err(CiError::Usage(format!(
        "{} could not detect which step should receive workflow arguments; name the build step `build`",
        resolved.path.display()
    )))
}

fn is_build_label(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value == "build" || value.starts_with("build ")
}

fn script_contains_build_command(script: &str) -> bool {
    script
        .lines()
        .any(|line| is_build_command_line(line.trim()))
}

fn is_build_command_line(line: &str) -> bool {
    if line.is_empty() || line.starts_with('#') {
        return false;
    }

    let line = line.to_ascii_lowercase();
    [
        "cargo build",
        "npm run build",
        "npm build",
        "yarn build",
        "pnpm build",
        "go build",
        "mvn package",
        "mvn install",
        "gradle build",
        "./gradlew build",
        "dotnet build",
        "python -m build",
        "python3 -m build",
    ]
    .iter()
    .any(|needle| line.contains(needle))
}

fn append_args_to_build_script(script: &str, args: &[String]) -> String {
    let rendered_args = shell_quote_args(args);
    let trailing_newline = script.ends_with('\n');
    let mut lines = script.lines().map(ToString::to_string).collect::<Vec<_>>();
    if lines.is_empty() {
        return rendered_args;
    }

    let target = lines
        .iter()
        .position(|line| is_build_command_line(line.trim()))
        .or_else(|| lines.iter().rposition(|line| !line.trim().is_empty()))
        .unwrap_or(0);
    lines[target].push(' ');
    lines[target].push_str(&rendered_args);

    let mut result = lines.join("\n");
    if trailing_newline {
        result.push('\n');
    }
    result
}

fn shell_quote_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }

    if arg.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '@' | '%' | '_' | '+' | '=' | ':' | ',' | '.' | '/' | '-'
            )
    }) {
        return arg.to_string();
    }

    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn native_step_inputs(step: &NativeStep, expr: &ExpressionContext<'_>) -> BTreeMap<String, String> {
    let mut inputs = interpolate_map(&step.extra, expr);
    inputs.extend(interpolate_map(&step.with, expr));
    inputs
}
