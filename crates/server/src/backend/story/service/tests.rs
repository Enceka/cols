use super::*;
use crate::infra::storage::atomic_private_write;

const ACCOUNT: AccountId = AccountId {
    title_id: 1002,
    user_id: 42,
};

async fn setup() -> (tempfile::TempDir, Arc<AccountStore>, StoryService) {
    let temporary = tempfile::tempdir().unwrap();
    let store = Arc::new(AccountStore::open(temporary.path().into()).await.unwrap());
    let directory = store
        .profile_directory(ACCOUNT.title_id, ACCOUNT.user_id)
        .await
        .unwrap();
    let mut blank = vec![0; profile::SNAPSHOT_BYTES];
    let crc = crc32fast::hash(&blank[4..]);
    blank[..4].copy_from_slice(&crc.to_le_bytes());
    let seed = profile::prepare_private(&blank, ACCOUNT.user_id, true).unwrap();
    atomic_private_write(&directory.join("private.bin"), &seed)
        .await
        .unwrap();
    let service = StoryService::new(store.clone());
    (temporary, store, service)
}

async fn private(store: &AccountStore) -> Vec<u8> {
    let path = store.profile_path(ACCOUNT, true).await.unwrap();
    read_bounded(&path, 61440).await.unwrap().unwrap()
}

#[tokio::test]
async fn first_completion_unlocks_the_second_level_once() {
    let (_temp, store, service) = setup().await;
    let before = private(&store).await;
    let completion = service
        .complete(ACCOUNT, codol_protocol::room::story::FIRST_MAP, 2, 90)
        .await
        .unwrap();
    assert!(completion.first_time && completion.attempts == 2 && completion.seconds == 90);
    let unlocked = private(&store).await;
    assert!(unlocked != before);
    assert_eq!(
        profile::story_unlock(
            &unlocked,
            ACCOUNT.user_id,
            codol_protocol::room::story::FIRST_COMPLETION_FLAG
        )
        .unwrap(),
        1
    );
    let ledger: Ledger = serde_json::from_slice(
        &read_bounded(
            &store
                .profile_directory(ACCOUNT.title_id, ACCOUNT.user_id)
                .await
                .unwrap()
                .join("story.json"),
            MAX_LEDGER_BYTES,
        )
        .await
        .unwrap()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(ledger.completions.len(), 1);
    let repeat = service
        .complete(ACCOUNT, codol_protocol::room::story::FIRST_MAP, 5, 120)
        .await
        .unwrap();
    let repeated = repeat.first_time;
    assert!(!repeated);
    assert_eq!(private(&store).await, unlocked);
    let ledger: Ledger = serde_json::from_slice(
        &read_bounded(
            &store
                .profile_directory(ACCOUNT.title_id, ACCOUNT.user_id)
                .await
                .unwrap()
                .join("story.json"),
            MAX_LEDGER_BYTES,
        )
        .await
        .unwrap()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(ledger.completions.len(), 1);
    assert_eq!(ledger.completions[0].attempts, 2);
}

#[tokio::test]
async fn only_the_first_level_and_sane_ranges_are_accepted() {
    let (_temp, store, service) = setup().await;
    let before = private(&store).await;
    assert!(service.complete(ACCOUNT, "mp_afghan", 1, 10).await.is_err());
    assert!(
        service
            .complete(ACCOUNT, codol_protocol::room::story::FIRST_MAP, 0, 10)
            .await
            .is_err()
    );
    assert!(
        service
            .complete(ACCOUNT, codol_protocol::room::story::FIRST_MAP, 1, 90_000)
            .await
            .is_err()
    );
    assert_eq!(private(&store).await, before);
    let ledger = store
        .profile_directory(ACCOUNT.title_id, ACCOUNT.user_id)
        .await
        .unwrap()
        .join("story.json");
    assert!(std::fs::metadata(&ledger).is_err());
}

#[tokio::test]
async fn a_replaced_seed_rejects_the_stored_completion() {
    let (_temp, store, service) = setup().await;
    service
        .complete(ACCOUNT, codol_protocol::room::story::FIRST_MAP, 1, 30)
        .await
        .unwrap();
    let path = store.profile_path(ACCOUNT, true).await.unwrap();
    let mut blank = vec![0; profile::SNAPSHOT_BYTES];
    let crc = crc32fast::hash(&blank[4..]);
    blank[..4].copy_from_slice(&crc.to_le_bytes());
    let replaced = profile::prepare_private(&blank, ACCOUNT.user_id, true).unwrap();
    atomic_private_write(&path, &replaced).await.unwrap();
    assert!(
        service
            .complete(ACCOUNT, codol_protocol::room::story::FIRST_MAP, 1, 30)
            .await
            .is_err()
    );
}
