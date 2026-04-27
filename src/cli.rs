use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::config::{
    Architecture, ArtifactMode, ColorWhen, ContainerRuntime, ContainerType, GitCommand, GitMode,
    InstallMode,
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
    #[arg(
        short = 'v',
        long = "verbose",
        action = ArgAction::Count,
        global = true,
        help = "Increase output detail; repeat up to -vvv for extra runner detail"
    )]
    pub verbose: u8,

    #[arg(
        short = 'q',
        long = "quiet",
        global = true,
        help = "Hide non-error output"
    )]
    pub quiet: bool,

    #[arg(
        long = "repo",
        alias = "repository",
        global = true,
        default_value = ".",
        help = "Repository root to operate on"
    )]
    pub repo: PathBuf,

    #[arg(
        long = "ci-dir",
        global = true,
        default_value = ".ci",
        help = "Directory containing native ci workflows"
    )]
    pub ci_dir: PathBuf,

    #[arg(long = "config", global = true, help = "Use an explicit config file")]
    pub config: Option<PathBuf>,

    #[arg(
        long = "color",
        global = true,
        default_value_t = ColorWhen::Auto,
        help = "Control colored output"
    )]
    pub color: ColorWhen,

    #[arg(
        long = "git-mode",
        global = true,
        help = "Choose how git commands are executed"
    )]
    pub git_mode: Option<GitMode>,

    #[arg(
        long = "git-command",
        global = true,
        value_name = "COMMAND",
        help = "Command used to run git, for example `flatpak-spawn --host git`"
    )]
    pub git_command: Option<GitCommand>,

    #[arg(
        long = "git-image",
        global = true,
        help = "Container image used when git runs in a container"
    )]
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

    #[arg(
        long = "arch",
        global = true,
        value_delimiter = ',',
        help = "Select target architectures, comma-separated or repeated"
    )]
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
    #[command(about = "Run one or more workflows")]
    Run(RunArgs),
    #[command(alias = "ls", about = "List discovered workflows")]
    List(ListArgs),
    #[command(about = "Install ci into a repository as Git hooks")]
    Install(InstallArgs),
    #[command(alias = "remove", about = "Remove ci-managed Git hooks and runners")]
    Uninstall(UninstallArgs),
    #[command(about = "Refresh the installed ci runner binary")]
    Update(UpdateArgs),
    #[command(about = "Run workflows for a Git hook invocation")]
    Hook(HookArgs),
    #[command(alias = "doctor", about = "Show repository and ci diagnostics")]
    Status(StatusArgs),
    #[command(about = "Explain why workflows would run")]
    Explain(ExplainArgs),
    #[command(about = "Print JSON schemas for config and workflow files")]
    Schema(SchemaArgs),
    #[command(about = "Export, keep, or remove recorded artifacts")]
    Clean(CleanArgs),
    #[command(about = "Generate shell completion scripts")]
    Completion(CompletionArgs),
    #[command(about = "Generate manual pages")]
    Man(ManArgs),
    #[command(about = "Create an initial native build workflow")]
    Init(InitArgs),
    #[command(name = "self", about = "Print information about the ci binary")]
    SelfCmd(SelfArgs),
    #[command(about = "Compare the installed repository ci runner with this binary")]
    Other(OtherArgs),
}

#[derive(Clone, Debug, Args, Default)]
pub struct ListArgs {
    #[arg(
        short = 'p',
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
    #[arg(value_name = "WORKFLOW", help = "Workflow name to run")]
    pub workflow: Option<String>,

    #[arg(
        value_name = "ARG",
        allow_hyphen_values = true,
        trailing_var_arg = true,
        help = "Arguments forwarded to the detected build step"
    )]
    pub args: Vec<String>,

    #[arg(
        short = 'e',
        long = "event",
        default_value = "manual",
        help = "Event name used to select workflows"
    )]
    pub event: String,

    #[arg(
        short = 'a',
        long = "all",
        help = "Run all workflows selected by the event"
    )]
    pub all: bool,

    #[arg(
        short = 'n',
        long = "dry-run",
        action = ArgAction::SetTrue,
        overrides_with = "no_dry_run",
        help = "Preview selected workflows without running steps"
    )]
    pub dry_run: bool,

    #[arg(
        long = "no-dry-run",
        action = ArgAction::SetTrue,
        overrides_with = "dry_run",
        help = "Run workflows even when --dry-run was set earlier"
    )]
    pub no_dry_run: bool,

    #[arg(
        short = 'f',
        long = "fail-fast",
        help = "Stop after the first failing workflow"
    )]
    pub fail_fast: bool,

    #[arg(
        short = 'k',
        long = "keep-going",
        help = "Continue running later workflows after failures"
    )]
    pub keep_going: bool,

    #[arg(
        long = "container-runtime",
        help = "Container runtime to use for containerized workflows"
    )]
    pub container_runtime: Option<ContainerRuntime>,

    #[arg(
        long = "respect-branches",
        help = "Apply branch filters during manual runs"
    )]
    pub respect_branches: bool,

    #[arg(
        long = "no-recursive-checkout",
        help = "Skip configured recursive submodule checkout before running"
    )]
    pub no_recursive_checkout: bool,

    #[arg(
        short = 'l',
        long = "lock",
        help = "Serialize this run with the repository ci lock"
    )]
    pub lock: bool,
}

