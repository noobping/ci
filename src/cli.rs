use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::config::{ArtifactMode, ColorWhen, ContainerRuntime, GitMode};
use crate::workflow::is_known_hook;

#[derive(Clone, Debug, Parser)]
#[command(name = "ci")]
#[command(version)]
#[command(about = "A small Git-native CI runner for Git repositories.")]
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

    #[arg(short = 'q', long = "quiet", global = true)]
    pub quiet: bool,

    #[arg(long = "silent", global = true)]
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
    Clean(CleanArgs),
    Completion(CompletionArgs),
    Man(ManArgs),
    Init(InitArgs),
    #[command(name = "self")]
    SelfCmd(SelfArgs),
}

#[derive(Clone, Debug, Args, Default)]
pub struct ListArgs {}

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
    #[arg(long = "mode", default_value = "link")]
    pub mode: InstallMode,

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

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum InstallMode {
    Link,
    Copy,
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
    #[arg(long = "source")]
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
        if argv[index] == OsString::from("doctor") {
            argv[index] = OsString::from("status");
        }
    }

    argv
}

pub fn doctor_alias_used(argv: &[OsString]) -> bool {
    find_command_index(argv)
        .map(|index| argv[index] == OsString::from("doctor"))
        .unwrap_or(false)
}

fn find_command_index(argv: &[OsString]) -> Option<usize> {
    let mut i = 1;
    while i < argv.len() {
        let current = argv[i].to_string_lossy();
        match current.as_ref() {
            "--repo" | "--ci-dir" | "--config" | "--color" | "--git-mode" | "--git-image" => {
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
