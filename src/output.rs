use std::io::IsTerminal;
use std::sync::Once;

use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriterExt;

use crate::cli::GlobalOptions;
use crate::config::ColorWhen;

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
        init_tracing(global);

        let verbosity = if global.verbose > 0 {
            Verbosity::Verbose(global.verbose)
        } else if global.silent {
            Verbosity::Silent
        } else if global.quiet {
            Verbosity::Quiet
        } else {
            Verbosity::Normal
        };

        Self { verbosity }
    }

    pub fn verbosity(&self) -> Verbosity {
        self.verbosity
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

        let _ = tracing_subscriber::fmt()
            .compact()
            .without_time()
            .with_target(false)
            .with_ansi(ansi)
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
    use crate::config::ColorWhen;

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
        }
    }

    #[test]
    fn verbose_overrides_silent() {
        let mut global = globals();
        global.verbose = 1;
        global.silent = true;

        assert_eq!(Output::from_globals(&global).verbosity(), Verbosity::Verbose(1));
    }
}
