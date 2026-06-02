//! Integration tests showcasing the `TempFile` / `TempDir` builders.

use async_tempfile::{TempDir, TempFile};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// The builder, used end-to-end: configure a name, then write and read back data.
#[tokio::test]
async fn build_write_and_read_back() {
    let mut file = TempFile::builder()
        .prefix("report_")
        .suffix(".txt")
        .create()
        .await
        .unwrap();

    // The configured prefix and suffix surround the random core.
    let name = file
        .file_path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("report_"), "name was {name}");
    assert!(name.ends_with(".txt"), "name was {name}");

    // The handle is a fully usable async file.
    file.write_all(b"hello builder").await.unwrap();
    file.flush().await.unwrap();
    file.rewind().await.unwrap();

    let mut contents = String::new();
    file.read_to_string(&mut contents).await.unwrap();
    assert_eq!(contents, "hello builder");
}

/// With no prefix/suffix configured, the default `atmp_` prefix is used.
#[tokio::test]
async fn build_with_defaults_uses_default_prefix() {
    let file = TempFile::builder().create().await.unwrap();
    let name = file
        .file_path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("atmp_"), "name was {name}");
}

/// The builder honors an explicit target directory.
#[tokio::test]
async fn build_in_explicit_directory() {
    let parent = TempDir::new().await.unwrap();

    let file = TempFile::builder()
        .prefix("scoped_")
        .dir(parent.dir_path().clone())
        .create()
        .await
        .unwrap();

    assert_eq!(file.file_path().parent().unwrap(), parent.dir_path());
    assert!(file.file_path().is_file());
}

/// Two builds never collide, even with identical prefix/suffix.
#[tokio::test]
async fn builds_have_unique_names() {
    let a = TempFile::builder().prefix("dup_").create().await.unwrap();
    let b = TempFile::builder().prefix("dup_").create().await.unwrap();
    assert_ne!(a.file_path(), b.file_path());
}

/// The directory builder creates a usable temporary directory.
#[tokio::test]
async fn build_directory_and_place_a_file_in_it() {
    let dir = TempDir::builder().prefix("ws_").create().await.unwrap();
    let dir_path = dir.dir_path().clone();

    let name = dir_path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("ws_"), "name was {name}");

    // A file created inside the built directory lives under it.
    let file = TempFile::new_in(dir_path.as_path()).await.unwrap();
    assert_eq!(file.file_path().parent().unwrap(), dir_path);

    // Dropping the directory removes it (and its contents) recursively.
    drop(file);
    drop(dir);
    assert!(!dir_path.exists());
}
