use std::io::IsTerminal;
use std::sync::Once;

use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriterExt;

use crate::cli::GlobalOptions;
use crate::config::{ColorWhen, Defaults, DefaultsConfig};

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
        Self::from_settings_with_policy(global, defaults, None)
    }

    pub fn from_settings_with_policy(
        global: &GlobalOptions,
        defaults: Option<&Defaults>,
        policy: Option<&DefaultsConfig>,
    ) -> Self {
        init_tracing(global);

        let mut verbose = global.verbose;
        let mut quiet = defaults.map(|value| value.quiet).unwrap_or(false);
        let mut silent = defaults.map(|value| value.silent).unwrap_or(false);

        if global.verbose > 0 {
            quiet = false;
            silent = false;
        } else {
            if global.quiet {
                quiet = true;
                silent = false;
            }
            if global.silent {
                quiet = false;
                silent = true;
            }
        }

        if let Some(policy) = policy {
            if let Some(value) = policy.quiet {
                quiet = value;
                if value {
                    verbose = 0;
                    silent = false;
                }
            }
            if let Some(value) = policy.silent {
                silent = value;
                if value {
                    verbose = 0;
                    quiet = false;
                }
            }
        }

        let verbosity = if verbose > 0 {
            Verbosity::Verbose(verbose)
        } else if silent {
            Verbosity::Silent
        } else if quiet {
            Verbosity::Quiet
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

    pub fn is_quiet_or_silent(&self) -> bool {
        matches!(self.verbosity, Verbosity::Quiet | Verbosity::Silent)
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
#[path = "output_tests.rs"]
mod tests;
