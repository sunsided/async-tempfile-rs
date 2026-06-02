//! Behavior tests that also exercise the thinner-covered corners of the API:
//! error `Display`/`From`, the trait-forwarding impls (`Deref`/`Borrow`/`AsRef`,
//! async I/O), the named/uuid constructors, and the `close`/`drop_async`
//! `NotFound` and error paths (which double as checks of the disarm-on-error
//! contract).

use async_tempfile::{Error, Ownership, PersistError, TempDir, TempFile};
use std::borrow::{Borrow, BorrowMut};
use std::io::SeekFrom;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

fn unique(tag: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "covtest_{}_{}_{}",
        std::process::id(),
        tag,
        N.fetch_add(1, Ordering::Relaxed)
    )
}

// --- errors -----------------------------------------------------------------

#[test]
fn error_display_and_from() {
    assert!(!format!("{}", Error::InvalidDirectory).is_empty());
    assert!(!format!("{}", Error::InvalidFile).is_empty());
    assert!(!format!("{}", Error::InvalidAffix).is_empty());

    let io_err = std::io::Error::other("boom");
    let e: Error = io_err.into(); // From<io::Error>
    assert!(matches!(e, Error::Io(_)));
    assert!(format!("{e}").contains("boom"));
}

#[test]
fn persist_error_display_and_source() {
    use std::error::Error as _;
    let pe = PersistError {
        error: Error::Io(std::io::Error::other("x")),
        path: std::path::PathBuf::from("/tmp/leftover_xyz"),
    };
    assert!(format!("{pe}").contains("leftover_xyz"));
    assert!(pe.source().is_some());
}

// --- TempFile constructors / reopen / clone ---------------------------------

#[tokio::test]
async fn tempfile_named_constructors() {
    let f = TempFile::new_with_name(unique("file")).await.unwrap();
    assert!(f.file_path().is_file());

    let dir = TempDir::new().await.unwrap();
    let f2 = TempFile::new_with_name_in(unique("file"), dir.dir_path().as_path())
        .await
        .unwrap();
    assert_eq!(f2.file_path().parent().unwrap(), dir.dir_path());
}

#[tokio::test]
async fn tempfile_from_existing_missing_is_invalid_file() {
    let missing = std::env::temp_dir().join(unique("missing"));
    let _ = tokio::fs::remove_file(&missing).await;
    let err = TempFile::from_existing(missing, Ownership::Borrowed)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidFile));
}

#[tokio::test]
async fn tempfile_reopen_and_clone() {
    let f = TempFile::new().await.unwrap();
    assert_eq!(f.ownership(), Ownership::Owned);

    let rw = f.open_rw().await.unwrap();
    assert_eq!(rw.file_path(), f.file_path());

    let ro = f.open_ro().await.unwrap();
    assert_eq!(ro.file_path(), f.file_path());

    let cloned = f.try_clone().await.unwrap();
    assert_eq!(cloned.file_path(), f.file_path());
}

// --- TempFile trait forwarding ----------------------------------------------

#[tokio::test]
async fn tempfile_deref_borrow_asref_and_debug() {
    let mut f = TempFile::new().await.unwrap();

    assert!(!format!("{f:?}").is_empty()); // Debug -> TempFileCore Debug

    let _: &File = &f; // Deref coercion -> Deref::deref
    let _: &mut File = &mut f; // DerefMut coercion -> DerefMut::deref_mut
    let _: &File = Borrow::<File>::borrow(&f); // Borrow<File>
    let _: &mut File = BorrowMut::<File>::borrow_mut(&mut f); // BorrowMut<File>
    let _: &File = f.as_ref(); // AsRef<File>
}

#[tokio::test]
async fn tempfile_async_read_write_seek_shutdown() {
    let mut f = TempFile::new().await.unwrap();
    f.write_all(b"hello").await.unwrap(); // poll_write
    f.flush().await.unwrap(); // poll_flush
    f.seek(SeekFrom::Start(0)).await.unwrap(); // start_seek + poll_complete

    let mut s = String::new();
    f.read_to_string(&mut s).await.unwrap(); // poll_read
    assert_eq!(s, "hello");

    f.shutdown().await.unwrap(); // poll_shutdown
}

// --- TempFile close / drop_async edge paths ---------------------------------

