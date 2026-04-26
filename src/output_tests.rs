use std::path::PathBuf;

use crate::cli::GlobalOptions;
use crate::config::{
    Architecture, ColorWhen, ContainerConfig, ContainerRuntime, Defaults, DefaultsConfig, GitMode,
    InstallMode,
};

use super::{Output, Verbosity};

fn globals() -> GlobalOptions {
    GlobalOptions {
        verbose: 0,
        quiet: false,
        silent: false,
        repo: PathBuf::from("."),
        ci_dir: PathBuf::from(".ci"),
        config: None,
        color: ColorWhen::Never,
        git_mode: None,
        git_command: None,
        git_image: None,
        container: false,
        no_container: false,
        arch: Vec::new(),
        tech_stack: None,
    }
}

#[test]
fn verbose_overrides_silent() {
    let mut global = globals();
    global.verbose = 1;
    global.silent = true;

    assert_eq!(
        Output::from_globals(&global).verbosity(),
        Verbosity::Verbose(1)
    );
    assert!(Output::from_globals(&global).is_verbose());
}

#[test]
fn config_can_enable_silent_by_default() {
    let global = globals();
    let defaults = defaults(false, true);

    assert_eq!(
        Output::from_settings(&global, Some(&defaults)).verbosity(),
        Verbosity::Silent
    );
    assert!(!Output::from_settings(&global, Some(&defaults)).is_verbose());
}

#[test]
fn config_can_enable_quiet_by_default() {
    let global = globals();
    let defaults = defaults(true, false);

    assert_eq!(
        Output::from_settings(&global, Some(&defaults)).verbosity(),
        Verbosity::Quiet
    );
}

#[test]
fn cli_silent_overrides_config_quiet() {
    let mut global = globals();
    global.silent = true;
    let defaults = defaults(true, false);

    assert_eq!(
        Output::from_settings(&global, Some(&defaults)).verbosity(),
        Verbosity::Silent
    );
}

#[test]
fn verbose_overrides_config_silent() {
    let mut global = globals();
    global.verbose = 1;
    let defaults = defaults(false, true);

    assert_eq!(
        Output::from_settings(&global, Some(&defaults)).verbosity(),
        Verbosity::Verbose(1)
    );
}

#[test]
fn policy_silent_overrides_verbose() {
    let mut global = globals();
    global.verbose = 1;
    let defaults = defaults(false, false);
    let policy = DefaultsConfig {
        silent: Some(true),
        ..DefaultsConfig::default()
    };

    assert_eq!(
        Output::from_settings_with_policy(&global, Some(&defaults), Some(&policy)).verbosity(),
        Verbosity::Silent
    );
}

#[test]
fn policy_quiet_overrides_verbose() {
    let mut global = globals();
    global.verbose = 1;
    let defaults = defaults(false, false);
    let policy = DefaultsConfig {
        quiet: Some(true),
        ..DefaultsConfig::default()
    };

    assert_eq!(
        Output::from_settings_with_policy(&global, Some(&defaults), Some(&policy)).verbosity(),
        Verbosity::Quiet
    );
}

fn defaults(quiet: bool, silent: bool) -> Defaults {
    Defaults {
        shell: "/bin/sh".to_string(),
        quiet,
        silent,
        fail_fast: true,
        arch: vec![Architecture::host()],
        container: ContainerConfig::default(),
        container_runtime: ContainerRuntime::Auto,
        git_mode: GitMode::Auto,
        git_command: None,
        git_image: "docker.io/alpine/git:latest".to_string(),
        install_mode: InstallMode::Link,
        recursive_checkout: true,
        branch_allow: Vec::new(),
        artifact_store: PathBuf::from("artifacts"),
        actions_cache: PathBuf::from("actions-cache"),
        node_image: "docker.io/library/node:20-alpine".to_string(),
    }
}
