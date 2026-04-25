use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::actions::ActionRunStep;
use crate::error::{CiError, Result};

pub(crate) fn strip_action_ref(value: &str) -> &str {
    value.split('@').next().unwrap_or(value)
}

pub(crate) struct RemoteActionSpec {
    pub(crate) owner: String,
    pub(crate) repo: String,
    pub(crate) subpath: String,
    pub(crate) reference: String,
}

pub(crate) fn parse_remote_action(value: &str) -> Result<RemoteActionSpec> {
    let (repo_path, reference) = value
        .split_once('@')
        .ok_or_else(|| CiError::Message(format!("remote action `{value}` is missing `@ref`")))?;
    let mut parts = repo_path.split('/');
    let owner = parts
        .next()
        .ok_or_else(|| CiError::Message(format!("invalid action reference `{value}`")))?;
    let repo = parts
        .next()
        .ok_or_else(|| CiError::Message(format!("invalid action reference `{value}`")))?;
    let subpath = parts.collect::<Vec<_>>().join("/");
    Ok(RemoteActionSpec {
        owner: owner.to_string(),
        repo: repo.to_string(),
        subpath,
        reference: reference.to_string(),
    })
}

#[derive(Debug, Deserialize)]
struct ActionMetadata {
    runs: RawActionRuns,
}

#[derive(Debug, Deserialize)]
struct RawActionRuns {
    using: String,
    main: Option<String>,
    image: Option<String>,
    dockerfile: Option<String>,
    entrypoint: Option<String>,
    args: Option<Vec<String>>,
    #[serde(default)]
    steps: Vec<RawActionMetadataStep>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawActionMetadataStep {
    name: Option<String>,
    #[serde(rename = "if")]
    if_condition: Option<String>,
    run: Option<String>,
    uses: Option<String>,
    shell: Option<String>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    #[serde(rename = "working-directory")]
    working_directory: Option<String>,
    #[serde(rename = "continue-on-error", default)]
    continue_on_error: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum ActionMetadataStep {
    Run(ActionRunStep),
    Uses,
}

pub(crate) enum ActionRuns {
    Composite {
        steps: Vec<ActionMetadataStep>,
    },
    Docker {
        image: Option<String>,
        dockerfile: Option<String>,
        entrypoint: Option<String>,
        args: Option<Vec<String>>,
    },
    Node {
        main: String,
    },
}

pub(crate) fn load_action_metadata(dir: &Path) -> Result<ActionDefinition> {
    for name in ["action.yml", "action.yaml"] {
        let path = dir.join(name);
        if path.exists() {
            let ActionMetadata { runs } = serde_yaml::from_str(&fs::read_to_string(path)?)?;
            let RawActionRuns {
                using,
                main,
                image,
                dockerfile,
                entrypoint,
                args,
                steps: raw_steps,
            } = runs;
            let steps = raw_steps
                .into_iter()
                .enumerate()
                .map(|(index, step)| {
                    let name = step.name.clone().unwrap_or_else(|| {
                        step.uses
                            .clone()
                            .unwrap_or_else(|| format!("step-{}", index + 1))
                    });
                    match (step.run, step.uses) {
                        (Some(run), None) => Ok(ActionMetadataStep::Run(ActionRunStep {
                            name,
                            run,
                            shell: step.shell,
                            env: step.env,
                            if_condition: step.if_condition,
                            working_directory: step.working_directory,
                            continue_on_error: step.continue_on_error,
                            timeout_minutes: None,
                        })),
                        (None, Some(_uses)) => Ok(ActionMetadataStep::Uses),
                        _ => Err(CiError::Message(format!(
                            "action {} has a composite step without exactly one of `run` or `uses`",
                            dir.display()
                        ))),
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            let runs = match using.as_str() {
                "composite" => ActionRuns::Composite { steps },
                "docker" => ActionRuns::Docker {
                    image,
                    dockerfile,
                    entrypoint,
                    args,
                },
                value if value.starts_with("node") => ActionRuns::Node {
                    main: main.ok_or_else(|| {
                        CiError::Message(format!(
                            "node action {} is missing runs.main",
                            dir.display()
                        ))
                    })?,
                },
                other => {
                    return Err(CiError::Message(format!(
                        "action {} uses unsupported runner `{other}`",
                        dir.display()
                    )))
                }
            };
            return Ok(ActionDefinition { runs });
        }
    }

    Err(CiError::Message(format!(
        "no action.yml or action.yaml found in {}",
        dir.display()
    )))
}

pub(crate) struct ActionDefinition {
    pub(crate) runs: ActionRuns,
}
