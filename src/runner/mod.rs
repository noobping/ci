use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::artifacts::ArtifactSession;
use crate::cli::{GlobalOptions, HookArgs, InitArgs, ListArgs, OtherArgs, RunArgs, SelfArgs};
use crate::config::{Architecture, ContainerRuntime, ContainerType, ResolvedConfig};
use crate::containers::{container_platform, ContainerBackend, ContainerShellSpec};
use crate::defaults::{generated_default_workflows, init_build_workflow_content};
use crate::error::{CiError, Result};
use crate::git::{command_exists, sanitize_component, GitService};
use crate::install::{inspect_installation, BinaryState};
use crate::output::Output;
use crate::repo::RepoInfo;

mod action_metadata;
mod actions;
mod builtins;
mod cache;
mod cleanup;
mod env;
mod file_actions;
mod file_system;
mod inputs;
mod jobs;
mod lock;
mod native_container;
mod native_workflow;

use self::actions::run_actions_workflow;
#[cfg(test)]
pub(crate) use self::cleanup::parse_cleanup_ignored_mode;
use self::env::{branch_from_hook, container_step_env, resolve_workdir, workflow_env};
#[cfg(test)]
pub(crate) use self::file_actions::{run_export_step, run_link_step};
#[cfg(test)]
pub(crate) use self::inputs::parse_path_list;
use self::lock::{new_run_id, RunLock};
use self::native_container::native_container_enabled;
use self::native_workflow::{run_native_yaml, run_native_yaml_containerized};
use crate::workflow::{
    self, kind_name, provider_name, select_workflows, ResolvedWorkflow, Workflow, WorkflowMatch,
    WorkflowSource,
};

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
    container_override: ContainerOverride,
    respect_branches: bool,
    recursive_checkout: bool,
    lock: bool,
    hook_args: Vec<String>,
    workflow_args: Vec<String>,
    branch: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct RunInvocation {
    pub(crate) event: String,
    pub(crate) arch: Architecture,
    pub(crate) container_runtime: ContainerRuntime,
    pub(crate) container_override: ContainerOverride,
    pub(crate) hook_args: Vec<String>,
    pub(crate) workflow_args: Vec<String>,
    pub(crate) branch: Option<String>,
}