#[tokio::test]
async fn tempfile_close_tolerates_already_removed() {
    let f = TempFile::new().await.unwrap();
    let path = f.file_path().clone();
    tokio::fs::remove_file(&path).await.unwrap();

    f.close()
        .expect("close must tolerate an already-removed file");
    assert!(!path.exists());
}

#[tokio::test]
async fn tempfile_drop_async_tolerates_already_removed() {
    let f = TempFile::new().await.unwrap();
    let path = f.file_path().clone();
    tokio::fs::remove_file(&path).await.unwrap();

    f.drop_async().await;
    assert!(!path.exists());
}

#[tokio::test]
async fn tempfile_close_error_disarms_and_leaves_leftover() {
    let f = TempFile::new().await.unwrap();
    let path = f.file_path().clone();
    // Swap the file for a directory at the same path: `remove_file` then fails
    // with a non-`NotFound` error, exercising the disarm-on-error arm.
    tokio::fs::remove_file(&path).await.unwrap();
    tokio::fs::create_dir(&path).await.unwrap();

    let err = f.close().expect_err("remove_file on a directory must fail");
    assert_ne!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(path.is_dir(), "close leaves the leftover in place on error");

    tokio::fs::remove_dir(&path).await.unwrap();
}

#[tokio::test]
async fn tempfile_drop_async_error_arm_leaves_drop_armed() {
    let f = TempFile::new().await.unwrap();
    let path = f.file_path().clone();
    tokio::fs::remove_file(&path).await.unwrap();
    tokio::fs::create_dir(&path).await.unwrap();

    f.drop_async().await; // remove_file on a dir errors -> "leave armed" arm
    assert!(
        path.is_dir(),
        "leftover remains (sync Drop retry also fails)"
    );

    tokio::fs::remove_dir(&path).await.unwrap();
}

// --- TempDir constructors / traits / builder suffix -------------------------

#[tokio::test]
async fn tempdir_named_constructors_and_traits() {
    let d = TempDir::new_with_name(unique("dir")).await.unwrap();
    assert!(d.dir_path().is_dir());

    let root = TempDir::new().await.unwrap();
    let d2 = TempDir::new_with_name_in(unique("dir"), root.dir_path().as_path())
        .await
        .unwrap();
    assert_eq!(d2.dir_path().parent().unwrap(), root.dir_path());

    assert!(!format!("{d:?}").is_empty()); // Debug
    let p: &std::path::Path = &d; // Deref to Path
    assert!(p.is_dir());

    let cloned = d.try_clone().await.unwrap();
    assert_eq!(cloned.dir_path(), d.dir_path());
    assert_eq!(d.ownership(), Ownership::Owned);
}

#[tokio::test]
async fn tempdir_from_existing_roundtrip_and_missing() {
    let d = TempDir::new().await.unwrap();
    let path = d.dir_path().clone();
    let borrowed = TempDir::from_existing(path.clone(), Ownership::Borrowed)
        .await
        .unwrap();
    assert_eq!(borrowed.dir_path(), &path);

    let missing = std::env::temp_dir().join(unique("missing_dir"));
    let _ = tokio::fs::remove_dir_all(&missing).await;
    let err = TempDir::from_existing(missing, Ownership::Borrowed)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidDirectory));
}

#[tokio::test]
async fn tempdir_builder_honors_suffix() {
    let d = TempDir::builder()
        .prefix("cov_")
        .suffix("_end")
        .create()
        .await
        .unwrap();
    let name = d
        .dir_path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("cov_"), "name was {name}");
    assert!(name.ends_with("_end"), "name was {name}");
}

// --- TempDir close / drop_async edge paths ----------------------------------

#[tokio::test]
async fn tempdir_close_tolerates_already_removed() {
    let d = TempDir::new().await.unwrap();
    let path = d.dir_path().clone();
    tokio::fs::remove_dir_all(&path).await.unwrap();

    d.close()
        .expect("close must tolerate an already-removed dir");
}

