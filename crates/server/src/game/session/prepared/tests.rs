#[test]
#[ignore = "requires captured local preparations via COOL_VOICE_MAPS"]
fn captured_voice_map_preparations_load_with_matching_banks() {
    let path =
        std::path::PathBuf::from(std::env::var_os("COOL_VOICE_MAPS").expect("set COOL_VOICE_MAPS"));
    let path = if path.is_absolute() {
        path
    } else {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    };
    let catalog: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let entries = catalog["entries"].as_array().unwrap();
    assert!(!entries.is_empty());
    for entry in entries {
        let file = path.parent().unwrap().join(entry["file"].as_str().unwrap());
        let state = super::PreparedGameState::load(&file).unwrap();
        assert_eq!(state.map(), entry["map"].as_str().unwrap());
        let resources = state.voice_resources().expect("voice-enabled preparation");
        for faction in 0..=1 {
            for gender in 0..=1 {
                let character = codol_protocol::game::snapshot::character::Character {
                    groups: [0; 11],
                    bits: [0; 11],
                    skin: 0,
                    faction,
                    gender,
                };
                assert!(
                    resources.prefix(character).is_some(),
                    "{} missing faction{faction}/gender{gender}",
                    state.map()
                );
            }
        }
    }
    println!("loaded {} voice-enabled map preparations", entries.len());
}
#[test]
fn compass_candidate_remains_metadata_only_and_checks_the_initialized_crc() {
    let probe = crate::content::maps::minimap::CompassProbe {
        schema_version: 1,
        client_sha256: codol_protocol::CLIENT_SHA256.into(),
        map: "mp_highrise".into(),
        ncs_crc: 0x12345678,
        entities_sha256: "a".repeat(64),
        minimap: crate::content::maps::minimap::Minimap {
            material: "compass_map_mp_highrise".into(),
            north_yaw: 0.0,
            upper_left: [100.0, 100.0],
            lower_right: [-100.0, -100.0],
            material_proof: None,
        },
    };
    let mut state = super::synthetic();
    state.map = probe.map.clone();
    let prepared = state.clone().with_resource_compass_probe(&probe).unwrap();
    assert!(!prepared.snapshot_probe());
    assert_eq!(prepared.snapshot_limit(), 0);
    assert!(prepared.minimap.is_none());
    assert!(prepared.resource_compass_probe.is_some());
    assert!(prepared.body(1).is_ok());
    state.ncs_crc = 123;
    assert!(state.clone().with_resource_compass_probe(&probe).is_err());
    state.ncs_crc = probe.ncs_crc;
    state.spawn_origin = Some([0.0, 0.0, 0.0]);
    assert!(state.with_resource_compass_probe(&probe).is_err());
}
use super::*;
#[test]
fn identical_guns_do_not_hide_changed_passive_or_offhand_spawn_equipment() {
    let current = synthetic();
    let mut selected = current.clone();
    selected.player_perks =
        crate::game::status::perks::Perks::selected([60041, 0, 0], &[60041]).unwrap();
    assert_eq!(
        current.selected_weapon_names(),
        selected.selected_weapon_names()
    );
    assert!(!current.same_spawn_equipment(&selected));
    selected.player_perks = Default::default();
    selected.player_grenades[0] = Some(crate::game::weapons::grenade::Equipped {
        definition: crate::game::weapons::grenade::captured("frag_grenade_mp"),
        count: 1,
    });
    assert!(!current.same_spawn_equipment(&selected));
    selected.player_grenades = [None, None];
    assert!(current.same_spawn_equipment(&selected));
}
fn write_captured_sniper_contact(root: &Path, output: &Path, raw: &serde_json::Value) {
    let staged = output.with_extension("animation-input.json");
    std::fs::write(&staged, serde_json::to_vec_pretty(raw).unwrap()).unwrap();
    let rebound = output.with_extension("animation-output.json");
    if rebound.exists() {
        std::fs::remove_file(&rebound).unwrap();
    }
    let status = std::process::Command::new("python3")
        .arg(root.join("../tools/prepare_bot_roster.py"))
        .arg("--rebind-animations")
        .arg("--game-state")
        .arg(&staged)
        .arg("--characters")
        .arg(root.join("bot-animation-capture-killhouse.json"))
        .arg("--output")
        .arg(&rebound)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "captured sniper animation rebinding failed"
    );
    PreparedGameState::load(&rebound).unwrap();
    std::fs::rename(&rebound, output).unwrap();
}
#[test]
#[ignore = "prepares a side-offset native melee scene from captured Killhouse"]
fn captured_wide_melee_scene_has_a_visible_target_outside_the_center_ray() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let mut raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("tdm-killhouse-hud-state.json")).unwrap())
            .unwrap();
    let collision = crate::game::world::collision::Collision::load(
        &root.join(raw["collision_file"].as_str().unwrap()),
    )
    .unwrap();
    let starts: [Vec<[f32; 3]>; 2] =
        serde_json::from_value(raw["match_config"]["spawns"].clone()).unwrap();
    let mut contact = None;
    'search: for seed in &starts[1] {
        for lateral in [-24.0, 24.0, -32.0, 32.0] {
            let ground = |dx, dy| {
                crate::game::ai::controller::grounded_spawn(
                    [seed[0] + dx, seed[1] + dy, seed[2]],
                    &collision,
                )
                .ok()
            };
            let (Some(player), Some(target), Some(bait)) = (
                ground(0.0, 0.0),
                ground(75.0, lateral),
                ground(300.0, lateral),
            ) else {
                continue;
            };
            if (target[2] - player[2]).abs() > 1.0 {
                continue;
            }
            let eye = [player[0], player[1], player[2] + 60.0];
            let target_eye = [target[0], target[1], target[2] + 60.0];
            let sight = collision.raycast(eye, target_eye);
            if sight.start_solid || sight.fraction < 1.0 {
                continue;
            }
            if collision.trace(player, target, 70.0).fraction < 1.0 {
                continue;
            }
            let actors = [crate::game::combat::Actor {
                client: 1,
                team: 1,
                origin: target,
                height: 70.0,
                yaw: 0.0,
            }];
            let straight = crate::game::combat::fire_at_actors(
                eye,
                [0; 3],
                1000,
                crate::game::combat::Ballistics {
                    damage: 1,
                    range: 96.0,
                    near_range: 96.0,
                    location_multipliers: None,
                    falloff: None,
                    pellets: 1,
                    spread_degrees: 0.0,
                },
                (0, 2),
                crate::game::combat::ActorScene {
                    actors: &actors,
                    regions: None,
                    collision: &collision,
                },
            );
            assert!(straight.damage.is_empty());
            contact = Some((player, target, bait));
            break 'search;
        }
    }
    let (player, target, bait) = contact.expect("no clear side-offset native contact");
    let mut friends = vec![player, bait];
    friends.extend_from_slice(&starts[0][..4]);
    let mut enemies = vec![target];
    enemies.extend_from_slice(&starts[0][5..10]);
    raw["spawn_origin"] = serde_json::json!(player);
    raw["match_config"]["spawns"] = serde_json::json!([friends, enemies]);
    raw["match_config"]["time_limit_msec"] = 60000.into();
    for bot in raw["match_config"]["roster"].as_array_mut().unwrap() {
        bot["weapon_name"] = "cheytac_mp".into();
    }
    let output = root.join("tdm-killhouse-wide-melee-test.json");
    write_captured_sniper_contact(&root, &output, &raw);
    eprintln!("wide melee player={player:?} target={target:?} bait={bait:?}");
}
#[test]
#[ignore = "prepares a local captured-map charge diagnostic"]
fn captured_charge_contact_scene_has_an_unobstructed_approach() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let mut raw: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("tdm-killhouse-hud-state-charge.json")).unwrap(),
    )
    .unwrap();
    let collision = crate::game::world::collision::Collision::load(
        &root.join(raw["collision_file"].as_str().unwrap()),
    )
    .unwrap();
    let starts: [Vec<[f32; 3]>; 2] =
        serde_json::from_value(raw["match_config"]["spawns"].clone()).unwrap();
    let (player, target, bait) = starts[1]
        .iter()
        .find_map(|&seed| {
            let ground = |dx| {
                crate::game::ai::controller::grounded_spawn(
                    [seed[0] + dx, seed[1], seed[2]],
                    &collision,
                )
                .ok()
            };
            let a = ground(0.0)?;
            let b = ground(75.0)?;
            let c = ground(300.0)?;
            if (b[2] - a[2]).abs() > 18.0 {
                return None;
            }
            let sight = collision.raycast([b[0], b[1], b[2] + 60.0], [c[0], c[1], c[2] + 60.0]);
            if sight.start_solid || sight.fraction < 1.0 {
                return None;
            }
            let mut motion = crate::game::player::movement::Motion::new(a);
            motion.apply(
                &codol_protocol::game::channel::usercmd::UserCommand {
                    time: 1000,
                    ..Default::default()
                },
                1.0,
                false,
                &collision,
            );
            motion.start_melee_charge(
                crate::game::weapons::held::MeleeApproach {
                    yaw: 0,
                    distance: 75,
                },
                100,
            );
            for time in (1010..=1100).step_by(10) {
                motion.apply(
                    &codol_protocol::game::channel::usercmd::UserCommand {
                        time,
                        ..Default::default()
                    },
                    1.0,
                    false,
                    &collision,
                );
            }
            let actors = [crate::game::combat::Actor {
                client: 6,
                team: 1,
                origin: b,
                height: 80.0,
                yaw: 0.0,
            }];
            let hit = crate::game::combat::melee_at_actors(
                [motion.origin[0], motion.origin[1], motion.origin[2] + 60.0],
                [0; 3],
                135,
                (0, 2),
                crate::game::combat::ActorScene {
                    actors: &actors,
                    regions: None,
                    collision: &collision,
                },
            );
            (!hit.damage.is_empty()).then_some((a, b, c))
        })
        .expect("no clear captured charge contact");
    let mut friends = vec![player, bait];
    friends.extend_from_slice(&starts[0][..4]);
    let mut enemies = vec![target];
    enemies.extend_from_slice(&starts[0][5..10]);
    raw["spawn_origin"] = serde_json::json!(player);
    raw["match_config"]["spawns"] = serde_json::json!([enemies, friends]);
    raw["match_config"]["time_limit_msec"] = 60000.into();
    for bot in raw["match_config"]["roster"].as_array_mut().unwrap() {
        bot["weapon_name"] = "cheytac_mp".into();
    }
    let output = root.join("tdm-killhouse-charge-contact-test.json");
    write_captured_sniper_contact(&root, &output, &raw);
    eprintln!("charge contact player={player:?} target={target:?} bait={bait:?}");
}
#[test]
#[ignore = "requires captured three-map charge preparations and CODOL_APPEARANCE_PROFILE"]
fn captured_three_maps_encode_saved_player_charge_and_roster_without_collisions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let profile = std::path::PathBuf::from(std::env::var_os("CODOL_APPEARANCE_PROFILE").unwrap());
    let public = std::fs::read(profile.join("public.bin")).unwrap();
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile.join("inventory.json")).unwrap()).unwrap();
    let owned: Vec<u32> = serde_json::from_value(inventory["items"].clone()).unwrap();
    for name in [
        "tdm-afghan-hud-state",
        "tdm-killhouse-hud-state",
        "tdm-crash-tropical-state",
    ] {
        let s = PreparedGameState::load(&root.join(format!("{name}-charge.json")))
            .unwrap()
            .for_profile(&public, &owned)
            .unwrap();
        let mut player = s.player().unwrap();
        let mut motion = s.motion().unwrap();
        let input = codol_protocol::game::channel::usercmd::UserCommand {
            time: 1100,
            buttons: 4,
            weapons: player.equipment().input_weapons(),
            ..Default::default()
        };
        motion.apply(&input, 1.0, false, s.collision.as_ref().unwrap());
        player.apply_with_melee_approach(
            &input,
            Some(crate::game::weapons::held::MeleeApproach {
                yaw: 0,
                distance: 100,
            }),
        );
        let (approach, duration) = player
            .take_melee_approach()
            .expect("saved gun needs a captured native charge animation");
        assert_eq!(player.equipment().firing.unwrap().animation & 0x3ff, 9);
        motion.start_melee_charge(approach, duration);
        let actors = s.tdm_match().unwrap().unwrap().snapshots();
        let snapshot = s
            .snapshot(
                1100,
                1100,
                true,
                [0; 3],
                Some(player.equipment()),
                SnapshotWorld {
                    motion: Some(&motion),
                    local_player: s.local_player_snapshot(true),
                    players: &actors,
                    ..Default::default()
                },
            )
            .unwrap();
        let wire =
            codol_protocol::game::channel::wire::compact_server_packet(4, 0, 0, b"", &snapshot)
                .unwrap();
        let initial = codol_protocol::game::channel::wire::compact_server_packet(
            3,
            0,
            0,
            b"",
            &s.body(1).unwrap(),
        )
        .unwrap();
        eprintln!(
            "{name} charge_delay={duration} snapshot={} wire={} gamestate={}",
            snapshot.len(),
            wire.len(),
            initial.len()
        );
    }
}
#[test]
#[ignore = "prepares a local captured-map melee contact diagnostic"]
fn captured_melee_contact_scene_has_a_clear_close_target() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let mut raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("tdm-afghan-hud-state.json")).unwrap())
            .unwrap();
    let collision = crate::game::world::collision::Collision::load(
        &root.join(raw["collision_file"].as_str().unwrap()),
    )
    .unwrap();
    let starts: [Vec<[f32; 3]>; 2] =
        serde_json::from_value(raw["match_config"]["spawns"].clone()).unwrap();
    let pair = starts[1]
        .iter()
        .find_map(|&seed| {
            let player = crate::game::ai::controller::grounded_spawn(seed, &collision).ok()?;
            let target = crate::game::ai::controller::grounded_spawn(
                [seed[0] + 50.0, seed[1], seed[2]],
                &collision,
            )
            .ok()?;
            if (target[2] - player[2]).abs() > 8.0 {
                return None;
            }
            let eye = [player[0], player[1], player[2] + 60.0];
            let actors = [crate::game::combat::Actor {
                client: 6,
                team: 1,
                origin: target,
                height: 70.0,
                yaw: 180.0,
            }];
            let hit = crate::game::combat::melee_at_actors(
                eye,
                [0; 3],
                135,
                (0, 2),
                crate::game::combat::ActorScene {
                    actors: &actors,
                    regions: None,
                    collision: &collision,
                },
            );
            (!hit.damage.is_empty()).then_some((player, target))
        })
        .expect("no verified nearby contact placement");
    let mut friend_starts = vec![pair.0];
    friend_starts.extend_from_slice(&starts[0][..5]);
    let mut enemy_starts = vec![pair.1];
    enemy_starts.extend_from_slice(&starts[0][5..10]);
    raw["spawn_origin"] = serde_json::json!(pair.0);
    raw["match_config"]["spawns"] = serde_json::json!([enemy_starts, friend_starts]);
    raw["match_config"]["time_limit_msec"] = 60000.into();
    // A captured 98-damage bolt-action target provides a controllable
    // contact window. No altered weapon definitions or account edits.
    raw["match_config"]["roster"][5]["weapon_name"] = "cheytac_mp".into();
    let output = root.join("tdm-afghan-melee-contact-test.json");
    std::fs::write(&output, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
    PreparedGameState::load(&output).unwrap();
    eprintln!(
        "melee contact: player={:?} target={:?} yaw=0; output={}",
        pair.0,
        pair.1,
        output.display()
    );
}
#[test]
#[ignore = "audits the locally captured weapon catalogue"]
fn captured_bullet_catalogue_validates_owned_backpacks() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let mut state = PreparedGameState::load(&root.join("tdm-afghan-hud-state.json")).unwrap();
    let source = state.arsenal_file.as_ref().unwrap();
    let captured: serde_json::Value =
        serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
    let arsenal = state.arsenal.clone().unwrap();
    let mut rows = Vec::new();
    let mut failed = Vec::new();
    for w in captured["weapons"].as_array().unwrap() {
        let Some(items) = w["item_ids"].as_array() else {
            continue;
        };
        let definition = STANDARD
            .decode(w["definition_base64"].as_str().unwrap())
            .unwrap();
        let word =
            |offset: usize| u32::from_le_bytes(definition[offset..offset + 4].try_into().unwrap());
        let kind = word(0x47c);
        if kind != 0 {
            continue;
        }
        let item = items[0].as_u64().unwrap() as u32;
        let result = arsenal
            .resolve(item, &[], &[item], 352)
            .and_then(|r| PreparedGameState::resolved_with_alternate(&arsenal, r, Some(54)))
            .and_then(|w| {
                state.bootstrap_weapon = Some(w);
                state.validate()
            });
        let error = result.err().map(|e| e.to_string());
        if error.is_some() && word(0x48c) <= 5 {
            failed.push((w["name"].clone(), error.clone()));
        }
        rows.push(serde_json::json!({"name":w["name"],"item":item,"fire_type":word(0x48c),"interval_msec":word(0xd10),"equipment_valid":error.is_none(),"error":error}));
    }
    let output = root.join("eel-lava-native/gun-audit.json");
    std::fs::write(&output,serde_json::to_vec_pretty(&serde_json::json!({"schema_version":1,"native_client_used":false,"weapons":rows,"failures":failed})).unwrap()).unwrap();
    assert!(
        failed.is_empty(),
        "unavailable regular bullet guns: {failed:?}; see {}",
        output.display()
    );
}

