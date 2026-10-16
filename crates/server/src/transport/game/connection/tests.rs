#[test]
fn spawn_view_resets_heading_without_accumulating_the_raw_input_offset() {
    let mut c = super::Connection {
        last_input: Some(codol_protocol::game::channel::usercmd::UserCommand {
            angles: [2000, 16000, 0],
            ..Default::default()
        }),
        ..Default::default()
    };
    c.reset_spawn_view([0, 32768, 0]);
    assert_eq!(c.view_angles(), [0, 32768, 0]);
    assert_eq!(c.view_offset, [0u16.wrapping_sub(2000), 16768, 0]);
    for desired in [[0, 49152, 0], [0, 5461, 0], [0, 32768, 0]] {
        // Ordinary mouse movement remains an input change after a reset.
        let raw = [2000u16, 17000, 0];
        c.last_input.as_mut().unwrap().angles =
            std::array::from_fn(|i| raw[i].wrapping_add(c.view_offset[i]));
        c.reset_spawn_view(desired);
        assert_eq!(c.view_angles(), desired);
        assert_eq!(
            std::array::from_fn::<_, 3, _>(|i| desired[i].wrapping_sub(c.view_offset[i])),
            raw
        );
    }
}
#[test]
fn respawn_waits_for_new_life_ack_then_anchors_once_and_preserves_mouse_turns() {
    let mut c = super::Connection {
        world: crate::game::session::local_world::LocalWorld {
            player_birth: 2000,
            ..Default::default()
        },
        native_sent: 9,
        native_ack: 9,
        ..Default::default()
    };
    c.reset_spawn_view([0, 32768, 0]);
    c.pending_inputs.insert(
        2050,
        codol_protocol::game::channel::usercmd::UserCommand {
            time: 2050,
            angles: [100, 5000, 0],
            ..Default::default()
        },
    );
    c.consume_inputs(2050);
    assert!(c.spawn_view_anchor.is_some());
    assert_eq!(c.view_angles(), [0, 32768, 0]);
    c.native_ack = 10;
    c.pending_inputs.insert(
        2100,
        codol_protocol::game::channel::usercmd::UserCommand {
            time: 2100,
            angles: [200, 6000, 0],
            ..Default::default()
        },
    );
    c.consume_inputs(2100);
    assert!(c.spawn_view_anchor.is_none());
    assert_eq!(c.view_angles(), [0, 32768, 0]);
    c.pending_inputs.insert(
        2150,
        codol_protocol::game::channel::usercmd::UserCommand {
            time: 2150,
            angles: [250, 6500, 0],
            ..Default::default()
        },
    );
    c.consume_inputs(2150);
    assert_eq!(c.view_angles(), [50, 33268, 0]);
}
use super::*;
fn prepared_player_for_flow(weapon: u16) -> Arc<crate::game::session::prepared::PreparedGameState> {
    Arc::new(serde_json::from_value(serde_json::json!({
        "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"capture_sha256":"0".repeat(64),
        "map":"mp_afghan","mode":"war","zone":"mp","playlist":0,"ncs_crc":0x12345678u32,
        "common_crc":1,"constant_count":2,"asset_count":1,"observed_crc_verified":false,
        "spawn_origin":[-300,0,0],"bootstrap_weapon":{"name":"m4_mp","index":weapon,
            "ammo":{"clip_key":weapon,"reserve_key":163,"clip":30,"reserve":90,
                "source_sha256":"0".repeat(64),"fire_interval_msec":79}}
    })).unwrap())
}