#[tokio::test]
async fn tempdir_close_error_disarms_and_leaves_leftover() {
    let d = TempDir::new().await.unwrap();
    let path = d.dir_path().clone();
    // Swap the dir for a regular file: `remove_dir_all` then fails with a
    // non-`NotFound` error, exercising the disarm-on-error arm.
    tokio::fs::remove_dir(&path).await.unwrap();
    tokio::fs::File::create(&path).await.unwrap();

    let err = d.close().expect_err("remove_dir_all on a file must fail");
    assert_ne!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(
        path.is_file(),
        "close leaves the leftover in place on error"
    );

    tokio::fs::remove_file(&path).await.unwrap();
}

#[tokio::test]
async fn tempdir_drop_async_error_arm_leaves_drop_armed() {
    let d = TempDir::new().await.unwrap();
    let path = d.dir_path().clone();
    tokio::fs::remove_dir(&path).await.unwrap();
    tokio::fs::File::create(&path).await.unwrap();

    d.drop_async().await; // remove_dir_all on a file errors -> "leave armed" arm
    assert!(
        path.is_file(),
        "leftover remains (sync Drop retry also fails)"
    );

    tokio::fs::remove_file(&path).await.unwrap();
}

// --- multi-reference deferral & keep / persist-failure ----------------------

#[tokio::test]
async fn tempfile_drop_async_with_clone_defers_cleanup() {
    let f = TempFile::new().await.unwrap();
    let clone = f.open_rw().await.unwrap();
    let path = f.file_path().clone();

    f.drop_async().await; // a clone is still alive -> early return, no delete
    assert!(path.is_file(), "file survives while a clone references it");

    drop(clone);
    assert!(!path.exists());
}

#[tokio::test]
async fn tempdir_keep_prevents_deletion() {
    let d = TempDir::new().await.unwrap();
    let path = d.keep();
    assert!(path.is_dir(), "kept dir survives the dropped handle");
    tokio::fs::remove_dir_all(path).await.unwrap();
}

#[tokio::test]
async fn tempdir_close_and_drop_async_with_clone_defer_cleanup() {
    // `close` with a live clone returns Ok and leaves cleanup to the clone.
    let d = TempDir::new().await.unwrap();
    let clone = d.try_clone().await.unwrap();
    let path = d.dir_path().clone();
    d.close().expect("close with a live clone returns Ok");
    assert!(path.is_dir(), "dir survives while a clone references it");
    drop(clone);
    assert!(!path.exists());

    // Same for `drop_async`.
    let d2 = TempDir::new().await.unwrap();
    let clone2 = d2.try_clone().await.unwrap();
    let path2 = d2.dir_path().clone();
    d2.drop_async().await;
    assert!(path2.is_dir());
    drop(clone2);
    assert!(!path2.exists());
}

#[tokio::test]
async fn tempdir_persist_failure_preserves_and_reports() {
    let d = TempDir::new().await.unwrap();
    let original = d.dir_path().clone();

    // Target under a non-existent parent: the rename fails, so persist must
    // leave the directory in place and report its path.
    let bad_target = std::env::temp_dir().join(unique("nope")).join("target");
    let err = d
        .persist(&bad_target)
        .await
        .expect_err("persist into a missing parent must fail");

    assert_eq!(err.path, original);
    assert!(
        original.is_dir(),
        "a failed persist leaves the dir in place"
    );

    tokio::fs::remove_dir_all(&original).await.ok();
}

// --- uuid feature constructors ----------------------------------------------

#[cfg(feature = "uuid")]
mod uuid_feature {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn tempfile_uuid_constructors() {
        let f = TempFile::new_with_uuid(Uuid::new_v4()).await.unwrap();
        assert!(f.file_path().is_file());

        let dir = TempDir::new().await.unwrap();
        let f2 = TempFile::new_with_uuid_in(Uuid::new_v4(), dir.dir_path().as_path())
            .await
            .unwrap();
        assert_eq!(f2.file_path().parent().unwrap(), dir.dir_path());
    }

    #[tokio::test]
    async fn tempdir_uuid_constructors() {
        let d = TempDir::new_with_uuid(Uuid::new_v4()).await.unwrap();
        assert!(d.dir_path().is_dir());

        let root = TempDir::new().await.unwrap();
        let d2 = TempDir::new_with_uuid_in(Uuid::new_v4(), root.dir_path().as_path())
            .await
            .unwrap();
        assert_eq!(d2.dir_path().parent().unwrap(), root.dir_path());
    }
}
