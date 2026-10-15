use super::*;
use codol_protocol::room::messages::bot_ready;
use codol_protocol::room::messages::roster_request;

fn actor(user: u64) -> Session {
    Session {
        title_id: 1002,
        user_id: user,
        iv_seed: user as u32,
        expires_at: 1000,
        key: [0; 24],
    }
}
fn call(rooms: &mut Rooms, body: &[u8], actor: &Session, now: u32) -> TaskResult {
    rooms
        .handle(&Request::parse(body).unwrap(), actor, now)
        .unwrap()
}
pub(super) fn room(rooms: &mut Rooms, owner: &Session, capacity: u32) -> (u64, [u8; 8]) {
    let result = call(rooms, &[11, 3, 13, 0], owner, 1).result.unwrap();
    let id = Reader::new(&result[0]).u64().unwrap();
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/room-browser-synthetic.json"
    )))
    .unwrap();
    let hex = fixture["listing"]["create_arguments"].as_str().unwrap();
    let encoded: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let mut source = Reader::new(&encoded);
    let address = source.blob().unwrap();
    let flags = source.u32().unwrap();
    source.u32().unwrap();
    let name = source.string(65).unwrap();
    source.u64().unwrap();
    source.string(65).unwrap();
    let mut w = Writer(vec![21, 3, 1]);
    w.blob(address).unwrap();
    w.u32(flags);
    w.u32(capacity);
    w.string(name);
    w.u64(owner.user_id);
    w.string(&format!("Host {}", owner.user_id));
    w.0.extend(source.remaining());
    let end = w.0.len();
    w.0[end - 19..end - 11].copy_from_slice(&id.to_le_bytes());
    let result = call(rooms, &w.0, owner, 1).result.unwrap();
    (
        id,
        Reader::new(&result[0]).blob().unwrap().try_into().unwrap(),
    )
}
fn search(paged: bool, initial: bool, cursor: [u8; 8], maximum: u32) -> Vec<u8> {
    let mut w = Writer(vec![21, 3, if paged { 13 } else { 5 }]);
    w.u32(4);
    if paged {
        w.boolean(initial);
        w.blob(&cursor).unwrap();
    } else {
        w.u32(0);
    }
    w.u32(maximum);
    for value in [-688906341, 0, 16, 0] {
        w.i32(value);
    }
    w.0.push(0);
    w.0
}
fn join(id: u64) -> Vec<u8> {
    let mut w = Writer(vec![11, 3, 11]);
    w.u64(id);
    w.u64(0);
    w.0.push(0);
    w.0
}
fn members(id: u64) -> Vec<u8> {
    let mut w = Writer(vec![11, 3, 15]);
    w.u64(id);
    w.u32(100);
    w.0.push(0);
    w.0
}
fn send(id: u64, recipient: Option<u64>, batch: &[u8]) -> Vec<u8> {
    let mut w = Writer(vec![11, 3, if recipient.is_some() { 5 } else { 4 }]);
    w.u64(id);
    if let Some(uid) = recipient {
        w.u64(uid);
    }
    w.blob(batch).unwrap();
    w.u32(1);
    w.0.push(0);
    w.0
}
fn leave(id: u64) -> Vec<u8> {
    let mut w = Writer(vec![11, 3, 2]);
    w.u64(id);
    w.0.push(0);
    w.0
}
fn result_id(record: &[u8]) -> [u8; 8] {
    let mut r = Reader::new(record);
    r.blob().unwrap();
    r.blob().unwrap().try_into().unwrap()
}
fn population(record: &[u8]) -> u32 {
    let mut r = Reader::new(record);
    r.blob().unwrap();
    r.blob().unwrap();
    r.u32().unwrap();
    r.u32().unwrap();
    r.u32().unwrap()
}

