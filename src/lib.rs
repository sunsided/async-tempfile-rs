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
#![allow(unsafe_code)]

mod errors;
mod random_name;
mod tempdir;
mod tempfile;

pub use errors::Error;
pub(crate) use random_name::RandomName;
use std::fmt::Debug;
pub use tempdir::TempDir;
pub use tempfile::TempFile;

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
