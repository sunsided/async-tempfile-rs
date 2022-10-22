use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum Error {
    /// An invalid or missing directory was specified.
    InvalidDirectory,
    /// An invalid or missing file was specified.
    InvalidFile,
    /// An I/O error occurred.
    Io(std::io::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDirectory => write!(f, "An invalid directory was specified"),
            Self::InvalidFile => write!(f, "An invalid file name was specified"),
            Self::Io(e) => Display::fmt(e, f),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
