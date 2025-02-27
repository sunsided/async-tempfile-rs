#[cfg(not(feature = "uuid"))]
use crate::RandomName;
use crate::{Error, Ownership};
use std::borrow::Borrow;
use std::fmt::{Debug, Formatter};
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "uuid")]
use uuid::Uuid;

const DIR_PREFIX: &str = "atmpd_";

/// A named temporary directory that will be cleaned automatically
/// after the last reference to it is dropped.
pub struct TempDir {
    /// A local reference to the directory.
    dir: ManuallyDrop<PathBuf>,

    /// A shared pointer to the owned (or non-owned) directory.
    /// The `Arc` ensures that the enclosed dir is kept alive
    /// until all references to it are dropped.
    core: ManuallyDrop<Arc<TempDirCore>>,
}

/// The instance that tracks the temporary file.
/// If dropped, the file will be deleted.
struct TempDirCore {
    /// The path of the contained file.
    path: PathBuf,

    /// A hacky approach to allow for "non-owned" files.
    /// If set to `Ownership::Owned`, the file specified in `path` will be deleted
    /// when this instance is dropped. If set to `Ownership::Borrowed`, the file will be kept.
    ownership: Ownership,
}

impl TempDir {
    /// Creates a new temporary directory in the default location.
    /// When the instance goes out of scope, the directory will be deleted.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempDir, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let dir = TempDir::new().await?;
    ///
    /// // The file exists.
    /// let dir_path = dir.dir_path().clone();
    /// assert!(fs::metadata(dir_path.clone()).await.is_ok());
    ///
    /// // Deletes the directory.
    /// drop(dir);
    ///
    /// // The directory was removed.
    /// assert!(fs::metadata(dir_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub async fn new() -> Result<Self, Error> {
        Self::new_in(Self::default_dir()).await
    }

    /// Creates a new temporary directory in the default location.
    /// When the instance goes out of scope, the directory will be deleted.
    ///
    /// ## Arguments
    ///
    /// * `name` - The name of the directory to create in the default temporary directory root.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempDir, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let dir = TempDir::new_with_name("temporary.dir").await?;
    ///
    /// // The directory exists.
    /// let dir_path = dir.dir_path().clone();
    /// assert!(fs::metadata(dir_path.clone()).await.is_ok());
    ///
    /// // Deletes the directory.
    /// drop(dir);
    ///
    /// // The directory was removed.
    /// assert!(fs::metadata(dir_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub async fn new_with_name<N: AsRef<str>>(name: N) -> Result<Self, Error> {
        Self::new_with_name_in(name, Self::default_dir()).await
    }

    /// Creates a new temporary directory in the default location.
    /// When the instance goes out of scope, the directory will be deleted.
    ///
    /// ## Arguments
    ///
    /// * `uuid` - A UUID to use as a suffix to the directory name.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempDir, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let id = uuid::Uuid::new_v4();
    /// let dir = TempDir::new_with_uuid(id).await?;
    ///
    /// // The directory exists.
    /// let dir_path = dir.dir_path().clone();
    /// assert!(fs::metadata(dir_path.clone()).await.is_ok());
    ///
    /// // Deletes the directory.
    /// drop(dir);
    ///
    /// // The directory was removed.
    /// assert!(fs::metadata(dir_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    #[cfg_attr(docsrs, doc(cfg(feature = "uuid")))]
    #[cfg(feature = "uuid")]
    pub async fn new_with_uuid(uuid: Uuid) -> Result<Self, Error> {
        Self::new_with_uuid_in(uuid, Self::default_dir()).await
    }

    /// Creates a new temporary directory in the specified location.
    /// When the instance goes out of scope, the directory will be deleted.
    ///
    /// ## Crate Features
    ///
    /// * `uuid` - When the `uuid` crate feature is enabled, a random UUIDv4 is used to
    ///   generate the temporary directory name.
    ///
    /// ## Arguments
    ///
    /// * `dir` - The directory to create the directory in.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempDir, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let path = std::env::temp_dir();
    /// let dir = TempDir::new_in(path).await?;
    ///
    /// // The directory exists.
    /// let dir_path = dir.dir_path().clone();
    /// assert!(fs::metadata(dir_path.clone()).await.is_ok());
    ///
    /// // Deletes the directory.
    /// drop(dir);
    ///
    /// // The directory was removed.
    /// assert!(fs::metadata(dir_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    pub async fn new_in<P: Borrow<Path>>(root_dir: P) -> Result<Self, Error> {
        #[cfg(feature = "uuid")]
        {
            let id = Uuid::new_v4();
            Self::new_with_uuid_in(id, root_dir).await
        }

        #[cfg(not(feature = "uuid"))]
        {
            let name = RandomName::new(DIR_PREFIX);
            Self::new_with_name_in(name, root_dir).await
        }
    }

    /// Creates a new temporary directory in the specified location.
    /// When the instance goes out of scope, the directory will be deleted.
    ///
    /// ## Arguments
    ///
    /// * `dir` - The root directory to create the directory in.
    /// * `name` - The directory name to use.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempDir, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let path = std::env::temp_dir();
    /// let dir = TempDir::new_with_name_in("temporary.dir", path).await?;
    ///
    /// // The directory exists.
    /// let dir_path = dir.dir_path().clone();
    /// assert!(fs::metadata(dir_path.clone()).await.is_ok());
    ///
    /// // Deletes the directory.
    /// drop(dir);
    ///
    /// // The directory was removed.
    /// assert!(fs::metadata(dir_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub async fn new_with_name_in<N: AsRef<str>, P: Borrow<Path>>(
        name: N,
        root_dir: P,
    ) -> Result<Self, Error> {
        let dir = root_dir.borrow();
        if !dir.is_dir() {
            return Err(Error::InvalidDirectory);
        }
        let file_name = name.as_ref();
        let mut path = PathBuf::from(dir);
        path.push(file_name);
        Self::new_internal(path, Ownership::Owned).await
    }

    /// Creates a new directory file in the specified location.
    /// When the instance goes out of scope, the directory will be deleted.
    ///
    /// ## Arguments
    ///
    /// * `dir` - The root directory to create the directory in.
    /// * `uuid` - A UUID to use as a suffix to the directory name.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempDir, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let path = std::env::temp_dir();
    /// let id = uuid::Uuid::new_v4();
    /// let dir = TempDir::new_with_uuid_in(id, path).await?;
    ///
    /// // The directory exists.
    /// let dir_path = dir.dir_path().clone();
    /// assert!(fs::metadata(dir_path.clone()).await.is_ok());
    ///
    /// // Deletes the directory.
    /// drop(dir);
    ///
    /// // The directory was removed.
    /// assert!(fs::metadata(dir_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    #[cfg_attr(docsrs, doc(cfg(feature = "uuid")))]
    #[cfg(feature = "uuid")]
    pub async fn new_with_uuid_in<P: Borrow<Path>>(uuid: Uuid, root_dir: P) -> Result<Self, Error> {
        let file_name = format!("{}{}", DIR_PREFIX, uuid);
        Self::new_with_name_in(file_name, root_dir).await
    }

    /// Wraps a new instance of this type around an existing directory.
    /// If `ownership` is set to [`Ownership::Borrowed`], this method does not take ownership of
    /// the file, i.e. the directory will not be deleted when the instance is dropped.
    ///
    /// ## Arguments
    ///
    /// * `path` - The path of the directory to wrap.
    /// * `ownership` - The ownership of the directory.
    pub async fn from_existing(path: PathBuf, ownership: Ownership) -> Result<Self, Error> {
        if !path.is_dir() {
            return Err(Error::InvalidDirectory);
        }
        Self::new_internal(path, ownership).await
    }

    /// Returns the path of the underlying temporary directory.
    pub fn dir_path(&self) -> &PathBuf {
        &self.core.path
    }

    /// Creates a new [`TempDir`] instance that shares the same underlying
    /// file handle as the existing [`TempDir`] instance.
    /// Reads, writes, and seeks will affect both [`TempDir`] instances simultaneously.
    #[allow(dead_code)]
    pub async fn try_clone(&self) -> Result<TempDir, Error> {
        Ok(TempDir {
            core: self.core.clone(),
            dir: self.dir.clone(),
        })
    }

    /// Determines the ownership of the temporary directory.
    /// ### Example
    /// ```
    /// # use async_tempfile::{Ownership, TempDir};
    /// # let _ = tokio_test::block_on(async {
    /// let dir = TempDir::new().await?;
    /// assert_eq!(dir.ownership(), Ownership::Owned);
    /// # drop(dir);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # });
    /// ```
    pub fn ownership(&self) -> Ownership {
        self.core.ownership
    }

    /// Asynchronously drops the [`TempDir`] instance by moving the drop operation
    /// to a blocking thread, avoiding potential blocking of the async runtime.
    ///
    /// This method is useful in cases where manually handling the blocking drop
    /// within an async context is required.
    ///
    /// ## Example
    /// ```
    /// # use async_tempfile::TempDir;
    /// # let _ = tokio_test::block_on(async {
    /// let dir = TempDir::new().await?;
    ///
    /// // Drop the directory asynchronously.
    /// dir.drop_async().await;
    ///
    /// // The directory is now removed.
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # });
    /// ```
    ///
    /// Note: This function spawns a blocking task for the drop operation.
    pub async fn drop_async(self) {
        tokio::task::spawn_blocking(move || drop(self)).await.ok();
    }

    async fn new_internal<P: Borrow<Path>>(path: P, ownership: Ownership) -> Result<Self, Error> {
        // Create the directory and all its parents.
        tokio::fs::create_dir_all(path.borrow()).await?;

        let core = TempDirCore {
            ownership,
            path: PathBuf::from(path.borrow()),
        };

        Ok(Self {
            dir: ManuallyDrop::new(PathBuf::from(path.borrow())),
            core: ManuallyDrop::new(Arc::new(core)),
        })
    }

    /// Gets the default temporary file directory.
    #[inline(always)]
    fn default_dir() -> PathBuf {
        std::env::temp_dir()
    }
}

