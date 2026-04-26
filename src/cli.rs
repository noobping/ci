use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::config::{
    Architecture, ArtifactMode, ColorWhen, ContainerRuntime, ContainerType, GitMode, InstallMode,
};
use crate::workflow::is_known_hook;

#[derive(Clone, Debug, Parser)]
#[command(name = "ci")]
#[command(version)]
#[command(about = "small Git-native CI runner and build tool.")]
#[command(disable_help_subcommand = false)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOptions,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Clone, Debug, Args)]
pub struct GlobalOptions {
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count, global = true)]
    pub verbose: u8,

    #[arg(
        short = 'q',
        long = "quiet",
        global = true,
        help = "Reduce normal output"
    )]
    pub quiet: bool,

    #[arg(
        long = "silent",
        global = true,
        help = "Alias for --quiet, useful for hooks and timers"
    )]
    pub silent: bool,

    #[arg(long = "repo", global = true, default_value = ".")]
    pub repo: PathBuf,

    #[arg(long = "ci-dir", global = true, default_value = ".ci")]
    pub ci_dir: PathBuf,

    #[arg(long = "config", global = true)]
    pub config: Option<PathBuf>,

    #[arg(long = "color", global = true, default_value_t = ColorWhen::Auto)]
    pub color: ColorWhen,

    #[arg(long = "git-mode", global = true)]
    pub git_mode: Option<GitMode>,

    #[arg(long = "git-image", global = true)]
    pub git_image: Option<String>,

    #[arg(
        short = 'c',
        long = "container",
        global = true,
        conflicts_with = "no_container",
        help = "Force native workflows to run steps in a container"
    )]
    pub container: bool,

    #[arg(
        short = 'C',
        long = "no-container",
        global = true,
        conflicts_with = "container",
        help = "Disable configured containers for native workflows"
    )]
    pub no_container: bool,

    #[arg(long = "arch", global = true, value_delimiter = ',')]
    pub arch: Vec<Architecture>,

    #[arg(
        short = 't',
        long = "tech",
        alias = "type",
        alias = "tech-stack",
        global = true,
        help = "Select the project tech stack for generated workflows and auto containers"
    )]
    pub tech_stack: Option<ContainerType>,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Commands {
    Run(RunArgs),
    #[command(alias = "ls")]
    List(ListArgs),
    Install(InstallArgs),
    #[command(alias = "remove")]
    Uninstall(UninstallArgs),
    Update(UpdateArgs),
    Hook(HookArgs),
    #[command(alias = "doctor")]
    Status(StatusArgs),
    Explain(ExplainArgs),
    Schema(SchemaArgs),
    Clean(CleanArgs),
    Completion(CompletionArgs),
    Man(ManArgs),
    Init(InitArgs),
    #[command(name = "self")]
    SelfCmd(SelfArgs),
}

#[derive(Clone, Debug, Args, Default)]
pub struct ListArgs {
    #[arg(
        long = "porcelain",
        conflicts_with = "no_porcelain",
        help = "Use stable tab-separated output"
    )]
    pub porcelain: bool,

    #[arg(long = "no-porcelain", help = "Keep aligned human-readable output")]
    pub no_porcelain: bool,
}

impl ListArgs {
    pub fn use_porcelain(&self, stdout_is_terminal: bool) -> bool {
        if self.porcelain {
            true
        } else if self.no_porcelain {
            false
        } else {
            !stdout_is_terminal
        }
    }
}

#[derive(Clone, Debug, Args)]
pub struct RunArgs {
    #[arg(value_name = "WORKFLOW")]
    pub workflow: Option<String>,

    #[arg(long = "event", default_value = "manual")]
    pub event: String,

    #[arg(long = "all")]
    pub all: bool,

    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long = "fail-fast")]
    pub fail_fast: bool,

    #[arg(long = "keep-going")]
    pub keep_going: bool,

    #[arg(long = "container-runtime")]
    pub container_runtime: Option<ContainerRuntime>,

    #[arg(long = "respect-branches")]
    pub respect_branches: bool,

    #[arg(long = "no-recursive-checkout")]
    pub no_recursive_checkout: bool,

    #[arg(long = "lock")]
    pub lock: bool,
}