#[test]
fn registered_room_is_browsable_and_join_metadata_precedes_peer_messages() {
    let mut rooms = Rooms::default();
    let owner = actor(42);
    let guest = actor(43);
    let (id, sid) = room(&mut rooms, &owner, 12);
    let result = call(&mut rooms, &search(false, true, [0; 8], 20), &guest, 2)
        .result
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result_id(&result[0]), sid);
    assert_eq!(population(&result[0]), 1);
    let joined = call(&mut rooms, &join(id), &guest, 2);
    assert!(joined.result.is_ok());
    assert_eq!(joined.pushes.len(), 1);
    assert_eq!(joined.deliveries.len(), 1);
    assert_eq!(joined.deliveries[0].target, Identity::from(&owner));
    let mut r = Reader::new(&joined.deliveries[0].body);
    assert_eq!(r.u32().unwrap(), 9);
    assert_eq!(r.u64().unwrap(), 43);
    assert_eq!(r.string(64).unwrap(), "NDM=");
    assert_eq!(r.u64().unwrap(), id);
    assert_eq!(r.u8().unwrap(), 2);
    assert_eq!(r.u32().unwrap(), 0);
    assert_eq!(r.u64().unwrap(), id);
    assert_eq!(r.u32().unwrap(), 2);
    let retry = call(&mut rooms, &join(id), &guest, 3);
    assert!(retry.result.is_ok());
    assert!(retry.pushes.is_empty() && retry.deliveries.is_empty());
    let list = call(&mut rooms, &members(id), &guest, 3).result.unwrap();
    assert_eq!(list.len(), 2);
    let mut owner_record = Reader::new(&list[0]);
    assert_eq!(owner_record.u64().unwrap(), 42);
    assert_eq!(owner_record.string(64).unwrap(), "SG9zdCA0Mg==");
    let mut foreign = guest.clone();
    foreign.iv_seed += 1;
    assert!(call(&mut rooms, &members(id), &foreign, 3).result.is_err());
    let result = call(&mut rooms, &search(false, true, [0; 8], 20), &guest, 3)
        .result
        .unwrap();
    assert_eq!(population(&result[0]), 2);
}

#[test]
fn cursor_is_bound_to_requester_and_snapshot_survives_deleted_last_record() {
    let mut rooms = Rooms::default();
    let guest = actor(100);
    let mut owned = Vec::new();
    for uid in 10..14 {
        let owner = actor(uid);
        let (id, sid) = room(&mut rooms, &owner, 12);
        owned.push((owner, id, sid));
    }
    let first = call(&mut rooms, &search(true, true, [9; 8], 2), &guest, 2)
        .result
        .unwrap();
    assert_eq!(first.len(), 2);
    let cursor = result_id(&first[1]);
    let (owner, id, _) = owned.iter().find(|(_, _, sid)| *sid == cursor).unwrap();
    assert!(call(&mut rooms, &leave(*id), owner, 3).result.is_ok());
    room(&mut rooms, &actor(99), 12);
    let second = call(&mut rooms, &search(true, false, cursor, 2), &guest, 3)
        .result
        .unwrap();
    assert_eq!(second.len(), 2);
    let end = result_id(&second[1]);
    assert!(
        call(&mut rooms, &search(true, false, end, 2), &guest, 3)
            .result
            .unwrap()
            .is_empty()
    );
    assert!(
        call(&mut rooms, &search(true, false, cursor, 2), &actor(101), 3)
            .result
            .is_err()
    );
    assert!(
        call(&mut rooms, &search(true, false, [0; 8], 2), &guest, 3)
            .result
            .is_err()
    );
    assert!(
        call(&mut rooms, &search(true, false, cursor, 3), &guest, 3)
            .result
            .is_err()
    );
    assert!(
        call(&mut rooms, &search(true, false, cursor, 2), &guest, 63)
            .result
            .is_err()
    );
}

