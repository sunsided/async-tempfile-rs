use std::borrow::{Borrow, BorrowMut};
use std::fmt::{Debug, Formatter};
use std::io::{ErrorKind, IoSlice, SeekFrom};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};

#[cfg(not(feature = "uuid"))]
use crate::random_name::RandomName;
use crate::{AtomicOwnership, Error, Ownership, PersistError};
#[cfg(feature = "uuid")]
use uuid::Uuid;

pub(crate) const FILE_PREFIX: &str = "atmp_";

/// Maximum number of attempts to find a free name when creating a file with a
/// randomly generated, collision-resistant name.
const MAX_NAME_ATTEMPTS: usize = 16;

/// How the underlying file should be opened or created.
#[derive(Copy, Clone, Eq, PartialEq)]
enum CreateMode {
    /// Create a brand-new file, failing if it already exists (`O_EXCL`).
    /// Used for unpredictable, auto-generated names to avoid clobbering an
    /// existing file or following a planted symlink.
    Exclusive,
    /// Create the file, or open it if it already exists. Used for user-supplied
    /// names, preserving historic behavior.
    CreateOrOpen,
    /// Open an existing file without creating it. Used by `from_existing`.
    OpenExisting,
}

/// A named temporary file that will be cleaned automatically
/// after the last reference to it is dropped.
pub struct TempFile {
    /// A local reference to the file. Used to write to or read from the file.
    ///
    /// Field order matters: `file` is declared before `core` so that this local
    /// handle is dropped (closed) before the shared `core` is released and the
    /// file is deleted. Required for correct deletion on Windows, which refuses
    /// to delete a file while a handle to it is open.
    file: File,

    /// A shared pointer to the owned (or non-owned) file.
    /// The `Arc` ensures that the enclosed file is kept alive
    /// until all references to it are dropped.
    core: Arc<TempFileCore>,
}

/// The instance that tracks the temporary file.
/// If dropped, the file will be deleted.
struct TempFileCore {
    /// The path of the contained file.
    path: PathBuf,

    /// Whether the file specified in `path` is owned (and deleted on drop) or
    /// merely borrowed. Stored atomically because it is read from `Drop`, which
    /// must never block on a lock, and mutated by `keep`/`persist`/`drop_async`.
    ownership: AtomicOwnership,
}

