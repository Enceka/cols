use super::*;
pub(crate) fn resources() -> RadarResources {
    let mut r = super::super::remote::test_resources();
    r.models.extend([
        ("tag_origin".into(), 48),
        ("c130_zoomrig".into(), 97),
        ("vehicle_ac130_low_mp".into(), 138),
        ("vehicle_mi-28_mp".into(), 145),
    ]);
    for model in ["vehicle_ac130_low_mp", "vehicle_mi-28_mp"] {
        r.model_bounds.insert(
            model.into(),
            ModelBounds {
                min: [-50.0, -50.0, -50.0],
                max: [50.0, 50.0, 50.0],
            },
        );
    }
    for (model, p) in [
        ("c130_zoomrig", [1000.0, 0.0, 2000.0]),
        ("vehicle_mi-28_mp", [40.0, 0.0, -100.0]),
    ] {
        r.camera_mounts.insert(
            model.into(),
            CameraMount {
                rotation: [0.0, 0.0, 0.0, 1.0],
                translation: p,
            },
        );
    }
    r.vehicles.insert("cobra_minigun_mp".into(), 2);
    r.animations.insert(
        "bh_rotors".into(),
        super::super::killstreaks::AnimationBinding {
            tree: 3,
            node: 11,
            parent: 10,
        },
    );
    let base = r.weapons["sentry_minigun_mp"].clone();
    for (name, index, clip, reload, interval) in [
        ("ac130_105mm_mp", 289, 1, 5000, 100),
        ("ac130_40mm_mp", 291, 4, 3000, 300),
        ("ac130_25mm_mp", 290, 20, 1500, 70),
    ] {
        let mut w = base.clone();
        w.weapon = index;
        w.clip_key = Some(index);
        w.ammo_key = Some(index);
        w.weapon_type = 2;
        w.projectile = Some(super::super::killstreaks::ProjectileFacts {
            lifetime_msec: 7000,
            explosion_kind: 1,
        });
        w.projectile_speed = 3000;
        w.radius = 600;
        w.inner_damage = 1200;
        w.outer_damage = 300;
        w.clip_size = clip;
        w.reload_msec = reload;
        w.fire_interval_msec = interval;
        r.weapons.insert(name.into(), w);
    }
    let mut w = base;
    w.weapon = 304;
    w.clip_key = Some(304);
    w.ammo_key = Some(304);
    w.fire_interval_msec = 79;
    w.damage_stages = [150, 0, 0, 0, 0, 150];
    w.damage_ranges = [2000.0, 0.0, 0.0, 0.0, 0.0, 2500.0];
    r.weapons.insert("cobra_player_minigun_mp".into(), w);
    r.air_route = Some(super::super::killstreaks::AirRoute {
        center: [0.0; 3],
        sources_sha256: ["0".repeat(64)],
    });
    r.air_paths = Some(
        serde_json::from_value(serde_json::json!({"sources_sha256":["0".repeat(64)],
        "entries":{"heli_start":[0],"heli_loop_start":[2]},"nodes":[
            {"name":"heli_start","origin":[0,0,1500],"target":1},
            {"name":"ingress","origin":[100,0,1500],"target":null},
            {"name":"heli_loop_start","origin":[100,100,1500],"target":3},
            {"name":"loop","origin":[1000,100,1500],"target":2}]}))
        .unwrap(),
    );
    r
}
fn collision() -> Collision {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,
        "client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,
        "brushes":[{"center":[0,0,-10],"half":[30000,30000,10],"contents":1,"planes":[]}],"triangles":[]})).unwrap()).unwrap();
    Collision::load(f.path()).unwrap()
}
fn launch(g: &mut Gunners, c: &Collision, id: u8) -> Key {
    let plan = g.plan(&resources(), id, 0, c).unwrap();
    g.launch(
        plan,
        1000,
        1,
        Source {
            owner: 0,
            team: 2,
            life: 7,
        },
        c,
    )
    .0
}
fn command(time: u32, weapon: u16, buttons: u32) -> UserCommand {
    UserCommand {
        time,
        weapons: [weapon, 0, 0],
        buttons,
        ..Default::default()
    }
}
fn equipment(g: &Gunners) -> Equipment {
    g.equipment(crate::game::weapons::held::drop_test_gun(352, 352, 19, 77).equipment())
        .unwrap()
}

