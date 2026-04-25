use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::IsTerminal;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use glob::glob;
use serde::Deserialize;

use crate::actions::{
    ActionRunStep, ActionService, ActionStep, ActionUsesStep, ActionsJob, ActionsWorkflow,
};
use crate::artifacts::ArtifactSession;
use crate::cli::{GlobalOptions, HookArgs, InitArgs, ListArgs, RunArgs, SelfArgs};
use crate::config::{Architecture, ContainerRuntime, ContainerType, ResolvedConfig};
use crate::error::{CiError, Result};
use crate::git::{
    command_exists, preferred_container_runtime, sanitize_component, CleanIgnoredMode, GitService,
};
use crate::output::Output;
use crate::repo::RepoInfo;
use crate::workflow::{
    self, canonical_events, kind_name, provider_name, select_workflows, NativeStep,
    ResolvedWorkflow, WorkflowMatch, WorkflowSource,
};

const DEFAULT_RUST_WORKFLOW: &str = r#"name: build
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
"#;

const DEFAULT_SHELL_WORKFLOW: &str = r#"name: build
on:
  - manual
steps:
  - name: Build
    run: echo "Add your build command to .ci/build.yml"
"#;

#[derive(Clone, Debug)]
pub struct AppContext {
    pub global: GlobalOptions,
    pub output: Output,
    pub repo: RepoInfo,
    pub config: ResolvedConfig,
    pub git: GitService,
}

impl AppContext {
    pub fn new(
        global: GlobalOptions,
        output: Output,
        repo: RepoInfo,
        config: ResolvedConfig,
        git: GitService,
    ) -> Self {
        Self {
            global,
            output,
            repo,
            config,
            git,
        }
    }
}

#[derive(Clone, Debug)]
struct RunRequest {
    workflow: Option<String>,
    event: String,
    dry_run: bool,
    keep_going: bool,
    arches: Vec<Architecture>,
    arch_overridden: bool,
    container_runtime: ContainerRuntime,
    respect_branches: bool,
    recursive_checkout: bool,
    lock: bool,
    hook_args: Vec<String>,
    branch: Option<String>,
}

#[derive(Clone, Debug)]
struct RunInvocation {
    event: String,
    arch: Architecture,
    container_runtime: ContainerRuntime,
    hook_args: Vec<String>,
    branch: Option<String>,
}

impl RunRequest {
    fn invocation_for_arch(&self, arch: Architecture) -> RunInvocation {
        RunInvocation {
            event: self.event.clone(),
            arch,
            container_runtime: self.container_runtime,
            hook_args: self.hook_args.clone(),
            branch: self.branch.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct ExpressionContext<'a> {
    event: &'a str,
    branch: Option<&'a str>,
    root: &'a Path,
    env: &'a BTreeMap<String, String>,
    matrix: &'a BTreeMap<String, String>,
    inputs: &'a BTreeMap<String, String>,
    success: bool,
    previous_failed: bool,
}

#[derive(Default)]
struct CacheState {
    pending: Vec<PendingCache>,
}

struct PendingCache {
    key: String,
    paths: Vec<String>,
}

#[derive(Clone, Copy)]
struct StepStatus {
    success: bool,
    previous_failed: bool,
}

struct NativeContainerExecution<'a> {
    backend: &'a ContainerBackend,
    image: String,
    platform: String,
}

struct PreparedNativeContainerImage {
    image: String,
    build_status: i32,
}

struct ActionsJobExecution<'a> {
    workflow: &'a ActionsWorkflow,
    resolved: &'a ResolvedWorkflow,
    job: &'a ActionsJob,
    arch: &'a Architecture,
    matrix: &'a BTreeMap<String, String>,
    base_env: &'a BTreeMap<String, String>,
    backend: Option<&'a ContainerBackend>,
    artifacts: &'a mut ArtifactSession,
    cache_state: &'a mut CacheState,
}

struct BuiltinStepInvocation<'a, 'b> {
    workflow_name: &'a str,
    default_name: &'a str,
    uses: &'a str,
    with: &'a BTreeMap<String, String>,
    extra: Option<&'a BTreeMap<String, String>>,
    inline_run: Option<String>,
    shell: Option<String>,
    workdir: Option<PathBuf>,
    expr: &'b ExpressionContext<'b>,
}

struct BuiltinStepState<'a> {
    artifacts: &'a mut ArtifactSession,
    cache_state: &'a mut CacheState,
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

const COMMIT_ACTION_NAMES: &[&str] = &["commit", "ci/commit"];
const SYNC_ACTION_NAMES: &[&str] = &["sync", "ci/sync"];

pub fn cmd_list(ctx: &AppContext, args: &ListArgs) -> Result<i32> {
    let porcelain = args.use_porcelain(std::io::stdout().is_terminal());
    let workflows = workflow::discover_all(&ctx.repo)?;
    if workflows.is_empty() {
        if !porcelain {
            ctx.output.info(format!(
                "No workflows found in {}",
                ctx.repo.ci_dir.display()
            ));
        }
        return Ok(0);
    }

    for item in workflows {
        if porcelain {
            println!(
                "{}\t{}\t{}\t{}",
                item.name,
                provider_name(&item.provider),
                kind_name(&item.kind),
                item.path.display()
            );
        } else {
            println!(
                "{:<28} {:<15} {:<12} {}",
                item.name,
                provider_name(&item.provider),
                kind_name(&item.kind),
                item.path.display()
            );
        }
    }

    Ok(0)
}

pub fn cmd_run(ctx: &AppContext, args: &RunArgs) -> Result<i32> {
    let keep_going = if args.keep_going {
        true
    } else if args.fail_fast {
        false
    } else {
        !ctx.config.defaults.fail_fast
    };

    let request = RunRequest {
        workflow: if args.all {
            None
        } else {
            args.workflow.clone()
        },
        event: args.event.clone(),
        dry_run: args.dry_run,
        keep_going,
        arches: ctx.config.defaults.arch.clone(),
        arch_overridden: !ctx.global.arch.is_empty(),
        container_runtime: args
            .container_runtime
            .unwrap_or(ctx.config.defaults.container_runtime),
        respect_branches: args.respect_branches,
        recursive_checkout: !args.no_recursive_checkout && ctx.config.defaults.recursive_checkout,
        lock: args.lock,
        hook_args: Vec::new(),
        branch: ctx.repo.branch.clone(),
    };
    execute_run(ctx, request)
}

pub fn cmd_hook(ctx: &AppContext, args: &HookArgs) -> Result<i32> {
    if !workflow::is_known_hook(&args.hook) {
        return Err(CiError::Usage(format!("unknown Git hook `{}`", args.hook)));
    }

    let branch = branch_from_hook(ctx, &args.hook, &args.hook_args)?;
    let keep_going = !ctx.config.defaults.fail_fast;
    let request = RunRequest {
        workflow: None,
        event: args.hook.clone(),
        dry_run: false,
        keep_going,
        arches: ctx.config.defaults.arch.clone(),
        arch_overridden: !ctx.global.arch.is_empty(),
        container_runtime: ctx.config.defaults.container_runtime,
        respect_branches: true,
        recursive_checkout: ctx.config.defaults.recursive_checkout,
        lock: true,
        hook_args: args.hook_args.clone(),
        branch: branch.clone(),
    };
    execute_run(ctx, request)
}

pub fn cmd_init(ctx: &AppContext, args: &InitArgs) -> Result<i32> {
    fs::create_dir_all(&ctx.repo.ci_dir)?;
    let build = ctx.repo.ci_dir.join("build.yml");
    if build.exists() && !args.force {
        return Err(CiError::Message(format!(
            "{} already exists; use `ci init --force` to replace it",
            build.display()
        )));
    }

    let content = if ctx.repo.root.join("Cargo.toml").exists() {
        DEFAULT_RUST_WORKFLOW
    } else {
        DEFAULT_SHELL_WORKFLOW
    };

    fs::write(&build, content)?;
    ctx.output.info(format!("Created {}", build.display()));
    Ok(0)
}

