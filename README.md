# async-tempfile

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
