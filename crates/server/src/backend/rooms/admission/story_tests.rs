use super::*;

fn session() -> Session {
    Session {
        title_id: 1002,
        user_id: 42,
        iv_seed: 17,
        expires_at: 100,
        key: [0; 24],
    }
}
fn command(text: &str) -> Vec<u8> {
    let mut out = Writer::default();
    out.u64(1_000_001);
    out.blob(
        &[
            (text.len() as u32).to_le_bytes().as_slice(),
            text.as_bytes(),
        ]
        .concat(),
    )
    .unwrap();
    out.0.push(0);
    out.0
}
fn admissions() -> GameAdmissions {
    GameAdmissions {
        address: "127.0.0.1:39004".parse().unwrap(),
        entries: Default::default(),
        bot_rooms: None,
        story_capture_map: Some(codol_protocol::room::story::FIRST_MAP.into()),
    }
}
const START: &str =
    "mmstart sv_resetServer; 2486 1; 3265 1; g_gametype coop; map coop_training_mp_cargoship";

#[test]
fn random_story_key_retries_and_rotates_with_channel_and_login() {
    let a = admissions();
    let mut s = session();
    let (id, key) = a
        .allocate_story(&s, 7, codol_protocol::room::story::CLIENT_BUILD, 1)
        .unwrap();
    assert_ne!(key, [0; 16]);
    assert_ne!(key.as_slice(), &s.key[..16]);
    assert_eq!(
        a.allocate_story(&s, 7, codol_protocol::room::story::CLIENT_BUILD, 2)
            .unwrap(),
        (id, key)
    );
    assert!(a.allocate_story(&s, 7, 0, 2).is_err());
    assert_eq!(a.entries.lock().unwrap().len(), 1);
    let (next, next_key) = a
        .allocate_story(&s, 8, codol_protocol::room::story::CLIENT_BUILD, 2)
        .unwrap();
    assert_ne!((next, next_key), (id, key));
    assert!(!a.entries.lock().unwrap().contains_key(&id));
    s.iv_seed += 1;
    let (login, _) = a
        .allocate_story(&s, 8, codol_protocol::room::story::CLIENT_BUILD, 3)
        .unwrap();
    assert_ne!(login, next);
    assert!(!a.entries.lock().unwrap().contains_key(&next));
    assert!(
        a.allocate_story(&s, 8, codol_protocol::room::story::CLIENT_BUILD, 100)
            .is_err()
    );
    a.revoke(&s);
    assert!(a.entries.lock().unwrap().is_empty());
}

#[test]
fn story_booking_is_authenticated_and_rejected_starts_do_not_replace_the_binding() {
    let a = admissions();
    let s = session();
    let (id, _) = a
        .allocate_story(&s, 7, codol_protocol::room::story::CLIENT_BUILD, 1)
        .unwrap();
    assert!(a.control(&command(START), &s, 2).is_err());
    let booking = command(&format!("mmbook 7 {} 0", STANDARD.encode(id)));
    let mut other = s.clone();
    other.user_id += 1;
    assert!(a.control(&booking, &other, 2).is_err());
    a.control(&booking, &s, 2).unwrap();
    a.control(&command(START), &s, 2).unwrap();
    assert!(
        a.control(
            &command(&START.replace("coop_training_mp_cargoship", "mp_afghan")),
            &s,
            2
        )
        .is_err()
    );
    assert_eq!(a.get(&id, 2).unwrap().start_command.as_deref(), Some(START));
    a.allocate(&s, [9; 16], codol_protocol::room::story::CLIENT_BUILD, 3)
        .unwrap();
    assert!(a.get(&id, 3).is_none());
}