pub fn cmd_self(ctx: &AppContext, _args: &SelfArgs) -> Result<i32> {
    println!("ci {}", env!("CARGO_PKG_VERSION"));
    println!("executable: {}", ctx.repo.current_exe.display());
    println!("repository: {}", ctx.repo.root.display());
    Ok(0)
}

fn execute_run(ctx: &AppContext, request: RunRequest) -> Result<i32> {
    ctx.repo.ensure_state_dirs()?;

    let _lock = if request.lock {
        Some(RunLock::acquire(&ctx.repo.state_dir.join("lock"))?)
    } else {
        None
    };

    if request.recursive_checkout {
        ctx.git.ensure_submodules(&ctx.repo)?;
    }

    let workflows = workflow::discover_all(&ctx.repo)?;
    let matches = select_workflows(
        &workflows,
        &ctx.config,
        request.workflow.as_deref(),
        &request.event,
        request.branch.as_deref(),
        request.respect_branches,
    );

    if matches.is_empty() {
        if let Some(name) = &request.workflow {
            return Ok(if workflows.iter().any(|workflow| workflow.name == *name) {
                0
            } else {
                127
            });
        }
        ctx.output
            .verbose(format!("no workflows matched event `{}`", request.event));
        return Ok(0);
    }

    if request.dry_run {
        for item in &matches {
            for arch in workflow_execution_arches(&request, &item.resolved) {
                println!(
                    "would run {} [{}] for {} because {}",
                    item.workflow.name,
                    provider_name(&item.workflow.provider),
                    arch,
                    item.reasons.join("; ")
                );
            }
        }
        return Ok(0);
    }

    let run_id = new_run_id();
    let mut artifacts = ArtifactSession::new(
        &ctx.repo,
        &request.event,
        request.branch.as_deref(),
        &run_id,
        ctx.output.clone(),
    )?;
    let mut last_failure = 0;

    for item in matches {
        let arches = workflow_execution_arches(&request, &item.resolved);
        let show_arch = arches.len() > 1 || !item.resolved.container.arch.is_empty();
        for arch in arches {
            if show_arch {
                ctx.output
                    .info(format!("==> {} ({arch})", item.workflow.name));
            } else {
                ctx.output.info(format!("==> {}", item.workflow.name));
            }
            let invocation = request.invocation_for_arch(arch);
            let status = run_one_workflow(ctx, &invocation, &item, &run_id, &mut artifacts)?;
            if status != 0 {
                last_failure = status;
                if !request.keep_going {
                    artifacts.finish()?;
                    return Ok(status);
                }
            }
        }
    }

    artifacts.finish()?;
    Ok(last_failure)
}

fn workflow_execution_arches(
    request: &RunRequest,
    resolved: &ResolvedWorkflow,
) -> Vec<Architecture> {
    if !request.arch_overridden {
        let container_arch = resolved.container.arch.to_vec();
        if !container_arch.is_empty() {
            return container_arch;
        }
    }
    request.arches.clone()
}

fn run_one_workflow(
    ctx: &AppContext,
    invocation: &RunInvocation,
    item: &WorkflowMatch,
    run_id: &str,
    artifacts: &mut ArtifactSession,
) -> Result<i32> {
    let base_env = workflow_env(ctx, invocation, &item.resolved, run_id);
    let status = match &item.workflow.source {
        WorkflowSource::Executable(_) => {
            run_executable(ctx, invocation, &item.resolved, &base_env)?
        }
        WorkflowSource::NativeYaml(native) => {
            if native_container_enabled(&item.resolved) {
                run_native_yaml_containerized(
                    ctx,
                    invocation,
                    &item.resolved,
                    &native.steps,
                    &base_env,
                    artifacts,
                )?
            } else {
                run_native_yaml(
                    ctx,
                    invocation,
                    &item.resolved,
                    &native.steps,
                    &base_env,
                    artifacts,
                    None,
                )?
            }
        }
        WorkflowSource::Container(_) => {
            run_container_workflow(ctx, invocation, &item.resolved, &base_env)?
        }
        WorkflowSource::Actions(actions) => run_actions_workflow(
            ctx,
            invocation,
            &item.resolved,
            actions,
            &base_env,
            artifacts,
        )?,
    };

    let mut stored = artifacts.take_pending_artifacts(&item.workflow.name);
    if status == 0 && !matches!(&item.workflow.source, WorkflowSource::Actions(_)) {
        stored.extend(artifacts.capture_declared(
            &item.workflow.name,
            &item.resolved.artifacts,
            false,
        )?);
    }
    artifacts.record_workflow(
        &item.workflow.name,
        provider_name(&item.workflow.provider),
        kind_name(&item.workflow.kind),
        &item.workflow.path,
        status,
        stored,
    );
    Ok(status)
}