#[test]
fn browse_filters_title_and_build_and_rejects_unknown_matching_rules() {
    let mut rooms = Rooms::default();
    room(&mut rooms, &actor(42), 12);
    let mut other = actor(44);
    other.title_id = 2000;
    room(&mut rooms, &other, 12);
    let guest = actor(43);
    assert_eq!(
        call(&mut rooms, &search(false, true, [0; 8], 20), &guest, 2)
            .result
            .unwrap()
            .len(),
        1
    );
    let mut wrong = search(false, true, [0; 8], 20);
    wrong[19..23].copy_from_slice(&123i32.to_le_bytes());
    assert!(
        call(&mut rooms, &wrong, &guest, 2)
            .result
            .unwrap()
            .is_empty()
    );
    let mut unknown = search(false, true, [0; 8], 20);
    unknown[29..33].copy_from_slice(&17i32.to_le_bytes());
    assert!(
        call(&mut rooms, &unknown, &guest, 2)
            .result
            .unwrap()
            .is_empty()
    );
}

#[test]
fn admission_enforces_capacity_pending_deadline_and_current_member_identity() {
    let mut rooms = Rooms::default();
    let owner = actor(42);
    let guest = actor(43);
    let (id, _) = room(&mut rooms, &owner, 2);
    assert!(call(&mut rooms, &join(id), &guest, 2).result.is_ok());
    assert!(call(&mut rooms, &join(id), &actor(44), 2).result.is_err());
    let mut other_title = actor(44);
    other_title.title_id += 1;
    assert!(call(&mut rooms, &join(id), &other_title, 2).result.is_err());
    assert!(call(&mut rooms, &[11, 3, 13, 0], &guest, 2).result.is_err());
    let expired = rooms.reap(33);
    assert!(!expired.is_empty());
    assert_eq!(rooms.channels[&id].population(), 1);
    assert!(call(&mut rooms, &join(id), &actor(44), 33).result.is_ok());
    let mut changed = actor(44);
    changed.key[0] = 1;
    assert!(call(&mut rooms, &members(id), &changed, 33).result.is_err());
}

#[test]
fn host_reply_ready_cancel_roster_requests_and_guest_leave_use_real_targets() {
    let mut rooms = Rooms::default();
    let owner = actor(42);
    let guest = actor(43);
    let (id, _) = room(&mut rooms, &owner, 12);
    call(&mut rooms, &join(id), &guest, 2);
    // Only the JoinResponse status is inspected by the relay; remaining owner
    // payload is opaque and a native client still requires its complete RoomInfo.
    let accepted = [3, 0, 0, 0, 10, 0, 0];
    let reply = call(&mut rooms, &send(id, Some(43), &accepted), &owner, 3);
    assert!(reply.result.is_ok());
    assert!(reply.pushes.is_empty());
    assert_eq!(reply.deliveries.len(), 1);
    assert_eq!(reply.deliveries[0].target, Identity::from(&guest));
    assert!(rooms.channels[&id].humans[&43].pending_until.is_none());
    for body in [bot_ready(true), bot_ready(false), roster_request()] {
        let reply = call(&mut rooms, &send(id, None, &body), &guest, 4);
        assert!(reply.result.is_ok());
        assert_eq!(reply.pushes.len(), 1);
        assert_eq!(reply.deliveries.len(), 1);
        assert_eq!(reply.deliveries[0].target, Identity::from(&owner));
        let mut r = Reader::new(&reply.deliveries[0].body);
        assert_eq!(r.u32().unwrap(), 7);
        assert_eq!(r.u64().unwrap(), 43);
        assert_eq!(r.string(64).unwrap(), "NDM=");
        assert_eq!(r.u64().unwrap(), id);
        assert_eq!(r.blob().unwrap(), body);
    }
    assert!(rooms.reap(33).is_empty());
    assert!(
        rooms
            .handle(
                &Request::parse(&send(id, None, &[2, 0, 0, 0, 28, 0])).unwrap(),
                &guest,
                34
            )
            .is_err()
    );
    assert_eq!(rooms.channels[&id].population(), 2);
    let left = call(&mut rooms, &leave(id), &guest, 34);
    assert!(left.result.is_ok());
    assert_eq!(left.deliveries.len(), 1);
    assert_eq!(Reader::new(&left.deliveries[0].body).u32().unwrap(), 10);
    assert!(call(&mut rooms, &members(id), &guest, 35).result.is_err());
    assert_eq!(rooms.channels[&id].population(), 1);
}

