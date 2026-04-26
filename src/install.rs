use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::cli::{GlobalOptions, InstallArgs, UninstallArgs, UpdateArgs};
use crate::config::{Architecture, InstallMode, ResolvedConfig};
use crate::error::{CiError, Result};
use crate::git::GitService;
use crate::output::Output;
use crate::repo::{absolute_path, RepoInfo};
use crate::runner::AppContext;
use crate::workflow::all_hooks;

pub const MANAGED_MARKER: &str = "managed-by: ci";
pub const MANAGED_HOOK_NAME: &str = "hook";
pub const MANAGED_RUNNER_NAME: &str = "run";

#[derive(Clone, Debug)]
enum HookInstallStrategy {
    DirectSymlink(PathBuf),
    UniversalScript,
}

#[derive(Clone, Debug)]
pub struct InstallState {
    pub binaries: Vec<BinaryState>,
    pub hooks: Vec<HookState>,
}

#[derive(Clone, Debug)]
pub enum BinaryState {
    Missing(PathBuf),
    Symlink {
        path: PathBuf,
        target: PathBuf,
        broken: bool,
    },
    Copy {
        path: PathBuf,
    },
}

#[derive(Clone, Copy, Debug)]
enum UpdateRepositoryDiscovery {
    Single,
    Direct,
    Recursive,
}

#[derive(Clone, Debug)]
pub struct HookState {
    pub name: String,
    pub path: PathBuf,
    pub exists: bool,
    pub managed: bool,
    pub executable: bool,
    pub backup: bool,
}

pub fn cmd_install(ctx: &AppContext, args: &InstallArgs) -> Result<i32> {
    let hooks = parse_hooks(args.hooks.as_deref(), ctx.repo.is_bare)?;
    let ci_bin_dir = managed_runner_dir(&ctx.repo);
    let mode = ctx
        .config
        .policy
        .install_mode
        .or(args.mode)
        .unwrap_or(ctx.config.defaults.install_mode);
    let source = install_source_for_mode(&mode, args.source.as_deref());
    let target_arches =
        install_target_arches_for_mode(&mode, args.source.as_deref(), &ctx.config.defaults.arch);
    let ci_bins = managed_runner_targets(&ctx.repo, &target_arches);
    let hooks_dir = ctx.repo.git_dir.join("hooks");

    ctx.output
        .info(format!("Installing ci into {}", ctx.repo.git_dir.display()));
    ctx.output.info(format!("Mode: {mode:?}").to_lowercase());

    if args.dry_run {
        println!("would create directory {}", ci_bin_dir.display());
        for (arch, ci_bin) in &ci_bins {
            let source = install_source_for_arch(&ctx.repo.current_exe, source, arch);
            println!(
                "would install binary {} from {}",
                ci_bin.display(),
                source.display()
            );
        }
        if matches!(mode, InstallMode::Link) {
            for stale_runner in stale_managed_runner_paths(&ctx.repo, &target_arches) {
                println!("would remove stale binary {}", stale_runner.display());
            }
        }
    } else {
        fs::create_dir_all(&ci_bin_dir)?;
        for (arch, ci_bin) in &ci_bins {
            let source = install_source_for_arch(&ctx.repo.current_exe, source, arch);
            install_binary(&source, ci_bin, &mode)?;
        }
        if matches!(mode, InstallMode::Link) {
            remove_stale_managed_runners(&ctx.repo, &target_arches)?;
        }
        remove_file_if_exists(&managed_hook_dispatcher_path(&ctx.repo))?;
        fs::create_dir_all(&hooks_dir)?;
    }

    let hook_strategy = install_hook_strategy(&ctx.repo, &mode, &target_arches);

    for hook in hooks {
        let hook_path = hooks_dir.join(hook);
        if args.dry_run {
            println!("would install hook {}", hook_path.display());
            continue;
        }
        install_hook(
            &hook_path,
            hook,
            args.force,
            args.backup_existing,
            &hook_strategy,
        )?;
    }

    ctx.output.info("Done.");
    Ok(0)
}

pub fn cmd_update(ctx: &AppContext, args: &UpdateArgs) -> Result<i32> {
    update_one_repo(ctx, args)
}

