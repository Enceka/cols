use super::*;
fn session(uid: u64) -> Session {
    Session {
        title_id: 1002,
        user_id: uid,
        iv_seed: 1,
        expires_at: 100,
        key: [0; 24],
    }
}
#[test]
fn channel_membership_precedes_each_bot_join_and_ready() {
    let owner = session(42);
    let policy = crate::backend::rooms::bots::Policy {
        version: 7,
        maps: Default::default(),
        loadouts: Default::default(),
    };
    let pushes = bot_pushes(&policy, 123, &owner, false).unwrap();
    assert_eq!(pushes.len(), 33);
    for (index, events) in pushes.chunks_exact(3).enumerate() {
        let client = index as u8 + 1;
        let uid = codol_protocol::room::messages::bot_id(client);
        let name = STANDARD.encode(codol_protocol::room::messages::bot_name(client));
        if client == 1 {
            assert_eq!(name, "Qm90IDE=");
        }
        if client == 11 {
            assert_eq!(name, "Qm90IDEx");
        }
        // Independent tagged SDK decoder contract, 0xa3d730/0xa55a40.
        let mut r = Reader::new(&events[0]);
        assert_eq!(r.u32().unwrap(), 9);
        assert_eq!(r.u64().unwrap(), uid);
        assert_eq!(r.string(65).unwrap(), name);
        assert_eq!(r.u64().unwrap(), 123);
        assert_eq!(r.u8().unwrap(), 2);
        assert_eq!(r.u32().unwrap(), 0);
        assert_eq!(r.u64().unwrap(), 123);
        assert_eq!(r.u32().unwrap(), u32::from(client) + 1);
        assert_eq!(r.u32().unwrap(), 0);
        assert_eq!(r.u64().unwrap(), owner.user_id);
        assert!(!r.boolean().unwrap());
        assert!(!r.boolean().unwrap());
        assert_eq!(r.u32().unwrap(), 0);
        assert!(r.remaining().is_empty());
        for (event, kind) in events[1..].iter().zip([6, 16]) {
            let mut r = Reader::new(event);
            assert_eq!(r.u32().unwrap(), 7);
            assert_eq!(r.u64().unwrap(), uid);
            assert_eq!(r.string(65).unwrap(), name);
            assert_eq!(r.u64().unwrap(), 123);
            let batch = r.blob().unwrap();
            assert!(batch.len() <= 1024);
            assert_eq!(
                u32::from_le_bytes(batch[..4].try_into().unwrap()) as usize,
                batch.len() - 4
            );
            // Broadcast manifold: 1+8+32+1+32 bits before its child.
            assert_eq!(
                (u16::from_le_bytes(batch[13..15].try_into().unwrap()) >> 3) & 255,
                kind
            );
            assert_eq!(r.u32().unwrap(), 1);
            assert!(r.remaining().is_empty());
            if kind == 16 {
                assert_eq!(
                    codol_protocol::room::messages::updates(batch, owner.user_id)
                        .unwrap()
                        .ready,
                    Some(false)
                );
            }
        }
    }
}
fn create(rooms: &mut Rooms, s: &Session) -> (u64, Vec<u8>) {
    let reply = rooms
        .handle(&Request::parse(&[11, 3, 13, 0]).unwrap(), s, 1)
        .unwrap()
        .result
        .unwrap();
    let id = Reader::new(&reply[0]).u64().unwrap();
    (id, reply[0].clone())
}
#[test]
fn bot_avatars_wait_for_members_and_follow_team_map_and_frontend_refresh() {
    use codol_protocol::room::messages::Loadout;
    use codol_protocol::room::messages::Member;
    use codol_protocol::room::messages::RoomInfo;
    use codol_protocol::room::messages::Updates;
    use codol_protocol::room::messages::bot_id;
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/room-loadout-synthetic.json"
    )))
    .unwrap();
    let template: Loadout = serde_json::from_value(fixture["loadout"].clone()).unwrap();
    let roster = (1..=11)
        .map(|client| {
            let mut axis = template;
            let mut allies = template;
            axis.clothing[0] = 500 + client;
            allies.clothing[0] = 800 + client;
            [axis, allies]
        })
        .collect::<Vec<_>>();
    let policy = crate::backend::rooms::bots::Policy {
        version: 7,
        maps: [
            ("mp_afghan".into(), "war".into()),
            ("mp_killhouse_sh".into(), "war".into()),
        ]
        .into(),
        loadouts: [
            (("mp_afghan".into(), "war".into()), roster.clone()),
            (("mp_killhouse_sh".into(), "war".into()), roster.clone()),
        ]
        .into(),
    };
    let owner = session(0x8000000000000001);
    let mut rooms = Rooms::default();
    let (id, _) = create(&mut rooms, &owner);
    let channel = rooms.channels.get_mut(&id).unwrap();
    channel.game = Some(GameSession {
        id: [0; 8],
        description: Description::parse(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/protocol/room-create-synthetic.bin"
            )),
            owner.user_id,
        )
        .unwrap(),
    });
    channel.room_info = Some(RoomInfo {
        mode: "war".into(),
        map: "mp_afghan".into(),
        playlist: 255,
        minimum: 4,
        maximum: 12,
    });
    assert!(
        channel
            .sync_bot_loadouts(&policy, id, true)
            .unwrap()
            .is_empty()
    );
    channel.bots_announced = true;
    assert!(
        channel
            .sync_bot_loadouts(&policy, id, true)
            .unwrap()
            .is_empty()
    );
    channel
        .update_members(&Updates {
            members: (1..=11)
                .map(|client| Member {
                    id: bot_id(client),
                    team: if client % 2 == 0 { 2 } else { 1 },
                    ready: false,
                })
                .collect(),
            ..Default::default()
        })
        .unwrap();
    let pushes = channel.sync_bot_loadouts(&policy, id, false).unwrap();
    assert_eq!(pushes.len(), 11);
    for (client, push) in (1..=11).zip(&pushes) {
        let mut r = Reader::new(push);
        assert_eq!(r.u32().unwrap(), 7);
        assert_eq!(r.u64().unwrap(), bot_id(client));
        assert_eq!(
            r.string(65).unwrap(),
            STANDARD.encode(format!("Bot {client}"))
        );
        assert_eq!(r.u64().unwrap(), id);
        let team = if client % 2 == 0 { 2 } else { 1 };
        let expected = roster[usize::from(client - 1)][usize::from(2 - team)]
            .record(team)
            .unwrap();
        assert_eq!(
            r.blob().unwrap(),
            codol_protocol::room::messages::to_member(&expected, owner.user_id).unwrap()
        );
        assert_eq!(r.u32().unwrap(), 1);
        assert!(r.remaining().is_empty());
    }
    assert!(!channel.owner_ready);
    assert!(
        channel
            .sync_bot_loadouts(&policy, id, false)
            .unwrap()
            .is_empty()
    );
    channel.member_teams.insert(bot_id(1), 2);
    assert_eq!(
        channel.sync_bot_loadouts(&policy, id, false).unwrap().len(),
        1
    );
    assert_eq!(channel.avatar_sent[&1].3.clothing[0], 501);
    assert_eq!(
        channel.sync_bot_loadouts(&policy, id, true).unwrap().len(),
        11
    );
    channel.room_info.as_mut().unwrap().map = "mp_killhouse_sh".into();
    assert_eq!(
        channel.sync_bot_loadouts(&policy, id, false).unwrap().len(),
        11
    );
    channel.member_teams.remove(&bot_id(1));
    assert_eq!(
        channel.sync_bot_loadouts(&policy, id, true).unwrap().len(),
        10
    );
    assert!(!channel.avatar_sent.contains_key(&1));
    channel.room_info.as_mut().unwrap().map = "mp_unknown".into();
    assert!(
        channel
            .sync_bot_loadouts(&policy, id, true)
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn native_cancel_and_avatar_republish_resend_loadouts_after_the_owner_echo() {
    use codol_protocol::room::messages::Loadout;
    use codol_protocol::room::messages::RoomInfo;
    use codol_protocol::room::messages::bot_id;
    use codol_protocol::room::messages::bot_ready;
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/room-loadout-synthetic.json"
    )))
    .unwrap();
    let loadout: Loadout = serde_json::from_value(fixture["loadout"].clone()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let server = crate::transport::game::server::GameServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        temp.path().join("game"),
    )
    .await
    .unwrap();
    let mut endpoint = server.admissions();
    drop(server);
    Arc::get_mut(&mut endpoint).unwrap().bot_rooms = Some(crate::backend::rooms::bots::Policy {
        version: 7,
        maps: [("mp_afghan".into(), "war".into())].into(),
        loadouts: [(("mp_afghan".into(), "war".into()), vec![[loadout; 2]; 11])].into(),
    });
    let mut rooms = Rooms::with_game_endpoint(endpoint);
    let owner = session(0x8000000000000001);
    let (id, _) = create(&mut rooms, &owner);
    let channel = rooms.channels.get_mut(&id).unwrap();
    channel.game = Some(GameSession {
        id: [0; 8],
        description: Description::parse(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/protocol/room-create-synthetic.bin"
            )),
            owner.user_id,
        )
        .unwrap(),
    });
    channel.room_info = Some(RoomInfo {
        mode: "war".into(),
        map: "mp_afghan".into(),
        playlist: 255,
        minimum: 4,
        maximum: 12,
    });
    channel.bots_announced = true;
    for client in 1..=11 {
        channel
            .member_teams
            .insert(bot_id(client), if client % 2 == 0 { 2 } else { 1 });
    }
    let request = |record: &[u8]| {
        let mut w = Writer(vec![11, 3, 4]);
        w.u64(id);
        w.blob(record).unwrap();
        w.u32(1);
        w.0.push(0);
        w.0
    };
    for (record, expected, expected_ready, avatar_count) in [
        (bot_ready(false), 23, false, 11),
        (bot_ready(true), 12, true, 0),
        (bot_ready(false), 23, false, 11), // Native match return rebuilds frontend objects.
        (loadout.record(2).unwrap(), 12, false, 11),
    ] {
        let data = request(&record);
        let reply = rooms
            .handle(&Request::parse(&data).unwrap(), &owner, 1)
            .unwrap();
        assert_eq!(reply.pushes.len(), expected);
        assert_eq!(rooms.channels[&id].owner_ready, expected_ready);
        let mut echo = Reader::new(&reply.pushes[0]);
        assert_eq!(echo.u32().unwrap(), 7);
        assert_eq!(echo.u64().unwrap(), owner.user_id);
        let avatars = reply.pushes[1..]
            .iter()
            .filter(|push| {
                let mut r = Reader::new(push);
                r.u32().unwrap();
                r.u64().unwrap();
                r.string(65).unwrap();
                r.u64().unwrap();
                codol_protocol::room::messages::updates(r.blob().unwrap(), owner.user_id)
                    .unwrap()
                    .avatar_refresh
            })
            .count();
        assert_eq!(avatars, avatar_count);
    }
    let denied = request(&bot_ready(false));
    assert!(
        rooms
            .handle(&Request::parse(&denied).unwrap(), &session(43), 1)
            .unwrap()
            .pushes
            .is_empty()
    );
}
#[tokio::test]
async fn supported_creation_joins_bots_after_receiver_ready_without_a_map_change() {
    let temp = tempfile::tempdir().unwrap();
    let server = crate::transport::game::server::GameServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        temp.path().join("game"),
    )
    .await
    .unwrap();
    let mut endpoint = server.admissions();
    drop(server);
    Arc::get_mut(&mut endpoint).unwrap().bot_rooms = Some(crate::backend::rooms::bots::Policy {
        version: 7,
        maps: [("mp_killhouse_sh".into(), "war".into())].into(),
        loadouts: Default::default(),
    });
    for (map, mode, playlist, capacity, explicit_unsupported, expected_joins) in [
        ("mp_killhouse_sh", "war", u32::MAX, 12, false, 33),
        ("mp_highrise", "war", u32::MAX, 12, false, 0),
        ("mp_killhouse_sh", "sd", u32::MAX, 12, false, 0),
        ("mp_killhouse_sh", "war", 0, 12, false, 0),
        ("mp_killhouse_sh", "war", u32::MAX, 18, false, 0),
        ("mp_killhouse_sh", "war", u32::MAX, 12, true, 0),
    ] {
        let mut rooms = Rooms::with_game_endpoint(endpoint.clone());
        let owner = session(0x8000000000000001);
        let (channel_id, _) = create(&mut rooms, &owner);
        let mut description = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/room-create-synthetic.bin"
        ))
        .to_vec();
        let n = description.len();
        description[n - 19..n - 11].copy_from_slice(&channel_id.to_le_bytes());
        for (attribute, value) in [
            (4, crc32fast::hash(map.as_bytes())),
            (5, playlist),
            (10, crc32fast::hash(mode.as_bytes())),
        ] {
            let offset = n - 74 + attribute * 5;
            description[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            Description::parse(&description, owner.user_id)
                .unwrap()
                .matches_custom_map(map, mode),
            playlist == u32::MAX
        );
        let mut create_game = vec![21, 3, 1];
        create_game.extend(description);
        let result = rooms
            .handle(&Request::parse(&create_game).unwrap(), &owner, 1)
            .unwrap();
        assert!(result.result.is_ok());
        assert!(result.pushes.is_empty()); // Receiver is not installed yet.
        rooms
            .channels
            .get_mut(&channel_id)
            .unwrap()
            .game
            .as_mut()
            .unwrap()
            .description
            .maximum = capacity;
        let broadcast = |batch: &[u8]| {
            let mut w = Writer(vec![11, 3, 4]);
            w.u64(channel_id);
            w.blob(batch).unwrap();
            w.u32(1);
            w.0.push(0);
            w.0
        };
        if explicit_unsupported {
            // A newer kind15 choice overrides a still-old creation CRC.
            let payload = b"war\0mp_highrise\0\xff\x04\x0c";
            let mut record = vec![0; payload.len() + 2];
            record[0] = 15 << 1;
            for (i, b) in payload.iter().enumerate() {
                record[i + 1] |= b << 1;
                record[i + 2] |= b >> 7;
            }
            let mut batch = (record.len() as u32).to_le_bytes().to_vec();
            batch.extend(record);
            let result = rooms
                .handle(&Request::parse(&broadcast(&batch)).unwrap(), &owner, 1)
                .unwrap();
            assert_eq!(result.pushes.len(), 1);
        }
        let message = broadcast(&[2, 0, 0, 0, 0x20, 0]);
        let request = Request::parse(&message).unwrap();
        assert!(
            rooms
                .handle(&request, &session(2), 1)
                .unwrap()
                .result
                .is_err()
        );
        let result = rooms.handle(&request, &owner, 1).unwrap();
        assert_eq!(result.pushes.len(), 1 + expected_joins, "{map}");
        assert_eq!(
            rooms.handle(&request, &owner, 1).unwrap().pushes.len(),
            if expected_joins > 0 { 12 } else { 1 }
        );
    }
}
#[test]
fn bot_direct_delivery_is_scoped_and_requests_the_authoritative_roster_once() {
    let mut rooms = Rooms::default();
    let owner = session(42);
    let (id, _) = create(&mut rooms, &owner);
    rooms.channels.get_mut(&id).unwrap().bots_announced = true;
    let request = |recipient| {
        let mut w = Writer(vec![11, 3, 5]);
        w.u64(id);
        w.u64(recipient);
        // Native kind 5 JoinResponse, status zero (untyped bit stream).
        w.blob(&[6, 0, 0, 0, 10, 0, 0, 0, 0, 0]).unwrap();
        w.u32(1);
        w.0.push(0);
        w.0
    };
    let bytes = request(codol_protocol::room::messages::bot_id(11));
    let req = Request::parse(&bytes).unwrap();
    let denied = rooms.handle(&req, &session(43), 1).unwrap();
    assert!(denied.result.is_err());
    assert!(denied.pushes.is_empty() && denied.deliveries.is_empty());
    let unknown = request(123);
    assert!(
        rooms
            .handle(&Request::parse(&unknown).unwrap(), &owner, 1)
            .is_err()
    );
    let reply = rooms.handle(&req, &owner, 1).unwrap();
    assert!(reply.result.is_ok());
    assert_eq!(reply.pushes.len(), 2);
    let repeated = rooms.handle(&req, &owner, 1).unwrap();
    assert_eq!(repeated.pushes.len(), 1);
    let channel = rooms.channels.get_mut(&id).unwrap();
    assert!(channel.game_teams().is_err());
    channel.member_teams.insert(owner.user_id, 2); // Native room axis.
    for client in 1..=11 {
        channel.member_teams.insert(
            codol_protocol::room::messages::bot_id(client),
            if client % 2 == 0 { 2 } else { 1 },
        );
    }
    assert_eq!(
        channel.game_teams().unwrap(),
        [1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2]
    );
    channel.member_teams.insert(owner.user_id, 1);
    assert!(channel.game_teams().is_err()); // Seven allies cannot be a 6v6 roster.
}
#[test]
fn human_readiness_rearms_bots_after_map_change_but_cancel_does_not_start_them() {
    let mut rooms = Rooms::default();
    let owner = session(42);
    let (id, _) = create(&mut rooms, &owner);
    rooms.channels.get_mut(&id).unwrap().bots_announced = true;
    for ready in [true, false, true] {
        let mut w = Writer(vec![11, 3, 4]);
        w.u64(id);
        w.blob(&[2, 0, 0, 0, 0x20, if ready { 2 } else { 0 }])
            .unwrap();
        w.u32(1);
        w.0.push(0);
        let result = rooms
            .handle(&Request::parse(&w.0).unwrap(), &owner, 1)
            .unwrap();
        assert_eq!(result.pushes.len(), 12);
        assert_eq!(rooms.channels[&id].owner_ready, ready);
        for push in &result.pushes[1..] {
            let mut r = Reader::new(push);
            assert_eq!(r.u32().unwrap(), 7);
            r.u64().unwrap();
            r.string(65).unwrap();
            assert_eq!(r.u64().unwrap(), id);
            assert_eq!(
                codol_protocol::room::messages::updates(r.blob().unwrap(), owner.user_id)
                    .unwrap()
                    .ready,
                Some(ready)
            );
        }
    }
}
#[test]
fn map_selection_clears_readiness_before_a_delayed_bot_join_reply() {
    let mut rooms = Rooms::default();
    let owner = session(42);
    let (id, _) = create(&mut rooms, &owner);
    let channel = rooms.channels.get_mut(&id).unwrap();
    channel.bots_announced = true;
    channel.owner_ready = true;
    let payload = b"war\0mp_storm_spring\0\xff\x04\x0c";
    let mut record = vec![0; payload.len() + 2];
    record[0] = 15 << 1;
    for (i, b) in payload.iter().enumerate() {
        record[i + 1] |= b << 1;
        record[i + 2] |= b >> 7;
    }
    let mut batch = (record.len() as u32).to_le_bytes().to_vec();
    batch.extend(record);
    let mut broadcast = Writer(vec![11, 3, 4]);
    broadcast.u64(id);
    broadcast.blob(&batch).unwrap();
    broadcast.u32(1);
    broadcast.0.push(0);
    let result = rooms
        .handle(&Request::parse(&broadcast.0).unwrap(), &owner, 1)
        .unwrap();
    assert_eq!(result.pushes.len(), 12);
    assert!(!rooms.channels[&id].owner_ready);
    let mut direct = Writer(vec![11, 3, 5]);
    direct.u64(id);
    direct.u64(codol_protocol::room::messages::bot_id(1));
    direct.blob(&[6, 0, 0, 0, 10, 0, 0, 0, 0, 0]).unwrap();
    direct.u32(1);
    direct.0.push(0);
    let reply = rooms
        .handle(&Request::parse(&direct.0).unwrap(), &owner, 1)
        .unwrap();
    assert_eq!(reply.pushes.len(), 1);
    for push in result.pushes[1..].iter().chain(&reply.pushes) {
        let mut r = Reader::new(push);
        assert_eq!(r.u32().unwrap(), 7);
        r.u64().unwrap();
        r.string(65).unwrap();
        assert_eq!(r.u64().unwrap(), id);
        assert_eq!(
            codol_protocol::room::messages::updates(r.blob().unwrap(), owner.user_id)
                .unwrap()
                .ready,
            Some(false)
        );
    }
}
#[test]
fn game_sessions_require_owned_channels_and_updates_require_the_issued_id() {
    let mut rooms = Rooms::default();
    let owner = session(0x8000000000000001);
    let (channel_id, _) = create(&mut rooms, &owner);
    let mut description = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/room-create-synthetic.bin"
    ))
    .to_vec();
    let n = description.len();
    description[n - 19..n - 11].copy_from_slice(&channel_id.to_le_bytes());
    let mut body = vec![21, 3, 1];
    body.extend_from_slice(&description);
    let request = Request::parse(&body).unwrap();
    let result = rooms.handle(&request, &owner, 1).unwrap().result.unwrap();
    let id = Reader::new(&result[0]).blob().unwrap().to_vec();
    assert_eq!(id.len(), 8);
    assert_eq!(
        rooms.handle(&request, &owner, 1).unwrap().result.unwrap(),
        result
    );
    assert!(rooms.handle(&request, &session(2), 1).is_err());
    let mut other_title = owner.clone();
    other_title.title_id += 1;
    assert!(
        rooms
            .handle(&request, &other_title, 1)
            .unwrap()
            .result
            .is_err()
    );
    let mut update = Writer(vec![21, 3, 12]);
    update.blob(&id).unwrap();
    update.u32(1);
    update.0.extend_from_slice(&description);
    assert!(
        rooms
            .handle(&Request::parse(&update.0).unwrap(), &owner, 1)
            .unwrap()
            .result
            .is_ok()
    );
    update.0[9] ^= 1;
    assert!(
        rooms
            .handle(&Request::parse(&update.0).unwrap(), &owner, 1)
            .unwrap()
            .result
            .is_err()
    );
    let mut members = Writer(vec![11, 3, 15]);
    members.u64(channel_id);
    members.u32(100);
    members.0.push(0);
    let reply = rooms
        .handle(&Request::parse(&members.0).unwrap(), &owner, 1)
        .unwrap()
        .result
        .unwrap();
    let mut reader = Reader::new(&reply[0]);
    assert_eq!(reader.u64().unwrap(), owner.user_id);
    assert_eq!(reader.string(65).unwrap(), "TGFiIHBsYXllcg==");
    assert_eq!(reader.u32().unwrap(), 0);
    let batch = [12, 0, 0, 0, 2, 2, 0, 0, 0, 8, 0, 0, 0, 128, 8, 0];
    let mut message = Writer(vec![11, 3, 4]);
    message.u64(channel_id);
    message.blob(&batch).unwrap();
    message.u32(1);
    message.0.push(0);
    let req = Request::parse(&message.0).unwrap();
    assert!(rooms.handle(&req, &session(2), 1).unwrap().result.is_err());
    for _ in 0..40 {
        assert!(rooms.handle(&req, &owner, 1).unwrap().result.is_ok());
    }
    assert_eq!(rooms.channels[&channel_id].messages.len(), 32);
    assert_eq!(rooms.channels[&channel_id].messages.back().unwrap(), &batch);
    message.0[18] = 255; // First framed length now exceeds the blob.
    assert!(
        rooms
            .handle(&Request::parse(&message.0).unwrap(), &owner, 1)
            .is_err()
    );
    let mut delete = Writer(vec![21, 3, 3]);
    delete.blob(&id).unwrap();
    delete.0.push(0);
    let req = Request::parse(&delete.0).unwrap();
    assert!(rooms.handle(&req, &session(2), 1).unwrap().result.is_err());
    assert!(rooms.handle(&req, &owner, 1).unwrap().result.is_ok());
    assert!(rooms.channels[&channel_id].game.is_none());
}
#[test]
fn channels_are_idempotent_isolated_and_expire_with_the_session() {
    let mut rooms = Rooms::default();
    let a = session(u64::MAX);
    let b = session(2);
    let (aid, record) = create(&mut rooms, &a);
    assert_eq!(create(&mut rooms, &a).0, aid);
    assert_ne!(create(&mut rooms, &b).0, aid);
    assert_eq!(
        &record[9..],
        &[
            8, 1, 0, 0, 0, 8, 0, 0, 0, 0, 10, 255, 255, 255, 255, 255, 255, 255, 255, 1, 0, 1, 0,
            8, 0, 0, 0, 0
        ]
    );
    let mut w = Writer(vec![11, 3, 2]);
    w.u64(aid);
    w.0.push(0);
    let leave = Request::parse(&w.0).unwrap();
    let mut other_title = a.clone();
    other_title.title_id += 1;
    for other in [&b, &other_title] {
        assert!(rooms.handle(&leave, other, 1).unwrap().result.is_err());
    }
    assert!(rooms.handle(&leave, &a, 1).unwrap().result.is_ok());
    assert!(!rooms.channels.contains_key(&aid));
    let new_id = create(&mut rooms, &a).0;
    let mut new_session = a.clone();
    new_session.iv_seed += 1;
    assert_ne!(create(&mut rooms, &new_session).0, new_id);
    assert!(!rooms.channels.contains_key(&new_id));
    let bad = Request::parse(&[11, 3, 13, 0, 0]).unwrap();
    let mut authenticated = a.clone();
    authenticated.expires_at = 1000;
    assert!(rooms.handle(&bad, &authenticated, 101).is_err());
    assert!(rooms.channels.is_empty());
}
#[test]
fn authenticated_session_renewal_preserves_only_its_own_room() {
    let mut rooms = Rooms::default();
    let mut owner = session(42);
    let (id, _) = create(&mut rooms, &owner);
    let (other, _) = create(&mut rooms, &session(43));
    owner.expires_at = 1000;
    rooms.renew_session(&owner, 99).unwrap();
    let req = Request::parse(&[11, 3, 13, 0]).unwrap();
    let reply = rooms.handle(&req, &owner, 101).unwrap().result.unwrap();
    assert_eq!(Reader::new(&reply[0]).u64().unwrap(), id);
    assert!(!rooms.channels.contains_key(&other));
    assert_eq!(rooms.channels[&id].expires_at, 1000);
}