#[test]
#[ignore = "requires local captured Afghan arsenal"]
fn captured_eel_backpack_keeps_its_native_sub_tick_fire_interval() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let mut state = PreparedGameState::load(&root.join("tdm-afghan-hud-state.json")).unwrap();
    let arsenal = state.arsenal.as_ref().unwrap();
    let parts = [11751, 11758, 11762];
    let owned = [11857, 11751, 11758, 11762];
    let weapon = PreparedGameState::resolved_with_alternate(
        arsenal,
        arsenal.resolve(11857, &parts, &owned, 352).unwrap(),
        Some(54),
    )
    .unwrap();
    assert_eq!(weapon.name, "famasss_mp");
    assert_eq!(weapon.ammo.as_ref().unwrap().fire_interval_msec, Some(35));
    state.bootstrap_weapon = Some(weapon);
    state.validate().unwrap();
    let mut p = state.primary_player().unwrap();
    let command = |time| codol_protocol::game::channel::usercmd::UserCommand {
        time,
        buttons: 1,
        weapons: [352, 0, 0],
        ..Default::default()
    };
    p.apply(&command(1001));
    p.apply(&command(1035));
    p.apply(&command(1036));
    p.apply(&command(1071));
    assert_eq!(
        p.take_shots().iter().map(|x| x.time).collect::<Vec<_>>(),
        [1001, 1036, 1071]
    );
    assert_eq!(p.equipment().ammo.unwrap().clip, 33);
    state
        .bootstrap_weapon
        .as_mut()
        .unwrap()
        .ammo
        .as_mut()
        .unwrap()
        .fire_interval_msec = Some(0);
    assert!(state.validate().is_err());
}