fn run_executable(
    ctx: &AppContext,
    invocation: &RunInvocation,
    resolved: &ResolvedWorkflow,
    env: &BTreeMap<String, String>,
) -> Result<i32> {
    if !resolved.path.exists() {
        return Err(CiError::NotFound(resolved.path.display().to_string()));
    }
    if fs::metadata(&resolved.path)?.permissions().mode() & 0o111 == 0 {
        return Err(CiError::NotExecutable(resolved.path.clone()));
    }

    let mut command = Command::new(&resolved.path);
    command
        .current_dir(resolve_workdir(
            &ctx.repo.root,
            resolved.execution.workspace.as_deref(),
        ))
        .args(&invocation.hook_args)
        .envs(env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    Ok(command.status()?.code().unwrap_or(1))
}

fn run_native_yaml_containerized(
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

fn run_native_yaml(
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
    let empty_matrix = BTreeMap::new();
    let empty_inputs = BTreeMap::new();
    for step in steps {
        let step_name = step
            .name
            .as_deref()
            .or(step.uses.as_deref())
            .unwrap_or("run");

        let condition_env = merged_env(base_env, &resolved.env, &step.env);
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
        if !evaluate_condition(step.if_condition.as_deref(), &expr) {
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
            if let Some(container) = container {
                container.backend.run_shell(&ContainerShellSpec {
                    image: &container.image,
                    repo_root: &ctx.repo.root,
                    shell,
                    script: &script,
                    env: &condition_env,
                    workdir: &workdir,
                    platform: Some(&container.platform),
                    options: None,
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

fn native_container_enabled(resolved: &ResolvedWorkflow) -> bool {
    resolved.container.kind.is_some()
        || resolved.container.image.is_some()
        || resolved.container.platform.is_some()
        || !resolved.container.arch.is_empty()
        || !resolved.container.packages.is_empty()
        || !resolved.container.components.is_empty()
}

fn prepare_native_container_image(
    ctx: &AppContext,
    backend: &ContainerBackend,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
    platform: &str,
) -> Result<PreparedNativeContainerImage> {
    let base_image = native_container_base_image(ctx, resolved, steps);
    validate_container_image_ref(&base_image)?;
    if resolved.container.packages.is_empty() && resolved.container.components.is_empty() {
        return Ok(PreparedNativeContainerImage {
            image: base_image,
            build_status: 0,
        });
    }

    if !resolved.container.components.is_empty()
        && native_container_effective_type(ctx, resolved, steps) == ContainerType::General
    {
        return Err(CiError::Usage(
            "container.components is only supported for Rust containers".to_string(),
        ));
    }

    validate_container_packages(&resolved.container.packages)?;
    let components = normalized_rust_components(&resolved.container.components)?;
    let tag = format!(
        "ci-{}-{}",
        sanitize_component(&resolved.name),
        sanitize_component(platform)
    );
    let dir = ctx.repo.state_dir.join("containers");
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{tag}.Containerfile"));
    fs::write(
        &file,
        generated_native_containerfile(&base_image, &resolved.container.packages, &components),
    )?;
    let build_status = backend.build(&file, &dir, &tag, Some(platform))?;
    Ok(PreparedNativeContainerImage {
        image: tag,
        build_status,
    })
}

fn native_container_base_image(
    ctx: &AppContext,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
) -> String {
    if let Some(image) = &resolved.container.image {
        return image.clone();
    }

    match native_container_effective_type(ctx, resolved, steps) {
        ContainerType::Rust => "docker.io/library/rust:latest".to_string(),
        ContainerType::General => "docker.io/library/debian:stable-slim".to_string(),
        ContainerType::Auto => unreachable!("container type is resolved before selecting image"),
    }
}

fn native_container_effective_type(
    ctx: &AppContext,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
) -> ContainerType {
    match resolved.container.kind.unwrap_or(ContainerType::Auto) {
        ContainerType::Auto
            if !resolved.container.components.is_empty()
                || native_workflow_looks_like_rust(ctx, steps) =>
        {
            ContainerType::Rust
        }
        ContainerType::Auto => ContainerType::General,
        kind => kind,
    }
}

fn native_workflow_looks_like_rust(ctx: &AppContext, steps: &[NativeStep]) -> bool {
    ctx.repo.root.join("Cargo.toml").exists()
        || steps.iter().any(|step| {
            step.run
                .as_deref()
                .map(|run| run.split_whitespace().any(|part| part == "cargo"))
                .unwrap_or(false)
        })
}

fn normalized_rust_components(components: &[String]) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for component in components {
        let value = component.trim();
        if value.is_empty() {
            return Err(CiError::Usage(
                "container component names must not be empty".to_string(),
            ));
        }
        if value.contains('\0') || value.contains('\n') || value.contains('\r') {
            return Err(CiError::Usage(format!(
                "container component `{component}` contains unsupported control characters"
            )));
        }

        let component = match value {
            "cargo-fmt" => "rustfmt",
            "cargo-clippy" => "clippy",
            other => other,
        };
        if !normalized.iter().any(|item| item == component) {
            normalized.push(component.to_string());
        }
    }
    Ok(normalized)
}

fn validate_container_packages(packages: &[String]) -> Result<()> {
    for package in packages {
        if package.trim().is_empty() {
            return Err(CiError::Usage(
                "container package names must not be empty".to_string(),
            ));
        }
        if package.contains('\0') || package.contains('\n') || package.contains('\r') {
            return Err(CiError::Usage(format!(
                "container package `{package}` contains unsupported control characters"
            )));
        }
    }
    Ok(())
}

fn validate_container_image_ref(image: &str) -> Result<()> {
    if image.trim().is_empty() {
        return Err(CiError::Usage(
            "container image must not be empty".to_string(),
        ));
    }
    if image.contains('\0') || image.chars().any(char::is_whitespace) {
        return Err(CiError::Usage(
            "container image contains unsupported whitespace or control characters".to_string(),
        ));
    }
    Ok(())
}

fn generated_native_containerfile(
    base_image: &str,
    packages: &[String],
    components: &[String],
) -> String {
    let mut content = format!("FROM {base_image}\n");

    if !components.is_empty() {
        let components = components
            .iter()
            .map(|component| sh_single_quote(component))
            .collect::<Vec<_>>()
            .join(" ");
        content.push_str(&format!("RUN rustup component add {components}\n"));
    }

    if packages.is_empty() {
        return content;
    }

    let packages = packages
        .iter()
        .map(|package| sh_single_quote(package))
        .collect::<Vec<_>>()
        .join(" ");
    content.push_str(&format!(
        "RUN set -eux; \\\n\
             if command -v apt-get >/dev/null 2>&1; then \\\n\
                 apt-get update; \\\n\
                 DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends {packages}; \\\n\
                 rm -rf /var/lib/apt/lists/*; \\\n\
             elif command -v dnf >/dev/null 2>&1; then \\\n\
                 dnf install -y {packages}; \\\n\
                 dnf clean all; \\\n\
             elif command -v apk >/dev/null 2>&1; then \\\n\
                 apk add --no-cache {packages}; \\\n\
             elif command -v zypper >/dev/null 2>&1; then \\\n\
                 zypper --non-interactive install {packages}; \\\n\
                 zypper clean --all; \\\n\
             else \\\n\
                 echo 'no supported package manager found in container image' >&2; \\\n\
                 exit 1; \\\n\
             fi\n"
    ));
    content
}

fn sh_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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

fn run_container_workflow(
    ctx: &AppContext,
    invocation: &RunInvocation,
    resolved: &ResolvedWorkflow,
    env: &BTreeMap<String, String>,
) -> Result<i32> {
    let backend = ContainerBackend::detect(invocation.container_runtime)?;
    let tag = format!("ci-{}", sanitize_component(&resolved.name));
    let platform = container_platform(resolved, &invocation.arch);
    let build_status = backend.build(&resolved.path, &ctx.repo.root, &tag, Some(&platform))?;
    if build_status != 0 {
        return Ok(build_status);
    }

    backend.run_shell(&ContainerShellSpec {
        image: &tag,
        repo_root: &ctx.repo.root,
        shell: &ctx.config.defaults.shell,
        script: "true",
        env,
        workdir: &ctx.repo.root,
        platform: Some(&platform),
        options: None,
    })
}

fn container_platform(resolved: &ResolvedWorkflow, arch: &Architecture) -> String {
    resolved
        .container
        .platform
        .clone()
        .unwrap_or_else(|| arch.platform())
}

fn run_actions_workflow(
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
        match step {
            ActionStep::Run(step) => {
                let exit_code = run_actions_run_step(ctx, execution, step, status)?;
                success = exit_code == 0;
                previous_failed = exit_code != 0;
                if exit_code != 0 && !step.continue_on_error {
                    return Ok(exit_code);
                }
            }
            ActionStep::Uses(step) => {
                let exit_code = run_actions_uses_step(ctx, execution, step, status)?;
                success = exit_code == 0;
                previous_failed = exit_code != 0;
                if exit_code != 0 && !step.continue_on_error {
                    return Ok(exit_code);
                }
            }
        }
    }
    Ok(0)
}

fn run_actions_run_step(
    ctx: &AppContext,
    execution: &ActionsJobExecution<'_>,
    step: &ActionRunStep,
    status: StepStatus,
) -> Result<i32> {
    let empty_inputs = BTreeMap::new();
    let merged = merged_env(
        &merged_env(
            execution.base_env,
            &execution.workflow.env,
            &execution.job.env,
        ),
        &execution.resolved.env,
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
        inputs: &empty_inputs,
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

    if let Some(container) = execution.job.container.as_ref() {
        let platform = container_platform(execution.resolved, execution.arch);
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
            });
    }

    let platform = container_platform(execution.resolved, execution.arch);
    if step.uses.starts_with("./") {
        let dir = ctx.repo.root.join(step.uses.trim_start_matches("./"));
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

fn run_builtin_step(
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
            ctx.git.ensure_submodules(&ctx.repo)?;
            Ok(Some(0))
        }
        "cache" | "actions/cache" => {
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
            Ok(Some(run_export_step(invocation.expr.root, &rendered_with)?))
        }
        name if COMMIT_ACTION_NAMES.contains(&name) => {
            Ok(Some(run_commit_step(ctx, &rendered_with)?))
        }
        name if SYNC_ACTION_NAMES.contains(&name) => Ok(Some(run_sync_step(ctx, &rendered_with)?)),
        "clean" | "ci/clean" => Ok(Some(run_clean_step(
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
        )?)),
        "cleanup" | "ci/cleanup" => Ok(Some(run_cleanup_step(
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
        )?)),
        _ => Ok(None),
    }
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
                })
            }
        }
    }
}

