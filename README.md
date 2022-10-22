# async-tempfile

Provides the `TempFile` struct, an asynchronous wrapper based on `tokio::fs`
for temporary files that will be automatically deleted when the last reference to
the struct is dropped.
