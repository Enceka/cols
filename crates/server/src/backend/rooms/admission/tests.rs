use super::*;
fn session() -> Session {
    Session {
        title_id: 1002,
        user_id: u64::MAX - 1,
        iv_seed: 41,
        expires_at: 100,
        key: [0; 24],
    }
}
fn command(text: &str) -> Vec<u8> {
    let mut w = Writer::default();
    w.u64(1_000_001);
    let mut data = (text.len() as u32).to_le_bytes().to_vec();
    data.extend_from_slice(text.as_bytes());
    w.blob(&data).unwrap();
    w.0.push(0);
    w.0
}
fn admissions() -> GameAdmissions {
    GameAdmissions {
        address: "127.0.0.1:39004".parse().unwrap(),
        entries: Mutex::new(HashMap::new()),
        bot_rooms: None,
        human_fill: None,
        story_capture_map: None,
    }
}
#[test]
fn booking_is_session_bound_and_start_is_required_for_udp_admission() {
    let a = admissions();
    let s = session();
    let (id, key) = a.allocate(&s, [7; 16], -688906341, 1).unwrap();
    assert_eq!(key, [7; 16]);
    assert_eq!(a.allocate(&s, key, -688906341, 2).unwrap().0, id);
    let start = command("mmstart sv_resetServer; playlist 0");
    assert!(a.control(&start, &s, 2).is_err());
    let book = command(&format!("mmbook 4 {} 0", STANDARD.encode(id)));
    let mut other = s.clone();
    other.user_id -= 1;
    assert!(a.control(&book, &other, 2).is_err());
    other = s.clone();
    other.title_id += 1;
    assert!(a.control(&book, &other, 2).is_err());
    other = s.clone();
    other.iv_seed += 1;
    assert!(a.control(&book, &other, 2).is_err());
    let reply = a.control(&book, &s, 2).unwrap();
    let mut r = Reader::new(&reply.pushes[0]);
    assert_eq!(r.u32().unwrap(), 21);
    assert_eq!(r.u64().unwrap(), 1_000_001);
    r.string(64).unwrap();
    assert_eq!(
        r.blob().unwrap(),
        [15u32.to_le_bytes().as_slice(), b"mmbook accepted"].concat()
    );
    assert!(a.get(&id, 2).is_none());
    assert!(a.control(&start, &s, 2).unwrap().result.is_ok());
    assert!(a.get(&id, 2).is_some());
    assert!(a.control(&book, &s, 100).is_err());
    assert!(a.get(&id, 100).is_none());
    a.revoke(&s);
    assert!(a.get(&id, 2).is_none());
}
#[test]
fn key_rotation_and_new_session_revoke_the_old_allocation() {
    let a = admissions();
    let mut s = session();
    let (old, _) = a.allocate(&s, [7; 16], -688906341, 1).unwrap();
    let (new, _) = a.allocate(&s, [8; 16], -688906341, 1).unwrap();
    assert_ne!(old, new);
    assert!(!a.entries.lock().unwrap().contains_key(&old));
    s.iv_seed += 1;
    a.revoke_other_sessions(&s);
    assert!(a.entries.lock().unwrap().is_empty());
    let (next, _) = a.allocate(&s, [8; 16], -688906341, 1).unwrap();
    assert_ne!(next, new);
    assert!(a.allocate(&s, [8; 16], -688906341, 100).is_err());
}
#[test]
fn live_game_lease_crosses_original_deadline_without_reviving_stale_sessions() {
    let a = admissions();
    let mut s = session();
    s.expires_at = 10_000;
    let (id, _) = a.allocate(&s, [7; 16], -688906341, 1).unwrap();
    a.control(
        &command(&format!("mmbook 7 {} 0", STANDARD.encode(id))),
        &s,
        2,
    )
    .unwrap();
    a.control(&command("mmstart sv_resetServer; playlist 0"), &s, 2)
        .unwrap();
    for field in 0..3 {
        let mut other = s.clone();
        match field {
            0 => other.title_id += 1,
            1 => other.user_id += 1,
            _ => other.iv_seed += 1,
        }
        a.renew_session(&other, 900).unwrap();
        assert!(a.get(&id, 931).is_none());
    }
    for now in [900, 1800, 2700, 3600] {
        a.renew_session(&s, now).unwrap();
        assert!(a.get(&id, now + 929).is_some());
    }
    a.renew_session(&s, 4530).unwrap();
    assert!(a.get(&id, 4530).is_none()); // idle expiry cannot be undone
    a.revoke(&s);
    a.renew_session(&s, 4500).unwrap();
    assert!(a.entries.lock().unwrap().is_empty());
    assert!(a.renew_session(&s, 10_000).is_err());
}