fn interpolate_map(
    values: &BTreeMap<String, String>,
    expr: &ExpressionContext<'_>,
) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), interpolate_expressions(value, expr)))
        .collect()
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
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
        let status = run_shell(shell, "cargo clean", workdir, expr.env)?;
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

fn run_export_step(root: &Path, rendered_with: &BTreeMap<String, String>) -> Result<i32> {
    let source = input_value(
        rendered_with,
        &["source", "sources", "src", "srcs", "path", "paths"],
    )
    .ok_or_else(|| CiError::Message("export requires `source` or `src`".to_string()))?;
    let destination = input_value(
        rendered_with,
        &["destination", "destenation", "dest", "dst", "target"],
    )
    .ok_or_else(|| CiError::Message("export requires `destination` or `dest`".to_string()))?;

    let specs = parse_path_list(source);
    if specs.is_empty() {
        return Err(CiError::Message(
            "export requires at least one source path".to_string(),
        ));
    }

    let replace = input_bool(rendered_with, &["replace", "overwrite"], false);
    let sources = expand_export_sources(root, &specs)?;
    let destination_path = resolve_export_path(root, destination);
    for source in &sources {
        let target = export_target_path(source, &destination_path, destination, sources.len())?;
        copy_export_path(source, &target, replace)?;
    }
    Ok(0)
}

fn run_commit_step(ctx: &AppContext, rendered_with: &BTreeMap<String, String>) -> Result<i32> {
    let paths = input_value(
        rendered_with,
        &["path", "paths", "source", "sources", "src", "srcs"],
    )
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
    let source_remote = input_value(rendered_with, &["source", "src"]).unwrap_or(remote);
    let destination_remote = input_value(
        rendered_with,
        &["destination", "destenation", "dest", "to", "target"],
    )
    .unwrap_or(remote);

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

fn expand_export_sources(root: &Path, specs: &[String]) -> Result<Vec<PathBuf>> {
    let mut sources = Vec::new();
    for spec in specs {
        let pattern_path = resolve_export_path(root, spec);
        let pattern = pattern_path.to_str().ok_or_else(|| {
            CiError::Message(format!("invalid export source {}", pattern_path.display()))
        })?;
        let before = sources.len();
        for entry in glob(pattern)? {
            let path = entry?;
            if path.exists() {
                sources.push(path);
            }
        }
        if sources.len() == before {
            return Err(CiError::Message(format!(
                "export source matched no paths: {spec}"
            )));
        }
    }
    Ok(sources)
}

fn resolve_export_path(root: &Path, value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn export_target_path(
    source: &Path,
    destination: &Path,
    raw_destination: &str,
    source_count: usize,
) -> Result<PathBuf> {
    if source_count == 1
        && !destination.is_dir()
        && !raw_destination.ends_with('/')
        && !raw_destination.ends_with(std::path::MAIN_SEPARATOR)
    {
        return Ok(destination.to_path_buf());
    }

    let name = source.file_name().ok_or_else(|| {
        CiError::Message(format!(
            "cannot export {} into a directory without a file name",
            source.display()
        ))
    })?;
    Ok(destination.join(name))
}

fn copy_export_path(source: &Path, target: &Path, replace: bool) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if replace => {
            if metadata.file_type().is_dir() {
                fs::remove_dir_all(target)?;
            } else {
                fs::remove_file(target)?;
            }
        }
        Ok(_) => {
            return Err(CiError::Message(format!(
                "export target already exists: {}; set `replace: true` or `overwrite: true`",
                target.display()
            )));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    copy_recursively(source, target)
}

fn input_value<'a>(values: &'a BTreeMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| values.get(*key).map(String::as_str))
}

fn input_bool(values: &BTreeMap<String, String>, keys: &[&str], default: bool) -> bool {
    input_value(values, keys).map(parse_bool).unwrap_or(default)
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
    ctx.output.verbose(format!("git {}", args.join(" ")));
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    ctx.git.status_in_dir(&ctx.repo.root, &args)
}

fn parse_cleanup_ignored_mode(step_name: &str, value: Option<&str>) -> Result<CleanIgnoredMode> {
    match value.map(str::trim).map(|value| value.to_ascii_lowercase()) {
        None => Ok(CleanIgnoredMode::Exclude),
        Some(value) if matches!(value.as_str(), "0" | "false" | "no" | "off") => {
            Ok(CleanIgnoredMode::Exclude)
        }
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on") => {
            Ok(CleanIgnoredMode::Include)
        }
        Some(value) if value == "only" => Ok(CleanIgnoredMode::Only),
        Some(value) => Err(CiError::Usage(format!(
            "{step_name} `ignored` must be one of `false`, `true`, or `only`; got `{value}`"
        ))),
    }
}

fn cleanup_repo_paths(
    root: &Path,
    path: Option<&str>,
    paths: Option<&str>,
    missing_ok: Option<&str>,
) -> Result<()> {
    let spec = path.or(paths).unwrap_or("");
    let missing_ok = missing_ok.map(parse_bool).unwrap_or(true);
    let targets = parse_path_list(spec);
    if targets.is_empty() {
        return Err(CiError::Message(
            "cleanup requires `with.path` or `with.paths`".to_string(),
        ));
    }

    for target in targets {
        let path = resolve_cleanup_path(root, &target)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_dir() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && missing_ok => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(CiError::Message(format!(
                    "cleanup target does not exist: {}",
                    path.display()
                )))
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(())
}

fn resolve_cleanup_path(root: &Path, value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(CiError::Message(format!(
            "cleanup only supports repo-relative paths, got {}",
            path.display()
        )));
    }

    let mut resolved = root.to_path_buf();
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(item) => {
                resolved.push(item);
                depth += 1;
            }
            Component::ParentDir => {
                if depth == 0 {
                    return Err(CiError::Message(format!(
                        "cleanup path escapes repository root: {value}"
                    )));
                }
                resolved.pop();
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(CiError::Message(format!(
                    "cleanup only supports repo-relative paths, got {value}"
                )))
            }
        }
    }

    Ok(resolved)
}

