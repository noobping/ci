#![cfg(unix)]

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MANAGED_MARKER: &str = "managed-by: ci";

const CLIENT_HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
    "post-index-change",
];

const SERVER_HOOKS: &[&str] = &[
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
];

#[derive(Clone, Debug)]
struct Global {
    repo: PathBuf,
    ci_dir: PathBuf,
    verbosity: Verbosity,
    color: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verbosity {
    Silent,
    Quiet,
    Normal,
    Verbose(u8),
}

impl Default for Global {
    fn default() -> Self {
        Self {
            repo: PathBuf::from("."),
            ci_dir: PathBuf::from(".ci"),
            verbosity: Verbosity::Normal,
            color: "auto".to_string(),
        }
    }
}

impl Global {
    fn info(&self, message: impl AsRef<str>) {
        if !matches!(self.verbosity, Verbosity::Quiet | Verbosity::Silent) {
            println!("{}", message.as_ref());
        }
    }

    fn warn(&self, message: impl AsRef<str>) {
        if !matches!(self.verbosity, Verbosity::Silent) {
            eprintln!("ci: warning: {}", message.as_ref());
        }
    }

    fn verbose(&self, message: impl AsRef<str>) {
        if matches!(self.verbosity, Verbosity::Verbose(_)) {
            eprintln!("ci: {}", message.as_ref());
        }
    }
}

#[derive(Clone, Debug)]
struct RepoInfo {
    root: PathBuf,
    git_dir: PathBuf,
    ci_dir: PathBuf,
    is_bare: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WorkflowKind {
    Executable,
    Yaml,
    Container,
}

#[derive(Clone, Debug)]
struct Workflow {
    name: String,
    path: PathBuf,
    kind: WorkflowKind,
}

#[derive(Clone, Debug)]
struct YamlWorkflow {
    name: Option<String>,
    on: Vec<String>,
    steps: Vec<YamlStep>,
}

#[derive(Clone, Debug)]
struct YamlStep {
    name: Option<String>,
    run: String,
}

#[derive(Clone, Debug)]
struct RunOptions {
    workflow: Option<String>,
    event: String,
    dry_run: bool,
    keep_going: bool,
    container_runtime: String,
    hook_args: Vec<String>,
}

fn main() {
    let code = match real_main() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("ci: error: {err}");
            3
        }
    };
    std::process::exit(code);
}

fn real_main() -> Result<i32, String> {
    let argv: Vec<String> = env::args().collect();
    let invoked_as = argv
        .first()
        .and_then(|p| Path::new(p).file_name())
        .and_then(OsStr::to_str)
        .unwrap_or("ci")
        .to_string();

    if is_known_hook(&invoked_as) {
        let global = Global::default();
        return cmd_hook(&global, &invoked_as, &argv[1..]);
    }

    let mut global = Global::default();
    let mut i = 1;

    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(0);
            }
            "-V" | "--version" => {
                println!("ci {VERSION}");
                return Ok(0);
            }
            "-v" | "--verbose" => {
                global.verbosity = match global.verbosity {
                    Verbosity::Verbose(n) => Verbosity::Verbose(n.saturating_add(1)),
                    _ => Verbosity::Verbose(1),
                };
                i += 1;
            }
            "-q" | "--quiet" => {
                global.verbosity = Verbosity::Quiet;
                i += 1;
            }
            "--silent" => {
                global.verbosity = Verbosity::Silent;
                i += 1;
            }
            "--repo" => {
                i += 1;
                global.repo = next_arg(&argv, i, "--repo")?.into();
                i += 1;
            }
            "--ci-dir" => {
                i += 1;
                global.ci_dir = next_arg(&argv, i, "--ci-dir")?.into();
                i += 1;
            }
            "--color" => {
                i += 1;
                global.color = next_arg(&argv, i, "--color")?.to_string();
                i += 1;
            }
            "run" => return cmd_run(&global, &argv[i + 1..]),
            "list" | "ls" => return cmd_list(&global, &argv[i + 1..]),
            "install" => return cmd_install(&global, &argv[i + 1..]),
            "uninstall" | "remove" => return cmd_uninstall(&global, &argv[i + 1..]),
            "update" => return cmd_update(&global, &argv[i + 1..]),
            "hook" => {
                let hook = argv
                    .get(i + 1)
                    .ok_or_else(|| "missing hook name; try `ci hook pre-commit`".to_string())?;
                return cmd_hook(&global, hook, &argv[i + 2..]);
            }
            "doctor" => return cmd_doctor(&global, &argv[i + 1..]),
            "init" => return cmd_init(&global, &argv[i + 1..]),
            "self" => return cmd_self(&global, &argv[i + 1..]),
            "help" => {
                if let Some(topic) = argv.get(i + 1) {
                    print_command_help(topic);
                } else {
                    print_help();
                }
                return Ok(0);
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown global option `{other}`; try `ci --help`"));
            }
            other => return Err(format!("unknown command `{other}`; try `ci --help`")),
        }
    }

    print_help();
    Ok(0)
}

