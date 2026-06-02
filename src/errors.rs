use std::fmt::{Display, Formatter};
use std::path::PathBuf;

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

/// The error returned by [`crate::TempFile::persist`] and
/// [`crate::TempDir::persist`] when the move to the target path fails.
///
/// On failure the temporary file or directory is **not** deleted: it is left in
/// place at [`PersistError::path`] so the caller can recover the data instead of
/// losing it. To restore automatic cleanup, re-wrap that path with
/// `from_existing(path, Ownership::Owned)`.
///
/// There is intentionally no `From<PersistError> for Error` conversion: a
/// blanket `?` would silently drop [`PersistError::path`], the one piece of
/// information that makes recovery possible. Handle the two fields explicitly,
/// or use `map_err(|e| e.error)` if you only care about the underlying error.
#[derive(Debug)]
pub struct PersistError {
    /// The underlying error that prevented the move (for example a cross-device
    /// rename, a permission error, or a missing target directory).
    pub error: Error,
    /// The path at which the temporary file or directory still resides.
    pub path: PathBuf,
}

impl Display for PersistError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "failed to persist temporary at {:?}: {}",
            self.path, self.error
        )
    }
}

impl std::error::Error for PersistError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