fn workflow_env(
    ctx: &AppContext,
    invocation: &RunInvocation,
    resolved: &ResolvedWorkflow,
    run_id: &str,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("CI".to_string(), "true".to_string());
    env.insert("CI_TOOL".to_string(), "ci".to_string());
    env.insert("CI_EVENT".to_string(), invocation.event.clone());
    env.insert("CI_HOOK".to_string(), invocation.event.clone());
    env.insert("CI_ARCH".to_string(), invocation.arch.to_string());
    env.insert(
        "CI_PLATFORM".to_string(),
        container_platform(resolved, &invocation.arch),
    );
    env.insert("CI_REPO".to_string(), ctx.repo.root.display().to_string());
    env.insert(
        "CI_GIT_DIR".to_string(),
        ctx.repo.git_dir.display().to_string(),
    );
    env.insert("CI_WORKFLOW".to_string(), resolved.name.clone());
    env.insert(
        "CI_WORKFLOW_PATH".to_string(),
        resolved.path.display().to_string(),
    );
    env.insert(
        "CI_WORKFLOW_DIR".to_string(),
        resolved
            .path
            .parent()
            .unwrap_or(&ctx.repo.ci_dir)
            .display()
            .to_string(),
    );
    env.insert("CI_RUN_ID".to_string(), run_id.to_string());
    env.insert(
        "CI_PROVIDER".to_string(),
        provider_name(&resolved.provider).to_string(),
    );
    env.insert("CI_HOOK_ARGS".to_string(), invocation.hook_args.join(" "));
    if let Some(branch) = invocation.branch.as_ref() {
        env.insert("CI_BRANCH".to_string(), branch.clone());
        env.insert("GITHUB_REF".to_string(), format!("refs/heads/{branch}"));
        env.insert("GITHUB_REF_NAME".to_string(), branch.clone());
        env.insert("GITEA_REF".to_string(), format!("refs/heads/{branch}"));
        env.insert("GITEA_REF_NAME".to_string(), branch.clone());
    }
    env.insert("GITHUB_ACTIONS".to_string(), "true".to_string());
    env.insert(
        "GITHUB_WORKSPACE".to_string(),
        ctx.repo.root.display().to_string(),
    );
    env.insert(
        "GITHUB_EVENT_NAME".to_string(),
        canonical_events(&invocation.event)
            .last()
            .cloned()
            .unwrap_or_else(|| invocation.event.clone()),
    );
    env.insert("GITHUB_WORKFLOW".to_string(), resolved.name.clone());
    env.insert(
        "GITEA_WORKSPACE".to_string(),
        ctx.repo.root.display().to_string(),
    );
    env.insert("GITEA_EVENT_NAME".to_string(), invocation.event.clone());
    for (key, value) in &resolved.env {
        env.insert(key.clone(), value.clone());
    }
    env
}

fn resolve_workdir(root: &Path, override_dir: Option<&Path>) -> PathBuf {
    match override_dir {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => root.join(path),
        None => root.to_path_buf(),
    }
}