#[derive(Clone, Debug, Args)]
pub struct InstallArgs {
    #[arg(long = "mode")]
    pub mode: Option<InstallMode>,

    #[arg(
        long = "source",
        help = "Binary source to install; use {arch} for per-architecture sources"
    )]
    pub source: Option<PathBuf>,

    #[arg(long = "hooks")]
    pub hooks: Option<String>,

    #[arg(long = "bare")]
    pub bare: bool,

    #[arg(long = "force")]
    pub force: bool,

    #[arg(long = "backup-existing")]
    pub backup_existing: bool,

    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Args)]
pub struct UninstallArgs {
    #[arg(long = "hooks")]
    pub hooks: Option<String>,

    #[arg(long = "keep-binary")]
    pub keep_binary: bool,

    #[arg(long = "restore")]
    pub restore: bool,

    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Args)]
pub struct UpdateArgs {
    #[arg(
        long = "source",
        help = "Binary source to install; use {arch} for per-architecture sources"
    )]
    pub source: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Args)]
pub struct HookArgs {
    pub hook: String,

    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub hook_args: Vec<String>,
}

#[derive(Clone, Debug, Args, Default)]
pub struct StatusArgs {}

#[derive(Clone, Debug, Args)]
pub struct ExplainArgs {
    pub subject: String,
}

#[derive(Clone, Debug, Args)]
pub struct SchemaArgs {
    #[arg(value_name = "config|workflow|all")]
    pub subject: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub struct CleanArgs {
    pub workflow: Option<String>,

    #[arg(long = "run-id")]
    pub run_id: Option<String>,

    #[arg(long = "mode", default_value = "keep")]
    pub mode: ArtifactMode,

    #[arg(long = "dest")]
    pub dest: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Args)]
pub struct InitArgs {
    #[arg(long = "force")]
    pub force: bool,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum CompletionShell {
    Bash,
}

#[derive(Clone, Debug, Args)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: CompletionShell,

    #[arg(long = "output")]
    pub output: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub struct ManArgs {
    #[arg(long = "dir")]
    pub dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Args, Default)]
pub struct SelfArgs {}

pub fn rewrite_argv(mut argv: Vec<OsString>) -> Vec<OsString> {
    if argv.is_empty() {
        return argv;
    }

    if let Some(name) = argv
        .first()
        .and_then(|value| std::path::Path::new(value).file_name())
        .and_then(OsStr::to_str)
        .map(str::to_string)
    {
        if is_known_hook(&name) {
            argv.insert(1, OsString::from("hook"));
            argv.insert(2, OsString::from(name));
            return argv;
        }
    }

    if let Some(index) = find_command_index(&argv) {
        if argv[index] == "doctor" {
            argv[index] = OsString::from("status");
        } else if !is_known_command(&argv[index]) {
            argv.insert(index, OsString::from("run"));
        }
    }

    argv
}

pub fn doctor_alias_used(argv: &[OsString]) -> bool {
    find_command_index(argv)
        .map(|index| argv[index] == "doctor")
        .unwrap_or(false)
}

fn find_command_index(argv: &[OsString]) -> Option<usize> {
    let mut i = 1;
    while i < argv.len() {
        let current = argv[i].to_string_lossy();
        match current.as_ref() {
            "--repo" | "--ci-dir" | "--config" | "--color" | "--git-mode" | "--git-image"
            | "--arch" | "--type" | "--tech" | "--tech-stack" | "-t" => {
                i += 2;
            }
            value if value.starts_with('-') => {
                i += 1;
            }
            _ => return Some(i),
        }
    }
    None
}

fn is_known_command(command: &OsStr) -> bool {
    matches!(
        command.to_str(),
        Some(
            "run"
                | "list"
                | "ls"
                | "install"
                | "uninstall"
                | "remove"
                | "update"
                | "hook"
                | "doctor"
                | "status"
                | "explain"
                | "schema"
                | "clean"
                | "completion"
                | "man"
                | "init"
                | "self"
                | "help"
        )
    )
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
