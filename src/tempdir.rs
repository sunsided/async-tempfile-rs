#[cfg(not(feature = "uuid"))]
use crate::RandomName;
use crate::{AtomicOwnership, Error, Ownership, PersistError};
use std::borrow::Borrow;
use std::fmt::{Debug, Formatter};
use std::io::ErrorKind;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(feature = "uuid")]
use uuid::Uuid;

pub(crate) const DIR_PREFIX: &str = "atmpd_";

/// Maximum number of attempts to find a free name when creating a directory
/// with a randomly generated, collision-resistant name.
const MAX_NAME_ATTEMPTS: usize = 16;

/// How the underlying directory should be created.
#[derive(Copy, Clone, Eq, PartialEq)]
enum DirCreateMode {
    /// Create a brand-new directory, failing if it already exists. Used for
    /// unpredictable, auto-generated names so two temporaries never share a dir.
    Exclusive,
    /// Create the directory and any missing parents (idempotent if it exists).
    /// Used for user-supplied names, preserving historic behavior.
    CreateAll,
    /// Do not create; the directory is expected to already exist. Used by
    /// `from_existing`.
    OpenExisting,
}

/// A named temporary directory that will be cleaned automatically
/// after the last reference to it is dropped.
pub struct TempDir {
    /// A local copy of the directory path. Unlike [`crate::TempFile`]'s file
    /// handle there is no OS resource to release here, so field drop order does
    /// not affect deletion; the field is laid out before `core` purely to mirror
    /// the `TempFile` layout.
    dir: PathBuf,

    /// A shared pointer to the owned (or non-owned) directory.
    /// The `Arc` ensures that the enclosed dir is kept alive
    /// until all references to it are dropped.
    core: Arc<TempDirCore>,
}

/// The instance that tracks the temporary directory.
/// If dropped, the directory will be deleted.
struct TempDirCore {
    /// The path of the contained directory.
    path: PathBuf,

