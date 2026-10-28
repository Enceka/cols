use super::*;

#[tokio::test]
async fn gameplay_policy_is_explicit_and_requires_correct_build_valid_rates_and_weapon_rules() {
    let (_temp, store, service) = setup().await;
    assert!(service.policy().await.unwrap().is_none());
    let path = store.root.join("progression-policy.json");
    let mut policy = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
        "enabled":true,"awards":{"kill_player_xp":100,"kill_weapon_xp":100,"headshot_player_bonus":0}});
    atomic_private_write(&path, &serde_json::to_vec(&policy).unwrap())
        .await
        .unwrap();
    assert_eq!(service.policy().await.unwrap().unwrap().kill_player_xp, 100);
    policy["enabled"] = false.into();
    atomic_private_write(&path, &serde_json::to_vec(&policy).unwrap())
        .await
        .unwrap();
    assert!(service.policy().await.unwrap().is_none());
    policy["enabled"] = true.into();
    policy["awards"]["kill_player_xp"] = 0.into();
    atomic_private_write(&path, &serde_json::to_vec(&policy).unwrap())
        .await
        .unwrap();
    assert!(service.policy().await.is_err());
    policy["awards"]["kill_player_xp"] = 100.into();
    policy["client_sha256"] = "other".into();
    atomic_private_write(&path, &serde_json::to_vec(&policy).unwrap())
        .await
        .unwrap();
    assert!(service.policy().await.is_err());
}

#[tokio::test]
async fn receipt_capacity_rejects_new_matches_without_evicting_retry_history() {
    let (_temp, store, service) = setup().await;
    service.commit(ACCOUNT, award(1)).await.unwrap();
    let mut ledger: Ledger = serde_json::from_slice(&bytes(&store).await).unwrap();
    ledger.receipts.clear();
    ledger.weapon_xp.clear();
    let tables = Tables::native().unwrap();
    for index in 1..=MAX_RECEIPTS {
        let input = MatchAward {
            match_id: format!("{index:032x}"),
            player_xp: 1,
            weapons: vec![],
        };
        ledger.receipts.push(SavedReceipt {
            award_sha256: digest(&serde_json::to_vec(&input).unwrap()),
            receipt: Receipt {
                match_id: input.match_id,
                revision: index as u64,
                player: tables.player.award(index as u32 - 1, 1).unwrap(),
                weapons: vec![],
            },
            acknowledged: false,
        });
    }
    ledger.revision = MAX_RECEIPTS as u64;
    let seed = tokio::fs::read(store.profile_path(ACCOUNT, true).await.unwrap())
        .await
        .unwrap();
    ledger.private_base64 =
        STANDARD.encode(profile::with_player_progress(&seed, 42, 1, MAX_RECEIPTS as u32).unwrap());
    let path = store
        .profile_directory(1002, 42)
        .await
        .unwrap()
        .join("progression.json");
    let full = serde_json::to_vec(&ledger).unwrap();
    atomic_private_write(&path, &full).await.unwrap();
    assert!(service.commit(ACCOUNT, award(1025)).await.is_err());
    assert_eq!(bytes(&store).await, full);
    let retry = MatchAward {
        match_id: format!("{:032x}", 1),
        player_xp: 1,
        weapons: vec![],
    };
    assert!(service.commit(ACCOUNT, retry).await.unwrap().duplicate);
    assert_eq!(bytes(&store).await, full);
}

#[tokio::test]
async fn offline_rank_boost_cannot_replace_the_imported_seed_of_a_progressed_account() {
    let (_temp, store, service) = setup().await;
    service.commit(ACCOUNT, award(1)).await.unwrap();
    let seed_path = store.profile_path(ACCOUNT, true).await.unwrap();
    let seed_before = tokio::fs::read(&seed_path).await.unwrap();
    let saved = bytes(&store).await;
    assert!(
        crate::diagnostics::account::boost_test_account(&store, 1002, 42, 100, 100)
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(seed_path).await.unwrap(), seed_before);
    assert_eq!(bytes(&store).await, saved);
}

