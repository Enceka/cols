use super::*;
use crate::backend::commerce::marketplace::LocalWallet;
use crate::backend::commerce::marketplace::Payment;
use crate::infra::storage::atomic_private_write;
use codol_protocol::lobby::{
    profile,
    task::{self, Writer},
};
#[tokio::test]
async fn committed_progression_projects_to_profile_rpc_both_inventory_rpcs_and_game_admission() {
    use crate::contracts::progression::{MatchAward, WeaponXp};
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let session = session(42);
    let account: AccountId = (&session).into();
    let directory = server
        .accounts
        .profile_directory(account.title_id, account.user_id)
        .await
        .unwrap();
    let mut blank = vec![0; profile::SNAPSHOT_BYTES];
    let crc = crc32fast::hash(&blank[4..]);
    blank[..4].copy_from_slice(&crc.to_le_bytes());
    let private = profile::prepare_private(&blank, account.user_id, true).unwrap();
    atomic_private_write(&directory.join("private.bin"), &private)
        .await
        .unwrap();
    server.validate_game_progression(account).await.unwrap();
    let crossing = profile::with_player_progress(&private, account.user_id, 1, 2000).unwrap();
    atomic_private_write(&directory.join("private.bin"), &crossing)
        .await
        .unwrap();
    assert!(server.validate_game_progression(account).await.is_err());
    atomic_private_write(&directory.join("private.bin"), &private)
        .await
        .unwrap();
    let public = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/public-profile-synthetic.bin"
    ));
    atomic_private_write(&directory.join("public.bin"), public)
        .await
        .unwrap();
    let inventory = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
        "title_id":account.title_id,"user_id":account.user_id.to_string(),"items":[10001,10049]});
    atomic_private_write(
        &directory.join("inventory.json"),
        &serde_json::to_vec(&inventory).unwrap(),
    )
    .await
    .unwrap();
    let rules = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
        "source_sha256":"a".repeat(64),"weapons":[{"item_id":10001,"is_primary":true,"attachments":[]}]});
    atomic_private_write(
        &server.accounts.root.join("progression-rules.json"),
        &serde_json::to_vec(&rules).unwrap(),
    )
    .await
    .unwrap();
    server
        .commit_match_progression(
            account,
            MatchAward {
                match_id: "1".repeat(32),
                player_xp: 2001,
                weapons: vec![WeaponXp {
                    item_id: 10001,
                    xp: 500,
                }],
            },
        )
        .await
        .unwrap();
    let profile_reply = server
        .handle(&[8, 3, 2, 0], &session, 1)
        .await
        .unwrap()
        .result
        .unwrap();
    let mut data = profile_reply[0][9..].to_vec();
    data.push(0);
    assert_eq!(
        profile::player_rank(&data, account.user_id).unwrap(),
        (1, 0)
    );
    assert_eq!(
        profile::player_progress(&data, account.user_id)
            .unwrap()
            .experience,
        1
    );
    let items = [
        task::InventoryItem { id: 10001, xp: 500 },
        task::InventoryItem { id: 10049, xp: 0 },
    ];
    let mut initial = Writer(vec![80, 3, 230]);
    initial.string("");
    initial.0.push(0);
    let initial_reply = server
        .handle(&initial.0, &session, 1)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        initial_reply,
        [task::player_data_with_inventory(account.user_id, &items, &[]).unwrap()]
    );
    let mut refresh = Writer(vec![80, 3, 203]);
    refresh.string("");
    refresh.u16(20000);
    refresh.0.push(0);
    let refresh_reply = server
        .handle(&refresh.0, &session, 1)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        refresh_reply,
        task::inventory_records_with_progress(account.user_id, &items).unwrap()
    );
    let game = server
        .game_profile(account.title_id, account.user_id)
        .await
        .unwrap();
    assert_eq!(game.rank, (1, 0));
    assert_eq!(game.public, public);
    assert_eq!(game.owned, [10001, 10049]);
}
#[tokio::test]
async fn game_profile_reads_latest_save_and_combines_starter_and_purchase_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let services = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(1);
    let directory = services
        .accounts
        .profile_directory(s.title_id, s.user_id)
        .await
        .unwrap();
    let inventory = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "title_id":s.title_id,"user_id":s.user_id.to_string(),"items":[10001]});
    atomic_private_write(
        &directory.join("inventory.json"),
        &serde_json::to_vec(&inventory).unwrap(),
    )
    .await
    .unwrap();
    let wallet = LocalWallet {
        schema_version: 1,
        client_sha256: codol_protocol::CLIENT_SHA256.into(),
        title_id: s.title_id,
        user_id: s.user_id.to_string(),
        cp: 0,
        gp: 0,
        purchases: vec![crate::backend::commerce::marketplace::Grant {
            sku_id: 14629,
            item_id: 20659,
            payment: Payment::Cp,
            price: 280,
        }],
    };
    atomic_private_write(
        &directory.join("wallet.json"),
        &serde_json::to_vec(&wallet).unwrap(),
    )
    .await
    .unwrap();
    for item in [0u32, 20659] {
        let mut plain = vec![0u8; 32768];
        plain[1000..1004].copy_from_slice(&item.to_le_bytes());
        let crc = crc32fast::hash(&plain[4..]);
        plain[..4].copy_from_slice(&crc.to_le_bytes());
        let mut public = Writer::default();
        public.u32(0);
        public.u32(1);
        public.u32(crc);
        public.blob(&plain).unwrap();
        for _ in 0..125 {
            public.i32(0);
        }
        public.0.push(0);
        let mut upload = vec![8, 3, 3];
        upload.extend_from_slice(&public.0);
        services
            .handle(&upload, &s, 1)
            .await
            .unwrap()
            .result
            .unwrap();
        let GameProfile {
            public: saved,
            owned,
            rank,
        } = services.game_profile(s.title_id, s.user_id).await.unwrap();
        assert_eq!(rank, (0, 0));
        assert_eq!(saved, public.0);
        assert_eq!(owned, [10001, 20659]);
        assert!(services.game_profile(s.title_id, 2).await.is_err());
        assert!(
            services
                .game_profile(s.title_id + 1, s.user_id)
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn room_name_check_preserves_utf8_and_reports_unchanged_text() {
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(1);
    let mut query = Writer(vec![71, 3, 2]);
    query.string("本地房间");
    query.0.push(0);
    let response = server
        .handle(&query.0, &s, 1)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(&response[0][..3], &[1, 0, 16]);
    assert_eq!(&response[0][3..], "本地房间\0".as_bytes());
    for n in 0..query.0.len() {
        assert!(server.handle(&query.0[..n], &s, 1).await.is_err());
    }
    for text in ["", "room\nname"] {
        let mut q = Writer(vec![71, 3, 2]);
        q.string(text);
        q.0.push(0);
        assert_eq!(
            server.handle(&q.0, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
    }
}
fn buy(service: u8, sku: u32) -> Vec<u8> {
    let mut w = Writer(vec![service, 3, if service == 80 { 154 } else { 131 }]);
    w.string("");
    w.u32(1);
    w.u32(sku);
    w.u32(1);
    w.u32(1);
    w.boolean(false);
    w.u32(1);
    w.u64(0);
    w.string("");
    w.u32(0);
    w.u32(0);
    w.u8(0);
    w.u32(u32::from(service == 80));
    w.0.push(0);
    w.0
}
#[tokio::test]
async fn purchases_commit_debit_and_item_together_and_reject_duplicates_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let server = LocalServices::open(root.clone()).await.unwrap();
    let s = session(0x89abcdef01234567);
    let wallet_path = server
        .accounts
        .profile_directory(s.title_id, s.user_id)
        .await
        .unwrap()
        .join("wallet.json");
    let initial = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "title_id":s.title_id,"user_id":s.user_id.to_string(),"cp":1000,"gp":3000});
    atomic_private_write(&wallet_path, &serde_json::to_vec(&initial).unwrap())
        .await
        .unwrap();
    let shop = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "title_id":s.title_id,"source_sha256":"a".repeat(64),"offers":[
            {"sku_id":10073,"item_id":10183,"currency":2,"price":2600},
            {"sku_id":14629,"item_id":20659,"currency":1,"price":280},
            {"sku_id":15,"item_id":10183,"currency":1,"price":100},
            {"sku_id":16,"item_id":5,"currency":2,"price":9999}]});
    atomic_private_write(&root.join("shop.json"), &serde_json::to_vec(&shop).unwrap())
        .await
        .unwrap();
    let request = buy(80, 10073);
    let (a, b) = tokio::join!(
        server.handle(&request, &s, 1),
        server.handle(&request, &s, 1)
    );
    assert_eq!(
        usize::from(a.unwrap().result.is_ok()) + usize::from(b.unwrap().result.is_ok()),
        1
    );
    assert!(
        server
            .handle(&buy(93, 14629), &s, 2)
            .await
            .unwrap()
            .result
            .unwrap()
            .is_empty()
    );
    drop(server);
    let server = LocalServices::open(root).await.unwrap();
    let wallet = server.commerce.wallet(&s).await.unwrap().unwrap();
    assert_eq!((wallet.cp, wallet.gp), (720, 400));
    assert_eq!(
        wallet
            .purchases
            .iter()
            .map(|p| p.item_id)
            .collect::<Vec<_>>(),
        [10183, 20659]
    );
    let records = server
        .handle(&[80, 3, 230, 16, 0, 0], &s, 3)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        records,
        [task::player_data_with_balances(s.user_id, &[10183, 20659], &[(1, 400)]).unwrap()]
    );
    let refresh = [80, 3, 203, 16, 0, 6, 32, 78, 0];
    let refreshed = server
        .handle(&refresh, &s, 3)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(refreshed.len(), 2);
    for (record, id) in refreshed.iter().zip([10183, 20659]) {
        let mut r = task::Reader::new(record);
        assert_eq!(r.u64().unwrap(), s.user_id);
        assert_eq!(r.string(16).unwrap(), "");
        assert_eq!(r.u32().unwrap(), id);
        assert_eq!(r.u32().unwrap(), 1);
    }
    assert_eq!(
        server
            .handle(&[80, 3, 130, 8, 1, 0, 0, 0, 0], &s, 3)
            .await
            .unwrap()
            .result,
        Ok(vec![vec![3, 1, 8, 144, 1, 0, 0]])
    );
    assert_eq!(
        server
            .handle(&refresh, &session(2), 3)
            .await
            .unwrap()
            .result,
        Ok(vec![])
    );
    for n in 0..refresh.len() {
        assert!(server.handle(&refresh[..n], &s, 3).await.is_err());
    }
    let before = tokio::fs::read(&wallet_path).await.unwrap();
    // Replays, alternative SKU for owned item, wrong currency, unknown SKU, overdraft.
    for request in [
        buy(80, 10073),
        buy(93, 14629),
        buy(93, 15),
        buy(93, 10073),
        buy(80, 999),
        buy(80, 16),
    ] {
        assert_eq!(
            server.handle(&request, &s, 4).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        assert_eq!(tokio::fs::read(&wallet_path).await.unwrap(), before);
    }
    assert_eq!(
        server
            .handle(&buy(80, 10073), &session(2), 4)
            .await
            .unwrap()
            .result,
        Err(task::TASK_FAILED)
    );
    assert!(server.commerce.wallet(&session(2)).await.unwrap().is_none());
}
#[tokio::test]
async fn test_account_boost_backs_up_progress_and_persists_consistent_isolated_balances() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let server = LocalServices::open(root.clone()).await.unwrap();
    let s = session(0x89abcdef01234567);
    let mut blank = vec![0; profile::SNAPSHOT_BYTES];
    // CRC32 of a 32764-byte zero default, computed independently.
    blank[..4].copy_from_slice(&0xb52d2b04u32.to_le_bytes());
    let seeded = profile::prepare_private(&blank, s.user_id, true).unwrap();
    let private_path = server.accounts.profile_path(&s, true).await.unwrap();
    atomic_private_write(&private_path, &seeded).await.unwrap();
    let public_path = server.accounts.profile_path(&s, false).await.unwrap();
    atomic_private_write(&public_path, b"preserve public")
        .await
        .unwrap();
    let inventory_path = private_path.with_file_name("inventory.json");
    let inventory = serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
        "title_id":s.title_id,"user_id":s.user_id.to_string(),"items":[792]
    }))
    .unwrap();
    atomic_private_write(&inventory_path, &inventory)
        .await
        .unwrap();
    let backup = server
        .boost_test_account(s.title_id, s.user_id, 100_000, 1_000_000)
        .await
        .unwrap();
    assert_eq!(
        tokio::fs::read(backup.join("private.bin")).await.unwrap(),
        seeded
    );
    assert!(!backup.join("wallet.json").exists());
    assert_eq!(
        tokio::fs::read(&public_path).await.unwrap(),
        b"preserve public"
    );
    assert_eq!(tokio::fs::read(&inventory_path).await.unwrap(), inventory);
    drop(server);
    let server = LocalServices::open(root).await.unwrap();
    let player = server
        .handle(&[80, 3, 230, 16, 0, 0], &s, 1)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        &player[0][11..28],
        &[8, 1, 0, 0, 0, 3, 1, 8, 64, 66, 15, 0, 8, 1, 0, 0, 0]
    );
    let external = [93, 3, 1, 8, 1, 0, 0, 0, 0];
    assert_eq!(
        server.handle(&external, &s, 1).await.unwrap().result,
        Ok(vec![vec![3, 2, 8, 160, 134, 1, 0]])
    );
    let mut other = s.clone();
    for change_title in [true, false] {
        if change_title {
            other.title_id = 999;
        } else {
            other = session(2);
        }
        assert_eq!(
            server.handle(&external, &other, 1).await.unwrap().result,
            Ok(vec![vec![3, 2, 8, 0, 0, 0, 0]])
        );
        let misplaced = server
            .accounts
            .profile_directory(other.title_id, other.user_id)
            .await
            .unwrap()
            .join("wallet.json");
        tokio::fs::copy(private_path.with_file_name("wallet.json"), misplaced)
            .await
            .unwrap();
        assert!(server.handle(&external, &other, 1).await.is_err());
    }
    let backup2 = server
        .boost_test_account(s.title_id, s.user_id, 7, 8)
        .await
        .unwrap();
    let previous: LocalWallet =
        serde_json::from_slice(&tokio::fs::read(backup2.join("wallet.json")).await.unwrap())
            .unwrap();
    assert_eq!((previous.cp, previous.gp), (100_000, 1_000_000));
    let before = tokio::fs::read(&private_path).await.unwrap();
    tokio::fs::write(private_path.with_file_name("wallet.json"), b"{}")
        .await
        .unwrap();
    assert!(
        server
            .boost_test_account(s.title_id, s.user_id, 1, 1)
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(private_path).await.unwrap(), before);
}
#[tokio::test]
async fn initial_coupon_and_balance_queries_have_distinct_verified_result_shapes() {
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(1);
    let coupons = [80, 3, 173, 16, 0, 6, 188, 2, 0];
    let balance = [93, 3, 1, 8, 1, 0, 0, 0, 0];
    assert_eq!(
        server.handle(&coupons, &s, 1).await.unwrap().result,
        Ok(vec![])
    );
    assert_eq!(
        server.handle(&balance, &s, 1).await.unwrap().result,
        Ok(vec![vec![3, 2, 8, 0, 0, 0, 0]])
    );
    for body in [&coupons[..], &balance[..]] {
        for size in 0..body.len() {
            assert!(server.handle(&body[..size], &s, 1).await.is_err());
        }
        let mut extra = body.to_vec();
        extra.push(0);
        assert!(server.handle(&extra, &s, 1).await.is_err());
    }
    let mut zero = coupons;
    zero[6] = 0;
    zero[7] = 0;
    assert_eq!(
        server.handle(&zero, &s, 1).await.unwrap().result,
        Err(task::TASK_FAILED)
    );
    let mut other = balance;
    other[4] = 2;
    assert_eq!(
        server.handle(&other, &s, 1).await.unwrap().result,
        Err(task::TASK_FAILED)
    );
    assert_eq!(
        server.handle(&[80, 3, 172, 0], &s, 1).await.unwrap().result,
        Err(task::TASK_FAILED)
    );
}
fn session(uid: u64) -> Session {
    Session {
        title_id: 1002,
        user_id: uid,
        iv_seed: 1,
        expires_at: u32::MAX,
        key: [0; 24],
    }
}
fn read(name: &str, uid: u64) -> Vec<u8> {
    let mut w = Writer(vec![10, 3, 12]);
    w.string("");
    w.string(name);
    w.u64(uid);
    w.string("");
    w.0.push(0);
    w.0
}
fn upload(name: &str, data: &[u8]) -> Vec<u8> {
    let mut w = Writer(vec![10, 3, 10]);
    w.string("");
    w.string(name);
    w.boolean(true);
    w.blob(data).unwrap();
    w.u64(0);
    w.string("");
    w.u32(0);
    w.0.push(0);
    w.0
}
#[tokio::test]
async fn upload_survives_restart_preserves_exact_bytes_and_isolates_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let s = session(u64::MAX);
    let server = LocalServices::open(root.clone()).await.unwrap();
    let request = read("the.json", s.user_id);
    assert_eq!(
        server.handle(&request, &s, 10).await.unwrap().result,
        Err(task::NO_FILE)
    );
    let data = b"{\"blacklist\":[],\"Recentmetlist\":[]}\0";
    let stored = server
        .handle(&upload("the.json", data), &s, 11)
        .await
        .unwrap()
        .result
        .unwrap();
    let mut expected = Writer::default();
    expected.u32(data.len() as u32);
    expected.u32(11);
    expected.u32(11);
    expected.boolean(true);
    expected.u64(s.user_id);
    expected.string("");
    expected.string("the.json");
    assert_eq!(stored, [expected.0]);
    drop(server);
    let server = LocalServices::open(root).await.unwrap();
    let mut blob = Writer::default();
    blob.blob(data).unwrap();
    assert_eq!(
        server
            .handle(&request, &s, 12)
            .await
            .unwrap()
            .result
            .unwrap(),
        [blob.0]
    );
    assert_eq!(
        server
            .handle(&request, &session(1), 12)
            .await
            .unwrap()
            .result,
        Err(task::TASK_FAILED)
    );
    assert_eq!(
        server
            .handle(&read("the.json", 1), &session(1), 12)
            .await
            .unwrap()
            .result,
        Err(task::NO_FILE)
    );
    let mut different_title = s.clone();
    different_title.title_id = 1;
    assert_eq!(
        server
            .handle(&request, &different_title, 12)
            .await
            .unwrap()
            .result,
        Err(task::NO_FILE)
    );
    let updated = server
        .handle(&upload("the.json", b"new"), &s, 20)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(&updated[0][5..15], &[8, 11, 0, 0, 0, 8, 20, 0, 0, 0]);
}
#[tokio::test]
async fn rejects_paths_corrupt_files_and_unsupported_tasks_without_success() {
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(1);
    for name in ["../secret", "/etc/passwd", "a\\b", ".hidden", ""] {
        assert_eq!(
            server
                .handle(&upload(name, b"x"), &s, 1)
                .await
                .unwrap()
                .result,
            Err(task::TASK_FAILED)
        );
    }
    assert_eq!(
        server.handle(&[80, 3, 69, 0], &s, 1).await.unwrap().result,
        Err(task::TASK_FAILED)
    );
    let path = server.accounts.user_path(&s, "the.json").await.unwrap();
    tokio::fs::write(&path, b"broken").await.unwrap();
    assert!(
        server
            .handle(&upload("the.json", b"x"), &s, 1)
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(path).await.unwrap(), b"broken");
    let mut request = Writer(vec![10, 3, 21]);
    request.string("");
    request.string("configs_s.info");
    request.0.push(0);
    assert_eq!(
        server.handle(&request.0, &s, 1).await.unwrap().result,
        Err(task::NO_FILE)
    );
    tokio::fs::write(
        server.accounts.root.join("publisher/configs_s.info"),
        vec![0; task::MAX_FILE_BYTES + 1],
    )
    .await
    .unwrap();
    assert!(server.handle(&request.0, &s, 1).await.is_err());
}

