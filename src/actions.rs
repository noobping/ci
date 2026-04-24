use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_yaml::Value;

use crate::error::{CiError, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionsProvider {
    GitHub,
    Gitea,
}

#[derive(Clone, Debug)]
pub struct ActionsWorkflow {
    pub name: String,
    pub path: PathBuf,
    pub provider: ActionsProvider,
    pub events: Vec<ActionEvent>,
    pub env: BTreeMap<String, String>,
    pub defaults: RunDefaults,
    pub jobs: Vec<ActionsJob>,
}

#[derive(Clone, Debug)]
pub struct ActionEvent {
    pub name: String,
    pub branches: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RunDefaults {
    pub shell: Option<String>,
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ActionsJob {
    pub id: String,
    pub name: String,
    pub needs: Vec<String>,
    pub if_condition: Option<String>,
    pub env: BTreeMap<String, String>,
    pub defaults: RunDefaults,
    pub matrix: Vec<BTreeMap<String, String>>,
    pub container: Option<ActionContainer>,
    pub services: BTreeMap<String, ActionService>,
    pub steps: Vec<ActionStep>,
    pub continue_on_error: bool,
    pub timeout_minutes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ActionContainer {
    pub image: String,
    pub env: BTreeMap<String, String>,
    pub options: Option<String>,
    pub ports: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ActionService {
    pub image: String,
    pub env: BTreeMap<String, String>,
    pub options: Option<String>,
    pub ports: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum ActionStep {
    Run(ActionRunStep),
    Uses(ActionUsesStep),
}

#[derive(Clone, Debug)]
pub struct ActionRunStep {
    pub name: String,
    pub run: String,
    pub shell: Option<String>,
    pub env: BTreeMap<String, String>,
    pub if_condition: Option<String>,
    pub working_directory: Option<String>,
    pub continue_on_error: bool,
    pub timeout_minutes: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ActionUsesStep {
    pub name: String,
    pub uses: String,
    pub with: BTreeMap<String, String>,
    pub env: BTreeMap<String, String>,
    pub if_condition: Option<String>,
    pub working_directory: Option<String>,
    pub continue_on_error: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawActionsWorkflow {
    name: Option<String>,
    #[serde(rename = "on")]
    on_value: Option<Value>,
    #[serde(default)]
    env: BTreeMap<String, Value>,
    defaults: Option<RawDefaults>,
    #[serde(default)]
    jobs: BTreeMap<String, RawJob>,
    permissions: Option<Value>,
    secrets: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawDefaults {
    run: Option<RawRunDefaults>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawRunDefaults {
    shell: Option<String>,
    #[serde(rename = "working-directory")]
    working_directory: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawJob {
    name: Option<String>,
    #[serde(rename = "if")]
    if_condition: Option<String>,
    #[serde(default)]
    needs: RawNeeds,
    #[serde(default)]
    env: BTreeMap<String, Value>,
    defaults: Option<RawDefaults>,
    strategy: Option<RawStrategy>,
    container: Option<RawContainer>,
    #[serde(default)]
    services: BTreeMap<String, RawContainer>,
    #[serde(default)]
    steps: Vec<RawStep>,
    #[serde(rename = "continue-on-error")]
    continue_on_error: Option<bool>,
    #[serde(rename = "timeout-minutes")]
    timeout_minutes: Option<u64>,
    uses: Option<String>,
    permissions: Option<Value>,
    secrets: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(untagged)]
enum RawNeeds {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawStrategy {
    matrix: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum RawContainer {
    Image(String),
    Detailed {
        image: String,
        #[serde(default)]
        env: BTreeMap<String, Value>,
        options: Option<String>,
        #[serde(default)]
        ports: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Default)]
struct RawStep {
    name: Option<String>,
    #[serde(rename = "if")]
    if_condition: Option<String>,
    run: Option<String>,
    uses: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, Value>,
    #[serde(default)]
    with: BTreeMap<String, Value>,
    shell: Option<String>,
    #[serde(rename = "working-directory")]
    working_directory: Option<String>,
    #[serde(rename = "continue-on-error")]
    continue_on_error: Option<bool>,
    #[serde(rename = "timeout-minutes")]
    timeout_minutes: Option<u64>,
}

impl RawNeeds {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

impl RawContainer {
    fn into_container(self) -> ActionContainer {
        match self {
            Self::Image(image) => ActionContainer {
                image,
                env: BTreeMap::new(),
                options: None,
                ports: Vec::new(),
            },
            Self::Detailed {
                image,
                env,
                options,
                ports,
            } => ActionContainer {
                image,
                env: stringify_map(env),
                options,
                ports,
            },
        }
    }

    fn into_service(self) -> ActionService {
        match self {
            Self::Image(image) => ActionService {
                image,
                env: BTreeMap::new(),
                options: None,
                ports: Vec::new(),
            },
            Self::Detailed {
                image,
                env,
                options,
                ports,
            } => ActionService {
                image,
                env: stringify_map(env),
                options,
                ports,
            },
        }
    }
}

impl ActionsWorkflow {
    pub fn provider_label(&self) -> &'static str {
        match self.provider {
            ActionsProvider::GitHub => "github",
            ActionsProvider::Gitea => "gitea",
        }
    }

    pub fn remote_base(&self) -> &'static str {
        match self.provider {
            ActionsProvider::GitHub => "https://github.com",
            ActionsProvider::Gitea => "https://gitea.com",
        }
    }

    pub fn matches_event(&self, events: &[String], branch: Option<&str>) -> Option<String> {
        for canonical in events {
            for action_event in &self.events {
                if action_event.name == *canonical {
                    if !action_event.branches.is_empty() {
                        if let Some(branch) = branch {
                            if !action_event.branches.iter().any(|item| item == branch) {
                                continue;
                            }
                        } else {
                            continue;
                        }
                    }
                    return Some(format!(
                        "{} declares `{}`",
                        self.path.display(),
                        action_event.name
                    ));
                }
            }
        }
        None
    }
}

pub fn load_actions_workflow(path: &Path, provider: ActionsProvider) -> Result<ActionsWorkflow> {
    let raw: RawActionsWorkflow = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    reject_unsupported_workflow(path, &raw)?;

    let defaults = raw.defaults.unwrap_or_default();
    let workflow = ActionsWorkflow {
        name: raw
            .name
            .clone()
            .unwrap_or_else(|| path.file_stem().and_then(|value| value.to_str()).unwrap_or("workflow").to_string()),
        path: path.to_path_buf(),
        provider,
        events: parse_events(path, raw.on_value.as_ref())?,
        env: stringify_map(raw.env),
        defaults: RunDefaults {
            shell: defaults.run.as_ref().and_then(|run| run.shell.clone()),
            working_directory: defaults.run.and_then(|run| run.working_directory),
        },
        jobs: parse_jobs(path, raw.jobs)?,
    };

    Ok(workflow)
}

fn reject_unsupported_workflow(path: &Path, raw: &RawActionsWorkflow) -> Result<()> {
    if raw.permissions.is_some() {
        return Err(CiError::Message(format!(
            "{} uses `permissions`, which is not supported locally",
            path.display()
        )));
    }
    if raw.secrets.is_some() {
        return Err(CiError::Message(format!(
            "{} uses `secrets`, which is not supported locally",
            path.display()
        )));
    }

    if let Some(on_value) = raw.on_value.as_ref() {
        if let Value::Mapping(map) = on_value {
            for key in map.keys() {
                if scalar_to_string(key).as_deref() == Some("workflow_call") {
                    return Err(CiError::Message(format!(
                        "{} uses `workflow_call`, which is not supported",
                        path.display()
                    )));
                }
            }
        }
    }

    Ok(())
}

fn parse_jobs(path: &Path, raw: BTreeMap<String, RawJob>) -> Result<Vec<ActionsJob>> {
    let mut jobs = Vec::new();
    for (id, job) in raw {
        if job.uses.is_some() {
            return Err(CiError::Message(format!(
                "{} job `{id}` uses reusable workflows (`jobs.<id>.uses`), which are not supported",
                path.display()
            )));
        }
        if job.permissions.is_some() {
            return Err(CiError::Message(format!(
                "{} job `{id}` uses `permissions`, which is not supported locally",
                path.display()
            )));
        }
        if job.secrets.is_some() {
            return Err(CiError::Message(format!(
                "{} job `{id}` uses `secrets`, which is not supported locally",
                path.display()
            )));
        }

        let defaults = job.defaults.unwrap_or_default();
        let matrix = expand_matrix(job.strategy.as_ref().and_then(|value| value.matrix.as_ref()))?;

        let mut steps = Vec::new();
        for (index, step) in job.steps.into_iter().enumerate() {
            let name = step
                .name
                .clone()
                .unwrap_or_else(|| step.uses.clone().unwrap_or_else(|| format!("step-{}", index + 1)));
            if let Some(run) = step.run {
                steps.push(ActionStep::Run(ActionRunStep {
                    name,
                    run,
                    shell: step.shell,
                    env: stringify_map(step.env),
                    if_condition: step.if_condition,
                    working_directory: step.working_directory,
                    continue_on_error: step.continue_on_error.unwrap_or(false),
                    timeout_minutes: step.timeout_minutes,
                }));
            } else if let Some(uses) = step.uses {
                steps.push(ActionStep::Uses(ActionUsesStep {
                    name,
                    uses,
                    with: stringify_map(step.with),
                    env: stringify_map(step.env),
                    if_condition: step.if_condition,
                    working_directory: step.working_directory,
                    continue_on_error: step.continue_on_error.unwrap_or(false),
                }));
            } else {
                return Err(CiError::Message(format!(
                    "{} job `{id}` has a step without `run` or `uses`",
                    path.display()
                )));
            }
        }

        let services = job
            .services
            .into_iter()
            .map(|(name, raw)| (name, raw.into_service()))
            .collect();

        jobs.push(ActionsJob {
            id: id.clone(),
            name: job.name.unwrap_or(id),
            needs: job.needs.into_vec(),
            if_condition: job.if_condition,
            env: stringify_map(job.env),
            defaults: RunDefaults {
                shell: defaults.run.as_ref().and_then(|run| run.shell.clone()),
                working_directory: defaults.run.and_then(|run| run.working_directory),
            },
            matrix,
            container: job.container.map(RawContainer::into_container),
            services,
            steps,
            continue_on_error: job.continue_on_error.unwrap_or(false),
            timeout_minutes: job.timeout_minutes,
        });
    }

    Ok(jobs)
}

fn expand_matrix(value: Option<&Value>) -> Result<Vec<BTreeMap<String, String>>> {
    let Some(value) = value else {
        return Ok(vec![BTreeMap::new()]);
    };
    let Value::Mapping(map) = value else {
        return Ok(vec![BTreeMap::new()]);
    };

    let mut axes = Vec::new();
    let mut include = Vec::new();
    let mut exclude = Vec::new();

    for (key, value) in map {
        let Some(key) = scalar_to_string(key) else {
            continue;
        };
        match key.as_str() {
            "include" => include = list_of_maps(value)?,
            "exclude" => exclude = list_of_maps(value)?,
            _ => {
                let values = list_of_strings(value);
                if !values.is_empty() {
                    axes.push((key, values));
                }
            }
        }
    }

    let mut rows = vec![BTreeMap::new()];
    for (axis, values) in axes {
        let mut next = Vec::new();
        for row in &rows {
            for value in &values {
                let mut updated = row.clone();
                updated.insert(axis.clone(), value.clone());
                next.push(updated);
            }
        }
        rows = next;
    }

    if rows.is_empty() {
        rows.push(BTreeMap::new());
    }

    if !exclude.is_empty() {
        rows.retain(|row| !exclude.iter().any(|candidate| is_subset(candidate, row)));
    }

    rows.extend(include);

    let mut unique = BTreeSet::new();
    rows.retain(|row| unique.insert(format!("{row:?}")));
    Ok(rows)
}

fn is_subset(left: &BTreeMap<String, String>, right: &BTreeMap<String, String>) -> bool {
    left.iter()
        .all(|(key, value)| right.get(key).map(|item| item == value).unwrap_or(false))
}

fn list_of_maps(value: &Value) -> Result<Vec<BTreeMap<String, String>>> {
    let Value::Sequence(items) = value else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for item in items {
        if let Value::Mapping(map) = item {
            let mut row = BTreeMap::new();
            for (key, value) in map {
                if let Some(key) = scalar_to_string(key) {
                    row.insert(key, yaml_value_to_string(value));
                }
            }
            result.push(row);
        }
    }
    Ok(result)
}

fn parse_events(path: &Path, value: Option<&Value>) -> Result<Vec<ActionEvent>> {
    let Some(value) = value else {
        return Ok(vec![ActionEvent {
            name: "workflow_dispatch".to_string(),
            branches: Vec::new(),
        }]);
    };

    match value {
        Value::String(name) => Ok(vec![ActionEvent {
            name: name.clone(),
            branches: Vec::new(),
        }]),
        Value::Sequence(items) => Ok(items
            .iter()
            .filter_map(scalar_to_string)
            .map(|name| ActionEvent {
                name,
                branches: Vec::new(),
            })
            .collect()),
        Value::Mapping(map) => {
            let mut events = Vec::new();
            for (key, value) in map {
                let Some(name) = scalar_to_string(key) else {
                    continue;
                };
                let branches = parse_event_branches(value);
                events.push(ActionEvent { name, branches });
            }
            if events.is_empty() {
                return Err(CiError::Message(format!(
                    "{} has an empty `on` block",
                    path.display()
                )));
            }
            Ok(events)
        }
        _ => Err(CiError::Message(format!(
            "{} has an unsupported `on` format",
            path.display()
        ))),
    }
}

fn parse_event_branches(value: &Value) -> Vec<String> {
    match value {
        Value::Mapping(map) => {
            for (key, value) in map {
                if scalar_to_string(key).as_deref() == Some("branches") {
                    return list_of_strings(value);
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn list_of_strings(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Sequence(items) => items.iter().filter_map(scalar_to_string).collect(),
        _ => Vec::new(),
    }
}

fn stringify_map(input: BTreeMap<String, Value>) -> BTreeMap<String, String> {
    input
        .into_iter()
        .map(|(key, value)| (key, yaml_value_to_string(&value)))
        .collect()
}

fn yaml_value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => String::new(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
