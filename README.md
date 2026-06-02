# async-tempfile

[![Crates.io](https://img.shields.io/crates/v/async-tempfile)](https://crates.io/crates/async-tempfile)
[![Crates.io](https://img.shields.io/crates/l/async-tempfile)](https://crates.io/crates/async-tempfile)
[![Build](https://img.shields.io/github/actions/workflow/status/sunsided/async-tempfile-rs/rust.yml?branch=main)](https://github.com/sunsided/async-tempfile-rs/actions/workflows/rust.yml)
[![docs.rs](https://img.shields.io/docsrs/async-tempfile)](https://docs.rs/async-tempfile/)
[![codecov](https://codecov.io/gh/sunsided/async-tempfile-rs/graph/badge.svg?token=LSY85I6M8Y)](https://codecov.io/gh/sunsided/async-tempfile-rs)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/rust-secure-code/safety-dance/)

Provides the `TempFile` struct, an asynchronous wrapper based on `tokio::fs`
for temporary files that will be automatically deleted when the last reference to
the struct is dropped.

```rust
use async_tempfile::TempFile;

#[tokio::main]
async fn main() {
    let parent = TempFile::new().await.unwrap();

    // The cloned reference will not delete the file when dropped.
    {
        let nested = parent.open_rw().await.unwrap();
        assert_eq!(nested.file_path(), parent.file_path());
        assert!(nested.file_path().is_file());
    }

    // The file still exists; it will be deleted when `parent` is dropped.
    assert!(parent.file_path().is_file());
}
```

## Builder, keep and persist

```rust
use async_tempfile::TempFile;

#[tokio::main]
async fn main() {
    // Configure a name via prefix/suffix; the file is created with an
    // exclusive (`O_EXCL`), unpredictable, collision-resistant name.
    let file = TempFile::builder()
        .prefix("session_")
        .suffix(".log")
        .create()
        .await
        .unwrap();

    // Turn the temporary file into a permanent one, or move it elsewhere:
    // let path = file.keep();                       // disables auto-deletion
    // let path = file.persist("/data/out.log").await.unwrap(); // moves it

    // Drop it explicitly on an async runtime without blocking the executor.
    file.drop_async().await;

    // `drop_async` consumes `file`, so the following is an *alternative* to the
    // line above (not an addition): close synchronously and observe any error.
    //   file.close().unwrap();
}
```
