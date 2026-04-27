use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::cli::GlobalOptions;
pub mod types;
mod validation;

pub use self::types::{
    format_arches, ArchFilter, Architecture, ArtifactConfig, ArtifactMode, ColorWhen,
    ContainerRuntime, ContainerType, EventFilter, GitCommand, GitMode, InstallMode,
};
use self::validation::validate_config_keys;
use crate::error::Result;
use crate::repo::RepoInfo;

const DEFAULT_BRANCHES: &[&str] = &["main", "master", "develop", "development"];
const DEFAULT_GIT_IMAGE: &str = "docker.io/alpine/git:latest";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BranchConfig {
    #[serde(default)]
    pub allow: Vec<String>,

    #[serde(default)]
    pub only: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ExecutionConfig {
    pub workspace: Option<PathBuf>,
    pub shell: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ContainerConfig {
    #[serde(rename = "type")]
    pub kind: Option<ContainerType>,
    pub image: Option<String>,
    pub platform: Option<String>,
    #[serde(alias = "working-directory", alias = "working_directory")]
    pub workdir: Option<String>,
    #[serde(alias = "read-only", alias = "read_only")]
    pub readonly: Option<bool>,
    #[serde(default)]
    pub arch: ArchFilter,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub volumes: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ActionsConfig {
    #[serde(alias = "node-image")]
    pub node_image: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WorkflowOverride {
    #[serde(default, rename = "on")]
    pub on: EventFilter,

    #[serde(
        default,
        rename = "tech",
        alias = "type",
        alias = "tech-stack",
        alias = "tech_stack"
    )]
    pub tech_stack: Option<ContainerType>,

    #[serde(default)]
    pub arch: ArchFilter,

    #[serde(default)]
    pub branches: BranchConfig,

    #[serde(default)]
    pub artifacts: ArtifactConfig,

    #[serde(default)]
    pub execution: ExecutionConfig,

    #[serde(default)]
    pub container: ContainerConfig,

    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DefaultsConfig {
    pub shell: Option<String>,
    pub quiet: Option<bool>,
    #[serde(alias = "fail-fast")]
    pub fail_fast: Option<bool>,
    #[serde(
        default,
        rename = "tech",
        alias = "type",
        alias = "tech-stack",
        alias = "tech_stack"
    )]
    pub tech_stack: Option<ContainerType>,
    #[serde(default)]
    pub arch: ArchFilter,
    #[serde(default)]
    pub container: ContainerConfig,
    #[serde(alias = "container-runtime")]
    pub container_runtime: Option<ContainerRuntime>,
    #[serde(alias = "git-mode")]
    pub git_mode: Option<GitMode>,
    #[serde(alias = "git-command")]
    pub git_command: Option<GitCommand>,
    #[serde(alias = "git-image")]
    pub git_image: Option<String>,
    #[serde(alias = "install-mode")]
    pub install_mode: Option<InstallMode>,
    #[serde(alias = "recursive-checkout")]
    pub recursive_checkout: Option<bool>,
    #[serde(alias = "artifact-store")]
    pub artifact_store: Option<PathBuf>,
    #[serde(alias = "actions-cache")]
    pub actions_cache: Option<PathBuf>,

    #[serde(default)]
    pub branches: BranchConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConfigFile {
    #[serde(flatten)]
    pub root_defaults: DefaultsConfig,

    #[serde(default)]
    pub defaults: DefaultsConfig,

    #[serde(default)]
    pub policy: DefaultsConfig,

    #[serde(default)]
    pub locked: DefaultsConfig,

    #[serde(default)]
    pub hooks: BTreeMap<String, WorkflowOverride>,

    #[serde(default)]
    pub workflows: BTreeMap<String, WorkflowOverride>,

    #[serde(alias = "other-workflows")]
    pub other_workflows: Option<bool>,

    #[serde(default)]
    pub actions: ActionsConfig,
}

#[derive(Clone, Debug)]
pub struct Defaults {
    pub shell: String,
    pub quiet: bool,
    pub fail_fast: bool,
    pub arch: Vec<Architecture>,
    pub container: ContainerConfig,
    pub container_runtime: ContainerRuntime,
    pub git_mode: GitMode,
    pub git_command: Option<GitCommand>,
    pub git_image: String,
    pub install_mode: InstallMode,
    pub recursive_checkout: bool,
    pub branch_allow: Vec<String>,
    pub artifact_store: PathBuf,
    pub actions_cache: PathBuf,
    pub node_image: String,
}

#[derive(Clone, Debug)]
pub struct ResolvedConfig {
    pub path: PathBuf,
    pub loaded: bool,
    pub paths: Vec<PathBuf>,
    pub policy: DefaultsConfig,
    pub global_tech_stack: Option<ContainerType>,
    pub defaults: Defaults,
    pub hooks: BTreeMap<String, WorkflowOverride>,
    pub workflows: BTreeMap<String, WorkflowOverride>,
    pub other_workflows: bool,
    pub actions: ActionsConfig,
}

impl ResolvedConfig {
    pub fn load(repo: &RepoInfo, global: &GlobalOptions) -> Result<Self> {
        let default_path = global
            .config
            .clone()
            .unwrap_or_else(|| repo.ci_dir.join("config.yml"));
        let loaded_files = if let Some(path) = &global.config {
            load_explicit_config_files(path)?
        } else {
            load_config_files(&default_config_paths(repo))?
        };

        let loaded = !loaded_files.is_empty();
        let paths = loaded_files
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        let file = merge_config_files(loaded_files.into_iter().map(|(_, file)| file));
        let file_defaults = file.root_defaults.merge(&file.defaults);
        let policy_defaults = file.policy;
        let effective_defaults = file_defaults.merge(&policy_defaults);

        let defaults = Defaults {
            shell: effective_defaults
                .shell
                .clone()
                .unwrap_or_else(|| "/bin/sh".to_string()),
            quiet: effective_defaults.quiet.unwrap_or(false),
            fail_fast: effective_defaults.fail_fast.unwrap_or(true),
            arch: selected_arches_with_policy(
                &global.arch,
                &file_defaults.arch,
                &policy_defaults.arch,
            ),
            container: default_container_config(&effective_defaults),
            container_runtime: effective_defaults
                .container_runtime
                .unwrap_or(ContainerRuntime::Auto),
            git_mode: policy_defaults
                .git_mode
                .or(global.git_mode)
                .or(file_defaults.git_mode)
                .unwrap_or(GitMode::Auto),
            git_command: policy_defaults
                .git_command
                .clone()
                .or_else(|| global.git_command.clone())
                .or_else(|| file_defaults.git_command.clone()),
            git_image: policy_defaults
                .git_image
                .clone()
                .or_else(|| global.git_image.clone())
                .or_else(|| file_defaults.git_image.clone())
                .unwrap_or_else(|| DEFAULT_GIT_IMAGE.to_string()),
            install_mode: policy_defaults
                .install_mode
                .or(file_defaults.install_mode)
                .unwrap_or_default(),
            recursive_checkout: effective_defaults.recursive_checkout.unwrap_or(true),
            branch_allow: if effective_defaults.branches.allow.is_empty() {
                DEFAULT_BRANCHES
                    .iter()
                    .map(|item| (*item).to_string())
                    .collect()
            } else {
                effective_defaults.branches.allow.clone()
            },
            artifact_store: effective_defaults
                .artifact_store
                .clone()
                .unwrap_or_else(|| PathBuf::from("artifacts")),
            actions_cache: effective_defaults
                .actions_cache
                .clone()
                .unwrap_or_else(|| PathBuf::from("actions-cache")),
            node_image: file
                .actions
                .node_image
                .clone()
                .unwrap_or_else(|| "docker.io/library/node:20-alpine".to_string()),
        };

        Ok(Self {
            path: default_path,
            loaded,
            paths,
            policy: policy_defaults.clone(),
            global_tech_stack: policy_defaults.tech_stack.or(global.tech_stack),
            defaults,
            hooks: file.hooks,
            workflows: file.workflows,
            other_workflows: file.other_workflows.unwrap_or(repo.is_bare),
            actions: file.actions,
        })
    }

    pub fn workflow_override(&self, name: &str) -> WorkflowOverride {
        self.workflows.get(name).cloned().unwrap_or_default()
    }

    pub fn hook_override(&self, event: &str) -> WorkflowOverride {
        self.hooks.get(event).cloned().unwrap_or_default()
    }
}

fn load_config_files(paths: &[PathBuf]) -> Result<Vec<(PathBuf, ConfigFile)>> {
    let mut loaded = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }

        let raw = fs::read_to_string(path)?;
        let value: Value = serde_yaml::from_str(&raw)?;
        validate_config_keys(&value, path)?;
        loaded.push((path.clone(), serde_yaml::from_str(&raw)?));
    }
    Ok(loaded)
}

