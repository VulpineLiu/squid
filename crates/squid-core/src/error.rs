//! Error types shared by every squid crate.

use std::io;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, SquidError>;

#[derive(Debug, thiserror::Error)]
pub enum SquidError {
    #[error("io error while accessing {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("path refused by safety policy: {path} ({reason})")]
    Safety { path: PathBuf, reason: String },

    #[error("nothing to do")]
    NothingToDo,

    #[error("operation cancelled")]
    Cancelled,

    #[error("unsupported on {platform}: {what}")]
    Unsupported {
        platform: &'static str,
        what: &'static str,
    },

    #[error(
        "permission denied for {path} — grant Full Disk Access to squid, or run again with sudo"
    )]
    Permission { path: PathBuf },

    #[error("{0}")]
    Other(String),
}

impl SquidError {
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        match source.kind() {
            io::ErrorKind::PermissionDenied => SquidError::Permission { path: path.into() },
            _ => SquidError::Io {
                path: path.into(),
                source,
            },
        }
    }

    pub fn safety(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        SquidError::Safety {
            path: path.into(),
            reason: reason.into(),
        }
    }

    /// Errors that are worth surfacing to the user, versus noisy "it moved
    /// while we were looking at it" cases that we just count and ignore.
    pub fn is_noteworthy(&self) -> bool {
        !matches!(
            self,
            SquidError::Io {
                source,
                ..
            } if matches!(
                source.kind(),
                io::ErrorKind::NotFound
            )
        )
    }
}

impl From<SquidError> for io::Error {
    fn from(err: SquidError) -> io::Error {
        match err {
            SquidError::Io { source, .. } => source,
            other => io::Error::other(other.to_string()),
        }
    }
}
