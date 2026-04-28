use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_yaml::Value;

use crate::config::{
    ArchFilter, ArtifactConfig, BranchConfig, ContainerConfig, EventFilter, ExecutionConfig,
    WorkflowOverride,
};
use crate::error::{CiError, Result};
use crate::workflow::{NativeStep, StepContainerConfig};

#[derive(Clone, Debug, Deserialize, Default)]
pub(crate) struct NativeWorkflowFile {
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) defaults: WorkflowOverride,
    #[serde(default, rename = "on")]
    pub(crate) on: EventFilter,
    #[serde(default, alias = "requires", alias = "depends", alias = "dependencies")]
    pub(crate) needs: WorkflowNeeds,
    #[serde(
        default,
        rename = "tech",
        alias = "type",
        alias = "tech-stack",
        alias = "tech_stack"
    )]
    pub(crate) tech_stack: Option<crate::config::ContainerType>,
    #[serde(default)]
    pub(crate) arch: ArchFilter,
    #[serde(default)]
    pub(crate) branches: BranchConfig,
    #[serde(default)]
    pub(crate) artifacts: ArtifactConfig,
    #[serde(default)]
    pub(crate) execution: ExecutionConfig,
    #[serde(default)]
    pub(crate) container: ContainerConfig,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) steps: Vec<RawNativeStep>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(untagged)]
pub(crate) enum WorkflowNeeds {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl WorkflowNeeds {
    pub(crate) fn to_vec(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::One(value) => vec![value.clone()],
            Self::Many(values) => values.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub(crate) struct RawNativeStep {
    name: Option<String>,
    run: Option<String>,
    #[serde(rename = "use")]
    use_value: Option<String>,
    uses: Option<String>,
    container: Option<RawStepContainer>,
    #[serde(alias = "read-only", alias = "read_only")]
    readonly: Option<bool>,
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
    pub(crate) fn into_step(self, workflow_path: &Path, index: usize) -> Result<NativeStep> {
        let RawNativeStep {
            name,
            run,
            use_value,
            uses,
            container,
            readonly,
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
            return Err(CiError::Message(format!(
                "{} step {} must define only one of `use` or `uses`",
                workflow_path.display(),
                index + 1
            )));
        }
        let uses = use_value.or(uses);
        let (container, container_config) = match container {
            Some(RawStepContainer::Bool(value)) => (Some(value), None),
            Some(RawStepContainer::Image(image)) => (
                Some(true),
                Some(StepContainerConfig {
                    image: Some(image),
                    ..StepContainerConfig::default()
                }),
            ),
            Some(RawStepContainer::Config(value)) => (
                Some(true),
                Some(StepContainerConfig {
                    file: value.file,
                    image: value.image,
                    platform: value.platform,
                    workdir: value.workdir,
                    readonly: value.readonly,
                    env: value.env,
                    volumes: value.volumes,
                    packages: value.packages,
                    components: value.components,
                }),
            ),
            None => (None, None),
        };

        NativeStep {
            name,
            run,
            uses,
            container,
            container_config,
            readonly,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum RawStepContainer {
    Bool(bool),
    Config(RawStepContainerConfig),
    Image(String),
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawStepContainerConfig {
    #[serde(
        rename = "file",
        alias = "containerfile",
        alias = "container-file",
        alias = "container_file",
        alias = "dockerfile",
        alias = "docker-file",
        alias = "docker_file"
    )]
    file: Option<String>,
    image: Option<String>,
    platform: Option<String>,
    #[serde(alias = "working-directory", alias = "working_directory")]
    workdir: Option<String>,
    #[serde(alias = "read-only", alias = "read_only")]
    readonly: Option<bool>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    volumes: Vec<String>,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    components: Vec<String>,
}

impl NativeWorkflowFile {
    pub(crate) fn metadata(&self) -> WorkflowOverride {
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

pub(crate) fn is_native_inline_action_builtin(uses: &str) -> bool {
    let normalized = uses
        .split('@')
        .next()
        .unwrap_or(uses)
        .trim()
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "clean" | "ci/clean" | "podman" | "ci/podman" | "docker" | "ci/docker"
    )
}

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