#[tokio::test]
async fn delivery_acknowledgement_survives_restart_without_changing_award_revision() {
    let (_temp, store, service) = setup().await;
    let first = service.commit(ACCOUNT, award(1)).await.unwrap();
    assert_eq!(
        service.pending(ACCOUNT).await.unwrap(),
        std::slice::from_ref(&first.receipt)
    );
    assert!(
        service
            .acknowledge(ACCOUNT, &first.receipt.match_id, 2)
            .await
            .is_err()
    );
    service
        .acknowledge(ACCOUNT, &first.receipt.match_id, 1)
        .await
        .unwrap();
    let after = bytes(&store).await;
    service
        .acknowledge(ACCOUNT, &first.receipt.match_id, 1)
        .await
        .unwrap();
    assert_eq!(bytes(&store).await, after);
    let restarted = ProgressionService::new(Arc::new(
        AccountStore::open(store.root.clone()).await.unwrap(),
    ));
    assert!(restarted.pending(ACCOUNT).await.unwrap().is_empty());
    assert!(restarted.commit(ACCOUNT, award(1)).await.unwrap().duplicate);
    assert!(restarted.pending(ACCOUNT).await.unwrap().is_empty());
    assert_eq!(
        restarted
            .commit(ACCOUNT, award(2))
            .await
            .unwrap()
            .receipt
            .revision,
        2
    );
}

