use std::path::Path;

use serde_yaml::Value;

use crate::error::{CiError, Result};

pub(crate) fn validate_config_keys(value: &Value, path: &Path) -> Result<()> {
    validate_mapping(value, path, "config", ROOT_CONFIG_KEYS)?;
    for (key, child) in mapping_entries(value, path, "config")? {
        match key.as_str() {
            "defaults" => validate_defaults_keys(child, path, "defaults")?,
            "policy" | "locked" => validate_defaults_keys(child, path, &key)?,
            "hooks" | "workflows" => {
                for (name, workflow) in mapping_entries(child, path, &key)? {
                    validate_workflow_override_keys(workflow, path, &format!("{key}.{name}"))?;
                }
            }
            "actions" => validate_mapping(child, path, "actions", ACTIONS_KEYS)?,
            key if DEFAULT_KEYS.contains(&key) => validate_default_child(key, child, path, key)?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_defaults_keys(value: &Value, path: &Path, label: &str) -> Result<()> {
    validate_mapping(value, path, label, DEFAULT_KEYS)?;
    for (key, child) in mapping_entries(value, path, label)? {
        validate_default_child(&key, child, path, &format!("{label}.{key}"))?;
    }
    Ok(())
}

fn validate_workflow_override_keys(value: &Value, path: &Path, label: &str) -> Result<()> {
    validate_mapping(value, path, label, WORKFLOW_OVERRIDE_KEYS)?;
    for (key, child) in mapping_entries(value, path, label)? {
        match key.as_str() {
            "branches" => validate_mapping(child, path, &format!("{label}.branches"), BRANCH_KEYS)?,
            "artifacts" => {
                validate_mapping(child, path, &format!("{label}.artifacts"), ARTIFACT_KEYS)?
            }
            "execution" => {
                validate_mapping(child, path, &format!("{label}.execution"), EXECUTION_KEYS)?
            }
            "container" => {
                validate_mapping(child, path, &format!("{label}.container"), CONTAINER_KEYS)?
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_default_child(key: &str, value: &Value, path: &Path, label: &str) -> Result<()> {
    match key {
        "branches" => validate_mapping(value, path, label, BRANCH_KEYS),
        "container" => validate_mapping(value, path, label, CONTAINER_KEYS),
        _ => Ok(()),
    }
}

fn validate_mapping(value: &Value, path: &Path, label: &str, allowed: &[&str]) -> Result<()> {
    for (key, _) in mapping_entries(value, path, label)? {
        if !allowed.contains(&key.as_str()) {
            return Err(CiError::Usage(format!(
                "{} has unknown key `{}` in {}; run `ci schema {}` for supported fields",
                path.display(),
                key,
                label,
                if label.starts_with("workflows") || label.starts_with("hooks") {
                    "workflow"
                } else {
                    "config"
                }
            )));
        }
    }
    Ok(())
}

fn mapping_entries<'a>(
    value: &'a Value,
    path: &Path,
    label: &str,
) -> Result<Vec<(String, &'a Value)>> {
    let Some(mapping) = value.as_mapping() else {
        return Err(CiError::Usage(format!(
            "{} section `{label}` must be a mapping",
            path.display()
        )));
    };
    mapping
        .iter()
        .map(|(key, value)| {
            key.as_str()
                .map(|key| (key.to_string(), value))
                .ok_or_else(|| {
                    CiError::Usage(format!(
                        "{} section `{label}` contains a non-string key",
                        path.display()
                    ))
                })
        })
        .collect()
}

const ROOT_CONFIG_KEYS: &[&str] = &[
    "shell",
    "quiet",
    "fail_fast",
    "fail-fast",
    "tech",
    "type",
    "tech-stack",
    "tech_stack",
    "arch",
    "container",
    "container_runtime",
    "container-runtime",
    "git_mode",
    "git-mode",
    "git_command",
    "git-command",
    "git_image",
    "git-image",
    "install_mode",
    "install-mode",
    "recursive_checkout",
    "recursive-checkout",
    "artifact_store",
    "artifact-store",
    "actions_cache",
    "actions-cache",
    "other_workflows",
    "other-workflows",
    "branches",
    "defaults",
    "policy",
    "locked",
    "hooks",
    "workflows",
    "actions",
];

const DEFAULT_KEYS: &[&str] = &[
    "shell",
    "quiet",
    "fail_fast",
    "fail-fast",
    "tech",
    "type",
    "tech-stack",
    "tech_stack",
    "arch",
    "container",
    "container_runtime",
    "container-runtime",
    "git_mode",
    "git-mode",
    "git_command",
    "git-command",
    "git_image",
    "git-image",
    "install_mode",
    "install-mode",
    "recursive_checkout",
    "recursive-checkout",
    "artifact_store",
    "artifact-store",
    "actions_cache",
    "actions-cache",
    "branches",
];

const WORKFLOW_OVERRIDE_KEYS: &[&str] = &[
    "on",
    "tech",
    "type",
    "tech-stack",
    "tech_stack",
    "arch",
    "branches",
    "artifacts",
    "execution",
    "container",
    "env",
];

const CONTAINER_KEYS: &[&str] = &[
    "type",
    "image",
    "platform",
    "workdir",
    "working-directory",
    "working_directory",
    "readonly",
    "read-only",
    "read_only",
    "arch",
    "packages",
    "components",
    "env",
    "volumes",
];

const BRANCH_KEYS: &[&str] = &["allow", "only"];
const ARTIFACT_KEYS: &[&str] = &["paths", "mode", "destination"];
const EXECUTION_KEYS: &[&str] = &["workspace", "shell"];
const ACTIONS_KEYS: &[&str] = &["node_image", "node-image"];