fn next_arg<'a>(argv: &'a [String], i: usize, flag: &str) -> Result<&'a str, String> {
    argv.get(i)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn cmd_run(global: &Global, args: &[String]) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_run_help();
        return Ok(0);
    }
    let repo = discover_repo(global)?;
    let opts = parse_run_options(args)?;
    run_workflows(global, &repo, &opts)
}

fn cmd_hook(global: &Global, hook: &str, hook_args: &[String]) -> Result<i32, String> {
    if !is_known_hook(hook) {
        return Err(format!("unknown Git hook `{hook}`"));
    }

    let repo = discover_repo(global)?;
    let opts = RunOptions {
        workflow: None,
        event: hook.to_string(),
        dry_run: false,
        keep_going: false,
        container_runtime: "auto".to_string(),
        hook_args: hook_args.to_vec(),
    };

    run_workflows(global, &repo, &opts)
}

fn cmd_list(global: &Global, args: &[String]) -> Result<i32, String> {
    reject_unknown_args(args, "list")?;
    let repo = discover_repo(global)?;
    let mut workflows = discover_workflows(&repo)?;
    workflows.sort_by(|a, b| a.name.cmp(&b.name));

    if workflows.is_empty() {
        global.info(format!("No workflows found in {}", repo.ci_dir.display()));
        return Ok(0);
    }

    for workflow in workflows {
        let kind = match workflow.kind {
            WorkflowKind::Executable => "executable",
            WorkflowKind::Yaml => "yaml",
            WorkflowKind::Container => "container",
        };
        println!("{:<28} {:<12} {}", workflow.name, kind, workflow.path.display());
    }

    Ok(0)
}

fn cmd_install(global: &Global, args: &[String]) -> Result<i32, String> {
    let mut mode = "link".to_string();
    let mut hooks_arg: Option<String> = None;
    let mut force = false;
    let mut backup_existing = false;
    let mut dry_run = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_install_help();
                return Ok(0);
            }
            "--mode" => {
                i += 1;
                mode = next_arg(args, i, "--mode")?.to_string();
                i += 1;
            }
            "--hooks" => {
                i += 1;
                hooks_arg = Some(next_arg(args, i, "--hooks")?.to_string());
                i += 1;
            }
            "--bare" => {
                // Kept for CLI clarity. Repository discovery asks Git whether the repo is bare.
                i += 1;
            }
            "--force" => {
                force = true;
                i += 1;
            }
            "--backup-existing" => {
                backup_existing = true;
                i += 1;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            other => return Err(format!("unknown install option `{other}`; try `ci help install`")),
        }
    }

    if mode != "link" && mode != "copy" {
        return Err("--mode must be `link` or `copy`".to_string());
    }

    let repo = discover_repo(global)?;
    let hooks = parse_hooks(hooks_arg.as_deref(), repo.is_bare)?;
    let source = env::current_exe().map_err(|e| format!("could not find current executable: {e}"))?;
    let ci_bin_dir = repo.git_dir.join("ci");
    let ci_bin = ci_bin_dir.join("ci");
    let hooks_dir = repo.git_dir.join("hooks");

    global.info(format!("Installing ci into {}", repo.git_dir.display()));
    global.info(format!("Mode: {mode}"));

    if dry_run {
        println!("would create directory {}", ci_bin_dir.display());
        println!("would install binary {} from {}", ci_bin.display(), source.display());
    } else {
        fs::create_dir_all(&ci_bin_dir)
            .map_err(|e| format!("could not create {}: {e}", ci_bin_dir.display()))?;
        install_binary(&source, &ci_bin, &mode)?;
    }

    if dry_run {
        println!("would create directory {}", hooks_dir.display());
    } else {
        fs::create_dir_all(&hooks_dir)
            .map_err(|e| format!("could not create {}: {e}", hooks_dir.display()))?;
    }

    for hook in hooks {
        let hook_path = hooks_dir.join(hook);
        if dry_run {
            println!("would install hook {}", hook_path.display());
            continue;
        }
        install_hook(&hook_path, hook, force, backup_existing)?;
        global.verbose(format!("installed hook {hook}"));
    }

    global.info("Done.");
    Ok(0)
}