impl TempFile {
    /// Creates a new temporary file in the default location.
    /// When the instance goes out of scope, the file will be deleted.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let file = TempFile::new().await?;
    ///
    /// // The file exists.
    /// let file_path = file.file_path().clone();
    /// assert!(fs::metadata(file_path.clone()).await.is_ok());
    ///
    /// // Deletes the file.
    /// drop(file);
    ///
    /// // The file was removed.
    /// assert!(fs::metadata(file_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub async fn new() -> Result<Self, Error> {
        Self::new_in(Self::default_dir()).await
    }

    /// Creates a new temporary file in the default location.
    /// When the instance goes out of scope, the file will be deleted.
    ///
    /// ## Arguments
    ///
    /// * `name` - The name of the file to create in the default temporary directory.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let file = TempFile::new_with_name("new_with_name_example.file").await?;
    ///
    /// // The file exists.
    /// let file_path = file.file_path().clone();
    /// assert!(fs::metadata(file_path.clone()).await.is_ok());
    ///
    /// // Deletes the file.
    /// drop(file);
    ///
    /// // The file was removed.
    /// assert!(fs::metadata(file_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub async fn new_with_name<N: AsRef<str>>(name: N) -> Result<Self, Error> {
        Self::new_with_name_in(name, Self::default_dir()).await
    }

    /// Creates a new temporary file in the default location.
    /// When the instance goes out of scope, the file will be deleted.
    ///
    /// ## Arguments
    ///
    /// * `uuid` - A UUID to use as a suffix to the file name.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let id = uuid::Uuid::new_v4();
    /// let file = TempFile::new_with_uuid(id).await?;
    ///
    /// // The file exists.
    /// let file_path = file.file_path().clone();
    /// assert!(fs::metadata(file_path.clone()).await.is_ok());
    ///
    /// // Deletes the file.
    /// drop(file);
    ///
    /// // The file was removed.
    /// assert!(fs::metadata(file_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    #[cfg_attr(docsrs, doc(cfg(feature = "uuid")))]
    #[cfg(feature = "uuid")]
    pub async fn new_with_uuid(uuid: Uuid) -> Result<Self, Error> {
        Self::new_with_uuid_in(uuid, Self::default_dir()).await
    }

    /// Creates a new temporary file in the specified location.
    /// When the instance goes out of scope, the file will be deleted.
    ///
    /// The file is created with a collision-resistant, unpredictable name using
    /// an exclusive (`O_EXCL`) create, so it never clobbers an existing file.
    ///
    /// ## Crate Features
    ///
    /// * `uuid` - When the `uuid` crate feature is enabled, a random UUIDv4 is used to
    ///   generate the temporary file name.
    ///
    /// ## Arguments
    ///
    /// * `dir` - The directory to create the file in.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let path = std::env::temp_dir();
    /// let file = TempFile::new_in(path).await?;
    ///
    /// // The file exists.
    /// let file_path = file.file_path().clone();
    /// assert!(fs::metadata(file_path.clone()).await.is_ok());
    ///
    /// // Deletes the file.
    /// drop(file);
    ///
    /// // The file was removed.
    /// assert!(fs::metadata(file_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    pub async fn new_in<P: Borrow<Path>>(dir: P) -> Result<Self, Error> {
        Self::create_with_affixes(dir.borrow(), FILE_PREFIX, "").await
    }

    /// Creates a new temporary file in the specified location.
    /// When the instance goes out of scope, the file will be deleted.
    ///
    /// ## Arguments
    ///
    /// * `dir` - The directory to create the file in.
    /// * `name` - The file name to use.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let path = std::env::temp_dir();
    /// let file = TempFile::new_with_name_in("new_with_name_in_example.file", path).await?;
    ///
    /// // The file exists.
    /// let file_path = file.file_path().clone();
    /// assert!(fs::metadata(file_path.clone()).await.is_ok());
    ///
    /// // Deletes the file.
    /// drop(file);
    ///
    /// // The file was removed.
    /// assert!(fs::metadata(file_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub async fn new_with_name_in<N: AsRef<str>, P: Borrow<Path>>(
        name: N,
        dir: P,
    ) -> Result<Self, Error> {
        let dir = dir.borrow();
        if !crate::path_is_dir(dir).await {
            return Err(Error::InvalidDirectory);
        }
        let path = dir.join(name.as_ref());
        Self::new_internal(path, Ownership::Owned, CreateMode::CreateOrOpen).await
    }

    /// Creates a new temporary file in the specified location.
    /// When the instance goes out of scope, the file will be deleted.
    ///
    /// ## Arguments
    ///
    /// * `dir` - The directory to create the file in.
    /// * `uuid` - A UUID to use as a suffix to the file name.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let path = std::env::temp_dir();
    /// let id = uuid::Uuid::new_v4();
    /// let file = TempFile::new_with_uuid_in(id, path).await?;
    ///
    /// // The file exists.
    /// let file_path = file.file_path().clone();
    /// assert!(fs::metadata(file_path.clone()).await.is_ok());
    ///
    /// // Deletes the file.
    /// drop(file);
    ///
    /// // The file was removed.
    /// assert!(fs::metadata(file_path).await.is_err());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    #[cfg_attr(docsrs, doc(cfg(feature = "uuid")))]
    #[cfg(feature = "uuid")]
    pub async fn new_with_uuid_in<P: Borrow<Path>>(uuid: Uuid, dir: P) -> Result<Self, Error> {
        let file_name = format!("{FILE_PREFIX}{uuid}");
        Self::new_with_name_in(file_name, dir).await
    }

    /// Wraps a new instance of this type around an existing file.
    /// If `ownership` is set to [`Ownership::Borrowed`], this method does not take ownership of
    /// the file, i.e. the file will not be deleted when the instance is dropped.
    ///
    /// ## Arguments
    ///
    /// * `path` - The path of the file to wrap.
    /// * `ownership` - The ownership of the file.
    pub async fn from_existing<P: Borrow<Path>>(
        path: P,
        ownership: Ownership,
    ) -> Result<Self, Error> {
        if !crate::path_is_file(path.borrow()).await {
            return Err(Error::InvalidFile);
        }
        Self::new_internal(path, ownership, CreateMode::OpenExisting).await
    }

    /// Creates a builder for configuring a new temporary file
    /// (prefix, suffix, directory) before creating it.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # let _ = tokio_test::block_on(async {
    /// let file = TempFile::builder()
    ///     .prefix("log_")
    ///     .suffix(".txt")
    ///     .create()
    ///     .await?;
    ///
    /// let name = file.file_path().file_name().unwrap().to_string_lossy().into_owned();
    /// assert!(name.starts_with("log_"));
    /// assert!(name.ends_with(".txt"));
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub fn builder() -> crate::TempFileBuilder {
        crate::TempFileBuilder::new()
    }

    /// Returns the path of the underlying temporary file.
    pub fn file_path(&self) -> &PathBuf {
        &self.core.path
    }

    /// Opens a new TempFile instance in read-write mode.
    pub async fn open_rw(&self) -> Result<TempFile, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.core.path)
            .await?;
        Ok(TempFile {
            file,
            core: self.core.clone(),
        })
    }

    /// Opens a new TempFile instance in read-only mode.
    pub async fn open_ro(&self) -> Result<TempFile, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(false)
            .open(&self.core.path)
            .await?;
        Ok(TempFile {
            file,
            core: self.core.clone(),
        })
    }

    /// Creates a new TempFile instance that shares the same underlying
    /// file handle as the existing TempFile instance.
    /// Reads, writes, and seeks will affect both TempFile instances simultaneously.
    pub async fn try_clone(&self) -> Result<TempFile, Error> {
        Ok(TempFile {
            file: self.file.try_clone().await?,
            core: self.core.clone(),
        })
    }

    /// Determines the ownership of the temporary file.
    /// ### Example
    /// ```
    /// # use async_tempfile::{Ownership, TempFile};
    /// # let _ = tokio_test::block_on(async {
    /// let file = TempFile::new().await?;
    /// assert_eq!(file.ownership(), Ownership::Owned);
    /// # drop(file);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # });
    /// ```
    pub fn ownership(&self) -> Ownership {
        self.core.ownership.get()
    }

    /// Disables automatic deletion and returns the path of the underlying file,
    /// turning the temporary file into a permanent one.
    ///
    /// This affects every clone that shares the same underlying file: none of
    /// them will delete it when dropped.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let file = TempFile::new().await?;
    /// let path = file.keep();
    ///
    /// // The file still exists after the handle is dropped.
    /// assert!(fs::metadata(path.clone()).await.is_ok());
    /// # fs::remove_file(path).await.ok();
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub fn keep(self) -> PathBuf {
        let path = self.core.path.clone();
        self.core.ownership.set_borrowed();
        path
    }

    /// Persists the temporary file by moving it to `target`, returning the new
    /// path. The file will no longer be deleted automatically.
    ///
    /// The move is performed with [`tokio::fs::rename`] and therefore must stay
    /// on the same filesystem (a cross-device move returns an error).
    ///
    /// On failure the temporary file is **not** deleted: it is left at its
    /// original location and that path is returned in [`PersistError::path`], so
    /// no data is lost on a cross-device or permission error. The caller may
    /// re-wrap it with [`TempFile::from_existing`] to restore automatic cleanup,
    /// or delete it. The local handle is closed before the rename so the move
    /// also succeeds on Windows.
    ///
    /// ## Arguments
    ///
    /// * `target` - The destination path to move the file to.
    ///
    /// ## Example
    ///
    /// ```
    /// # use async_tempfile::{TempFile, Error};
    /// # use tokio::fs;
    /// # let _ = tokio_test::block_on(async {
    /// let file = TempFile::new().await?;
    /// let target = std::env::temp_dir().join("persisted_async_tempfile.txt");
    ///
    /// let path = file.persist(&target).await.map_err(|e| e.error)?;
    /// assert!(fs::metadata(path.clone()).await.is_ok());
    /// # fs::remove_file(path).await.ok();
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub async fn persist<P: AsRef<Path>>(self, target: P) -> Result<PathBuf, PersistError> {
        let target = target.as_ref().to_path_buf();
        let TempFile { file, core } = self;
        // Close our handle before the rename: Windows refuses to move a file
        // while a handle to it is open.
        drop(file);

        match tokio::fs::rename(&core.path, &target).await {
            Ok(()) => {
                core.ownership.set_borrowed();
                Ok(target)
            }
            Err(e) => {
                // Preserve the caller's data: leave the temporary in place and
                // report where it is, rather than deleting it on the way out.
                core.ownership.set_borrowed();
                Err(PersistError {
                    error: Error::Io(e),
                    path: core.path.clone(),
                })
            }
        }
    }

    /// Asynchronously drops the TempFile, ensuring any resources are properly released.
    /// This is useful for explicitly managing the lifecycle of the TempFile
    /// in an asynchronous context.
    ///
    /// When this is the last reference to an owned file, the file is removed via
    /// [`tokio::fs::remove_file`] without blocking the runtime. The synchronous
    /// `Drop` remains armed as a backstop, so a cancelled or panicking
    /// `drop_async` still cleans up the file.
    ///
    /// ## Example
    ///
    /// ```rust
    /// # use async_tempfile::{TempFile, Error};
    /// # let _ = tokio_test::block_on(async {
    /// let file = TempFile::new().await?;
    /// let path = file.file_path().to_path_buf();
    /// assert!(path.is_file());
    ///
    /// file.drop_async().await; // Explicitly drop the TempFile
    ///
    /// assert!(!path.exists());
    /// # Ok::<(), Error>(())
    /// # });
    /// ```
    pub async fn drop_async(self) {
        let TempFile { file, core } = self;
        // Close the local read-write handle before attempting deletion.
        drop(file);

        // Only the sole owner removes the file asynchronously; otherwise the
        // remaining references' `Drop` impls handle cleanup.
        let Some(core) = Arc::into_inner(core) else {
            return;
        };

        if core.ownership.is_owned() {
            // `core` is still marked owned here. If this future is cancelled or
            // panics at the await point, `core` is dropped and its synchronous
            // `Drop` deletes the file. We disarm only after a confirmed removal.
            match tokio::fs::remove_file(&core.path).await {
                Ok(()) => core.ownership.set_borrowed(),
                Err(e) if e.kind() == ErrorKind::NotFound => core.ownership.set_borrowed(),
                // Leave armed: the synchronous `Drop` below retries the removal.
                Err(_) => {}
            }
        }

        drop(core);
    }

    /// Creates a file named `{prefix}{random}{suffix}` with an unpredictable,
    /// collision-resistant random core, using an exclusive (`O_EXCL`) create and
    /// retrying on the (astronomically unlikely) collision. Shared by `new_in`
    /// and [`crate::TempFileBuilder`].
    pub(crate) async fn create_with_affixes(
        dir: &Path,
        prefix: &str,
        suffix: &str,
    ) -> Result<Self, Error> {
        if !crate::path_is_dir(dir).await {
            return Err(Error::InvalidDirectory);
        }
        let mut last_err = None;
        for _ in 0..MAX_NAME_ATTEMPTS {
            let name = format!("{prefix}{}{suffix}", Self::random_core_name());
            match Self::new_internal(dir.join(name), Ownership::Owned, CreateMode::Exclusive).await
            {
                Ok(file) => return Ok(file),
                Err(Error::Io(e)) if e.kind() == ErrorKind::AlreadyExists => {
                    last_err = Some(Error::Io(e));
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or(Error::InvalidFile))
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
        mode: CreateMode,
    ) -> Result<Self, Error> {
        let path = path.borrow();

        // A single read-write handle both creates (per `mode`) and serves the
        // file. Keeping just this one handle alive holds the inode open for the
        // lifetime of the (shared) core, so no separate keep-alive handle is
        // needed - that only wasted a file descriptor per file.
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        match mode {
            CreateMode::Exclusive => {
                options.create_new(true);
            }
            CreateMode::CreateOrOpen => {
                options.create(true);
            }
            CreateMode::OpenExisting => {}
        }

        let file = options.open(path).await?;
        let core = TempFileCore {
            ownership: AtomicOwnership::new(ownership),
            path: PathBuf::from(path),
        };

        Ok(Self {
            file,
            core: Arc::new(core),
        })
    }

    /// Gets the default temporary file directory.
    #[inline(always)]
    fn default_dir() -> PathBuf {
        std::env::temp_dir()
    }
}

/// Ensures that the underlying file is deleted if this is an owned instance.
/// If the underlying file is not owned, this operation does nothing.
impl Drop for TempFileCore {
    fn drop(&mut self) {
        // Ensure we don't drop borrowed files. Read via the lock-free atomic:
        // `Drop` may run on a runtime worker thread and must never block.
        if !self.ownership.is_owned() {
            return;
        }

        // The owning `TempFile`'s handle (declared before `core`) has already
        // been closed by the time this runs, so the file can be deleted even on
        // platforms that lock open files (Windows).
        //
        // Synchronous on purpose: `Drop` must not re-enter the async runtime, as
        // it may run on a runtime worker thread. Use `drop_async` for an async
        // deletion path at an explicit await point.
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Debug for TempFileCore {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.path)
    }
}

impl Debug for TempFile {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.core)
    }
}

/// Allows implicit treatment of TempFile as a File.
impl Deref for TempFile {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

/// Allows implicit treatment of TempFile as a mutable File.
impl DerefMut for TempFile {
    fn deref_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

impl Borrow<File> for TempFile {
    fn borrow(&self) -> &File {
        &self.file
    }
}

impl BorrowMut<File> for TempFile {
    fn borrow_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

impl AsRef<File> for TempFile {
    fn as_ref(&self) -> &File {
        &self.file
    }
}

/// Forwarding AsyncWrite to the embedded File
impl AsyncWrite for TempFile {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.file).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.file).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.file).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.file).poll_write_vectored(cx, bufs)
    }
}

/// Forwarding AsyncRead to the embedded File
impl AsyncRead for TempFile {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.file).poll_read(cx, buf)
    }
}

/// Forwarding AsyncSeek to the embedded File
impl AsyncSeek for TempFile {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> std::io::Result<()> {
        Pin::new(&mut self.file).start_seek(position)
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        Pin::new(&mut self.file).poll_complete(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random_name::RandomName;

    #[test]
    fn test_random_name() {
        let name = RandomName::new(FILE_PREFIX);
        assert!(name.as_ref().starts_with(FILE_PREFIX))
    }
}