    /// Whether the directory specified in `path` is owned (and deleted on drop)
    /// or merely borrowed. Stored atomically because it is read from `Drop`,
    /// which must never block on a lock, and mutated by
    /// `keep`/`persist`/`drop_async`.
    ownership: AtomicOwnership,
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
    /// let dir = TempDir::new_with_name("new_with_name_example.dir").await?;
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
    /// The directory is created with a collision-resistant, unpredictable name
    /// using an exclusive create, so it never reuses an existing directory.
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
        Self::create_with_affixes(root_dir.borrow(), DIR_PREFIX, "").await
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
    /// let dir = TempDir::new_with_name_in("new_with_name_in_example.dir", path).await?;
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
        let root = root_dir.borrow();
        if !crate::path_is_dir(root).await {
            return Err(Error::InvalidDirectory);
        }
        let path = root.join(name.as_ref());
        Self::new_internal(path, Ownership::Owned, DirCreateMode::CreateAll).await
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
        let dir_name = format!("{DIR_PREFIX}{uuid}");
        Self::new_with_name_in(dir_name, root_dir).await
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
        if !crate::path_is_dir(&path).await {
            return Err(Error::InvalidDirectory);
        }
        Self::new_internal(path, ownership, DirCreateMode::OpenExisting).await
    }

    /// Creates a builder for configuring a new temporary directory
    /// (prefix, suffix, root) before creating it.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempDir, Error};
    /// # let _ = tokio_test::block_on(async {
    /// let dir = TempDir::builder()
    ///     .prefix("build_")
    ///     .create()
    ///     .await?;
    ///
    /// let name = dir.dir_path().file_name().unwrap().to_string_lossy().into_owned();
    /// assert!(name.starts_with("build_"));
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub fn builder() -> crate::TempDirBuilder {
        crate::TempDirBuilder::new()
    }

    /// Returns the path of the underlying temporary directory.
    pub fn dir_path(&self) -> &PathBuf {
        &self.core.path
    }

    /// Creates a new [`TempDir`] instance that shares the same underlying
    /// directory as the existing [`TempDir`] instance. The directory is removed
    /// once the last of the shared instances is dropped.
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
        self.core.ownership.get()
    }

    /// Disables automatic deletion and returns the path of the underlying
    /// directory, turning the temporary directory into a permanent one.
    ///
    /// This affects every clone that shares the same underlying directory: none
    /// of them will delete it when dropped.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempDir, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let dir = TempDir::new().await?;
    /// let path = dir.keep();
    ///
    /// // The directory still exists after the handle is dropped.
    /// assert!(fs::metadata(path.clone()).await.is_ok());
    /// # fs::remove_dir_all(path).await.ok();
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub fn keep(self) -> PathBuf {
        let path = self.core.path.clone();
        self.core.ownership.set_borrowed();
        path
    }

    /// Persists the temporary directory by moving it to `target`, returning the
    /// new path. The directory will no longer be deleted automatically.
    ///
    /// The move is performed with [`tokio::fs::rename`] and therefore must stay
    /// on the same filesystem (a cross-device move returns an error).
    ///
    /// On failure the temporary directory is **not** deleted: it is left at its
    /// original location and that path is returned in [`PersistError::path`], so
    /// no data is lost. The caller may re-wrap it with [`TempDir::from_existing`]
    /// to restore automatic cleanup, or delete it.
    ///
    /// ## Arguments
    ///
    /// * `target` - The destination path to move the directory to.
    pub async fn persist<P: AsRef<Path>>(self, target: P) -> Result<PathBuf, PersistError> {
        let target = target.as_ref().to_path_buf();
        match tokio::fs::rename(&self.core.path, &target).await {
            Ok(()) => {
                self.core.ownership.set_borrowed();
                Ok(target)
            }
            Err(e) => {
                // Preserve the caller's data: leave the directory in place and
                // report where it is, rather than deleting it on the way out.
                self.core.ownership.set_borrowed();
                Err(PersistError {
                    error: Error::Io(e),
                    path: self.core.path.clone(),
                })
            }
        }
    }

    /// Asynchronously drops the [`TempDir`] instance, removing the directory via
    /// [`tokio::fs::remove_dir_all`] without blocking the runtime when this is
    /// the last reference to an owned directory.
    ///
    /// The synchronous `Drop` remains armed as a backstop, so a cancelled or
    /// panicking `drop_async` still cleans up the directory.
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
    pub async fn drop_async(self) {
        let TempDir { dir, core } = self;
        drop(dir);

        let Some(core) = Arc::into_inner(core) else {
            return;
        };

        if core.ownership.is_owned() {
            // Still marked owned: a cancellation or panic at the await point
            // leaves `core`'s synchronous `Drop` to delete the directory.
            match tokio::fs::remove_dir_all(&core.path).await {
                Ok(()) => core.ownership.set_borrowed(),
                Err(e) if e.kind() == ErrorKind::NotFound => core.ownership.set_borrowed(),
                // Leave armed: the synchronous `Drop` below retries removal.
                Err(_) => {}
            }
        }

        drop(core);
    }

    /// Closes the directory, removing it (and its contents) when this is the
    /// last reference to an owned directory, and **returns the removal result**
    /// so the caller can observe a failure - unlike the implicit `Drop`, which
    /// has no way to report one. This is the synchronous sibling of
    /// [`drop_async`](Self::drop_async); prefer `drop_async` inside an async
    /// context to avoid blocking the runtime on the removal syscalls.
    ///
    /// If other clones still reference the directory, cleanup is left to them
    /// and `Ok(())` is returned. On error the directory is left in place and the
    /// synchronous `Drop` remains armed as a backstop, so it will retry the
    /// removal; the returned `Err` is purely informational.
    ///
    /// ## Example
    ///
    /// ```rust
    /// # use async_tempfile::{TempDir, Error};
    /// # let _ = tokio_test::block_on(async {
    /// let dir = TempDir::new().await?;
    /// let path = dir.dir_path().to_path_buf();
    ///
    /// dir.close()?; // Explicitly close, surfacing any removal error.
    ///
    /// assert!(!path.exists());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub fn close(self) -> std::io::Result<()> {
        let TempDir { dir, core } = self;
        drop(dir);

        // Only the sole owner removes the directory; otherwise the remaining
        // references' `Drop` impls handle cleanup.
        let Some(core) = Arc::into_inner(core) else {
            return Ok(());
        };

        if core.ownership.is_owned() {
            match std::fs::remove_dir_all(&core.path) {
                Ok(()) => core.ownership.set_borrowed(),
                Err(e) if e.kind() == ErrorKind::NotFound => core.ownership.set_borrowed(),
                // Leave armed: dropping `core` below lets `Drop` retry removal.
                // The error is still surfaced to the caller.
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    /// Creates a directory named `{prefix}{random}{suffix}` with an
    /// unpredictable, collision-resistant random core, using an exclusive create
    /// and retrying on the (very unlikely) collision. Shared by `new_in` and
    /// [`crate::TempDirBuilder`].
    pub(crate) async fn create_with_affixes(
        root: &Path,
        prefix: &str,
        suffix: &str,
    ) -> Result<Self, Error> {
        if !crate::path_is_dir(root).await {
            return Err(Error::InvalidDirectory);
        }
        // Affixes are name fragments, not paths: a separator would let the
        // composed name escape `root` (`../` traversal, or an absolute prefix
        // replacing it via `Path::join`).
        if !crate::affix_is_safe(prefix) || !crate::affix_is_safe(suffix) {
            return Err(Error::InvalidAffix);
        }
        let mut last_err = None;
        for _ in 0..MAX_NAME_ATTEMPTS {
            let name = format!("{prefix}{}{suffix}", Self::random_core_name());
            match Self::new_internal(root.join(name), Ownership::Owned, DirCreateMode::Exclusive)
                .await
            {
                Ok(dir) => return Ok(dir),
                Err(Error::Io(e)) if e.kind() == ErrorKind::AlreadyExists => {
                    last_err = Some(Error::Io(e));
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or(Error::InvalidDirectory))
    }

    /// Generates the unpredictable, collision-resistant random core of a name,
    /// without any prefix or suffix.
    fn random_core_name() -> String {
        #[cfg(feature = "uuid")]
        {
            Uuid::new_v4().to_string()
        }

        #[cfg(not(feature = "uuid"))]
        {
            RandomName::new("").as_str().to_string()
        }
    }

    async fn new_internal<P: Borrow<Path>>(
        path: P,
        ownership: Ownership,
        mode: DirCreateMode,
    ) -> Result<Self, Error> {
        let path = path.borrow();

        match mode {
            DirCreateMode::Exclusive => tokio::fs::create_dir(path).await?,
            DirCreateMode::CreateAll => tokio::fs::create_dir_all(path).await?,
            DirCreateMode::OpenExisting => {}
        }

        let core = TempDirCore {
            ownership: AtomicOwnership::new(ownership),
            path: PathBuf::from(path),
        };

        Ok(Self {
            dir: PathBuf::from(path),
            core: Arc::new(core),
        })
    }

    /// Gets the default temporary file directory.
    #[inline(always)]
    fn default_dir() -> PathBuf {
        std::env::temp_dir()
    }
}

/// Ensures that the underlying directory is deleted if this is an owned instance.
/// If the underlying directory is not owned, this operation does nothing.
impl Drop for TempDirCore {
    fn drop(&mut self) {
        // Ensure we don't drop borrowed directories. Read via the lock-free
        // atomic: `Drop` may run on a runtime worker thread and must never block.
        if !self.ownership.is_owned() {
            return;
        }

        // Synchronous on purpose: `Drop` must not re-enter the async runtime.
        // Using remove_dir_all to delete all content recursively. `drop_async`
        // provides an async deletion path at an explicit await point.
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