fn load_explicit_config_files(path: &Path) -> Result<Vec<(PathBuf, ConfigFile)>> {
    let mut loaded = load_policy_config_files(&base_policy_config_paths())?;
    loaded.extend(load_config_files(&[path.to_path_buf()])?);
    Ok(loaded)
}

fn load_policy_config_files(paths: &[PathBuf]) -> Result<Vec<(PathBuf, ConfigFile)>> {
    Ok(load_config_files(paths)?
        .into_iter()
        .filter_map(|(path, file)| policy_only_config(file).map(|file| (path, file)))
        .collect())
}

fn policy_only_config(file: ConfigFile) -> Option<ConfigFile> {
    let policy = file.policy.merge(&file.locked);
    if policy.is_empty() {
        return None;
    }

    Some(ConfigFile {
        policy,
        ..ConfigFile::default()
    })
}

fn merge_config_files(files: impl IntoIterator<Item = ConfigFile>) -> ConfigFile {
    let mut merged = ConfigFile::default();
    for file in files {
        merged.root_defaults = merged
            .root_defaults
            .merge(&file.root_defaults.merge(&file.defaults));
        let file_policy = file.policy.merge(&file.locked);
        merged.policy = file_policy.merge(&merged.policy);
        merge_workflow_maps(&mut merged.hooks, file.hooks);
        merge_workflow_maps(&mut merged.workflows, file.workflows);
        merged.other_workflows = file.other_workflows.or(merged.other_workflows);
        merged.actions = merged.actions.merge(&file.actions);
    }
    merged
}

