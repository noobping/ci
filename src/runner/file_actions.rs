use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use glob::glob;

use crate::error::{CiError, Result};

use super::file_system::copy_recursively;
use super::inputs::{input_bool, input_value, parse_path_list};

const FILE_SOURCE_INPUT_KEYS: &[&str] = &[
    "source", "sources", "src", "srcs", "from", "froms", "path", "paths",
];
const FILE_DESTINATION_INPUT_KEYS: &[&str] =
    &["destination", "destenation", "dest", "dst", "to", "target"];

pub(crate) fn run_export_step(
    root: &Path,
    rendered_with: &BTreeMap<String, String>,
) -> Result<i32> {
    let source = input_value(rendered_with, FILE_SOURCE_INPUT_KEYS).ok_or_else(|| {
        CiError::Message("export requires `source`, `src`, or `from`".to_string())
    })?;
    let destination = input_value(rendered_with, FILE_DESTINATION_INPUT_KEYS).ok_or_else(|| {
        CiError::Message("export requires `destination`, `dest`, or `to`".to_string())
    })?;

    let specs = parse_path_list(source);
    if specs.is_empty() {
        return Err(CiError::Message(
            "export requires at least one source path".to_string(),
        ));
    }

    let replace = input_bool(rendered_with, &["replace", "overwrite"], false);
    let sources = expand_action_sources(root, &specs, "export")?;
    let destination_path = resolve_export_path(root, destination);
    for source in &sources {
        let target = export_target_path(source, &destination_path, destination, sources.len())?;
        copy_export_path(source, &target, replace)?;
    }
    Ok(0)
}

pub(crate) fn run_link_step(root: &Path, rendered_with: &BTreeMap<String, String>) -> Result<i32> {
    let source = input_value(rendered_with, FILE_SOURCE_INPUT_KEYS)
        .ok_or_else(|| CiError::Message("link requires `source`, `src`, or `from`".to_string()))?;
    let destination = input_value(rendered_with, FILE_DESTINATION_INPUT_KEYS).ok_or_else(|| {
        CiError::Message("link requires `destination`, `dest`, or `to`".to_string())
    })?;

    let specs = parse_path_list(source);
    if specs.is_empty() {
        return Err(CiError::Message(
            "link requires at least one source path".to_string(),
        ));
    }

    let replace = input_bool(rendered_with, &["replace", "overwrite"], false);
    let sources = expand_action_sources(root, &specs, "link")?;
    let destination_path = resolve_export_path(root, destination);
    for source in &sources {
        let target = export_target_path(source, &destination_path, destination, sources.len())?;
        create_link_path(source, &target, replace)?;
    }
    Ok(0)
}

fn expand_action_sources(root: &Path, specs: &[String], action: &str) -> Result<Vec<PathBuf>> {
    let mut sources = Vec::new();
    for spec in specs {
        let pattern_path = resolve_export_path(root, spec);
        let pattern = pattern_path.to_str().ok_or_else(|| {
            CiError::Message(format!(
                "invalid {action} source {}",
                pattern_path.display()
            ))
        })?;
        let before = sources.len();
        for entry in glob(pattern)? {
            let path = entry?;
            if path.exists() {
                sources.push(path);
            }
        }
        if sources.len() == before {
            return Err(CiError::Message(format!(
                "{action} source matched no paths: {spec}"
            )));
        }
    }
    Ok(sources)
}

fn resolve_export_path(root: &Path, value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn export_target_path(
    source: &Path,
    destination: &Path,
    raw_destination: &str,
    source_count: usize,
) -> Result<PathBuf> {
    if source_count == 1
        && !destination.is_dir()
        && !raw_destination.ends_with('/')
        && !raw_destination.ends_with(std::path::MAIN_SEPARATOR)
    {
        return Ok(destination.to_path_buf());
    }

    let name = source.file_name().ok_or_else(|| {
        CiError::Message(format!(
            "cannot export {} into a directory without a file name",
            source.display()
        ))
    })?;
    Ok(destination.join(name))
}

fn copy_export_path(source: &Path, target: &Path, replace: bool) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if replace => {
            if metadata.file_type().is_dir() {
                fs::remove_dir_all(target)?;
            } else {
                fs::remove_file(target)?;
            }
        }
        Ok(_) => {
            return Err(CiError::Message(format!(
                "export target already exists: {}; set `replace: true` or `overwrite: true`",
                target.display()
            )));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    copy_recursively(source, target)
}

fn create_link_path(source: &Path, target: &Path, replace: bool) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if replace => {
            if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(target)?;
            } else {
                fs::remove_file(target)?;
            }
        }
        Ok(_) => {
            return Err(CiError::Message(format!(
                "link target already exists: {}; set `replace: true` or `overwrite: true`",
                target.display()
            )));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(link_source_for_target(source, target), target)?;
    Ok(())
}

fn link_source_for_target(source: &Path, target: &Path) -> PathBuf {
    target
        .parent()
        .and_then(|parent| relative_path_between(parent, source))
        .unwrap_or_else(|| source.to_path_buf())
}

fn relative_path_between(from: &Path, to: &Path) -> Option<PathBuf> {
    if from.is_absolute() != to.is_absolute() {
        return None;
    }

    let from_components = lexical_components(from)?;
    let to_components = lexical_components(to)?;
    let common = from_components
        .iter()
        .zip(&to_components)
        .take_while(|(left, right)| left == right)
        .count();

    let mut relative = PathBuf::new();
    for _ in common..from_components.len() {
        relative.push("..");
    }
    for component in &to_components[common..] {
        relative.push(component);
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Some(relative)
}

fn lexical_components(path: &Path) -> Option<Vec<std::ffi::OsString>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) => return None,
            Component::RootDir => components.push(std::ffi::OsString::from("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                if components
                    .last()
                    .map(|value| value != ".." && value != "/")
                    .unwrap_or(false)
                {
                    components.pop();
                } else {
                    components.push(std::ffi::OsString::from(".."));
                }
            }
            Component::Normal(value) => components.push(value.to_os_string()),
        }
    }
    Some(components)
}