#[test]
fn owner_disconnect_closes_room_and_never_republishes_old_ids() {
    let mut rooms = Rooms::default();
    let owner = actor(42);
    let guest = actor(43);
    let (id, _) = room(&mut rooms, &owner, 12);
    call(&mut rooms, &join(id), &guest, 2);
    let pushes = rooms.disconnect(Identity::from(&owner));
    assert_eq!(pushes.len(), 2);
    assert!(pushes.iter().all(|p| p.target == Identity::from(&guest)));
    assert!(rooms.channels.is_empty());
    assert!(
        call(&mut rooms, &search(false, true, [0; 8], 20), &guest, 3)
            .result
            .unwrap()
            .is_empty()
    );
    assert!(call(&mut rooms, &join(id), &guest, 3).result.is_err());
}

#[test]
fn owner_broadcast_join_response_also_confirms_a_pending_channel_member() {
    let mut rooms = Rooms::default();
    let owner = actor(42);
    let guest = actor(43);
    let (id, _) = room(&mut rooms, &owner, 12);
    call(&mut rooms, &join(id), &guest, 2);
    let approved = call(
        &mut rooms,
        &send(id, None, &[3, 0, 0, 0, 10, 0, 0]),
        &owner,
        3,
    );
    assert!(approved.result.is_ok());
    assert!(rooms.channels[&id].humans[&43].pending_until.is_none());
    assert!(rooms.reap(33).is_empty());
    assert_eq!(rooms.channels[&id].population(), 2);
}

#[test]
fn configured_bot_fill_uses_replaceable_seats_and_refills_after_a_human_leaves() {
    let fill = crate::backend::rooms::bots::FillPolicy {
        target: 4,
        policy: crate::backend::rooms::bots::Policy {
            version: 7,
            maps: std::collections::BTreeSet::from([("mp_afghan".into(), "war".into())]),
            loadouts: Default::default(),
        },
    };
    let mut admissions =
        crate::backend::rooms::admission::GameAdmissions::new("127.0.0.1:39004".parse().unwrap());
    admissions.human_fill = Some(fill.clone());
    let mut rooms = Rooms::with_game_endpoint(Arc::new(admissions));
    let owner = actor(42);
    let guest = actor(43);
    let (id, _) = room(&mut rooms, &owner, 12);
    rooms.channels.get_mut(&id).unwrap().room_info =
        Some(codol_protocol::room::messages::RoomInfo {
            mode: "war".into(),
            map: "mp_afghan".into(),
            playlist: 255,
            minimum: 4,
            maximum: 12,
        });
    let mut result = TaskResult::success("test", vec![]);
    super::fill::sync(
        rooms.channels.get_mut(&id).unwrap(),
        id,
        &fill,
        Identity::from(&owner),
        &mut result,
    )
    .unwrap();
    assert_eq!(rooms.channels[&id].population(), 4);
    assert_eq!(rooms.channels[&id].fill_bots.len(), 3);
    let listing = call(&mut rooms, &search(false, true, [0; 8], 20), &guest, 2)
        .result
        .unwrap();
    assert_eq!(population(&listing[0]), 1); // Only real seats are nonreplaceable.
    let joined = call(&mut rooms, &join(id), &guest, 2);
    assert!(joined.result.is_ok());
    assert_eq!(rooms.channels[&id].population(), 4);
    assert_eq!(rooms.channels[&id].fill_bots.len(), 2);
    let events: Vec<_> = joined
        .deliveries
        .iter()
        .map(|p| Reader::new(&p.body).u32().unwrap())
        .collect();
    assert_eq!(events, vec![10, 9]); // Vacate the Bot before announcing the human.
    call(&mut rooms, &leave(id), &guest, 3);
    super::fill::sync(
        rooms.channels.get_mut(&id).unwrap(),
        id,
        &fill,
        Identity::from(&owner),
        &mut result,
    )
    .unwrap();
    assert_eq!(rooms.channels[&id].population(), 4);
    assert_eq!(rooms.channels[&id].fill_bots.len(), 3);
}
