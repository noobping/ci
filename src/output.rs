use std::io::IsTerminal;
use std::sync::Once;

use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriterExt;

use crate::cli::GlobalOptions;
use crate::config::{ColorWhen, Defaults};

static INIT_TRACING: Once = Once::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbosity {
    Silent,
    Quiet,
    Normal,
    Verbose(u8),
}

#[derive(Clone, Debug)]
pub struct Output {
    verbosity: Verbosity,
}

impl Output {
    pub fn from_globals(global: &GlobalOptions) -> Self {
        Self::from_settings(global, None)
    }

    pub fn from_settings(global: &GlobalOptions, defaults: Option<&Defaults>) -> Self {
        init_tracing(global);

        let verbosity = if global.verbose > 0 {
            Verbosity::Verbose(global.verbose)
        } else if global.quiet {
            Verbosity::Quiet
        } else if global.silent || defaults.map(|value| value.silent).unwrap_or(false) {
            Verbosity::Silent
        } else {
            Verbosity::Normal
        };

        Self { verbosity }
    }

    pub fn verbosity(&self) -> Verbosity {
        self.verbosity
    }

    pub fn is_verbose(&self) -> bool {
        matches!(self.verbosity, Verbosity::Verbose(_))
    }

    pub fn info(&self, message: impl AsRef<str>) {
        if !matches!(self.verbosity, Verbosity::Quiet | Verbosity::Silent) {
            tracing::info!("{}", message.as_ref());
        }
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        if !matches!(self.verbosity, Verbosity::Silent) {
            tracing::warn!("{}", message.as_ref());
        }
    }

    pub fn error(&self, message: impl AsRef<str>) {
        tracing::error!("{}", message.as_ref());
    }

    pub fn verbose(&self, message: impl AsRef<str>) {
        if let Verbosity::Verbose(level) = self.verbosity {
            if level > 1 {
                tracing::trace!("{}", message.as_ref());
            } else {
                tracing::debug!("{}", message.as_ref());
            }
        }
    }
}

fn init_tracing(global: &GlobalOptions) {
    INIT_TRACING.call_once(|| {
        let writer = std::io::stderr
            .with_max_level(Level::WARN)
            .or_else(std::io::stdout);
        let ansi = color_enabled(global.color);
        let max_level = if global.verbose > 1 {
            Level::TRACE
        } else if global.verbose > 0 {
            Level::DEBUG
        } else {
            Level::INFO
        };

        let _ = tracing_subscriber::fmt()
            .compact()
            .without_time()
            .with_target(false)
            .with_ansi(ansi)
            .with_max_level(max_level)
            .with_writer(writer)
            .try_init();
    });
}

fn color_enabled(color: ColorWhen) -> bool {
    match color {
        ColorWhen::Always => true,
        ColorWhen::Never => false,
        ColorWhen::Auto => std::io::stdout().is_terminal() || std::io::stderr().is_terminal(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::cli::GlobalOptions;
    use crate::config::{
        Architecture, ColorWhen, ContainerConfig, ContainerRuntime, Defaults, GitMode,
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
            git_image: None,
            arch: Vec::new(),
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
        let defaults = defaults(true);

        assert_eq!(
            Output::from_settings(&global, Some(&defaults)).verbosity(),
            Verbosity::Silent
        );
        assert!(!Output::from_settings(&global, Some(&defaults)).is_verbose());
    }

    #[test]
    fn verbose_overrides_config_silent() {
        let mut global = globals();
        global.verbose = 1;
        let defaults = defaults(true);

        assert_eq!(
            Output::from_settings(&global, Some(&defaults)).verbosity(),
            Verbosity::Verbose(1)
        );
    }

    fn defaults(silent: bool) -> Defaults {
        Defaults {
            shell: "/bin/sh".to_string(),
            silent,
            fail_fast: true,
            arch: vec![Architecture::host()],
            container: ContainerConfig::default(),
            container_runtime: ContainerRuntime::Auto,
            git_mode: GitMode::Auto,
            git_image: "docker.io/alpine/git:latest".to_string(),
            recursive_checkout: true,
            branch_allow: Vec::new(),
            artifact_store: PathBuf::from("artifacts"),
            actions_cache: PathBuf::from("actions-cache"),
            node_image: "docker.io/library/node:20-alpine".to_string(),
        }
    }
}