#[derive(Clone, Debug, Args)]
pub struct InstallArgs {
    #[arg(short = 'm', long = "mode", help = "Install mode for hook runners")]
    pub mode: Option<InstallMode>,

    #[arg(
        short = 's',
        long = "source",
        help = "Binary source to install; use {arch} for per-architecture sources"
    )]
    pub source: Option<PathBuf>,

    #[arg(
        short = 'H',
        long = "hooks",
        help = "Hooks to manage, comma-separated, or `all`"
    )]
    pub hooks: Option<String>,

    #[arg(short = 'b', long = "bare", help = "Install into a bare repository")]
    pub bare: bool,

    #[arg(
        short = 'f',
        long = "force",
        help = "Overwrite existing unmanaged hook files"
    )]
    pub force: bool,

    #[arg(
        short = 'B',
        long = "backup-existing",
        help = "Back up existing hooks before replacing them"
    )]
    pub backup_existing: bool,

    #[arg(short = 'n', long = "dry-run", help = "Show what would be installed")]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Args)]
pub struct UninstallArgs {
    #[arg(
        short = 'H',
        long = "hooks",
        help = "Hooks to remove, comma-separated, or `all`"
    )]
    pub hooks: Option<String>,

    #[arg(
        short = 'k',
        long = "keep-binary",
        help = "Keep the installed runner binary"
    )]
    pub keep_binary: bool,

    #[arg(
        short = 'r',
        long = "restore",
        help = "Restore backups for removed hooks"
    )]
    pub restore: bool,

    #[arg(short = 'n', long = "dry-run", help = "Show what would be removed")]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Args)]
pub struct UpdateArgs {
    #[arg(
        short = 's',
        long = "source",
        help = "Binary source to install; use {arch} for per-architecture sources"
    )]
    pub source: Option<PathBuf>,

    #[arg(short = 'n', long = "dry-run", help = "Show what would be updated")]
    pub dry_run: bool,

    #[arg(
        short = 'a',
        long = "all",
        help = "Update ci in Git repositories directly in PATH or --repo"
    )]
    pub all: bool,

    #[arg(
        short = 'r',
        long = "recursive",
        help = "Recursively update ci in Git repositories under PATH or --repo"
    )]
    pub recursive: bool,

    #[arg(
        value_name = "PATH",
        help = "Repository to update, or search directory for --all/--recursive"
    )]
    pub path: Option<PathBuf>,
}

impl UpdateArgs {
    pub fn selected_update(&self) -> bool {
        self.all || self.recursive || self.path.is_some()
    }
}

#[derive(Clone, Debug, Args)]
pub struct HookArgs {
    #[arg(value_name = "HOOK", help = "Git hook name being invoked")]
    pub hook: String,

    #[arg(
        value_name = "ARG",
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Arguments passed by Git to the hook"
    )]
    pub hook_args: Vec<String>,
}

#[derive(Clone, Debug, Args, Default)]
pub struct StatusArgs {}

#[derive(Clone, Debug, Args)]
pub struct ExplainArgs {
    #[arg(value_name = "WORKFLOW", help = "Workflow name or subject to explain")]
    pub subject: String,
}

#[derive(Clone, Debug, Args)]
pub struct SchemaArgs {
    #[arg(value_name = "config|workflow|all", help = "Schema subject to print")]
    pub subject: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub struct CleanArgs {
    #[arg(
        value_name = "WORKFLOW",
        help = "Workflow whose artifacts should be cleaned"
    )]
    pub workflow: Option<String>,

    #[arg(
        short = 'r',
        long = "run-id",
        help = "Clean artifacts from a specific run id"
    )]
    pub run_id: Option<String>,

    #[arg(
        short = 'm',
        long = "mode",
        default_value = "keep",
        help = "How to handle matched artifacts"
    )]
    pub mode: ArtifactMode,

    #[arg(
        short = 'd',
        long = "dest",
        help = "Destination for exported artifacts"
    )]
    pub dest: Option<PathBuf>,

    #[arg(short = 'n', long = "dry-run", help = "Show what would be cleaned")]
    pub dry_run: bool,
}

#[derive(Clone, Debug, Args)]
pub struct InitArgs {
    #[arg(
        short = 'f',
        long = "force",
        help = "Replace an existing .ci/build.yml"
    )]
    pub force: bool,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum CompletionShell {
    Bash,
}

#[derive(Clone, Debug, Args)]
pub struct CompletionArgs {
    #[arg(value_enum, help = "Shell to generate completions for")]
    pub shell: CompletionShell,

    #[arg(short = 'o', long = "output", help = "Write completions to a file")]
    pub output: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
pub struct ManArgs {
    #[arg(
        short = 'd',
        long = "dir",
        help = "Directory to write generated man pages into"
    )]
    pub dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Args, Default)]
pub struct SelfArgs {}

#[derive(Clone, Debug, Args)]
pub struct OtherArgs {}

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
            "--repo" | "--repository" | "--ci-dir" | "--config" | "--color" | "--git-mode"
            | "--git-command" | "--git-image" | "--arch" | "--type" | "--tech" | "--tech-stack"
            | "-t" => {
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
                | "other"
                | "help"
        )
    )
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
