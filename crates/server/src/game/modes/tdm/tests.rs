use super::*;
use crate::game::ai::loadout::BotLoadout;
#[test]
#[ignore = "uses the locally captured Killhouse ledge to verify Bot falling authority"]
fn captured_bot_fall_uses_real_map_collision_and_preserves_team_scores() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let template = crate::game::session::prepared::PreparedGameState::load(
        &root.join("tdm-killhouse-hud-state.json"),
    )
    .unwrap();
    let profiles = root.join("../client-work/server-data/profiles/1002");
    let profiles: Vec<_> = std::fs::read_dir(profiles)
        .unwrap()
        .map(|x| x.unwrap().path())
        .filter(|p| p.join("public.bin").is_file() && p.join("inventory.json").is_file())
        .collect();
    assert_eq!(
        profiles.len(),
        1,
        "this local capture fixture requires one account"
    );
    let public = std::fs::read(profiles[0].join("public.bin")).unwrap();
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profiles[0].join("inventory.json")).unwrap())
            .unwrap();
    let owned: Vec<u32> = serde_json::from_value(inventory["items"].clone()).unwrap();
    let state = template
        .for_initial_backpack(&public)
        .unwrap()
        .for_profile(&public, &owned)
        .unwrap();
    let route: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("landing-diagnostic-route.json")).unwrap())
            .unwrap();
    let origin: [f32; 3] = serde_json::from_value(route["origin"].clone()).unwrap();
    let end: [f32; 3] = serde_json::from_value(route["end"].clone()).unwrap();
    let collision = state.collision.as_ref().unwrap();
    let mut game = state.tdm_match().unwrap().unwrap();
    for b in &mut game.bots {
        b.health = 0;
        b.snapshot.alive = false;
    }
    let bot = &mut game.bots[0];
    bot.health = 100;
    bot.snapshot.alive = true;
    bot.snapshot.origin = origin;
    bot.controller.respawn(1000, origin, [0; 3]).unwrap();
    let mut local = state.local_player_snapshot(false).unwrap();
    local.alive = false;
    let weapon = state.match_bot_weapon().unwrap();
    for now in (1050..=4000).step_by(50) {
        // This fixture guides the actor over the captured ledge; it does
        // not validate navigation's preferred route or native visuals.
        game.next_route_client = 1;
        game.bots[0].steering = Some(crate::game::ai::navigation::Waypoint::walk(end));
        game.bots[0].steering_time = now;
        game.step(
            now,
            (local, 70.0, 0),
            weapon,
            collision,
            |s, f, fire, toggle| state.animate_enemy(s, f, fire, toggle),
        )
        .unwrap();
    }
    let kills = game.take_kills();
    assert_eq!(kills.len(), 1);
    assert!(kills[0].falling);
    assert_eq!(game.bots[0].health, 0);
    assert_eq!(game.scoreboard(1).team_scores, [0; 4]);
    let proof = serde_json::json!({"client_sha256":codol_protocol::CLIENT_SHA256,"read_only_captured_resources":true,"native_visual_verified":false,"map":state.map(),"ncs_crc":state.ncs_crc(),"source_route":route,"victim":kills[0].victim.client_index,"fall_time":kills[0].time,"death_origin":kills[0].victim.origin,"held_weapon":kills[0].weapon,"deaths":game.scoreboard(1).rows[1].deaths,"personal_score":game.scoreboard(1).rows[1].score,"team_scores":game.scoreboard(1).team_scores});
    std::fs::write(
        root.join("bot-landing-captured-proof.json"),
        serde_json::to_vec_pretty(&proof).unwrap(),
    )
    .unwrap();
}
#[test]
fn room_team_assignment_selects_bot_outfits_and_their_pinned_baselines() {
    let outfit: BotLoadout = serde_json::from_value(serde_json::json!({
        "class":"botclass1","weapon_name":"m4_mp","personality":"default",
        "attachment_models":[3,2,5,6,0,0,0,0,0,0],
        "animation":{"model":4,"origin":[0,0,0],"bounds_min":[-15,-15,0],"bounds_max":[15,15,70],
            "idle_animation":395,"walk_animation":243,"fire_animation":616,"death_animation":47,
            "move_speed":190,"angles":[0,0,0],"weapon":352,"source_sha256":"a".repeat(64)},
        "outfits":[{"model":201,"attachment_models":[202,203,204,205,0,0,0,0,0,0]},
                   {"model":301,"attachment_models":[302,303,304,305,0,0,0,0,0,0]}]
    }))
    .unwrap();
    let weapon = crate::game::weapons::held::HeldWeapons::new(
        codol_protocol::game::snapshot::Equipment {
            perks: Default::default(),
            abilities: Default::default(),
            offhands: [None, None],
            offhand_weapon: 0,
            grenade_time: 0,
            move_multiplier: 1.0,
            weapon: 352,
            inventory_weapon: None,
            alternate_weapon: None,
            alternates: [None, None],
            fire_count: 0,
            queued_burst: false,
            barrel_flags: 0,
            spread_scale: 0.0,
            needs_rechamber: false,
            ammo: None,
            viewmodel: None,
            firing: None,
            aim: None,
            secondary: None,
            events: None,
        },
        Some(80),
        None,
        None,
    );
    for swapped in [false, true] {
        let mut c = config();
        c.roster = vec![outfit.clone(); 11];
        if swapped {
            c.teams = c.teams.map(|t| 3 - t);
        }
        let mut game = TdmMatch::new(c, template(), 190.0, &world()).unwrap();
        game.configure_roster(vec![weapon.clone(); 11]).unwrap();
        for (actor, baseline) in game.snapshots().iter().zip(game.baselines()) {
            let expected = if actor.team == 1 { 201 } else { 301 };
            assert_eq!(actor.model, expected);
            assert_eq!(actor.attachment_models[0], expected + 1);
            assert_eq!(baseline.model, actor.model);
            assert_eq!(baseline.attachment_models, actor.attachment_models);
        }
    }
}
fn world() -> Collision {
    let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
        "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}],"triangles":[]});
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
    Collision::load(file.path()).unwrap()
}
fn template() -> EnemySnapshot {
    EnemySnapshot {
        entity_number: 0,
        client_index: 0,
        team: 2,
        model: 4,
        attachment_models: [0; 10],
        attachment_tags: [0; 10],
        origin: [0.0; 3],
        angles: [0; 3],
        legs_animation: 395,
        torso_animation: 395,
        weapon: 352,
        events: EventQueue::default(),
        alive: true,
    }
}
fn config() -> MatchConfig {
    MatchConfig {
        spawn_angles: None,
        player_count: 12,
        spawns: [
            (0..6).map(|i| [300.0, i as f32 * 100.0, 0.0]).collect(),
            (0..6).map(|i| [-300.0, i as f32 * 100.0, 0.0]).collect(),
        ],
        score_limit: 7500,
        time_limit_msec: 600000,
        teams: default_teams(),
        roster: Vec::new(),
        models: Vec::new(),
    }
}
#[test]
fn sentry_consumes_only_successful_placement_and_retains_owner_credit_after_death() {
    let collision = world();
    let resources = crate::game::rewards::deployables::test_resources();
    let mut game = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
    game.acceptance_stationary = true;
    let catalog: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/killstreaks-3.3.3.3.json"
    )))
    .unwrap();
    let item = catalog["marketplace"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["native_id"] == 16)
        .unwrap()["item_id"]
        .as_u64()
        .unwrap() as u32;
    let offset = catalog["class_offsets"][0][0].as_u64().unwrap() as usize;
    let mut profile = vec![0; offset + 12];
    profile[offset..offset + 4].copy_from_slice(&(item + 1).to_le_bytes());
    let preview =
        crate::game::rewards::killstreaks::Previews::from_profile(&profile, 0, &[item], 0.0)
            .unwrap();
    game.streaks.configure_radios(0, preview);
    game.streaks.enable_supported(&resources);
    let local = template();
    assert!(!game.deploy_sentry(2100, local, 70.0, &resources, &collision));
    for victim in 6..=10 {
        game.damage_bots(2200, 0, 352, &[(victim, 100, false)], false);
    }
    assert!(game.streaks.can_radio(0, 2, 16, 2200));
    collision.replace_props(&[crate::game::world::props::Collider::new(
        256,
        [96.0, 0.0, 0.0],
        [0; 3],
        [-30.0, -30.0, 0.0],
        [30.0, 30.0, 80.0],
    )]);
    assert!(!game.deploy_sentry(2300, local, 70.0, &resources, &collision));
    assert!(game.streaks.can_radio(0, 2, 16, 2300));
    assert!(game.deployables.snapshots().is_empty());
    collision.replace_props(&[]);
    assert!(game.deploy_sentry(2400, local, 70.0, &resources, &collision));
    assert!(!game.streaks.can_radio(0, 2, 16, 2400));
    assert!(!game.deploy_sentry(2401, local, 70.0, &resources, &collision));
    game.player_killed(
        2500,
        local,
        PlayerHit {
            damage: 100,
            attacker: 11,
            weapon: 352,
            headshot: false,
        },
    );
    let mut dead = local;
    dead.alive = false;
    let weapon = BotWeapon {
        weapon: 352,
        fire_interval_msec: 80,
        ballistics: crate::game::combat::Ballistics {
            damage: 20,
            range: 1000.0,
            near_range: 1000.0,
            location_multipliers: None,
            falloff: None,
            pellets: 1,
            spread_degrees: 0.0,
        },
    };
    game.step(5300, (dead, 70.0, 0), weapon, &collision, |s, _, _, _| s)
        .unwrap();
    for time in (6400..=6600).step_by(50) {
        game.step_deployables(time, (dead, 70.0, 0), false, &collision);
    }
    let kills = game.take_kills();
    assert!(
        kills
            .iter()
            .any(|k| k.attacker == 0 && k.weapon == 347 && k.victim.client_index == 6)
    );
    assert_eq!(game.rows[0].kills, 6);
    assert_eq!(game.streaks.snapshot(0, 0).progress, 0);
    assert_eq!(game.world_models()[0].entity_number, 384);
}
#[test]
fn map_headings_follow_team_and_death_selected_spawn_for_players_and_bots() {
    let c = world();
    let mut config = config();
    let angles: [Vec<_>; 2] = std::array::from_fn(|team| {
        (0..6)
            .map(|index| [0, (team * 20000 + index * 1000) as u16, 0])
            .collect()
    });
    config.spawn_angles = Some(angles.clone());
    let mut game = TdmMatch::new(config.clone(), template(), 190.0, &c).unwrap();
    assert_eq!(game.player_spawn_angles(), angles[1][0]);
    game.rows[0].deaths = 2;
    assert_eq!(game.player_spawn_angles(), angles[1][2]);
    let mut ordinal = [0, 1];
    for bot in &game.bots {
        let team = usize::from(bot.snapshot.team - 1);
        assert_eq!(bot.snapshot.angles, angles[team][ordinal[team]]);
        ordinal[team] += 1;
    }
    config.teams = config.teams.map(|t| 3 - t);
    let game = TdmMatch::new(config.clone(), template(), 190.0, &c).unwrap();
    assert_eq!(game.player_spawn_angles(), angles[0][0]);
    config.spawn_angles.as_mut().unwrap()[0].pop();
    assert!(config.validate(&c).is_err());
}
#[test]
fn replay_support_expiry_uses_recording_time_even_when_an_actor_did_not_move() {
    let c = world();
    let mut m = TdmMatch::new(config(), template(), 190.0, &c).unwrap();
    for bot in &mut m.bots {
        bot.weapon = Some(crate::game::weapons::held::tests::scavenger_player(90, 180));
    }
    let mut profile = vec![0; codol_protocol::lobby::profile::SNAPSHOT_BYTES];
    let layout: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/killstreaks-3.3.3.3.json"
    )))
    .unwrap();
    let offset = layout["class_offsets"][0][0].as_u64().unwrap() as usize;
    profile[offset..offset + 4].copy_from_slice(&60079i32.to_le_bytes());
    let previews =
        crate::game::rewards::killstreaks::Previews::from_profile(&profile, 0, &[60078], 0.0)
            .unwrap();
    m.streaks.configure_radios(0, previews);
    for n in 1..=4 {
        m.streaks.spawn(1);
        m.streaks.credit(n * 1000, 1, n, Some(0));
    }
    assert!(m.streaks.try_radio(0, m.teams()[0], 3, 5000));
    let enemy = m
        .bots
        .iter()
        .find(|b| b.snapshot.team != m.teams()[0])
        .unwrap()
        .snapshot
        .client_index;
    assert_eq!(
        m.replay_views(5001)
            .iter()
            .find(|v| v.client == enemy)
            .unwrap()
            .radar_flags,
        Some(2)
    );
    assert_eq!(
        m.replay_views(35000)
            .iter()
            .find(|v| v.client == enemy)
            .unwrap()
            .radar_flags,
        Some(0)
    );
}
#[test]
fn staged_stationary_fire_uses_ordinary_weapons_and_damage_without_moving_actors() {
    let collision = world();
    let mut m = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
    m.acceptance_stationary = true;
    m.acceptance_fire_after_msec = Some(4000);
    m.advance_clock(1000);
    let mut held = crate::game::weapons::held::tests::scavenger_player(90, 180);
    held.set_ballistics(game_combat::Ballistics {
        damage: 33,
        range: 8192.0,
        near_range: 500.0,
        location_multipliers: None,
        falloff: None,
        pellets: 1,
        spread_degrees: 0.0,
    });
    for bot in &mut m.bots {
        bot.weapon = Some(held.clone());
        bot.initial_weapon = Some(held.clone());
    }
    let before = m.snapshots();
    let mut local = template();
    local.origin = [200.0, 0.0, 0.0];
    assert!(m.stationary_shots(4999, local, 100).is_empty());
    let mut tick = Tick::default();
    for now in (5000..5200).step_by(50) {
        let shots = m.stationary_shots(now, local, 100);
        m.resolve_bot_shots(now, (local, 80.0, 100), shots, &collision, &mut tick);
    }
    assert!(
        tick.hits
            .iter()
            .any(|h| h.damage > 0 && m.teams()[usize::from(h.attacker)] != local.team)
    );
    assert_eq!(
        before.iter().map(|b| b.origin).collect::<Vec<_>>(),
        m.snapshots().iter().map(|b| b.origin).collect::<Vec<_>>()
    );
    assert!(m.stationary_shots(5250, local, 0).is_empty());
}
#[test]
fn stationary_acceptance_keeps_six_enemies_still_and_respawns_real_deaths() {
    let c = world();
    let mut game = TdmMatch::new(config(), template(), 190.0, &c).unwrap();
    game.acceptance_stationary = true;
    game.streaks.enable();
    assert_eq!(
        game.bots
            .iter()
            .filter(|b| b.snapshot.team != game.teams()[0])
            .count(),
        6
    );
    let before = game.snapshots();
    let fallback = BotWeapon {
        weapon: 352,
        fire_interval_msec: 80,
        ballistics: game_combat::Ballistics {
            damage: 33,
            range: 8192.0,
            near_range: 500.0,
            location_multipliers: None,
            falloff: None,
            pellets: 1,
            spread_degrees: 0.0,
        },
    };
    for now in (1000..=5000).step_by(50) {
        let tick = game
            .step(now, (template(), 70.0, 100), fallback, &c, |_, _, _, _| {
                panic!("stationary bots must not animate an AI step")
            })
            .unwrap();
        assert!(tick.hits.is_empty());
    }
    assert_eq!(game.snapshots(), before);
    let enemy = game
        .bots
        .iter()
        .position(|b| b.snapshot.team != game.teams()[0])
        .unwrap();
    let client = game.bots[enemy].snapshot.client_index;
    game.damage_bots(5100, 0, 352, &[(client, 100, false)], false);
    assert_eq!(game.streaks.snapshot(0, 323).progress, 1);
    assert!(!game.bots[enemy].snapshot.alive);
    game.step(8100, (template(), 70.0, 100), fallback, &c, |s, _, _, _| s)
        .unwrap();
    assert!(game.bots[enemy].snapshot.alive);
    assert_eq!(game.bots[enemy].health, 100);
    assert_eq!(game.bots[enemy].snapshot.origin, before[enemy].origin);
    assert_eq!(game.scoreboard(1).rows[0].kills, 1);
}
#[test]
fn nuclear_end_kills_live_actors_atomically_and_overrides_a_losing_team_score() {
    let mut m = TdmMatch::new(config(), template(), 190.0, &world()).unwrap();
    m.streaks.enable();
    let enemy = m
        .bots
        .iter()
        .position(|b| b.snapshot.team != m.teams()[0])
        .unwrap();
    for n in 1..=25 {
        let victim = m.bots[enemy].snapshot;
        m.streaks.spawn(victim.client_index);
        m.credit(n * 1000, victim, 0, 352, false, false);
    }
    assert_eq!(m.rows[0].kills, 25);
    assert_eq!(m.rows[0].killstreak, 25);
    assert!(m.streaks.owned(0));
    assert!(m.streaks.try_use(0, 26000));
    m.rules.scores[1] = 5000;
    m.advance_clock(1000);
    m.advance_clock(601000);
    assert!(!m.rules.finished);
    let mut local = template();
    local.client_index = 0;
    local.team = m.teams()[0];
    local.alive = true;
    m.finish_nuke(37500, 0, 334, local);
    assert!(m.rules.finished);
    assert_eq!(m.winning_team(), local.team);
    assert_eq!(m.result(), "Allies win");
    assert_eq!(m.rows[0].deaths, 1);
    assert_eq!(m.rows[0].killstreak, 25);
    assert!(m.bots.iter().all(|b| b.health == 0));
    let board = m.scoreboard(42);
    m.finish_nuke(40000, 0, 334, local);
    assert_eq!(m.scoreboard(42), board);
    assert_eq!(
        m.take_kills().iter().filter(|k| k.weapon == 334).count(),
        12
    );
}
#[test]
fn four_player_world_keeps_only_its_active_actors_and_preserves_full_roster_requirements() {
    let collision = world();
    let mut small = config();
    small.player_count = 4;
    small.teams = [2, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    small
        .spawns
        .iter_mut()
        .for_each(|starts| starts.truncate(2));
    let game = TdmMatch::new(small.clone(), template(), 190.0, &collision).unwrap();
    assert_eq!(
        game.snapshots()
            .iter()
            .map(|s| s.client_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    let board = game.scoreboard(42);
    assert_eq!(board.rows.len(), 4);
    assert_eq!(board.rows[0].user_id, 42);
    assert_eq!(game.actors(template(), 70.0).len(), 4);
    let mut bad = small.clone();
    bad.teams[4] = 1;
    assert!(bad.validate(&collision).is_err());
    bad = small.clone();
    bad.teams[2] = 2;
    assert!(bad.validate(&collision).is_err());
    small.player_count = 12;
    small.teams = default_teams();
    assert!(small.validate(&collision).is_err());
}
#[test]
fn fatal_fall_scores_once_as_an_environmental_death_without_awarding_a_kill() {
    let mut game = TdmMatch::new(config(), template(), 190.0, &world()).unwrap();
    game.actor_fell(2000, template());
    let board = game.scoreboard(1);
    assert_eq!(
        (
            board.rows[0].kills,
            board.rows[0].deaths,
            board.rows[0].score
        ),
        (0, 1, 0)
    );
    assert_eq!(board.team_scores[2], 0);
    let kills = game.take_kills();
    assert_eq!(kills.len(), 1);
    assert!(kills[0].falling && !kills[0].headshot && !kills[0].melee);
    assert_eq!(kills[0].weapon, 352);
    assert_eq!(kills[0].attacker, kills[0].victim.client_index);
    game.rules.finished = true;
    game.actor_fell(2100, template());
    assert!(game.take_kills().is_empty());
}
#[test]
fn bot_fall_applies_its_perk_and_runs_the_normal_death_respawn_lifecycle() {
    let c = world();
    for lightweight in [false, true] {
        let mut game = TdmMatch::new(config(), template(), 190.0, &c).unwrap();
        // Isolate one actor's physical fall from gunfire by retaining no
        // other living targets in this simulation fixture.
        for b in &mut game.bots {
            b.health = 0;
            b.snapshot.alive = false;
        }
        let bot = &mut game.bots[0];
        bot.health = 100;
        bot.snapshot.alive = true;
        bot.snapshot.origin = [0.0, 0.0, 350.0];
        bot.controller
            .respawn(1000, bot.snapshot.origin, [0; 3])
            .unwrap();
        let mut held = crate::game::weapons::held::tests::scavenger_player(90, 180);
        if lightweight {
            held.set_perks(
                crate::game::status::perks::Perks::selected([60041, 0, 0], &[60041]).unwrap(),
            );
        }
        bot.weapon = Some(held.clone());
        bot.initial_weapon = Some(held);
        let mut local = template();
        local.alive = false;
        let fallback = BotWeapon {
            weapon: 352,
            fire_interval_msec: 80,
            ballistics: game_combat::Ballistics {
                damage: 33,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
        };
        let animate = |mut s: EnemySnapshot, f: &crate::game::ai::controller::BotFrame, _, _| {
            s.origin = f.motion.origin;
            s.angles = f.angles;
            s
        };
        for now in (1050..=3000).step_by(50) {
            game.step(now, (local, 70.0, 0), fallback, &c, animate)
                .unwrap();
        }
        let board = game.scoreboard(1);
        if lightweight {
            assert_eq!(game.bots[0].health, 100);
            assert!(game.bots[0].snapshot.alive);
            assert_eq!(board.rows[1].deaths, 0);
            assert!(game.take_kills().is_empty());
        } else {
            assert_eq!(game.bots[0].health, 0);
            assert!(!game.bots[0].snapshot.alive);
            assert_eq!(
                (
                    board.rows[1].kills,
                    board.rows[1].deaths,
                    board.rows[1].score
                ),
                (0, 1, 0)
            );
            assert_eq!(board.team_scores, [0; 4]);
            let kills = game.take_kills();
            assert_eq!(kills.len(), 1);
            assert!(kills[0].falling);
            game.step(6500, (local, 70.0, 0), fallback, &c, animate)
                .unwrap();
            assert_eq!(game.bots[0].health, 100);
            assert!(game.bots[0].snapshot.alive);
            assert_eq!(game.scoreboard(1).rows[1].deaths, 1);
            assert!(game.take_kills().is_empty());
        }
    }
}
#[test]
fn perception_obeys_team_field_of_view_close_awareness_and_walls() {
    let collision = world();
    let actor = |client, team, x, y| Actor {
        client,
        team,
        origin: [x, y, 0.0],
        height: 70.0,
        yaw: 0.0,
    };
    let allies = actor(1, 2, 100.0, 0.0);
    let behind = actor(2, 1, -500.0, 0.0);
    let front = actor(3, 1, 700.0, 0.0);
    assert_eq!(
        visible_enemy(
            [0.0; 3],
            0,
            2,
            &[allies, behind, front],
            1000,
            &crate::game::weapons::smoke::Smoke::default(),
            &collision
        )
        .unwrap()
        .0,
        3
    );
    assert!(
        visible_enemy(
            [0.0; 3],
            0,
            2,
            &[allies, behind],
            1000,
            &crate::game::weapons::smoke::Smoke::default(),
            &collision
        )
        .is_none()
    );
    assert_eq!(
        visible_enemy(
            [0.0; 3],
            0,
            2,
            &[actor(4, 1, -100.0, 0.0)],
            1000,
            &crate::game::weapons::smoke::Smoke::default(),
            &collision
        )
        .unwrap()
        .0,
        4
    );
    // An enemy under the solid floor cannot be acquired even at close range.
    let underground = Actor {
        origin: [0.0, 0.0, -100.0],
        ..front
    };
    assert!(
        visible_enemy(
            [0.0; 3],
            0,
            2,
            &[underground],
            1000,
            &crate::game::weapons::smoke::Smoke::default(),
            &collision
        )
        .is_none()
    );
}
#[test]
fn perception_loses_smoke_occluded_opponents_and_recovers_after_optical_fade() {
    let collision = world();
    let opponent = Actor {
        client: 1,
        team: 1,
        origin: [300.0, 0.0, 0.0],
        height: 70.0,
        yaw: 0.0,
    };
    let mut smoke = crate::game::weapons::smoke::Smoke::default();
    smoke.spawn(&crate::game::weapons::grenade::Explosion {
        time: 1000,
        origin: [150.0, 0.0, 2.0],
        normal: [0.0, 0.0, 1.0],
        definition: crate::game::weapons::grenade::captured("smoke_grenade_mp"),
    });
    assert!(visible_enemy([0.0; 3], 0, 2, &[opponent], 1000, &smoke, &collision).is_some());
    assert!(visible_enemy([0.0; 3], 0, 2, &[opponent], 5000, &smoke, &collision).is_none());
    assert!(visible_enemy([0.0; 3], 0, 2, &[opponent], 14000, &smoke, &collision).is_some());
    assert!(!smoke.snapshots(14000).is_empty());
    let outside = Actor {
        origin: [300.0, 400.0, 0.0],
        ..opponent
    };
    assert!(
        visible_enemy(
            [-500.0, 400.0, 0.0],
            0,
            2,
            &[outside],
            5000,
            &smoke,
            &collision
        )
        .is_some()
    );
}
#[test]
fn tactical_notifications_exclude_team_mates_and_death_resets_the_native_named_state() {
    let collision = world();
    let local = template();
    let mut game = TdmMatch::new(config(), local, 190.0, &collision).unwrap();
    let enemy = game
        .bots
        .iter()
        .position(|b| b.snapshot.team != local.team)
        .unwrap();
    let ally = game
        .bots
        .iter()
        .position(|b| b.snapshot.team == local.team)
        .unwrap();
    for bot in &mut game.bots {
        bot.snapshot.origin = [2000.0, 0.0, 0.0];
    }
    game.bots[enemy].snapshot.origin = [128.0, 0.0, 0.0];
    game.bots[ally].snapshot.origin = [128.0, 0.0, 0.0];
    let explosion = crate::game::weapons::grenade::Explosion {
        time: 1000,
        origin: [128.0, 0.0, 60.0],
        normal: [0.0, 0.0, 1.0],
        definition: crate::game::weapons::grenade::captured("flash_grenade_mp"),
    };
    assert_eq!(game.player_tactical(1000, &explosion, local, &collision), 1);
    assert!(game.bots[enemy].tactical.blind(1000));
    assert!(!game.bots[ally].tactical.blind(1000));
    let victim = game.bots[enemy].snapshot;
    game.credit(1200, victim, 0, 89, false, false);
    assert_eq!(
        game.bots[enemy].tactical.snapshot(1200),
        Some(
            codol_protocol::game::snapshot::shellshock::ShellShockSnapshot {
                index: 5,
                begin_time: 0,
                duration_msec: 0
            }
        )
    );
    assert!(!game.bots[enemy].tactical.blind(1200));
}
#[test]
fn gas_applies_owner_credit_and_mask_immunity_then_clears_on_death_respawn_and_finish() {
    let collision = world();
    let mut game = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
    for bot in &mut game.bots {
        bot.snapshot.origin = [2000.0, 0.0, 0.0];
    }
    let enemy = game
        .bots
        .iter()
        .position(|b| b.snapshot.team != template().team)
        .unwrap();
    let ally = game
        .bots
        .iter()
        .position(|b| b.snapshot.team == template().team)
        .unwrap();
    game.bots[enemy].snapshot.origin = [0.0; 3];
    game.bots[ally].snapshot.origin = [0.0; 3];
    let mut gas = crate::game::status::gas::Gas::default();
    let mut persistent = crate::game::weapons::smoke::Smoke::default();
    assert!(gas.deploy(
        &crate::game::weapons::grenade::Explosion {
            time: 2000,
            origin: [0.0, 0.0, 2.0],
            normal: [0.0, 0.0, 1.0],
            definition: crate::game::weapons::grenade::captured("tabun_gas_mp")
        },
        0,
        template().team,
        &collision,
        &mut persistent
    ));
    let mut held = crate::game::weapons::held::tests::scavenger_player(90, 180);
    held.set_perks(crate::game::status::perks::Perks::selected([0, 0, 60137], &[60137]).unwrap());
    assert!(held.gas_protected());
    game.bots[enemy].weapon = Some(held);
    assert_eq!(game.advance_gas(2000, &gas, &collision), None);
    assert_eq!(game.bots[enemy].health, 100);
    assert!(game.bots[enemy].tactical.snapshot(2000).is_none());
    game.bots[enemy]
        .weapon
        .as_mut()
        .unwrap()
        .set_perks(Default::default());
    assert_eq!(
        game.advance_gas(2100, &gas, &collision),
        Some(codol_protocol::game::snapshot::hit_feedback::HitKind::Hit)
    );
    assert_eq!(game.bots[enemy].health, 87);
    assert_eq!(game.bots[ally].health, 100);
    assert_eq!(game.advance_gas(2100, &gas, &collision), None);
    for now in (3100..=9100).step_by(1000) {
        game.advance_gas(now, &gas, &collision);
    }
    assert_eq!(game.bots[enemy].health, 0);
    assert!(!game.bots[enemy].snapshot.alive);
    assert_eq!(
        game.bots[enemy]
            .tactical
            .snapshot(9100)
            .unwrap()
            .duration_msec,
        0
    );
    let kills = game.take_kills();
    assert_eq!(kills.len(), 1);
    assert_eq!((kills[0].attacker, kills[0].weapon), (0, 251));
    assert!(!kills[0].falling);
    assert_eq!(game.scoreboard(1).rows[0].score, 100);
    assert_eq!(game.scoreboard(1).rows[enemy + 1].deaths, 1);
    let weapon = BotWeapon {
        weapon: 352,
        fire_interval_msec: 80,
        ballistics: crate::game::combat::Ballistics {
            damage: 33,
            range: 8192.0,
            near_range: 500.0,
            location_multipliers: None,
            falloff: None,
            pellets: 1,
            spread_degrees: 0.0,
        },
    };
    game.step(
        12100,
        (template(), 70.0, 100),
        weapon,
        &collision,
        |s, _, _, _| s,
    )
    .unwrap();
    assert_eq!(game.bots[enemy].health, 100);
    assert_eq!(
        game.bots[enemy]
            .tactical
            .snapshot(12100)
            .unwrap()
            .duration_msec,
        0
    );
    game.bots[enemy].tactical.apply(
        12200,
        crate::game::status::tactical::Hit {
            kind: crate::game::status::tactical::Kind::Gas,
            duration_msec: 1000,
        },
    );
    game.finish(12300);
    assert_eq!(
        game.bots[enemy]
            .tactical
            .snapshot(12300)
            .unwrap()
            .duration_msec,
        0
    );
}
#[test]
fn emp_excludes_allies_checks_hardwire_and_clears_the_archived_actor_on_death_and_finish() {
    let collision = world();
    let local = template();
    let mut game = TdmMatch::new(config(), local, 190.0, &collision).unwrap();
    for bot in &mut game.bots {
        bot.snapshot.origin = [2000.0, 0.0, 0.0];
    }
    let enemy = game
        .bots
        .iter()
        .position(|b| b.snapshot.team != local.team)
        .unwrap();
    let ally = game
        .bots
        .iter()
        .position(|b| b.snapshot.team == local.team)
        .unwrap();
    game.bots[enemy].snapshot.origin = [0.0; 3];
    game.bots[ally].snapshot.origin = [0.0; 3];
    let explosion = crate::game::weapons::grenade::Explosion {
        time: 2000,
        origin: [0.0, 0.0, 60.0],
        normal: [0.0, 0.0, 1.0],
        definition: crate::game::weapons::grenade::captured("emp_grenade_mp"),
    };
    let mut held = crate::game::weapons::held::tests::scavenger_player(90, 180);
    held.set_perks(crate::game::status::perks::Perks::selected([0, 60213, 0], &[60213]).unwrap());
    game.bots[enemy].weapon = Some(held.clone());
    assert_eq!(game.player_emp(2000, &explosion, local, &collision), 0);
    assert!(!game.bots[enemy].emp.active(2000));
    held.set_perks(Default::default());
    game.bots[enemy].weapon = Some(held);
    assert_eq!(game.player_emp(2000, &explosion, local, &collision), 1);
    assert!(game.bots[enemy].emp.active(2000));
    assert!(!game.bots[ally].emp.active(2000));
    game.bots[enemy].snapshot_time = 2000;
    assert!(
        game.replay_views(2000)
            .iter()
            .find(|v| v.client == game.bots[enemy].snapshot.client_index)
            .unwrap()
            .vitals
            .emp_jammed
    );
    let victim = game.bots[enemy].snapshot;
    game.credit(2100, victim, 0, 74, false, false);
    assert!(!game.bots[enemy].emp.active(2100));
    assert!(
        !game
            .replay_views(2100)
            .iter()
            .find(|v| v.client == victim.client_index)
            .unwrap()
            .vitals
            .emp_jammed
    );
    game.player_emp(2200, &explosion, local, &collision);
    game.finish(2300);
    assert!(!game.bots[enemy].emp.active(2300));
    assert_eq!(
        game.bots[enemy]
            .tactical
            .snapshot(2300)
            .unwrap()
            .duration_msec,
        0
    );
}
#[test]
fn grenade_blast_excludes_allies_and_suicide_never_awards_a_kill() {
    let collision = world();
    let mut game = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
    for bot in &mut game.bots {
        bot.snapshot.origin = [2000.0, 0.0, 0.0];
        bot.birth = 1000;
    }
    let enemy = game
        .bots
        .iter()
        .position(|b| b.snapshot.team != template().team)
        .unwrap();
    let ally = game
        .bots
        .iter()
        .position(|b| b.snapshot.team == template().team)
        .unwrap();
    game.bots[enemy].snapshot.origin = [64.0, 0.0, 0.0];
    game.bots[ally].snapshot.origin = [64.0, 0.0, 0.0];
    let explosion = crate::game::weapons::grenade::Explosion {
        time: 1000,
        origin: [64.0, 0.0, 40.0],
        normal: [0.0, 0.0, 1.0],
        definition: crate::game::weapons::grenade::captured("frag_grenade_mp"),
    };
    let (feedback, damage) = game.player_explosion(3000, &explosion, template(), &collision);
    assert_eq!(
        feedback,
        Some(codol_protocol::game::snapshot::hit_feedback::HitKind::Kill)
    );
    assert_eq!(game.bots[enemy].health, 0);
    assert_eq!(game.bots[ally].health, 100);
    assert!(damage >= 100);
    assert_eq!(game.rows[0].kills, 1);
    game.player_killed(
        3000,
        template(),
        PlayerHit {
            damage,
            attacker: 0,
            weapon: 95,
            headshot: false,
        },
    );
    assert_eq!(
        (game.rows[0].kills, game.rows[0].deaths, game.rows[0].score),
        (1, 1, 0)
    );
    assert_eq!(game.scoreboard(1).team_scores[2], 100);
    assert!(
        game.take_kills()
            .iter()
            .all(|k| k.weapon == 95 && !k.headshot && !k.melee)
    );
}
#[test]
fn native_room_teams_control_spawns_scoring_and_winner() {
    let collision = world();
    let mut config = config();
    config.teams = [1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2];
    config.score_limit = 100;
    let mut m = TdmMatch::new(config, template(), 200.0, &collision).unwrap();
    assert_eq!(m.player_spawn(&collision).unwrap()[0], 300.0);
    for bot in &m.bots {
        assert_eq!(
            bot.snapshot.team,
            m.teams()[usize::from(bot.snapshot.client_index)]
        );
        assert_eq!(
            bot.snapshot.origin[0],
            if bot.snapshot.team == 1 {
                300.0
            } else {
                -300.0
            }
        );
    }
    m.damage_bots(2100, 0, 352, &[(1, 100, false)], false);
    assert_eq!(m.scoreboard(42).team_scores, [0, 100, 0, 0]);
    assert_eq!(m.winning_team(), 1);
    assert_eq!(m.scoreboard(42).rows[1].deaths, 1);
}
#[test]
fn player_shots_follow_either_room_team_and_teammates_block_without_damage() {
    let collision = world();
    for player_team in [1, 2] {
        let mut config = config();
        config.teams = std::array::from_fn(|i| {
            if i % 2 == 0 {
                player_team
            } else {
                3 - player_team
            }
        });
        let mut local = template();
        local.team = player_team;
        let mut m = TdmMatch::new(config, local, 200.0, &collision).unwrap();
        for bot in &mut m.bots {
            bot.snapshot.origin = [500.0, 500.0, 0.0];
        }
        // Bot 2 is friendly and shields the enemy Bot 1 directly behind.
        m.bots[0].snapshot.origin = [200.0, 0.0, 0.0];
        m.bots[1].snapshot.origin = [100.0, 0.0, 0.0];
        let mut shot = crate::game::weapons::held::Shot {
            time: 2100,
            angles: [0; 3],
            weapon: 352,
            ballistics: game_combat::Ballistics {
                damage: 50,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
        };
        let blocked = m.player_fire(shot, [0.0, 0.0, 60.0], local, &collision, None);
        assert!(blocked.damage.is_empty());
        assert!(blocked.committed_feedback.is_none());
        assert!(m.bots.iter().all(|b| b.health == 100));
        assert_eq!(m.scoreboard(42).team_scores, [0; 4]);

        m.bots[1].snapshot.origin[1] = 100.0;
        shot.time = 1500;
        let protected = m.player_fire(shot, [0.0, 0.0, 60.0], local, &collision, None);
        assert!(!protected.damage.is_empty());
        assert!(protected.committed_feedback.is_none());
        assert_eq!(m.bots[0].health, 100);
        for time in [2200, 2300] {
            shot.time = time;
            let hit = m.player_fire(shot, [0.0, 0.0, 60.0], local, &collision, None);
            assert_eq!(hit.damage, [(1, 50, false)]);
            assert_eq!(
                hit.committed_feedback,
                Some(if time == 2200 {
                    codol_protocol::game::snapshot::hit_feedback::HitKind::Hit
                } else {
                    codol_protocol::game::snapshot::hit_feedback::HitKind::Kill
                })
            );
        }
        assert_eq!(m.bots[0].health, 0);
        assert_eq!(m.bots[1].health, 100);
        let board = m.scoreboard(42);
        assert_eq!(board.rows[0].kills, 1);
        assert_eq!(board.rows[1].deaths, 1);
        assert_eq!(board.rows[2].deaths, 0);
        assert_eq!(board.team_scores[usize::from(player_team)], 100);
        assert_eq!(board.team_scores[usize::from(3 - player_team)], 0);
        let kills = m.take_kills();
        assert_eq!(kills.len(), 1);
        assert_eq!(kills[0].victim.client_index, 1);
    }
}
#[test]
fn melee_respects_teams_range_lifetimes_and_records_one_knife_kill() {
    use codol_protocol::game::snapshot::hit_feedback::HitKind;
    let collision = world();
    for team in [1, 2] {
        let mut c = config();
        c.teams = std::array::from_fn(|i| if i % 2 == 0 { team } else { 3 - team });
        let local = template();
        let mut m = TdmMatch::new(c, local, 190.0, &collision).unwrap();
        for b in &mut m.bots {
            b.snapshot.origin = [500.0, 500.0, 0.0];
        }
        let mut strike = crate::game::weapons::held::MeleeStrike {
            time: 2100,
            angles: [0; 3],
            weapon: 352,
            damage: 135,
        };
        m.bots[0].snapshot.origin = [60.0, 0.0, 0.0];
        m.bots[1].snapshot.origin = [25.0, 0.0, 0.0];
        assert!(
            m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                .damage
                .is_empty()
        );
        m.bots[1].snapshot.origin[1] = 100.0;
        m.bots[0].snapshot.origin[0] = 80.0; // nearest face lies beyond64.
        assert!(
            m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                .damage
                .is_empty()
        );
        m.bots[0].snapshot.origin[0] = 60.0;
        strike.time = 900; // stale command from before this life.
        assert!(
            m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                .committed_feedback
                .is_none()
        );
        assert_eq!(m.bots[0].health, 100);
        strike.time = 1500;
        assert!(
            m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                .committed_feedback
                .is_none()
        );
        assert_eq!(m.bots[0].health, 100);
        strike.time = 2200;
        assert!(
            m.player_melee(
                strike,
                [0.0, 0.0, 60.0],
                EnemySnapshot {
                    alive: false,
                    ..local
                },
                &collision
            )
            .damage
            .is_empty()
        );
        let hit = m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision);
        assert_eq!(hit.committed_feedback, Some(HitKind::Kill));
        assert!(hit.impacts.is_empty());
        assert_eq!(m.bots[0].health, 0);
        assert!(
            m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                .committed_feedback
                .is_none()
        );
        let kills = m.take_kills();
        assert_eq!(kills.len(), 1);
        assert!(kills[0].melee && !kills[0].headshot);
        assert_eq!(kills[0].weapon, 352);
        let board = m.scoreboard(0);
        assert_eq!(board.rows[0].kills, 1);
        assert_eq!(board.rows[0].headshots, 0);
        assert_eq!(board.team_scores[usize::from(team)], 100);
    }
}
#[test]
fn melee_approach_accepts_native_side_hints_and_rejects_range_or_visibility_forgery() {
    let collision = world();
    let local = template();
    let mut game = TdmMatch::new(config(), local, 190.0, &collision).unwrap();
    for bot in &mut game.bots {
        bot.snapshot.origin = [500.0, 500.0, 0.0];
    }
    game.bots[5].snapshot.origin = [75.0, -24.0, 0.03125];
    let mut motion = crate::game::player::movement::Motion::new([0.0, 0.0, 0.03125]);
    motion.apply(
        &codol_protocol::game::channel::usercmd::UserCommand {
            time: 1000,
            ..Default::default()
        },
        1.0,
        false,
        &collision,
    );
    let input = codol_protocol::game::channel::usercmd::UserCommand {
        time: 2100,
        buttons: 4,
        melee_yaw: 62306,
        melee_distance: 78,
        melee_entity: 2047,
        ..Default::default()
    };
    assert!(
        game.player_melee_approach(&input, &motion, local, &collision)
            .is_some()
    );
    for bad in [
        codol_protocol::game::channel::usercmd::UserCommand {
            melee_yaw: 0,
            ..input.clone()
        },
        codol_protocol::game::channel::usercmd::UserCommand {
            melee_distance: 110,
            ..input.clone()
        },
        codol_protocol::game::channel::usercmd::UserCommand {
            melee_entity: 7,
            ..input.clone()
        },
        codol_protocol::game::channel::usercmd::UserCommand {
            angles: [0, 32768, 0],
            ..input.clone()
        },
    ] {
        assert!(
            game.player_melee_approach(&bad, &motion, local, &collision)
                .is_none()
        );
    }
    game.bots[0].snapshot.origin = [35.0, -11.2, 0.03125];
    assert!(
        game.player_melee_approach(&input, &motion, local, &collision)
            .is_none()
    );
    game.bots[0].snapshot.origin = [500.0, 500.0, 0.0];
    game.bots[5].snapshot.origin = [130.0, 0.0, 0.03125];
    let far = codol_protocol::game::channel::usercmd::UserCommand {
        melee_yaw: 0,
        melee_distance: 130,
        ..input
    };
    assert!(
        game.player_melee_approach(&far, &motion, local, &collision)
            .is_none()
    );
}
#[test]
fn melee_approach_validates_the_hint_against_current_enemy_and_view() {
    let collision = world();
    let local = template();
    let mut game = TdmMatch::new(config(), local, 190.0, &collision).unwrap();
    for bot in &mut game.bots {
        bot.snapshot.origin = [500.0, 500.0, 0.0];
    }
    game.bots[5].snapshot.origin = [75.0, 0.0, 0.03125];
    let mut motion = crate::game::player::movement::Motion::new([0.0, 0.0, 0.03125]);
    motion.apply(
        &codol_protocol::game::channel::usercmd::UserCommand {
            time: 1000,
            ..Default::default()
        },
        1.0,
        false,
        &collision,
    );
    let input = codol_protocol::game::channel::usercmd::UserCommand {
        time: 1100,
        buttons: 4,
        melee_yaw: 0,
        melee_distance: 75,
        melee_entity: 2047,
        ..Default::default()
    };
    assert!(
        game.player_melee_approach(&input, &motion, local, &collision)
            .is_some()
    );
    for bad in [
        codol_protocol::game::channel::usercmd::UserCommand {
            melee_yaw: 32768,
            ..input.clone()
        },
        codol_protocol::game::channel::usercmd::UserCommand {
            melee_distance: 200,
            ..input.clone()
        },
        codol_protocol::game::channel::usercmd::UserCommand {
            melee_entity: 1,
            ..input.clone()
        },
        codol_protocol::game::channel::usercmd::UserCommand {
            angles: [0, 32768, 0],
            ..input.clone()
        },
    ] {
        assert!(
            game.player_melee_approach(&bad, &motion, local, &collision)
                .is_none()
        );
    }
    game.bots[0].snapshot.origin = [40.0, 0.0, 0.03125];
    assert!(
        game.player_melee_approach(&input, &motion, local, &collision)
            .is_none()
    );
    game.bots[0].snapshot.origin = [500.0, 500.0, 0.0];
    game.bots[5].snapshot.alive = false;
    assert!(
        game.player_melee_approach(&input, &motion, local, &collision)
            .is_none()
    );
}
#[test]
fn committed_feedback_ignores_protection_and_keeps_a_multi_target_kill() {
    use codol_protocol::game::snapshot::hit_feedback::HitKind;
    let collision = world();
    let mut m = TdmMatch::new(config(), template(), 200.0, &collision).unwrap();
    assert_eq!(m.damage_bots(1500, 0, 352, &[(6, 100, false)], false), None);
    assert_eq!(m.bots[5].health, 100);
    assert_eq!(m.damage_bots(2100, 0, 352, &[(6, 0, false)], false), None);
    assert_eq!(
        m.damage_bots(2200, 0, 352, &[(6, 100, false), (7, 30, false)], false),
        Some(HitKind::Kill)
    );
    assert_eq!(m.bots[5].health, 0);
    assert_eq!(m.bots[6].health, 70);
    assert_eq!(m.damage_bots(2300, 0, 352, &[(6, 100, false)], false), None);
    assert_eq!(
        m.damage_bots(2400, 0, 352, &[(7, 30, false)], false),
        Some(HitKind::Hit)
    );
    m.rules.finished = true;
    assert_eq!(m.damage_bots(2500, 0, 352, &[(7, 100, false)], false), None);
}
#[test]
fn six_per_team_have_unique_clients_lives_scores_and_respawns() {
    let collision = world();
    let mut m = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
    assert_eq!(m.bots.len(), 11);
    let snapshots = m.snapshots();
    let death_waypoints = [codol_protocol::game::snapshot::hit_feedback::DeathWaypointSnapshot {
        material: 169,
        origin: [1234.0, -4321.0, 154.0],
    }; 5];
    let baseline = codol_protocol::game::snapshot::player_frame_with_world(
        1000,
        1000,
        Some([0.0; 3]),
        true,
        None,
        [0; 3],
        codol_protocol::game::snapshot::WorldSnapshot {
            local_player: Some(template()),
            players: &snapshots,
            held_weapon_tag: Some(1),
            death_waypoints: Some(&death_waypoints),
            hit_feedback: Some(
                codol_protocol::game::snapshot::hit_feedback::HitFeedbackSnapshot {
                    material: 144,
                    event_time: 1000,
                    visible: true,
                    kind: codol_protocol::game::snapshot::hit_feedback::HitKind::Kill,
                },
            ),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        baseline.len() + 5 < codol_protocol::game::channel::wire::FRAGMENT_SIZE,
        "twelve-player baseline {} bytes",
        baseline.len()
    );
    assert_eq!(m.bots.iter().filter(|b| b.snapshot.team == 2).count(), 5);
    assert_eq!(m.bots.iter().filter(|b| b.snapshot.team == 1).count(), 6);
    m.damage_bots(2100, 0, 352, &[(6, 100, true)], false);
    assert!(!m.snapshots()[5].alive);
    let board = m.scoreboard(123);
    assert_eq!(board.rows[0].user_id, 123);
    assert_eq!(
        (
            board.rows[0].kills,
            board.rows[0].headshots,
            board.rows[6].deaths
        ),
        (1, 1, 1)
    );
    assert_eq!(board.team_scores, [0, 0, 100, 0]);
    let kills = m.take_kills();
    assert_eq!(kills.len(), 1);
    m.damage_bots(2200, 0, 352, &[(6, 100, true)], false);
    assert!(m.take_kills().is_empty());
    let weapon = BotWeapon {
        weapon: 352,
        fire_interval_msec: 79,
        ballistics: game_combat::Ballistics {
            damage: 33,
            range: 8192.0,
            near_range: 500.0,
            location_multipliers: None,
            falloff: None,
            pellets: 1,
            spread_degrees: 0.0,
        },
    };
    let mut local = template();
    local.alive = false;
    m.step(
        5100,
        (local, 70.0, 0),
        weapon,
        &collision,
        |mut snapshot, frame, _, _| {
            snapshot.origin = frame.motion.origin;
            snapshot.angles = frame.angles;
            snapshot
        },
    )
    .unwrap();
    assert!(m.snapshots()[5].alive);
    assert_eq!(m.bots[5].health, 100);
    assert_eq!(m.scoreboard(123).rows[6].deaths, 1);
}
#[test]
fn score_and_time_limits_stop_combat_without_additional_score() {
    let collision = world();
    let mut c = config();
    c.score_limit = 100;
    let mut m = TdmMatch::new(c, template(), 190.0, &collision).unwrap();
    m.damage_bots(2100, 0, 352, &[(6, 100, false)], false);
    assert!(m.rules.finished);
    assert_eq!(m.result(), "Allies win");
    assert_eq!(m.scoreboard(1).score_limit, 100);
    let final_board = m.scoreboard(1);
    m.damage_bots(2200, 0, 352, &[(7, 100, false)], false);
    m.player_killed(
        2300,
        template(),
        PlayerHit {
            damage: 100,
            attacker: 6,
            weapon: 352,
            headshot: false,
        },
    );
    m.advance_clock(700000);
    assert_eq!(m.scoreboard(1), final_board);
    assert_eq!(m.rules.finished_at, Some(2100));
    assert_eq!(m.winning_team(), 2);
}
#[test]
fn deadline_freezes_a_loss_and_a_draw_without_respawning_or_rescoring() {
    let collision = world();
    for (attacker, winner) in [(Some(6), 1), (None, 0)] {
        let mut c = config();
        c.time_limit_msec = 60_000;
        let mut m = TdmMatch::new(c, template(), 190.0, &collision).unwrap();
        m.advance_clock(1000);
        if let Some(attacker) = attacker {
            m.damage_bots(2100, attacker, 352, &[(1, 100, false)], false);
        }
        m.advance_clock(60999);
        assert!(!m.rules.finished);
        m.advance_clock(61000);
        assert!(m.rules.finished);
        assert_eq!(m.winning_team(), winner);
        assert_eq!(m.duration_seconds(), 60);
        let final_board = m.scoreboard(1);
        let snapshots = m.snapshots();
        m.damage_bots(62000, 0, 352, &[(6, 100, false)], false);
        m.advance_clock(120000);
        assert_eq!(m.scoreboard(1), final_board);
        assert_eq!(m.rules.finished_at, Some(61000));
        assert_eq!(m.snapshots(), snapshots);
    }
}
#[test]
#[ignore = "requires local captured Afghan preparation"]
fn captured_map_starts_build_a_real_twelve_player_match() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../research-output/tdm-state.json");
    let state = crate::game::session::prepared::PreparedGameState::load(&path).unwrap();
    assert!(state.is_match());
}
#[test]
#[ignore = "requires local three-map faction outfits and CODOL_APPEARANCE_PROFILE"]
fn captured_room_factions_keep_outfits_and_resource_packets_consistent() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let profile = std::path::PathBuf::from(std::env::var_os("CODOL_APPEARANCE_PROFILE").unwrap());
    let public = std::fs::read(profile.join("public.bin")).unwrap();
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join("inventory.json")).unwrap()).unwrap();
    let owned: Vec<u32> = serde_json::from_value(inventory["items"].clone()).unwrap();
    let suffix =
        std::env::var("CODOL_BOT_OUTFIT_SUFFIX").unwrap_or_else(|_| "faction-outfits".into());
    for name in [
        "tdm-afghan-hud-state",
        "tdm-killhouse-hud-state",
        "tdm-crash-tropical-state",
    ] {
        let state = crate::game::session::prepared::PreparedGameState::load(
            &root.join(format!("{name}-{suffix}.json")),
        )
        .unwrap();
        for teams in [default_teams(), default_teams().map(|t| 3 - t)] {
            let resolved = state
                .clone()
                .with_room_teams(teams)
                .unwrap()
                .for_profile(&public, &owned)
                .unwrap();
            let game = resolved.tdm_match().unwrap().unwrap();
            let actors = game.snapshots();
            for (i, actor) in actors.iter().enumerate() {
                let outfit = game.config.roster[i].outfits.unwrap()[usize::from(teams[i + 1] - 1)];
                assert_eq!(actor.model, outfit.model);
                assert_eq!(actor.attachment_models, outfit.attachment_models);
                assert_eq!(game.baselines()[i].model, actor.model);
            }
            let gamestate = resolved.body(1).unwrap();
            let wire = codol_protocol::game::channel::wire::compact_server_packet(
                3, 0, 0, b"", &gamestate,
            )
            .unwrap();
            let snapshot = resolved
                .snapshot(
                    1000,
                    1000,
                    true,
                    [0; 3],
                    resolved.player().map(|p| p.equipment()),
                    crate::game::session::prepared::SnapshotWorld {
                        motion: resolved.motion().as_ref(),
                        local_player: resolved.local_player_snapshot(true),
                        players: &actors,
                        vitals: Some(Default::default()),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(snapshot.len() < codol_protocol::game::channel::wire::FRAGMENT_SIZE - 4);
            eprintln!(
                "{name} local_team={} models={} wire={} snapshot={}",
                teams[0],
                game.config.models.len(),
                wire.len(),
                snapshot.len()
            );
        }
    }
}

#[test]
#[ignore = "requires local three-map captures and CODOL_GROUND_PROFILE directory"]
fn captured_bot_ground_contacts() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let profile = std::path::PathBuf::from(
        std::env::var_os("CODOL_GROUND_PROFILE").expect("set local profile directory"),
    );
    let public = std::fs::read(profile.join("public.bin")).unwrap();
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join("inventory.json")).unwrap()).unwrap();
    let owned: Vec<u32> = serde_json::from_value(inventory["items"].clone()).unwrap();
    let suffix = std::env::var("CODOL_GROUND_CAPTURE").unwrap_or_else(|_| "current".into());
    assert!(
        suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    );
    for name in [
        "tdm-afghan-hud-state",
        "tdm-killhouse-hud-state",
        "tdm-crash-tropical-state",
    ] {
        let state = crate::game::session::prepared::PreparedGameState::load(
            &root.join(format!("{name}.json")),
        )
        .unwrap()
        .for_profile(&public, &owned)
        .unwrap();
        let collision = state.collision.as_ref().unwrap();
        if let Some(path) = std::env::var_os("CODOL_GROUND_POSES") {
            let capture: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(capture["client_sha256"], codol_protocol::CLIENT_SHA256);
            assert_eq!(capture["read_only"], true);
            if capture["ncs_crc"] != state.ncs_crc() {
                continue;
            }
            let mut contacts = Vec::new();
            let clients: Vec<_> = if let Some(frames) = capture["frames"].as_array() {
                frames
                    .iter()
                    .flat_map(|f| f["clients"].as_array().unwrap())
                    .collect()
            } else {
                capture["clients"].as_array().unwrap().iter().collect()
            };
            for client in clients.into_iter().filter(|c| c["client"] != 0) {
                let pose = &client["ground_pose"];
                if pose["stable_read"] != true || pose["plausible"] != true {
                    continue;
                }
                let root_origin: [f32; 3] =
                    serde_json::from_value(pose["root_origin"].clone()).unwrap();
                // Include a small upward margin for integer network Z and
                // interpolation, without moving the authoritative actor.
                let support = collision.trace(
                    [root_origin[0], root_origin[1], root_origin[2] + 2.0],
                    [root_origin[0], root_origin[1], root_origin[2] - 32.0],
                    70.0,
                );
                for foot in pose["feet"].as_array().unwrap() {
                    let p: [f32; 3] = serde_json::from_value(foot["origin"].clone()).unwrap();
                    let floor =
                        collision.raycast([p[0], p[1], p[2] + 16.0], [p[0], p[1], p[2] - 64.0]);
                    let movement_floor = collision
                        .movement_surface([p[0], p[1], p[2] + 16.0], [p[0], p[1], p[2] - 64.0]);
                    contacts.push(serde_json::json!({"client":client["client"],"legs":client["legs_animation"],
                        "time":pose["time"],"bone":foot["name"],"origin":p,
                        "root_origin":root_origin,"root_floor_gap":(support.fraction<1.0 && !support.start_solid).then_some(support.fraction*34.0-2.0),
                        "root_normal":support.normal,
                        "movement_floor_gap":(movement_floor.fraction<1.0 && !movement_floor.start_solid).then_some(movement_floor.fraction*80.0-16.0),
                        "floor_gap":(floor.fraction<1.0 && !floor.start_solid).then_some(floor.fraction*80.0-16.0),
                        "floor_normal":floor.normal}));
                }
            }
            assert!(!contacts.is_empty(), "no current rendered feet in capture");
            let output = root.join(format!("bot-feet-{name}-{suffix}.json"));
            std::fs::write(&output, serde_json::to_vec(&contacts).unwrap()).unwrap();
            eprintln!(
                "rendered feet diagnostic: {} -> {}",
                contacts.len(),
                output.display()
            );
            continue;
        }
        let mut game = state.tdm_match().unwrap().unwrap();
        for bot in &mut game.bots {
            bot.random = u64::from(bot.snapshot.client_index) * 0x9e3779b9;
        }
        let mut local = game.bots[0].snapshot;
        local.entity_number = 0;
        local.client_index = 0;
        local.alive = false;
        let mut frames = Vec::new();
        for now in (1000..=61_000).step_by(50) {
            game.step(
                now,
                (local, 70.0, 0),
                state.match_bot_weapon().unwrap(),
                collision,
                |enemy, frame, firing, toggle| state.animate_enemy(enemy, frame, firing, toggle),
            )
            .unwrap();
            for bot in game.bots.iter().filter(|b| b.health > 0) {
                let motion = &bot.view_motion;
                let origin = motion.origin;
                let down = collision.trace(origin, [origin[0], origin[1], origin[2] - 16.0], 70.0);
                let point = collision.raycast(origin, [origin[0], origin[1], origin[2] - 32.0]);
                frames.push(
                    serde_json::json!({"time":now,"client":bot.snapshot.client_index,
                    "origin":origin,"velocity":motion.velocity,"grounded":motion.grounded(),
                    "drop":down.fraction * 16.0,"normal":down.normal,"solid":down.start_solid,
                    "point_drop":point.fraction * 32.0,"legs":bot.snapshot.legs_animation}),
                );
            }
        }
        let output = root.join(format!("bot-ground-{name}-{suffix}.json"));
        std::fs::write(&output, serde_json::to_vec(&frames).unwrap()).unwrap();
        eprintln!(
            "ground contact diagnostic: {} samples -> {}",
            frames.len(),
            output.display()
        );
    }
}
