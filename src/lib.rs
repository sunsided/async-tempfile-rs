use std::borrow::{Borrow, BorrowMut};
use std::fmt::{Debug, Formatter};
use std::io::{Error, IoSlice, SeekFrom};
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf};
use uuid::Uuid;

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

/// The instance that tracks the temporary file.
/// If dropped, the file will be deleted.
struct TempFileCore {
    /// The path of the contained file.
    path: PathBuf,

    /// A hacky approach to allow for "non-owned" files.
    /// If set to `Some(File)`, the file specified in `path` will be deleted
    /// when this instance is dropped. If set to `None`, the file will be kept.
    file: Option<File>,
}

impl TempFile {
    /// Creates a new temporary file in the default location.
    /// When the instance goes out of scope, the file will be deleted.
    ///
    /// ## Example
    ///
    /// ```
    /// let file = tempfile::TempFile::new();
    ///
    /// // Deletes the file.
    /// drop(file);
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
    /// let file = tempfile::TempFile::new_in(std::env::temp_dir());
    ///
    /// // Deletes the file.
    /// drop(file);
    /// ```
    pub async fn new_in<P: Borrow<PathBuf>>(dir: P) -> Result<Self, Error> {
        let dir = dir.borrow();
        assert!(dir.is_dir()); // TODO: Return error instead
        let file_name = format!("img_pp_{}", Uuid::new_v4());
        let mut path = dir.clone();
        path.push(file_name);
        Self::new_internal(path, true).await
    }

    /// Wraps a new instance of this type around an existing file.
    /// This method does not take ownership of the file, i.e. the file will not
    /// be deleted when the instance is dropped.
    ///
    /// ## Arguments
    ///
    /// * `path` - The path of the file to wrap.
    pub async fn from_existing(path: PathBuf) -> Result<Self, Error> {
        debug_assert!(path.is_file()); // TODO: Return error instead
        Self::new_internal(path, false).await
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

    async fn new_internal(path: PathBuf, owned: bool) -> Result<Self, Error> {
        let core = TempFileCore {
            file: if owned {
                Some(
                    OpenOptions::new()
                        .create_new(true)
                        .read(false)
                        .write(true)
                        .open(path.clone())
                        .await?,
                )
            } else {
                // Ensure we won't drop non-owned files.
                None
            },
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
        // Closing the file handle first, as otherwise the file might not be deleted.
        // Ensure we don't drop borrowed files.
        if let Some(file) = self.file.take() {
            drop(file);

            // TODO: Use asynchronous variant if running in an async context.
            // Note that if TempFile is used from the executor's handle,
            //      this may block the executor itself.
            let _ = std::fs::remove_file(&self.path);
        }
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
    ) -> Poll<Result<usize, Error>> {
        Pin::new(self.file.deref_mut()).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(self.file.deref_mut()).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(self.file.deref_mut()).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize, Error>> {
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
mod test {
    use super::*;

    #[tokio::test]
    async fn file_is_deleted_when_dropping() {
        let path = {
            let file = TempFile::new().await.unwrap();
            assert!(file.file_path().is_file());
            file.file_path().clone()
        };

        // File is now deleted.
        assert!(!path.is_file());
    }

    #[tokio::test]
    async fn file_is_not_dropped_while_still_referenced() {
        let parent = TempFile::new().await.unwrap();

        {
            let nested = parent.open_rw().await.unwrap();
            assert_eq!(nested.file_path(), parent.file_path());
            assert!(nested.file_path().is_file());
        }

        assert!(parent.file_path().is_file());
    }
}
