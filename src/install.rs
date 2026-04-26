use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::cli::{InstallArgs, InstallMode, UninstallArgs, UpdateArgs};
use crate::config::Architecture;
use crate::error::{CiError, Result};
use crate::repo::RepoInfo;
use crate::runner::AppContext;
use crate::workflow::all_hooks;

pub const MANAGED_MARKER: &str = "managed-by: ci";
pub const MANAGED_HOOK_NAME: &str = "hook";
pub const MANAGED_RUNNER_NAME: &str = "run";

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
    let target_arches = install_target_arches(args.source.as_deref(), &ctx.config.defaults.arch);
    let ci_bins = managed_runner_targets(&ctx.repo, &target_arches);
    let hooks_dir = ctx.repo.git_dir.join("hooks");

    ctx.output
        .info(format!("Installing ci into {}", ctx.repo.git_dir.display()));
    ctx.output
        .info(format!("Mode: {:?}", args.mode).to_lowercase());

    if args.dry_run {
        println!("would create directory {}", ci_bin_dir.display());
        for (arch, ci_bin) in &ci_bins {
            let source =
                install_source_for_arch(&ctx.repo.current_exe, args.source.as_deref(), arch);
            println!(
                "would install binary {} from {}",
                ci_bin.display(),
                source.display()
            );
        }
    } else {
        fs::create_dir_all(&ci_bin_dir)?;
        for (arch, ci_bin) in &ci_bins {
            let source =
                install_source_for_arch(&ctx.repo.current_exe, args.source.as_deref(), arch);
            install_binary(&source, ci_bin, &args.mode)?;
        }
        install_hook_dispatcher(&managed_hook_dispatcher_path(&ctx.repo))?;
        fs::create_dir_all(&hooks_dir)?;
    }

    for hook in hooks {
        let hook_path = hooks_dir.join(hook);
        if args.dry_run {
            println!("would install hook {}", hook_path.display());
            continue;
        }
        install_hook(&hook_path, hook, args.force, args.backup_existing)?;
    }

    ctx.output.info("Done.");
    Ok(0)
}

pub fn cmd_update(ctx: &AppContext, args: &UpdateArgs) -> Result<i32> {
    let target_arches = install_target_arches(args.source.as_deref(), &ctx.config.defaults.arch);
    let ci_bins = managed_runner_targets(&ctx.repo, &target_arches);
    let installed_ci_bins = managed_runner_paths(&ctx.repo, &ctx.config.defaults.arch);
    let legacy_ci_bin = legacy_managed_runner_path(&ctx.repo);

    if !installed_ci_bins
        .iter()
        .any(|path| path_exists_or_symlink(path))
        && !path_exists_or_symlink(&legacy_ci_bin)
    {
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
        install_hook_dispatcher(&managed_hook_dispatcher_path(&ctx.repo))?;
    }

    refresh_managed_hooks(ctx, args.dry_run)?;
    ctx.output.info("Updated ci installation.");
    Ok(0)
}

pub fn cmd_uninstall(ctx: &AppContext, args: &UninstallArgs) -> Result<i32> {
    let hooks = parse_hooks(args.hooks.as_deref().or(Some("all")), ctx.repo.is_bare)?;
    let hooks_dir = ctx.repo.git_dir.join("hooks");

    for hook in hooks {
        let hook_path = hooks_dir.join(hook);
        let backup_path = hooks_dir.join(format!("{hook}.ci-backup"));

        if !hook_path.exists() {
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
    for hook in all_hooks() {
        let hook_path = hooks_dir.join(hook);
        if is_managed_hook(&hook_path) {
            if dry_run {
                println!("would refresh hook {}", hook_path.display());
            } else {
                install_hook(&hook_path, hook, true, false)?;
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

fn install_hook_dispatcher(path: &Path) -> Result<()> {
    let script = format!(
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
    );
    fs::write(path, script)?;
    chmod_executable(path)?;
    Ok(())
}

fn install_hook(hook_path: &Path, hook: &str, force: bool, backup_existing: bool) -> Result<()> {
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
    symlink(format!("../ci/{MANAGED_HOOK_NAME}"), hook_path)?;
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
        .map(|target| target == Path::new("../ci").join(MANAGED_HOOK_NAME))
        .unwrap_or(false)
    {
        return true;
    }

    fs::read_to_string(path)
        .map(|content| content.contains(MANAGED_MARKER))
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
