#![cfg(unix)]

pub mod actions;
pub mod artifacts;
pub mod cli;
pub mod conditions;
pub mod config;
pub mod containers;
pub mod defaults;
pub mod docs;
pub mod error;
pub mod git;
pub mod install;
pub mod output;
pub mod repo;
pub mod runner;
pub mod schema;
pub mod schema_definitions;
pub mod status;
pub mod workflow;

use clap::Parser;

use crate::cli::{Cli, Commands};
use crate::error::Result;
use crate::git::GitService;
use crate::output::Output;
use crate::repo::RepoInfo;
use crate::runner::AppContext;

pub fn entrypoint(argv: Vec<std::ffi::OsString>) -> i32 {
    let doctor_alias = cli::doctor_alias_used(&argv);
    let cli = Cli::parse_from(cli::rewrite_argv(argv));
    let bootstrap_output = Output::from_globals(&cli.global);

    match run(cli, doctor_alias, bootstrap_output.clone()) {
        Ok(code) => code,
        Err(err) => {
            bootstrap_output.error(err.to_string());
            err.exit_code()
        }
    }
}

fn run(cli: Cli, doctor_alias: bool, bootstrap_output: Output) -> Result<i32> {
    match &cli.command {
        Commands::Completion(args) => return docs::cmd_completion(args),
        Commands::Man(args) => return docs::cmd_man(args),
        Commands::Schema(args) => return schema::cmd_schema(args),
        _ => {}
    }

    let bootstrap_git = GitService::bootstrap(&cli.global, bootstrap_output.clone());
    let mut repo = RepoInfo::discover(&cli.global, &bootstrap_git)?;
    let config = config::ResolvedConfig::load(&repo, &cli.global)?;
    let output = Output::from_settings(&cli.global, Some(&config.defaults));

    if doctor_alias {
        output.warn("`doctor` is deprecated; use `status`");
    }

    let git = GitService::configured(&config.defaults, &cli.global, output.clone());

    repo.apply_defaults(&config.defaults);
    repo.refresh_branch(&git)?;

    let ctx = AppContext::new(cli.global.clone(), output, repo, config, git);

    match cli.command {
        Commands::Run(args) => runner::cmd_run(&ctx, &args),
        Commands::List(args) => runner::cmd_list(&ctx, &args),
        Commands::Install(args) => install::cmd_install(&ctx, &args),
        Commands::Uninstall(args) => install::cmd_uninstall(&ctx, &args),
        Commands::Update(args) => install::cmd_update(&ctx, &args),
        Commands::Hook(args) => runner::cmd_hook(&ctx, &args),
        Commands::Status(args) => status::cmd_status(&ctx, &args),
        Commands::Explain(args) => status::cmd_explain(&ctx, &args),
        Commands::Schema(_) => unreachable!(),
        Commands::Clean(args) => artifacts::cmd_clean(&ctx, &args),
        Commands::Completion(_) | Commands::Man(_) => unreachable!(),
        Commands::Init(args) => runner::cmd_init(&ctx, &args),
        Commands::SelfCmd(args) => runner::cmd_self(&ctx, &args),
        Commands::Other(args) => runner::cmd_other(&ctx, &args),
    }
}
