use super::*;

#[tokio::test]
async fn certificate_reuses_identity_and_never_exposes_private_key() {
    let dir = tempfile::tempdir().unwrap();
    let cert = keys::ensure_identity(dir.path(), false).await.unwrap();
    let initial = serde_json::to_string(&cert).unwrap();
    assert!(!initial.contains("PRIVATE KEY") && !initial.contains("key.pem"));
    assert!(cert.pem.contains("BEGIN CERTIFICATE"));
    let parsed = openssl::x509::X509::from_pem(cert.pem.as_bytes()).unwrap();
    assert!(parsed.verify(&parsed.public_key().unwrap()).unwrap());
    let san = parsed.subject_alt_names().unwrap();
    assert!(san.iter().any(|value| value.dnsname() == Some("localhost")));
    assert!(
        san.iter()
            .any(|value| value.ipaddress() == Some(&[127, 0, 0, 1]))
    );
    assert_eq!(
        cert.fingerprint,
        keys::ensure_identity(dir.path(), false)
            .await
            .unwrap()
            .fingerprint
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            cert.private_key.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let old_key = std::fs::read(&cert.private_key).unwrap();
    std::fs::write(&cert.private_key, b"invalid existing key").unwrap();
    assert!(keys::ensure_identity(dir.path(), false).await.is_err());
    assert_eq!(
        std::fs::read(&cert.private_key).unwrap(),
        b"invalid existing key"
    );
    std::fs::write(&cert.private_key, old_key).unwrap();
    assert_ne!(
        cert.fingerprint,
        keys::ensure_identity(dir.path(), true)
            .await
            .unwrap()
            .fingerprint
    );
}

#[tokio::test]
async fn settings_preserve_profiles_and_existing_publisher_and_exclude_duplicate_managers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("control");
    let owner = Hosting::open(root.clone(), "127.0.0.1:39080".parse().unwrap())
        .await
        .unwrap();
    assert!(
        Hosting::open(root, "127.0.0.1:39081".parse().unwrap())
            .await
            .is_err()
    );
    let snapshot = owner.snapshot().await;
    let mut settings: Settings = serde_json::from_value(snapshot["settings"].clone()).unwrap();
    settings.service_data_dir = dir.path().join("accounts");
    let publisher = settings.service_data_dir.join("publisher");
    std::fs::create_dir_all(&publisher).unwrap();
    std::fs::write(publisher.join("configs_s.info"), b"operator-owned-config").unwrap();
    std::fs::create_dir_all(settings.service_data_dir.join("profiles")).unwrap();
    std::fs::write(
        settings.service_data_dir.join("profiles/keep"),
        b"player-data",
    )
    .unwrap();
    let changed = owner
        .save(settings.clone(), snapshot["revision"].as_str().unwrap())
        .await
        .unwrap();
    assert!(
        owner
            .save(settings.clone(), snapshot["revision"].as_str().unwrap())
            .await
            .is_err()
    );
    assert!(owner.start().await.is_err()); // Missing maps must not silently create a fake world.
    assert!(!owner.root.join("identity.json").exists());
    owner.prepare(&settings).await.unwrap();
    assert_eq!(
        std::fs::read(publisher.join("configs_s.info")).unwrap(),
        b"operator-owned-config"
    );
    assert_eq!(
        std::fs::read(settings.service_data_dir.join("profiles/keep")).unwrap(),
        b"player-data"
    );
    assert!(
        publisher
            .join("mp_playlists_shipRetail_release.info")
            .is_file()
    );
    let mut wrong = settings.clone();
    wrong.auth_port = wrong.lobby_port;
    assert!(
        owner
            .save(wrong, changed["revision"].as_str().unwrap())
            .await
            .is_err()
    );
    wrong = settings;
    wrong.auth_port = 39080;
    assert!(wrong.validate(39080).is_err());
}