fn merge_workflow_maps(
    merged: &mut BTreeMap<String, WorkflowOverride>,
    next: BTreeMap<String, WorkflowOverride>,
) {
    for (name, workflow) in next {
        merged
            .entry(name)
            .and_modify(|existing| *existing = existing.merge(&workflow))
            .or_insert(workflow);
    }
}

fn default_config_paths(repo: &RepoInfo) -> Vec<PathBuf> {
    let mut paths = base_policy_config_paths();
    paths.push(repo.ci_dir.join("config.yml"));
    paths.push(repo.ci_dir.join("config.yaml"));
    paths
}

fn base_policy_config_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/etc/ci.yml"),
        PathBuf::from("/etc/ci.yaml"),
        PathBuf::from("/etc/ci/config.yml"),
        PathBuf::from("/etc/ci/config.yaml"),
    ];
    paths.extend(user_config_paths());
    paths
}

fn user_config_paths() -> Vec<PathBuf> {
    if let Some(config_home) =
        env::var_os("XDG_CONFIG_HOME").filter(|value| !value.as_os_str().is_empty())
    {
        let config_home = PathBuf::from(config_home);
        return vec![
            config_home.join("ci/config.yml"),
            config_home.join("ci/config.yaml"),
        ];
    }

    env::var_os("HOME")
        .filter(|value| !value.as_os_str().is_empty())
        .map(|home| {
            let config_home = PathBuf::from(home).join(".config");
            vec![
                config_home.join("ci/config.yml"),
                config_home.join("ci/config.yaml"),
            ]
        })
        .unwrap_or_default()
}

fn selected_arches_with_policy(
    global: &[Architecture],
    configured: &ArchFilter,
    policy: &ArchFilter,
) -> Vec<Architecture> {
    if !policy.is_empty() {
        return policy.to_vec();
    }
    selected_arches(global, configured)
}

fn selected_arches(global: &[Architecture], configured: &ArchFilter) -> Vec<Architecture> {
    if !global.is_empty() {
        return global.to_vec();
    }

    let configured = configured.to_vec();
    if configured.is_empty() {
        vec![Architecture::host()]
    } else {
        configured
    }
}

fn default_container_config(defaults: &DefaultsConfig) -> ContainerConfig {
    let mut container = defaults.container.clone();
    if container.kind.is_none() {
        container.kind = defaults.tech_stack;
    }
    if container.arch.is_empty() && !defaults.arch.is_empty() {
        container.arch = defaults.arch.clone();
    }
    container
}

impl ContainerConfig {
    fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.image.is_none()
            && self.platform.is_none()
            && self.workdir.is_none()
            && self.readonly.is_none()
            && self.arch.is_empty()
            && self.packages.is_empty()
            && self.components.is_empty()
            && self.env.is_empty()
            && self.volumes.is_empty()
    }
}

