use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::cli::GlobalOptions;
use crate::config::{Defaults, GitCommand, GitMode};
use crate::error::{CiError, Result};
use crate::output::Output as CliOutput;
use crate::repo::RepoInfo;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanIgnoredMode {
    Exclude,
    Include,
    Only,
}

#[derive(Clone, Debug)]
pub struct GitService {
    mode: GitMode,
    command: Option<GitCommand>,
    image: String,
    output: CliOutput,
}

impl GitService {
    pub fn bootstrap(global: &GlobalOptions, output: CliOutput) -> Self {
        Self {
            mode: global.git_mode.unwrap_or(GitMode::Auto),
            command: global.git_command.clone(),
            image: global
                .git_image
                .clone()
                .unwrap_or_else(|| "docker.io/alpine/git:latest".to_string()),
            output,
        }
    }

    pub fn configured(defaults: &Defaults, _global: &GlobalOptions, output: CliOutput) -> Self {
        Self {
            mode: defaults.git_mode,
            command: defaults.git_command.clone(),
            image: defaults.git_image.clone(),
            output,
        }
    }

    pub fn mode(&self) -> GitMode {
        self.mode
    }

    pub fn command(&self) -> Option<&GitCommand> {
        self.command.as_ref()
    }

    pub fn output_in_dir(&self, dir: &Path, args: &[&str]) -> Result<String> {
        let output = self.run_git(dir, args)?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(CiError::Message(format!(
                "git failed in {}: {}",
                dir.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    pub fn status_in_dir(&self, dir: &Path, args: &[&str]) -> Result<i32> {
        Ok(self.run_git(dir, args)?.status.code().unwrap_or(1))
    }

    pub fn ensure_submodules(&self, repo: &RepoInfo) -> Result<()> {
        self.output
            .verbose(format!("updating submodules in {}", repo.root.display()));
        let status = self.status_in_dir(
            &repo.root,
            &["submodule", "update", "--init", "--recursive"],
        )?;
        if status == 0 {
            Ok(())
        } else {
            Err(CiError::Message(format!(
                "git submodule update failed in {} with exit code {status}",
                repo.root.display()
            )))
        }
    }

    pub fn restore_tracked_files(&self, repo: &RepoInfo) -> Result<()> {
        self.output.verbose(format!(
            "restoring tracked files in {}",
            repo.root.display()
        ));
        if self.status_in_dir(&repo.root, &["rev-parse", "--verify", "HEAD"])? != 0 {
            return Ok(());
        }

        let status = self.status_in_dir(&repo.root, &["reset", "--hard", "HEAD"])?;
        if status == 0 {
            Ok(())
        } else {
            Err(CiError::Message(format!(
                "git reset --hard failed in {} with exit code {status}",
                repo.root.display()
            )))
        }
    }

    pub fn clean_untracked_files(&self, repo: &RepoInfo, ignored: CleanIgnoredMode) -> Result<()> {
        self.output.verbose(format!(
            "cleaning untracked files in {}",
            repo.root.display()
        ));
        let args = match ignored {
            CleanIgnoredMode::Exclude => vec!["clean", "-fd"],
            CleanIgnoredMode::Include => vec!["clean", "-fdx"],
            CleanIgnoredMode::Only => vec!["clean", "-fdX"],
        };
        let status = self.status_in_dir(&repo.root, &args)?;
        if status == 0 {
            Ok(())
        } else {
            Err(CiError::Message(format!(
                "git clean failed in {} with exit code {status}",
                repo.root.display()
            )))
        }
    }

    pub fn fetch_prune(&self, repo: &RepoInfo) -> Result<()> {
        self.output
            .verbose(format!("pruning remote refs in {}", repo.root.display()));
        let status = self.status_in_dir(&repo.root, &["fetch", "--all", "--prune"])?;
        if status == 0 {
            Ok(())
        } else {
            Err(CiError::Message(format!(
                "git fetch --all --prune failed in {} with exit code {status}",
                repo.root.display()
            )))
        }
    }

    pub fn current_branch(&self, repo: &RepoInfo) -> Result<Option<String>> {
        let branch = self.output_in_dir(&repo.root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        if branch == "HEAD" {
            Ok(None)
        } else {
            Ok(Some(branch))
        }
    }

    pub fn clone_action_repo(
        &self,
        cache_root: &Path,
        provider_base: &str,
        owner: &str,
        repo: &str,
        reference: &str,
    ) -> Result<PathBuf> {
        std::fs::create_dir_all(cache_root)?;
        let repo_dir = cache_root.join(format!(
            "{}__{}__{}",
            owner,
            repo,
            sanitize_component(reference)
        ));
        if repo_dir.exists() {
            return Ok(repo_dir);
        }

        let parent = repo_dir.parent().ok_or_else(|| {
            CiError::Message("could not determine action cache parent".to_string())
        })?;
        let target_name = repo_dir
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| CiError::Message("invalid action cache path".to_string()))?;
        let remote = format!("{provider_base}/{owner}/{repo}.git");

        let args = [
            "clone",
            "--depth",
            "1",
            "--branch",
            reference,
            &remote,
            target_name,
        ];
        let status = self.status_in_dir(parent, &args)?;
        if status == 0 {
            Ok(repo_dir)
        } else {
            Err(CiError::Message(format!(
                "failed to clone action {owner}/{repo}@{reference}"
            )))
        }
    }

    fn run_git(&self, dir: &Path, args: &[&str]) -> Result<Output> {
        let args = git_args_for_verbosity(args, self.output.is_verbose(), self.output.is_quiet());
        self.output.verbose(format!("git {}", args.join(" ")));
        let mode = self.execution_mode(dir)?;
        self.output
            .verbose_at(2, format!("git execution mode: {}", mode.description()));
        self.output
            .verbose_at(3, format!("git working directory: {}", dir.display()));
        match mode {
            ExecutionMode::Custom(command) => self.run_git_command(dir, &command, &args),
            ExecutionMode::FlatpakHost => Command::new("flatpak-spawn")
                .arg("--host")
                .arg("git")
                .current_dir(dir)
                .args(&args)
                .output()
                .map_err(Into::into),
            ExecutionMode::Host => Command::new("git")
                .current_dir(dir)
                .args(&args)
                .output()
                .map_err(Into::into),
            ExecutionMode::Container(runtime) => self.run_git_container(&runtime, dir, &args),
        }
    }

    fn execution_mode(&self, dir: &Path) -> Result<ExecutionMode> {
        match self.mode {
            GitMode::Custom => self
                .command
                .clone()
                .map(ExecutionMode::Custom)
                .ok_or_else(|| CiError::Usage("git-mode custom requires git-command".to_string())),
            GitMode::Flatpak => Ok(ExecutionMode::FlatpakHost),
            GitMode::Host => Ok(ExecutionMode::Host),
            GitMode::Auto => {
                if let Some(command) = &self.command {
                    Ok(ExecutionMode::Custom(command.clone()))
                } else if flatpak_host_available_in_dir(dir) {
                    Ok(ExecutionMode::FlatpakHost)
                } else if command_exists("git") {
                    Ok(ExecutionMode::Host)
                } else {
                    Ok(ExecutionMode::Container(preferred_container_runtime()))
                }
            }
            GitMode::Alias => Ok(ExecutionMode::Container(preferred_container_runtime())),
        }
    }

    fn run_git_command(
        &self,
        dir: &Path,
        git_command: &GitCommand,
        args: &[&str],
    ) -> Result<Output> {
        let Some((program, command_args)) = git_command.parts().split_first() else {
            return Err(CiError::Message(
                "git command must not be empty".to_string(),
            ));
        };
        Command::new(program)
            .current_dir(dir)
            .args(command_args)
            .args(args)
            .output()
            .map_err(Into::into)
    }

    fn run_git_container(&self, runtime: &str, dir: &Path, args: &[&str]) -> Result<Output> {
        let parent = if dir.is_dir() {
            dir
        } else {
            dir.parent()
                .ok_or_else(|| CiError::Message(format!("cannot mount {}", dir.display())))?
        };
        let mount_target = "/work";
        let workdir = if dir == parent {
            mount_target.to_string()
        } else {
            format!(
                "{mount_target}/{}",
                dir.strip_prefix(parent).unwrap_or(dir).display()
            )
        };
        let mount = format!("{}:{mount_target}", parent.display());

        let mut command = Command::new(runtime);
        command
            .arg("run")
            .arg("--rm")
            .arg("-v")
            .arg(mount)
            .arg("-w")
            .arg(workdir)
            .arg(&self.image)
            .arg("git");

        for arg in args {
            command.arg(arg);
        }

        command.output().map_err(Into::into)
    }
}

fn git_args_for_verbosity<'a>(args: &'a [&'a str], verbose: bool, quiet: bool) -> Vec<&'a str> {
    if args.is_empty() || has_quiet_arg(args) || has_verbose_arg(args) {
        return args.to_vec();
    }

    let command = args[0];

    if verbose {
        let supports_verbose = matches!(
            command,
            "add" | "clone" | "fetch" | "pull" | "push" | "status"
        );
        if !supports_verbose {
            return args.to_vec();
        }

        let mut verbose_args = Vec::with_capacity(args.len() + 1);
        verbose_args.push(command);
        verbose_args.push("--verbose");
        verbose_args.extend_from_slice(&args[1..]);
        return verbose_args;
    }

    if quiet {
        let Some(quiet_flag) = quiet_flag_for_command(command) else {
            return args.to_vec();
        };

        let mut quiet_args = Vec::with_capacity(args.len() + 1);
        quiet_args.push(command);
        quiet_args.push(quiet_flag);
        quiet_args.extend_from_slice(&args[1..]);
        return quiet_args;
    }

    args.to_vec()
}

fn quiet_flag_for_command(command: &str) -> Option<&'static str> {
    if supports_quiet(command) {
        Some("--quiet")
    } else if supports_no_quiet_or_silent(command) {
        None
    } else {
        Some("--silent")
    }
}

fn supports_quiet(command: &str) -> bool {
    matches!(
        command,
        "add"
            | "checkout"
            | "clean"
            | "clone"
            | "commit"
            | "fetch"
            | "pull"
            | "push"
            | "reset"
            | "restore"
            | "submodule"
    )
}

fn supports_no_quiet_or_silent(command: &str) -> bool {
    matches!(command, "rev-parse" | "status")
}

fn has_quiet_arg(args: &[&str]) -> bool {
    args.iter()
        .any(|arg| matches!(*arg, "-q" | "--quiet" | "--silent"))
}

fn has_verbose_arg(args: &[&str]) -> bool {
    args.iter()
        .any(|arg| matches!(*arg, "-v" | "--verbose" | "--verbose=true"))
}

enum ExecutionMode {
    Custom(GitCommand),
    FlatpakHost,
    Host,
    Container(String),
}

impl ExecutionMode {
    fn description(&self) -> String {
        match self {
            Self::Custom(command) => format!("custom `{}`", command.render()),
            Self::FlatpakHost => "flatpak host".to_string(),
            Self::Host => "host".to_string(),
            Self::Container(runtime) => format!("container via {runtime}"),
        }
    }
}

fn running_in_flatpak() -> bool {
    env::var_os("FLATPAK_ID")
        .filter(|value| !value.is_empty())
        .is_some()
        || Path::new("/.flatpak-info").exists()
}

fn flatpak_host_available_in_dir(dir: &Path) -> bool {
    running_in_flatpak()
        && command_exists("flatpak-spawn")
        && Command::new("flatpak-spawn")
            .arg("--host")
            .arg("true")
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
}

pub(crate) fn flatpak_host_command_exists(name: &str) -> bool {
    running_in_flatpak()
        && command_exists("flatpak-spawn")
        && Command::new("flatpak-spawn")
            .arg("--host")
            .arg("sh")
            .arg("-c")
            .arg(format!("command -v {name} >/dev/null 2>&1"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
}

pub fn command_exists(name: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name} >/dev/null 2>&1"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn preferred_container_runtime() -> String {
    if command_exists("podman") {
        "podman".to_string()
    } else {
        "docker".to_string()
    }
}

pub fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

pub fn render_command(command: &Command) -> String {
    let mut parts = Vec::new();
    parts.push(command.get_program().to_string_lossy().to_string());
    for arg in command.get_args() {
        parts.push(arg.to_string_lossy().to_string());
    }
    parts.join(" ")
}

pub fn env_pairs(values: &[(String, String)]) -> Vec<OsString> {
    values
        .iter()
        .map(|(key, value)| OsString::from(format!("{key}={value}")))
        .collect()
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