pub fn cmd_update_all(
    global: &GlobalOptions,
    args: &UpdateArgs,
    bootstrap_git: &GitService,
    output: Output,
) -> Result<i32> {
    let base = absolute_path(args.path.as_deref().unwrap_or(&global.repo))?;
    let discovery = if args.recursive {
        UpdateRepositoryDiscovery::Recursive
    } else if args.all {
        UpdateRepositoryDiscovery::Direct
    } else {
        UpdateRepositoryDiscovery::Single
    };
    let repos = discover_update_repositories(&base, discovery, output.clone())?;
    if repos.is_empty() {
        return Err(CiError::Message(format!(
            "no Git repositories found in {}",
            base.display()
        )));
    }

    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for repo_path in repos {
        let mut repo_global = global.clone();
        repo_global.repo = repo_path.clone();
        let ctx = match update_context_for_repo(&repo_global, bootstrap_git) {
            Ok(ctx) => ctx,
            Err(err) => {
                failed += 1;
                output.error(format!("failed to read {}: {err}", repo_path.display()));
                continue;
            }
        };

        if !managed_install_exists(&ctx.repo, &ctx.config.defaults.arch) {
            skipped += 1;
            output.info(format!(
                "Skipping {} (ci is not installed)",
                ctx.repo.root.display()
            ));
            continue;
        }

        output.info(format!("Updating {}", ctx.repo.root.display()));
        match update_one_repo(&ctx, args) {
            Ok(_) => updated += 1,
            Err(err) => {
                failed += 1;
                output.error(format!(
                    "failed to update {}: {err}",
                    ctx.repo.root.display()
                ));
            }
        }
    }

    output.info(format!(
        "Updated {updated} ci installation(s); skipped {skipped}; failed {failed}."
    ));
    if failed == 0 {
        Ok(0)
    } else {
        Err(CiError::Message(format!(
            "failed to update {failed} ci installation(s)"
        )))
    }
}

fn update_context_for_repo(
    global: &GlobalOptions,
    bootstrap_git: &GitService,
) -> Result<AppContext> {
    let mut repo = RepoInfo::discover(global, bootstrap_git)?;
    let config = ResolvedConfig::load(&repo, global)?;
    let output =
        Output::from_settings_with_policy(global, Some(&config.defaults), Some(&config.policy));
    let git = GitService::configured(&config.defaults, global, output.clone());
    repo.apply_defaults(&config.defaults);
    repo.refresh_branch(&git)?;
    Ok(AppContext::new(global.clone(), output, repo, config, git))
}

fn update_one_repo(ctx: &AppContext, args: &UpdateArgs) -> Result<i32> {
    let target_arches = install_target_arches(args.source.as_deref(), &ctx.config.defaults.arch);
    let ci_bins = managed_runner_targets(&ctx.repo, &target_arches);

    if !managed_install_exists(&ctx.repo, &ctx.config.defaults.arch) {
        return Err(CiError::Message(format!(
            "ci does not look installed in {}; run `ci install` first",
            ctx.repo.git_dir.display()
        )));
    }

    if args.dry_run {
        for (arch, ci_bin) in &ci_bins {
            let source =
                install_source_for_arch(&ctx.repo.current_exe, args.source.as_deref(), arch);
            println!(
                "would update {} from {}",
                ci_bin.display(),
                source.display()
            );
        }
    } else {
        fs::create_dir_all(managed_runner_dir(&ctx.repo))?;
        for (arch, ci_bin) in &ci_bins {
            let source =
                install_source_for_arch(&ctx.repo.current_exe, args.source.as_deref(), arch);
            if is_symlink(ci_bin) {
                remove_file_if_exists(ci_bin)?;
                symlink(&source, ci_bin)?;
            } else {
                fs::copy(&source, ci_bin)?;
                chmod_executable(ci_bin)?;
            }
        }
        remove_file_if_exists(&managed_hook_dispatcher_path(&ctx.repo))?;
    }

    refresh_managed_hooks(ctx, args.dry_run)?;
    ctx.output.info("Updated ci installation.");
    Ok(0)
}

fn discover_update_repositories(
    base: &Path,
    discovery: UpdateRepositoryDiscovery,
    output: Output,
) -> Result<Vec<PathBuf>> {
    match discovery {
        UpdateRepositoryDiscovery::Single => Ok(vec![base.to_path_buf()]),
        UpdateRepositoryDiscovery::Direct => discover_git_repositories_direct(base, &output),
        UpdateRepositoryDiscovery::Recursive => discover_git_repositories_recursive(base, &output),
    }
}

