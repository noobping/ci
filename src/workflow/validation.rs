use std::path::Path;

use serde_yaml::Value;

use crate::error::{CiError, Result};

pub(crate) fn validate_native_workflow_keys(value: &Value, path: &Path) -> Result<()> {
    validate_mapping(value, path, "workflow", NATIVE_WORKFLOW_KEYS)?;
    for (key, child) in mapping_entries(value, path, "workflow")? {
        match key.as_str() {
            "defaults" => validate_workflow_override_section(child, path, "defaults")?,
            "branches" => validate_mapping(child, path, "branches", BRANCH_KEYS)?,
            "artifacts" => validate_mapping(child, path, "artifacts", ARTIFACT_KEYS)?,
            "execution" => validate_mapping(child, path, "execution", EXECUTION_KEYS)?,
            "container" => validate_mapping(child, path, "container", CONTAINER_KEYS)?,
            _ => {}
        }
    }
    Ok(())
}

fn validate_workflow_override_section(value: &Value, path: &Path, label: &str) -> Result<()> {
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

fn validate_mapping(value: &Value, path: &Path, label: &str, allowed: &[&str]) -> Result<()> {
    for (key, _) in mapping_entries(value, path, label)? {
        if !allowed.contains(&key.as_str()) {
            return Err(CiError::Usage(format!(
                "{} has unknown key `{}` in {}; run `ci schema workflow` for supported fields",
                path.display(),
                key,
                label
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

const NATIVE_WORKFLOW_KEYS: &[&str] = &[
    "name",
    "defaults",
    "on",
    "needs",
    "requires",
    "depends",
    "dependencies",
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
    "steps",
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
