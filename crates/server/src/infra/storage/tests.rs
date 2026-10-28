use super::*;

#[tokio::test]
async fn atomic_private_replacement_persists_updates_without_leaving_temporary_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("player.json");
    for content in [b"initial".as_slice(), b"updated", b"replacement"] {
        atomic_private_write(&path, content).await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), content);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}