#[test]
fn plan_requires_real_mounts_and_weapons_and_disjoint_vehicle_slots() {
    let c = collision();
    let g = Gunners::default();
    let mut r = resources();
    assert!(g.plan(&r, 12, u16::MAX, &c).is_none());
    assert_eq!(g.plan(&r, 12, 3, &c).unwrap().slot, 2);
    r.camera_mounts.remove("c130_zoomrig");
    assert!(g.plan(&r, 13, 0, &c).is_none());
    r = resources();
    r.weapons.get_mut("ac130_25mm_mp").unwrap().clip_size = 0;
    assert!(!Gunners::resources_present(&r, 13));
    r = resources();
    r.air_paths = None;
    assert!(g.plan(&r, 12, 0, &c).is_none());
}
#[test]
fn remote_gunner_cancels_old_links_and_weapon_callbacks_without_restoring_an_old_life() {
    use crate::game::rewards::{inventory::Inventory, remote::Remotes};
    let c = collision();
    let resources = resources();
    let mut remote = Remotes::default();
    let mut inventory = Inventory::default();
    for (life, id, terminal) in [(3, 13, 312), (4, 12, 318)] {
        let now = life * 1000;
        inventory.grant(0, id, life);
        let ticket = inventory.reserve(0, id, life).unwrap();
        assert!(remote.begin(now, ticket, life, terminal, [0; 3]));
        let plan = remote.plan_gunner(&resources, id, 0, &c).unwrap();
        assert!(inventory.commit(ticket, 0, life, true));
        remote.launch_gunner(
            now + 1800,
            plan,
            Source {
                owner: 0,
                team: 2,
                life,
            },
            18,
            &c,
        );
        assert_eq!(remote.operator_life(), Some(life));
        assert_eq!(
            remote.links().unwrap().camera.unwrap().entity,
            if id == 12 { 624 } else { 640 }
        );
        assert!(remote.links().unwrap().weapon_view);
        assert!(remote.gunner_active());
        assert_eq!(
            remote.drain_weapon_changes()[0].weapon,
            if id == 13 { 289 } else { 304 }
        );
        remote.finish_operator();
        assert!(!remote.gunner_active());
        assert!(remote.drain_weapon_changes().is_empty());
        remote.advance(now + 1820, false, 0, &c);
        assert!(remote.links().is_none());
        let returns = remote.drain_returns();
        assert_eq!(returns.len(), 1);
        assert_eq!((returns[0].life, returns[0].weapon), (life, terminal));
    }
}
#[test]
fn thermal_toggle_changes_the_captured_vision_once_without_disabling_thermal_rendering() {
    use crate::game::rewards::{inventory::Inventory, remote::Remotes};
    let c = collision();
    let mut r = resources();
    r.remote_vision = Some(super::super::killstreaks::RemoteVisionResources {
        flight: "thermal_mp".into(),
        arena_absent: true,
        rawfiles_sha256: [
            "vision/black_bw.vision",
            "vision/thermal_mp.vision",
            "vision/missilecam.vision",
        ]
        .into_iter()
        .map(|n| (n.into(), "a".repeat(64)))
        .collect(),
    });
    let mut remote = Remotes::default();
    let mut stock = Inventory::default();
    stock.grant(0, 13, 7);
    let ticket = stock.reserve(0, 13, 7).unwrap();
    assert!(remote.begin(1000, ticket, 7, 312, [0; 3]));
    let plan = remote.plan_gunner(&r, 13, 0, &c).unwrap();
    assert!(stock.commit(ticket, 0, 7, true));
    remote.launch_gunner(
        2800,
        plan,
        Source {
            owner: 0,
            team: 2,
            life: 7,
        },
        18,
        &c,
    );
    assert_eq!(
        remote
            .drain_vision()
            .iter()
            .map(|v| v.command())
            .collect::<Vec<_>>(),
        [
            b"Q \"black_bw\" 0".to_vec(),
            b"Q \"thermal_mp\" 1000".to_vec()
        ]
    );
    let use_button = codol_protocol::game::channel::usercmd::BUTTON_USE;
    remote.input(&command(2820, 289, use_button), &c);
    remote.input(&command(2840, 289, use_button), &c);
    let changes = remote.drain_vision();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].command(), b"Q \"missilecam\" 620");
    assert!(remote.links().unwrap().thermal_vision);
    remote.input(&command(2860, 289, 0), &c);
    remote.input(&command(2880, 289, use_button), &c);
    assert_eq!(remote.drain_vision()[0].command(), b"Q \"thermal_mp\" 510");
    remote.finish_operator();
    assert!(remote.drain_vision().is_empty());
}

