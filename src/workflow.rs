use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use walkdir::WalkDir;

use crate::actions::{self, ActionsProvider, ActionsWorkflow};
use crate::config::{
    ArtifactConfig, BranchConfig, ContainerConfig, EventFilter, ExecutionConfig, ResolvedConfig,
    WorkflowOverride,
};
use crate::error::{CiError, Result};
use crate::repo::RepoInfo;

pub const CLIENT_HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
    "post-index-change",
];

pub const SERVER_HOOKS: &[&str] = &[
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowKind {
    Executable,
    NativeYaml,
    Container,
    GitHubActions,
    GiteaActions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowProvider {
    Native,
    GitHubActions,
    GiteaActions,
}

#[derive(Clone, Debug)]
pub struct Workflow {
    pub name: String,
    pub path: PathBuf,
    pub kind: WorkflowKind,
    pub provider: WorkflowProvider,
    pub source: WorkflowSource,
}

#[derive(Clone, Debug)]
pub enum WorkflowSource {
    Executable(ExecutableWorkflow),
    NativeYaml(NativeWorkflow),
    Container(ContainerWorkflow),
    Actions(ActionsWorkflow),
}

#[derive(Clone, Debug)]
pub struct ExecutableWorkflow {
    pub metadata: WorkflowOverride,
}

#[derive(Clone, Debug)]
pub struct NativeWorkflow {
    pub metadata: WorkflowOverride,
    pub steps: Vec<NativeStep>,
}

#[derive(Clone, Debug)]
pub struct ContainerWorkflow {
    pub metadata: WorkflowOverride,
}

#[derive(Clone, Debug, Deserialize)]
pub struct NativeStep {
    pub name: Option<String>,
    pub run: String,
    pub shell: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(rename = "if")]
    pub if_condition: Option<String>,
    #[serde(rename = "working-directory")]
    pub working_directory: Option<String>,
    #[serde(rename = "continue-on-error", default)]
    pub continue_on_error: bool,
    #[serde(rename = "timeout-minutes")]
    pub timeout_minutes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ResolvedWorkflow {
    pub name: String,
    pub path: PathBuf,
    pub kind: WorkflowKind,
    pub provider: WorkflowProvider,
    pub source: WorkflowSource,
    pub events: Vec<String>,
    pub branches: BranchConfig,
    pub artifacts: ArtifactConfig,
    pub execution: ExecutionConfig,
    pub container: ContainerConfig,
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct WorkflowMatch {
    pub workflow: Workflow,
    pub resolved: ResolvedWorkflow,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct NativeWorkflowFile {
    name: Option<String>,
    #[serde(default, rename = "on")]
    on: EventFilter,
    #[serde(default)]
    branches: BranchConfig,
    #[serde(default)]
    artifacts: ArtifactConfig,
    #[serde(default)]
    execution: ExecutionConfig,
    #[serde(default)]
    container: ContainerConfig,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    steps: Vec<NativeStep>,
}

impl NativeWorkflowFile {
    fn metadata(&self) -> WorkflowOverride {
        WorkflowOverride {
            on: self.on.clone(),
            branches: self.branches.clone(),
            artifacts: self.artifacts.clone(),
            execution: self.execution.clone(),
            container: self.container.clone(),
            env: self.env.clone(),
        }
    }
}

pub fn is_known_hook(name: &str) -> bool {
    CLIENT_HOOKS.contains(&name) || SERVER_HOOKS.contains(&name)
}

pub fn all_hooks() -> Vec<&'static str> {
    let mut hooks = CLIENT_HOOKS.to_vec();
    for hook in SERVER_HOOKS {
        if !hooks.contains(hook) {
            hooks.push(hook);
        }
    }
    hooks
}

pub fn discover_all(repo: &RepoInfo) -> Result<Vec<Workflow>> {
    let mut workflows = Vec::new();
    discover_native(repo, &mut workflows)?;
    discover_actions_dir(
        &repo.root.join(".github").join("workflows"),
        ActionsProvider::GitHub,
        &mut workflows,
    )?;
    discover_actions_dir(
        &repo.root.join(".gitea").join("workflows"),
        ActionsProvider::Gitea,
        &mut workflows,
    )?;
    workflows.sort_by(|left, right| left.name.cmp(&right.name).then(left.path.cmp(&right.path)));
    Ok(workflows)
}

pub fn canonical_events(event: &str) -> Vec<String> {
    let mut result = vec![event.to_string()];
    if event == "manual" {
        result.push("workflow_dispatch".to_string());
    }
    if matches!(event, "pre-push" | "pre-receive" | "post-receive" | "update") {
        result.push("push".to_string());
    }
    result
}

pub fn resolve_workflow(workflow: &Workflow, config: &ResolvedConfig, event: &str) -> ResolvedWorkflow {
    let local = workflow.local_override();
    let merged = config
        .hook_override(event)
        .merge(&config.workflow_override(&workflow.name))
        .merge(&local);

    ResolvedWorkflow {
        name: workflow.name.clone(),
        path: workflow.path.clone(),
        kind: workflow.kind.clone(),
        provider: workflow.provider.clone(),
        source: workflow.source.clone(),
        events: merged.on.to_vec(),
        branches: merged.branches,
        artifacts: merged.artifacts,
        execution: merged.execution,
        container: merged.container,
        env: merged.env,
    }
}

pub fn select_workflows(
    workflows: &[Workflow],
    config: &ResolvedConfig,
    requested_name: Option<&str>,
    event: &str,
    branch: Option<&str>,
    respect_branches: bool,
) -> Vec<WorkflowMatch> {
    let canonical = canonical_events(event);
    let automation = event != "manual" || respect_branches;

    workflows
        .iter()
        .filter_map(|workflow| {
            let resolved = resolve_workflow(workflow, config, event);

            if let Some(name) = requested_name {
                if workflow.name != name {
                    return None;
                }
                if automation && !branch_allowed(config, &resolved.branches, branch) {
                    return None;
                }
                return Some(WorkflowMatch {
                    workflow: workflow.clone(),
                    resolved,
                    reasons: vec![format!("selected explicitly as `{name}`")],
                });
            }

            let mut reasons = Vec::new();
            let mut matched = false;

            match &workflow.source {
                WorkflowSource::Actions(action) => {
                    if let Some(reason) = action.matches_event(&canonical, branch) {
                        reasons.push(reason);
                        matched = true;
                    }
                }
                _ if event == "manual" => {
                    reasons.push("manual run selects native workflows".to_string());
                    matched = true;
                }
                _ => {
                    if workflow.name == event || workflow.name.ends_with(&format!("/{event}")) {
                        reasons.push(format!("workflow name matches `{event}`"));
                        matched = true;
                    }

                    if !matched
                        && resolved
                            .events
                            .iter()
                            .any(|item| item == event || item == "all")
                    {
                        reasons.push(format!("workflow `on` includes `{event}`"));
                        matched = true;
                    }
                }
            }

            if !matched {
                return None;
            }

            if automation && !branch_allowed(config, &resolved.branches, branch) {
                return None;
            }

            Some(WorkflowMatch {
                workflow: workflow.clone(),
                resolved,
                reasons,
            })
        })
        .collect()
}

pub fn explain_subject(
    workflows: &[Workflow],
    config: &ResolvedConfig,
    subject: &str,
    branch: Option<&str>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let by_name: Vec<_> = workflows.iter().filter(|workflow| workflow.name == subject).collect();

    if !by_name.is_empty() {
        for workflow in by_name {
            let resolved = resolve_workflow(workflow, config, "manual");
            lines.push(format!(
                "{} [{}] at {}",
                workflow.name,
                provider_name(&workflow.provider),
                workflow.path.display()
            ));
            match &workflow.source {
                WorkflowSource::Actions(action) => {
                    let events = action
                        .events
                        .iter()
                        .map(|event| {
                            if event.branches.is_empty() {
                                event.name.clone()
                            } else {
                                format!("{} on {:?}", event.name, event.branches)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(format!("  events: {events}"));
                }
                _ => {
                    let events = if resolved.events.is_empty() {
                        "manual + filename matching".to_string()
                    } else {
                        resolved.events.join(", ")
                    };
                    lines.push(format!("  events: {events}"));
                }
            }
            let branches = resolved.branches.effective(&config.defaults);
            lines.push(format!("  branches: {:?}", branches));
        }
        return lines;
    }

    let matches = select_workflows(workflows, config, None, subject, branch, true);
    if matches.is_empty() {
        lines.push(format!("No workflows matched `{subject}`"));
        return lines;
    }

    lines.push(format!("Matched workflows for `{subject}`:"));
    for item in matches {
        lines.push(format!(
            "- {} [{}] because {}",
            item.workflow.name,
            provider_name(&item.workflow.provider),
            item.reasons.join("; ")
        ));
    }
    lines
}

pub fn provider_name(provider: &WorkflowProvider) -> &'static str {
    match provider {
        WorkflowProvider::Native => "native",
        WorkflowProvider::GitHubActions => "github-actions",
        WorkflowProvider::GiteaActions => "gitea-actions",
    }
}

pub fn kind_name(kind: &WorkflowKind) -> &'static str {
    match kind {
        WorkflowKind::Executable => "executable",
        WorkflowKind::NativeYaml => "yaml",
        WorkflowKind::Container => "container",
        WorkflowKind::GitHubActions | WorkflowKind::GiteaActions => "actions",
    }
}

fn branch_allowed(config: &ResolvedConfig, branches: &BranchConfig, branch: Option<&str>) -> bool {
    let allowed = branches.effective(&config.defaults);
    if allowed.is_empty() {
        return true;
    }
    match branch {
        Some(branch) => allowed.iter().any(|item| item == branch),
        None => true,
    }
}

fn discover_native(repo: &RepoInfo, workflows: &mut Vec<Workflow>) -> Result<()> {
    if !repo.ci_dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(&repo.ci_dir).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        if path == repo.ci_dir.join("config.yml") || path == repo.ci_dir.join("config.yaml") {
            continue;
        }

        let file_name = path.file_name().and_then(|value| value.to_str()).unwrap_or_default();
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();

        if (file_name == "workflow.yml" || file_name == "workflow.yaml") && directory_has_other_runnables(path.parent())? {
            continue;
        }

        if file_name == "Containerfile" || file_name == "Dockerfile" {
            workflows.push(discover_container_workflow(&repo.ci_dir, path)?);
            continue;
        }

        if extension == "yml" || extension == "yaml" {
            workflows.push(discover_native_yaml(&repo.ci_dir, path)?);
            continue;
        }

        if is_executable(path)? {
            workflows.push(discover_executable_workflow(&repo.ci_dir, path)?);
        }
    }

    Ok(())
}

fn discover_actions_dir(dir: &Path, provider: ActionsProvider, workflows: &mut Vec<Workflow>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(dir).max_depth(1).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
        if extension != "yml" && extension != "yaml" {
            continue;
        }

        let action = actions::load_actions_workflow(path, provider)?;
        let kind = match provider {
            ActionsProvider::GitHub => WorkflowKind::GitHubActions,
            ActionsProvider::Gitea => WorkflowKind::GiteaActions,
        };
        let provider = match provider {
            ActionsProvider::GitHub => WorkflowProvider::GitHubActions,
            ActionsProvider::Gitea => WorkflowProvider::GiteaActions,
        };
        workflows.push(Workflow {
            name: action.name.clone(),
            path: path.to_path_buf(),
            kind,
            provider,
            source: WorkflowSource::Actions(action),
        });
    }

    Ok(())
}

fn discover_native_yaml(base: &Path, path: &Path) -> Result<Workflow> {
    let file: NativeWorkflowFile = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    let metadata = file.metadata();
    Ok(Workflow {
        name: file
            .name
            .clone()
            .unwrap_or_else(|| workflow_name(base, path, &WorkflowKind::NativeYaml)),
        path: path.to_path_buf(),
        kind: WorkflowKind::NativeYaml,
        provider: WorkflowProvider::Native,
        source: WorkflowSource::NativeYaml(NativeWorkflow {
            metadata,
            steps: file.steps,
        }),
    })
}

fn discover_executable_workflow(base: &Path, path: &Path) -> Result<Workflow> {
    Ok(Workflow {
        name: workflow_name(base, path, &WorkflowKind::Executable),
        path: path.to_path_buf(),
        kind: WorkflowKind::Executable,
        provider: WorkflowProvider::Native,
        source: WorkflowSource::Executable(ExecutableWorkflow {
            metadata: load_directory_metadata(path.parent())?,
        }),
    })
}

fn discover_container_workflow(base: &Path, path: &Path) -> Result<Workflow> {
    Ok(Workflow {
        name: workflow_name(base, path, &WorkflowKind::Container),
        path: path.to_path_buf(),
        kind: WorkflowKind::Container,
        provider: WorkflowProvider::Native,
        source: WorkflowSource::Container(ContainerWorkflow {
            metadata: load_directory_metadata(path.parent())?,
        }),
    })
}

fn load_directory_metadata(dir: Option<&Path>) -> Result<WorkflowOverride> {
    let Some(dir) = dir else {
        return Ok(WorkflowOverride::default());
    };
    for file_name in ["workflow.yml", "workflow.yaml"] {
        let path = dir.join(file_name);
        if path.exists() {
            let file: NativeWorkflowFile = serde_yaml::from_str(&fs::read_to_string(path)?)?;
            return Ok(file.metadata());
        }
    }
    Ok(WorkflowOverride::default())
}

fn directory_has_other_runnables(dir: Option<&Path>) -> Result<bool> {
    let Some(dir) = dir else {
        return Ok(false);
    };

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path.file_name().and_then(|value| value.to_str()).unwrap_or_default();
        if matches!(file_name, "workflow.yml" | "workflow.yaml") {
            continue;
        }
        if matches!(file_name, "Containerfile" | "Dockerfile") || is_executable(&path)? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn workflow_name(base: &Path, path: &Path, kind: &WorkflowKind) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let parent = rel.parent().unwrap_or_else(|| Path::new(""));
    let file_stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("workflow");
    let file_name = path.file_name().and_then(|value| value.to_str()).unwrap_or(file_stem);

    let raw = match kind {
        WorkflowKind::Container => {
            if parent.as_os_str().is_empty() {
                "container".to_string()
            } else {
                path_to_name(parent)
            }
        }
        WorkflowKind::NativeYaml
            if (file_name == "workflow.yml" || file_name == "workflow.yaml")
                && !parent.as_os_str().is_empty() =>
        {
            path_to_name(parent)
        }
        _ => {
            let mut name = rel.with_file_name(file_stem);
            if name.as_os_str().is_empty() {
                name = PathBuf::from(file_stem);
            }
            path_to_name(&name)
        }
    };

    raw.trim_matches('/').to_string()
}

fn path_to_name(path: &Path) -> String {
    path.components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_executable(path: &Path) -> Result<bool> {
    let mode = fs::metadata(path)?.permissions().mode();
    Ok(mode & 0o111 != 0)
}

impl Workflow {
    fn local_override(&self) -> WorkflowOverride {
        match &self.source {
            WorkflowSource::Executable(item) => item.metadata.clone(),
            WorkflowSource::NativeYaml(item) => item.metadata.clone(),
            WorkflowSource::Container(item) => item.metadata.clone(),
            WorkflowSource::Actions(_) => WorkflowOverride::default(),
        }
    }
}