fn preparing_connection() -> Connection {
    let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
        "map":"maps/mp/mp_afghan.d3dbsp","ncs_crc":0x12345678u32,
        "brushes":[{"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}],"triangles":[]});
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
    let collision = Arc::new(crate::game::world::collision::Collision::load(file.path()).unwrap());
    let config = serde_json::from_value(serde_json::json!({
        "spawns":[(0..6).map(|i| [300,i*100,0]).collect::<Vec<_>>(),(0..6).map(|i| [-300,i*100,0]).collect::<Vec<_>>()],
        "score_limit":7500,"time_limit_msec":600000
    })).unwrap();
    let template = codol_protocol::game::snapshot::enemy::EnemySnapshot {
        entity_number: 0,
        client_index: 0,
        team: 2,
        model: 4,
        attachment_models: [0; 10],
        attachment_tags: [0; 10],
        origin: [-300.0, 0.0, 0.0],
        angles: [0; 3],
        legs_animation: 395,
        torso_animation: 395,
        weapon: 352,
        events: Default::default(),
        alive: true,
    };
    let mut connection = Connection::with_game_state(prepared_player_for_flow(352));
    Arc::make_mut(connection.world.game_state.as_mut().unwrap()).collision =
        Some(collision.clone());
    connection.world.tdm =
        Some(crate::game::modes::tdm::TdmMatch::new(config, template, 190.0, &collision).unwrap());
    connection.world.motion = connection.world.game_state.as_ref().unwrap().motion();
    connection
}

#[test]
fn registered_nuclear_activator_reaches_the_extra_slot_instead_of_ordinary_radio_rejection() {
    let mut c = preparing_connection();
    let nuke = crate::game::rewards::killstreaks::test_resources();
    let mut radar = crate::game::rewards::remote::test_resources();
    radar.radios.push(crate::game::rewards::killstreaks::Radio {
        native_id: 15,
        weapon: nuke.weapon,
        switch: nuke.switch,
        fire_interval_msec: nuke.fire_interval_msec,
    });
    crate::game::session::prepared::test_reward_resources(
        Arc::make_mut(c.world.game_state.as_mut().unwrap()),
        nuke.clone(),
        radar,
    );
    let tdm = c.world.tdm.as_mut().unwrap();
    tdm.advance_clock(1000);
    tdm.advance_clock(7000);
    tdm.streaks.enable();
    tdm.streaks.acceptance_seed(24);
    tdm.streaks.credit(7000, 6, 1, Some(0));
    assert!(tdm.streaks.owned(0));
    c.world.player.as_mut().unwrap().grant_nuke(&nuke);
    c.world
        .player
        .as_mut()
        .unwrap()
        .set_switch_timing(nuke.switch);
    for time in (7050..=10000).step_by(50) {
        c.pending_inputs.insert(
            time,
            codol_protocol::game::channel::usercmd::UserCommand {
                time,
                weapons: [nuke.weapon, 0, 0],
                ..Default::default()
            },
        );
        c.consume_inputs(time);
        if c.world
            .tdm
            .as_ref()
            .unwrap()
            .streaks
            .countdown_end()
            .is_some()
        {
            break;
        }
    }
    assert!(!c.world.tdm.as_ref().unwrap().streaks.owned(0));
    assert!(
        c.world
            .tdm
            .as_ref()
            .unwrap()
            .streaks
            .countdown_end()
            .is_some()
    );
    assert!(c.nuke_return_at.is_some());
    assert_eq!(
        c.world.player.as_ref().unwrap().equipment().weapon,
        nuke.weapon
    );
}

#[test]
fn frozen_input_advances_ack_without_motion_ammo_actions_or_weapon_selection_and_unlocks_once() {
    let mut c = preparing_connection();
    let origin = c.world.motion.as_ref().unwrap().origin;
    let ammo = c.world.player.as_ref().unwrap().equipment().ammo.unwrap();
    let events = c.world.player.as_ref().unwrap().equipment().events.unwrap();
    c.world.tdm.as_mut().unwrap().advance_clock(1000);
    for time in [1100, 2000, 5999] {
        c.pending_inputs.insert(
            time,
            codol_protocol::game::channel::usercmd::UserCommand {
                time,
                direction: 1,
                buttons: 1 | 4 | 0x400 | 0x14000 | 0x800,
                angles: [1000, 2000, 0],
                weapons: [353, 0, 0],
                ..Default::default()
            },
        );
        c.consume_inputs(time);
    }
    assert_eq!(c.input_count, 3);
    assert_eq!(c.world.motion.as_ref().unwrap().command_time, 5999);
    assert_eq!(c.world.motion.as_ref().unwrap().origin, origin);
    assert_eq!(c.view_angles(), [0; 3]);
    let held = c.world.player.as_mut().unwrap();
    assert_eq!(held.equipment().weapon, 352);
    assert_eq!(held.equipment().ammo.unwrap().clip, ammo.clip);
    assert_eq!(held.equipment().ammo.unwrap().reserve, ammo.reserve);
    assert_eq!(held.equipment().events.unwrap(), events);
    assert!(
        held.take_shots().is_empty()
            && held.take_throws().is_empty()
            && held.take_melee_strikes().is_empty()
    );
    assert_ne!(c.snapshot_motion().unwrap().flags & 0x800, 0);
    c.world.tdm.as_mut().unwrap().advance_clock(6000);
    assert_eq!(c.snapshot_motion().unwrap().flags & 0x800, 0);
    c.pending_inputs.insert(
        6050,
        codol_protocol::game::channel::usercmd::UserCommand {
            time: 6050,
            direction: 1,
            weapons: [352, 0, 0],
            ..Default::default()
        },
    );
    c.consume_inputs(6050);
    assert!(c.world.motion.as_ref().unwrap().origin[0] > origin[0]);
}

#[test]
fn preparation_backpack_equips_after_resource_ack_without_respawning_and_live_changes_wait() {
    let mut c = preparing_connection();
    let origin = c.world.motion.as_ref().unwrap().origin;
    let life = c.world.vitals.spawn_count;
    let sequence = c.server_commands.push(b"d 5796 selected_mp").unwrap();
    c.pending_loadout = Some((sequence, prepared_player_for_flow(353)));
    c.apply_acknowledged_loadout();
    assert_eq!(c.world.player.as_ref().unwrap().equipment().weapon, 352);
    assert!(c.pending_loadout.is_some());
    c.server_commands.mark_sent(sequence).unwrap();
    c.server_commands.acknowledge(sequence).unwrap();
    c.apply_acknowledged_loadout();
    assert_eq!(c.world.player.as_ref().unwrap().equipment().weapon, 353);
    assert!(c.next_spawn_loadout.is_none());
    assert_eq!(c.world.motion.as_ref().unwrap().origin, origin);
    assert_eq!(c.world.vitals.spawn_count, life);
    assert_ne!(c.snapshot_motion().unwrap().flags & 0x800, 0);
    c.world.tdm.as_mut().unwrap().advance_clock(1000);
    c.world.tdm.as_mut().unwrap().advance_clock(6000);
    c.last_snapshot_time = 6000;
    c.world.tdm.as_mut().unwrap().mark_combat(0);
    c.pending_loadout = Some((sequence, prepared_player_for_flow(354)));
    c.apply_acknowledged_loadout();
    assert_eq!(c.world.player.as_ref().unwrap().equipment().weapon, 353);
    assert!(c.next_spawn_loadout.is_some());
}
#[test]
fn unused_grace_life_can_change_backpack_until_its_deadline_but_damage_closes_the_window() {
    for damaged in [false, true] {
        let mut c = preparing_connection();
        c.world.tdm.as_mut().unwrap().advance_clock(1000);
        c.world.tdm.as_mut().unwrap().advance_clock(6000);
        c.last_snapshot_time = 20_999;
        if damaged {
            c.world.vitals.damage_event = 1;
        }
        c.pending_loadout = Some((0, prepared_player_for_flow(353)));
        c.apply_acknowledged_loadout();
        assert_eq!(
            c.world.player.as_ref().unwrap().equipment().weapon,
            if damaged { 352 } else { 353 }
        );
        assert_eq!(c.next_spawn_loadout.is_some(), damaged);
    }
    let mut c = preparing_connection();
    c.world.tdm.as_mut().unwrap().advance_clock(1000);
    c.world.tdm.as_mut().unwrap().advance_clock(21_000);
    c.last_snapshot_time = 21_000;
    c.pending_loadout = Some((0, prepared_player_for_flow(353)));
    c.apply_acknowledged_loadout();
    assert_eq!(c.world.player.as_ref().unwrap().equipment().weapon, 352);
    assert!(c.next_spawn_loadout.is_some());
}
#[test]
fn firing_during_grace_closes_backpack_replenishment_even_without_a_damage_definition() {
    let mut c = preparing_connection();
    c.world.tdm.as_mut().unwrap().advance_clock(1000);
    c.world.tdm.as_mut().unwrap().advance_clock(6000);
    c.last_snapshot_time = 6500;
    c.pending_inputs.insert(
        6500,
        codol_protocol::game::channel::usercmd::UserCommand {
            time: 6500,
            buttons: 1,
            weapons: [352, 0, 0],
            ..Default::default()
        },
    );
    c.consume_inputs(6500);
    assert_eq!(
        c.world
            .player
            .as_ref()
            .unwrap()
            .equipment()
            .ammo
            .unwrap()
            .clip,
        29
    );
    c.pending_loadout = Some((0, prepared_player_for_flow(353)));
    c.apply_acknowledged_loadout();
    assert_eq!(c.world.player.as_ref().unwrap().equipment().weapon, 352);
    assert!(c.next_spawn_loadout.is_some());
}
#[test]
fn glass_authority_survives_backpack_respawn_and_is_isolated_from_new_connections() {
    let export = serde_json::json!({
        "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
        "map":"maps/mp/mp_afghan.d3dbsp","ncs_crc":0x12345678,
        "brushes":[{"center":[50,0,45],"half":[1,100,45],"contents":0x0800_0010,"planes":[],"glass_piece":4}],
        "triangles":[]
    });
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
    let mut template = crate::game::session::prepared::synthetic();
    template.collision = Some(Arc::new(
        crate::game::world::collision::Collision::load(file.path()).unwrap(),
    ));
    let template = Arc::new(template);
    let mut first = Connection::with_game_state(template.clone());
    let second = Connection::with_game_state(template.clone());
    first
        .world
        .game_state
        .as_ref()
        .unwrap()
        .collision
        .as_ref()
        .unwrap()
        .shatter_glass(4);
    first.next_spawn_loadout = Some(template.clone());
    first.equip_spawn_loadout();
    assert_eq!(
        first
            .world
            .game_state
            .as_ref()
            .unwrap()
            .collision
            .as_ref()
            .unwrap()
            .destroyed_glass(),
        vec![4]
    );
    assert!(
        second
            .world
            .game_state
            .as_ref()
            .unwrap()
            .collision
            .as_ref()
            .unwrap()
            .destroyed_glass()
            .is_empty()
    );
    assert!(
        template
            .collision
            .as_ref()
            .unwrap()
            .destroyed_glass()
            .is_empty()
    );
    let third = Connection::with_game_state(template);
    assert!(
        third
            .world
            .game_state
            .as_ref()
            .unwrap()
            .collision
            .as_ref()
            .unwrap()
            .destroyed_glass()
            .is_empty()
    );
}
#[test]
fn raw_landing_event_and_profile_immunity_apply_once_before_health_recovery() {
    let landing =
        crate::game::player::landing::calculate(2000, 214.0, 0.0, 0.0, 800, 3 << 20).unwrap();
    let mut c = Connection {
        world: crate::game::session::local_world::LocalWorld {
            player: Some(crate::game::weapons::held::tests::scavenger_player(90, 180)),
            ..Default::default()
        },
        ..Default::default()
    };
    c.apply_landing(landing);
    assert_eq!(c.world.vitals.health, 50);
    assert_eq!(c.world.vitals.damage_event, 1);
    let events = c.world.player.as_ref().unwrap().equipment().events.unwrap();
    assert_eq!(events.slots[0], landing.event.unwrap());
    assert_eq!(c.world.health_recovery.recover(6999, 50), 50);
    assert_eq!(c.world.health_recovery.recover(7000, 50), 60);
    c.world.vitals.health = 100;
    c.world
        .player
        .as_mut()
        .unwrap()
        .set_perks(crate::game::status::perks::Perks::selected([60041, 0, 0], &[60041]).unwrap());
    c.apply_landing(landing);
    assert_eq!(c.world.vitals.health, 100);
    assert_eq!(c.world.vitals.damage_event, 1);
}
#[test]
fn profile_notice_waits_for_loaded_profile_hud_and_first_snapshot_ack() {
    let mut state = crate::game::session::prepared::synthetic();
    state.profile_hash = Some([1; 32]);
    let mut c = Connection {
        world: crate::game::session::local_world::LocalWorld {
            game_state: Some(Arc::new(state)),
            player: Some(crate::game::weapons::held::tests::scavenger_player(90, 180)),
            ..Default::default()
        },
        connected: true,
        gamestate_update: Some(6),
        snapshot_first_sequence: Some(4),
        native_ack: 3,
        ..Default::default()
    };
    assert!(c.poll_profile_ready().is_none());
    let seq = c.server_commands.push(b"s scr_gameended \"0\"").unwrap();
    c.hud_rules_sent = true;
    c.hud_rules_sequence = seq;
    c.server_commands.mark_sent(seq).unwrap();
    c.server_commands.acknowledge(seq).unwrap();
    assert!(c.poll_profile_ready().is_none());
    c.native_ack = 4;
    assert_eq!(
        c.poll_profile_ready(),
        Some(b"\xff\xff\xff\xffserverWillWriteProfile\0".as_slice())
    );
    assert!(c.poll_profile_ready().is_none());
    assert_eq!(c.server_commands.sequence(), seq);
    c.profile_ready_sent = false;
    Arc::make_mut(c.world.game_state.as_mut().unwrap()).profile_hash = None;
    assert!(c.poll_profile_ready().is_none());
}
#[test]
fn native_perk_display_waits_for_live_hud_rules_without_delaying_abilities() {
    let player = crate::game::weapons::held::tests::scavenger_player(90, 180);
    let actual = player.equipment();
    let mut c = Connection::default();
    assert_eq!(c.hud_equipment(actual).perks, [0; 3]);
    assert_eq!(c.hud_equipment(actual).abilities, actual.abilities);
    let sequence = c.server_commands.push(b"s scr_gameended \"0\"").unwrap();
    c.hud_rules_sent = true;
    c.hud_rules_sequence = sequence;
    c.server_commands.mark_sent(sequence).unwrap();
    assert_eq!(c.hud_equipment(actual).perks, [0; 3]);
    c.server_commands.acknowledge(sequence).unwrap();
    assert_eq!(c.hud_equipment(actual).perks, [4, 0, 0]);
    assert_eq!(player.equipment().perks, [4, 0, 0]);
}
#[test]
fn native_perk_sidebar_stays_visible_in_each_live_spawn_without_expiring() {
    let equipment = crate::game::weapons::held::tests::scavenger_player(90, 180).equipment();
    let mut c = Connection {
        world: crate::game::session::local_world::LocalWorld {
            vitals: Default::default(),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(!c.perk_display(None));
    assert!(c.perk_display(Some(equipment)));
    c.world.vitals.health = 0;
    assert!(!c.perk_display(Some(equipment)));
    c.world.vitals.health = 100;
    c.world.vitals.spawn_count += 1;
    assert!(c.perk_display(Some(equipment)));
    let empty = codol_protocol::game::snapshot::Equipment {
        perks: [0; 3],
        ..equipment
    };
    assert!(!c.perk_display(Some(empty)));
    assert_eq!(equipment.perks, [4, 0, 0]);
}
#[test]
fn a_stalled_ack_keeps_its_input_base_while_new_snapshot_history_stays_bounded() {
    let mut c = Connection {
        native_ack: 4,
        ..Default::default()
    };
    let base = codol_protocol::game::channel::usercmd::UserCommand {
        weapons: [352, 0, 0],
        ..Default::default()
    };
    c.remember_input_base(4, base.clone());
    for sequence in 5..1000 {
        c.remember_input_base(sequence, Default::default());
    }
    assert_eq!(c.snapshot_input_bases.len(), 256);
    assert_eq!(c.input_base_for(4).unwrap(), base);
    c.native_ack = 999;
    for sequence in 1000..2000 {
        c.remember_input_base(sequence, Default::default());
    }
    assert_eq!(c.snapshot_input_bases.len(), 256);
    assert!(c.input_base_for(999).is_ok());
    assert!(c.input_base_for(4).is_err());
}
#[test]
fn training_backpack_changes_wait_for_the_next_spawn() {
    let current = Arc::new(crate::game::session::prepared::synthetic());
    let next = Arc::new(crate::game::session::prepared::synthetic());
    let mut c = Connection::with_game_state(current.clone());
    c.world.bot =
        Some(crate::game::ai::controller::BotController::new([0.0; 3], [0; 3], 190.0).unwrap());
    c.world.player = Some(crate::game::weapons::held::HeldWeapons::new(
        codol_protocol::game::snapshot::Equipment {
            perks: Default::default(),
            abilities: Default::default(),
            offhands: [None, None],
            offhand_weapon: 0,
            grenade_time: 0,
            move_multiplier: 1.0,
            inventory_weapon: None,
            inventory_override: None,
            alternate_weapon: None,
            alternates: [None, None],
            fire_count: 0,
            queued_burst: false,
            barrel_flags: 0,
            spread_scale: 0.0,
            needs_rechamber: false,
            weapon: 352,
            secondary: None,
            events: None,
            aim: None,
            viewmodel: None,
            firing: None,
            ammo: None,
        },
        None,
        None,
        None,
    ));
    c.pending_loadout = Some((1, next.clone()));
    c.apply_acknowledged_loadout(); // Resources not yet acknowledged.
    assert!(c.pending_loadout.is_some() && c.next_spawn_loadout.is_none());
    c.pending_loadout = Some((0, next.clone()));
    c.apply_acknowledged_loadout();
    assert!(c.pending_loadout.is_none());
    assert!(Arc::ptr_eq(c.world.game_state.as_ref().unwrap(), &current));
    assert_eq!(c.world.player.as_ref().unwrap().equipment().weapon, 352);
    assert_eq!(
        c.feedback.back().map(String::as_str),
        Some("Backpack will change on next spawn")
    );
    c.equip_spawn_loadout();
    assert!(Arc::ptr_eq(c.world.game_state.as_ref().unwrap(), &next));
    assert!(c.next_spawn_loadout.is_none());
    // Without a training life there is no spawn to wait for.
    let mut plain = Connection::with_game_state(current);
    plain.pending_loadout = Some((0, next.clone()));
    plain.apply_acknowledged_loadout();
    assert!(Arc::ptr_eq(plain.world.game_state.as_ref().unwrap(), &next));
}
#[test]
fn headshot_kills_are_counted_for_their_attacker() {
    let mut c = Connection::default();
    c.queue_obituary(
        1000,
        352,
        1,
        0,
        KillCause {
            headshot: true,
            ..Default::default()
        },
    );
    c.queue_obituary(1100, 352, 1, 0, KillCause::default());
    c.queue_obituary(
        1200,
        352,
        0,
        1,
        KillCause {
            headshot: true,
            ..Default::default()
        },
    );
    assert_eq!(c.headshots, [1, 1]);
    assert!(c.obituaries[0].1.headshot && !c.obituaries[1].1.headshot);
}
#[test]
fn intermission_camera_uses_eye_height_without_moving_the_actor_or_accumulating_height() {
    let mut c = Connection {
        world: crate::game::session::local_world::LocalWorld {
            motion: Some(crate::game::player::movement::Motion::new([
                3076.0, -838.0, 4.0,
            ])),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(c.snapshot_motion().unwrap().origin, [3076.0, -838.0, 4.0]);
    c.world.vitals.intermission = true;
    for _ in 0..20 {
        let view = c.snapshot_motion().unwrap();
        assert_eq!(view.origin, [3076.0, -838.0, 64.0]);
        assert_eq!(view.view_height, 0.0);
        assert_eq!(
            c.world.motion.as_ref().unwrap().origin,
            [3076.0, -838.0, 4.0]
        );
    }
    c.world.motion.as_mut().unwrap().view_height = 20.0;
    assert_eq!(c.snapshot_motion().unwrap().origin[2], 24.0);
}
#[test]
fn native_encoded_nickname_is_decoded_without_accepting_controls() {
    assert_eq!(
        decode_connect_name("c3RlYW11c2VyQ2xpZW50"),
        "steamuserClient"
    );
    assert_eq!(decode_connect_name("Player"), "Player");
    assert_eq!(decode_connect_name("cXVpdAo="), "cXVpdAo=");
}
#[test]
fn configured_player_name_is_used_only_after_connect_identity_is_verified() {
    let mut connection = Connection {
        challenge: Some(12345),
        ..Default::default()
    };
    connection.set_player_name(Some("同名玩家".into()));
    let valid = b"\xff\xff\xff\xffconnect fffffffffffffffe \"\\name\\ClientDefault\\xuid\\fffffffffffffffe\\challenge\\12345\\2490\\7\"";
    assert!(connection.handle(valid, 42, 7).is_err());
    assert!(connection.display_name.is_none());
    connection.handle(valid, u64::MAX - 1, 7).unwrap();
    assert_eq!(connection.display_name.as_deref(), Some("同名玩家"));
}
#[test]
fn predicted_inputs_are_queued_until_the_clock_reaches_them_without_losing_transitions() {
    let mut c = Connection {
        last_snapshot_time: 1000,
        ..Default::default()
    };
    for (time, direction) in [(1010, 1), (1030, 0), (1020, 1), (4000, 1)] {
        c.queue_input(codol_protocol::game::channel::usercmd::UserCommand {
            time,
            direction,
            ..Default::default()
        });
    }
    c.consume_inputs(1000);
    assert_eq!(c.input_count, 0);
    c.consume_inputs(1025);
    assert_eq!(c.input_count, 2);
    assert_eq!(c.last_input.as_ref().unwrap().direction, 1);
    c.consume_inputs(1030);
    assert_eq!(c.input_count, 3);
    assert_eq!(c.last_input.as_ref().unwrap().direction, 0);
    c.consume_inputs(5000);
    assert_eq!(c.input_count, 3);
}
#[test]
fn missed_snapshot_ticks_do_not_slow_the_server_clock_or_acknowledge_unapplied_input() {
    let now = Instant::now();
    let mut c = Connection::with_game_state(Arc::new(
        crate::game::session::prepared::synthetic()
            .with_snapshot_frames(60)
            .unwrap(),
    ));
    c.challenge = Some(0x75);
    c.native_sent = 3;
    c.gamestate_update = Some(6);
    c.poll_snapshot(now).unwrap().unwrap();
    let mut packet = c
        .poll_snapshot(now + Duration::from_millis(275))
        .unwrap()
        .unwrap();
    game_wire::xor_stream(&mut packet[8..], 0x75 ^ 5, b"");
    assert_eq!(&packet[9..13], &1275u32.to_le_bytes());
    assert_eq!(&packet[17..21], &1000u32.to_le_bytes());
}
#[test]
fn input_baseline_follows_acknowledged_snapshot_not_latest_view() {
    let mut c = Connection::default();
    let first = codol_protocol::game::channel::usercmd::UserCommand {
        angles: [100, 200, 0],
        weapons: [352, 0, 0],
        ..Default::default()
    };
    let second = codol_protocol::game::channel::usercmd::UserCommand {
        angles: [300, 400, 0],
        buttons: 0x800,
        ..first.clone()
    };
    c.snapshot_input_bases.push_back((4, first.clone()));
    c.snapshot_input_bases.push_back((6, second.clone()));
    assert!(c.input_base_for(3).is_err());
    assert_eq!(c.input_base_for(4).unwrap(), first);
    assert_eq!(c.input_base_for(5).unwrap(), first); // reliable-only message
    assert_eq!(c.input_base_for(6).unwrap(), second);
}
#[test]
fn model_commands_remain_bounded_while_the_local_player_can_initialize() {
    let now = Instant::now();
    let mut c = Connection::with_game_state(Arc::new(
        crate::game::session::prepared::synthetic()
            .with_snapshot_frames(60)
            .unwrap(),
    ));
    c.challenge = Some(0x75);
    c.native_sent = 3;
    c.gamestate_update = Some(6);
    c.model_rules_sent = true;
    for id in 200..260 {
        c.server_commands
            .push(format!("d {} character_part_{id}_tf_m", 2051 + id).as_bytes())
            .unwrap();
    }
    c.model_rules_sequence = c.server_commands.sequence();
    let packet = c.poll_snapshot(now).unwrap().unwrap();
    assert!(packet.len() < game_wire::FRAGMENT_SIZE + 4);
    assert_eq!(c.snapshot_count, 1);
    assert_eq!(c.snapshot_first_sequence, Some(4));
    assert!(c.input_base_for(4).is_ok());
    assert!(c.server_commands.acknowledged() < c.model_rules_sequence);
}
#[test]
fn snapshot_probe_requires_opt_in_initialization_and_has_rate_and_count_limits() {
    let now = Instant::now();
    let mut disabled =
        Connection::with_game_state(Arc::new(crate::game::session::prepared::synthetic()));
    disabled.gamestate_update = Some(6);
    assert!(disabled.poll_snapshot(now).unwrap().is_none());
    let mut c = Connection::with_game_state(Arc::new(
        crate::game::session::prepared::synthetic()
            .with_snapshot_frames(60)
            .unwrap(),
    ));
    c.challenge = Some(0x75);
    c.native_sent = 3;
    assert!(c.poll_snapshot(now).unwrap().is_none());
    c.gamestate_update = Some(5);
    assert!(c.poll_snapshot(now).unwrap().is_none());
    c.gamestate_update = Some(6);
    for i in 0..60u32 {
        if i == 1 {
            c.native_ack = 4;
        }
        let at = now + Duration::from_millis(u64::from(i) * 50);
        let mut packet = c.poll_snapshot(at).unwrap().unwrap();
        assert_eq!(&packet[..4], &(i + 4).to_le_bytes());
        game_wire::xor_stream(&mut packet[8..], 0x75 ^ (i + 4) as u8, b"");
        assert_eq!(packet[14], if i == 0 { 8 } else { 0 });
        assert_eq!(&packet[9..13], &(1000 + i * 50).to_le_bytes());
        // The frozen captured layout places PS.commandTime at body+9.
        // Sending more frames without input must not acknowledge input
        // that the client has not generated yet.
        assert_eq!(&packet[17..21], &1000u32.to_le_bytes());
        assert!(
            c.poll_snapshot(at + Duration::from_millis(49))
                .unwrap()
                .is_none()
        );
    }
    assert!(
        c.poll_snapshot(now + Duration::from_secs(60))
            .unwrap()
            .is_none()
    );
    assert_eq!(c.native_sent, 63);
    assert_eq!(c.native_received, 0); // sending must never invent client ACKs
}
#[test]
fn acknowledged_match_exit_stops_snapshots_but_unacknowledged_exit_is_retried() {
    let now = Instant::now();
    let mut c = Connection::with_game_state(Arc::new(
        crate::game::session::prepared::synthetic()
            .with_snapshot_frames(60)
            .unwrap(),
    ));
    c.challenge = Some(0x75);
    c.native_sent = 3;
    c.gamestate_update = Some(6);
    let leave = c.server_commands.push(b"t EXE_MATCHENDED").unwrap();
    c.match_leave_sequence = Some(leave);
    assert!(c.poll_snapshot(now).unwrap().is_some());
    assert!(
        c.poll_snapshot(now + Duration::from_millis(50))
            .unwrap()
            .is_some()
    );
    c.server_commands.acknowledge(leave).unwrap();
    let count = c.snapshot_count;
    assert!(
        c.poll_snapshot(now + Duration::from_millis(100))
            .unwrap()
            .is_none()
    );
    assert_eq!(c.snapshot_count, count);
}
#[test]
fn normal_play_keeps_the_frame_stream_past_the_diagnostic_frame_limit() {
    let mut c = Connection::with_game_state(Arc::new(
        crate::game::session::prepared::synthetic().with_continuous_play(),
    ));
    c.challenge = Some(0x75);
    c.native_sent = 3;
    c.gamestate_update = Some(6);
    c.snapshot_count = 18000;
    assert!(c.poll_snapshot(Instant::now()).unwrap().is_some());
    assert_eq!(c.snapshot_count, 18001);
}
#[test]
fn stats_and_heartbeat_require_prior_stages_and_all_fragments() {
    let mut connection = Connection::default();
    let heartbeat = b"\xff\xff\xff\xffheartbeat\0";
    assert!(connection.handle(heartbeat, 42, 7).is_err());
    let raw = vec![0; codol_protocol::lobby::profile::SNAPSHOT_BYTES];
    for (index, data) in raw
        .chunks(codol_protocol::game::snapshot::stats::CHUNK_BYTES)
        .enumerate()
    {
        let mut packet = b"\xff\xff\xff\xffstats\0".to_vec();
        packet.extend_from_slice(&[index as u8, 0, 0]);
        packet.extend_from_slice(data);
        if index == 0 {
            assert!(connection.handle(&packet, 42, 7).is_err());
            connection.connected = true;
        }
        let reply = connection.handle(&packet, 42, 7).unwrap().unwrap();
        if index < 26 {
            assert_ne!(reply, b"\xff\xff\xff\xffstatResponse 0\0");
            assert!(connection.handle(heartbeat, 42, 7).is_err());
        } else {
            assert_eq!(reply, b"\xff\xff\xff\xffstatResponse 0\0");
        }
    }
    assert_eq!(connection.stage(), "stats_received");
    assert_eq!(
        connection.handle(heartbeat, 42, 7).unwrap().unwrap(),
        b"\xff\xff\xff\xffheartbeatresponse\0"
    );
}
#[test]
fn challenge_requires_admitted_identity_and_retries_reuse_nonce() {
    let mut connection = Connection::default();
    let request = b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\"\0";
    assert!(connection.handle(request, 1, 7).is_err());
    assert!(connection.challenge.is_none());
    let response = connection
        .handle(request, u64::MAX - 1, 7)
        .unwrap()
        .unwrap();
    assert!(response.starts_with(b"\xff\xff\xff\xffchallengeResponse "));
    assert_eq!(
        connection
            .handle(request, u64::MAX - 1, 7)
            .unwrap()
            .unwrap(),
        response
    );
    assert!(
        connection
            .handle(
                b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\" trailing",
                u64::MAX - 1,
                7
            )
            .is_err()
    );
    assert!(
        connection
            .handle(&[0, 0], u64::MAX - 1, 7)
            .unwrap()
            .is_none()
    );
    assert!(
        connection
            .handle(b"\xff\xff\xff\xffarbitrary", u64::MAX - 1, 7)
            .unwrap()
            .is_none()
    );
}

#[test]
fn connect_binds_both_identities_build_and_nonce_without_duplicate_fields() {
    let mut connection = Connection::default();
    let valid = b"\xff\xff\xff\xffconnect fffffffffffffffe \"\\name\\Lab player\\xuid\\fffffffffffffffe\\challenge\\12345\\2490\\-688906341\"";
    assert!(connection.handle(valid, u64::MAX - 1, -688906341).is_err());
    connection.challenge = Some(12345);
    for (old, new) in [
        ("fffffffffffffffe", "ffffffffffffffff"),
        ("12345", "12346"),
        ("-688906341", "0"),
        ("\\name\\", "\\xuid\\"),
    ] {
        let text = std::str::from_utf8(&valid[4..]).unwrap().replace(old, new);
        let mut damaged = vec![255; 4];
        damaged.extend_from_slice(text.as_bytes());
        assert!(
            connection
                .handle(&damaged, u64::MAX - 1, -688906341)
                .is_err()
        );
        assert!(!connection.connected);
    }
    for _ in 0..2 {
        assert_eq!(
            connection
                .handle(valid, u64::MAX - 1, -688906341)
                .unwrap()
                .unwrap(),
            b"\xff\xff\xff\xffconnectResponse\0"
        );
    }
    assert_eq!(connection.stage(), "connected");
    let mut extra = valid.to_vec();
    extra.extend_from_slice(b" garbage");
    assert!(connection.handle(&extra, u64::MAX - 1, -688906341).is_err());
}

fn eof(sequence: u32, message_ack: u32, reliable_ack: u32) -> Vec<u8> {
    let mut wire = sequence.to_le_bytes().to_vec();
    wire.push(0);
    wire.extend_from_slice(&message_ack.to_le_bytes());
    wire.extend_from_slice(&reliable_ack.to_le_bytes());
    wire.push(3 ^ 0x75 ^ message_ack as u8);
    wire
}

#[test]
fn probes_keep_client_ack_zero_terminate_stream_and_require_valid_progress() {
    let mut connection = Connection {
        challenge: Some(0x75),
        connected: true,
        ..Default::default()
    };
    assert!(connection.handle(&eof(1, 0, 0), 1, 7).is_err());
    let profile = vec![0; codol_protocol::lobby::profile::SNAPSHOT_BYTES];
    for (i, chunk) in profile
        .chunks(codol_protocol::game::snapshot::stats::CHUNK_BYTES)
        .enumerate()
    {
        let mut part = vec![i as u8, 0, 0];
        part.extend_from_slice(chunk);
        connection.stats.accept(&part).unwrap();
    }
    assert!(connection.handle(&eof(1, 1, 0), 1, 7).is_err());
    let first = connection.handle(&eof(1, 0, 0), 1, 7).unwrap().unwrap();
    assert_eq!(&first[..8], &[1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(first[8], 6 ^ 0x75 ^ 1);
    assert!(connection.handle(&eof(1, 0, 0), 1, 7).is_err());
    assert!(connection.handle(&eof(2, 1, 1), 1, 7).is_err());
    assert_eq!(connection.native_received, 1); // rejection must not consume sequence
    let mut second = connection.handle(&eof(2, 1, 0), 1, 7).unwrap().unwrap();
    assert_eq!(&second[..8], &[2, 0, 0, 0, 0, 0, 0, 0]);
    game_wire::xor_stream(&mut second[8..], 0x75 ^ 2, b"");
    assert_eq!(&second[8..], &[1, 1, 0, 0, 0, 0, 6]);
    assert!(connection.handle(&eof(3, 3, 1), 1, 7).is_err());
    assert!(connection.handle(&eof(3, 2, 1), 1, 7).unwrap().is_none());
    assert_eq!(connection.stage(), "reliable_probe_acknowledged");
    assert!(connection.handle(&eof(4, 1, 0), 1, 7).is_err());
}

#[test]
fn resource_collection_keeps_transport_alive_without_publishing_gameplay_snapshots() {
    let state = crate::game::session::prepared::synthetic()
        .with_continuous_play()
        .resource_initialization()
        .unwrap();
    let mut c = Connection::with_game_state(Arc::new(state)).with_resource_collection();
    c.challenge = Some(0x75);
    c.connected = true;
    let profile = vec![0; codol_protocol::lobby::profile::SNAPSHOT_BYTES];
    for (i, chunk) in profile
        .chunks(codol_protocol::game::snapshot::stats::CHUNK_BYTES)
        .enumerate()
    {
        let mut part = vec![i as u8, 0, 0];
        part.extend_from_slice(chunk);
        c.stats.accept(&part).unwrap();
    }
    c.handle(&eof(1, 0, 0), 1, 7).unwrap().unwrap();
    c.handle(&eof(2, 1, 0), 1, 7).unwrap().unwrap();
    c.handle(&eof(3, 2, 1), 1, 7).unwrap().unwrap();
    let mut keepalive = c.handle(&eof(4, 3, 1), 1, 7).unwrap().unwrap();
    game_wire::xor_stream(&mut keepalive[8..], 0x75 ^ 4, b"");
    assert_eq!(&keepalive[8..], &[game_wire::SERVER_EOF]);
    c.gamestate_update = Some(6);
    assert!(c.poll_snapshot(Instant::now()).unwrap().is_none());
    assert!(c.pending_inputs.is_empty());
    assert_eq!(c.snapshot_count, 0);
}
#[test]
fn gamestate_requires_probe_ack_and_client_commands_commit_atomically() {
    let mut connection =
        Connection::with_game_state(Arc::new(crate::game::session::prepared::synthetic()));
    connection.challenge = Some(0x75);
    connection.connected = true;
    let profile = vec![0; codol_protocol::lobby::profile::SNAPSHOT_BYTES];
    for (i, chunk) in profile
        .chunks(codol_protocol::game::snapshot::stats::CHUNK_BYTES)
        .enumerate()
    {
        let mut part = vec![i as u8, 0, 0];
        part.extend_from_slice(chunk);
        connection.stats.accept(&part).unwrap();
    }
    assert!(
        connection
            .handle(b"\xff\xff\xff\xffgamestateupdate 1\0", 1, 7)
            .is_err()
    );
    connection.handle(&eof(1, 0, 0), 1, 7).unwrap();
    connection.handle(&eof(2, 1, 0), 1, 7).unwrap();
    assert!(connection.handle(&eof(3, 2, 0), 1, 7).unwrap().is_none());
    let mut packet = connection.handle(&eof(4, 2, 1), 1, 7).unwrap().unwrap();
    assert_eq!(packet[..8], [3, 0, 0, 0, 0, 0, 0, 0]);
    game_wire::xor_stream(&mut packet[8..], 0x75 ^ 3, b"");
    assert_eq!(packet[8], 3);
    assert_eq!(packet.last(), Some(&6));
    assert_eq!(connection.stage(), "gamestate_sent");
    connection
        .handle(b"\xff\xff\xff\xffgamestateupdate 1\0", 1, 7)
        .unwrap();
    assert_eq!(connection.stage(), "gamestate_parser_entered");
    connection
        .handle(b"\xff\xff\xff\xffgamestateupdate 6\0", 1, 7)
        .unwrap();
    connection
        .handle(b"\xff\xff\xff\xffgamestateupdate 3\0", 1, 7)
        .unwrap();
    assert_eq!(connection.stage(), "gamestate_initialized");
    assert_eq!(connection.gamestate_progress(), Some(6));
    assert!(
        connection
            .handle(b"\xff\xff\xff\xffgamestateupdate 7\0", 1, 7)
            .is_err()
    );
    // One reliable command, then EOF in the same pool byte (2 | 3<<3).
    let mut wire = eof(5, 3, 1);
    wire[4] = 1;
    wire.truncate(13);
    let mut body = vec![0x1a, 1, 0, 0, 0, b'a', 0];
    game_wire::xor_stream(&mut body, 0x75 ^ 3 ^ 1, b"");
    wire.extend_from_slice(&body);
    let mut broken = wire.clone();
    broken.pop();
    assert!(connection.handle(&broken, 1, 7).is_err());
    assert_eq!(connection.client_commands.sequence(), 0);
    let mut response = connection.handle(&wire, 1, 7).unwrap().unwrap();
    assert_eq!(response[..8], [4, 0, 0, 0, 1, 0, 0, 0]);
    game_wire::xor_stream(&mut response[8..], 0x75 ^ 4, b"a");
    assert_eq!(response[8..], [6]);
    assert_eq!(connection.stage(), "gamestate_acknowledged");
    wire[..4].copy_from_slice(&6u32.to_le_bytes());
    assert!(connection.handle(&wire, 1, 7).unwrap().is_none());
    assert_eq!(connection.client_commands.sequence(), 1);
}

#[test]
fn body_changes_require_an_acknowledged_matching_appearance_baseline() {
    let mut c = preparing_connection();
    c.native_sent = 8;
    c.native_ack = 8;
    let ordinary = c.world.tdm.as_ref().unwrap().baselines()[0];
    let mut suit = ordinary;
    suit.model = 109;
    suit.attachment_models = [0; 10];
    suit.attachment_tags = [0; 10];
    c.world_history.push_back(WorldHistory {
        sequence: 7,
        local: Some(ordinary),
        ..Default::default()
    });
    assert!(c.world_baseline(0, false, Some(suit), &[]).is_none());
    c.world_history.push_back(WorldHistory {
        sequence: 9,
        local: Some(suit),
        ..Default::default()
    });
    assert!(c.world_baseline(0, false, Some(suit), &[]).is_none());
    c.native_ack = 9;
    c.native_sent = 9;
    let mut firing = suit;
    firing.weapon = 187;
    firing.origin[0] += 40.0;
    assert_eq!(
        c.world_baseline(0, false, Some(firing), &[])
            .unwrap()
            .sequence,
        9
    );
    // Respawn may safely select the acknowledged ordinary appearance again.
    assert_eq!(
        c.world_baseline(0, false, Some(ordinary), &[])
            .unwrap()
            .sequence,
        7
    );
    c.world_history.pop_front();
    assert!(c.world_baseline(0, false, Some(ordinary), &[]).is_none());
}
