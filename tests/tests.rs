use tempfile::TempFile;

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