fn cmd_update(global: &Global, args: &[String]) -> Result<i32, String> {
    let mut source: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_update_help();
                return Ok(0);
            }
            "--source" => {
                i += 1;
                source = Some(PathBuf::from(next_arg(args, i, "--source")?));
                i += 1;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            other => return Err(format!("unknown update option `{other}`; try `ci help update`")),
        }
    }

    let repo = discover_repo(global)?;
    let source = match source {
        Some(path) => path,
        None => env::current_exe().map_err(|e| format!("could not find current executable: {e}"))?,
    };
    let ci_bin = repo.git_dir.join("ci").join("ci");

    if !ci_bin.exists() && !is_symlink(&ci_bin) {
        return Err(format!(
            "ci does not look installed in {}; run `ci install` first",
            repo.git_dir.display()
        ));
    }

    if dry_run {
        println!("would update {} from {}", ci_bin.display(), source.display());
    } else if is_symlink(&ci_bin) {
        remove_file_if_exists(&ci_bin)?;
        symlink(&source, &ci_bin)
            .map_err(|e| format!("could not create symlink {} -> {}: {e}", ci_bin.display(), source.display()))?;
    } else {
        fs::copy(&source, &ci_bin).map_err(|e| {
            format!(
                "could not copy {} to {}: {e}",
                source.display(),
                ci_bin.display()
            )
        })?;
        chmod_executable(&ci_bin)?;
    }

    refresh_managed_hooks(global, &repo, dry_run)?;
    global.info("Updated ci installation.");
    Ok(0)
}

fn cmd_uninstall(global: &Global, args: &[String]) -> Result<i32, String> {
    let mut hooks_arg: Option<String> = None;
    let mut keep_binary = false;
    let mut restore = false;
    let mut dry_run = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_uninstall_help();
                return Ok(0);
            }
            "--hooks" => {
                i += 1;
                hooks_arg = Some(next_arg(args, i, "--hooks")?.to_string());
                i += 1;
            }
            "--keep-binary" => {
                keep_binary = true;
                i += 1;
            }
            "--restore" => {
                restore = true;
                i += 1;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            other => return Err(format!("unknown uninstall option `{other}`; try `ci help uninstall`")),
        }
    }

    let repo = discover_repo(global)?;
    let hooks = parse_hooks(hooks_arg.as_deref().or(Some("all")), repo.is_bare)?;
    let hooks_dir = repo.git_dir.join("hooks");

    for hook in hooks {
        let hook_path = hooks_dir.join(hook);
        let backup_path = hooks_dir.join(format!("{hook}.ci-backup"));

        if !hook_path.exists() {
            if restore && backup_path.exists() {
                if dry_run {
                    println!("would restore {} to {}", backup_path.display(), hook_path.display());
                } else {
                    fs::rename(&backup_path, &hook_path).map_err(|e| {
                        format!(
                            "could not restore {} to {}: {e}",
                            backup_path.display(),
                            hook_path.display()
                        )
                    })?;
                }
            }
            continue;
        }

        if !is_managed_hook(&hook_path) {
            global.warn(format!("skipping user-owned hook {}", hook_path.display()));
            continue;
        }

        if dry_run {
            println!("would remove hook {}", hook_path.display());
        } else {
            fs::remove_file(&hook_path)
                .map_err(|e| format!("could not remove {}: {e}", hook_path.display()))?;
        }

        if restore && backup_path.exists() {
            if dry_run {
                println!("would restore {} to {}", backup_path.display(), hook_path.display());
            } else {
                fs::rename(&backup_path, &hook_path).map_err(|e| {
                    format!(
                        "could not restore {} to {}: {e}",
                        backup_path.display(),
                        hook_path.display()
                    )
                })?;
            }
        }
    }

    if !keep_binary {
        let ci_bin = repo.git_dir.join("ci").join("ci");
        let ci_dir = repo.git_dir.join("ci");
        if dry_run {
            println!("would remove binary {}", ci_bin.display());
        } else {
            remove_file_if_exists(&ci_bin)?;
            let _ = fs::remove_dir(&ci_dir);
        }
    }

    global.info("Removed ci installation.");
    Ok(0)
}

fn cmd_doctor(global: &Global, args: &[String]) -> Result<i32, String> {
    reject_unknown_args(args, "doctor")?;
    let repo = discover_repo(global)?;

    println!("Repository: {}", repo.root.display());
    println!("Git dir:    {}", repo.git_dir.display());
    println!("CI dir:     {}", repo.ci_dir.display());
    println!("Bare repo:  {}", repo.is_bare);
    println!();

    if repo.ci_dir.exists() {
        println!("OK   .ci directory exists");
    } else {
        println!("WARN .ci directory does not exist");
    }

    let workflows = discover_workflows(&repo)?;
    if workflows.is_empty() {
        println!("WARN no workflows found");
    } else {
        println!("OK   found {} workflow(s)", workflows.len());
    }

    let ci_bin = repo.git_dir.join("ci").join("ci");
    if ci_bin.exists() || is_symlink(&ci_bin) {
        println!("OK   ci binary installed at {}", ci_bin.display());
    } else {
        println!("WARN ci binary is not installed into this repository");
    }

    let hooks_dir = repo.git_dir.join("hooks");
    let installed: Vec<_> = all_hooks()
        .into_iter()
        .filter(|hook| is_managed_hook(&hooks_dir.join(hook)))
        .collect();

    if installed.is_empty() {
        println!("WARN no ci-managed Git hooks installed");
    } else {
        println!("OK   ci-managed hooks: {}", installed.join(", "));
    }

    if command_exists("podman") {
        println!("OK   podman available");
    } else if command_exists("docker") {
        println!("OK   docker available");
    } else {
        println!("WARN podman/docker not found; container workflows will not run");
    }

    Ok(0)
}

