use std::ffi::OsString;

use clap::{CommandFactory, Parser};

use crate::config::{ArtifactMode, ContainerType, InstallMode};

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

    let no_porcelain = match Cli::try_parse_from(["ci", "list", "--no-porcelain"]).expect("parse") {
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
fn list_porcelain_short_works() {
    let porcelain = match Cli::try_parse_from(["ci", "list", "-p"]).expect("parse") {
        Cli {
            command: Commands::List(args),
            ..
        } => args,
        _ => panic!("expected list command"),
    };

    assert!(porcelain.use_porcelain(true));
}

#[test]
fn list_porcelain_flags_conflict() {
    assert!(Cli::try_parse_from(["ci", "list", "--porcelain", "--no-porcelain"]).is_err());
}

#[test]
fn commands_and_options_have_help_text() {
    let command = Cli::command();
    for subcommand in command.get_subcommands() {
        if subcommand.get_name() != "help" {
            assert!(
                subcommand.get_about().is_some(),
                "{} is missing command help text",
                subcommand.get_name()
            );
        }
        assert_options_have_help(subcommand);
    }
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
fn verbose_short_flags_count_extra_levels() {
    let cli = Cli::try_parse_from(rewrite(["ci", "-vvv", "build"])).expect("parse");

    assert_eq!(cli.global.verbose, 3);
    assert!(matches!(cli.command, Commands::Run(_)));
}

#[test]
fn silent_flag_is_not_supported() {
    assert!(Cli::try_parse_from(rewrite(["ci", "--silent", "build"])).is_err());
}

#[test]
fn repo_aliases_work_before_rewritten_workflow() {
    let cli = Cli::try_parse_from(rewrite(["ci", "--repository", "/tmp/project", "build"]))
        .expect("parse");

    assert_eq!(cli.global.repo, std::path::PathBuf::from("/tmp/project"));
    match cli.command {
        Commands::Run(args) => assert_eq!(args.workflow.as_deref(), Some("build")),
        _ => panic!("expected run command"),
    }
}

#[test]
fn git_command_global_option_works_before_rewritten_workflow() {
    let cli = Cli::try_parse_from(rewrite([
        "ci",
        "--git-mode",
        "custom",
        "--git-command",
        "flatpak-spawn --host git",
        "build",
    ]))
    .expect("parse");

    assert_eq!(
        cli.global
            .git_command
            .as_ref()
            .expect("git command")
            .parts(),
        [
            "flatpak-spawn".to_string(),
            "--host".to_string(),
            "git".to_string()
        ]
        .as_slice()
    );
    match cli.command {
        Commands::Run(args) => assert_eq!(args.workflow.as_deref(), Some("build")),
        _ => panic!("expected run command"),
    }
}

#[test]
fn update_recursive_short_accepts_optional_path() {
    let cli = Cli::try_parse_from(rewrite(["ci", "update", "-r", "/tmp/projects"])).expect("parse");

    match cli.command {
        Commands::Update(args) => {
            assert!(args.selected_update());
            assert!(args.recursive);
            assert_eq!(
                args.path.as_deref(),
                Some(std::path::Path::new("/tmp/projects"))
            );
        }
        _ => panic!("expected update command"),
    }
}

#[test]
fn update_all_accepts_optional_path() {
    let cli =
        Cli::try_parse_from(rewrite(["ci", "update", "--all", "/tmp/projects"])).expect("parse");

    match cli.command {
        Commands::Update(args) => {
            assert!(args.selected_update());
            assert!(args.all);
            assert!(!args.recursive);
            assert_eq!(
                args.path.as_deref(),
                Some(std::path::Path::new("/tmp/projects"))
            );
        }
        _ => panic!("expected update command"),
    }
}

#[test]
fn update_short_flags_work() {
    let cli = Cli::try_parse_from(rewrite([
        "ci",
        "update",
        "-s",
        "/tmp/ci",
        "-n",
        "-a",
        "/tmp/projects",
    ]))
    .expect("parse");

    match cli.command {
        Commands::Update(args) => {
            assert_eq!(
                args.source.as_deref(),
                Some(std::path::Path::new("/tmp/ci"))
            );
            assert!(args.dry_run);
            assert!(args.all);
            assert_eq!(
                args.path.as_deref(),
                Some(std::path::Path::new("/tmp/projects"))
            );
        }
        _ => panic!("expected update command"),
    }
}

#[test]
fn update_path_targets_selected_repo() {
    let cli = Cli::try_parse_from(rewrite(["ci", "update", "/tmp/projects"])).expect("parse");

    match cli.command {
        Commands::Update(args) => {
            assert!(args.selected_update());
            assert!(!args.all);
            assert!(!args.recursive);
            assert_eq!(
                args.path.as_deref(),
                Some(std::path::Path::new("/tmp/projects"))
            );
        }
        _ => panic!("expected update command"),
    }
}

#[test]
fn run_accepts_build_args_after_workflow() {
    let cli = Cli::try_parse_from(rewrite([
        "ci",
        "run",
        "build",
        "--no-default-features",
        "--features",
        "sqlite",
    ]))
    .expect("parse");

    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.workflow.as_deref(), Some("build"));
            assert_eq!(
                args.args,
                vec!["--no-default-features", "--features", "sqlite"]
            );
        }
        _ => panic!("expected run command"),
    }
}

