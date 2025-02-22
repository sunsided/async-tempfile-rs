//! # async-tempfile
//!
//! Provides the [`TempFile`] struct, an asynchronous wrapper based on `tokio::fs` for temporary
//! files that will be automatically deleted when the last reference to the struct is dropped.
//!
//! ```
//! use async_tempfile::TempFile;
//!
//! #[tokio::main]
//! async fn main() {
//!     let parent = TempFile::new().await.unwrap();
//!
//!     // The cloned reference will not delete the file when dropped.
//!     {
//!         let nested = parent.open_rw().await.unwrap();
//!         assert_eq!(nested.file_path(), parent.file_path());
//!         assert!(nested.file_path().is_file());
//!     }
//!
//!     // The file still exists; it will be deleted when `parent` is dropped.
//!     assert!(parent.file_path().is_file());
//! }
//! ```
//!
//! ## Features
//!
//! * `uuid` - (Default) Enables random file name generation based on the [`uuid`](https://crates.io/crates/uuid) crate.
//!            Provides the `new` and `new_in`, as well as the `new_with_uuid*` group of methods.

// Document crate features on docs.rs.
#![cfg_attr(docsrs, feature(doc_cfg))]
// Required for dropping the file.
use std::borrow::{Borrow, BorrowMut};
use std::fmt::{Debug, Formatter};
use std::io::{IoSlice, SeekFrom};
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};

#[cfg(not(feature = "uuid"))]
use crate::random_name::RandomName;
use crate::Error;
use crate::Ownership;
#[cfg(feature = "uuid")]
use uuid::Uuid;

const FILE_PREFIX: &str = "atmp_";

/// A named temporary file that will be cleaned automatically
/// after the last reference to it is dropped.
pub struct TempFile {
    /// A local reference to the file. Used to write to or read from the file.
    file: ManuallyDrop<File>,

    /// A shared pointer to the owned (or non-owned) file.
    /// The `Arc` ensures that the enclosed file is kept alive
    /// until all references to it are dropped.
    core: ManuallyDrop<Arc<TempFileCore>>,
}

/// The instance that tracks the temporary file.
/// If dropped, the file will be deleted.
struct TempFileCore {
    /// The path of the contained file.
    path: PathBuf,

    /// Pointer to the file to keep it alive.
    file: ManuallyDrop<File>,

    /// A hacky approach to allow for "non-owned" files.
    /// If set to `Ownership::Owned`, the file specified in `path` will be deleted
    /// when this instance is dropped. If set to `Ownership::Borrowed`, the file will be kept.
    ownership: Ownership,
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
    /// let file = TempFile::new_with_name("temporary.file").await?;
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
        #[cfg(feature = "uuid")]
        {
            let id = Uuid::new_v4();
            Self::new_with_uuid_in(id, dir).await
        }

        #[cfg(not(feature = "uuid"))]
        {
            let name = RandomName::new(FILE_PREFIX);
            Self::new_with_name_in(name, dir).await
        }
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
    /// let file = TempFile::new_with_name_in("temporary.file", path).await?;
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
        if !dir.is_dir() {
            return Err(Error::InvalidDirectory);
        }
        let file_name = name.as_ref();
        let mut path = PathBuf::from(dir);
        path.push(file_name);
        Self::new_internal(path, Ownership::Owned).await
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
        let file_name = format!("{}{}", FILE_PREFIX, uuid);
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
        if !path.borrow().is_file() {
            return Err(Error::InvalidFile);
        }
        Self::new_internal(path, ownership).await
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
            core: self.core.clone(),
            file: ManuallyDrop::new(file),
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
            core: self.core.clone(),
            file: ManuallyDrop::new(file),
        })
    }

    /// Creates a new TempFile instance that shares the same underlying
    /// file handle as the existing TempFile instance.
    /// Reads, writes, and seeks will affect both TempFile instances simultaneously.
    #[allow(dead_code)]
    pub async fn try_clone(&self) -> Result<TempFile, Error> {
        Ok(TempFile {
            core: self.core.clone(),
            file: ManuallyDrop::new(self.file.try_clone().await?),
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
        self.core.ownership
    }

    /// Asynchronously drops the TempFile, ensuring any resources are properly released.
    /// This is useful for explicitly managing the lifecycle of the TempFile
    /// in an asynchronous context.
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
        tokio::task::spawn_blocking(move || drop(self)).await.ok();
    }
    
    async fn new_internal<P: Borrow<Path>>(path: P, ownership: Ownership) -> Result<Self, Error> {
        let path = path.borrow();

        let core = TempFileCore {
            file: ManuallyDrop::new(
                OpenOptions::new()
                    .create(ownership == Ownership::Owned)
                    .read(false)
                    .write(true)
                    .open(path)
                    .await?,
            ),
            ownership,
            path: PathBuf::from(path),
        };

        let file = OpenOptions::new().read(true).write(true).open(path).await?;
        Ok(Self {
            file: ManuallyDrop::new(file),
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
impl Drop for TempFile {
    fn drop(&mut self) {
        // Ensure all file handles are closed before we attempt to delete the file itself via core.
        drop(unsafe { ManuallyDrop::take(&mut self.file) });
        drop(unsafe { ManuallyDrop::take(&mut self.core) });
    }
}

/// Ensures that the underlying file is deleted if this is an owned instance.
/// If the underlying file is not owned, this operation does nothing.
impl Drop for TempFileCore {
    fn drop(&mut self) {
        // Ensure we don't drop borrowed files.
        if self.ownership != Ownership::Owned {
            return;
        }

        // Closing the file handle first, as otherwise the file might not be deleted.
        drop(unsafe { ManuallyDrop::take(&mut self.file) });

        // TODO: Use asynchronous variant if running in an async context.
        // Note that if TempFile is used from the executor's handle,
        //      this may block the executor itself.
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
        Pin::new(self.file.deref_mut()).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(self.file.deref_mut()).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(self.file.deref_mut()).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(self.file.deref_mut()).poll_write_vectored(cx, bufs)
    }
}

/// Forwarding AsyncWrite to the embedded TempFile
impl AsyncRead for TempFile {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(self.file.deref_mut()).poll_read(cx, buf)
    }
}

/// Forwarding AsyncSeek to the embedded File
impl AsyncSeek for TempFile {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> std::io::Result<()> {
        Pin::new(self.file.deref_mut()).start_seek(position)
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        Pin::new(self.file.deref_mut()).poll_complete(cx)
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
