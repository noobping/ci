use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cli::GlobalOptions;
use crate::error::Result;
use crate::repo::RepoInfo;

const DEFAULT_BRANCHES: &[&str] = &["main", "master", "develop", "development"];
const DEFAULT_GIT_IMAGE: &str = "docker.io/alpine/git:latest";

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Architecture(String);

impl Architecture {
    pub fn host() -> Self {
        Self::from_alias(std::env::consts::ARCH).unwrap_or_else(|| Self("x64".to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn runner_suffix(&self) -> String {
        self.0
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect()
    }

    pub fn platform(&self) -> String {
        match self.0.as_str() {
            "x64" => "linux/amd64".to_string(),
            "arm64" => "linux/arm64".to_string(),
            value if value.starts_with("linux/") => value.to_string(),
            value => format!("linux/{value}"),
        }
    }

    fn from_alias(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }

        let value = value.to_ascii_lowercase().replace('-', "_");
        let value = value.strip_prefix("linux/").unwrap_or(&value);
        let canonical = match value {
            "amd64" | "x64" | "x86_64" => "x64".to_string(),
            "arm64" | "aarch64" => "arm64".to_string(),
            other => other.replace('_', "-"),
        };
        Some(Self(canonical))
    }
}

impl Default for Architecture {
    fn default() -> Self {
        Self::host()
    }
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Architecture {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Self::from_alias(value).ok_or_else(|| "architecture must not be empty".to_string())
    }
}

impl Serialize for Architecture {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Architecture {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerType {
    Auto,
    General,
    Rust,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ContainerConfig {
    #[serde(rename = "type")]
    pub kind: Option<ContainerType>,
    pub image: Option<String>,
    pub platform: Option<String>,
    #[serde(default)]
    pub arch: ArchFilter,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub components: Vec<String>,
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
    pub silent: Option<bool>,
    pub fail_fast: Option<bool>,
    #[serde(default)]
    pub arch: ArchFilter,
    #[serde(default)]
    pub container: ContainerConfig,
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

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArchFilter {
    #[default]
    None,
    Single(Architecture),
    Many(Vec<Architecture>),
}

impl ArchFilter {
    pub fn to_vec(&self) -> Vec<Architecture> {
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

    pub fn allows(&self, arch: &Architecture) -> bool {
        match self {
            Self::None => true,
            Self::Many(values) if values.is_empty() => true,
            Self::Single(value) => value == arch,
            Self::Many(values) => values.iter().any(|value| value == arch),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Self::None => true,
            Self::Many(values) => values.is_empty(),
            Self::Single(_) => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Defaults {
    pub shell: String,
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

        let file_defaults = file.root_defaults.merge(&file.defaults);

        let defaults = Defaults {
            shell: file_defaults
                .shell
                .clone()
                .unwrap_or_else(|| "/bin/sh".to_string()),
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
    if container.arch.is_empty() && !defaults.arch.is_empty() {
        container.arch = defaults.arch.clone();
    }
    container
}

pub fn format_arches(arches: &[Architecture]) -> String {
    if arches.is_empty() {
        return Architecture::host().to_string();
    }
    arches
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

impl DefaultsConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            shell: other.shell.clone().or_else(|| self.shell.clone()),
            silent: other.silent.or(self.silent),
            fail_fast: other.fail_fast.or(self.fail_fast),
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
mod tests {
    use super::{default_container_config, ConfigFile, ContainerType};

    #[test]
    fn defaults_arch_accepts_single_value_or_list() {
        let single: ConfigFile = serde_yaml::from_str(
            r#"
defaults:
  arch: amd64
"#,
        )
        .expect("parse single arch");
        assert_eq!(
            single
                .defaults
                .arch
                .to_vec()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["x64"]
        );

        let many: ConfigFile = serde_yaml::from_str(
            r#"
defaults:
  arch:
    - x86_64
    - aarch64
"#,
        )
        .expect("parse arch list");
        assert_eq!(
            many.defaults
                .arch
                .to_vec()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["x64", "arm64"]
        );
    }

    #[test]
    fn container_config_accepts_type_arch_packages_and_components() {
        let file: ConfigFile = serde_yaml::from_str(
            r#"
workflows:
  build:
    container:
      type: rust
      arch:
        - amd64
        - aarch64
      packages:
        - htop
      components:
        - cargo-fmt
"#,
        )
        .expect("parse container config");
        let container = &file.workflows.get("build").expect("workflow").container;

        assert_eq!(container.kind, Some(ContainerType::Rust));
        assert_eq!(
            container
                .arch
                .to_vec()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["x64", "arm64"]
        );
        assert_eq!(container.packages, vec!["htop"]);
        assert_eq!(container.components, vec!["cargo-fmt"]);
    }

    #[test]
    fn defaults_arch_becomes_default_container_arch() {
        let file: ConfigFile = serde_yaml::from_str(
            r#"
defaults:
  arch:
    - amd64
    - aarch64
"#,
        )
        .expect("parse defaults");
        let container = default_container_config(&file.defaults);

        assert_eq!(
            container
                .arch
                .to_vec()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["x64", "arm64"]
        );
    }

    #[test]
    fn config_file_accepts_default_fields_at_root() {
        let file: ConfigFile = serde_yaml::from_str(
            r#"
container:
  type: rust
  arch:
    - amd64
    - aarch64
  components:
    - cargo-fmt
"#,
        )
        .expect("parse root defaults");
        let defaults = file.root_defaults.merge(&file.defaults);

        assert_eq!(defaults.container.kind, Some(ContainerType::Rust));
        assert_eq!(
            defaults
                .container
                .arch
                .to_vec()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["x64", "arm64"]
        );
        assert_eq!(defaults.container.components, vec!["cargo-fmt"]);
    }

    #[test]
    fn explicit_defaults_override_root_default_shorthand() {
        let file: ConfigFile = serde_yaml::from_str(
            r#"
container:
  arch: amd64
defaults:
  container:
    arch: aarch64
"#,
        )
        .expect("parse mixed defaults");
        let defaults = file.root_defaults.merge(&file.defaults);

        assert_eq!(
            defaults
                .container
                .arch
                .to_vec()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["arm64"]
        );
    }

    #[test]
    fn defaults_container_arch_overrides_defaults_arch_for_containers() {
        let file: ConfigFile = serde_yaml::from_str(
            r#"
defaults:
  arch: amd64
  container:
    arch: aarch64
"#,
        )
        .expect("parse defaults");
        let container = default_container_config(&file.defaults);

        assert_eq!(
            container
                .arch
                .to_vec()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["arm64"]
        );
    }
}