#[test]
fn rewritten_workflow_accepts_build_args_after_workflow() {
    let cli =
        Cli::try_parse_from(rewrite(["ci", "build", "--no-default-features"])).expect("parse");

    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.workflow.as_deref(), Some("build"));
            assert_eq!(args.args, vec!["--no-default-features"]);
        }
        _ => panic!("expected run command"),
    }
}

#[test]
fn build_args_after_separator_can_match_ci_options() {
    let cli = Cli::try_parse_from(rewrite(["ci", "build", "--", "--dry-run"])).expect("parse");

    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.workflow.as_deref(), Some("build"));
            assert!(!args.dry_run);
            assert_eq!(args.args, vec!["--dry-run"]);
        }
        _ => panic!("expected run command"),
    }
}

#[test]
fn run_dry_run_flags_can_be_cleared_before_separator() {
    let cli = Cli::try_parse_from(rewrite([
        "ci",
        "build",
        "--dry-run",
        "--no-dry-run",
        "--",
        "--dry-run",
    ]))
    .expect("parse");

    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.workflow.as_deref(), Some("build"));
            assert!(!args.dry_run);
            assert!(args.no_dry_run);
            assert_eq!(args.args, vec!["--dry-run"]);
        }
        _ => panic!("expected run command"),
    }
}

#[test]
fn run_dry_run_before_separator_stays_ci_option() {
    let cli = Cli::try_parse_from(rewrite(["ci", "build", "--dry-run"])).expect("parse");

    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.workflow.as_deref(), Some("build"));
            assert!(args.dry_run);
            assert!(args.args.is_empty());
        }
        _ => panic!("expected run command"),
    }
}

#[test]
fn run_short_flags_work() {
    let cli = Cli::try_parse_from(rewrite([
        "ci", "run", "-e", "pre-push", "-a", "-n", "-f", "-l",
    ]))
    .expect("parse");

    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.event, "pre-push");
            assert!(args.all);
            assert!(args.dry_run);
            assert!(args.fail_fast);
            assert!(args.lock);
        }
        _ => panic!("expected run command"),
    }
}

#[test]
fn install_short_flags_work() {
    let cli = Cli::try_parse_from(rewrite([
        "ci", "install", "-m", "copy", "-s", "/tmp/ci", "-H", "pre-push", "-b", "-f", "-B", "-n",
    ]))
    .expect("parse");

    match cli.command {
        Commands::Install(args) => {
            assert_eq!(args.mode, Some(InstallMode::Copy));
            assert_eq!(
                args.source.as_deref(),
                Some(std::path::Path::new("/tmp/ci"))
            );
            assert_eq!(args.hooks.as_deref(), Some("pre-push"));
            assert!(args.bare);
            assert!(args.force);
            assert!(args.backup_existing);
            assert!(args.dry_run);
        }
        _ => panic!("expected install command"),
    }
}