fn discover_git_repositories_direct(base: &Path, output: &Output) -> Result<Vec<PathBuf>> {
    let mut repos = BTreeSet::new();
    insert_git_repository_at(base, &mut repos);

    if !base.is_dir() {
        return Ok(repos.into_iter().collect());
    }

    for entry in fs::read_dir(base)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                output.warn(format!("skipping unreadable path: {err}"));
                continue;
            }
        };
        let path = entry.path();
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            insert_git_repository_at(&path, &mut repos);
        }
    }

    Ok(repos.into_iter().collect())
}

fn discover_git_repositories_recursive(base: &Path, output: &Output) -> Result<Vec<PathBuf>> {
    let mut repos = BTreeSet::new();
    let mut walker = WalkDir::new(base).follow_links(false).into_iter();
    while let Some(entry) = walker.next() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                output.warn(format!("skipping unreadable path: {err}"));
                continue;
            }
        };
        let path = entry.path();
        let file_name = entry.file_name();
        if file_name == ".git" {
            if let Some(parent) = path.parent() {
                repos.insert(parent.to_path_buf());
            }
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if entry.file_type().is_dir() && looks_like_bare_git_repository(path) {
            repos.insert(path.to_path_buf());
            walker.skip_current_dir();
        }
    }
    Ok(repos.into_iter().collect())
}

fn insert_git_repository_at(path: &Path, repos: &mut BTreeSet<PathBuf>) {
    if path_exists_or_symlink(&path.join(".git")) || looks_like_bare_git_repository(path) {
        repos.insert(path.to_path_buf());
    }
}

fn looks_like_bare_git_repository(path: &Path) -> bool {
    path.join("HEAD").is_file() && path.join("objects").is_dir() && path.join("refs").is_dir()
}

fn managed_install_exists(repo: &RepoInfo, arches: &[Architecture]) -> bool {
    let installed_ci_bins = managed_runner_paths(repo, arches);
    let legacy_ci_bin = legacy_managed_runner_path(repo);
    installed_ci_bins
        .iter()
        .any(|path| path_exists_or_symlink(path))
        || path_exists_or_symlink(&legacy_ci_bin)
}

pub fn cmd_uninstall(ctx: &AppContext, args: &UninstallArgs) -> Result<i32> {
    let hooks = parse_hooks(args.hooks.as_deref().or(Some("all")), ctx.repo.is_bare)?;
    let hooks_dir = ctx.repo.git_dir.join("hooks");

    for hook in hooks {
        let hook_path = hooks_dir.join(hook);
        let backup_path = hooks_dir.join(format!("{hook}.ci-backup"));

        if !path_exists_or_symlink(&hook_path) {
            if args.restore && backup_path.exists() {
                if args.dry_run {
                    println!(
                        "would restore {} to {}",
                        backup_path.display(),
                        hook_path.display()
                    );
                } else {
                    fs::rename(&backup_path, &hook_path)?;
                }
            }
            continue;
        }

        if !is_managed_hook(&hook_path) {
            ctx.output
                .warn(format!("skipping user-owned hook {}", hook_path.display()));
            continue;
        }

        if args.dry_run {
            println!("would remove hook {}", hook_path.display());
        } else {
            fs::remove_file(&hook_path)?;
        }

        if args.restore && backup_path.exists() {
            if args.dry_run {
                println!(
                    "would restore {} to {}",
                    backup_path.display(),
                    hook_path.display()
                );
            } else {
                fs::rename(&backup_path, &hook_path)?;
            }
        }
    }

    if !args.keep_binary {
        let ci_bins = managed_runner_paths(&ctx.repo, &ctx.config.defaults.arch);
        let legacy_ci_bin = legacy_managed_runner_path(&ctx.repo);
        let hook_dispatcher = managed_hook_dispatcher_path(&ctx.repo);
        let ci_dir = managed_runner_dir(&ctx.repo);
        if args.dry_run {
            for ci_bin in &ci_bins {
                println!("would remove binary {}", ci_bin.display());
            }
            if path_exists_or_symlink(&legacy_ci_bin) {
                println!("would remove legacy binary {}", legacy_ci_bin.display());
            }
            if path_exists_or_symlink(&hook_dispatcher) {
                println!("would remove hook dispatcher {}", hook_dispatcher.display());
            }
        } else {
            for ci_bin in &ci_bins {
                remove_file_if_exists(ci_bin)?;
            }
            remove_file_if_exists(&legacy_ci_bin)?;
            remove_file_if_exists(&hook_dispatcher)?;
            let _ = fs::remove_dir(&ci_dir);
        }
    }

    ctx.output.info("Removed ci installation.");
    Ok(0)
}

