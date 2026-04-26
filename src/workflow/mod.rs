use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::actions::ActionsWorkflow;
use crate::config::{
    ArchFilter, ArtifactConfig, BranchConfig, ContainerConfig, ExecutionConfig, ResolvedConfig,
    WorkflowOverride,
};
use crate::error::{CiError, Result};
mod discovery;
mod native;
mod validation;

pub use self::discovery::discover_all;
use self::native::is_native_inline_action_builtin;
#[cfg(test)]
pub(crate) use self::native::NativeWorkflowFile;
#[cfg(test)]
pub(crate) use self::validation::validate_native_workflow_keys;

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
    pub needs: Vec<String>,
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
    pub container_config: Option<StepContainerConfig>,
    pub readonly: Option<bool>,
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
    pub(crate) fn validate(self, workflow_path: &Path, index: usize) -> Result<Self> {
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

#[derive(Clone, Debug, Default)]
pub struct StepContainerConfig {
    pub file: Option<String>,
    pub image: Option<String>,
    pub platform: Option<String>,
    pub workdir: Option<String>,
    pub readonly: Option<bool>,
    pub env: BTreeMap<String, String>,
    pub volumes: Vec<String>,
    pub packages: Vec<String>,
    pub components: Vec<String>,
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

pub fn expand_workflow_dependencies(
    workflows: &[Workflow],
    config: &ResolvedConfig,
    event: &str,
    selected: Vec<WorkflowMatch>,
) -> Result<Vec<WorkflowMatch>> {
    let mut expander = DependencyExpander {
        workflows,
        config,
        event,
        visiting: Vec::new(),
        completed: BTreeSet::new(),
        ordered: Vec::new(),
    };

    for item in selected {
        expander.add(item.workflow.clone(), Some(item), None)?;
    }

    Ok(expander.ordered)
}

struct DependencyExpander<'a> {
    workflows: &'a [Workflow],
    config: &'a ResolvedConfig,
    event: &'a str,
    visiting: Vec<String>,
    completed: BTreeSet<String>,
    ordered: Vec<WorkflowMatch>,
}

impl DependencyExpander<'_> {
    fn add(
        &mut self,
        workflow: Workflow,
        selected: Option<WorkflowMatch>,
        required_by: Option<&str>,
    ) -> Result<()> {
        let key = workflow_key(&workflow);
        if self.completed.contains(&key) {
            return Ok(());
        }

        if let Some(index) = self.visiting.iter().position(|item| item == &key) {
            let mut cycle = self.visiting[index..].to_vec();
            cycle.push(key);
            return Err(CiError::Usage(format!(
                "workflow dependency cycle: {}",
                cycle.join(" -> ")
            )));
        }

        self.visiting.push(key.clone());
        for need in &workflow.needs {
            let dependency = workflow_dependency(self.workflows, &workflow, need)?;
            self.add(dependency, None, Some(&workflow.name))?;
        }
        self.visiting.pop();

        self.completed.insert(key);
        let item = selected.unwrap_or_else(|| WorkflowMatch {
            resolved: resolve_workflow(&workflow, self.config, self.event),
            workflow,
            reasons: vec![format!(
                "required by `{}`",
                required_by.unwrap_or("selected workflow")
            )],
        });
        self.ordered.push(item);
        Ok(())
    }
}

fn workflow_dependency(
    workflows: &[Workflow],
    dependent: &Workflow,
    need: &str,
) -> Result<Workflow> {
    let matches = workflows
        .iter()
        .filter(|workflow| workflow.name == need)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(CiError::Usage(format!(
            "workflow `{}` needs missing workflow `{need}`",
            dependent.name
        ))),
        [workflow] => Ok((*workflow).clone()),
        _ => Err(CiError::Usage(format!(
            "workflow `{}` needs ambiguous workflow `{need}`",
            dependent.name
        ))),
    }
}

fn workflow_key(workflow: &Workflow) -> String {
    format!("{}:{}", workflow.name, workflow.path.display())
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
mod tests;