#[test]
#[ignore = "requires local captured Afghan arsenal"]
fn captured_special_forms_and_stock_keep_native_inventory_ammo_and_mobility() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("tdm-afghan-hud-state.json")).unwrap())
            .unwrap();
    let arsenal = crate::content::weapons::arsenal::Arsenal::load(
        root.join(state["arsenal_file"].as_str().unwrap()),
        state["ncs_crc"].as_u64().unwrap() as u32,
    )
    .unwrap();
    for item in [10007, 11462, 10553, 10080, 11497, 10604] {
        let resolved = arsenal.resolve(item, &[], &[item], 352).unwrap();
        let weapon =
            PreparedGameState::resolved_with_alternate(&arsenal, resolved, Some(54)).unwrap();
        let alternate = weapon.alternate.as_ref().unwrap();
        assert_eq!(
            weapon.ammo.as_ref().unwrap().clip_key,
            alternate.ammo.as_ref().unwrap().clip_key
        );
        assert_eq!(
            weapon.ammo.as_ref().unwrap().reserve_key,
            alternate.ammo.as_ref().unwrap().reserve_key
        );
        let mut p = PreparedGameState::make_player(&weapon);
        let command = |time, form| codol_protocol::game::channel::usercmd::UserCommand {
            time,
            weapons: [form, if form == 353 { 352 } else { 0 }, 0],
            ..Default::default()
        };
        p.apply(&command(1000, 352));
        p.apply(&command(1100, 353));
        p.apply(&command(1150, 353));
        p.apply(&command(3000, 353));
        let e = p.equipment();
        assert_eq!(e.weapon, 353);
        assert_eq!(e.alternate_weapon, Some(352));
        assert_eq!(e.alternates[0].unwrap().weapon, 353);
        assert_eq!(e.ammo.unwrap().clip, weapon.ammo.as_ref().unwrap().clip);
        let body = codol_protocol::game::snapshot::player_frame(
            3050,
            3000,
            Some([0.0; 3]),
            true,
            Some(e),
            [0; 3],
        )
        .unwrap();
        codol_protocol::game::channel::wire::server_packet(4, 0, 0, b"", &body).unwrap();
        eprintln!(
            "item={item} base={} alt={} inventory={:?}/{:?} shared_clip={} shared_reserve={}",
            weapon.name,
            alternate.name,
            e.inventory_weapon,
            e.alternates[0].map(|w| w.weapon),
            e.ammo.unwrap().clip_key,
            e.ammo.unwrap().reserve_key
        );
    }
    for (parts, expected) in [(vec![], 1.0), (vec![22142], 2.2), (vec![22142, 20743], 2.3)] {
        let mut owned = vec![10001];
        owned.extend(&parts);
        let resolved = arsenal.resolve(10001, &parts, &owned, 352).unwrap();
        let weapon =
            PreparedGameState::resolved_weapon(resolved, &arsenal.source_sha256, Some(54)).unwrap();
        let p = PreparedGameState::make_player(&weapon);
        assert!((p.move_scale() - 0.95).abs() < 1e-6);
        assert!((p.ads_move_scale() - expected).abs() < 1e-6);
        eprintln!(
            "M4 parts={parts:?} hip={} ADS={} native_speed={}",
            p.move_scale(),
            p.ads_move_scale(),
            190.0 * 0.4 * p.ads_move_scale()
        );
    }
}