#[tokio::test]
async fn publisher_listing_matches_served_bytes_and_has_stable_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(1);
    let encrypted = codol_protocol::lobby::publisher::crypt_configs(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/publisher/configs_s.ini"
    )))
    .unwrap();
    tokio::fs::write(
        server.accounts.root.join("publisher/configs_s.info"),
        &encrypted,
    )
    .await
    .unwrap();
    let query = [10, 3, 20, 16, 0, 8, 0, 0, 0, 0, 6, 32, 0, 6, 0, 0, 0];
    let first = server.handle(&query, &s, 1).await.unwrap().result.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(
        first,
        server
            .handle(&query, &s, 999)
            .await
            .unwrap()
            .result
            .unwrap()
    );
    // Read in the exact order of target deserializer VA 0xa5b230.
    let mut record = first[0].clone();
    record.push(0);
    let mut r = task::Reader::new(&record);
    assert_eq!(r.u32().unwrap(), encrypted.len() as u32);
    assert_ne!(r.u64().unwrap(), 0);
    let created = r.u32().unwrap();
    let modified = r.u32().unwrap();
    assert_eq!(created, modified);
    assert_ne!(modified, 0);
    assert!(r.boolean().unwrap());
    assert_eq!(r.u64().unwrap(), 0);
    assert_eq!(r.string(128).unwrap(), "configs_s.info");
    assert_eq!(r.string(50).unwrap(), "");
    r.finish().unwrap();
    let mut get = Writer(vec![10, 3, 21]);
    get.string("");
    get.string("configs_s.info");
    get.0.push(0);
    let records = server.handle(&get.0, &s, 1).await.unwrap().result.unwrap();
    let mut raw = records[0].clone();
    raw.push(0);
    let mut r = task::Reader::new(&raw);
    assert_eq!(r.blob().unwrap(), encrypted);
    r.finish().unwrap();
    let mut unsupported = query;
    unsupported[14] = 1;
    assert_eq!(
        server.handle(&unsupported, &s, 1).await.unwrap().result,
        Err(task::TASK_FAILED)
    );
    for n in 0..query.len() {
        assert!(server.handle(&query[..n], &s, 1).await.is_err());
    }
}

