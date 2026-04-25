use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::cli::GlobalOptions;
pub mod types;
mod validation;

pub use self::types::{
    format_arches, ArchFilter, Architecture, ArtifactConfig, ArtifactMode, ColorWhen,
    ContainerRuntime, ContainerType, EventFilter, GitMode,
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
    pub silent: Option<bool>,
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
    #[serde(alias = "git-image")]
    pub git_image: Option<String>,
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
    pub hooks: BTreeMap<String, WorkflowOverride>,

    #[serde(default)]
    pub workflows: BTreeMap<String, WorkflowOverride>,

    #[serde(default)]
    pub actions: ActionsConfig,
}

#[derive(Clone, Debug)]
pub struct Defaults {
    pub shell: String,
    pub quiet: bool,
    pub silent: bool,
    pub fail_fast: bool,
    pub arch: Vec<Architecture>,
    pub container: ContainerConfig,
    pub container_runtime: ContainerRuntime,
    pub git_mode: GitMode,
    pub git_image: String,
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
    pub global_tech_stack: Option<ContainerType>,
    pub defaults: Defaults,
    pub hooks: BTreeMap<String, WorkflowOverride>,
    pub workflows: BTreeMap<String, WorkflowOverride>,
    pub actions: ActionsConfig,
}

impl ResolvedConfig {
    pub fn load(repo: &RepoInfo, global: &GlobalOptions) -> Result<Self> {
        let path = global
            .config
            .clone()
            .unwrap_or_else(|| repo.ci_dir.join("config.yml"));
        let loaded = path.exists();
        let file: ConfigFile = if loaded {
            let raw = fs::read_to_string(&path)?;
            let value: Value = serde_yaml::from_str(&raw)?;
            validate_config_keys(&value, &path)?;
            serde_yaml::from_str(&raw)?
        } else {
            ConfigFile::default()
        };

        let file_defaults = file.root_defaults.merge(&file.defaults);

        let defaults = Defaults {
            shell: file_defaults
                .shell
                .clone()
                .unwrap_or_else(|| "/bin/sh".to_string()),
            quiet: file_defaults.quiet.unwrap_or(false),
            silent: file_defaults.silent.unwrap_or(false),
            fail_fast: file_defaults.fail_fast.unwrap_or(true),
            arch: selected_arches(&global.arch, &file_defaults.arch),
            container: default_container_config(&file_defaults),
            container_runtime: file_defaults
                .container_runtime
                .unwrap_or(ContainerRuntime::Auto),
            git_mode: global
                .git_mode
                .or(file_defaults.git_mode)
                .unwrap_or(GitMode::Auto),
            git_image: global
                .git_image
                .clone()
                .or_else(|| file_defaults.git_image.clone())
                .unwrap_or_else(|| DEFAULT_GIT_IMAGE.to_string()),
            recursive_checkout: file_defaults.recursive_checkout.unwrap_or(true),
            branch_allow: if file_defaults.branches.allow.is_empty() {
                DEFAULT_BRANCHES
                    .iter()
                    .map(|item| (*item).to_string())
                    .collect()
            } else {
                file_defaults.branches.allow.clone()
            },
            artifact_store: file_defaults
                .artifact_store
                .clone()
                .unwrap_or_else(|| PathBuf::from("artifacts")),
            actions_cache: file_defaults
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
            path,
            loaded,
            global_tech_stack: global.tech_stack,
            defaults,
            hooks: file.hooks,
            workflows: file.workflows,
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

impl DefaultsConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            shell: other.shell.clone().or_else(|| self.shell.clone()),
            quiet: other.quiet.or(self.quiet),
            silent: other.silent.or(self.silent),
            fail_fast: other.fail_fast.or(self.fail_fast),
            tech_stack: other.tech_stack.or(self.tech_stack),
            arch: self.arch.merged(&other.arch),
            container: self.container.merge(&other.container),
            container_runtime: other.container_runtime.or(self.container_runtime),
            git_mode: other.git_mode.or(self.git_mode),
            git_image: other.git_image.clone().or_else(|| self.git_image.clone()),
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
