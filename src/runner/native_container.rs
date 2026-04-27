use std::fs;
use std::path::PathBuf;

use crate::config::ContainerType;
use crate::containers::{
    generated_native_container_image_name, generated_native_containerfile,
    normalized_rust_components, validate_container_image_ref, validate_container_packages,
    ContainerBackend,
};
use crate::defaults::{default_build_stack, detect_default_build_stack};
use crate::error::{CiError, Result};
use crate::git::sanitize_component;
use crate::runner::{AppContext, ContainerOverride};
use crate::workflow::{NativeStep, ResolvedWorkflow};

pub(crate) struct NativeContainerExecution<'a> {
    pub(crate) backend: &'a ContainerBackend,
    pub(crate) image: String,
    pub(crate) platform: String,
}

pub(crate) struct PreparedNativeContainerImage {
    pub(crate) image: String,
    pub(crate) build_status: i32,
}

pub(crate) fn native_container_enabled(
    resolved: &ResolvedWorkflow,
    override_mode: ContainerOverride,
) -> bool {
    match override_mode {
        ContainerOverride::Force => true,
        ContainerOverride::Disable => false,
        ContainerOverride::Auto => {
            resolved.container.kind.is_some()
                || resolved.container.image.is_some()
                || resolved.container.platform.is_some()
                || resolved.container.readonly.is_some()
                || !resolved.container.arch.is_empty()
                || !resolved.container.packages.is_empty()
                || !resolved.container.components.is_empty()
        }
    }
}

pub(crate) fn prepare_native_container_image(
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
        && native_container_effective_type(ctx, resolved, steps) != ContainerType::Rust
    {
        return Err(CiError::Usage(
            "container.components is only supported for Rust containers".to_string(),
        ));
    }

    validate_container_packages(&resolved.container.packages)?;
    let components = normalized_rust_components(&resolved.container.components)?;
    let image_name = generated_native_container_image_name(&resolved.name, platform);
    let file_stem = format!(
        "ci-{}-{}",
        sanitize_component(&resolved.name),
        sanitize_component(platform)
    );
    let dir = ctx.repo.state_dir.join("containers");
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{file_stem}.Containerfile"));
    fs::write(
        &file,
        generated_native_containerfile(&base_image, &resolved.container.packages, &components),
    )?;
    ctx.output.verbose_at(
        2,
        format!(
            "building workflow `{}` package container from {}",
            resolved.name,
            file.display()
        ),
    );
    ctx.output.verbose_at(
        3,
        format!(
            "workflow `{}` package container base `{base_image}` tagged `{image_name}`",
            resolved.name
        ),
    );
    let build_status = backend.build(&file, &dir, &image_name, Some(platform))?;
    Ok(PreparedNativeContainerImage {
        image: image_name,
        build_status,
    })
}

pub(crate) fn native_container_base_image(
    ctx: &AppContext,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
) -> String {
    if let Some(image) = &resolved.container.image {
        return image.clone();
    }

    match native_container_effective_type(ctx, resolved, steps) {
        ContainerType::Rust => "docker.io/library/rust:latest".to_string(),
        ContainerType::Node => "docker.io/library/node:22-bookworm-slim".to_string(),
        ContainerType::Go => "docker.io/library/golang:latest".to_string(),
        ContainerType::Python => "docker.io/library/python:3".to_string(),
        ContainerType::Maven => "docker.io/library/maven:latest".to_string(),
        ContainerType::Gradle => "docker.io/library/gradle:latest".to_string(),
        ContainerType::Dotnet => "mcr.microsoft.com/dotnet/sdk:latest".to_string(),
        ContainerType::General => "docker.io/library/debian:stable-slim".to_string(),
        ContainerType::Auto => unreachable!("container type is resolved before selecting image"),
    }
}

pub(crate) fn native_container_effective_type(
    ctx: &AppContext,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
) -> ContainerType {
    match ctx
        .global
        .tech_stack
        .or(ctx.config.global_tech_stack)
        .or(resolved.container.kind)
        .unwrap_or(ContainerType::Auto)
    {
        ContainerType::Auto
            if !resolved.container.components.is_empty()
                || native_workflow_looks_like_stack(ctx, steps, ContainerType::Rust) =>
        {
            ContainerType::Rust
        }
        ContainerType::Auto => {
            detect_native_workflow_stack(ctx, steps).unwrap_or(ContainerType::General)
        }
        kind => kind,
    }
}

pub(crate) fn native_container_cache_mounts(
    ctx: &AppContext,
    resolved: &ResolvedWorkflow,
    steps: &[NativeStep],
) -> Result<Vec<(PathBuf, String)>> {
    let stack = native_container_effective_type(ctx, resolved, steps);
    let root = ctx
        .repo
        .state_dir
        .join("container-cache")
        .join(sanitize_component(&resolved.name))
        .join(sanitize_component(stack.as_name()));
    let targets: &[&str] = match stack {
        ContainerType::Rust => &["/usr/local/cargo/registry", "/usr/local/cargo/git"],
        ContainerType::Node => &[
            "/root/.npm",
            "/root/.cache/pnpm",
            "/usr/local/share/.cache/yarn",
        ],
        ContainerType::Go => &["/go/pkg/mod", "/root/.cache/go-build"],
        ContainerType::Python => &["/root/.cache/pip", "/root/.cache/uv"],
        ContainerType::Maven => &["/root/.m2"],
        ContainerType::Gradle => &["/home/gradle/.gradle", "/root/.gradle"],
        ContainerType::Dotnet => &["/root/.nuget/packages"],
        ContainerType::Auto | ContainerType::General => &[],
    };

    let mut mounts = Vec::new();
    for target in targets {
        let path = root.join(sanitize_component(target.trim_start_matches('/')));
        fs::create_dir_all(&path)?;
        mounts.push((path, (*target).to_string()));
    }
    Ok(mounts)
}

fn detect_native_workflow_stack(ctx: &AppContext, steps: &[NativeStep]) -> Option<ContainerType> {
    detect_default_build_stack(&ctx.repo.root)
        .map(|stack| stack.container_type)
        .or_else(|| {
            [
                ContainerType::Rust,
                ContainerType::Node,
                ContainerType::Go,
                ContainerType::Maven,
                ContainerType::Gradle,
                ContainerType::Dotnet,
                ContainerType::Python,
            ]
            .into_iter()
            .find(|stack| native_workflow_looks_like_stack(ctx, steps, *stack))
        })
}

fn native_workflow_looks_like_stack(
    ctx: &AppContext,
    steps: &[NativeStep],
    stack: ContainerType,
) -> bool {
    default_build_stack(&ctx.repo.root, Some(stack)).is_some_and(|detected| {
        detect_default_build_stack(&ctx.repo.root)
            .map(|repo_stack| repo_stack.container_type == stack)
            .unwrap_or(false)
            || steps.iter().any(|step| {
                step.run
                    .as_deref()
                    .map(|run| command_mentions_tool(run, &detected.host_tool))
                    .unwrap_or(false)
            })
    })
}

fn command_mentions_tool(run: &str, tool: &str) -> bool {
    run.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/')))
        .any(|part| {
            let name = part.rsplit('/').next().unwrap_or(part);
            name == tool
                || (tool == "npm" && matches!(name, "node" | "npm" | "npx" | "yarn" | "pnpm"))
                || (tool == "python3" && matches!(name, "python" | "python3" | "pip" | "pip3"))
                || (tool == "gradle" && matches!(name, "gradle" | "gradlew"))
                || (tool == "java" && matches!(name, "gradle" | "gradlew" | "java"))
        })
}