fn cmd_init(global: &Global, args: &[String]) -> Result<i32, String> {
    let mut force = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_init_help();
                return Ok(0);
            }
            "--force" => {
                force = true;
                i += 1;
            }
            other => return Err(format!("unknown init option `{other}`; try `ci help init`")),
        }
    }

    let repo = discover_repo(global)?;
    fs::create_dir_all(&repo.ci_dir)
        .map_err(|e| format!("could not create {}: {e}", repo.ci_dir.display()))?;

    let build = repo.ci_dir.join("build.yml");
    if build.exists() && !force {
        return Err(format!(
            "{} already exists; use `ci init --force` to replace it",
            build.display()
        ));
    }

    let cargo_toml = repo.root.join("Cargo.toml");
    let content = if cargo_toml.exists() {
        DEFAULT_RUST_WORKFLOW
    } else {
        DEFAULT_SHELL_WORKFLOW
    };

    fs::write(&build, content).map_err(|e| format!("could not write {}: {e}", build.display()))?;
    global.info(format!("Created {}", build.display()));
    Ok(0)
}

fn cmd_self(_global: &Global, args: &[String]) -> Result<i32, String> {
    reject_unknown_args(args, "self")?;
    let exe = env::current_exe().map_err(|e| format!("could not find current executable: {e}"))?;
    println!("ci {VERSION}");
    println!("executable: {}", exe.display());
    println!("invocation:  {}", env::args().next().unwrap_or_else(|| "ci".to_string()));
    Ok(0)
}

fn parse_run_options(args: &[String]) -> Result<RunOptions, String> {
    let mut workflow = None;
    let mut event = "manual".to_string();
    let mut dry_run = false;
    let mut keep_going = false;
    let mut container_runtime = "auto".to_string();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_run_help();
                return Err("help requested".to_string());
            }
            "--event" => {
                i += 1;
                event = next_arg(args, i, "--event")?.to_string();
                i += 1;
            }
            "--all" => {
                workflow = None;
                i += 1;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--keep-going" => {
                keep_going = true;
                i += 1;
            }
            "--fail-fast" => {
                keep_going = false;
                i += 1;
            }
            "--container-runtime" => {
                i += 1;
                container_runtime = next_arg(args, i, "--container-runtime")?.to_string();
                i += 1;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown run option `{other}`; try `ci help run`"));
            }
            name => {
                if workflow.is_some() {
                    return Err("only one workflow name may be provided".to_string());
                }
                workflow = Some(name.to_string());
                i += 1;
            }
        }
    }

    if !["auto", "podman", "docker"].contains(&container_runtime.as_str()) {
        return Err("--container-runtime must be auto, podman, or docker".to_string());
    }

    Ok(RunOptions {
        workflow,
        event,
        dry_run,
        keep_going,
        container_runtime,
        hook_args: Vec::new(),
    })
}

fn run_workflows(global: &Global, repo: &RepoInfo, opts: &RunOptions) -> Result<i32, String> {
    let mut workflows = discover_workflows(repo)?;
    workflows.sort_by(|a, b| a.name.cmp(&b.name));

    let selected: Vec<Workflow> = workflows
        .into_iter()
        .filter(|workflow| {
            if let Some(name) = &opts.workflow {
                workflow.name == *name
            } else if opts.event == "manual" {
                true
            } else {
                workflow_matches_event(workflow, &opts.event)
            }
        })
        .collect();

    if selected.is_empty() {
        if let Some(name) = &opts.workflow {
            eprintln!("ci: workflow not found: {name}");
            return Ok(127);
        }
        global.verbose(format!("no workflows matched event `{}`", opts.event));
        return Ok(0);
    }

    let mut last_failure = 0;
    for workflow in selected {
        if opts.dry_run {
            println!("would run {} ({:?})", workflow.name, workflow.kind);
            continue;
        }

        global.info(format!("==> {}", workflow.name));
        let code = run_one_workflow(global, repo, opts, &workflow)?;
        if code != 0 {
            last_failure = code;
            if !opts.keep_going {
                return Ok(code);
            }
        }
    }

    Ok(last_failure)
}

fn run_one_workflow(
    global: &Global,
    repo: &RepoInfo,
    opts: &RunOptions,
    workflow: &Workflow,
) -> Result<i32, String> {
    match workflow.kind {
        WorkflowKind::Executable => run_executable(repo, opts, workflow),
        WorkflowKind::Yaml => run_yaml(global, repo, opts, workflow),
        WorkflowKind::Container => run_container(global, repo, opts, workflow),
    }
}