fn run_shell(
    shell: &str,
    script: &str,
    workdir: &Path,
    env: &BTreeMap<String, String>,
) -> Result<i32> {
    let mut command = Command::new(shell);
    command
        .arg("-c")
        .arg(script)
        .current_dir(workdir)
        .envs(env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    Ok(command.status()?.code().unwrap_or(1))
}

fn merged_env(
    base: &BTreeMap<String, String>,
    middle: &BTreeMap<String, String>,
    top: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = base.clone();
    for (key, value) in middle {
        merged.insert(key.clone(), value.clone());
    }
    for (key, value) in top {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

fn branch_from_hook(ctx: &AppContext, hook: &str, hook_args: &[String]) -> Result<Option<String>> {
    if hook == "update" {
        return Ok(hook_args
            .first()
            .and_then(|value| value.strip_prefix("refs/heads/"))
            .map(ToOwned::to_owned)
            .or_else(|| ctx.repo.branch.clone()));
    }

    if matches!(hook, "pre-receive" | "post-receive") {
        let mut stdin = String::new();
        std::io::stdin().read_to_string(&mut stdin)?;
        for line in stdin.lines() {
            let mut parts = line.split_whitespace();
            let _old = parts.next();
            let _new = parts.next();
            if let Some(reference) = parts.next() {
                if let Some(branch) = reference.strip_prefix("refs/heads/") {
                    return Ok(Some(branch.to_string()));
                }
            }
        }
    }

    Ok(ctx.repo.branch.clone())
}

fn new_run_id() -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{stamp}-{}", std::process::id())
}

struct RunLock {
    file: fs::File,
}

impl RunLock {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive().map_err(|_| {
            CiError::Message(format!("another `ci` run is active ({})", path.display()))
        })?;
        Ok(Self { file })
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

struct ContainerShellSpec<'a> {
    image: &'a str,
    repo_root: &'a Path,
    shell: &'a str,
    script: &'a str,
    env: &'a BTreeMap<String, String>,
    workdir: &'a Path,
    platform: Option<&'a str>,
    options: Option<&'a str>,
}

struct ContainerBackend {
    runtime: String,
}

impl ContainerBackend {
    fn detect(runtime: ContainerRuntime) -> Result<Self> {
        let runtime = match runtime {
            ContainerRuntime::Podman => "podman".to_string(),
            ContainerRuntime::Docker => "docker".to_string(),
            ContainerRuntime::Auto => preferred_container_runtime(),
        };

        if !command_exists(&runtime) {
            return Err(CiError::Message(format!(
                "container runtime `{runtime}` is not available"
            )));
        }

        Ok(Self { runtime })
    }

    fn build(&self, file: &Path, context: &Path, tag: &str, platform: Option<&str>) -> Result<i32> {
        let mut command = Command::new(&self.runtime);
        if self.runtime == "docker" && platform.is_some() {
            command.arg("buildx").arg("build").arg("--load");
        } else {
            command.arg("build");
        }
        if let Some(platform) = platform {
            command.arg("--platform").arg(platform);
        }
        command
            .arg("-f")
            .arg(file)
            .arg("-t")
            .arg(tag)
            .arg(context)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        Ok(command.status()?.code().unwrap_or(1))
    }

    fn run_shell(&self, spec: &ContainerShellSpec<'_>) -> Result<i32> {
        let mount = format!("{}:/work", spec.repo_root.display());
        let container_workdir = if let Ok(relative) = spec.workdir.strip_prefix(spec.repo_root) {
            if relative.as_os_str().is_empty() {
                "/work".to_string()
            } else {
                format!("/work/{}", relative.display())
            }
        } else {
            "/work".to_string()
        };

        let mut command = Command::new(&self.runtime);
        command
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg("host")
            .arg("-v")
            .arg(mount)
            .arg("-w")
            .arg(container_workdir);
        if let Some(platform) = spec.platform {
            command.arg("--platform").arg(platform);
        }
        if let Some(options) = spec.options {
            for part in options.split_whitespace() {
                command.arg(part);
            }
        }
        for (key, value) in spec.env {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command
            .arg(spec.image)
            .arg(spec.shell)
            .arg("-c")
            .arg(spec.script)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        Ok(command.status()?.code().unwrap_or(1))
    }

    fn run_action_container(
        &self,
        image: &str,
        action_dir: &Path,
        env: &BTreeMap<String, String>,
        entrypoint: Option<&str>,
        args: &[String],
        platform: Option<&str>,
    ) -> Result<i32> {
        let mount = format!("{}:/action", action_dir.display());
        let mut command = Command::new(&self.runtime);
        command
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg("host")
            .arg("-v")
            .arg(mount)
            .arg("-w")
            .arg("/action");
        if let Some(platform) = platform {
            command.arg("--platform").arg(platform);
        }
        if let Some(entrypoint) = entrypoint {
            command.arg("--entrypoint").arg(entrypoint);
        }
        for (key, value) in env {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command.arg(image);
        for arg in args {
            command.arg(arg);
        }
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        Ok(command.status()?.code().unwrap_or(1))
    }

    fn start_service(
        &self,
        name: &str,
        service: &ActionService,
        platform: Option<&str>,
    ) -> Result<()> {
        let mut command = Command::new(&self.runtime);
        command
            .arg("run")
            .arg("-d")
            .arg("--rm")
            .arg("--name")
            .arg(name)
            .arg("--network")
            .arg("host");
        if let Some(platform) = platform {
            command.arg("--platform").arg(platform);
        }
        if let Some(options) = service.options.as_deref() {
            for part in options.split_whitespace() {
                command.arg(part);
            }
        }
        for (key, value) in &service.env {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command.arg(&service.image);
        let status = command.status()?.code().unwrap_or(1);
        if status == 0 {
            Ok(())
        } else {
            Err(CiError::Message(format!(
                "failed to start service {} from {}",
                name, service.image
            )))
        }
    }

    fn stop_container(&self, name: &str) -> Result<()> {
        let status = Command::new(&self.runtime)
            .arg("rm")
            .arg("-f")
            .arg(name)
            .status()?
            .code()
            .unwrap_or(1);
        if status == 0 {
            Ok(())
        } else {
            Err(CiError::Message(format!("failed to stop container {name}")))
        }
    }
}

fn order_jobs(jobs: &[ActionsJob]) -> Result<Vec<ActionsJob>> {
    let map: BTreeMap<_, _> = jobs
        .iter()
        .map(|job| (job.id.clone(), job.clone()))
        .collect();
    let mut ordered = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();

    for job in jobs {
        visit_job(&map, &job.id, &mut visiting, &mut visited, &mut ordered)?;
    }

    Ok(ordered)
}

fn visit_job(
    map: &BTreeMap<String, ActionsJob>,
    id: &str,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    ordered: &mut Vec<ActionsJob>,
) -> Result<()> {
    if visited.contains(id) {
        return Ok(());
    }
    if !visiting.insert(id.to_string()) {
        return Err(CiError::Message(format!(
            "cyclic job dependency involving `{id}`"
        )));
    }

    let job = map
        .get(id)
        .ok_or_else(|| CiError::Message(format!("unknown job dependency `{id}`")))?;
    for need in &job.needs {
        visit_job(map, need, visiting, visited, ordered)?;
    }

    visiting.remove(id);
    visited.insert(id.to_string());
    ordered.push(job.clone());
    Ok(())
}

fn parse_path_list(value: &str) -> Vec<String> {
    value
        .split(['\n', ','])
        .map(str::trim)
        .map(|item| item.strip_prefix("- ").unwrap_or(item).trim())
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn restore_cache(ctx: &AppContext, key: &str, paths: &[String]) -> Result<()> {
    let cache_root = ctx
        .repo
        .state_dir
        .join("cache")
        .join(sanitize_component(key));
    if !cache_root.exists() {
        return Ok(());
    }

    for path in paths {
        let target = ctx.repo.root.join(path);
        let source = cache_root.join(path);
        if source.exists() {
            copy_recursively(&source, &target)?;
        }
    }
    Ok(())
}

fn save_pending_caches(ctx: &AppContext, cache_state: &CacheState) -> Result<()> {
    for pending in &cache_state.pending {
        let cache_root = ctx
            .repo
            .state_dir
            .join("cache")
            .join(sanitize_component(&pending.key));
        fs::create_dir_all(&cache_root)?;
        for path in &pending.paths {
            let source = ctx.repo.root.join(path);
            if source.exists() {
                copy_recursively(&source, &cache_root.join(path))?;
            }
        }
    }
    Ok(())
}

fn copy_recursively(source: &Path, target: &Path) -> Result<()> {
    if source.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &target.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, target)?;
        Ok(())
    }
}

fn evaluate_condition(expr: Option<&str>, ctx: &ExpressionContext<'_>) -> bool {
    let Some(expr) = expr else {
        return ctx.success;
    };
    let expr = trim_expr(expr);

    if expr.contains("||") {
        return expr
            .split("||")
            .any(|part| evaluate_condition(Some(part), ctx));
    }
    if expr.contains("&&") {
        return expr
            .split("&&")
            .all(|part| evaluate_condition(Some(part), ctx));
    }
    if let Some(rest) = expr.strip_prefix('!') {
        return !evaluate_condition(Some(rest), ctx);
    }

    match expr {
        "true" | "always" | "always()" => return true,
        "false" | "cancelled" | "cancelled()" => return false,
        "success" | "success()" => return ctx.success,
        "failure" | "failure()" => return ctx.previous_failed,
        _ => {}
    }

    if let Some(target) = function_arg(expr, "exists") {
        return condition_target_exists(&resolve_condition_target(target, ctx), ctx);
    }
    if let Some(target) = function_arg(expr, "missing") {
        return !condition_target_exists(&resolve_condition_target(target, ctx), ctx);
    }

    if let Some(rest) = expr
        .strip_prefix("startsWith(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let mut parts = rest.splitn(2, ',');
        let left = parts.next().unwrap_or("").trim();
        let right = parts
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('\'')
            .trim_matches('"');
        return resolve_expr_value(left, ctx)
            .map(|value| value.starts_with(right))
            .unwrap_or(false);
    }

    if let Some((left, right)) = expr.split_once("==") {
        return resolve_expr_value(left.trim(), ctx)
            .map(|value| value == trim_literal(right))
            .unwrap_or(false);
    }
    if let Some((left, right)) = expr.split_once("!=") {
        return resolve_expr_value(left.trim(), ctx)
            .map(|value| value != trim_literal(right))
            .unwrap_or(false);
    }

    resolve_expr_value(expr, ctx)
        .map(|value| !value.is_empty() && value != "false")
        .unwrap_or(false)
}

fn interpolate_expressions(value: &str, ctx: &ExpressionContext<'_>) -> String {
    let mut rendered = String::new();
    let mut remaining = value;

    while let Some(start) = remaining.find("${{") {
        rendered.push_str(&remaining[..start]);
        let after = &remaining[start + 3..];
        if let Some(end) = after.find("}}") {
            let expr = after[..end].trim();
            rendered.push_str(&resolve_expr_value(expr, ctx).unwrap_or_default());
            remaining = &after[end + 2..];
        } else {
            rendered.push_str(&remaining[start..]);
            return rendered;
        }
    }

    rendered.push_str(remaining);
    rendered
}

fn resolve_expr_value(expr: &str, ctx: &ExpressionContext<'_>) -> Option<String> {
    match trim_expr(expr) {
        "github.ref" | "gitea.ref" => ctx.branch.map(|branch| format!("refs/heads/{branch}")),
        "github.ref_name" | "gitea.ref_name" => ctx.branch.map(ToOwned::to_owned),
        "github.event_name" | "gitea.event_name" => Some(
            canonical_events(ctx.event)
                .last()
                .cloned()
                .unwrap_or_else(|| ctx.event.to_string()),
        ),
        "github.workspace" | "gitea.workspace" => ctx.env.get("CI_REPO").cloned(),
        value if value.starts_with("env.") => {
            ctx.env.get(value.trim_start_matches("env.")).cloned()
        }
        value if value.starts_with("matrix.") => {
            ctx.matrix.get(value.trim_start_matches("matrix.")).cloned()
        }
        value if value.starts_with("inputs.") => {
            ctx.inputs.get(value.trim_start_matches("inputs.")).cloned()
        }
        value if ctx.inputs.contains_key(value) => ctx.inputs.get(value).cloned(),
        value => Some(trim_literal(value)),
    }
}

fn trim_expr(expr: &str) -> &str {
    let trimmed = expr.trim();
    trimmed
        .strip_prefix("${{")
        .and_then(|value| value.strip_suffix("}}"))
        .map(str::trim)
        .unwrap_or(trimmed)
}

fn trim_literal(value: &str) -> String {
    value
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string()
}

enum ConditionTarget {
    Generic(String),
    Path(String),
    Env(String),
}

fn resolve_condition_target(value: &str, ctx: &ExpressionContext<'_>) -> ConditionTarget {
    let value = trim_literal(value);
    if let Some(name) = value.strip_prefix("env:") {
        return ConditionTarget::Env(resolve_expr_value(name, ctx).unwrap_or_default());
    }
    if let Some(path) = value.strip_prefix("path:") {
        return ConditionTarget::Path(resolve_expr_value(path, ctx).unwrap_or_default());
    }
    ConditionTarget::Generic(resolve_expr_value(&value, ctx).unwrap_or_default())
}

fn function_arg<'a>(expr: &'a str, name: &str) -> Option<&'a str> {
    expr.strip_prefix(name)
        .and_then(|value| value.strip_prefix('('))
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
}

fn condition_target_exists(target: &ConditionTarget, ctx: &ExpressionContext<'_>) -> bool {
    match target {
        ConditionTarget::Generic(value) => target_exists(value, ctx.root),
        ConditionTarget::Path(value) => path_target_exists(value, ctx.root),
        ConditionTarget::Env(name) => env_target_exists(name, ctx.env),
    }
}

fn executable_exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let path = Path::new(name);
    if path.components().count() > 1 {
        return executable_file_exists(path);
    }

    env::var_os("PATH")
        .map(|value| env::split_paths(&value).any(|dir| executable_file_exists(&dir.join(name))))
        .unwrap_or(false)
}

fn target_exists(name: &str, root: &Path) -> bool {
    if name.is_empty() {
        return false;
    }

    let candidate = Path::new(name);
    if candidate.is_absolute() {
        return filesystem_entry_exists(candidate);
    }
    if candidate.components().count() > 1 || name.starts_with('.') {
        return filesystem_entry_exists(&root.join(candidate));
    }

    filesystem_entry_exists(&root.join(candidate)) || executable_exists(name)
}

fn path_target_exists(name: &str, root: &Path) -> bool {
    if name.is_empty() {
        return false;
    }

    let candidate = Path::new(name);
    if candidate.is_absolute() {
        filesystem_entry_exists(candidate)
    } else {
        filesystem_entry_exists(&root.join(candidate))
    }
}

fn env_target_exists(name: &str, env: &BTreeMap<String, String>) -> bool {
    if name.is_empty() {
        return false;
    }

    env.get(name)
        .map(|value| !value.is_empty())
        .or_else(|| std::env::var_os(name).map(|value| !value.is_empty()))
        .unwrap_or(false)
}

fn filesystem_entry_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn executable_file_exists(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn strip_action_ref(value: &str) -> &str {
    value.split('@').next().unwrap_or(value)
}

struct RemoteActionSpec {
    owner: String,
    repo: String,
    subpath: String,
    reference: String,
}

fn parse_remote_action(value: &str) -> Result<RemoteActionSpec> {
    let (repo_path, reference) = value
        .split_once('@')
        .ok_or_else(|| CiError::Message(format!("remote action `{value}` is missing `@ref`")))?;
    let mut parts = repo_path.split('/');
    let owner = parts
        .next()
        .ok_or_else(|| CiError::Message(format!("invalid action reference `{value}`")))?;
    let repo = parts
        .next()
        .ok_or_else(|| CiError::Message(format!("invalid action reference `{value}`")))?;
    let subpath = parts.collect::<Vec<_>>().join("/");
    Ok(RemoteActionSpec {
        owner: owner.to_string(),
        repo: repo.to_string(),
        subpath,
        reference: reference.to_string(),
    })
}

#[derive(Debug, Deserialize)]
struct ActionMetadata {
    runs: RawActionRuns,
}

#[derive(Debug, Deserialize)]
struct RawActionRuns {
    using: String,
    main: Option<String>,
    image: Option<String>,
    dockerfile: Option<String>,
    entrypoint: Option<String>,
    args: Option<Vec<String>>,
    #[serde(default)]
    steps: Vec<RawActionMetadataStep>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawActionMetadataStep {
    name: Option<String>,
    #[serde(rename = "if")]
    if_condition: Option<String>,
    run: Option<String>,
    uses: Option<String>,
    shell: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(rename = "working-directory")]
    working_directory: Option<String>,
    #[serde(rename = "continue-on-error", default)]
    continue_on_error: bool,
}

#[derive(Clone, Debug)]
enum ActionMetadataStep {
    Run(ActionRunStep),
    Uses,
}

enum ActionRuns {
    Composite {
        steps: Vec<ActionMetadataStep>,
    },
    Docker {
        image: Option<String>,
        dockerfile: Option<String>,
        entrypoint: Option<String>,
        args: Option<Vec<String>>,
    },
    Node {
        main: String,
    },
}

fn load_action_metadata(dir: &Path) -> Result<ActionDefinition> {
    for name in ["action.yml", "action.yaml"] {
        let path = dir.join(name);
        if path.exists() {
            let ActionMetadata { runs } = serde_yaml::from_str(&fs::read_to_string(path)?)?;
            let RawActionRuns {
                using,
                main,
                image,
                dockerfile,
                entrypoint,
                args,
                steps: raw_steps,
            } = runs;
            let steps = raw_steps
                .into_iter()
                .enumerate()
                .map(|(index, step)| {
                    let name = step.name.clone().unwrap_or_else(|| {
                        step.uses
                            .clone()
                            .unwrap_or_else(|| format!("step-{}", index + 1))
                    });
                    match (step.run, step.uses) {
                        (Some(run), None) => Ok(ActionMetadataStep::Run(ActionRunStep {
                            name,
                            run,
                            shell: step.shell,
                            env: step.env,
                            if_condition: step.if_condition,
                            working_directory: step.working_directory,
                            continue_on_error: step.continue_on_error,
                            timeout_minutes: None,
                        })),
                        (None, Some(_uses)) => Ok(ActionMetadataStep::Uses),
                        _ => Err(CiError::Message(format!(
                            "action {} has a composite step without exactly one of `run` or `uses`",
                            dir.display()
                        ))),
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            let runs = match using.as_str() {
                "composite" => ActionRuns::Composite { steps },
                "docker" => ActionRuns::Docker {
                    image,
                    dockerfile,
                    entrypoint,
                    args,
                },
                value if value.starts_with("node") => ActionRuns::Node {
                    main: main.ok_or_else(|| {
                        CiError::Message(format!(
                            "node action {} is missing runs.main",
                            dir.display()
                        ))
                    })?,
                },
                other => {
                    return Err(CiError::Message(format!(
                        "action {} uses unsupported runner `{other}`",
                        dir.display()
                    )))
                }
            };
            return Ok(ActionDefinition { runs });
        }
    }

    Err(CiError::Message(format!(
        "no action.yml or action.yaml found in {}",
        dir.display()
    )))
}

struct ActionDefinition {
    runs: ActionRuns,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use tempfile::TempDir;

    use crate::git::CleanIgnoredMode;

    use super::{
        evaluate_condition, executable_exists, generated_native_containerfile,
        normalized_rust_components, parse_cleanup_ignored_mode, parse_path_list, run_export_step,
        ExpressionContext,
    };

    fn expr_ctx<'a>(
        root: &'a Path,
        env: &'a BTreeMap<String, String>,
        success: bool,
        previous_failed: bool,
    ) -> ExpressionContext<'a> {
        let empty = Box::leak(Box::new(BTreeMap::new()));
        ExpressionContext {
            event: "manual",
            branch: None,
            root,
            env,
            matrix: empty,
            inputs: empty,
            success,
            previous_failed,
        }
    }

    fn expr_ctx_with_inputs<'a>(
        root: &'a Path,
        env: &'a BTreeMap<String, String>,
        inputs: &'a BTreeMap<String, String>,
    ) -> ExpressionContext<'a> {
        let empty = Box::leak(Box::new(BTreeMap::new()));
        ExpressionContext {
            event: "manual",
            branch: None,
            root,
            env,
            matrix: empty,
            inputs,
            success: true,
            previous_failed: false,
        }
    }

    #[test]
    fn native_condition_defaults_to_success() {
        let temp = TempDir::new().expect("tempdir");
        let env = BTreeMap::new();
        assert!(evaluate_condition(
            None,
            &expr_ctx(temp.path(), &env, true, false)
        ));
        assert!(!evaluate_condition(
            None,
            &expr_ctx(temp.path(), &env, false, true)
        ));
    }

    #[test]
    fn native_condition_shorthands_work() {
        let temp = TempDir::new().expect("tempdir");
        let env = BTreeMap::new();
        let failed = expr_ctx(temp.path(), &env, false, true);
        let succeeded = expr_ctx(temp.path(), &env, true, false);

        assert!(evaluate_condition(Some("always"), &failed));
        assert!(evaluate_condition(Some("failure"), &failed));
        assert!(!evaluate_condition(Some("success"), &failed));
        assert!(evaluate_condition(Some("success"), &succeeded));
        assert!(!evaluate_condition(Some("failure"), &succeeded));
        assert!(evaluate_condition(Some("!failure"), &succeeded));
    }

    #[test]
    fn exists_conditions_work_with_explicit_paths() {
        let temp = TempDir::new().expect("tempdir");
        let tool = temp.path().join("tool");
        fs::write(&tool, "#!/bin/sh\nexit 0\n").expect("write tool");

        let mut permissions = fs::metadata(&tool).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&tool, permissions).expect("chmod");

        let tool = tool.display().to_string();
        let env = BTreeMap::new();
        let condition = format!("exists('{tool}')");
        let missing = format!("missing('{tool}-missing')");

        assert!(evaluate_condition(
            Some(&condition),
            &expr_ctx(temp.path(), &env, true, false)
        ));
        assert!(evaluate_condition(
            Some(&missing),
            &expr_ctx(temp.path(), &env, true, false)
        ));
        assert!(executable_exists(&tool));
        assert!(!executable_exists(&format!("{tool}-missing")));
    }

    #[test]
    fn exists_checks_repo_relative_files_and_env_paths() {
        let temp = TempDir::new().expect("tempdir");
        fs::create_dir_all(temp.path().join("target")).expect("create dir");
        fs::write(temp.path().join("marker.txt"), "ok").expect("write file");

        let mut env = BTreeMap::new();
        env.insert("BUILD_DIR".to_string(), "target".to_string());
        env.insert("HOME".to_string(), "/tmp/test-home".to_string());

        let ctx = expr_ctx(temp.path(), &env, true, false);
        assert!(evaluate_condition(Some("exists(target)"), &ctx));
        assert!(evaluate_condition(Some("exists(env.BUILD_DIR)"), &ctx));
        assert!(evaluate_condition(Some("exists(marker.txt)"), &ctx));
        assert!(evaluate_condition(Some("exists(path:marker.txt)"), &ctx));
        assert!(evaluate_condition(Some("exists(path:env.BUILD_DIR)"), &ctx));
        assert!(evaluate_condition(Some("exists(env:HOME)"), &ctx));
        assert!(evaluate_condition(
            Some("missing(env:NOT_SET_FOR_TEST)"),
            &ctx
        ));
        assert!(evaluate_condition(Some("missing(dist)"), &ctx));
    }

    #[test]
    fn exists_conditions_can_reference_action_inputs() {
        let temp = TempDir::new().expect("tempdir");
        fs::create_dir_all(temp.path().join("target/release")).expect("create dir");
        fs::write(temp.path().join("target/release/ci"), "bin").expect("write file");

        let env = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        inputs.insert("src".to_string(), "target/release/ci".to_string());

        let ctx = expr_ctx_with_inputs(temp.path(), &env, &inputs);
        assert!(evaluate_condition(Some("exists(src)"), &ctx));
        assert!(evaluate_condition(Some("exists(inputs.src)"), &ctx));
    }

    #[test]
    fn cleanup_ignored_mode_parses_supported_values() {
        assert_eq!(
            parse_cleanup_ignored_mode("cleanup", None).expect("default ignored mode"),
            CleanIgnoredMode::Exclude
        );
        assert_eq!(
            parse_cleanup_ignored_mode("cleanup", Some("true")).expect("include ignored"),
            CleanIgnoredMode::Include
        );
        assert_eq!(
            parse_cleanup_ignored_mode("cleanup", Some("only")).expect("ignored only"),
            CleanIgnoredMode::Only
        );
        assert!(parse_cleanup_ignored_mode("cleanup", Some("maybe")).is_err());
    }

    #[test]
    fn path_list_accepts_yaml_sequence_text() {
        assert_eq!(
            parse_path_list("- target/release/ci\n- dist/app.tar.gz"),
            vec!["target/release/ci", "dist/app.tar.gz"]
        );
    }

    #[test]
    fn rust_component_aliases_normalize_to_rustup_components() {
        assert_eq!(
            normalized_rust_components(&[
                "cargo-fmt".to_string(),
                "cargo-clippy".to_string(),
                "rust-src".to_string(),
                "rustfmt".to_string(),
            ])
            .expect("normalize components"),
            vec!["rustfmt", "clippy", "rust-src"]
        );
    }

    #[test]
    fn generated_containerfile_installs_components_before_packages() {
        let content = generated_native_containerfile(
            "docker.io/library/rust:latest",
            &["htop".to_string()],
            &["rustfmt".to_string(), "clippy".to_string()],
        );

        assert!(content.contains("RUN rustup component add 'rustfmt' 'clippy'"));
        assert!(content.contains("apt-get install -y --no-install-recommends 'htop'"));
        assert!(
            content
                .find("rustup component add")
                .expect("components line")
                < content.find("apt-get install").expect("package line")
        );
    }

    #[test]
    fn export_single_file_uses_exact_destination_path() {
        let temp = TempDir::new().expect("tempdir");
        fs::create_dir_all(temp.path().join("target/release")).expect("create target");
        fs::write(temp.path().join("target/release/ci"), "bin").expect("write source");

        let mut inputs = BTreeMap::new();
        inputs.insert("src".to_string(), "target/release/ci".to_string());
        inputs.insert("dest".to_string(), "dist/ci".to_string());

        assert_eq!(
            run_export_step(temp.path(), &inputs).expect("export should succeed"),
            0
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("dist/ci")).expect("read exported file"),
            "bin"
        );
    }

    #[test]
    fn export_existing_target_requires_replace_or_overwrite() {
        let temp = TempDir::new().expect("tempdir");
        fs::create_dir_all(temp.path().join("target/release")).expect("create target");
        fs::create_dir_all(temp.path().join("dist")).expect("create dist");
        fs::write(temp.path().join("target/release/ci"), "new").expect("write source");
        fs::write(temp.path().join("dist/ci"), "old").expect("write existing");

        let mut inputs = BTreeMap::new();
        inputs.insert("src".to_string(), "target/release/ci".to_string());
        inputs.insert("dest".to_string(), "dist/ci".to_string());

        let err = run_export_step(temp.path(), &inputs).expect_err("export should fail");
        assert!(err
            .to_string()
            .contains("set `replace: true` or `overwrite: true`"));
        assert_eq!(
            fs::read_to_string(temp.path().join("dist/ci")).expect("read existing file"),
            "old"
        );

        inputs.insert("overwrite".to_string(), "true".to_string());
        assert_eq!(
            run_export_step(temp.path(), &inputs).expect("export should overwrite"),
            0
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("dist/ci")).expect("read overwritten file"),
            "new"
        );
    }

    #[test]
    fn export_multiple_sources_uses_destination_as_directory() {
        let temp = TempDir::new().expect("tempdir");
        fs::write(temp.path().join("one.txt"), "one").expect("write source one");
        fs::write(temp.path().join("two.txt"), "two").expect("write source two");

        let mut inputs = BTreeMap::new();
        inputs.insert("source".to_string(), "one.txt\ntwo.txt".to_string());
        inputs.insert("destination".to_string(), "out".to_string());

        assert_eq!(
            run_export_step(temp.path(), &inputs).expect("export should succeed"),
            0
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("out/one.txt")).expect("read one"),
            "one"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("out/two.txt")).expect("read two"),
            "two"
        );
    }
}
