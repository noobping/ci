use std::collections::BTreeMap;
use std::path::PathBuf;

mod parser;

pub use self::parser::load_actions_workflow;

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

#[cfg(test)]
mod tests;
