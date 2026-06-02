//! Adversarial `prefix` / `suffix` handling.
//!
//! Affixes are composed into a single path component
//! (`{prefix}{random}{suffix}`). These tests pin down the behavior for hostile
//! affixes:
//!
//! * Path separators / traversal must be **rejected** so a name can never escape
//!   the target directory (`../` traversal, or an absolute prefix that would
//!   otherwise replace the target via `Path::join`).
//! * Names the OS itself rejects (an embedded NUL, an over-long component) must
//!   surface as a clean `Err`, never a panic or an infinite retry loop.
//! * Ordinary affixes keep working and stay inside the target directory.

use async_tempfile::{TempDir, TempFile};

// --- path separators / traversal are rejected -------------------------------

#[tokio::test]
async fn file_prefix_with_path_separator_is_rejected() {
    let dir = TempDir::new().await.unwrap();
    let result = TempFile::builder()
        .prefix("sub/evil_")
        .dir(dir.dir_path().clone())
        .create()
        .await;
    assert!(result.is_err(), "separator in prefix must be rejected");
}

#[tokio::test]
async fn file_suffix_with_path_separator_is_rejected() {
    let dir = TempDir::new().await.unwrap();
    let result = TempFile::builder()
        .suffix("/etc/passwd")
        .dir(dir.dir_path().clone())
        .create()
        .await;
    assert!(result.is_err(), "separator in suffix must be rejected");
}

#[tokio::test]
async fn file_traversal_prefix_cannot_escape_target_dir() {
    // A nested target dir so an escape would be observable: `../` would land the
    // file in the parent (`root`) instead of `target`.
    let root = TempDir::new().await.unwrap();
    let target = TempDir::new_in(root.dir_path().as_path()).await.unwrap();

    let result = TempFile::builder()
        .prefix("../")
        .dir(target.dir_path().clone())
        .create()
        .await;

    assert!(result.is_err(), "`../` traversal must be rejected");

    // Nothing leaked one level up into `root`.
    let mut entries = tokio::fs::read_dir(root.dir_path()).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        assert_eq!(
            entry.path(),
            *target.dir_path(),
            "unexpected entry escaped into the parent dir: {:?}",
            entry.path()
        );
    }
}

#[tokio::test]
async fn file_absolute_prefix_is_rejected() {
    let dir = TempDir::new().await.unwrap();
    let result = TempFile::builder()
        .prefix("/tmp/abs_")
        .dir(dir.dir_path().clone())
        .create()
        .await;
    assert!(
        result.is_err(),
        "absolute (separator-bearing) prefix must be rejected"
    );
}

#[tokio::test]
async fn dir_prefix_with_path_separator_is_rejected() {
    let root = TempDir::new().await.unwrap();
    let result = TempDir::builder()
        .prefix("../escaped_")
        .dir(root.dir_path().clone())
        .create()
        .await;
    assert!(result.is_err(), "separator in dir prefix must be rejected");
}

// --- OS-rejected names surface as a clean `Err`, never a panic --------------

#[tokio::test]
async fn file_nul_byte_in_suffix_errors_without_panic() {
    let dir = TempDir::new().await.unwrap();
    let result = TempFile::builder()
        .suffix("\0bad")
        .dir(dir.dir_path().clone())
        .create()
        .await;
    assert!(result.is_err(), "embedded NUL must error, not panic");
}

#[tokio::test]
async fn file_overlong_prefix_errors_without_panic() {
    let dir = TempDir::new().await.unwrap();
    let result = TempFile::builder()
        .prefix("a".repeat(5000))
        .dir(dir.dir_path().clone())
        .create()
        .await;
    assert!(result.is_err(), "over-long name must error, not panic");
}

// --- ordinary affixes keep working and stay contained -----------------------

#[tokio::test]
async fn ordinary_affixes_still_work_and_stay_contained() {
    let dir = TempDir::new().await.unwrap();
    let file = TempFile::builder()
        .prefix("ok_")
        .suffix(".log")
        .dir(dir.dir_path().clone())
        .create()
        .await
        .unwrap();

    let name = file
        .file_path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("ok_"), "name was {name}");
    assert!(name.ends_with(".log"), "name was {name}");

    // The file lives directly inside the requested directory.
    assert_eq!(file.file_path().parent().unwrap(), dir.dir_path());
}
