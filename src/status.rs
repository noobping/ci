use std::fs::OpenOptions;

use fs2::FileExt;

use crate::artifacts::load_manifests;
use crate::cli::{ExplainArgs, StatusArgs};
use crate::config::format_arches;
use crate::git::{command_exists, preferred_container_runtime};
use crate::install::{inspect_installation, BinaryState};
use crate::runner::AppContext;
use crate::workflow::{self, provider_name, WorkflowSource};

pub fn cmd_status(ctx: &AppContext, _args: &StatusArgs) -> crate::error::Result<i32> {
    let workflows = workflow::discover_all(&ctx.repo)?;
    let install = inspect_installation(&ctx.repo, &ctx.config.defaults.arch);
    let manifests = load_manifests(&ctx.repo.runs_dir)?;
    let lock_path = ctx.repo.state_dir.join("lock");

    println!("Repository: {}", ctx.repo.root.display());
    println!("Git dir:    {}", ctx.repo.git_dir.display());
    println!("CI dir:     {}", ctx.repo.ci_dir.display());
    println!("Bare repo:  {}", ctx.repo.is_bare);
    println!("Git mode:   {:?}", ctx.git.mode());
    println!("Arch:       {}", format_arches(&ctx.config.defaults.arch));
    println!(
        "Config:     {}",
        if ctx.config.loaded {
            ctx.config.path.display().to_string()
        } else {
            format!("{} (default)", ctx.config.path.display())
        }
    );
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

    println!(
        "{}   preferred container runtime: {}",
        if command_exists("podman") || command_exists("docker") {
            "OK"
        } else {
            "WARN"
        },
        preferred_container_runtime()
    );
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
    let workflows = workflow::discover_all(&ctx.repo)?;
    for line in workflow::explain_subject(
        &workflows,
        &ctx.config,
        &args.subject,
        ctx.repo.branch.as_deref(),
    ) {
        println!("{line}");
    }
    Ok(0)
}
