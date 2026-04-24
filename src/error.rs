use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, CiError>;

#[derive(Debug, Error)]
pub enum CiError {
    #[error("{0}")]
    Usage(String),

    #[error("{0}")]
    Message(String),

    #[error("workflow found but is not executable: {0}")]
    NotExecutable(PathBuf),

    #[error("workflow not found: {0}")]
    NotFound(String),

    #[error("interrupted")]
    Interrupted,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Walkdir(#[from] walkdir::Error),

    #[error(transparent)]
    GlobPattern(#[from] glob::PatternError),

    #[error(transparent)]
    Glob(#[from] glob::GlobError),
}

impl CiError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 2,
            Self::NotExecutable(_) => 126,
            Self::NotFound(_) => 127,
            Self::Interrupted => 130,
            Self::Message(_) => 3,
            Self::Io(_)
            | Self::Yaml(_)
            | Self::Json(_)
            | Self::Walkdir(_)
            | Self::GlobPattern(_)
            | Self::Glob(_) => 3,
        }
    }
}
