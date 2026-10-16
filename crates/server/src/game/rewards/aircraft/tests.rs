use super::*;
#[test]
fn remote_vehicle_slots_are_shared_without_reducing_aircraft_capacity() {
    let c = collision();
    let r = resources();
    let mut a = Aircraft::default();
    a.reserve_slots(u16::MAX);
    assert!(a.plan(&r, 8, &c).is_none());
    a.reserve_slots(3);
    let plan = a.plan(&r, 8, &c).unwrap();
    assert_eq!(plan.entity, FIRST_ENTITY + 2);
    a.launch(1000, plan, source(1000), &c);
    assert_eq!(a.vehicle_slots(), 4);
    a.reserve_slots(0);
}
fn collision() -> Collision {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[20000,20000,10],"contents":1,"planes":[]}],"triangles":[]})).unwrap()).unwrap();
    Collision::load(f.path()).unwrap()
}
fn resources() -> RadarResources {
    let mut r = crate::game::rewards::deployables::test_resources();
    r.models.insert("weapon_minigun".into(), 153);
    r.vehicles.extend([
        ("cobra_mp".into(), 3),
        ("pavelow_mp".into(), 6),
        ("harrier_mp".into(), 4),
    ]);
    r.animations.insert(
        "bh_rotors".into(),
        super::super::killstreaks::AnimationBinding {
            tree: 3,
            node: 11,
            parent: 10,
        },
    );
    let w = r.weapons["sentry_minigun_mp"].clone();
    for (name, id) in [("cobra_20mm_mp", 302), ("pavelow_minigun_mp", 335)] {
        let mut w = w.clone();
        w.weapon = id;
        r.weapons.insert(name.into(), w);
    }
    for (name, model) in [("vehicle_mi24p_hind_mp", 146), ("vehicle_pavelow", 148)] {
        r.models.insert(name.into(), model);
        r.model_bounds.insert(
            name.into(),
            ModelBounds {
                min: [-60.0, -60.0, -30.0],
                max: [60.0, 60.0, 30.0],
            },
        );
    }
    for (name, fx) in [
        ("explosions/aerial_explosion", 5),
        ("smoke/smoke_trail_white_heli_emitter", 47),
        ("smoke/smoke_trail_black_heli_emitter", 45),
        ("misc/flares_cobra", 93),
    ] {
        r.effects.insert(name.into(), fx);
    }
    r.air_paths=Some(serde_json::from_value(serde_json::json!({"sources_sha256":["0".repeat(64)],"entries":{"heli_start":[0],"heli_loop_start":[2],"heli_leave":[4]},"nodes":[
 {"name":"heli_start","origin":[0,0,1000],"target":1},
 {"name":"dest","origin":[500,0,1000],"target":null},
 {"name":"heli_loop_start","origin":[500,500,1000],"target":3},
 {"name":"loop","origin":[1000,500,1000],"target":2},
 {"name":"heli_leave","origin":[8000,500,1000],"target":null}]})).unwrap());
    r
}
fn source(serial: u32) -> Source {
    Source {
        owner: 0,
        team: 2,
        life: 3,
        serial,
    }
}
#[test]
fn aircraft_requires_native_vehicle_and_rotor_bindings_before_consuming_inventory() {
    let c = collision();
    let mut r = resources();
    let a = Aircraft::default();
    assert!(a.plan(&r, 8, &c).is_some());
    r.vehicles.remove("cobra_mp");
    assert!(a.plan(&r, 8, &c).is_none());
    r.vehicles.insert("cobra_mp".into(), 3);
    r.animations.remove("bh_rotors");
    assert!(a.plan(&r, 8, &c).is_none());
    assert!(a.plan(&r, 10, &c).is_none());
}
#[test]
fn chopper_slot_stays_busy_until_departure_or_crash_removal() {
    let c = collision();
    let r = resources();
    let mut a = Aircraft::default();
    let p = a.plan(&r, 8, &c).unwrap();
    let e = p.entity;
    a.launch(1000, p, source(1000), &c);
    assert!(a.plan(&r, 10, &c).is_none());
    a.damage(1100, e, 1500, (1, 1), true, &c);
    assert!(a.plan(&r, 8, &c).is_none());
    a.advance(15000, false, &c);
    assert!(a.plan(&r, 10, &c).is_some());
}
#[test]
fn destroyed_aircraft_follows_authored_crash_descent_and_collides_before_buried_endpoint() {
    let c = collision();
    let mut r = resources();
    let g = r.air_paths.as_mut().unwrap();
    for node in [
        serde_json::json!({"name":"heli_crash_start","origin":[0,500,1000],"target":6}),
        serde_json::json!({"name":"crash_descent","origin":[0,1500,-100],"target":null}),
    ] {
        g.nodes.push(serde_json::from_value(node).unwrap());
    }
    g.entries.insert("heli_crash_start".into(), vec![5]);
    let mut a = Aircraft::default();
    let plan = a.plan(&r, 8, &c).unwrap();
    let entity = plan.entity;
    a.launch(1000, plan, source(1000), &c);
    a.damage(1000, entity, 1500, (1, 1), true, &c);
    a.advance(1500, false, &c);
    assert!(a.flights[0].crash_route);
    assert!(a.flights[0].motion.origin[1] > 0.0);
    assert!(a.targets().is_empty());
    let mut events = Vec::new();
    for now in (1550..20000).step_by(50) {
        a.advance(now, false, &c);
        events.extend(a.drain());
    }
    assert!(a.snapshots().is_empty());
    assert_eq!(events.len(), 1);
    let Event::Fx { origin, .. } = events[0] else {
        panic!("expected final crash effect")
    };
    assert!(origin[1] > 500.0);
    assert!(
        origin[2] >= 29.9,
        "full body must stop above the solid floor"
    );
}
#[test]
fn gun_armor_enemy_filter_crash_and_match_end_share_one_lifetime() {
    let r = resources();
    let c = collision();
    let mut a = Aircraft::default();
    let plan = a.plan(&r, 8, &c).unwrap();
    let entity = plan.entity;
    a.launch(1000, plan, source(1000), &c);
    assert!(!a.damage(1050, entity, 1000, (0, 2), true, &c));
    assert_eq!(a.flights[0].health, 1500);
    assert!(a.damage(1100, entity, 100, (1, 1), false, &c));
    assert_eq!(a.flights[0].health, 1470);
    assert!(a.damage(1200, entity, 1500, (1, 1), true, &c));
    assert!(a.flights[0].crash.is_some());
    assert!(a.targets().is_empty());
    assert!(a.shots(1250, &[], 0, &c).is_empty());
    for now in (1250..15000).step_by(50) {
        a.advance(now, false, &c);
    }
    assert!(a.snapshots().is_empty());
    assert_eq!(a.drain().len(), 1);
    assert!(a.drain().is_empty());
    let plan = a.plan(&r, 8, &c).unwrap();
    a.launch(20000, plan, source(20000), &c);
    a.advance(20001, true, &c);
    assert!(a.snapshots().is_empty());
    assert!(a.targets().is_empty());
}
#[test]
fn flare_is_single_use_and_old_entity_identity_cannot_spend_new_stock() {
    let r = resources();
    let c = collision();
    let mut a = Aircraft::default();
    let plan = a.plan(&r, 10, &c).unwrap();
    a.launch(1000, plan, source(1000), &c);
    let old = a.targets()[0];
    assert!(a.decoy(old));
    assert!(!a.decoy(old));
    assert_eq!(a.drain().len(), 1);
    a.advance(2000, true, &c);
    let plan = a.plan(&r, 10, &c).unwrap();
    a.launch(3000, plan, source(3000), &c);
    assert_eq!(a.targets()[0].entity, old.entity);
    assert!(!a.decoy(old));
    assert!(a.decoy(a.targets()[0]));
}
#[test]
fn emp_only_crashes_enemy_aircraft_and_never_enables_unverified_actions() {
    let r = resources();
    let c = collision();
    let mut a = Aircraft::default();
    let plan = a.plan(&r, 10, &c).unwrap();
    a.launch(1000, plan, source(1000), &c);
    a.emp(1100, 2);
    assert!(a.flights[0].crash.is_none());
    a.emp(1200, 1);
    assert!(a.flights[0].crash.is_some());
    assert!(a.targets().is_empty());
    assert!(!crate::game::rewards::mechanisms::Mechanism::radio_available(8, &r));
    assert!(!crate::game::rewards::mechanisms::Mechanism::radio_available(10, &r));
}