pub fn inspect_installation(repo: &RepoInfo, arches: &[Architecture]) -> InstallState {
    let arch_bins = managed_runner_paths(repo, arches);
    let legacy_bin = legacy_managed_runner_path(repo);

    let binaries = if arch_bins.iter().any(|path| path_exists_or_symlink(path))
        || !path_exists_or_symlink(&legacy_bin)
    {
        arch_bins.into_iter().map(binary_state).collect()
    } else {
        vec![binary_state(legacy_bin)]
    };

    let hooks_dir = repo.git_dir.join("hooks");
    let hooks = all_hooks()
        .into_iter()
        .map(|hook| {
            let path = hooks_dir.join(hook);
            HookState {
                name: hook.to_string(),
                exists: path_exists_or_symlink(&path),
                managed: is_managed_hook(&path),
                executable: is_executable(&path),
                backup: hooks_dir.join(format!("{hook}.ci-backup")).exists(),
                path,
            }
        })
        .collect();

    InstallState { binaries, hooks }
}

fn binary_state(bin: PathBuf) -> BinaryState {
    if !bin.exists() && !is_symlink(&bin) {
        BinaryState::Missing(bin)
    } else if is_symlink(&bin) {
        let target = fs::read_link(&bin).unwrap_or_default();
        let resolved = if target.is_absolute() {
            target.clone()
        } else {
            bin.parent().unwrap_or_else(|| Path::new("/")).join(&target)
        };
        BinaryState::Symlink {
            broken: !resolved.exists(),
            path: bin,
            target,
        }
    } else {
        BinaryState::Copy { path: bin }
    }
}

pub fn parse_hooks(input: Option<&str>, is_bare: bool) -> Result<Vec<&'static str>> {
    let requested = input.unwrap_or(if is_bare { "server" } else { "client" });
    let hooks = match requested {
        "all" => all_hooks(),
        "client" => crate::workflow::CLIENT_HOOKS.to_vec(),
        "server" => crate::workflow::SERVER_HOOKS.to_vec(),
        other => {
            let mut hooks = Vec::new();
            for hook in other
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
            {
                let known = all_hooks()
                    .into_iter()
                    .find(|candidate| *candidate == hook)
                    .ok_or_else(|| CiError::Usage(format!("unknown Git hook `{hook}`")))?;
                hooks.push(known);
            }
            hooks
        }
    };
    Ok(hooks)
}

fn refresh_managed_hooks(ctx: &AppContext, dry_run: bool) -> Result<()> {
    let hooks_dir = ctx.repo.git_dir.join("hooks");
    let hook_strategy = refresh_hook_strategy(&ctx.repo);
    for hook in all_hooks() {
        let hook_path = hooks_dir.join(hook);
        if is_managed_hook(&hook_path) {
            if dry_run {
                println!("would refresh hook {}", hook_path.display());
            } else {
                install_hook(&hook_path, hook, true, false, &hook_strategy)?;
            }
        }
    }
    Ok(())
}

fn install_binary(source: &Path, target: &Path, mode: &InstallMode) -> Result<()> {
    remove_file_if_exists(target)?;
    match mode {
        InstallMode::Link => symlink(source, target)?,
        InstallMode::Copy => {
            fs::copy(source, target)?;
            chmod_executable(target)?;
        }
    }
    Ok(())
}

fn install_source_for_arch(
    default_source: &Path,
    source: Option<&Path>,
    arch: &Architecture,
) -> PathBuf {
    let Some(source) = source else {
        return default_source.to_path_buf();
    };

    let value = source.to_string_lossy();
    if value.contains("{arch}") {
        PathBuf::from(value.replace("{arch}", &arch.runner_suffix()))
    } else {
        source.to_path_buf()
    }
}