#[test]
#[ignore = "requires local captured Afghan arsenal"]
fn captured_guns_keep_native_melee_timings_ammo_and_snapshot_encoding() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("tdm-afghan-hud-state.json")).unwrap())
            .unwrap();
    let arsenal = crate::content::weapons::arsenal::Arsenal::load(
        root.join(state["arsenal_file"].as_str().unwrap()),
        state["ncs_crc"].as_u64().unwrap() as u32,
    )
    .unwrap();
    for (name, duration, delay, recover) in [
        ("m4_mp", 699, 250, 400),
        ("ak104common_mp", 699, 250, 400),
        ("usp_mp", 500, 50, 250),
    ] {
        let w = PreparedGameState::resolved_weapon(
            arsenal.bot_weapon(name, 352).unwrap(),
            &arsenal.source_sha256,
            Some(54),
        )
        .unwrap();
        let timing = w.melee.unwrap();
        assert_eq!(
            (
                timing.duration_msec,
                timing.hit_msec,
                timing.recover_msec,
                timing.damage
            ),
            (duration, delay, recover, 135)
        );
        let mut p = PreparedGameState::make_player(&w);
        let clip = p.equipment().ammo.unwrap().clip;
        for (time, state) in [
            (1100, 13),
            (1100 + u32::from(delay), 14),
            (1100 + u32::from(duration), 15),
            (1100 + u32::from(duration + recover), 0),
        ] {
            p.apply(&codol_protocol::game::channel::usercmd::UserCommand {
                time,
                buttons: 4,
                weapons: [352, 0, 0],
                ..Default::default()
            });
            assert_eq!(p.equipment().firing.unwrap().state, state);
            let body = codol_protocol::game::snapshot::player_frame(
                time,
                time,
                Some([0.0; 3]),
                true,
                Some(p.equipment()),
                [0; 3],
            )
            .unwrap();
            codol_protocol::game::channel::wire::server_packet(4, 0, 0, b"", &body).unwrap();
        }
        assert_eq!(p.take_melee_strikes().len(), 1);
        assert_eq!(p.equipment().ammo.unwrap().clip, clip);
        eprintln!(
            "{name} melee={duration}/{delay}/{recover}ms damage=135 clip={clip} unchanged; states13/14/15/0 encoded"
        );
    }
}
#[test]
fn projectile_rifle_and_pistol_classes_never_create_hitscan_damage() {
    for (kind, class, shoots_bullets) in [
        (0, 0, true),
        (0, 5, true),
        (2, 0, false),
        (2, 5, false),
        (2, 7, false),
    ] {
        let mut definition = vec![0; crate::content::weapons::attributes::WEAPON_DEF_SIZE];
        for (offset, value) in [
            (0x47c, kind),
            (0x480, class),
            (0x6b8, 1),
            (0xd08, 2),
            (0xd10, 80),
            (0x71c, 500),
            (0x728, 700),
            (0x730, 400),
            (0x738, 600),
            (0x74c, 100),
            (0x750, 100),
            (0x7c8, 1.0f32.to_bits()),
            (0xbb4, 40),
            (0xbcc, 1000.0f32.to_bits()),
        ] {
            definition[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let weapon = PreparedGameState::resolved_weapon(
            crate::content::weapons::arsenal::ResolvedWeapon {
                name: "test_mp".into(),
                base_index: 1,
                index: 352,
                attachments: vec![],
                composed_name: "** 1 0 0".into(),
                definition,
                melee_charge_animation: false,
            },
            &"a".repeat(64),
            Some(54),
        )
        .unwrap();
        let mut player = PreparedGameState::make_player(&weapon);
        player.apply(&codol_protocol::game::channel::usercmd::UserCommand {
            time: 1001,
            buttons: 1,
            weapons: [352, 0, 0],
            ..Default::default()
        });
        assert_eq!(player.equipment().ammo.unwrap().clip, 1);
        assert_eq!(
            player.take_shots().len(),
            usize::from(shoots_bullets),
            "kind={kind} class={class}"
        );
    }
}
#[test]
#[ignore = "requires local captured Afghan arsenal"]
fn captured_crossbow_and_ballistic_knife_types_cannot_enter_bullet_combat() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("tdm-afghan-hud-state.json")).unwrap())
            .unwrap();
    let arsenal = crate::content::weapons::arsenal::Arsenal::load(
        root.join(state["arsenal_file"].as_str().unwrap()),
        state["ncs_crc"].as_u64().unwrap() as u32,
    )
    .unwrap();
    for name in ["crossbow_mp", "crossbowrare_mp", "knifeballistic_mp"] {
        let mut resolved = arsenal.bot_weapon(name, 352).unwrap();
        assert_eq!(
            u32::from_le_bytes(resolved.definition[0x47c..0x480].try_into().unwrap()),
            2
        );
        let original_range =
            f32::from_le_bytes(resolved.definition[0xbcc..0xbd0].try_into().unwrap());
        let weapon =
            PreparedGameState::resolved_weapon(resolved.clone(), &arsenal.source_sha256, Some(54))
                .unwrap();
        assert!(weapon.ballistics.is_none());
        // Native data has range999999, which the old generic range check
        // also rejected. A legal-range variation must remain projectile.
        resolved.definition[0xbcc..0xbd0].copy_from_slice(&1000.0f32.to_le_bytes());
        let weapon =
            PreparedGameState::resolved_weapon(resolved, &arsenal.source_sha256, Some(54)).unwrap();
        assert!(weapon.ballistics.is_none());
        eprintln!(
            "{name}: native_type=2 original_range={original_range} legal_range_variation_has_no_hitscan=true"
        );
    }
}
#[test]
#[ignore = "requires local captured Afghan arsenal"]
fn captured_ranger_triggers_use_the_composed_two_shell_magazine() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let state = PreparedGameState::load(&root.join("tdm-afghan-hud-state.json")).unwrap();
    let arsenal = state.arsenal.as_ref().unwrap();
    let weapon = PreparedGameState::resolved_weapon(
        arsenal.bot_weapon("ranger_mp", 352).unwrap(),
        &arsenal.source_sha256,
        Some(54),
    )
    .unwrap();
    assert_eq!(weapon.fire_type, Some(5));
    assert_eq!(weapon.ammo.as_ref().unwrap().clip, 2);
    let command = |time, buttons| codol_protocol::game::channel::usercmd::UserCommand {
        time,
        buttons,
        weapons: [352, 0, 0],
        ..Default::default()
    };
    let mut p = PreparedGameState::make_player(&weapon);
    p.apply(&command(1001, 0x100000));
    assert_eq!(p.equipment().ammo.unwrap().clip, 1);
    p.apply(&command(1010, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 0);
    assert_eq!(p.take_shots().len(), 2);
    let mut p = PreparedGameState::make_player(&weapon);
    p.apply(&command(1001, 0x100001));
    let shots = p.take_shots();
    assert_eq!(shots.len(), 2);
    assert_eq!(p.equipment().barrel_flags, 3);
    assert!(shots.iter().all(|s| s.ballistics.pellets > 1));
    let body = codol_protocol::game::snapshot::player_frame(
        1001,
        1001,
        Some([0.0; 3]),
        true,
        Some(p.equipment()),
        [0; 3],
    )
    .unwrap();
    codol_protocol::game::channel::wire::server_packet(4, 0, 0, b"", &body).unwrap();
    eprintln!(
        "Ranger: interval={}ms pellets={}+{} snapshot={} bytes",
        weapon.ammo.unwrap().fire_interval_msec.unwrap(),
        shots[0].ballistics.pellets,
        shots[1].ballistics.pellets,
        body.len()
    );
}
#[test]
#[ignore = "requires independently captured map states in CODOL_PRESENTATION_STATES"]
fn captured_maps_have_grounded_podiums_within_snapshot_budget() {
    let paths =
        std::env::var_os("CODOL_PRESENTATION_STATES").expect("set captured map state paths");
    for path in std::env::split_paths(&paths) {
        let mut state = PreparedGameState::load(&path).unwrap();
        let config = state.match_config.as_ref().unwrap();
        let mut actors = vec![state.local_player_snapshot(true).unwrap()];
        for (i, entry) in config.roster.iter().enumerate() {
            let mut actor = actors[0];
            actor.client_index = i as u8 + 1;
            actor.entity_number = i as u16 + 1;
            actor.model = entry.animation.model;
            actor.attachment_models = entry.attachment_models;
            actor.team = config.teams[i + 1];
            actor.weapon = 352 + i as u16;
            actors.push(actor);
        }
        let rows: Vec<_> = (0..12)
            .map(
                |client_number| codol_protocol::game::snapshot::scoreboard::ScoreboardRow {
                    client_number,
                    score: 1200 - i32::from(client_number) * 100,
                    ..Default::default()
                },
            )
            .collect();
        let scene = state.top_players_scene(&rows, actors.clone()).unwrap();
        state.player_card = Some(codol_protocol::game::snapshot::player_card::Card {
            rank: 69,
            prestige: 2,
            icon: 138,
            title: 214,
            background: 396,
            nameplate: 0,
        });
        let live = state
            .snapshot(
                10000,
                10000,
                true,
                [0; 3],
                state.player().map(|p| p.equipment()),
                SnapshotWorld {
                    motion: state.motion().as_ref(),
                    local_player: Some(actors[0]),
                    players: &actors[1..],
                    vitals: Some(Default::default()),
                    ..Default::default()
                },
            )
            .unwrap();
        eprintln!(
            "{}: live characters plus held weapons {} bytes",
            state.map,
            live.len()
        );
        assert!(live.len() <= codol_protocol::game::channel::wire::FRAGMENT_SIZE - 5);
        let mut firing = actors.clone();
        for actor in &mut firing {
            for _ in 0..2 {
                actor
                    .events
                    .push(codol_protocol::game::snapshot::events::FIRE_WEAPON, 31)
                    .unwrap();
                actor
                    .events
                    .push(codol_protocol::game::snapshot::events::EJECT_BRASS, 0)
                    .unwrap();
            }
        }
        let impacts: Vec<_> = (54..62)
            .map(
                |entity_number| codol_protocol::game::snapshot::impact::ImpactSnapshot {
                    event: Default::default(),
                    entity_number,
                    origin: [123.0, 456.0, 70.0],
                    normal_byte: 0,
                    surface: 1,
                    weapon: 352,
                },
            )
            .collect();
        let delta = state
            .snapshot(
                10050,
                10050,
                false,
                [0; 3],
                state.player().map(|p| p.equipment()),
                SnapshotWorld {
                    delta_distance: 1,
                    motion: state.motion().as_ref(),
                    local_player: Some(firing[0]),
                    players: &firing[1..],
                    local_player_baseline: Some(actors[0]),
                    player_baselines: &actors[1..],
                    impacts: &impacts,
                    held_weapons_in_baseline: false,
                    vitals: Some(Default::default()),
                    ..Default::default()
                },
            )
            .unwrap();
        eprintln!(
            "{}: first weapon delta with firing/events {} bytes",
            state.map,
            delta.len()
        );
        assert!(delta.len() <= codol_protocol::game::channel::wire::FRAGMENT_SIZE - 5);
        let snapshot = state
            .snapshot(
                10000,
                10000,
                true,
                scene.angles,
                state.player().map(|p| p.equipment()),
                SnapshotWorld {
                    motion: Some(&scene.camera),
                    local_player: Some(scene.actors[0]),
                    players: &scene.actors[1..],
                    corpses: &scene.poses,
                    vitals: Some(codol_protocol::game::snapshot::PlayerVitals {
                        intermission: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        eprintln!(
            "{}: camera={:?}, actors={:?}, bytes={}",
            state.map,
            scene.camera.origin,
            scene.poses.iter().map(|a| a.origin).collect::<Vec<_>>(),
            snapshot.len()
        );
        assert!(snapshot.len() <= codol_protocol::game::channel::wire::FRAGMENT_SIZE - 5);
    }
}
#[test]
fn first_spawn_below_terrain_is_grounded_like_a_respawn() {
    let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
        "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,62],"half":[4000,4000,10],"contents":1,"planes":[]}],"triangles":[]});
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
    let collision = crate::game::world::collision::Collision::load(file.path()).unwrap();
    let mut state = synthetic();
    // Floor top is z=72; the raw entity origin is four units inside it.
    state.spawn_origin = Some([0.0, 0.0, 68.0]);
    state.collision = Some(Arc::new(collision));
    state.ground_first_spawn().unwrap();
    let origin = state.spawn_origin.unwrap();
    assert!((72.0..72.5).contains(&origin[2]), "{origin:?}");
    let collision = state.collision.clone().unwrap();
    let mut motion = state.motion().unwrap();
    for _ in 0..30 {
        let cmd = codol_protocol::game::channel::usercmd::UserCommand {
            time: motion.command_time + 16,
            direction: 1,
            ..Default::default()
        };
        motion.apply(&cmd, 1.0, false, &collision);
    }
    assert!(motion.origin[0] > 50.0, "{:?}", motion.origin);
}
#[test]
fn match_summary_map_ids_follow_the_current_map_table() {
    let mut state = synthetic();
    for (map, id) in [
        ("mp_afghan", 1),
        ("mp_crash_tropical", 4),
        ("mp_killhouse_sh", 8),
    ] {
        state.map = map.into();
        assert_eq!(state.summary_map_id().unwrap(), id);
    }
    state.map = "mp_unknown".into();
    assert!(state.summary_map_id().is_err());
}
#[test]
fn walking_full_body_animation_is_not_overridden_by_a_standing_torso_pose() {
    let mut state = synthetic();
    let config:crate::game::ai::enemy::EnemyConfig=serde_json::from_value(serde_json::json!({
        "model":4,"origin":[0,0,0],"bounds_min":[-15,-15,0],"bounds_max":[15,15,70],
        "idle_animation":395,"walk_animation":243,"walk_left_animation":241,"walk_right_animation":240,"walk_back_animation":242,
        "fire_animation":616,"death_animation":47,"move_speed":94.0,"angles":[0,0,0],"weapon":352,"source_sha256":"a".repeat(64)
    })).unwrap();
    state.training_enemy = Some(config.clone());
    let mut motion = crate::game::player::movement::Motion::new([0.0; 3]);
    motion.velocity = [94.0, 0.0, 0.0];
    let mut frame = crate::game::ai::controller::BotFrame {
        landings: Vec::new(),
        motion,
        angles: [0; 3],
        phase: crate::game::ai::controller::BotPhase::Chase,
        shot: None,
        locomotion: crate::game::ai::controller::Locomotion::Walk,
    };
    let current = state.animate_enemy(config.snapshot(true), &frame, false, false);
    assert_eq!((current.legs_animation, current.torso_animation), (243, 0));
    frame.motion.velocity = [-94.0, 0.0, 0.0];
    assert_eq!(
        state
            .animate_enemy(current, &frame, false, false)
            .legs_animation,
        242
    );
    frame.motion.velocity = [0.0, 94.0, 0.0];
    assert_eq!(
        state
            .animate_enemy(current, &frame, false, false)
            .legs_animation,
        241
    );
    frame.motion.velocity = [0.0, -94.0, 0.0];
    assert_eq!(
        state
            .animate_enemy(current, &frame, false, false)
            .legs_animation,
        240
    );
    assert_eq!(
        state
            .animate_enemy(current, &frame, true, true)
            .torso_animation,
        616 | 0x400
    );
    state.training_enemy.as_mut().unwrap().locomotion =
        Some(crate::game::ai::enemy::NativeLocomotion {
            run_forward: 328,
            run_back: 325,
            run_left: 328,
            run_right: 328,
            sprint: 309,
            fire_standing: 617,
            script_sha256: "a".repeat(64),
        });
    frame.motion.velocity = [190.0, 0.0, 0.0];
    frame.motion.flags = 0;
    frame.locomotion = crate::game::ai::controller::Locomotion::Run;
    let running = state.animate_enemy(current, &frame, true, false);
    assert_eq!((running.legs_animation, running.torso_animation), (328, 0));
    frame.motion.flags = 0x4000;
    frame.locomotion = crate::game::ai::controller::Locomotion::Sprint;
    assert_eq!(
        state
            .animate_enemy(current, &frame, false, false)
            .legs_animation,
        309
    );
    frame.motion.flags = 0x10;
    frame.locomotion = crate::game::ai::controller::Locomotion::Walk;
    assert_eq!(
        state
            .animate_enemy(current, &frame, false, false)
            .legs_animation,
        243
    );
    frame.motion.velocity = [0.0; 3];
    let standing = state.animate_enemy(current, &frame, true, false);
    assert_eq!(
        (standing.legs_animation, standing.torso_animation),
        (395, 617)
    );
}
#[test]
fn prepared_training_state_cannot_match_another_playlist_or_console_suffix() {
    let state = synthetic();
    let start =
        "mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist 0; start_map_rotate_once 0";
    assert!(state.matches(-688906341, start));
    assert!(!state.matches(0, start));
    assert!(!state.matches(-688906341, &start.replace("playlist 0", "playlist 1")));
    assert!(!state.matches(-688906341, &format!("{start}; map other")));
}
#[test]
fn bootstrap_supplies_a_complete_normal_speed_slowmotion_config() {
    let body = synthetic().body(1).unwrap();
    let mut r = codol_protocol::game::channel::wire::Reader::new(&body);
    assert_eq!(r.u8().unwrap(), 3);
    r.u32().unwrap();
    r.string().unwrap();
    r.string().unwrap();
    r.u32().unwrap();
    assert_eq!(r.u8().unwrap(), 4);
    let count = r.u16().unwrap();
    let mut index = -1i32;
    let mut slowmo = None;
    for _ in 0..count {
        index = if r.bits(1).unwrap() == 1 {
            index + 1
        } else {
            r.bits(13).unwrap() as i32
        };
        let value = r.string().unwrap();
        if index == 6415 {
            slowmo = Some(std::str::from_utf8(value).unwrap().to_owned());
        }
    }
    let fields = slowmo
        .unwrap()
        .split_whitespace()
        .map(|v| v.parse::<f32>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(fields, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(r.bits(10).unwrap(), 0);
    assert_eq!(r.bits(10).unwrap(), 0);
    assert_eq!(r.u8().unwrap(), 6);
    r.u32().unwrap();
    r.u32().unwrap();
    assert_eq!(r.u8().unwrap(), 6);
    r.finish().unwrap();
}
