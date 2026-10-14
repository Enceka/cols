use super::*;

#[tokio::test]
async fn no_create_session_story_allocation_is_opt_in_and_uses_the_owned_channel() {
    let temp = tempfile::tempdir().unwrap();
    let s = Session {
        title_id: 1002,
        user_id: 42,
        iv_seed: 17,
        expires_at: 100,
        key: [0; 24],
    };
    let mut query = Writer(vec![21, 3, 5]);
    query.u32(1);
    query.u32(0);
    query.u32(10);
    query.i32(codol_protocol::story::CLIENT_BUILD);
    query.0.push(0);
    for capture in [false, true] {
        let server = crate::game_server::GameServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            temp.path().join(if capture { "story" } else { "normal" }),
        )
        .await
        .unwrap();
        let server = if capture {
            server
                .with_story_capture_map(codol_protocol::story::FIRST_MAP.into())
                .unwrap()
        } else {
            server
        };
        let endpoint = server.admissions();
        let mut rooms = Rooms::with_game_endpoint(endpoint.clone());
        let request = Request::parse(&query.0).unwrap();
        assert!(rooms.handle(&request, &s, 1).unwrap().result.is_err());
        let result = rooms
            .handle(&Request::parse(&[11, 3, 13, 0]).unwrap(), &s, 1)
            .unwrap()
            .result
            .unwrap();
        let channel = Reader::new(&result[0]).u64().unwrap();
        let reply = rooms.handle(&request, &s, 1).unwrap();
        if !capture {
            assert_eq!(reply.label, "game_allocation_requires_live_key");
            assert!(reply.result.is_err());
            continue;
        }
        assert_eq!(reply.label, "local_story_transport_allocated");
        let records = reply.result.unwrap();
        let again = rooms.handle(&request, &s, 2).unwrap().result.unwrap();
        assert_eq!(records, again);
        let mut r = Reader::new(&records[0]);
        r.blob().unwrap();
        let id: [u8; 8] = r.blob().unwrap().try_into().unwrap();
        let mut other = s.clone();
        other.user_id += 1;
        assert!(rooms.handle(&request, &other, 2).unwrap().result.is_err());
        let mut leave = Writer(vec![11, 3, 2]);
        leave.u64(channel);
        leave.0.push(0);
        rooms
            .handle(&Request::parse(&leave.0).unwrap(), &s, 3)
            .unwrap();
        let mut control = Writer::default();
        control.u64(1_000_001);
        let book = format!("mmbook 7 {} 0", STANDARD.encode(id));
        control
            .blob(
                &[
                    (book.len() as u32).to_le_bytes().as_slice(),
                    book.as_bytes(),
                ]
                .concat(),
            )
            .unwrap();
        control.0.push(0);
        assert!(endpoint.control(&control.0, &s, 3).is_err());
    }
}