#[test]
fn helicopter_support_identity_does_not_change_when_ingress_starts_the_ride_clock() {
    let c = collision();
    let mut g = Gunners::default();
    let key = launch(&mut g, &c, 12);
    let source = g.targets()[0].source;
    g.advance(5000, false, 0, Some(key), &c);
    assert_eq!(g.targets()[0].source, source);
}
#[test]
fn cannon_switch_keeps_each_magazine_and_reload_clock_and_never_mutates_rifle() {
    let c = collision();
    let mut g = Gunners::default();
    let key = launch(&mut g, &c, 13);
    let rifle = crate::game::weapons::held::drop_test_gun(352, 352, 19, 77);
    g.input(&command(1020, 289, 0));
    g.input(&command(1040, 289, 1));
    g.advance(1060, false, 0, Some(key), &c);
    assert_eq!(equipment(&g).ammo.unwrap().clip, 0);
    assert_eq!(equipment(&g).ammo.unwrap().reserve, 1);
    assert_eq!(equipment(&g).firing.unwrap().state, 8);
    assert!(equipment(&g).firing.unwrap().remaining_msec > 4900);
    g.input(&command(1080, 291, 1));
    g.advance(1100, false, 0, Some(key), &c);
    assert_eq!(equipment(&g).ammo.unwrap().clip, 3);
    g.input(&command(1120, 290, 1));
    g.advance(1140, false, 0, Some(key), &c);
    let e = equipment(&g);
    assert_eq!(e.weapon, 290);
    assert_eq!(e.ammo.unwrap().clip, 19);
    let slots = e.inventory_override.unwrap();
    assert_eq!(
        slots[..3]
            .iter()
            .map(|s| s.unwrap().weapon)
            .collect::<Vec<_>>(),
        [289, 291, 290]
    );
    assert_eq!(
        slots[..3]
            .iter()
            .map(|s| s.unwrap().ammo.unwrap().clip)
            .collect::<Vec<_>>(),
        [0, 3, 19]
    );
    for (i, weapon) in [289, 291, 290].into_iter().enumerate() {
        g.input(&command(1142 + i as u32, weapon, 0));
        let e = equipment(&g);
        assert_eq!(e.weapon, weapon);
        let projected = e.inventory_override.unwrap();
        assert_eq!(
            projected.map(|s| s.map(|w| w.weapon)),
            slots.map(|s| s.map(|w| w.weapon))
        );
        codol_protocol::game::snapshot::player_frame_with_world(
            1160,
            1150,
            Some([0.0; 3]),
            true,
            Some(e),
            [0; 3],
            codol_protocol::game::snapshot::WorldSnapshot {
                motion: Some(Default::default()),
                ..Default::default()
            },
        )
        .unwrap();
    }
    g.input(&command(1160, 289, 0));
    g.advance(6000, false, 0, Some(key), &c);
    assert_eq!(equipment(&g).ammo.unwrap().clip, 0);
    g.advance(6060, false, 0, Some(key), &c);
    assert_eq!(equipment(&g).ammo.unwrap().clip, 1);
    assert_eq!(rifle.equipment().weapon, 352);
    let ammo = rifle.equipment().ammo.unwrap();
    assert_eq!((ammo.clip, ammo.reserve), (19, 77));
}
#[test]
fn detached_shell_hits_once_and_preserves_original_owner_and_life() {
    let c = collision();
    let mut g = Gunners::default();
    let key = launch(&mut g, &c, 13);
    g.input(&command(1020, 289, 0));
    g.input(&command(1040, 289, 1));
    g.advance(1060, false, 0, Some(key), &c);
    assert_eq!(g.shells().len(), 1);
    assert_eq!(g.advance(1100, false, 0, None, &c), Some(key));
    g.advance(8000, false, 0, None, &c);
    let hits = g.drain_hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].life, 7);
    assert_eq!(hits[0].blast.owner, 0);
    assert_eq!(hits[0].blast.reward_life, Some(7));
    assert!((hits[0].blast.origin[2] - 0.125).abs() < 0.001);
    assert!(g.props().is_empty());
    assert_eq!(g.drain().len(), 1);
    g.advance(10000, false, 0, None, &c);
    assert!(g.drain_hits().is_empty() && g.drain().is_empty());
}
#[test]
fn helicopter_mouse_aim_fire_cadence_damage_and_emp_cleanup_have_exact_ownership() {
    let c = collision();
    let mut g = Gunners::default();
    let key = launch(&mut g, &c, 12);
    g.input(&command(1020, 304, 0));
    let mut input = command(
        1040,
        304,
        codol_protocol::game::channel::usercmd::BUTTON_REMOTE | 1,
    );
    input.remote = [129, 129];
    let before = g.ride.as_ref().unwrap().aim;
    g.input(&input);
    g.advance(1060, false, 0, Some(key), &c);
    let shots = g.drain_shots();
    assert_eq!(shots.len(), 1);
    assert_eq!(
        (shots[0].owner, shots[0].life, shots[0].weapon),
        (0, 7, 304)
    );
    assert_ne!(before, g.ride.as_ref().unwrap().aim);
    assert!(g.damage(key.entity, 10000, 2));
    assert!(g.advance(1080, false, 0, Some(key), &c).is_none());
    assert_eq!(g.advance(1100, false, 1 << 2, Some(key), &c), Some(key));
    assert!(g.props().is_empty());
    assert!(g.drain().is_empty());
    let key = launch(&mut g, &c, 12);
    g.damage(key.entity, 10000, 1);
    assert_eq!(g.advance(1020, false, 0, Some(key), &c), Some(key));
    assert_eq!(g.drain().len(), 1);
}
#[test]
#[ignore = "requires captured target gunner camera mounts and Afghan world collision"]
fn captured_gunner_paths_and_ammunition_use_this_map() {
    let r = RadarResources::load(
        std::path::Path::new(&std::env::var("CODOL_REWARD_RESOURCES").unwrap()),
        "mp_afghan",
        523897474,
    )
    .unwrap();
    let c = Collision::load(std::path::Path::new(
        &std::env::var("CODOL_COLLISION_FILE").unwrap(),
    ))
    .unwrap();
    for id in [12, 13] {
        let mut g = Gunners::default();
        assert!(
            Gunners::resources_present(&r, id),
            "missing gunner resources for {id}"
        );
        let plan = g
            .plan(&r, id, 0, &c)
            .expect("captured gunner flight clearance");
        let key = g
            .launch(
                plan,
                1000,
                1,
                Source {
                    owner: 0,
                    team: 2,
                    life: 7,
                },
                &c,
            )
            .0;
        assert!(g.advance(1100, false, 0, Some(key), &c).is_none());
        assert!(g.props().iter().all(|p| valid(p.origin)));
        if id == 13 {
            assert_eq!(equipment(&g).ammo.unwrap().clip, 1);
            assert_eq!(g.ride.as_ref().unwrap().plan.guns[2].magazine, 20);
        }
    }
}

