use super::*;

fn room() -> SharedMatch {
    let collision = serde_json::json!({"schema_version":1,"read_only":true,
        "client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/mp_afghan.d3dbsp","ncs_crc":0x12345678,
        "brushes":[{"center":[0,0,-10],"half":[1000,1000,10],"contents":0x08000010,"planes":[]}],"triangles":[]});
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), serde_json::to_vec(&collision).unwrap()).unwrap();
    let collision = Arc::new(crate::game::world::collision::Collision::load(file.path()).unwrap());
    let players = (0..2)
        .map(|i| {
            let state = crate::game::session::prepared::human_test_state(
                [i as f32 * 150.0, 0.0, 0.0],
                collision.clone(),
            );
            (
                Participant {
                    user_id: 42 + i,
                    seed: i as u32,
                    team: i as u8 + 1,
                    bot: None,
                },
                format!("Player {i}"),
                Arc::new(state),
            )
        })
        .collect();
    SharedMatch::new(players).unwrap()
}

#[test]
fn one_clock_waits_for_all_clients_and_is_independent_of_view_polling() {
    let mut world = room();
    let now = Instant::now();
    world.attach(42).unwrap();
    world.attach(43).unwrap();
    world.ready(42).unwrap();
    world.advance(now).unwrap();
    assert!(world.epoch.is_none());
    world.ready(43).unwrap();
    world.advance(now).unwrap();
    world
        .advance(now + std::time::Duration::from_millis(50))
        .unwrap();
    let first = world.view(42).unwrap();
    let second = world.view(43).unwrap();
    assert_eq!(first.time, 1050);
    assert_eq!(second.time, first.time);
    assert_eq!(first.local.origin, second.players[0].origin);
    assert_eq!(second.local.origin, first.players[0].origin);
    for _ in 0..10 {
        assert_eq!(world.view(42).unwrap().time, 1050);
    }
    world
        .advance(now + std::time::Duration::from_millis(50))
        .unwrap();
    assert_eq!(world.time, 1050);
}

#[test]
fn actual_weapon_inputs_damage_the_other_authority_once_and_share_scores_and_respawn() {
    let mut world = room();
    let now = Instant::now();
    for id in [42, 43] {
        world.attach(id).unwrap();
        world.ready(id).unwrap();
    }
    world.advance(now).unwrap();
    for step in 1..=8u32 {
        let command = UserCommand {
            time: 1000 + step * 100,
            buttons: 1,
            weapons: [352, 0, 0],
            ..Default::default()
        };
        world.submit(42, command.clone()).unwrap();
        world.submit(42, command).unwrap();
        world
            .advance(now + std::time::Duration::from_millis(u64::from(step * 100)))
            .unwrap();
    }
    let a = world.view(42).unwrap();
    let b = world.view(43).unwrap();
    assert_eq!(a.scoreboard.rows[0].kills, 1);
    assert_eq!(b.scoreboard.rows[0].deaths, 1);
    assert_eq!(a.scoreboard.team_scores, b.scoreboard.team_scores);
    assert_eq!(world.scores[1], 100);
    assert_eq!(b.vitals.health, 0);
    let mut old = UserCommand {
        time: 1100,
        buttons: 1,
        weapons: [352, 0, 0],
        ..Default::default()
    };
    world.submit(42, old.clone()).unwrap();
    world
        .advance(now + std::time::Duration::from_millis(4000))
        .unwrap();
    assert_eq!(world.view(43).unwrap().vitals.health, 100);
    assert_eq!(world.scores[1], 100);
    old.time = 100000;
    world.submit(42, old).unwrap();
    assert!(world.humans[0].inputs.is_empty());
    world.detach(43);
    assert!(world.ended.is_some());
    assert!(!world.view(42).unwrap().players[0].alive);
}

#[test]
fn a_filled_bot_is_ready_without_a_socket_and_fires_in_the_shared_world() {
    let mut world = room();
    let bot = &mut world.humans[1];
    bot.member.bot = Some(1);
    bot.ai = Some(super::bots::Ai::new(&bot.state).unwrap());
    bot.connected = true;
    bot.ready = true;
    assert!(world.attach(43).is_err());
    let now = Instant::now();
    world.attach(42).unwrap();
    world.ready(42).unwrap();
    world.advance(now).unwrap();
    for step in 1..=80 {
        world
            .advance(now + std::time::Duration::from_millis(step * 50))
            .unwrap();
    }
    assert!(world.epoch.is_some());
    assert!(world.humans[0].row.deaths > 0 || world.humans[0].vitals.health < 100);
    let a = world.view(42).unwrap();
    let b = world.view(43).unwrap();
    assert_eq!(a.scoreboard.team_scores, b.scoreboard.team_scores);
    assert_eq!(a.players[0].origin, b.local.origin);
}
