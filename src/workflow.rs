use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_yaml::Value;
use walkdir::WalkDir;

use crate::actions::{self, ActionsProvider, ActionsWorkflow};
use crate::config::{
    ArchFilter, ArtifactConfig, BranchConfig, ContainerConfig, EventFilter, ExecutionConfig,
    ResolvedConfig, WorkflowOverride,
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

#[derive(Clone, Debug)]
pub struct NativeStep {
    pub name: Option<String>,
    pub run: Option<String>,
    pub uses: Option<String>,
    pub container: Option<bool>,
    pub with: BTreeMap<String, String>,
    pub extra: BTreeMap<String, String>,
    pub shell: Option<String>,
    pub env: BTreeMap<String, String>,
    pub if_condition: Option<String>,
    pub working_directory: Option<String>,
    pub continue_on_error: bool,
    pub timeout_minutes: Option<u64>,
}

impl NativeStep {
    fn validate(self, workflow_path: &Path, index: usize) -> Result<Self> {
        if self.run.is_some()
            && self
                .uses
                .as_deref()
                .map(is_native_inline_action_builtin)
                .unwrap_or(false)
        {
            return Ok(self);
        }

        match (self.run.is_some(), self.uses.is_some()) {
            (true, false) | (false, true) => Ok(self),
            _ => Err(crate::error::CiError::Message(format!(
                "{} step {} must define exactly one of `run` or `use`",
                workflow_path.display(),
                index + 1
            ))),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedWorkflow {
    pub name: String,
    pub path: PathBuf,
    pub kind: WorkflowKind,
    pub provider: WorkflowProvider,
    pub source: WorkflowSource,
    pub events: Vec<String>,
    pub arch: ArchFilter,
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
    #[serde(default)]
    defaults: WorkflowOverride,
    #[serde(default, rename = "on")]
    on: EventFilter,
    #[serde(
        default,
        rename = "tech",
        alias = "type",
        alias = "tech-stack",
        alias = "tech_stack"
    )]
    tech_stack: Option<crate::config::ContainerType>,
    #[serde(default)]
    arch: ArchFilter,
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
    steps: Vec<RawNativeStep>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawNativeStep {
    name: Option<String>,
    run: Option<String>,
    #[serde(rename = "use")]
    use_value: Option<String>,
    uses: Option<String>,
    container: Option<bool>,
    shell: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, Value>,
    #[serde(default)]
    with: BTreeMap<String, Value>,
    #[serde(default, flatten)]
    extra: BTreeMap<String, Value>,
    #[serde(rename = "if")]
    if_condition: Option<String>,
    #[serde(rename = "working-directory")]
    working_directory: Option<String>,
    #[serde(rename = "continue-on-error", default)]
    continue_on_error: bool,
    #[serde(rename = "timeout-minutes")]
    timeout_minutes: Option<u64>,
}

impl RawNativeStep {
    fn into_step(self, workflow_path: &Path, index: usize) -> Result<NativeStep> {
        let RawNativeStep {
            name,
            run,
            use_value,
            uses,
            container,
            shell,
            env,
            with,
            extra,
            if_condition,
            working_directory,
            continue_on_error,
            timeout_minutes,
        } = self;
        let source_count = usize::from(use_value.is_some()) + usize::from(uses.is_some());
        if source_count > 1 {
            return Err(crate::error::CiError::Message(format!(
                "{} step {} must define only one of `use` or `uses`",
                workflow_path.display(),
                index + 1
            )));
        }
        let uses = use_value.or(uses);

        NativeStep {
            name,
            run,
            uses,
            container,
            with: stringify_yaml_map(with),
            extra: stringify_yaml_map(extra),
            shell,
            env: stringify_yaml_map(env),
            if_condition,
            working_directory,
            continue_on_error,
            timeout_minutes,
        }
        .validate(workflow_path, index)
    }
}

impl NativeWorkflowFile {
    fn metadata(&self) -> WorkflowOverride {
        let local = WorkflowOverride {
            on: self.on.clone(),
            tech_stack: self.tech_stack,
            arch: self.arch.clone(),
            branches: self.branches.clone(),
            artifacts: self.artifacts.clone(),
            execution: self.execution.clone(),
            container: self.container.clone(),
            env: self.env.clone(),
        };
        self.defaults.merge(&local)
    }
}

fn is_native_inline_action_builtin(uses: &str) -> bool {
    let normalized = uses
        .split('@')
        .next()
        .unwrap_or(uses)
        .trim()
        .to_ascii_lowercase();
    matches!(normalized.as_str(), "clean" | "ci/clean")
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
    if matches!(
        event,
        "pre-push" | "pre-receive" | "post-receive" | "update"
    ) {
        result.push("push".to_string());
    }
    result
}

pub fn resolve_workflow(
    workflow: &Workflow,
    config: &ResolvedConfig,
    event: &str,
) -> ResolvedWorkflow {
    let local = workflow.local_override();
    let merged = config
        .hook_override(event)
        .merge(&config.workflow_override(&workflow.name))
        .merge(&local);

    let mut merged_container = merged.container.clone();
    if merged_container.kind.is_none() {
        merged_container.kind = merged.tech_stack;
    }
    let mut container = config.defaults.container.merge(&merged_container);
    if let Some(tech_stack) = config.global_tech_stack {
        container.kind = Some(tech_stack);
    }

    ResolvedWorkflow {
        name: workflow.name.clone(),
        path: workflow.path.clone(),
        kind: workflow.kind.clone(),
        provider: workflow.provider.clone(),
        source: workflow.source.clone(),
        events: merged.on.to_vec(),
        arch: merged.arch,
        branches: merged.branches,
        artifacts: merged.artifacts,
        execution: merged.execution,
        container,
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
    let by_name: Vec<_> = workflows
        .iter()
        .filter(|workflow| workflow.name == subject)
        .collect();

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
            let container_arch = resolved.container.arch.to_vec();
            if !container_arch.is_empty() {
                lines.push(format!(
                    "  container arch: {:?}",
                    container_arch
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                ));
            }
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

        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();

        if (file_name == "workflow.yml" || file_name == "workflow.yaml")
            && directory_has_other_runnables(path.parent())?
        {
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

fn discover_actions_dir(
    dir: &Path,
    provider: ActionsProvider,
    workflows: &mut Vec<Workflow>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(dir).max_depth(1).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
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
    let raw = fs::read_to_string(path)?;
    let value: Value = serde_yaml::from_str(&raw)?;
    validate_native_workflow_keys(&value, path)?;
    let file: NativeWorkflowFile = serde_yaml::from_str(&raw)?;
    let metadata = file.metadata();
    let steps = file
        .steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| step.into_step(path, index))
        .collect::<Result<Vec<_>>>()?;
    Ok(Workflow {
        name: file
            .name
            .clone()
            .unwrap_or_else(|| workflow_name(base, path, &WorkflowKind::NativeYaml)),
        path: path.to_path_buf(),
        kind: WorkflowKind::NativeYaml,
        provider: WorkflowProvider::Native,
        source: WorkflowSource::NativeYaml(NativeWorkflow { metadata, steps }),
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
            let raw = fs::read_to_string(&path)?;
            let value: Value = serde_yaml::from_str(&raw)?;
            validate_native_workflow_keys(&value, &path)?;
            let file: NativeWorkflowFile = serde_yaml::from_str(&raw)?;
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
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
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
    let file_stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("workflow");
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(file_stem);

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

fn validate_native_workflow_keys(value: &Value, path: &Path) -> Result<()> {
    validate_mapping(value, path, "workflow", NATIVE_WORKFLOW_KEYS)?;
    for (key, child) in mapping_entries(value, path, "workflow")? {
        match key.as_str() {
            "defaults" => validate_workflow_override_section(child, path, "defaults")?,
            "branches" => validate_mapping(child, path, "branches", BRANCH_KEYS)?,
            "artifacts" => validate_mapping(child, path, "artifacts", ARTIFACT_KEYS)?,
            "execution" => validate_mapping(child, path, "execution", EXECUTION_KEYS)?,
            "container" => validate_mapping(child, path, "container", CONTAINER_KEYS)?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_workflow_override_section(value: &Value, path: &Path, label: &str) -> Result<()> {
    validate_mapping(value, path, label, WORKFLOW_OVERRIDE_KEYS)?;
    for (key, child) in mapping_entries(value, path, label)? {
        match key.as_str() {
            "branches" => validate_mapping(child, path, &format!("{label}.branches"), BRANCH_KEYS)?,
            "artifacts" => {
                validate_mapping(child, path, &format!("{label}.artifacts"), ARTIFACT_KEYS)?
            }
            "execution" => {
                validate_mapping(child, path, &format!("{label}.execution"), EXECUTION_KEYS)?
            }
            "container" => {
                validate_mapping(child, path, &format!("{label}.container"), CONTAINER_KEYS)?
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_mapping(value: &Value, path: &Path, label: &str, allowed: &[&str]) -> Result<()> {
    for (key, _) in mapping_entries(value, path, label)? {
        if !allowed.contains(&key.as_str()) {
            return Err(CiError::Usage(format!(
                "{} has unknown key `{}` in {}; run `ci schema workflow` for supported fields",
                path.display(),
                key,
                label
            )));
        }
    }
    Ok(())
}

fn mapping_entries<'a>(
    value: &'a Value,
    path: &Path,
    label: &str,
) -> Result<Vec<(String, &'a Value)>> {
    let Some(mapping) = value.as_mapping() else {
        return Err(CiError::Usage(format!(
            "{} section `{label}` must be a mapping",
            path.display()
        )));
    };
    mapping
        .iter()
        .map(|(key, value)| {
            key.as_str()
                .map(|key| (key.to_string(), value))
                .ok_or_else(|| {
                    CiError::Usage(format!(
                        "{} section `{label}` contains a non-string key",
                        path.display()
                    ))
                })
        })
        .collect()
}

const NATIVE_WORKFLOW_KEYS: &[&str] = &[
    "name",
    "defaults",
    "on",
    "tech",
    "type",
    "tech-stack",
    "tech_stack",
    "arch",
    "branches",
    "artifacts",
    "execution",
    "container",
    "env",
    "steps",
];

const WORKFLOW_OVERRIDE_KEYS: &[&str] = &[
    "on",
    "tech",
    "type",
    "tech-stack",
    "tech_stack",
    "arch",
    "branches",
    "artifacts",
    "execution",
    "container",
    "env",
];

const CONTAINER_KEYS: &[&str] = &[
    "type",
    "image",
    "platform",
    "workdir",
    "working-directory",
    "working_directory",
    "arch",
    "packages",
    "components",
    "env",
    "volumes",
];

const BRANCH_KEYS: &[&str] = &["allow", "only"];
const ARTIFACT_KEYS: &[&str] = &["paths", "mode", "destination"];
const EXECUTION_KEYS: &[&str] = &["workspace", "shell"];

fn stringify_yaml_map(map: BTreeMap<String, Value>) -> BTreeMap<String, String> {
    map.into_iter()
        .map(|(key, value)| (key, stringify_yaml_value(&value)))
        .collect()
}

fn stringify_yaml_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_yaml::Value;

    use super::{validate_native_workflow_keys, NativeWorkflowFile};

    #[test]
    fn native_clean_step_accepts_inline_run_and_top_level_options() {
        let file: NativeWorkflowFile = serde_yaml::from_str(
            r#"
steps:
  - name: Fresh clean
    use: clean
    cargo: true
    purge: true
    ignored: only
    run: cargo sweep -i
"#,
        )
        .expect("parse workflow");
        let step = file
            .steps
            .into_iter()
            .next()
            .expect("step")
            .into_step(Path::new(".ci/build.yml"), 0)
            .expect("valid step");

        assert_eq!(step.name.as_deref(), Some("Fresh clean"));
        assert_eq!(step.uses.as_deref(), Some("clean"));
        assert_eq!(step.run.as_deref(), Some("cargo sweep -i"));
        assert_eq!(step.extra.get("cargo").map(String::as_str), Some("true"));
        assert_eq!(step.extra.get("purge").map(String::as_str), Some("true"));
        assert_eq!(step.extra.get("ignored").map(String::as_str), Some("only"));
    }

    #[test]
    fn native_non_clean_action_rejects_run_with_use() {
        let file: NativeWorkflowFile = serde_yaml::from_str(
            r#"
steps:
  - use: checkout
    run: git status
"#,
        )
        .expect("parse workflow");
        let err = file
            .steps
            .into_iter()
            .next()
            .expect("step")
            .into_step(Path::new(".ci/build.yml"), 0)
            .expect_err("step should be rejected");

        assert!(err
            .to_string()
            .contains("must define exactly one of `run` or `use`"));
    }

    #[test]
    fn native_step_rejects_multiple_action_source_aliases() {
        let file: NativeWorkflowFile = serde_yaml::from_str(
            r#"
steps:
  - use: clean
    uses: checkout
"#,
        )
        .expect("parse workflow");
        let err = file
            .steps
            .into_iter()
            .next()
            .expect("step")
            .into_step(Path::new(".ci/build.yml"), 0)
            .expect_err("step should be rejected");

        assert!(err
            .to_string()
            .contains("must define only one of `use` or `uses`"));
    }

    #[test]
    fn native_step_accepts_container_override() {
        let file: NativeWorkflowFile = serde_yaml::from_str(
            r#"
steps:
  - name: Install locally
    container: false
    run: ci completion bash
"#,
        )
        .expect("parse workflow");
        let step = file
            .steps
            .into_iter()
            .next()
            .expect("step")
            .into_step(Path::new(".ci/build.yml"), 0)
            .expect("valid step");

        assert_eq!(step.container, Some(false));
    }

    #[test]
    fn native_workflow_parses_container_arch_aliases() {
        let file: NativeWorkflowFile = serde_yaml::from_str(
            r#"
container:
  arch:
    - amd64
    - aarch64
steps:
  - run: echo ok
"#,
        )
        .expect("parse workflow");
        let arch = file
            .container
            .arch
            .to_vec()
            .into_iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();

        assert_eq!(arch, vec!["x64", "arm64"]);
    }

    #[test]
    fn native_workflow_accepts_top_level_tech_stack_aliases() {
        let file: NativeWorkflowFile = serde_yaml::from_str(
            r#"
tech-stack: node
steps:
  - run: npm run build
"#,
        )
        .expect("parse workflow");
        let metadata = file.metadata();

        assert_eq!(
            metadata.tech_stack,
            Some(crate::config::ContainerType::Node)
        );
    }

    #[test]
    fn native_workflow_defaults_merge_under_direct_fields() {
        let file: NativeWorkflowFile = serde_yaml::from_str(
            r#"
defaults:
  container:
    type: rust
    arch: amd64
    components:
      - cargo-fmt
container:
  arch: aarch64
steps:
  - run: echo ok
"#,
        )
        .expect("parse workflow");
        let metadata = file.metadata();

        assert_eq!(
            metadata
                .container
                .arch
                .to_vec()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["arm64"]
        );
        assert_eq!(metadata.container.components, vec!["cargo-fmt"]);
    }

    #[test]
    fn native_workflow_validation_rejects_unknown_top_level_keys() {
        let value: Value = serde_yaml::from_str(
            r#"
contaner:
  type: rust
steps:
  - run: echo ok
"#,
        )
        .expect("parse yaml value");

        let err = validate_native_workflow_keys(&value, Path::new(".ci/build.yml"))
            .expect_err("unknown key should be rejected");

        assert!(err.to_string().contains("unknown key `contaner`"));
    }
}