fn run_executable(repo: &RepoInfo, opts: &RunOptions, workflow: &Workflow) -> Result<i32, String> {
    let mut command = Command::new(&workflow.path);
    command
        .current_dir(&repo.root)
        .args(&opts.hook_args)
        .env("CI", "true")
        .env("CI_TOOL", "ci")
        .env("CI_EVENT", &opts.event)
        .env("CI_HOOK", &opts.event)
        .env("CI_REPO", &repo.root)
        .env("CI_GIT_DIR", &repo.git_dir)
        .env("CI_WORKFLOW", &workflow.name)
        .env("CI_WORKFLOW_PATH", &workflow.path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = command
        .status()
        .map_err(|e| format!("could not run {}: {e}", workflow.path.display()))?;
    Ok(status.code().unwrap_or(1))
}

fn run_yaml(
    global: &Global,
    repo: &RepoInfo,
    opts: &RunOptions,
    workflow: &Workflow,
) -> Result<i32, String> {
    let yaml = parse_yaml_workflow(&workflow.path)?;
    if yaml.steps.is_empty() {
        global.warn(format!("{} has no steps", workflow.path.display()));
        return Ok(0);
    }

    for step in yaml.steps {
        let step_name = step.name.as_deref().unwrap_or("run");
        global.info(format!("--> {step_name}"));
        let status = Command::new("/bin/sh")
            .arg("-c")
            .arg(&step.run)
            .current_dir(&repo.root)
            .env("CI", "true")
            .env("CI_TOOL", "ci")
            .env("CI_EVENT", &opts.event)
            .env("CI_HOOK", &opts.event)
            .env("CI_REPO", &repo.root)
            .env("CI_GIT_DIR", &repo.git_dir)
            .env("CI_WORKFLOW", &workflow.name)
            .env("CI_WORKFLOW_PATH", &workflow.path)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("could not run step `{step_name}`: {e}"))?;

        let code = status.code().unwrap_or(1);
        if code != 0 {
            return Ok(code);
        }
    }

    Ok(0)
}

fn run_container(
    global: &Global,
    repo: &RepoInfo,
    opts: &RunOptions,
    workflow: &Workflow,
) -> Result<i32, String> {
    let runtime = match opts.container_runtime.as_str() {
        "podman" => "podman".to_string(),
        "docker" => "docker".to_string(),
        "auto" => {
            if command_exists("podman") {
                "podman".to_string()
            } else if command_exists("docker") {
                "docker".to_string()
            } else {
                return Err("container workflow requires podman or docker".to_string());
            }
        }
        _ => unreachable!(),
    };

    let tag = format!("ci-{}", sanitize_tag(&workflow.name));
    global.info(format!("--> build container with {runtime}"));

    let build_status = Command::new(&runtime)
        .arg("build")
        .arg("-f")
        .arg(&workflow.path)
        .arg("-t")
        .arg(&tag)
        .arg(&repo.root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("could not run {runtime} build: {e}"))?;

    let build_code = build_status.code().unwrap_or(1);
    if build_code != 0 {
        return Ok(build_code);
    }

    global.info(format!("--> run container {tag}"));
    let mount = format!("{}:/work", repo.root.display());
    let run_status = Command::new(&runtime)
        .arg("run")
        .arg("--rm")
        .arg("-e")
        .arg("CI=true")
        .arg("-e")
        .arg("CI_TOOL=ci")
        .arg("-e")
        .arg(format!("CI_EVENT={}", opts.event))
        .arg("-e")
        .arg(format!("CI_WORKFLOW={}", workflow.name))
        .arg("-v")
        .arg(mount)
        .arg("-w")
        .arg("/work")
        .arg(&tag)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("could not run {runtime}: {e}"))?;

    Ok(run_status.code().unwrap_or(1))
}

fn discover_repo(global: &Global) -> Result<RepoInfo, String> {
    let repo_arg = absolute_path(&global.repo)?;
    let git_dir_raw = git_output(&repo_arg, &["rev-parse", "--git-dir"])?;
    let is_bare_raw = git_output(&repo_arg, &["rev-parse", "--is-bare-repository"])?;
    let is_bare = is_bare_raw.trim() == "true";

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
        let top = git_output(&repo_arg, &["rev-parse", "--show-toplevel"])?;
        canonicalize_best(Path::new(top.trim()))?
    };

    let ci_dir = if global.ci_dir.is_absolute() {
        global.ci_dir.clone()
    } else {
        root.join(&global.ci_dir)
    };

    Ok(RepoInfo {
        root,
        git_dir,
        ci_dir,
        is_bare,
    })
}

fn git_output(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "not a Git repository or Git failed for {}: {}",
            repo.display(),
            stderr.trim()
        ))
    }
}

fn discover_workflows(repo: &RepoInfo) -> Result<Vec<Workflow>, String> {
    let mut workflows = Vec::new();
    if !repo.ci_dir.exists() {
        return Ok(workflows);
    }

    walk_ci_dir(&repo.ci_dir, &repo.ci_dir, &mut workflows)?;
    workflows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(workflows)
}

