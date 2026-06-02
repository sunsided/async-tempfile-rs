//! Tests for the drop-safety contract: temporary files/directories are always
//! cleaned up (even under cancellation or panic) and dropping never deadlocks.
//!
//! See the project plan's "Drop-safety contract" for the invariants exercised
//! here:
//!   1. lock-free core → `Drop` cannot deadlock on a lock
//!   2. synchronous `Drop` never re-enters the runtime
//!   3. the synchronous `Drop` is the always-armed backstop; async paths disarm
//!      only after success → cancellation/panic always falls back to cleanup
//!   4. field order closes handles before deletion

use std::collections::HashSet;
use std::time::Duration;

use async_tempfile::{Ownership, TempDir, TempFile};

const WATCHDOG: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// A. Feature/behavior
// ---------------------------------------------------------------------------

#[tokio::test]
async fn keep_prevents_deletion() {
    let file = TempFile::new().await.unwrap();
    assert_eq!(file.ownership(), Ownership::Owned);
    let path = file.keep();

    assert!(path.is_file(), "kept file must survive the dropped handle");
    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn keep_affects_clones_sharing_the_core() {
    let file = TempFile::new().await.unwrap();
    let clone = file.open_rw().await.unwrap();
    let path = file.file_path().clone();

    // Keeping via one handle disables deletion for the shared underlying file.
    let _ = file.keep();
    drop(clone);

    assert!(path.is_file(), "clone must not delete a kept file");
    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn close_deletes_owned_file_and_reports_ok() {
    let file = TempFile::new().await.unwrap();
    let path = file.file_path().clone();
    assert!(path.is_file());

    file.close().expect("closing an owned file should succeed");

    assert!(!path.exists(), "close must delete the owned file");
}

#[tokio::test]
async fn close_via_clone_leaves_file_for_remaining_reference() {
    let file = TempFile::new().await.unwrap();
    let clone = file.open_rw().await.unwrap();
    let path = file.file_path().clone();

    // Closing one handle while another reference is alive must not delete the
    // file: cleanup falls to the surviving reference's `Drop`.
    file.close().expect("close with live clones returns Ok");
    assert!(
        path.is_file(),
        "file must survive while a clone references it"
    );

    drop(clone);
    assert!(!path.exists(), "last reference dropping deletes the file");
}

#[tokio::test]
async fn close_on_borrowed_file_is_a_noop() {
    let file = TempFile::new().await.unwrap();
    let path = file.keep(); // disarm: now borrowed
    let borrowed = TempFile::from_existing(path.clone(), Ownership::Borrowed)
        .await
        .unwrap();

    borrowed
        .close()
        .expect("closing a borrowed file returns Ok");
    assert!(path.is_file(), "borrowed file must not be deleted by close");

    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn close_deletes_owned_dir_recursively() {
    let dir = TempDir::new().await.unwrap();
    let path = dir.dir_path().clone();
    // Leave a file inside so the removal must recurse. `keep()` closes the
    // handle but leaves the file on disk, so this stays correct on Windows,
    // where an open handle inside the directory would block `remove_dir_all`.
    let inner_path = TempFile::new_in(path.as_path()).await.unwrap().keep();
    assert!(inner_path.is_file());

    dir.close().expect("closing an owned dir should succeed");

    assert!(!path.exists(), "close must remove the dir and its contents");
}

#[tokio::test]
async fn persist_moves_and_survives() {
    let file = TempFile::new().await.unwrap();
    let original = file.file_path().clone();
    let target = std::env::temp_dir().join(format!("persist_test_{}.bin", std::process::id()));
    let _ = tokio::fs::remove_file(&target).await;

    let persisted = file.persist(&target).await.unwrap();
    assert_eq!(persisted, target);
    assert!(persisted.is_file(), "persisted file must exist");
    assert!(
        !original.exists(),
        "original temp path must be gone after move"
    );

    tokio::fs::remove_file(target).await.unwrap();
}

#[tokio::test]
async fn builder_honors_prefix_and_suffix() {
    let file = TempFile::builder()
        .prefix("pfx_")
        .suffix(".sfx")
        .create()
        .await
        .unwrap();

    let name = file
        .file_path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("pfx_"), "name was {name}");
    assert!(name.ends_with(".sfx"), "name was {name}");
}

#[tokio::test]
async fn dir_builder_and_persist() {
    let dir = TempDir::builder().prefix("dirpfx_").create().await.unwrap();
    let name = dir
        .dir_path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("dirpfx_"), "name was {name}");

    let target = std::env::temp_dir().join(format!("persist_dir_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&target).await;
    let persisted = dir.persist(&target).await.unwrap();
    assert!(persisted.is_dir());
    tokio::fs::remove_dir_all(persisted).await.unwrap();
}

// ---------------------------------------------------------------------------
// B. Cleanup under cancellation (invariant 3)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn drop_async_cancelled_still_deletes() {
    let file = TempFile::new().await.unwrap();
    let path = file.file_path().clone();
    assert!(path.is_file());

    // Drive `drop_async` until it parks at the `remove_file` await, then drop
    // the future without completing it. The synchronous backstop in `Drop` must
    // still delete the file.
    let mut fut = tokio_test::task::spawn(file.drop_async());
    let _ = fut.poll();
    drop(fut);

    assert!(
        !path.exists(),
        "a cancelled drop_async must still delete the file via the sync backstop"
    );
}

#[tokio::test]
async fn persist_failure_preserves_file_and_reports_path() {
    let file = TempFile::new().await.unwrap();
    let original = file.file_path().clone();

    // A fresh, non-existent directory under temp_dir() → the rename fails
    // reliably because the target's parent directory is missing.
    let missing_dir =
        std::env::temp_dir().join(format!("async_tempfile_missing_{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&missing_dir).await;
    let bad_target = missing_dir.join("target.bin");

    let err = file
        .persist(&bad_target)
        .await
        .expect_err("persist into a missing directory must fail");

    // The data is not lost: the temporary still exists at the reported path.
    assert_eq!(err.path, original);
    assert!(
        original.exists(),
        "a failed persist must NOT delete the original temp file"
    );
    assert!(!bad_target.exists());

    // The caller can recover it; re-adopting restores automatic cleanup.
    let recovered = TempFile::from_existing(original.clone(), Ownership::Owned)
        .await
        .unwrap();
    drop(recovered);
    assert!(!original.exists());
}

// ---------------------------------------------------------------------------
// C. Panic-safety (invariant 3)
// ---------------------------------------------------------------------------

#[test]
fn panic_while_holding_deletes_file() {
    use std::sync::{Arc, Mutex};

    let captured: Arc<Mutex<Option<std::path::PathBuf>>> = Arc::new(Mutex::new(None));
    let captured_clone = captured.clone();

    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let file = TempFile::new().await.unwrap();
            *captured_clone.lock().unwrap() = Some(file.file_path().clone());
            panic!("boom while holding a TempFile");
        });
    });

    // The spawned thread panics; unwinding must run the file's `Drop`.
    assert!(handle.join().is_err(), "thread was expected to panic");

    let path = captured.lock().unwrap().clone().expect("path was captured");
    assert!(
        !path.exists(),
        "a TempFile must be deleted while unwinding from a panic"
    );
}

