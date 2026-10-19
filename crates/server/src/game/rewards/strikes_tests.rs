use super::selection::{Action, Point, Selection};
use super::strikes::*;
use super::{inventory::Inventory, killstreaks::RadarResources};
use crate::content::maps::minimap::Minimap;
use crate::game::world::{collision::Collision, props::Blast};
use codol_protocol::game::channel::usercmd::{
    BUTTON_LOCATION_CANCEL, BUTTON_LOCATION_CONFIRM, UserCommand,
};

pub(crate) fn resources() -> RadarResources {
    let mut r = crate::game::rewards::deployables::test_resources();
    r.air_route = Some(super::killstreaks::AirRoute {
        center: [0.0, 0.0, 1500.0],
        sources_sha256: ["0".repeat(64)],
    });
    r.selectors.insert("map_artillery_selector".into(), 1);
    r.selectors.insert("map_mortar_selector".into(), 2);
    r.effects
        .insert("explosions/artilleryexp_dirt_brown".into(), 67);
    r.models.insert("vehicle_mig29_desert".into(), 147);
    r.models.insert("vehicle_b2_bomber".into(), 140);
    let mut w = r.weapons["sentry_minigun_mp"].clone();
    for (id, weapon) in [(6, 324), (11, 328), (17, 333)] {
        let mut radio = r.radios[0].clone();
        radio.native_id = id;
        radio.weapon = weapon;
        radio.switch.raise_msec = 100;
        r.radios.push(radio);
    }
    w.weapon = 298;
    r.weapons.insert("artillery_mp".into(), w.clone());
    w.weapon = 349;
    r.weapons.insert("stealth_bomb_mp".into(), w.clone());
    w.weapon = 333;
    w.weapon_type = 4;
    w.radius = 512;
    w.inner_damage = 600;
    w.outer_damage = 75;
    r.weapons.insert("mortar_mp".into(), w);
    r
}

#[test]
fn strike_availability_requires_each_registered_weapon_effect_pair() {
    let mut r = resources();
    for id in [6, 11, 17] {
        assert!(crate::game::rewards::mechanisms::Mechanism::radio_available(id, &r));
    }
    r.weapons.remove("artillery_mp");
    assert!(!crate::game::rewards::mechanisms::Mechanism::radio_available(6, &r));
    assert!(crate::game::rewards::mechanisms::Mechanism::radio_available(11, &r));
    r = resources();
    r.weapons.get_mut("mortar_mp").unwrap().weapon = 0;
    assert!(!crate::game::rewards::mechanisms::Mechanism::radio_available(17, &r));
    r = resources();
    assert!(!crate::game::rewards::mechanisms::Mechanism::radio_available(7, &r));
}

