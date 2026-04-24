use crate::cli::GlobalOptions;

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
        let verbosity = if global.silent {
            Verbosity::Silent
        } else if global.quiet {
            Verbosity::Quiet
        } else if global.verbose > 0 {
            Verbosity::Verbose(global.verbose)
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
            println!("{}", message.as_ref());
        }
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        if !matches!(self.verbosity, Verbosity::Silent) {
            eprintln!("ci: warning: {}", message.as_ref());
        }
    }

    pub fn error(&self, message: impl AsRef<str>) {
        eprintln!("ci: error: {}", message.as_ref());
    }

    pub fn verbose(&self, message: impl AsRef<str>) {
        if matches!(self.verbosity, Verbosity::Verbose(_)) {
            eprintln!("ci: {}", message.as_ref());
        }
    }
}
