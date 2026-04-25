use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{CiError, Result};
use crate::git::CleanIgnoredMode;

use super::inputs::{parse_bool, parse_path_list};

pub(crate) fn parse_cleanup_ignored_mode(
    step_name: &str,
    value: Option<&str>,
) -> Result<CleanIgnoredMode> {
    match value.map(str::trim).map(|value| value.to_ascii_lowercase()) {
        None => Ok(CleanIgnoredMode::Exclude),
        Some(value) if matches!(value.as_str(), "0" | "false" | "no" | "off") => {
            Ok(CleanIgnoredMode::Exclude)
        }
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on") => {
            Ok(CleanIgnoredMode::Include)
        }
        Some(value) if value == "only" => Ok(CleanIgnoredMode::Only),
        Some(value) => Err(CiError::Usage(format!(
            "{step_name} `ignored` must be one of `false`, `true`, or `only`; got `{value}`"
        ))),
    }
}

pub(crate) fn cleanup_repo_paths(
    root: &Path,
    path: Option<&str>,
    paths: Option<&str>,
    missing_ok: Option<&str>,
) -> Result<()> {
    let spec = path.or(paths).unwrap_or("");
    let missing_ok = missing_ok.map(parse_bool).unwrap_or(true);
    let targets = parse_path_list(spec);
    if targets.is_empty() {
        return Err(CiError::Message(
            "cleanup requires `with.path` or `with.paths`".to_string(),
        ));
    }

    for target in targets {
        let path = resolve_cleanup_path(root, &target)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_dir() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && missing_ok => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(CiError::Message(format!(
                    "cleanup target does not exist: {}",
                    path.display()
                )))
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(())
}

fn resolve_cleanup_path(root: &Path, value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(CiError::Message(format!(
            "cleanup only supports repo-relative paths, got {}",
            path.display()
        )));
    }

    let mut resolved = root.to_path_buf();
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(item) => {
                resolved.push(item);
                depth += 1;
            }
            Component::ParentDir => {
                if depth == 0 {
                    return Err(CiError::Message(format!(
                        "cleanup path escapes repository root: {value}"
                    )));
                }
                resolved.pop();
                depth -= 1;
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(CiError::Message(format!(
                    "cleanup only supports repo-relative paths, got {value}"
                )))
            }
        }
    }

    Ok(resolved)
}