#[test]
fn harrier_is_advertised_only_with_the_full_attack_aircraft_registration() {
    let mut r = resources();
    let mut radio = r.radios[0].clone();
    radio.native_id = 7;
    radio.weapon = 316;
    r.radios.push(radio);
    r.models.insert("vehicle_av8b_harrier_jet_mp".into(), 139);
    r.model_bounds.insert(
        "vehicle_av8b_harrier_jet_mp".into(),
        crate::game::rewards::killstreaks::ModelBounds {
            min: [-50.0, -50.0, -50.0],
            max: [50.0, 50.0, 50.0],
        },
    );
    r.vehicles.insert("harrier_mp".into(), 4);
    r.effects
        .insert("explosions/aerial_explosion_harrier".into(), 71);
    r.effects.insert("explosions/aerial_explosion".into(), 5);
    r.effects
        .insert("smoke/smoke_trail_white_heli_emitter".into(), 47);
    r.effects
        .insert("smoke/smoke_trail_black_heli_emitter".into(), 45);
    let mut gun = r.weapons["artillery_mp"].clone();
    gun.weapon = 325;
    r.weapons.insert("harrier_20mm_mp".into(), gun);
    let mut missile = r.weapons["artillery_mp"].clone();
    missile.weapon = 328;
    missile.weapon_type = 2;
    missile.projectile = Some(crate::game::rewards::killstreaks::ProjectileFacts {
        lifetime_msec: 7000,
        explosion_kind: 1,
    });
    missile.projectile_speed = 4000;
    missile.damage_stages[0] = 100;
    missile.radius = 100;
    missile.inner_damage = 100;
    r.weapons.insert("harrier_missile_mp".into(), missile);
    r.air_paths = Some(
        serde_json::from_value(serde_json::json!({
            "sources_sha256":["0".repeat(64)],
            "entries":{"heli_start":[0],"heli_loop_start":[2],"heli_leave":[4]},
            "nodes":[
                {"name":"heli_start","origin":[0,0,1000],"target":1},
                {"name":"ingress","origin":[100,0,1000],"target":null},
                {"name":"heli_loop_start","origin":[100,100,1000],"target":3},
                {"name":"loop","origin":[200,100,1000],"target":2},
                {"name":"heli_leave","origin":[1000,1000,1000],"target":null}
            ]
        }))
        .unwrap(),
    );
    assert!(crate::game::rewards::mechanisms::Mechanism::radio_available(7, &r));
    r.vehicles.remove("harrier_mp");
    assert!(!crate::game::rewards::mechanisms::Mechanism::radio_available(7, &r));
}
fn minimap() -> Minimap {
    serde_json::from_value(serde_json::json!({"material":"compass_map_mp_afghan","north_yaw":0,"upper_left":[1000,1000],"lower_right":[-1000,-1000]})).unwrap()
}
fn collision(roof: bool) -> Collision {
    let f = tempfile::NamedTempFile::new().unwrap();
    let mut brushes = vec![
        serde_json::json!({"center":[0,0,-10],"half":[16000,16000,10],"contents":1,"planes":[]}),
    ];
    if roof {
        brushes.push(
            serde_json::json!({"center":[0,0,90],"half":[1000,1000,5],"contents":1,"planes":[]}),
        );
        brushes.push(
            serde_json::json!({"center":[150,0,100],"half":[5,1000,100],"contents":1,"planes":[]}),
        );
    }
    std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]})).unwrap()).unwrap();
    Collision::load(f.path()).unwrap()
}
#[test]
fn compass_endpoints_and_rotated_map_inverse_preserve_world_positions() {
    for yaw in [0.0f32, 37.0, 90.0, 180.0, 270.0] {
        let mut map = minimap();
        map.north_yaw = yaw;
        let (sin, cos) = yaw.to_radians().sin_cos();
        map.lower_right = [
            map.upper_left[0] + sin * 2000.0 - cos * 1000.0,
            map.upper_left[1] - cos * 2000.0 - sin * 1000.0,
        ];
        assert_eq!(map.selected_position([128, 128, 0]).0, map.upper_left);
        for (a, b) in map
            .selected_position([127, 127, 255])
            .0
            .into_iter()
            .zip(map.lower_right)
        {
            assert!((a - b).abs() < 0.001);
        }
        assert!((map.selected_position([127, 127, 255]).1 - yaw).abs() < 0.001);
        assert!((map.world_height() - 1000.0).abs() < 0.001);
    }
}
#[test]
fn mortar_collects_three_distinct_edges_and_cancel_keeps_exact_stock() {
    let r = resources();
    let map = minimap();
    let c = collision(false);
    let mut inventory = Inventory::default();
    assert!(inventory.grant(0, 17, 0));
    let ticket = inventory.reserve(0, 17, 0).unwrap();
    let mut input = UserCommand {
        time: 1000,
        weapons: [333, 0, 0],
        ..Default::default()
    };
    let mut selection = Selection::new(ticket, 0, &input, &r, &map).unwrap();
    for n in 0..3 {
        input.time = 1200 + n * 200;
        input.buttons = BUTTON_LOCATION_CONFIRM;
        input.selected = [100 + n as u8 * 30, 128, 0];
        assert!(matches!(selection.command(&input, &map, &c), Action::None) || n == 2);
        input.time += 50;
        assert!(matches!(selection.command(&input, &map, &c), Action::None));
        assert_eq!(selection.points.len(), n as usize + 1);
        input.time += 50;
        input.buttons = 0;
        selection.command(&input, &map, &c);
    }
    assert!(inventory.contains(17));
    assert_eq!(selection.points.len(), 3);
    assert!(inventory.cancel(ticket));
    let newer = inventory.reserve(0, 17, 0).unwrap();
    assert_ne!(newer, ticket);
    assert!(!inventory.commit(ticket, 0, 0, true));
    assert!(inventory.commit(newer, 0, 0, true));
}
#[test]
fn confirmation_wins_but_missing_ground_never_completes_a_strike() {
    let r = resources();
    let map = minimap();
    let c = collision(false);
    let mut i = Inventory::default();
    i.grant(0, 6, 0);
    let mut input = UserCommand {
        time: 1000,
        weapons: [324, 0, 0],
        ..Default::default()
    };
    let mut s = Selection::new(i.reserve(0, 6, 0).unwrap(), 0, &input, &r, &map).unwrap();
    input.time = 1200;
    input.buttons = BUTTON_LOCATION_CONFIRM | BUTTON_LOCATION_CANCEL;
    input.selected = [0, 0, 0];
    assert!(matches!(s.command(&input, &map, &c), Action::Complete));
    assert!(s.snapshot().directional);
}
#[test]
fn ballistic_mortar_hits_selected_ground_and_emits_owned_damage_once() {
    let r = resources();
    let c = collision(false);
    let mut strikes = Strikes::default();
    let points = [Point {
        origin: [200.0, 150.0, 0.0],
        yaw: 0.0,
        coverage: None,
    }; 3];
    let plan = strikes
        .plan(
            1000,
            17,
            Source {
                owner: 0,
                team: 1,
                life: 7,
            },
            &points,
            &r,
            &c,
        )
        .unwrap();
    strikes.launch(plan);
    assert_eq!(strikes.missiles(1000).len(), 1);
    let mut hits = Vec::new();
    for now in (1000..30000).step_by(50) {
        hits.extend(strikes.advance(now, false, 0, &c));
    }
    assert_eq!(hits.len(), 6);
    for h in hits {
        assert_eq!(
            (h.blast.owner, h.team, h.life, h.blast.weapon),
            (0, 1, 7, 333)
        );
        assert!((h.blast.origin[0] - 200.0).abs() < 5.0);
        assert!((h.blast.origin[1] - 150.0).abs() < 1.0);
        assert!(damage(h, [200.0, 150.0, 0.0], 40.0, &c) > 500);
    }
    assert!(strikes.advance(31000, false, 0, &c).is_empty());
    assert!(strikes.missiles(31000).is_empty());
}
#[test]
fn emp_cancels_unreleased_attacks_and_match_end_clears_all_entities() {
    let r = resources();
    let c = collision(false);
    let points = [Point {
        origin: [0.0, 0.0, 0.0],
        yaw: 0.0,
        coverage: None,
    }; 3];
    let mut strikes = Strikes::default();
    strikes.launch(
        strikes
            .plan(
                1000,
                17,
                Source {
                    owner: 0,
                    team: 1,
                    life: 0,
                },
                &points,
                &r,
                &c,
            )
            .unwrap(),
    );
    // One shell already released. Future launches are cancelled by EMP.
    let mut hits = Vec::new();
    for now in (1000..25000).step_by(100) {
        hits.extend(strikes.advance(now, false, 2, &c));
    }
    assert_eq!(hits.len(), 1);
    strikes.launch(
        strikes
            .plan(
                30000,
                6,
                Source {
                    owner: 0,
                    team: 1,
                    life: 0,
                },
                &points[..1],
                &r,
                &c,
            )
            .unwrap(),
    );
    assert!(!strikes.planes(30000).is_empty());
    assert!(strikes.advance(30001, true, 0, &c).is_empty());
    assert!(strikes.planes(30001).is_empty());
    assert!(strikes.drain().is_empty());
}
#[test]
fn airstrike_indoor_advantage_changes_damage_and_mortar_uses_line_of_sight() {
    let c = collision(true);
    let mut h = Hit {
        blast: Blast {
            reward_life: None,
            owner: 0,
            source_entity: None,
            weapon: 298,
            origin: [300.0, 0.0, 16.0],
            radius: 512.0,
            inner: 200,
            outer: 30,
        },
        team: 1,
        life: 0,
        airstrike: true,
        area: None,
    };
    assert_eq!(damage(h, [0.0, 0.0, 0.0], 40.0, &c), 0);
    h.blast.origin = [50.0, 0.0, 16.0];
    assert!(damage(h, [0.0, 0.0, 0.0], 40.0, &c) > 100);
    h.airstrike = false;
    h.blast.origin = [0.0, 0.0, 150.0];
    assert_eq!(damage(h, [0.0, 0.0, 0.0], 40.0, &c), 0);
}
#[test]
fn all_registered_strikes_use_bounded_native_entities_and_capacity_retains_calls() {
    let r = resources();
    let c = collision(false);
    let p = [Point {
        origin: [0.0, 0.0, 0.0],
        yaw: 45.0,
        coverage: None,
    }; 3];
    let mut strikes = Strikes::default();
    for id in [6, 11, 17] {
        let plan = strikes
            .plan(
                1000,
                id,
                Source {
                    owner: 0,
                    team: 1,
                    life: 0,
                },
                if id == 17 { &p } else { &p[..1] },
                &r,
                &c,
            )
            .unwrap();
        strikes.launch(plan);
    }
    let mut counts = std::collections::BTreeMap::new();
    for now in (1000..40000).step_by(50) {
        for hit in strikes.advance(now, false, 0, &c) {
            *counts.entry(hit.blast.weapon).or_insert(0) += 1;
        }
        for plane in strikes.planes(now) {
            assert!(plane.origin.iter().all(|v| v.abs() < 32768.0));
        }
    }
    assert_eq!(
        counts,
        std::collections::BTreeMap::from([(298, 36), (349, 12), (333, 6)])
    );
    assert!(strikes.planes(40000).is_empty());
    assert!(strikes.missiles(40000).is_empty());
}

