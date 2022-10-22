use async_tempfile::TempFile;
use tokio::fs::OpenOptions;
use uuid::Uuid;

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

#[tokio::test]
async fn borrowed_file_is_not_dropped() {
    let file_name = format!("test_{}", Uuid::new_v4());
    let path = std::env::temp_dir().join(file_name);
    let _original = OpenOptions::new()
        .create_new(true)
        .read(false)
        .write(true)
        .open(path.clone())
        .await
        .unwrap();

    {
        let temp = TempFile::from_existing(path.clone()).await.unwrap();
        assert!(temp.file_path().is_file());
    }

    assert!(path.is_file());
    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn owned_file_is_dropped() {
    let file_name = format!("test_{}", Uuid::new_v4());
    let path = std::env::temp_dir().join(file_name);
    let _original = OpenOptions::new()
        .create_new(true)
        .read(false)
        .write(true)
        .open(path.clone())
        .await
        .unwrap();

    {
        let temp = TempFile::owned_from_existing(path.clone()).await.unwrap();
        assert!(temp.file_path().is_file());
    }

    assert!(!path.is_file());
    assert!(tokio::fs::remove_file(path).await.is_err());
}