/// Ensures the file handles are closed before the core reference is freed.
/// If the core reference would be freed while handles are still open, it is
/// possible that the underlying file cannot be deleted.
impl Drop for TempDir {
    fn drop(&mut self) {
        // Ensure all directory handles are closed before we attempt to delete the directory itself via core.
        drop(unsafe { ManuallyDrop::take(&mut self.dir) });
        drop(unsafe { ManuallyDrop::take(&mut self.core) });
    }
}

/// Ensures that the underlying directory is deleted if this is an owned instance.
/// If the underlying directory is not owned, this operation does nothing.
impl Drop for TempDirCore {
    /// See also [`TempDirCore::close`].
    fn drop(&mut self) {
        // Ensure we don't drop borrowed directories.
        if self.ownership != Ownership::Owned {
            return;
        }

        // TODO: Use asynchronous variant if running in an async context.
        // Note that if TempDir is used from the executor's handle,
        //      this may block the executor itself.
        // Using remove_dir_all to delete all content recursively.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl Debug for TempDirCore {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.path)
    }
}

impl Debug for TempDir {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.core)
    }
}

/// Allows implicit treatment of TempDir as a Path.
impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.dir
    }
}

impl Borrow<Path> for TempDir {
    fn borrow(&self) -> &Path {
        &self.dir
    }
}

