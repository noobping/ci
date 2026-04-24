use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::GlobalOptions;
use crate::config::Defaults;
use crate::error::{CiError, Result};
use crate::git::GitService;

#[derive(Clone, Debug)]
pub struct RepoInfo {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub ci_dir: PathBuf,
    pub is_bare: bool,
    pub current_exe: PathBuf,
    pub branch: Option<String>,
    pub state_dir: PathBuf,
    pub runs_dir: PathBuf,
    pub artifact_store: PathBuf,
    pub actions_cache: PathBuf,
}

impl RepoInfo {
    pub fn discover(global: &GlobalOptions, git: &GitService) -> Result<Self> {
        let repo_arg = absolute_path(&global.repo)?;
        let git_dir_raw = git.output_in_dir(&repo_arg, &["rev-parse", "--git-dir"])?;
        let is_bare =
            git.output_in_dir(&repo_arg, &["rev-parse", "--is-bare-repository"])? == "true";

        let git_dir_path = PathBuf::from(git_dir_raw.trim());
        let git_dir = if git_dir_path.is_absolute() {
            git_dir_path
        } else {
            repo_arg.join(git_dir_path)
        };
        let git_dir = canonicalize_best(&git_dir)?;

        let root = if is_bare {
            git_dir.clone()
        } else {
            canonicalize_best(Path::new(
                git.output_in_dir(&repo_arg, &["rev-parse", "--show-toplevel"])?
                    .trim(),
            ))?
        };

        let ci_dir = if global.ci_dir.is_absolute() {
            global.ci_dir.clone()
        } else {
            root.join(&global.ci_dir)
        };

        let current_exe = env::current_exe()
            .map_err(|err| CiError::Message(format!("could not find current executable: {err}")))?;
        let state_dir = git_dir.join("ci");

        Ok(Self {
            root,
            git_dir,
            ci_dir,
            is_bare,
            current_exe,
            branch: None,
            runs_dir: state_dir.join("runs"),
            artifact_store: state_dir.join("artifacts"),
            actions_cache: state_dir.join("actions-cache"),
            state_dir,
        })
    }

    pub fn apply_defaults(&mut self, defaults: &Defaults) {
        self.artifact_store = if defaults.artifact_store.is_absolute() {
            defaults.artifact_store.clone()
        } else {
            self.state_dir.join(&defaults.artifact_store)
        };
        self.actions_cache = if defaults.actions_cache.is_absolute() {
            defaults.actions_cache.clone()
        } else {
            self.state_dir.join(&defaults.actions_cache)
        };
        self.runs_dir = self.state_dir.join("runs");
    }

    pub fn refresh_branch(&mut self, git: &GitService) -> Result<()> {
        self.branch = git.current_branch(self)?;
        Ok(())
    }

    pub fn ensure_state_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        fs::create_dir_all(&self.runs_dir)?;
        fs::create_dir_all(&self.artifact_store)?;
        fs::create_dir_all(&self.actions_cache)?;
        Ok(())
    }
}

pub fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

pub fn canonicalize_best(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).or_else(|_| absolute_path(path))
}
