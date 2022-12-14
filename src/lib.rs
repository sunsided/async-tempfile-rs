//! # async-tempfile
//!
//! Provides the TempFile struct, an asynchronous wrapper based on `tokio::fs` for temporary
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

mod errors;

pub use errors::Error;
use std::borrow::{Borrow, BorrowMut};
use std::fmt::{Debug, Formatter};
use std::io::{IoSlice, SeekFrom};
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};
use uuid::Uuid;

const FILE_PREFIX: &'static str = "atmp_";

/// A named temporary file that will be cleaned automatically
/// after the last reference to it is dropped.
pub struct TempFile {
    /// A local reference to the file. Used to write to or read from the file.
    file: ManuallyDrop<File>,

    /// A shared pointer to the owned (or non-owned) file.
    /// The `Arc` ensures that the enclosed file is kept alive
    /// until all references to it are dropped.
    core: ManuallyDrop<Arc<Box<TempFileCore>>>,
}

/// Determines the ownership of a temporary file.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum Ownership {
    /// The file is owned by [`TempFile`] and will be deleted when
    /// the last reference to it is dropped.
    Owned,
    /// The file is borrowed by [`TempFile`] and will be left untouched
    /// when the last reference to it is dropped.
    Borrowed,
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
        Self::new_in(std::env::temp_dir()).await
    }

    /// Creates a new temporary file in the specified location.
    /// When the instance goes out of scope, the file will be deleted.
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
    /// let file = TempFile::new_in(std::env::temp_dir()).await?;
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
    pub async fn new_in<P: Borrow<PathBuf>>(dir: P) -> Result<Self, Error> {
        let dir = dir.borrow();
        if !dir.is_dir() {
            return Err(Error::InvalidDirectory);
        }
        let file_name = format!("{}{}", FILE_PREFIX, Uuid::new_v4());
        let mut path = dir.clone();
        path.push(file_name);
        Ok(Self::new_internal(path, Ownership::Owned).await?)
    }

    /// Wraps a new instance of this type around an existing file.
    /// If `ownership` is set to [`Ownership::Borrowed`], this method does not take ownership of
    /// the file, i.e. the file will not be deleted when the instance is dropped.
    ///
    /// ## Arguments
    ///
    /// * `path` - The path of the file to wrap.
    /// * `ownership` - The ownership of the file.
    pub async fn from_existing(path: PathBuf, ownership: Ownership) -> Result<Self, Error> {
        if !path.is_file() {
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

    async fn new_internal(path: PathBuf, ownership: Ownership) -> Result<Self, Error> {
        let core = TempFileCore {
            file: ManuallyDrop::new(
                OpenOptions::new()
                    .create(ownership == Ownership::Owned)
                    .read(false)
                    .write(true)
                    .open(path.clone())
                    .await?,
            ),
            ownership,
            path: path.clone(),
        };

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.clone())
            .await?;
        Ok(Self {
            file: ManuallyDrop::new(file),
            core: ManuallyDrop::new(Arc::new(Box::new(core))),
        })
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

/// Ensures that the underlying file is deleted if this is a owned instance.
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