fn walk_ci_dir(base: &Path, dir: &Path, workflows: &mut Vec<Workflow>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("could not read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("could not read directory entry: {e}"))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|e| format!("could not stat {}: {e}", path.display()))?;

        if meta.is_dir() {
            walk_ci_dir(base, &path, workflows)?;
            continue;
        }

        if !meta.is_file() {
            continue;
        }

        let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
        let ext = path.extension().and_then(OsStr::to_str).unwrap_or_default();

        let kind = if file_name == "Containerfile" || file_name == "Dockerfile" {
            Some(WorkflowKind::Container)
        } else if ext == "yml" || ext == "yaml" {
            Some(WorkflowKind::Yaml)
        } else if is_executable(&path) {
            Some(WorkflowKind::Executable)
        } else {
            None
        };

        if let Some(kind) = kind {
            let name = workflow_name(base, &path, &kind);
            workflows.push(Workflow { name, path, kind });
        }
    }
    Ok(())
}

fn workflow_name(base: &Path, path: &Path, kind: &WorkflowKind) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let parent = rel.parent().unwrap_or_else(|| Path::new(""));
    let file_stem = path.file_stem().and_then(OsStr::to_str).unwrap_or("workflow");
    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or(file_stem);

    let raw = match kind {
        WorkflowKind::Container => {
            if parent.as_os_str().is_empty() {
                "container".to_string()
            } else {
                path_to_name(parent)
            }
        }
        WorkflowKind::Yaml if (file_name == "workflow.yml" || file_name == "workflow.yaml")
            && !parent.as_os_str().is_empty() =>
        {
            path_to_name(parent)
        }
        _ => {
            let mut name = rel.with_file_name(file_stem);
            if name.as_os_str().is_empty() {
                name = PathBuf::from(file_stem);
            }
            path_to_name(&name)
        }
    };

    raw.trim_matches('/').to_string()
}

fn path_to_name(path: &Path) -> String {
    path.components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

fn workflow_matches_event(workflow: &Workflow, event: &str) -> bool {
    if workflow.name == event || workflow.name.ends_with(&format!("/{event}")) {
        return true;
    }

    if workflow.kind == WorkflowKind::Yaml {
        if let Ok(parsed) = parse_yaml_workflow(&workflow.path) {
            return parsed.on.iter().any(|item| item == event || item == "all");
        }
    }

    false
}

fn parse_yaml_workflow(path: &Path) -> Result<YamlWorkflow, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("could not read workflow {}: {e}", path.display()))?;
    let lines: Vec<&str> = content.lines().collect();

    let mut workflow = YamlWorkflow {
        name: None,
        on: Vec::new(),
        steps: Vec::new(),
    };

    let mut mode = "";
    let mut current_name: Option<String> = None;
    let mut current_run: Option<String> = None;
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        if starts_top_level(line, "name:") {
            workflow.name = Some(clean_scalar(after_colon(trimmed)));
            mode = "";
            i += 1;
            continue;
        }

        if starts_top_level(line, "on:") {
            mode = "on";
            let rest = after_colon(trimmed).trim();
            if rest.starts_with('[') && rest.ends_with(']') {
                workflow.on.extend(parse_inline_list(rest));
            } else if !rest.is_empty() {
                workflow.on.push(clean_scalar(rest));
            }
            i += 1;
            continue;
        }

        if starts_top_level(line, "steps:") {
            mode = "steps";
            i += 1;
            continue;
        }

        if mode == "on" && trimmed.starts_with("- ") {
            workflow.on.push(clean_scalar(trimmed[2..].trim()));
            i += 1;
            continue;
        }

        if mode == "steps" {
            if trimmed.starts_with("- ") {
                let item = trimmed[2..].trim();
                if item.starts_with("name:") {
                    flush_step(&mut workflow.steps, &mut current_name, &mut current_run);
                    current_name = Some(clean_scalar(after_colon(item)));
                    i += 1;
                    continue;
                }
                if item.starts_with("run:") {
                    flush_step(&mut workflow.steps, &mut current_name, &mut current_run);
                    let value = after_colon(item).trim();
                    if value == "|" || value == ">" {
                        let (block, next) = collect_block(&lines, i, indent_of(line));
                        workflow.steps.push(YamlStep { name: None, run: block });
                        i = next;
                    } else {
                        workflow.steps.push(YamlStep {
                            name: None,
                            run: clean_scalar(value),
                        });
                        i += 1;
                    }
                    continue;
                }
            }

            if trimmed.starts_with("run:") {
                let value = after_colon(trimmed).trim();
                if value == "|" || value == ">" {
                    let (block, next) = collect_block(&lines, i, indent_of(line));
                    current_run = Some(block);
                    i = next;
                } else {
                    current_run = Some(clean_scalar(value));
                    i += 1;
                }
                continue;
            }
        }

        i += 1;
    }

    flush_step(&mut workflow.steps, &mut current_name, &mut current_run);

    if workflow.on.is_empty() {
        workflow.on.push("manual".to_string());
    }

    Ok(workflow)
}

