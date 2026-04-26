use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Architecture;
use crate::containers::container_platform;
use crate::error::Result;
use crate::runner::{AppContext, RunInvocation};
use crate::workflow::{canonical_events, provider_name, ResolvedWorkflow};

pub(crate) fn workflow_env(
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
    env.insert("CI_HOST_ARCH".to_string(), Architecture::host().to_string());
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
    env.insert(
        "CI_WORKFLOW_ARGS".to_string(),
        invocation.workflow_args.join(" "),
    );
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

pub(crate) fn resolve_workdir(root: &Path, override_dir: Option<&Path>) -> PathBuf {
    match override_dir {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => root.join(path),
        None => root.to_path_buf(),
    }
}

pub(crate) fn run_shell(
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

pub(crate) fn merged_env(
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

pub(crate) fn container_step_env(
    base: &BTreeMap<String, String>,
    resolved: &ResolvedWorkflow,
) -> BTreeMap<String, String> {
    merged_env(base, &resolved.container.env, &BTreeMap::new())
}

pub(crate) fn branch_from_hook(
    ctx: &AppContext,
    hook: &str,
    hook_args: &[String],
) -> Result<Option<String>> {
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