impl Borrow<Path> for &TempDir {
    fn borrow(&self) -> &Path {
        &self.dir
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TempFile;

    #[tokio::test]
    async fn test_new() -> Result<(), Error> {
        let dir = TempDir::new().await?;

        // The directory exists.
        let dir_path = dir.dir_path().clone();
        assert!(tokio::fs::metadata(dir_path.clone()).await.is_ok());

        // Deletes the directory.
        drop(dir);

        assert!(tokio::fs::metadata(dir_path).await.is_err());
        Ok(())
    }

    #[tokio::test]
    #[cfg(not(target_os = "windows"))]
    async fn test_files_in_dir() -> Result<(), Error> {
        let dir = TempDir::new().await?;
        let file = TempFile::new_in(&dir).await?;
        let file2 = TempFile::new_in(&dir).await?;

        // The directory exists.
        let dir_path = dir.dir_path().clone();
        assert!(tokio::fs::metadata(dir_path.clone()).await.is_ok());

        // The files exist.
        let file_path = file.file_path().clone();
        let file_path2 = file2.file_path().clone();
        assert!(tokio::fs::metadata(file_path.clone()).await.is_ok());
        assert!(tokio::fs::metadata(file_path2.clone()).await.is_ok());

        // Deletes the directory.
        drop(dir);

        // The files are gone (even though they are still open).
        // TODO: This may cause trouble on Windows as Windows locks files when open.
        assert!(tokio::fs::metadata(file_path).await.is_err());
        assert!(tokio::fs::metadata(file_path2).await.is_err());

        // The directory is gone.
        assert!(tokio::fs::metadata(dir_path).await.is_err());
        Ok(())
    }
}