#[tokio::test]
async fn only_authenticated_profile_lookup_gets_target_missing_profile_error() {
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(u64::MAX);
    let mut public = Writer(vec![8, 3, 1]);
    public.u64(s.user_id);
    public.0.push(0);
    assert_eq!(
        server.handle(&public.0, &s, 1).await.unwrap().result,
        Err(task::NO_PROFILE)
    );
    assert_eq!(
        server.handle(&[8, 3, 2, 0], &s, 1).await.unwrap().result,
        Err(task::NO_PROFILE)
    );
    assert_eq!(
        server
            .handle(&public.0, &session(1), 1)
            .await
            .unwrap()
            .result,
        Err(task::TASK_FAILED)
    );
    assert!(server.handle(&[8, 3, 2, 0, 0], &s, 1).await.is_err());
}

#[tokio::test]
async fn public_profile_persists_restarts_and_rejects_bad_crc_without_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let server = LocalServices::open(root.clone()).await.unwrap();
    let s = session(u64::MAX);
    let arguments = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/public-profile-synthetic.bin"
    ));
    let mut upload = vec![8, 3, 3];
    upload.extend_from_slice(arguments);
    assert!(
        server
            .handle(&upload, &s, 1)
            .await
            .unwrap()
            .result
            .unwrap()
            .is_empty()
    );
    drop(server);
    let server = LocalServices::open(root).await.unwrap();
    let mut get = Writer(vec![8, 3, 1]);
    get.u64(s.user_id);
    get.0.push(0);
    let expected = profile::public_record(s.user_id, arguments).unwrap();
    assert_eq!(
        server
            .handle(&get.0, &s, 2)
            .await
            .unwrap()
            .result
            .unwrap()
            .as_slice(),
        std::slice::from_ref(&expected)
    );
    upload[14] ^= 1;
    assert!(server.handle(&upload, &s, 3).await.is_err());
    assert_eq!(
        server.handle(&get.0, &s, 4).await.unwrap().result.unwrap(),
        [expected]
    );
    assert_eq!(
        server.handle(&get.0, &session(1), 4).await.unwrap().result,
        Err(task::TASK_FAILED)
    );
    let mut other = Writer(vec![8, 3, 1]);
    other.u64(1);
    other.0.push(0);
    assert_eq!(
        server
            .handle(&other.0, &session(1), 4)
            .await
            .unwrap()
            .result,
        Err(task::NO_PROFILE)
    );
}
#[tokio::test]
async fn private_profile_is_bound_to_session_and_survives_restart() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let server = LocalServices::open(root.clone()).await.unwrap();
    let s = session(u64::MAX);
    let data = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/private-profile-synthetic.bin"
    ));
    let path = server.accounts.profile_path(&s, true).await.unwrap();
    atomic_private_write(&path, data).await.unwrap();
    drop(server);
    let server = LocalServices::open(root).await.unwrap();
    let record = profile::private_record(s.user_id, data).unwrap();
    assert_eq!(
        server
            .handle(&[8, 3, 2, 0], &s, 1)
            .await
            .unwrap()
            .result
            .unwrap(),
        [record]
    );
    assert_eq!(
        server
            .handle(&[8, 3, 2, 0], &session(1), 1)
            .await
            .unwrap()
            .result,
        Err(task::NO_PROFILE)
    );
    let mut other_title = s.clone();
    other_title.title_id = 2;
    assert_eq!(
        server
            .handle(&[8, 3, 2, 0], &other_title, 1)
            .await
            .unwrap()
            .result,
        Err(task::NO_PROFILE)
    );
    let mut damaged = data.to_vec();
    damaged[11] ^= 1;
    tokio::fs::write(path, damaged).await.unwrap();
    assert!(server.handle(&[8, 3, 2, 0], &s, 1).await.is_err());
}
#[tokio::test]
async fn item_grants_require_the_authenticated_identity_and_sample() {
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(1);
    let request = [80, 3, 230, 16, 0, 0];
    let path = server
        .accounts
        .profile_path(&s, false)
        .await
        .unwrap()
        .with_file_name("inventory.json");
    let data=serde_json::to_vec(&serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"title_id":s.title_id,"user_id":"1","items":[792,10001]})).unwrap();
    atomic_private_write(&path, &data).await.unwrap();
    let reply = server.handle(&request, &s, 1).await.unwrap();
    assert_eq!(reply.label, "local_player_inventory");
    let records = reply.result.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(&records[0][16..21], &[8, 2, 0, 0, 0]);
    let other = session(2);
    let other_path = server
        .accounts
        .profile_path(&other, false)
        .await
        .unwrap()
        .with_file_name("inventory.json");
    atomic_private_write(&other_path, &data).await.unwrap();
    assert!(server.handle(&request, &other, 1).await.is_err());
    tokio::fs::write(&path, b"invalid").await.unwrap();
    assert!(server.handle(&request, &s, 1).await.is_err());
}