impl RunRequest {
    fn invocation_for_arch(&self, arch: Architecture) -> RunInvocation {
        RunInvocation {
            event: self.event.clone(),
            arch,
            container_runtime: self.container_runtime,
            container_override: self.container_override,
            hook_args: self.hook_args.clone(),
            workflow_args: self.workflow_args.clone(),
            branch: self.branch.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContainerOverride {
    Auto,
    Force,
    Disable,
}

fn container_override(global: &GlobalOptions) -> ContainerOverride {
    if global.no_container {
        ContainerOverride::Disable
    } else if global.container {
        ContainerOverride::Force
    } else {
        ContainerOverride::Auto
    }
}

pub fn cmd_list(ctx: &AppContext, args: &ListArgs) -> Result<i32> {
    let porcelain = args.use_porcelain(std::io::stdout().is_terminal());
    let workflows = available_workflows(ctx)?;
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
    if !args.args.is_empty() && (args.all || args.workflow.as_deref() != Some("build")) {
        return Err(CiError::Usage(
            "workflow arguments are only supported for `ci run build ...`".to_string(),
        ));
    }

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
        dry_run: args.dry_run && !args.no_dry_run,
        keep_going,
        arches: ctx.config.defaults.arch.clone(),
        arch_overridden: !ctx.global.arch.is_empty(),
        container_runtime: args
            .container_runtime
            .unwrap_or(ctx.config.defaults.container_runtime),
        container_override: container_override(&ctx.global),
        respect_branches: args.respect_branches,
        recursive_checkout: !args.no_recursive_checkout && ctx.config.defaults.recursive_checkout,
        lock: args.lock,
        hook_args: Vec::new(),
        workflow_args: args.args.clone(),
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
        container_override: container_override(&ctx.global),
        respect_branches: true,
        recursive_checkout: ctx.config.defaults.recursive_checkout,
        lock: true,
        hook_args: args.hook_args.clone(),
        workflow_args: Vec::new(),
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

    let content = init_build_workflow_content(&ctx.repo.root, default_tech_stack_override(ctx));

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

pub fn cmd_other(ctx: &AppContext, _args: &OtherArgs) -> Result<i32> {
    let current_hash = file_content_hash(&ctx.repo.current_exe)?;
    println!("ci {}", env!("CARGO_PKG_VERSION"));
    println!("repository: {}", ctx.repo.root.display());
    println!("current executable: {}", ctx.repo.current_exe.display());
    println!("current hash: {current_hash}");

    let host_arch = Architecture::host();
    let install = inspect_installation(&ctx.repo, &[host_arch]);
    let Some(binary) = install.binaries.into_iter().next() else {
        println!("status: missing");
        return Ok(0);
    };

    match binary {
        BinaryState::Missing(path) => {
            println!("installed executable: {}", path.display());
            println!("installed: missing");
            println!("status: missing");
        }
        BinaryState::Symlink {
            path,
            target,
            broken,
        } => {
            println!("installed executable: {}", path.display());
            println!("installed symlink: {}", target.display());
            if broken {
                println!("installed: broken symlink");
                println!("status: missing");
            } else {
                print_installed_hash_status(&path, &current_hash)?;
            }
        }
        BinaryState::Copy { path } => {
            println!("installed executable: {}", path.display());
            println!("installed: copy");
            print_installed_hash_status(&path, &current_hash)?;
        }
    }
    Ok(0)
}

fn print_installed_hash_status(path: &Path, current_hash: &str) -> Result<()> {
    let installed_hash = file_content_hash(path)?;
    println!("installed hash: {installed_hash}");
    if installed_hash == current_hash {
        println!("status: same");
    } else {
        println!("status: update-needed");
    }
    Ok(())
}

fn file_content_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = 0xcbf29ce484222325u64;
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("{hash:016x}"))
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

    let workflows = available_workflows(ctx)?;
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
    let matches =
        workflow::expand_workflow_dependencies(&workflows, &ctx.config, &request.event, matches)?;

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
        let show_arch = arches.len() > 1
            || (request.container_override != ContainerOverride::Disable
                && !item.resolved.container.arch.is_empty());
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

pub(crate) fn available_workflows(ctx: &AppContext) -> Result<Vec<Workflow>> {
    let workflows = workflow::discover_all(&ctx.repo, ctx.config.other_workflows)?;
    if workflows.is_empty() {
        Ok(generated_default_workflows(
            &ctx.repo.root,
            &ctx.repo.ci_dir,
            default_tech_stack_override(ctx),
            &command_exists,
        ))
    } else {
        Ok(workflows)
    }
}

fn default_tech_stack_override(ctx: &AppContext) -> Option<ContainerType> {
    ctx.global
        .tech_stack
        .or(ctx.config.global_tech_stack)
        .or(ctx.config.defaults.container.kind)
        .filter(|kind| !matches!(kind, ContainerType::Auto | ContainerType::General))
}

fn workflow_execution_arches(
    request: &RunRequest,
    resolved: &ResolvedWorkflow,
) -> Vec<Architecture> {
    if request.container_override != ContainerOverride::Disable && !request.arch_overridden {
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
    let mut invocation = invocation.clone();
    if item.resolved.name != "build" {
        invocation.workflow_args.clear();
    }

    let base_env = workflow_env(ctx, &invocation, &item.resolved, run_id);
    let status = match &item.workflow.source {
        WorkflowSource::Executable(_) => {
            run_executable(ctx, &invocation, &item.resolved, &base_env)?
        }
        WorkflowSource::NativeYaml(native) => {
            if native_container_enabled(&item.resolved, invocation.container_override) {
                run_native_yaml_containerized(
                    ctx,
                    &invocation,
                    &item.resolved,
                    &native.steps,
                    &base_env,
                    artifacts,
                )?
            } else {
                run_native_yaml(
                    ctx,
                    &invocation,
                    &item.resolved,
                    &native.steps,
                    &base_env,
                    artifacts,
                    None,
                )?
            }
        }
        WorkflowSource::Container(_) => {
            if invocation.container_override == ContainerOverride::Disable {
                return Err(CiError::Usage(format!(
                    "{} is a container workflow and cannot run with --no-container",
                    item.workflow.path.display()
                )));
            }
            run_container_workflow(ctx, &invocation, &item.resolved, &base_env)?
        }
        WorkflowSource::Actions(actions) => run_actions_workflow(
            ctx,
            &invocation,
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
        .args(&invocation.workflow_args)
        .envs(env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    Ok(command.status()?.code().unwrap_or(1))
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
        env: &container_step_env(env, resolved),
        workdir: &ctx.repo.root,
        platform: Some(&platform),
        options: None,
        extra_volumes: &resolved.container.volumes,
        cache_mounts: &[],
        container_workdir: resolved.container.workdir.as_deref(),
        readonly: resolved.container.readonly.unwrap_or(false),
    })
}

#[cfg(test)]
mod tests;
