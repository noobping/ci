use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_yaml::Value;
use walkdir::WalkDir;

use crate::actions::{self, ActionsProvider};
use crate::config::WorkflowOverride;
use crate::error::Result;
use crate::repo::RepoInfo;
use crate::workflow::{
    ContainerWorkflow, ExecutableWorkflow, NativeWorkflow, Workflow, WorkflowKind,
    WorkflowProvider, WorkflowSource,
};

use super::native::NativeWorkflowFile;
use super::validation::validate_native_workflow_keys;

pub fn discover_all(repo: &RepoInfo, include_other_workflows: bool) -> Result<Vec<Workflow>> {
    let mut workflows = Vec::new();
    discover_native(repo, &mut workflows)?;
    if include_other_workflows {
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
    }
    workflows.sort_by(|left, right| left.name.cmp(&right.name).then(left.path.cmp(&right.path)));
    Ok(workflows)
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
            needs: Vec::new(),
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
    let needs = file.needs.to_vec();
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
        needs,
        source: WorkflowSource::NativeYaml(NativeWorkflow { metadata, steps }),
    })
}

fn discover_executable_workflow(base: &Path, path: &Path) -> Result<Workflow> {
    let metadata = load_directory_metadata(path.parent())?;
    Ok(Workflow {
        name: workflow_name(base, path, &WorkflowKind::Executable),
        path: path.to_path_buf(),
        kind: WorkflowKind::Executable,
        provider: WorkflowProvider::Native,
        needs: metadata.needs,
        source: WorkflowSource::Executable(ExecutableWorkflow {
            metadata: metadata.metadata,
        }),
    })
}

fn discover_container_workflow(base: &Path, path: &Path) -> Result<Workflow> {
    let metadata = load_directory_metadata(path.parent())?;
    Ok(Workflow {
        name: workflow_name(base, path, &WorkflowKind::Container),
        path: path.to_path_buf(),
        kind: WorkflowKind::Container,
        provider: WorkflowProvider::Native,
        needs: metadata.needs,
        source: WorkflowSource::Container(ContainerWorkflow {
            metadata: metadata.metadata,
        }),
    })
}

struct DirectoryMetadata {
    metadata: WorkflowOverride,
    needs: Vec<String>,
}

fn load_directory_metadata(dir: Option<&Path>) -> Result<DirectoryMetadata> {
    let Some(dir) = dir else {
        return Ok(DirectoryMetadata {
            metadata: WorkflowOverride::default(),
            needs: Vec::new(),
        });
    };
    for file_name in ["workflow.yml", "workflow.yaml"] {
        let path = dir.join(file_name);
        if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let value: Value = serde_yaml::from_str(&raw)?;
            validate_native_workflow_keys(&value, &path)?;
            let file: NativeWorkflowFile = serde_yaml::from_str(&raw)?;
            return Ok(DirectoryMetadata {
                metadata: file.metadata(),
                needs: file.needs.to_vec(),
            });
        }
    }
    Ok(DirectoryMetadata {
        metadata: WorkflowOverride::default(),
        needs: Vec::new(),
    })
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