#[test]
#[ignore = "requires captured spawn candidates and Afghan collision for native acceptance placement"]
fn captured_open_spawns_are_visible_from_the_gunner_orbit() {
    let r = RadarResources::load(
        std::path::Path::new(&std::env::var("CODOL_REWARD_RESOURCES").unwrap()),
        "mp_afghan",
        523897474,
    )
    .unwrap();
    let c = Collision::load(std::path::Path::new(
        &std::env::var("CODOL_COLLISION_FILE").unwrap(),
    ))
    .unwrap();
    let candidates: Vec<V3> = serde_json::from_slice(
        &std::fs::read(std::env::var("CODOL_NATIVE_CANDIDATES").unwrap()).unwrap(),
    )
    .unwrap();
    let mut g = Gunners::default();
    let plan = g.plan(&r, 13, 0, &c).unwrap();
    g.launch(
        plan,
        1000,
        1,
        Source {
            owner: 0,
            team: 1,
            life: 1,
        },
        &c,
    );
    let mut cameras = Vec::new();
    for now in (1000..41000).step_by(250) {
        let ride = g.ride.as_mut().unwrap();
        ride.pose(now);
        cameras.push(ride.camera);
    }
    let mut scores = Vec::new();
    for p in candidates {
        let Ok(ground) = crate::game::ai::controller::grounded_spawn(p, &c) else {
            continue;
        };
        let target = add(ground, [0.0, 0.0, 2.0]);
        let clear = cameras
            .iter()
            .filter(|&&camera| {
                let t = c.raycast_excluding_prop(camera, target, Some(624));
                !t.start_solid && !t.all_solid && t.fraction >= 1.0
            })
            .count();
        scores.push(
            serde_json::json!({"spawn":p,"ground":ground,"clear":clear,"samples":cameras.len()}),
        );
    }
    scores.sort_by_key(|x| std::cmp::Reverse(x["clear"].as_u64().unwrap()));
    assert!(
        scores
            .first()
            .is_some_and(|s| s["clear"].as_u64().unwrap() >= 80)
    );
    std::fs::write(
        std::env::var("CODOL_NATIVE_SPAWN_REPORT").unwrap(),
        serde_json::to_vec_pretty(&scores).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires native cannon resources and Afghan terrain"]
fn captured_cannon_impact_damages_the_spawn_it_is_aimed_at() {
    let r = RadarResources::load(
        std::path::Path::new(&std::env::var("CODOL_REWARD_RESOURCES").unwrap()),
        "mp_afghan",
        523897474,
    )
    .unwrap();
    let c = Collision::load(std::path::Path::new(
        &std::env::var("CODOL_COLLISION_FILE").unwrap(),
    ))
    .unwrap();
    let p = crate::game::ai::controller::grounded_spawn([-440.0, -848.0, -112.6], &c).unwrap();
    let mut g = Gunners::default();
    let plan = g.plan(&r, 13, 0, &c).unwrap();
    let key = g
        .launch(
            plan,
            1000,
            1,
            Source {
                owner: 0,
                team: 1,
                life: 7,
            },
            &c,
        )
        .0;
    let ride = g.ride.as_mut().unwrap();
    ride.aim = direction_angles(sub(add(p, [0.0, 0.0, 2.0]), ride.camera));
    g.input(&command(1020, 289, 0));
    g.input(&command(1040, 289, 1));
    g.advance(1060, false, 0, Some(key), &c);
    g.advance(8000, false, 0, Some(key), &c);
    let hits = g.drain_hits();
    assert!(!hits.is_empty());
    let hit = hits[0];
    let damage = crate::game::rewards::strikes::damage(hit, p, 35.0, &c);
    let path = c.raycast(hit.blast.origin, add(p, [0.0, 0.0, 35.0]));
    assert!(
        damage >= 100,
        "spawn={p:?} impact={:?} radius={} damage={damage} trace={path:?}",
        hit.blast.origin,
        hit.blast.radius
    );
}