#[test]
fn uninstall_short_flags_work() {
    let cli = Cli::try_parse_from(rewrite([
        "ci",
        "uninstall",
        "-H",
        "pre-push",
        "-k",
        "-r",
        "-n",
    ]))
    .expect("parse");

    match cli.command {
        Commands::Uninstall(args) => {
            assert_eq!(args.hooks.as_deref(), Some("pre-push"));
            assert!(args.keep_binary);
            assert!(args.restore);
            assert!(args.dry_run);
        }
        _ => panic!("expected uninstall command"),
    }
}

#[test]
fn clean_short_flags_work() {
    let cli = Cli::try_parse_from(rewrite([
        "ci",
        "clean",
        "build",
        "-r",
        "run-1",
        "-m",
        "move",
        "-d",
        "/tmp/artifacts",
        "-n",
    ]))
    .expect("parse");

    match cli.command {
        Commands::Clean(args) => {
            assert_eq!(args.workflow.as_deref(), Some("build"));
            assert_eq!(args.run_id.as_deref(), Some("run-1"));
            assert_eq!(args.mode, ArtifactMode::Move);
            assert_eq!(
                args.dest.as_deref(),
                Some(std::path::Path::new("/tmp/artifacts"))
            );
            assert!(args.dry_run);
        }
        _ => panic!("expected clean command"),
    }
}

#[test]
fn init_completion_and_man_short_flags_work() {
    let init = Cli::try_parse_from(rewrite(["ci", "init", "-f"])).expect("parse init");
    match init.command {
        Commands::Init(args) => assert!(args.force),
        _ => panic!("expected init command"),
    }

    let completion =
        Cli::try_parse_from(rewrite(["ci", "completion", "bash", "-o", "/tmp/ci.bash"]))
            .expect("parse completion");
    match completion.command {
        Commands::Completion(args) => {
            assert_eq!(
                args.output.as_deref(),
                Some(std::path::Path::new("/tmp/ci.bash"))
            );
        }
        _ => panic!("expected completion command"),
    }

    let man = Cli::try_parse_from(rewrite(["ci", "man", "-d", "/tmp/man"])).expect("parse man");
    match man.command {
        Commands::Man(args) => {
            assert_eq!(args.dir.as_deref(), Some(std::path::Path::new("/tmp/man")));
        }
        _ => panic!("expected man command"),
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

#[test]
fn tech_stack_flag_is_global_and_has_aliases() {
    let short = Cli::try_parse_from(rewrite(["ci", "-t", "node", "build"])).expect("parse");
    assert_eq!(short.global.tech_stack, Some(ContainerType::Node));
    assert!(matches!(short.command, Commands::Run(_)));

    let type_alias =
        Cli::try_parse_from(rewrite(["ci", "build", "--type", "golang"])).expect("parse");
    assert_eq!(type_alias.global.tech_stack, Some(ContainerType::Go));

    let stack_alias =
        Cli::try_parse_from(rewrite(["ci", "--tech-stack", "py", "build"])).expect("parse");
    assert_eq!(stack_alias.global.tech_stack, Some(ContainerType::Python));
}

fn rewrite<const N: usize>(argv: [&str; N]) -> Vec<OsString> {
    rewrite_argv(argv.into_iter().map(OsString::from).collect())
}

fn assert_options_have_help(command: &clap::Command) {
    for arg in command.get_arguments() {
        let id = arg.get_id().as_str();
        if matches!(id, "help" | "version") {
            continue;
        }
        assert!(
            arg.get_help().is_some(),
            "{} option `{id}` is missing help text",
            command.get_name()
        );
    }

    for subcommand in command.get_subcommands() {
        assert_options_have_help(subcommand);
    }
}