const ACCOUNT: AccountId = AccountId {
    title_id: 1002,
    user_id: 42,
};
async fn setup() -> (tempfile::TempDir, Arc<AccountStore>, ProgressionService) {
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
    let inventory = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
        "title_id":ACCOUNT.title_id,"user_id":"42","items":[10001,10049]});
    atomic_private_write(
        &directory.join("inventory.json"),
        &serde_json::to_vec(&inventory).unwrap(),
    )
    .await
    .unwrap();
    let rules = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
        "source_sha256":"a".repeat(64),"weapons":[
            {"item_id":10001,"is_primary":true,"attachments":[{"item_id":20001,"displayed_level":2}]},
            {"item_id":10049,"is_primary":false,"attachments":[]}]});
    atomic_private_write(
        &store.root.join("progression-rules.json"),
        &serde_json::to_vec(&rules).unwrap(),
    )
    .await
    .unwrap();
    let service = ProgressionService::new(store.clone());
    (temporary, store, service)
}
fn award(id: u32) -> MatchAward {
    MatchAward {
        match_id: format!("{id:032x}"),
        player_xp: 2001,
        weapons: vec![WeaponXp {
            item_id: 10001,
            xp: 500,
        }],
    }
}
async fn bytes(store: &AccountStore) -> Vec<u8> {
    tokio::fs::read(
        store
            .profile_directory(1002, 42)
            .await
            .unwrap()
            .join("progression.json"),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn restart_replays_the_original_receipt_and_both_profile_and_inventory_projection() {
    let (_temp, store, service) = setup().await;
    let directory = store.profile_directory(1002, 42).await.unwrap();
    let private_before = tokio::fs::read(directory.join("private.bin"))
        .await
        .unwrap();
    let inventory_before = tokio::fs::read(directory.join("inventory.json"))
        .await
        .unwrap();
    assert!(committed_private(&store, ACCOUNT).await.unwrap().is_none());
    let commit = service.commit(ACCOUNT, award(1)).await.unwrap();
    assert!(!commit.duplicate);
    assert_eq!(commit.receipt.player.after.level, 2);
    assert_eq!(commit.receipt.player.after.xp_within_level, 1);
    assert_eq!(commit.receipt.weapons[0].eligible_attachments, [20001]);
    let saved = bytes(&store).await;
    let reopened_store = Arc::new(AccountStore::open(store.root.clone()).await.unwrap());
    let reopened = ProgressionService::new(reopened_store.clone());
    let retry = reopened.commit(ACCOUNT, award(1)).await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.receipt, commit.receipt);
    assert_eq!(bytes(&reopened_store).await, saved);
    let profiles = crate::backend::profile::ProfileService::new(reopened_store.clone());
    let private = profiles.read(ACCOUNT, true).await.unwrap().unwrap();
    let progress = profile::player_progress(&private, 42).unwrap();
    assert_eq!(progress.rank, 2);
    assert_eq!(progress.experience, 1);
    let owned = [10001, 10049];
    assert_eq!(
        inventory_progress(&reopened_store, ACCOUNT, &owned)
            .await
            .unwrap(),
        [
            InventoryItem { id: 10001, xp: 500 },
            InventoryItem { id: 10049, xp: 0 }
        ]
    );
    assert_eq!(
        tokio::fs::read(directory.join("private.bin"))
            .await
            .unwrap(),
        private_before
    );
    assert_eq!(
        tokio::fs::read(directory.join("inventory.json"))
            .await
            .unwrap(),
        inventory_before
    );
    // Eligibility is a reminder, not ownership.
    assert_eq!(
        crate::backend::inventory::InventoryService::new(store.clone())
            .items(ACCOUNT)
            .await
            .unwrap(),
        owned
    );
}

#[tokio::test]
async fn concurrent_duplicate_and_distinct_matches_commit_without_lost_xp() {
    let (_temp, store, service) = setup().await;
    let (a, b) = tokio::join!(
        service.commit(ACCOUNT, award(1)),
        service.commit(ACCOUNT, award(1))
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.duplicate, b.duplicate);
    assert_eq!(a.receipt, b.receipt);
    let (a, b) = tokio::join!(
        service.commit(ACCOUNT, award(2)),
        service.commit(ACCOUNT, award(3))
    );
    assert!(a.is_ok() && b.is_ok());
    let private = committed_private(&store, ACCOUNT).await.unwrap().unwrap();
    assert_eq!(
        Tables::native()
            .unwrap()
            .player_total(profile::player_progress(&private, 42).unwrap())
            .unwrap(),
        6003
    );
    assert_eq!(
        inventory_progress(&store, ACCOUNT, &[10001]).await.unwrap()[0].xp,
        1500
    );
    let ledger: Ledger = serde_json::from_slice(&bytes(&store).await).unwrap();
    assert_eq!(ledger.revision, 3);
}

#[tokio::test]
async fn conflicts_unknown_unowned_or_duplicate_weapon_ids_never_partially_commit() {
    let (_temp, store, service) = setup().await;
    service.commit(ACCOUNT, award(1)).await.unwrap();
    let before = bytes(&store).await;
    let mut conflict = award(1);
    conflict.player_xp += 1;
    let mut unowned = award(2);
    unowned.weapons.push(WeaponXp {
        item_id: 10050,
        xp: 500,
    });
    let mut duplicate = award(3);
    duplicate.weapons.push(duplicate.weapons[0].clone());
    for invalid in [conflict, unowned, duplicate] {
        assert!(service.commit(ACCOUNT, invalid).await.is_err());
        assert_eq!(bytes(&store).await, before);
    }
    let wrong_title = AccountId {
        title_id: 1003,
        ..ACCOUNT
    };
    assert!(service.commit(wrong_title, award(2)).await.is_err());
    assert!(
        service
            .commit(
                AccountId {
                    user_id: 43,
                    ..ACCOUNT
                },
                award(2)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn corrupt_identity_crc_and_replaced_seed_are_rejected_on_all_read_paths() {
    let (_temp, store, service) = setup().await;
    service.commit(ACCOUNT, award(1)).await.unwrap();
    let directory = store.profile_directory(1002, 42).await.unwrap();
    let good = bytes(&store).await;
    let mut bad: Ledger = serde_json::from_slice(&good).unwrap();
    bad.user_id = "43".into();
    for bad_bytes in [serde_json::to_vec(&bad).unwrap(), b"{}".to_vec()] {
        atomic_private_write(&directory.join("progression.json"), &bad_bytes)
            .await
            .unwrap();
        assert!(committed_private(&store, ACCOUNT).await.is_err());
        assert!(inventory_progress(&store, ACCOUNT, &[10001]).await.is_err());
        assert!(service.commit(ACCOUNT, award(2)).await.is_err());
    }
    atomic_private_write(&directory.join("progression.json"), &good)
        .await
        .unwrap();
    let seed = tokio::fs::read(directory.join("private.bin"))
        .await
        .unwrap();
    let updated = profile::with_player_progress(&seed, 42, 2, 0).unwrap();
    atomic_private_write(&directory.join("private.bin"), &updated)
        .await
        .unwrap();
    assert!(committed_private(&store, ACCOUNT).await.is_err());
    assert!(service.commit(ACCOUNT, award(2)).await.is_err());
    assert_eq!(bytes(&store).await, good);
}

#[tokio::test]
async fn changed_rules_require_explicit_migration_and_failures_before_commit_leave_legacy_reads() {
    let (_temp, store, service) = setup().await;
    let path = store.root.join("progression-rules.json");
    let rules = tokio::fs::read(&path).await.unwrap();
    tokio::fs::remove_file(&path).await.unwrap();
    assert!(service.commit(ACCOUNT, award(1)).await.is_err());
    assert!(committed_private(&store, ACCOUNT).await.unwrap().is_none());
    assert_eq!(
        inventory_progress(&store, ACCOUNT, &[10001]).await.unwrap()[0].xp,
        0
    );
    atomic_private_write(&path, &rules).await.unwrap();
    service.commit(ACCOUNT, award(1)).await.unwrap();
    let saved = bytes(&store).await;
    let mut changed: serde_json::Value = serde_json::from_slice(&rules).unwrap();
    changed["weapons"][0]["is_primary"] = serde_json::json!(false);
    atomic_private_write(&path, &serde_json::to_vec(&changed).unwrap())
        .await
        .unwrap();
    assert!(service.commit(ACCOUNT, award(2)).await.is_err());
    assert_eq!(bytes(&store).await, saved);
    assert_eq!(
        inventory_progress(&store, ACCOUNT, &[10001]).await.unwrap()[0].xp,
        500
    );
}

#[tokio::test]
async fn old_receipts_remain_retryable_after_later_matches_without_replaying_attachment_eligibility()
 {
    let (_temp, store, service) = setup().await;
    let first = service.commit(ACCOUNT, award(1)).await.unwrap();
    let later = service.commit(ACCOUNT, award(2)).await.unwrap();
    assert!(later.receipt.weapons[0].eligible_attachments.is_empty());
    let retry = service.commit(ACCOUNT, award(1)).await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.receipt, first.receipt);
    assert_eq!(
        inventory_progress(&store, ACCOUNT, &[10001]).await.unwrap()[0].xp,
        1000
    );
}

#[tokio::test]
async fn acknowledged_receipts_archive_into_one_segment_and_free_the_live_window() {
    let (_temp, store, service) = setup().await;
    let first = service.commit(ACCOUNT, award(1)).await.unwrap();
    service.commit(ACCOUNT, award(2)).await.unwrap();
    let directory = store.profile_directory(1002, 42).await.unwrap();
    let before = committed_private(&store, ACCOUNT).await.unwrap();
    assert!(service.archive(ACCOUNT).await.unwrap().is_none());
    service
        .acknowledge(ACCOUNT, &first.receipt.match_id, first.receipt.revision)
        .await
        .unwrap();
    let archived = service.archive(ACCOUNT).await.unwrap().unwrap();
    assert!(archived.file == "progression-archive-0000.json");
    assert!((archived.from_revision, archived.to_revision) == (1, 1));
    let raw = tokio::fs::read(directory.join(&archived.file))
        .await
        .unwrap();
    assert!(digest(&raw) == archived.sha256);
    let segment: Segment = serde_json::from_slice(&raw).unwrap();
    assert!(segment.receipts.len() == 1);
    assert!(segment.player_total_after == segment.receipts[0].receipt.player.after.total_xp);
    let ledger: Ledger = serde_json::from_slice(&bytes(&store).await).unwrap();
    assert!(ledger.base.revision == 1);
    assert!(ledger.archived.len() == 1);
    assert!(ledger.retired.len() == 1);
    assert!(ledger.receipts.len() == 1);
    assert!(ledger.revision == 2);
    assert!(committed_private(&store, ACCOUNT).await.unwrap() == before);
    assert!(inventory_progress(&store, ACCOUNT, &[10001]).await.unwrap()[0].xp == 1000);
    service.commit(ACCOUNT, award(3)).await.unwrap();
    let after: Ledger = serde_json::from_slice(&bytes(&store).await).unwrap();
    assert!(after.revision == 3);
    assert!(after.base.revision == 1);
}

#[tokio::test]
async fn archived_receipts_stay_retryable_and_tampered_segments_are_rejected() {
    let (_temp, store, service) = setup().await;
    let first = service.commit(ACCOUNT, award(1)).await.unwrap();
    service
        .acknowledge(ACCOUNT, &first.receipt.match_id, first.receipt.revision)
        .await
        .unwrap();
    let archived = service.archive(ACCOUNT).await.unwrap().unwrap();
    let retry = service.commit(ACCOUNT, award(1)).await.unwrap();
    assert!(retry.duplicate);
    assert!(retry.receipt == first.receipt);
    let mut conflict = award(1);
    conflict.player_xp = 7;
    assert!(service.commit(ACCOUNT, conflict).await.is_err());
    let directory = store.profile_directory(1002, 42).await.unwrap();
    let raw = tokio::fs::read(directory.join(&archived.file))
        .await
        .unwrap();
    atomic_private_write(&directory.join(&archived.file), b"{}")
        .await
        .unwrap();
    assert!(service.commit(ACCOUNT, award(1)).await.is_err());
    atomic_private_write(&directory.join(&archived.file), &raw)
        .await
        .unwrap();
    assert!(service.commit(ACCOUNT, award(1)).await.unwrap().duplicate);
}

#[tokio::test]
async fn capacity_archives_the_acknowledged_prefix_and_keeps_live_retry_history() {
    let (_temp, store, service) = setup().await;
    service.commit(ACCOUNT, award(1)).await.unwrap();
    let mut ledger: Ledger = serde_json::from_slice(&bytes(&store).await).unwrap();
    ledger.receipts.clear();
    ledger.weapon_xp.clear();
    let tables = Tables::native().unwrap();
    for index in 1..=MAX_RECEIPTS {
        let input = MatchAward {
            match_id: format!("{index:032x}"),
            player_xp: 1,
            weapons: Vec::new(),
        };
        ledger.receipts.push(SavedReceipt {
            award_sha256: digest(&serde_json::to_vec(&input).unwrap()),
            receipt: Receipt {
                match_id: input.match_id,
                revision: index as u64,
                player: tables.player.award(index as u32 - 1, 1).unwrap(),
                weapons: Vec::new(),
            },
            acknowledged: index < MAX_RECEIPTS,
        });
    }
    ledger.revision = MAX_RECEIPTS as u64;
    let seed = tokio::fs::read(store.profile_path(ACCOUNT, true).await.unwrap())
        .await
        .unwrap();
    ledger.private_base64 =
        STANDARD.encode(profile::with_player_progress(&seed, 42, 1, MAX_RECEIPTS as u32).unwrap());
    let path = store
        .profile_directory(1002, 42)
        .await
        .unwrap()
        .join("progression.json");
    atomic_private_write(&path, &serde_json::to_vec(&ledger).unwrap())
        .await
        .unwrap();
    service.commit(ACCOUNT, award(1025)).await.unwrap();
    let saved: Ledger = serde_json::from_slice(&bytes(&store).await).unwrap();
    assert!(saved.base.revision == (MAX_RECEIPTS - 1) as u64);
    assert!(saved.archived.len() == 1);
    assert!(saved.retired.len() == MAX_RECEIPTS - 1);
    assert!(saved.receipts.len() == 2);
    assert!(saved.revision == MAX_RECEIPTS as u64 + 1);
    let retry = MatchAward {
        match_id: format!("{:032x}", 1),
        player_xp: 1,
        weapons: Vec::new(),
    };
    assert!(service.commit(ACCOUNT, retry).await.unwrap().duplicate);
    let unchanged: Ledger = serde_json::from_slice(&bytes(&store).await).unwrap();
    assert!(unchanged.revision == MAX_RECEIPTS as u64 + 1);
}

#[tokio::test]
async fn retry_of_an_archived_match_survives_tombstone_trimming() {
    let (_temp, store, service) = setup().await;
    let first = service.commit(ACCOUNT, award(1)).await.unwrap();
    service
        .acknowledge(ACCOUNT, &first.receipt.match_id, first.receipt.revision)
        .await
        .unwrap();
    service.archive(ACCOUNT).await.unwrap().unwrap();
    let path = store
        .profile_directory(1002, 42)
        .await
        .unwrap()
        .join("progression.json");
    let mut ledger: Ledger = serde_json::from_slice(&bytes(&store).await).unwrap();
    ledger.retired.clear();
    atomic_private_write(&path, &serde_json::to_vec(&ledger).unwrap())
        .await
        .unwrap();
    let retry = service.commit(ACCOUNT, award(1)).await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.receipt, first.receipt);
    let mut conflict = award(1);
    conflict.player_xp = 7;
    assert!(service.commit(ACCOUNT, conflict).await.is_err());
}