fn install_target_arches(source: Option<&Path>, configured: &[Architecture]) -> Vec<Architecture> {
    if source.map(source_has_arch_template).unwrap_or(false) {
        runner_arches(configured)
    } else {
        vec![Architecture::host()]
    }
}

fn source_has_arch_template(source: &Path) -> bool {
    source.to_string_lossy().contains("{arch}")
}

fn install_source_for_mode<'a>(mode: &InstallMode, source: Option<&'a Path>) -> Option<&'a Path> {
    match mode {
        InstallMode::Link => None,
        InstallMode::Copy => source,
    }
}

fn install_target_arches_for_mode(
    mode: &InstallMode,
    source: Option<&Path>,
    configured: &[Architecture],
) -> Vec<Architecture> {
    match mode {
        InstallMode::Link => vec![Architecture::host()],
        InstallMode::Copy => install_target_arches(source, configured),
    }
}

fn install_hook_strategy(
    repo: &RepoInfo,
    mode: &InstallMode,
    target_arches: &[Architecture],
) -> HookInstallStrategy {
    if matches!(mode, InstallMode::Link) {
        return direct_hook_strategy(target_arches);
    }

    let final_arches = final_runner_arches(repo, target_arches);
    if final_arches.len() > 1 {
        HookInstallStrategy::UniversalScript
    } else {
        direct_hook_strategy_for_suffix(final_arches.into_iter().next())
    }
}

fn refresh_hook_strategy(repo: &RepoInfo) -> HookInstallStrategy {
    let installed_arches = existing_runner_arches(repo);
    if installed_arches.len() > 1 {
        HookInstallStrategy::UniversalScript
    } else {
        direct_hook_strategy_for_suffix(installed_arches.into_iter().next())
    }
}

fn direct_hook_strategy(target_arches: &[Architecture]) -> HookInstallStrategy {
    direct_hook_strategy_for_suffix(
        runner_arches(target_arches)
            .into_iter()
            .next()
            .map(|arch| arch.runner_suffix()),
    )
}

fn direct_hook_strategy_for_suffix(suffix: Option<String>) -> HookInstallStrategy {
    let suffix = suffix.unwrap_or_else(|| Architecture::host().runner_suffix());
    HookInstallStrategy::DirectSymlink(PathBuf::from(format!(
        "../ci/{MANAGED_RUNNER_NAME}.{suffix}"
    )))
}

fn final_runner_arches(repo: &RepoInfo, target_arches: &[Architecture]) -> BTreeSet<String> {
    let mut arches = existing_runner_arches(repo);
    for arch in runner_arches(target_arches) {
        arches.insert(arch.runner_suffix());
    }
    arches
}

fn existing_runner_arches(repo: &RepoInfo) -> BTreeSet<String> {
    let mut arches = BTreeSet::new();
    let Ok(entries) = fs::read_dir(managed_runner_dir(repo)) else {
        return arches;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path_exists_or_symlink(&path) {
            continue;
        }
        if let Some(suffix) = runner_suffix_from_path(&path) {
            arches.insert(suffix.to_string());
        }
    }

    arches
}

fn remove_stale_managed_runners(repo: &RepoInfo, keep_arches: &[Architecture]) -> Result<()> {
    for runner in stale_managed_runner_paths(repo, keep_arches) {
        remove_file_if_exists(&runner)?;
    }
    Ok(())
}

fn stale_managed_runner_paths(repo: &RepoInfo, keep_arches: &[Architecture]) -> Vec<PathBuf> {
    let keep: BTreeSet<_> = runner_arches(keep_arches)
        .into_iter()
        .map(|arch| arch.runner_suffix())
        .collect();

    let Ok(entries) = fs::read_dir(managed_runner_dir(repo)) else {
        return Vec::new();
    };

    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            runner_suffix_from_path(path)
                .map(|suffix| !keep.contains(suffix))
                .unwrap_or(false)
        })
        .collect()
}

fn runner_suffix_from_path(path: &Path) -> Option<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(&format!("{MANAGED_RUNNER_NAME}.")))
        .filter(|suffix| !suffix.is_empty())
}

