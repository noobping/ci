use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::path::Path;

use fs2::FileExt;

use crate::artifacts::load_manifests;
use crate::cli::{ExplainArgs, StatusArgs};
use crate::conditions::{evaluate_condition, interpolate_expressions, ExpressionContext};
use crate::config::{format_arches, Architecture, GitMode};
use crate::containers::{container_platform, ContainerBackend};
use crate::git::command_exists;
use crate::install::{inspect_installation, BinaryState};
use crate::runner::{available_workflows, AppContext};
use crate::workflow::{self, provider_name, select_workflows, WorkflowSource};

pub fn cmd_status(ctx: &AppContext, _args: &StatusArgs) -> crate::error::Result<i32> {
    let workflows = available_workflows(ctx)?;
    let install = inspect_installation(&ctx.repo, &ctx.config.defaults.arch);
    let manifests = load_manifests(&ctx.repo.runs_dir)?;
    let lock_path = ctx.repo.state_dir.join("lock");

    println!("Repository: {}", ctx.repo.root.display());
    println!("Git dir:    {}", ctx.repo.git_dir.display());
    println!("CI dir:     {}", ctx.repo.ci_dir.display());
    println!("Bare repo:  {}", ctx.repo.is_bare);
    println!("Git mode:   {:?}", ctx.git.mode());
    println!("Arch:       {}", format_arches(&ctx.config.defaults.arch));
    if ctx.config.loaded {
        println!(
            "Config:     {}",
            ctx.config
                .paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else {
        println!("Config:     {} (default)", ctx.config.path.display());
    }
    println!();

    if ctx.repo.ci_dir.exists() {
        println!("OK   .ci directory exists");
    } else {
        println!("WARN .ci directory does not exist");
    }

    println!("OK   found {} workflow(s)", workflows.len());
    for workflow in &workflows {
        let details = match &workflow.source {
            WorkflowSource::Actions(action) => format!(
                "events: {:?}",
                action
                    .events
                    .iter()
                    .map(|event| event.name.clone())
                    .collect::<Vec<_>>()
            ),
            _ => String::new(),
        };
        println!(
            "     - {} [{}] {} {}",
            workflow.name,
            provider_name(&workflow.provider),
            workflow.path.display(),
            details
        );
    }

    for binary in install.binaries {
        match binary {
            BinaryState::Missing(path) => println!(
                "WARN ci binary is not installed into this repository ({})",
                path.display()
            ),
            BinaryState::Copy { path } => println!("OK   ci copy installed at {}", path.display()),
            BinaryState::Symlink {
                path,
                target,
                broken,
            } => {
                if broken {
                    println!(
                        "WARN ci symlink {} -> {} is broken",
                        path.display(),
                        target.display()
                    );
                } else {
                    println!("OK   ci symlink {} -> {}", path.display(), target.display());
                }
            }
        }
    }

    let managed_hooks: Vec<_> = install
        .hooks
        .iter()
        .filter(|hook| hook.managed)
        .map(|hook| hook.name.clone())
        .collect();
    if managed_hooks.is_empty() {
        println!("WARN no ci-managed Git hooks installed");
    } else {
        println!("OK   ci-managed hooks: {}", managed_hooks.join(", "));
    }

    let preferred_container_runtime = ContainerBackend::preferred_runtime_label();
    println!(
        "{}   preferred container runtime: {}",
        if preferred_container_runtime.is_some() {
            "OK"
        } else {
            "WARN"
        },
        preferred_container_runtime.unwrap_or_else(|| "podman or docker".to_string())
    );
    for arch in &ctx.config.defaults.arch {
        if arch == &Architecture::host() {
            println!("OK   container arch {arch}: host architecture");
        } else if binfmt_available(arch) {
            println!("OK   container arch {arch}: binfmt handler found");
        } else {
            println!(
                "WARN container arch {arch}: non-host architecture may need binfmt/qemu support"
            );
        }
    }
    println!(
        "{}   host git {}",
        if command_exists("git") { "OK" } else { "WARN" },
        if command_exists("git") {
            "available"
        } else {
            "missing"
        }
    );
    println!(
        "OK   git mode: {}",
        format!("{:?}", ctx.git.mode()).to_ascii_lowercase()
    );
    if let Some(command) = ctx.git.command() {
        println!("OK   git command: {}", command.render());
    } else if matches!(ctx.git.mode(), GitMode::Flatpak) {
        println!("OK   git command: flatpak-spawn --host git");
    }
    println!(
        "{}   node {}",
        if command_exists("node") { "OK" } else { "WARN" },
        if command_exists("node") {
            "available"
        } else {
            "missing"
        }
    );

    println!("OK   actions cache: {}", ctx.repo.actions_cache.display());
    println!("OK   artifact store: {}", ctx.repo.artifact_store.display());
    println!("OK   recorded runs: {}", manifests.len());

    let lock_status = if lock_path.exists() {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        match file.try_lock_exclusive() {
            Ok(()) => {
                let _ = file.unlock();
                "idle"
            }
            Err(_) => "busy",
        }
    } else {
        "not-created"
    };
    println!("OK   lock file: {} ({lock_status})", lock_path.display());

    Ok(0)
}

pub fn cmd_explain(ctx: &AppContext, args: &ExplainArgs) -> crate::error::Result<i32> {
    let workflows = available_workflows(ctx)?;
    let matches = select_workflows(
        &workflows,
        &ctx.config,
        Some(&args.subject),
        "manual",
        ctx.repo.branch.as_deref(),
        false,
    );
    if matches.is_empty() {
        for line in workflow::explain_subject(
            &workflows,
            &ctx.config,
            &args.subject,
            ctx.repo.branch.as_deref(),
        ) {
            println!("{line}");
        }
        return Ok(0);
    }

    println!(
        "Precedence: CLI flags > workflow fields > workflow defaults > project config > user config > system config > auto-detect; policy/locked applies last with system policy strongest"
    );
    for item in matches {
        let container_arches = item.resolved.container.arch.to_vec();
        let arches = if !ctx.global.arch.is_empty() || container_arches.is_empty() {
            ctx.config.defaults.arch.clone()
        } else {
            container_arches
        };
        println!(
            "{} [{}] at {}",
            item.workflow.name,
            provider_name(&item.workflow.provider),
            item.workflow.path.display()
        );
        println!("  selected because: {}", item.reasons.join("; "));
        println!("  arches: {}", format_arches(&arches));
        if let Some(kind) = item.resolved.container.kind {
            println!("  container type: {}", kind.as_name());
        }
        if let Some(image) = &item.resolved.container.image {
            println!("  container image: {image}");
        }
        for arch in &arches {
            println!(
                "  platform({arch}): {}",
                container_platform(&item.resolved, arch)
            );
        }
        explain_native_steps(ctx, &item.resolved, &item.workflow.source, &arches);
    }
    Ok(0)
}

fn explain_native_steps(
    ctx: &AppContext,
    resolved: &crate::workflow::ResolvedWorkflow,
    source: &WorkflowSource,
    arches: &[Architecture],
) {
    let WorkflowSource::NativeYaml(native) = source else {
        return;
    };
    let empty = BTreeMap::new();
    for arch in arches {
        let mut env = BTreeMap::new();
        env.insert("CI".to_string(), "true".to_string());
        env.insert("CI_EVENT".to_string(), "manual".to_string());
        env.insert("CI_ARCH".to_string(), arch.to_string());
        env.insert("CI_HOST_ARCH".to_string(), Architecture::host().to_string());
        env.insert("CI_REPO".to_string(), ctx.repo.root.display().to_string());
        if let Some(branch) = ctx.repo.branch.as_ref() {
            env.insert("CI_BRANCH".to_string(), branch.clone());
        }
        for (key, value) in &resolved.env {
            env.insert(key.clone(), value.clone());
        }
        for (key, value) in &resolved.container.env {
            env.insert(key.clone(), value.clone());
        }
        let mut previous_failed = false;
        let mut success = true;
        println!("  steps({arch}):");
        for step in &native.steps {
            let mut inputs = step.extra.clone();
            inputs.extend(step.with.clone());
            let expr = ExpressionContext {
                event: "manual",
                branch: ctx.repo.branch.as_deref(),
                root: &ctx.repo.root,
                env: &env,
                matrix: &empty,
                inputs: &inputs,
                success,
                previous_failed,
            };
            let name = step
                .name
                .as_deref()
                .or(step.uses.as_deref())
                .unwrap_or("run");
            let should_run = evaluate_condition(step.if_condition.as_deref(), &expr);
            if should_run {
                println!("    OK   {name}");
            } else {
                println!(
                    "    SKIP {name}: if `{}` is false",
                    step.if_condition.as_deref().unwrap_or("success()")
                );
            }
            if let Some(run) = step.run.as_deref() {
                println!("         run: {}", interpolate_expressions(run, &expr));
            }
            previous_failed = false;
            success = true;
        }
    }
}

fn binfmt_available(arch: &Architecture) -> bool {
    let handler = match arch.as_str() {
        "arm64" => "qemu-aarch64",
        "x64" => "qemu-x86_64",
        other => other,
    };
    let path = Path::new("/proc/sys/fs/binfmt_misc").join(handler);
    fs::read_to_string(path)
        .map(|value| value.contains("enabled"))
        .unwrap_or(false)
}
