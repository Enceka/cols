use super::*;

#[tokio::test]
async fn new_players_are_isolated_with_selected_level_and_ownership_and_no_inherited_progress() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("publisher")).unwrap();
    for name in ["configs_s.info", "mp_playlists_shipRetail_release.info"] {
        std::fs::write(source.join("publisher").join(name), b"fixture").unwrap();
    }
    std::fs::write(source.join("do-not-copy"), b"existing account data").unwrap();
    let template = temp.path().join("template.json");
    let public = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/public-profile-synthetic.bin"
    ));
    std::fs::write(&template, serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"native_user_id":"42",
        "public_base64":STANDARD.encode(public),"starter_items":[10001],"all_items":[10001,10049,792]
    })).unwrap()).unwrap();
    let parent = temp.path().join("players");
    let mut directories = Vec::new();
    for (level, unlock_all) in [(1, false), (37, true), (70, false)] {
        let directory = create(
            &source,
            &template,
            &parent,
            1002,
            Settings {
                nickname: "同名玩家".into(),
                level,
                unlock_all,
            },
        )
        .await
        .unwrap();
        let root = directory.join("server-data");
        let id = directory.file_name().unwrap().to_str().unwrap();
        let login_name = format!("c{}", &id[..14]);
        let user_id = codol_protocol::auth::local_identity::user_id(&login_name).unwrap();
        let store = crate::infra::storage::accounts::AccountStore::open(root.clone())
            .await
            .unwrap();
        let account = store.profile_directory(1002, user_id).await.unwrap();
        let private = std::fs::read(account.join("private.bin")).unwrap();
        assert_eq!(
            profile::player_rank(&private, user_id).unwrap(),
            (level - 1, 0)
        );
        assert_eq!(std::fs::read(account.join("public.bin")).unwrap(), public);
        let inventory: serde_json::Value =
            serde_json::from_slice(&std::fs::read(account.join("inventory.json")).unwrap())
                .unwrap();
        assert_eq!(
            inventory["items"].as_array().unwrap().len(),
            if unlock_all { 3 } else { 1 }
        );
        assert_eq!(
            nickname(&root, 1002, user_id).await.unwrap().as_deref(),
            Some("同名玩家")
        );
        assert!(nickname(&root, 1002, 43).await.unwrap().is_none());
        let wallet: serde_json::Value =
            serde_json::from_slice(&std::fs::read(account.join("wallet.json")).unwrap()).unwrap();
        assert_eq!(wallet["gp"], 0);
        assert!(!account.join("progression.json").exists());
        assert!(!root.join("do-not-copy").exists());
        directories.push(directory);
    }
    assert!(directories.windows(2).all(|d| d[0] != d[1]));
    assert_eq!(
        std::fs::read(source.join("do-not-copy")).unwrap(),
        b"existing account data"
    );
    let invalid = Settings {
        nickname: "bad\nname".into(),
        level: 1,
        unlock_all: false,
    };
    assert!(
        create(&source, &template, &parent, 1002, invalid)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 3);
    std::fs::remove_file(source.join("publisher/configs_s.info")).unwrap();
    assert!(
        create(
            &source,
            &template,
            &parent,
            1002,
            Settings {
                nickname: "Player".into(),
                level: 1,
                unlock_all: false
            }
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 3);
}
