use super::*;
fn resources() -> RadarResources {
    let mut r = super::super::gunner_test_resources();
    r.models.insert("german_sheperd_dog".into(), 105);
    r.model_bounds.insert(
        "german_sheperd_dog".into(),
        ModelBounds {
            min: [-20.0, -10.0, -2.0],
            max: [20.0, 10.0, 38.0],
        },
    );
    let mut w = r.weapons["cobra_player_minigun_mp"].clone();
    w.weapon = 305;
    w.damage_stages[0] = 1000;
    w.fire_delay_msec = 100;
    r.weapons.insert("dog_bite".into(), w);
    r.animals = Some(Resources {
        mask: 2,
        selector: 1,
        clips: [
            ("idle", 29, 0),
            ("run", 39, 26),
            ("bite", 49, 32),
            ("death", 30, 0),
        ]
        .into_iter()
        .map(|(k, node, parent)| {
            (
                k.into(),
                AnimationBinding {
                    tree: 10,
                    node,
                    parent,
                },
            )
        })
        .collect(),
        ancestors: [
            ("idle", vec![]),
            ("run", vec![26]),
            ("bite", vec![31, 32]),
            ("death", vec![]),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect(),
        durations: [
            ("idle", 3100),
            ("run", 400),
            ("bite", 1500),
            ("death", 1167),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v))
        .collect(),
        rawfiles_sha256: ["animlists/common_dog.animlist", "animtrees/dog.atr"]
            .into_iter()
            .map(|k| (k.into(), "a".repeat(64)))
            .collect(),
    });
    r
}
fn world(wall: bool) -> Collision {
    let mut brushes = vec![
        serde_json::json!({"center":[0,0,-10],"half":[2000,2000,10],"contents":1,"planes":[]}),
    ];
    if wall {
        for y in [-110, 110] {
            brushes.push(
                serde_json::json!({"center":[0,y,40],"half":[10,100,40],"contents":1,"planes":[]}),
            );
        }
    }
    let p = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(p.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,
        "client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]})).unwrap()).unwrap();
    Collision::load(p.path()).unwrap()
}
fn nav(c: &Collision) -> Navigation {
    Navigation::build([[-300.0, 0.0, 0.125], [300.0, 0.0, 0.125]].into_iter(), c)
}
fn launch(d: &mut Dogs, c: &Collision) {
    let p = d.plan(&resources(), [-300.0, 0.0, 0.125], 2, c).unwrap();
    d.launch(
        1000,
        p,
        Source {
            owner: 0,
            team: 2,
            life: 7,
        },
    );
}
fn target(origin: V3, life: u32, born: u32) -> Target {
    Target {
        actor: Actor {
            client: 6,
            team: 1,
            origin,
            height: 70.0,
            yaw: 0.0,
        },
        life,
        born,
    }
}

#[test]
fn pack_uses_captured_clips_limits_live_dogs_and_preserves_stock_when_busy_or_missing() {
    let c = world(false);
    let n = nav(&c);
    let mut dogs = Dogs::default();
    let mut r = resources();
    r.animals
        .as_mut()
        .unwrap()
        .ancestors
        .get_mut("bite")
        .unwrap()
        .push(49);
    assert!(!Dogs::resources_present(&r));
    assert!(dogs.plan(&r, [0.0; 3], 2, &c).is_none());
    launch(&mut dogs, &c);
    assert!(dogs.plan(&resources(), [0.0; 3], 1, &c).is_none());
    for now in (1000..=4000).step_by(100) {
        assert!(dogs.advance(now, false, &[], &n, &c).is_empty());
    }
    assert_eq!(dogs.dogs.iter().filter(|d| d.health > 0).count(), 4);
    let entity = dogs.dogs[0].entity;
    dogs.damage(4100, entity, 100, 2, &c);
    assert_eq!(dogs.dogs[0].health, 100);
    dogs.damage(4200, entity, 100, 1, &c);
    assert_eq!(dogs.dogs[0].health, 0);
    assert_eq!(dogs.snapshots()[0].actor.unwrap().animation, 30);
    dogs.advance(4300, false, &[], &n, &c);
    assert_eq!(dogs.dogs.iter().filter(|d| d.health > 0).count(), 4);
    assert_eq!(dogs.packs[0].remaining, 3);
}

#[test]
fn bite_waits_for_the_captured_delay_and_never_hits_a_new_target_life() {
    let c = world(false);
    let n = nav(&c);
    let mut dogs = Dogs::default();
    launch(&mut dogs, &c);
    dogs.advance(1000, false, &[], &n, &c);
    let origin = add(dogs.dogs[0].feet, [40.0, 0.0, 0.0]);
    assert!(
        dogs.advance(2999, false, &[target(origin, 10, 1000)], &n, &c)
            .is_empty()
    );
    assert!(dogs.dogs[0].pending.is_none());
    assert!(
        dogs.advance(3000, false, &[target(origin, 10, 1000)], &n, &c)
            .is_empty()
    );
    assert_eq!(dogs.dogs[0].pending, Some((6, 10, 3100)));
    assert!(
        dogs.advance(3099, false, &[target(origin, 10, 1000)], &n, &c)
            .is_empty()
    );
    assert!(
        dogs.advance(3100, false, &[target(origin, 11, 1000)], &n, &c)
            .is_empty()
    );
    dogs.advance(4500, false, &[target(origin, 11, 1000)], &n, &c);
    let bites = dogs.advance(4600, false, &[target(origin, 11, 1000)], &n, &c);
    assert!(!bites.is_empty());
    assert!(bites.iter().all(
        |b| (b.owner, b.life, b.weapon, b.target, b.target_life, b.damage)
            == (0, 7, 305, 6, 11, 1000)
    ));
    assert!(
        dogs.advance(4600, false, &[target(origin, 11, 1000)], &n, &c)
            .is_empty()
    );
}

#[test]
fn chase_moves_a_complete_grounded_dog_and_rejects_a_gap_that_only_its_center_fits() {
    let c = world(false);
    let n = nav(&c);
    let mut dogs = Dogs::default();
    launch(&mut dogs, &c);
    dogs.advance(1000, false, &[], &n, &c);
    let start = dogs.dogs[0].feet;
    assert!(
        walk(
            start,
            add(start, [5.0, 0.0, 0.0]),
            &dogs.dogs[0].definition,
            Some(dogs.dogs[0].entity),
            &c
        )
        .is_some(),
        "first grounded step from {start:?}"
    );
    for now in (3000..=5000).step_by(20) {
        dogs.advance(now, false, &[target([600.0, 0.0, 0.125], 1, 0)], &n, &c);
    }
    assert!(
        length(sub(dogs.dogs[0].feet, start)) > 100.0,
        "start={start:?} end={:?} path={:?} target={:?}",
        dogs.dogs[0].feet,
        dogs.dogs[0].path,
        dogs.dogs[0].target
    );
    assert!((dogs.dogs[0].feet[2] - 0.125).abs() < 0.01);
    let c = world(true);
    let d = Definition::load(&resources()).unwrap();
    assert_eq!(
        c.raycast([-40.0, 0.0, 20.0], [40.0, 0.0, 20.0]).fraction,
        1.0
    );
    assert!(walk([-40.0, 0.0, 0.125], [40.0, 0.0, 0.125], &d, None, &c).is_none());
}

#[test]
fn death_expiry_and_match_end_clear_pending_bites_and_animal_collision() {
    for finished in [false, true] {
        let c = world(false);
        let n = nav(&c);
        let mut dogs = Dogs::default();
        launch(&mut dogs, &c);
        dogs.advance(1000, false, &[], &n, &c);
        let origin = add(dogs.dogs[0].feet, [40.0, 0.0, 0.0]);
        dogs.advance(3000, false, &[target(origin, 1, 0)], &n, &c);
        assert!(
            dogs.advance(
                if finished { 3050 } else { 61000 },
                finished,
                &[target(origin, 1, 0)],
                &n,
                &c
            )
            .is_empty()
        );
        assert!(dogs.snapshots().is_empty());
        assert!(dogs.advance(62000, false, &[], &n, &c).is_empty());
    }
}

#[test]
fn killing_a_dog_during_windup_cancels_its_bite_and_live_wounds_recover_after_five_seconds() {
    let c = world(false);
    let n = nav(&c);
    let mut dogs = Dogs::default();
    launch(&mut dogs, &c);
    dogs.advance(1000, false, &[], &n, &c);
    let origin = add(dogs.dogs[0].feet, [40.0, 0.0, 0.0]);
    dogs.advance(3000, false, &[target(origin, 1, 0)], &n, &c);
    let entity = dogs.dogs[0].entity;
    assert!(dogs.dogs[0].pending.is_some());
    dogs.damage(3050, entity, 100, 1, &c);
    assert!(
        dogs.advance(3100, false, &[target(origin, 1, 0)], &n, &c)
            .is_empty()
    );
    assert!(dogs.dogs[0].pending.is_none());
    let alive = dogs.dogs.iter().find(|d| d.health > 0).unwrap().entity;
    dogs.damage(4000, alive, 49, 1, &c);
    dogs.advance(8999, false, &[], &n, &c);
    assert_eq!(
        dogs.dogs.iter().find(|d| d.entity == alive).unwrap().health,
        51
    );
    dogs.advance(9000, false, &[], &n, &c);
    assert_eq!(
        dogs.dogs.iter().find(|d| d.entity == alive).unwrap().health,
        100
    );
}

#[test]
#[ignore = "requires the captured native dog tree and Afghan collision"]
fn captured_dog_and_armor_bindings_plan_on_the_real_map() {
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
    let dogs = Dogs::default();
    assert!(Dogs::resources_present(&r));
    let plan = dogs.plan(&r, [-472.0, -800.0, -141.03853], 1, &c).unwrap();
    assert!(plan.spawns.len() >= 4);
    assert_eq!(r.animals.as_ref().unwrap().clips["bite"].node, 49);
    let base = crate::game::weapons::held::drop_test_gun(352, 352, 19, 77).equipment();
    for team in [1, 2] {
        assert!(crate::game::rewards::armor::Plan::new(&r, 23, team, base).is_some());
        assert!(crate::game::rewards::armor::Plan::new(&r, 24, team, base).is_none());
    }
    assert!(r.weapons["minigungunship_mp"].clip_key.is_none());
}