impl DefaultsConfig {
    fn is_empty(&self) -> bool {
        self.shell.is_none()
            && self.quiet.is_none()
            && self.fail_fast.is_none()
            && self.tech_stack.is_none()
            && self.arch.is_empty()
            && self.container.is_empty()
            && self.container_runtime.is_none()
            && self.git_mode.is_none()
            && self.git_command.is_none()
            && self.git_image.is_none()
            && self.install_mode.is_none()
            && self.recursive_checkout.is_none()
            && self.artifact_store.is_none()
            && self.actions_cache.is_none()
            && self.branches.allow.is_empty()
            && self.branches.only.is_empty()
    }

    pub fn merge(&self, other: &Self) -> Self {
        Self {
            shell: other.shell.clone().or_else(|| self.shell.clone()),
            quiet: other.quiet.or(self.quiet),
            fail_fast: other.fail_fast.or(self.fail_fast),
            tech_stack: other.tech_stack.or(self.tech_stack),
            arch: self.arch.merged(&other.arch),
            container: self.container.merge(&other.container),
            container_runtime: other.container_runtime.or(self.container_runtime),
            git_mode: other.git_mode.or(self.git_mode),
            git_command: other
                .git_command
                .clone()
                .or_else(|| self.git_command.clone()),
            git_image: other.git_image.clone().or_else(|| self.git_image.clone()),
            install_mode: other.install_mode.or(self.install_mode),
            recursive_checkout: other.recursive_checkout.or(self.recursive_checkout),
            artifact_store: other
                .artifact_store
                .clone()
                .or_else(|| self.artifact_store.clone()),
            actions_cache: other
                .actions_cache
                .clone()
                .or_else(|| self.actions_cache.clone()),
            branches: self.branches.merge(&other.branches),
        }
    }
}

impl ActionsConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            node_image: other.node_image.clone().or_else(|| self.node_image.clone()),
        }
    }
}

impl WorkflowOverride {
    pub fn merge(&self, other: &Self) -> Self {
        let mut env = self.env.clone();
        for (key, value) in &other.env {
            env.insert(key.clone(), value.clone());
        }

        Self {
            on: self.on.merged(&other.on),
            tech_stack: other.tech_stack.or(self.tech_stack),
            arch: self.arch.merged(&other.arch),
            branches: self.branches.merge(&other.branches),
            artifacts: self.artifacts.merge(&other.artifacts),
            execution: self.execution.merge(&other.execution),
            container: self.container.merge(&other.container),
            env,
        }
    }
}

impl BranchConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            allow: if other.allow.is_empty() {
                self.allow.clone()
            } else {
                other.allow.clone()
            },
            only: if other.only.is_empty() {
                self.only.clone()
            } else {
                other.only.clone()
            },
        }
    }

    pub fn effective<'a>(&'a self, defaults: &'a Defaults) -> &'a [String] {
        if !self.only.is_empty() {
            &self.only
        } else if !self.allow.is_empty() {
            &self.allow
        } else {
            &defaults.branch_allow
        }
    }
}

impl ArtifactConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            paths: if other.paths.is_empty() {
                self.paths.clone()
            } else {
                other.paths.clone()
            },
            mode: other.mode.or(self.mode),
            destination: other
                .destination
                .clone()
                .or_else(|| self.destination.clone()),
        }
    }
}

impl ExecutionConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            workspace: other.workspace.clone().or_else(|| self.workspace.clone()),
            shell: other.shell.clone().or_else(|| self.shell.clone()),
        }
    }
}

impl ContainerConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            kind: other.kind.or(self.kind),
            image: other.image.clone().or_else(|| self.image.clone()),
            platform: other.platform.clone().or_else(|| self.platform.clone()),
            workdir: other.workdir.clone().or_else(|| self.workdir.clone()),
            readonly: other.readonly.or(self.readonly),
            arch: self.arch.merged(&other.arch),
            packages: if other.packages.is_empty() {
                self.packages.clone()
            } else {
                other.packages.clone()
            },
            components: if other.components.is_empty() {
                self.components.clone()
            } else {
                other.components.clone()
            },
            env: {
                let mut env = self.env.clone();
                for (key, value) in &other.env {
                    env.insert(key.clone(), value.clone());
                }
                env
            },
            volumes: if other.volumes.is_empty() {
                self.volumes.clone()
            } else {
                other.volumes.clone()
            },
        }
    }
}

pub fn path_relative_to(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests;
