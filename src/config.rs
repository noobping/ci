use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::cli::GlobalOptions;
use crate::error::Result;
use crate::repo::RepoInfo;

const DEFAULT_BRANCHES: &[&str] = &["main", "master", "develop", "development"];
const DEFAULT_GIT_IMAGE: &str = "docker.io/alpine/git:latest";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ColorWhen {
    #[default]
    Auto,
    Always,
    Never,
}

impl fmt::Display for ColorWhen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerRuntime {
    #[default]
    Auto,
    Podman,
    Docker,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum GitMode {
    Host,
    #[default]
    Auto,
    Alias,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactMode {
    #[default]
    Keep,
    Move,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BranchConfig {
    #[serde(default)]
    pub allow: Vec<String>,

    #[serde(default)]
    pub only: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ArtifactConfig {
    #[serde(default)]
    pub paths: Vec<String>,

    pub mode: Option<ArtifactMode>,

    pub destination: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ExecutionConfig {
    pub workspace: Option<PathBuf>,
    pub shell: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ContainerConfig {
    pub platform: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ActionsConfig {
    pub node_image: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WorkflowOverride {
    #[serde(default, rename = "on")]
    pub on: EventFilter,

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
    pub fail_fast: Option<bool>,
    pub container_runtime: Option<ContainerRuntime>,
    pub git_mode: Option<GitMode>,
    pub git_image: Option<String>,
    pub recursive_checkout: Option<bool>,
    pub artifact_store: Option<PathBuf>,
    pub actions_cache: Option<PathBuf>,

    #[serde(default)]
    pub branches: BranchConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub defaults: DefaultsConfig,

    #[serde(default)]
    pub hooks: BTreeMap<String, WorkflowOverride>,

    #[serde(default)]
    pub workflows: BTreeMap<String, WorkflowOverride>,

    #[serde(default)]
    pub actions: ActionsConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EventFilter {
    #[default]
    None,
    Single(String),
    Many(Vec<String>),
}

impl EventFilter {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::Single(value) => vec![value.clone()],
            Self::Many(values) => values.clone(),
        }
    }

    pub fn merged(&self, other: &Self) -> Self {
        match other {
            Self::None => self.clone(),
            _ => other.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Defaults {
    pub shell: String,
    pub fail_fast: bool,
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
            serde_yaml::from_str(&fs::read_to_string(&path)?)?
        } else {
            ConfigFile::default()
        };

        let defaults = Defaults {
            shell: file
                .defaults
                .shell
                .clone()
                .unwrap_or_else(|| "/bin/sh".to_string()),
            fail_fast: file.defaults.fail_fast.unwrap_or(true),
            container_runtime: file
                .defaults
                .container_runtime
                .unwrap_or(ContainerRuntime::Auto),
            git_mode: global
                .git_mode
                .or(file.defaults.git_mode)
                .unwrap_or(GitMode::Auto),
            git_image: global
                .git_image
                .clone()
                .or_else(|| file.defaults.git_image.clone())
                .unwrap_or_else(|| DEFAULT_GIT_IMAGE.to_string()),
            recursive_checkout: file.defaults.recursive_checkout.unwrap_or(true),
            branch_allow: if file.defaults.branches.allow.is_empty() {
                DEFAULT_BRANCHES.iter().map(|item| (*item).to_string()).collect()
            } else {
                file.defaults.branches.allow.clone()
            },
            artifact_store: file
                .defaults
                .artifact_store
                .clone()
                .unwrap_or_else(|| PathBuf::from("artifacts")),
            actions_cache: file
                .defaults
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

impl WorkflowOverride {
    pub fn merge(&self, other: &Self) -> Self {
        let mut env = self.env.clone();
        for (key, value) in &other.env {
            env.insert(key.clone(), value.clone());
        }

        Self {
            on: self.on.merged(&other.on),
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
            destination: other.destination.clone().or_else(|| self.destination.clone()),
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
            platform: other.platform.clone().or_else(|| self.platform.clone()),
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
