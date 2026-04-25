use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

use crate::config::{Architecture, ArtifactMode, ColorWhen, ContainerRuntime, GitMode};
use crate::workflow::is_known_hook;

#[derive(Clone, Debug, Parser)]
#[command(name = "ci")]
#[command(version)]
#[command(about = "small Git-native CI runner.")]
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
            | "--arch" => {
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
mod tests {
    use std::ffi::OsString;

    use clap::Parser;

    use super::{rewrite_argv, Cli, Commands, ListArgs};

    #[test]
    fn list_defaults_to_porcelain_when_stdout_is_not_a_terminal() {
        let args = ListArgs::default();

        assert!(!args.use_porcelain(true));
        assert!(args.use_porcelain(false));
    }

    #[test]
    fn list_porcelain_flags_override_auto_detection() {
        let porcelain = match Cli::try_parse_from(["ci", "list", "--porcelain"]).expect("parse") {
            Cli {
                command: Commands::List(args),
                ..
            } => args,
            _ => panic!("expected list command"),
        };
        assert!(porcelain.use_porcelain(true));
        assert!(porcelain.use_porcelain(false));

        let no_porcelain =
            match Cli::try_parse_from(["ci", "list", "--no-porcelain"]).expect("parse") {
                Cli {
                    command: Commands::List(args),
                    ..
                } => args,
                _ => panic!("expected list command"),
            };
        assert!(!no_porcelain.use_porcelain(true));
        assert!(!no_porcelain.use_porcelain(false));
    }

    #[test]
    fn list_porcelain_flags_conflict() {
        assert!(Cli::try_parse_from(["ci", "list", "--porcelain", "--no-porcelain"]).is_err());
    }

    #[test]
    fn unknown_command_is_rewritten_as_run_workflow() {
        let cli = Cli::try_parse_from(rewrite(["ci", "build"])).expect("parse");

        match cli.command {
            Commands::Run(args) => assert_eq!(args.workflow.as_deref(), Some("build")),
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn unknown_command_rewrite_keeps_global_options_before_workflow() {
        let cli = Cli::try_parse_from(rewrite([
            "ci",
            "--repo",
            "/tmp/project",
            "build",
            "--dry-run",
        ]))
        .expect("parse");

        assert_eq!(cli.global.repo, std::path::PathBuf::from("/tmp/project"));
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.workflow.as_deref(), Some("build"));
                assert!(args.dry_run);
            }
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn known_commands_and_aliases_are_not_rewritten_as_workflows() {
        let list = Cli::try_parse_from(rewrite(["ci", "list"])).expect("parse");
        assert!(matches!(list.command, Commands::List(_)));

        let list_alias = Cli::try_parse_from(rewrite(["ci", "ls"])).expect("parse");
        assert!(matches!(list_alias.command, Commands::List(_)));

        let status_alias = Cli::try_parse_from(rewrite(["ci", "doctor"])).expect("parse");
        assert!(matches!(status_alias.command, Commands::Status(_)));
    }

    #[test]
    fn arch_flag_normalizes_aliases() {
        let cli = Cli::try_parse_from(rewrite([
            "ci",
            "--arch",
            "amd64,arm64",
            "--arch",
            "x86_64",
            "run",
        ]))
        .expect("parse");

        assert_eq!(
            cli.global
                .arch
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec!["x64", "arm64", "x64"]
        );
    }

    #[test]
    fn container_flags_are_global_and_conflict() {
        let forced = Cli::try_parse_from(rewrite(["ci", "-c", "build"])).expect("parse");
        assert!(forced.global.container);
        assert!(!forced.global.no_container);
        assert!(matches!(forced.command, Commands::Run(_)));

        let disabled = Cli::try_parse_from(rewrite(["ci", "build", "-C"])).expect("parse");
        assert!(!disabled.global.container);
        assert!(disabled.global.no_container);
        assert!(matches!(disabled.command, Commands::Run(_)));

        assert!(Cli::try_parse_from(rewrite(["ci", "-c", "-C", "build"])).is_err());
    }

    fn rewrite<const N: usize>(argv: [&str; N]) -> Vec<OsString> {
        rewrite_argv(argv.into_iter().map(OsString::from).collect())
    }
}