fn universal_hook_script() -> String {
    format!(
        "#!/usr/bin/env sh\n\
         # {MANAGED_MARKER}\n\
         ci_hook=$(basename \"$0\")\n\
         ci_machine=$(uname -m 2>/dev/null || printf unknown)\n\
         case \"$ci_machine\" in\n\
         \tx86_64|amd64) ci_arch=x64 ;;\n\
         \taarch64|arm64) ci_arch=arm64 ;;\n\
         \t*) ci_arch=$ci_machine ;;\n\
         esac\n\
         ci_dir=$(dirname \"$0\")/../ci\n\
         ci_runner=\"$ci_dir/{MANAGED_RUNNER_NAME}.$ci_arch\"\n\
         if [ ! -x \"$ci_runner\" ]; then\n\
         \tci_runner=\"$ci_dir/{MANAGED_RUNNER_NAME}\"\n\
         fi\n\
         exec \"$ci_runner\" hook \"$ci_hook\" \"$@\"\n"
    )
}

fn install_hook(
    hook_path: &Path,
    hook: &str,
    force: bool,
    backup_existing: bool,
    strategy: &HookInstallStrategy,
) -> Result<()> {
    if path_exists_or_symlink(hook_path) && !is_managed_hook(hook_path) {
        if backup_existing {
            let backup_path = hook_path.with_file_name(format!("{hook}.ci-backup"));
            remove_file_if_exists(&backup_path)?;
            fs::rename(hook_path, &backup_path)?;
        } else if !force {
            return Err(CiError::Message(format!(
                "{} already exists and is not managed by ci; use --backup-existing or --force",
                hook_path.display()
            )));
        }
    }

    remove_file_if_exists(hook_path)?;
    match strategy {
        HookInstallStrategy::DirectSymlink(target) => symlink(target, hook_path)?,
        HookInstallStrategy::UniversalScript => {
            fs::write(hook_path, universal_hook_script())?;
            chmod_executable(hook_path)?;
        }
    }
    Ok(())
}

fn managed_runner_dir(repo: &RepoInfo) -> PathBuf {
    repo.git_dir.join("ci")
}

fn managed_hook_dispatcher_path(repo: &RepoInfo) -> PathBuf {
    managed_runner_dir(repo).join(MANAGED_HOOK_NAME)
}

fn managed_runner_paths(repo: &RepoInfo, arches: &[Architecture]) -> Vec<PathBuf> {
    managed_runner_targets(repo, arches)
        .into_iter()
        .map(|(_, path)| path)
        .collect()
}

fn managed_runner_targets(
    repo: &RepoInfo,
    arches: &[Architecture],
) -> Vec<(Architecture, PathBuf)> {
    runner_arches(arches)
        .into_iter()
        .map(|arch| {
            let path = managed_runner_path(repo, &arch);
            (arch, path)
        })
        .collect()
}

fn runner_arches(arches: &[Architecture]) -> Vec<Architecture> {
    if arches.is_empty() {
        vec![Architecture::host()]
    } else {
        arches.to_vec()
    }
}

fn managed_runner_path(repo: &RepoInfo, arch: &Architecture) -> PathBuf {
    managed_runner_dir(repo).join(format!("{MANAGED_RUNNER_NAME}.{}", arch.runner_suffix()))
}

fn legacy_managed_runner_path(repo: &RepoInfo) -> PathBuf {
    managed_runner_dir(repo).join(MANAGED_RUNNER_NAME)
}

fn path_exists_or_symlink(path: &Path) -> bool {
    path.exists() || is_symlink(path)
}

pub fn is_managed_hook(path: &Path) -> bool {
    if fs::read_link(path)
        .map(|target| is_managed_hook_symlink_target(&target))
        .unwrap_or(false)
    {
        return true;
    }

    fs::read_to_string(path)
        .map(|content| content.contains(MANAGED_MARKER))
        .unwrap_or(false)
}

fn is_managed_hook_symlink_target(target: &Path) -> bool {
    let ci_dir = Path::new("../ci");
    if target == ci_dir.join(MANAGED_HOOK_NAME) || target == ci_dir.join(MANAGED_RUNNER_NAME) {
        return true;
    }

    target.parent() == Some(ci_dir)
        && target
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(&format!("{MANAGED_RUNNER_NAME}.")))
            .map(|suffix| !suffix.is_empty())
            .unwrap_or(false)
}

pub fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|value| value.file_type().is_symlink())
        .unwrap_or(false)
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn chmod_executable(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

pub fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.is_dir() && !meta.file_type().is_symlink() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
