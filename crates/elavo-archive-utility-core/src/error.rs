use std::fmt;

/// A compress or extract failure the window can show as a sentence.
#[derive(Debug)]
pub enum Error {
    Cancelled,
    Empty,
    Unsupported {
        format: String,
        action: &'static str,
    },
    /// An external program Archive Utility shells out to is not on `PATH`.
    ToolMissing {
        tool: &'static str,
        purpose: String,
    },
    Failed(String),
    PasswordRequired,
    WrongPassword,
    UnsafePath(String),
    Io(std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => write!(f, "Cancelled."),
            Error::Empty => write!(f, "Add a file or a folder first."),
            Error::Unsupported { format, action } => {
                write!(f, "{format} cannot {action}.")
            }
            Error::ToolMissing { tool, purpose } => {
                write!(f, "Install {tool} to {purpose}.")
            }
            Error::Failed(message) => write!(f, "{message}"),
            Error::PasswordRequired => write!(f, "This archive needs a password."),
            Error::WrongPassword => write!(f, "That password did not open the archive."),
            Error::UnsafePath(path) => {
                write!(f, "The archive contains an unsafe path ({path}).")
            }
            Error::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::Interrupted && err.to_string().contains("cancelled") {
            Error::Cancelled
        } else {
            Error::Io(err)
        }
    }
}

pub(crate) fn cancelled_io() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled")
}
