use std::ffi::OsString;

use clap::Parser;

use crate::config::ContainerType;

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
