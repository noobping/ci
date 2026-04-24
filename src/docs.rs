use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use clap::CommandFactory;
use clap_complete::generate;
use clap_mangen::Man;

use crate::cli::{Cli, CompletionArgs, CompletionShell, ManArgs};
use crate::error::Result;

pub fn cmd_completion(args: &CompletionArgs) -> Result<i32> {
    let mut command = Cli::command();
    let name = command.get_name().to_string();

    match &args.output {
        Some(path) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(path)?;
            generate(shell(args.shell), &mut command, name, &mut file);
        }
        None => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            generate(shell(args.shell), &mut command, name, &mut handle);
        }
    }

    Ok(0)
}

pub fn cmd_man(args: &ManArgs) -> Result<i32> {
    let command = Cli::command();

    match &args.dir {
        Some(dir) => {
            fs::create_dir_all(dir)?;
            write_man_tree(command, "ci", dir)?;
        }
        None => {
            let stdout = io::stdout();
            let mut handle = stdout.lock();
            render_man(command, "ci", &mut handle)?;
        }
    }

    Ok(0)
}

fn shell(shell: CompletionShell) -> clap_complete::Shell {
    match shell {
        CompletionShell::Bash => clap_complete::Shell::Bash,
    }
}

fn write_man_tree(command: clap::Command, page_name: &str, dir: &Path) -> Result<()> {
    let subcommands = command.get_subcommands().cloned().collect::<Vec<_>>();
    let path = dir.join(format!("{page_name}.1"));
    let mut file = File::create(path)?;
    render_man(command, page_name, &mut file)?;

    for subcommand in subcommands {
        let sub_name = format!("{page_name}-{}", subcommand.get_name());
        write_man_tree(subcommand, &sub_name, dir)?;
    }

    Ok(())
}

fn render_man(command: clap::Command, page_name: &str, output: &mut dyn Write) -> Result<()> {
    Man::new(command).title(page_name).render(output)?;
    Ok(())
}