fn flush_step(steps: &mut Vec<YamlStep>, name: &mut Option<String>, run: &mut Option<String>) {
    if let Some(command) = run.take() {
        steps.push(YamlStep {
            name: name.take(),
            run: command,
        });
    } else {
        *name = None;
    }
}

fn collect_block(lines: &[&str], start: usize, base_indent: usize) -> (String, usize) {
    let mut block = String::new();
    let mut i = start + 1;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        let indent = indent_of(line);

        if !trimmed.is_empty() && indent <= base_indent {
            break;
        }

        if trimmed.is_empty() {
            block.push('\n');
        } else {
            let strip = (base_indent + 2).min(line.len());
            block.push_str(line.get(strip..).unwrap_or(trimmed));
            block.push('\n');
        }
        i += 1;
    }

    (block, i)
}

fn starts_top_level(line: &str, prefix: &str) -> bool {
    indent_of(line) == 0 && line.trim_start().starts_with(prefix)
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

fn after_colon(s: &str) -> &str {
    s.split_once(':').map(|(_, right)| right).unwrap_or("").trim()
}

fn clean_scalar(value: &str) -> String {
    let value = value.trim();
    let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
    let value = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')).unwrap_or(value);
    value.trim().to_string()
}

fn parse_inline_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(clean_scalar)
        .filter(|s| !s.is_empty())
        .collect()
}

fn install_binary(source: &Path, target: &Path, mode: &str) -> Result<(), String> {
    remove_file_if_exists(target)?;
    match mode {
        "link" => symlink(source, target).map_err(|e| {
            format!(
                "could not create symlink {} -> {}: {e}",
                target.display(),
                source.display()
            )
        })?,
        "copy" => {
            fs::copy(source, target).map_err(|e| {
                format!(
                    "could not copy {} to {}: {e}",
                    source.display(),
                    target.display()
                )
            })?;
            chmod_executable(target)?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn install_hook(
    hook_path: &Path,
    hook: &str,
    force: bool,
    backup_existing: bool,
) -> Result<(), String> {
    if hook_path.exists() && !is_managed_hook(hook_path) {
        if backup_existing {
            let backup_path = hook_path.with_file_name(format!("{hook}.ci-backup"));
            remove_file_if_exists(&backup_path)?;
            fs::rename(hook_path, &backup_path).map_err(|e| {
                format!(
                    "could not back up {} to {}: {e}",
                    hook_path.display(),
                    backup_path.display()
                )
            })?;
        } else if !force {
            return Err(format!(
                "{} already exists and is not managed by ci; use --backup-existing or --force",
                hook_path.display()
            ));
        }
    }

    let script = format!(
        "#!/usr/bin/env sh\n# {MANAGED_MARKER}\nexec \"$(dirname \"$0\")/../ci/ci\" hook {hook} \"$@\"\n"
    );
    fs::write(hook_path, script).map_err(|e| format!("could not write {}: {e}", hook_path.display()))?;
    chmod_executable(hook_path)?;
    Ok(())
}

fn refresh_managed_hooks(global: &Global, repo: &RepoInfo, dry_run: bool) -> Result<(), String> {
    let hooks_dir = repo.git_dir.join("hooks");
    for hook in all_hooks() {
        let hook_path = hooks_dir.join(hook);
        if is_managed_hook(&hook_path) {
            if dry_run {
                println!("would refresh hook {}", hook_path.display());
            } else {
                install_hook(&hook_path, hook, true, false)?;
            }
            global.verbose(format!("refreshed hook {hook}"));
        }
    }
    Ok(())
}

fn parse_hooks(input: Option<&str>, is_bare: bool) -> Result<Vec<&'static str>, String> {
    let requested = input.unwrap_or(if is_bare { "server" } else { "client" });
    let hooks = match requested {
        "all" => all_hooks(),
        "client" => CLIENT_HOOKS.to_vec(),
        "server" => SERVER_HOOKS.to_vec(),
        other => {
            let mut hooks = Vec::new();
            for hook in other.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let known = all_hooks()
                    .into_iter()
                    .find(|known| *known == hook)
                    .ok_or_else(|| format!("unknown Git hook `{hook}`"))?;
                hooks.push(known);
            }
            hooks
        }
    };
    Ok(hooks)
}

fn all_hooks() -> Vec<&'static str> {
    let mut hooks = Vec::new();
    hooks.extend_from_slice(CLIENT_HOOKS);
    for hook in SERVER_HOOKS {
        if !hooks.contains(hook) {
            hooks.push(hook);
        }
    }
    hooks
}

fn is_known_hook(hook: &str) -> bool {
    CLIENT_HOOKS.contains(&hook) || SERVER_HOOKS.contains(&hook)
}

fn reject_unknown_args(args: &[String], command: &str) -> Result<(), String> {
    if let Some(arg) = args.first() {
        Err(format!("unknown {command} option `{arg}`; try `ci help {command}`"))
    } else {
        Ok(())
    }
}