#[test]
fn harrier_waits_for_bombing_then_hovers_with_six_owned_air_to_air_missiles() {
    let mut r = resources();
    let c = collision();
    r.models.insert("vehicle_av8b_harrier_jet_mp".into(), 139);
    r.model_bounds.insert(
        "vehicle_av8b_harrier_jet_mp".into(),
        r.model_bounds["vehicle_mi24p_hind_mp"],
    );
    r.effects
        .insert("explosions/aerial_explosion_harrier".into(), 62);
    let gun = r.weapons["cobra_20mm_mp"].clone();
    r.weapons.insert("harrier_20mm_mp".into(), gun.clone());
    let mut rocket = gun;
    rocket.weapon = 309;
    rocket.weapon_type = 2;
    rocket.radius = 512;
    rocket.inner_damage = 1200;
    rocket.outer_damage = 300;
    rocket.projectile_speed = 6000;
    r.weapons.insert("harrier_missile_mp".into(), rocket);
    r.air_route = Some(crate::game::rewards::killstreaks::AirRoute {
        center: [0.0, 0.0, 1000.0],
        sources_sha256: ["0".repeat(64)],
    });
    let mut a = Aircraft::default();
    let p = crate::game::rewards::selection::Point {
        origin: [0.0; 3],
        yaw: 0.0,
        coverage: Some(1100.0),
    };
    let plan = a.plan_harrier(&r, p, &c).unwrap();
    a.launch(1000, plan, source(1000), &c);
    assert!(a.snapshots().is_empty());
    a.advance(4999, false, &c);
    assert!(a.targets().is_empty());
    a.advance(6000, false, &c);
    assert_eq!(a.snapshots().len(), 1);
    for n in 0..6 {
        let origin = a.flights[0].motion.origin;
        let enemy = crate::game::rewards::airspace::Target {
            entity: 448,
            source: crate::game::rewards::killstreaks::SupportFlight {
                actor: 1,
                id: 8,
                team: 1,
                start: 500,
                end: 100000,
            },
            center: [origin[0] + 2000.0, origin[1], origin[2]],
        };
        let launches = a.launches(6000 + n * 5000, &[enemy], 0, 1, &c);
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].reward_life, Some(3));
        assert!(a.launches(6001 + n * 5000, &[enemy], 0, 1, &c).is_empty());
    }
    assert_eq!(a.flights[0].rockets_remaining, 0);
    for now in (6050..150000).step_by(50) {
        a.advance(now, false, &c);
    }
    assert!(a.snapshots().is_empty());
}