// ---------------------------------------------------------------------------
// D. Deadlock watchdog (invariants 1 & 2)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_drop_on_multi_thread_runtime_does_not_deadlock() {
    let path = tokio::time::timeout(WATCHDOG, async {
        let file = TempFile::new().await.unwrap();
        let path = file.file_path().clone();
        drop(file); // synchronous Drop on a runtime worker thread
        path
    })
    .await
    .expect("synchronous drop must not deadlock on a multi-thread runtime");

    assert!(!path.exists());
}

#[tokio::test(flavor = "current_thread")]
async fn sync_drop_on_current_thread_runtime_does_not_deadlock() {
    let path = tokio::time::timeout(WATCHDOG, async {
        let dir = TempDir::new().await.unwrap();
        let path = dir.dir_path().clone();
        drop(dir);
        path
    })
    .await
    .expect("synchronous drop must not deadlock on a current-thread runtime");

    assert!(!path.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_async_does_not_deadlock() {
    let path = tokio::time::timeout(WATCHDOG, async {
        let file = TempFile::new().await.unwrap();
        let path = file.file_path().clone();
        file.drop_async().await;
        path
    })
    .await
    .expect("drop_async must not deadlock");

    assert!(!path.exists());
}

// ---------------------------------------------------------------------------
// E. Stress / leak detection (no leaks, no double-free, names unique)
// ---------------------------------------------------------------------------

// Modest concurrency: these run in parallel with every other test in this
// binary, so the peak open-file-descriptor count must stay well under a typical
// `ulimit -n` of 1024. Each live `TempFile` holds exactly one descriptor.
const TASKS: u32 = 8;
const ITERS: u32 = 25;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_concurrent_create_and_drop_leaves_nothing() {
    let dir = TempDir::new().await.unwrap();
    let dir_path = dir.dir_path().clone();

    let mut handles = Vec::new();
    for task in 0..TASKS {
        let d = dir_path.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..ITERS {
                let file = TempFile::new_in(d.as_path()).await.unwrap();
                // Exercise the shared-core clone path too.
                let clone = file.open_rw().await.unwrap();
                drop(clone);
                // Alternate between the sync and async drop paths.
                if task % 2 == 0 {
                    file.drop_async().await;
                } else {
                    drop(file);
                }
            }
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    // Every created file is owned and dropped → the directory must be empty.
    let mut entries = tokio::fs::read_dir(&dir_path).await.unwrap();
    let mut leftover = Vec::new();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        leftover.push(entry.file_name());
    }
    assert!(leftover.is_empty(), "temporary files leaked: {leftover:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_auto_names_are_unique() {
    let dir = TempDir::new().await.unwrap();
    let dir_path = dir.dir_path().clone();

    let mut handles = Vec::new();
    for _ in 0..TASKS {
        let d = dir_path.clone();
        handles.push(tokio::spawn(async move {
            let mut paths = Vec::new();
            for _ in 0..ITERS {
                // The per-process counter (and UUIDv4 under the `uuid` feature)
                // makes every generated name unique for the lifetime of the
                // process, so the file can be dropped immediately - no need to
                // hold descriptors open to prove uniqueness.
                let file = TempFile::new_in(d.as_path()).await.unwrap();
                paths.push(file.file_path().clone());
            }
            paths
        }));
    }

    let mut all = HashSet::new();
    for handle in handles {
        for path in handle.await.unwrap() {
            assert!(all.insert(path.clone()), "duplicate temp name: {path:?}");
        }
    }
    assert_eq!(all.len() as u32, TASKS * ITERS);
}
