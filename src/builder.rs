use crate::tempdir::DIR_PREFIX;
use crate::tempfile::FILE_PREFIX;
use crate::{Error, TempDir, TempFile};
use std::path::PathBuf;

/// A builder for configuring and creating a [`TempFile`].
///
/// Obtain one via [`TempFile::builder`]. The file name is composed as
/// `{prefix}{random}{suffix}`, where the random core is unpredictable and
/// collision-resistant, and the file is created with an exclusive
/// (`O_EXCL`) create so it never clobbers an existing file.
///
/// ## Example
///
/// ```
/// # use async_tempfile::{TempFile, Error};
/// # let _ = tokio_test::block_on(async {
/// let file = TempFile::builder()
///     .prefix("session_")
///     .suffix(".tmp")
///     .dir(std::env::temp_dir())
///     .create()
///     .await?;
///
/// let name = file.file_path().file_name().unwrap().to_string_lossy().into_owned();
/// assert!(name.starts_with("session_"));
/// assert!(name.ends_with(".tmp"));
/// # Ok::<(), Error>(())
/// # });
/// ```
#[derive(Debug, Clone)]
pub struct TempFileBuilder {
    dir: Option<PathBuf>,
    prefix: String,
    suffix: String,
}

impl TempFileBuilder {
    pub(crate) fn new() -> Self {
        Self {
            dir: None,
            prefix: FILE_PREFIX.to_string(),
            suffix: String::new(),
        }
    }

    /// Sets the file name prefix. Defaults to `atmp_`.
    pub fn prefix<S: Into<String>>(mut self, prefix: S) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Sets the file name suffix (for example a file extension). Defaults to empty.
    pub fn suffix<S: Into<String>>(mut self, suffix: S) -> Self {
        self.suffix = suffix.into();
        self
    }

    /// Sets the directory to create the file in. Defaults to the system
    /// temporary directory ([`std::env::temp_dir`]).
    pub fn dir<P: Into<PathBuf>>(mut self, dir: P) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Creates the temporary file with the configured options.
    pub async fn create(self) -> Result<TempFile, Error> {
        let dir = self.dir.unwrap_or_else(std::env::temp_dir);
        TempFile::create_with_affixes(&dir, &self.prefix, &self.suffix).await
    }
}

/// A builder for configuring and creating a [`TempDir`].
///
/// Obtain one via [`TempDir::builder`]. The directory name is composed as
/// `{prefix}{random}{suffix}` with an unpredictable, collision-resistant random
/// core, created with an exclusive create.
///
/// ## Example
///
/// ```
/// # use async_tempfile::{TempDir, Error};
/// # let _ = tokio_test::block_on(async {
/// let dir = TempDir::builder()
///     .prefix("workspace_")
///     .create()
///     .await?;
///
/// let name = dir.dir_path().file_name().unwrap().to_string_lossy().into_owned();
/// assert!(name.starts_with("workspace_"));
/// # Ok::<(), Error>(())
/// # });
/// ```
#[derive(Debug, Clone)]
pub struct TempDirBuilder {
    root: Option<PathBuf>,
    prefix: String,
    suffix: String,
}

impl TempDirBuilder {
    pub(crate) fn new() -> Self {
        Self {
            root: None,
            prefix: DIR_PREFIX.to_string(),
            suffix: String::new(),
        }
    }

    /// Sets the directory name prefix. Defaults to `atmpd_`.
    pub fn prefix<S: Into<String>>(mut self, prefix: S) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Sets the directory name suffix. Defaults to empty.
    pub fn suffix<S: Into<String>>(mut self, suffix: S) -> Self {
        self.suffix = suffix.into();
        self
    }

    /// Sets the root directory to create the directory in. Defaults to the
    /// system temporary directory ([`std::env::temp_dir`]).
    pub fn dir<P: Into<PathBuf>>(mut self, root: P) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Creates the temporary directory with the configured options.
    pub async fn create(self) -> Result<TempDir, Error> {
        let root = self.root.unwrap_or_else(std::env::temp_dir);
        TempDir::create_with_affixes(&root, &self.prefix, &self.suffix).await
    }
}