fn is_managed_hook(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|content| content.contains(MANAGED_MARKER))
        .unwrap_or(false)
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn chmod_executable(path: &Path) -> Result<(), String> {
    let mut perms = fs::metadata(path)
        .map_err(|e| format!("could not stat {}: {e}", path.display()))?
        .permissions();
    perms.set_mode(perms.mode() | 0o755);
    fs::set_permissions(path, perms)
        .map_err(|e| format!("could not chmod +x {}: {e}", path.display()))
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.is_dir() && !meta.file_type().is_symlink() {
                fs::remove_dir_all(path)
                    .map_err(|e| format!("could not remove directory {}: {e}", path.display()))
            } else {
                fs::remove_file(path)
                    .map_err(|e| format!("could not remove {}: {e}", path.display()))
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not stat {}: {e}", path.display())),
    }
}

fn command_exists(name: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn sanitize_tag(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' })
        .collect()
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()
            .map_err(|e| format!("could not read current directory: {e}"))?
            .join(path))
    }
}

fn canonicalize_best(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).or_else(|_| absolute_path(path))
}

const DEFAULT_RUST_WORKFLOW: &str = r#"name: build
on:
  - manual
  - pre-push
steps:
  - name: Format
    run: cargo fmt --check
  - name: Lint
    run: cargo clippy --all-targets -- -D warnings
  - name: Test
    run: cargo test --all
  - name: Build
    run: cargo build --release
"#;

const DEFAULT_SHELL_WORKFLOW: &str = r#"name: build
on:
  - manual
steps:
  - name: Build
    run: echo "Add your build command to .ci/build.yml"
"#;

fn print_help() {
    println!(r#"ci - small Git-native CI runner

Usage:
  ci [OPTIONS] <COMMAND>

Commands:
  run        Run workflows from .ci
  list       List discovered workflows
  install    Install ci into a Git repository
  uninstall  Remove ci from a Git repository
  update     Update ci installation in a Git repository
  hook       Run workflows for a Git hook
  doctor     Check repository and ci installation
  init       Create a starter .ci workflow
  self       Show information about this ci binary
  help       Show help for a command

Options:
  -h, --help             Show help
  -V, --version          Show version
  -v, --verbose          Increase verbosity; can be repeated
  -q, --quiet            Reduce output
      --silent           Suppress all non-error output
      --repo <PATH>      Repository path [default: .]
      --ci-dir <DIR>     Workflow directory [default: .ci]
      --color <WHEN>     auto, always, never [default: auto]

Examples:
  ci init
  ci list
  ci run build
  ci install --mode link --hooks pre-commit,pre-push
  ci update
  ci uninstall --restore
"#);
}

fn print_command_help(topic: &str) {
    match topic {
        "run" => print_run_help(),
        "install" => print_install_help(),
        "uninstall" | "remove" => print_uninstall_help(),
        "update" => print_update_help(),
        "init" => print_init_help(),
        _ => print_help(),
    }
}

fn print_run_help() {
    println!(r#"Usage:
  ci run [OPTIONS] [WORKFLOW]

Arguments:
  [WORKFLOW]              Workflow name to run

Options:
      --event <EVENT>     Event or hook name, e.g. pre-commit [default: manual]
      --all               Run all workflows
      --dry-run           Show what would run
      --fail-fast         Stop after first failure [default]
      --keep-going        Continue after failures
      --container-runtime <RUNTIME>
                           auto, podman, docker [default: auto]
  -h, --help              Show help
"#);
}

fn print_install_help() {
    println!(r#"Usage:
  ci install [OPTIONS]

Options:
      --mode <MODE>        install mode: link or copy [default: link]
      --hooks <HOOKS>      all, client, server, or comma-separated list
      --bare              Accepted for clarity; Git still decides whether repo is bare
      --force             Replace existing non-ci hooks
      --backup-existing   Backup existing hooks before replacing them
      --dry-run           Show what would be installed
  -h, --help              Show help

Examples:
  ci install --mode link
  ci install --mode copy --hooks pre-commit,pre-push
  ci --repo /srv/git/project.git install --hooks server
"#);
}

fn print_update_help() {
    println!(r#"Usage:
  ci update [OPTIONS]

Options:
      --source <PATH>      Binary to copy or link from [default: current executable]
      --dry-run           Show what would be updated
  -h, --help              Show help
"#);
}

fn print_uninstall_help() {
    println!(r#"Usage:
  ci uninstall [OPTIONS]

Options:
      --hooks <HOOKS>      all, client, server, or comma-separated list [default: all]
      --keep-binary        Keep .git/ci/ci
      --restore            Restore hook-name.ci-backup files
      --dry-run            Show what would be removed
  -h, --help               Show help
"#);
}

fn print_init_help() {
    println!(r#"Usage:
  ci init [OPTIONS]

Options:
      --force              Replace existing .ci/build.yml
  -h, --help               Show help
"#);
}