#[test]
fn heavy_has_two_bound_turrets_and_cleans_them_with_parent() {
    let c = collision();
    let mut r = resources();
    let mut a = Aircraft::default();
    r.models.remove("weapon_minigun");
    assert!(a.plan(&r, 10, &c).is_none());
    r.models.insert("weapon_minigun".into(), 153);
    let plan = a.plan(&r, 10, &c).unwrap();
    a.launch(1000, plan, source(1), &c);
    let states = a.snapshots();
    assert_eq!(
        states.iter().map(|p| p.entity_number).collect::<Vec<_>>(),
        vec![560, 576, 577]
    );
    assert!(states[0].vehicle.is_some());
    for gun in &states[1..] {
        assert_eq!(gun.model, 153);
        assert_eq!(gun.weapon, 335);
        assert!(gun.vehicle.is_none());
        assert!(!gun.turret_pose.unwrap().firing);
    }
    assert!(states[1].origin[1] > states[2].origin[1]);
    a.advance(2000, true, &c);
    assert!(a.snapshots().is_empty());
}

#[test]
fn cobra_loop_follows_shots_stops_after_silence_and_does_not_survive_crash() {
    let c = collision();
    let mut r = resources();
    r.loop_sounds.insert("weap_cobra_20mm_fire_npc".into(), 240);
    let mut a = Aircraft::default();
    let plan = a.plan(&r, 8, &c).unwrap();
    a.launch(1000, plan, source(1), &c);
    a.advance(7000, false, &c);
    let origin = a.flights[0].motion.origin;
    let target = TargetActor {
        actor: crate::game::combat::Actor {
            client: 1,
            team: 1,
            origin: [origin[0] + 600.0, origin[1], 0.0],
            height: 80.0,
            yaw: 0.0,
        },
        life: 1,
        born: 0,
        score: 0,
        hidden: false,
        airplane_hidden: false,
    };
    assert!(a.shots(7000, &[target], 0, &c).is_empty());
    assert_eq!(a.snapshots()[0].vehicle.unwrap().loop_sound, 0);
    a.advance(7500, false, &c);
    assert!(!a.shots(7500, &[target], 0, &c).is_empty());
    assert_eq!(a.snapshots()[0].vehicle.unwrap().loop_sound, 240);
    a.advance(7550, false, &c);
    assert!(a.shots(7550, &[], 0, &c).is_empty());
    assert_eq!(a.snapshots()[0].vehicle.unwrap().loop_sound, 0);
    a.advance(8000, false, &c);
    a.shots(8000, &[target], 0, &c);
    a.advance(8500, false, &c);
    assert!(!a.shots(8500, &[target], 0, &c).is_empty());
    assert_eq!(a.snapshots()[0].vehicle.unwrap().loop_sound, 240);
    let entity = a.snapshots()[0].entity_number;
    assert!(a.damage(8500, entity, u16::MAX, (1, 1), true, &c));
    assert_eq!(a.snapshots()[0].vehicle.unwrap().loop_sound, 0);
}
