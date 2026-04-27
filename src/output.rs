use std::fs::File;
use std::io::{IsTerminal, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::sync::{Mutex, Once, OnceLock};

use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriterExt;

use crate::cli::GlobalOptions;
use crate::config::{ColorWhen, Defaults, DefaultsConfig};

static INIT_TRACING: Once = Once::new();
static MUTE_OUTPUT: Once = Once::new();
static ORIGINAL_STDERR: OnceLock<Mutex<File>> = OnceLock::new();
const STDOUT_FILENO: std::os::raw::c_int = 1;
const STDERR_FILENO: std::os::raw::c_int = 2;

unsafe extern "C" {
    fn dup(oldfd: std::os::raw::c_int) -> std::os::raw::c_int;
    fn dup2(oldfd: std::os::raw::c_int, newfd: std::os::raw::c_int) -> std::os::raw::c_int;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbosity {
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

        if global.verbose > 0 {
            quiet = false;
        } else if global.quiet {
            quiet = true;
        }

        if let Some(policy) = policy {
            if let Some(value) = policy.quiet {
                if value {
                    verbose = 0;
                }
                quiet = value;
            }
        }

        let verbosity = if verbose > 0 {
            Verbosity::Verbose(verbose)
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

    pub fn verbose_level(&self) -> u8 {
        match self.verbosity {
            Verbosity::Verbose(level) => level,
            _ => 0,
        }
    }

    pub fn is_quiet(&self) -> bool {
        matches!(self.verbosity, Verbosity::Quiet)
    }

    pub fn info(&self, message: impl AsRef<str>) {
        if !self.is_quiet() {
            tracing::info!("{}", message.as_ref());
        }
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        if !self.is_quiet() {
            tracing::warn!("{}", message.as_ref());
        }
    }

    pub fn error(&self, message: impl AsRef<str>) {
        if self.is_quiet() {
            write_critical_error(message.as_ref());
        } else {
            tracing::error!("{}", message.as_ref());
        }
    }

    pub fn verbose(&self, message: impl AsRef<str>) {
        self.verbose_at(1, message);
    }

    pub fn verbose_at(&self, level: u8, message: impl AsRef<str>) {
        let level = level.max(1);
        if self.verbose_level() >= level {
            match level {
                1 => tracing::debug!("{}", message.as_ref()),
                _ => tracing::trace!("{}", message.as_ref()),
            }
        }
    }
}

pub fn mute_process_output() {
    MUTE_OUTPUT.call_once(|| {
        let stderr_fd = unsafe { dup(STDERR_FILENO) };
        if stderr_fd >= 0 {
            let stderr = unsafe { File::from_raw_fd(stderr_fd) };
            let _ = ORIGINAL_STDERR.set(Mutex::new(stderr));
        }

        let Ok(devnull) = std::fs::OpenOptions::new().write(true).open("/dev/null") else {
            return;
        };

        unsafe {
            dup2(devnull.as_raw_fd(), STDOUT_FILENO);
            dup2(devnull.as_raw_fd(), STDERR_FILENO);
        }
    });
}

fn write_critical_error(message: &str) {
    if let Some(stderr) = ORIGINAL_STDERR.get() {
        if let Ok(mut stderr) = stderr.lock() {
            let _ = writeln!(stderr, "ERROR {message}");
            return;
        }
    }

    tracing::error!("{message}");
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