#[test]
#[ignore = "requires matching local AFP resources and collision capture"]
fn captured_afghan_strikes_bind_real_selector_weapon_and_surface_resources() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lab = root.join("research-output/killstreak-controls-20261014");
    let map: serde_json::Value =
        serde_json::from_slice(&std::fs::read(lab.join("map.json")).unwrap()).unwrap();
    let r = RadarResources::load(
        &lab.join("resources-afghan-selectors.json"),
        "mp_afghan",
        523897474,
    )
    .unwrap();
    let c = Collision::load(std::path::Path::new(
        map["collision_file"].as_str().unwrap(),
    ))
    .unwrap();
    let minimap: Minimap = serde_json::from_value(map["minimap"].clone()).unwrap();
    assert_eq!(r.selectors["map_artillery_selector"], 1);
    assert_eq!(r.selectors["map_mortar_selector"], 2);
    let (xy, yaw) = minimap.selected_position([0, 0, 0]);
    let target = super::selection::ground(xy, r.air_route.as_ref().unwrap().center[2], &c).unwrap();
    let points = [Point {
        origin: target,
        yaw,
        coverage: None,
    }; 3];
    let mut strikes = Strikes::default();
    for id in [6, 11, 17] {
        strikes.launch(
            strikes
                .plan(
                    1000,
                    id,
                    Source {
                        owner: 0,
                        team: 2,
                        life: 0,
                    },
                    if id == 17 { &points } else { &points[..1] },
                    &r,
                    &c,
                )
                .unwrap(),
        );
    }
    let mut counts = std::collections::BTreeMap::new();
    for now in (1000..40000).step_by(50) {
        for hit in strikes.advance(now, false, 0, &c) {
            *counts.entry(hit.blast.weapon).or_insert(0) += 1;
        }
    }
    for weapon in [298, 349, 333] {
        assert!(counts[&weapon] > 0);
    }
    println!("captured terrain={target:?} owned impact counts={counts:?}");
}

