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
//! * `uuid` - Enables UUID-based random file name generation via the [`uuid`](https://crates.io/crates/uuid) crate
//!   and the `new_with_uuid*` group of methods. Not enabled by default; `new`/`new_in` work without it.

// Document crate features on docs.rs.
#![cfg_attr(docsrs, feature(doc_cfg))]
// This crate deletes files purely from safe code; no `unsafe` is permitted.
#![forbid(unsafe_code)]

mod builder;
mod errors;
mod random_name;
mod tempdir;
mod tempfile;

pub use builder::{TempDirBuilder, TempFileBuilder};
pub use errors::Error;
#[cfg(not(feature = "uuid"))]
pub(crate) use random_name::RandomName;
pub use tempdir::TempDir;
pub use tempfile::TempFile;

use std::sync::atomic::{AtomicBool, Ordering};

/// Determines the ownership of a temporary file or directory.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum Ownership {
    /// The file or directory is owned by [`TempFile`] and will be deleted when
    /// the last reference to it is dropped.
    Owned,
    /// The file or directory is borrowed by [`TempFile`] and will be left untouched
    /// when the last reference to it is dropped.
    Borrowed,
}

/// Lock-free, interior-mutable storage of an [`Ownership`] value, shared across
/// all clones via the `Arc`-wrapped core.
///
/// Lock-free is a hard requirement, not an optimization: the value is read from
/// `Drop`, which may run on a runtime worker thread and therefore must never
/// block on a lock. `keep`, `persist`, and `drop_async` flip it to `Borrowed`
/// to disable automatic deletion.
pub(crate) struct AtomicOwnership(AtomicBool);

impl AtomicOwnership {
    /// Creates a new value from the given ownership (`Owned` == `true`).
    pub(crate) fn new(ownership: Ownership) -> Self {
        Self(AtomicBool::new(matches!(ownership, Ownership::Owned)))
    }

    /// Returns the current ownership.
    pub(crate) fn get(&self) -> Ownership {
        if self.is_owned() {
            Ownership::Owned
        } else {
            Ownership::Borrowed
        }
    }

    /// Returns `true` if the value is currently `Owned`.
    pub(crate) fn is_owned(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Flips the value to `Borrowed`, disabling automatic deletion. Idempotent.
    pub(crate) fn set_borrowed(&self) {
        self.0.store(false, Ordering::Release);
    }
}