#[tokio::test]
async fn full_catalogue_inventory_loads_and_refreshes_without_truncation() {
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(1);
    let path = server
        .accounts
        .profile_path(&s, false)
        .await
        .unwrap()
        .with_file_name("inventory.json");
    let items: Vec<u32> = (1..=6772).collect();
    let data = serde_json::to_vec_pretty(&serde_json::json!({
        "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
        "title_id":s.title_id,"user_id":"1","items":items
    }))
    .unwrap();
    assert!(data.len() > 32768);
    atomic_private_write(&path, &data).await.unwrap();
    let full = server
        .handle(&[80, 3, 230, 16, 0, 0], &s, 1)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(full, vec![task::player_data(s.user_id, &items).unwrap()]);
    let mut query = Writer(vec![80, 3, 203]);
    query.string("");
    query.u16(20000);
    query.0.push(0);
    let refreshed = server
        .handle(&query.0, &s, 1)
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(refreshed.len(), 6772);
    assert_eq!(
        refreshed,
        task::inventory_records(s.user_id, &items).unwrap()
    );
    assert!(task::reply(1, 203, Ok(&refreshed)).unwrap().len() > 65536);
}

#[tokio::test]
async fn empty_lab_catalogue_only_answers_supported_unfiltered_queries() {
    let temp = tempfile::tempdir().unwrap();
    let server = LocalServices::open(temp.path().join("data")).await.unwrap();
    let s = session(1);
    let mut query = Writer(vec![80, 3, 70]);
    query.string("");
    query.u32(1);
    query.u32(2000);
    query.boolean(true);
    query.u32(0);
    query.u32(0);
    query.string("982808");
    query.0.push(0);
    assert_eq!(
        server.handle(&query.0, &s, 1).await.unwrap().result,
        Ok(vec![])
    );
    let mut links = Writer(vec![80, 3, 107]);
    links.string("");
    links.u32(1);
    links.u32(2000);
    links.string("982808");
    links.0.push(0);
    assert_eq!(
        server.handle(&links.0, &s, 1).await.unwrap().result,
        Ok(vec![])
    );
    for n in 0..links.0.len() {
        assert!(server.handle(&links.0[..n], &s, 1).await.is_err());
    }
    for n in 0..query.0.len() {
        assert!(server.handle(&query.0[..n], &s, 1).await.is_err());
    }
    let mut page = query.0.clone();
    page[6] = 2;
    assert_eq!(
        server.handle(&page, &s, 1).await.unwrap().result,
        Err(task::TASK_FAILED)
    );
    let mut filter = query.0;
    filter[18] = 1;
    assert_eq!(
        server.handle(&filter, &s, 1).await.unwrap().result,
        Err(task::TASK_FAILED)
    );
}