#[test]
fn selector_quantization_binds_bomb_centers_and_damage_to_one_world_circle() {
    let r = resources();
    let map = minimap();
    let c = collision(false);
    for id in [6, 11] {
        let mut stock = Inventory::default();
        stock.grant(0, id, 0);
        let mut cmd = UserCommand {
            time: 1000,
            weapons: [r.radio(id).unwrap().weapon, 0, 0],
            ..Default::default()
        };
        let mut selection =
            Selection::new(stock.reserve(0, id, 0).unwrap(), 0, &cmd, &r, &map).unwrap();
        let expected = f32::from(selection.snapshot().radius) * map.world_height() / 62.0;
        cmd.time = 2000;
        cmd.buttons = BUTTON_LOCATION_CONFIRM;
        cmd.selected = [0, 0, 0];
        assert!(matches!(
            selection.command(&cmd, &map, &c),
            Action::Complete
        ));
        assert_eq!(selection.points[0].coverage, Some(expected));
        let center = selection.points[0].origin;
        let mut strikes = Strikes::default();
        strikes.launch(
            strikes
                .plan(
                    2000,
                    id,
                    Source {
                        owner: 0,
                        team: 1,
                        life: 0,
                    },
                    &selection.points,
                    &r,
                    &c,
                )
                .unwrap(),
        );
        let mut count = 0;
        for now in (2000..40000).step_by(50) {
            for hit in strikes.advance(now, false, 0, &c) {
                count += 1;
                let distance =
                    (hit.blast.origin[0] - center[0]).hypot(hit.blast.origin[1] - center[1]);
                assert!(distance + hit.blast.radius <= expected + 0.01);
                assert_eq!(
                    damage(hit, [center[0] + expected + 1.0, center[1], 0.0], 40.0, &c),
                    0
                );
            }
        }
        assert!(count > 0);
    }
}
