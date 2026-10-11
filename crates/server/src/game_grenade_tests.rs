use super::*;

#[test]
fn captured_profile_frag_count_and_ownership_are_independent_of_gun_ammo() {
    let mut snapshot = vec![0; 32768];
    snapshot[4418..4422].copy_from_slice(&60001u32.to_le_bytes());
    let grenades = resolve(&snapshot, 0, &[60000]).unwrap();
    let grenade = grenades[0].as_ref().unwrap();
    assert_eq!(
        (
            grenade.definition.index,
            grenade.definition.clip_key,
            grenade.count
        ),
        (95, 95, 1)
    );
    assert_eq!(
        (
            grenade.definition.throw_delay_msec,
            grenade.definition.fuse_msec
        ),
        (129, 3500)
    );
    let [offset, bit] = CATALOG.double_ammo_offsets[0][0];
    snapshot[offset] |= 1 << bit;
    assert_eq!(
        resolve(&snapshot, 0, &[60000]).unwrap()[0]
            .as_ref()
            .unwrap()
            .count,
        2
    );
    assert!(resolve(&snapshot, 0, &[]).is_err());
    snapshot[4418..4422].copy_from_slice(&1u32.to_le_bytes());
    assert!(resolve(&snapshot, 0, &[]).unwrap()[0].is_none());
    for definition in &CATALOG.grenades {
        definition.validate().unwrap();
    }
}
fn world(wall: bool) -> Collision {
    let mut brushes = vec![
        serde_json::json!({"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}),
    ];
    if wall {
        brushes.push(
            serde_json::json!({"center":[64,0,100],"half":[2,1000,100],"contents":1,"planes":[]}),
        );
    }
    let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
        "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]});
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
    Collision::load(file.path()).unwrap()
}
#[test]
fn saved_tactical_smoke_uses_its_own_count_and_fuse_without_explosive_damage() {
    let mut snapshot = vec![0; 32768];
    snapshot[4418..4422].copy_from_slice(&60001u32.to_le_bytes());
    let layout: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/passive-skills-3.3.3.3.json"
    ))
    .unwrap();
    let tactical = layout["class_offsets"][0][7].as_u64().unwrap() as usize;
    snapshot[tactical..tactical + 4].copy_from_slice(&60006u32.to_le_bytes());
    let equipped = resolve(&snapshot, 0, &[60000, 60005]).unwrap();
    assert_eq!(equipped[0].as_ref().unwrap().definition.index, 95);
    let smoke = equipped[1].as_ref().unwrap();
    assert_eq!(
        (
            smoke.definition.index,
            smoke.count,
            smoke.definition.fuse_msec
        ),
        (236, 1, 1000)
    );
    assert!(!smoke.definition.cook);
    let [offset, bit] = CATALOG.double_ammo_offsets[0][1];
    snapshot[offset] |= 1 << bit;
    let equipped = resolve(&snapshot, 0, &[60000, 60005]).unwrap();
    assert_eq!(equipped[1].as_ref().unwrap().count, 2);
    assert_eq!(equipped[0].as_ref().unwrap().count, 1);
    assert!(resolve(&snapshot, 0, &[60000]).is_err());
    let collision = world(false);
    let mut flight = Grenades::default();
    assert!(flight.launch(
        Throw {
            time: 1000,
            angles: [0; 3],
            definition: smoke.definition.clone(),
            fuse_msec: 1000,
            in_hand: false,
            dropped: false
        },
        [0.0, 0.0, 60.0],
        [0.0; 3],
        &collision
    ));
    assert!(flight.advance(1999, &collision).is_empty());
    let explosion = flight.advance(2000, &collision).pop().unwrap();
    assert_eq!(explosion.time, 2000);
    assert_eq!(
        radius_damage(&explosion, explosion.origin, 60.0, &collision),
        0
    );
    assert!(flight.snapshots().is_empty());
    assert!(flight.advance(3000, &collision).is_empty());
}
#[test]
fn sphere_hits_walls_gravity_reaches_floor_and_explosion_fires_once() {
    let collision = world(true);
    let mut flight = Grenades::default();
    assert!(flight.launch(
        Throw {
            time: 1000,
            angles: [0; 3],
            definition: captured("frag_grenade_mp"),
            fuse_msec: 3500,
            in_hand: false,
            dropped: false
        },
        [0.0, 0.0, 60.0],
        [0.0; 3],
        &collision
    ));
    assert!(flight.advance(1400, &collision).is_empty());
    let state = flight.snapshots()[0];
    assert!(state.origin[0] < 62.0);
    assert!(state.origin[2] >= 1.9);
    let explosions = flight.advance(4500, &collision);
    assert_eq!(explosions.len(), 1);
    assert!(explosions[0].origin[2] >= 1.9);
    assert!(flight.snapshots().is_empty());
    assert!(flight.advance(4600, &collision).is_empty());
    let mut blast = explosions[0].clone();
    blast.origin = [0.0, 0.0, 40.0];
    assert_eq!(
        radius_damage(&blast, [128.0, 0.0, 0.0], 80.0, &collision),
        0
    );
    assert_eq!(
        radius_damage(&blast, [0.0, 0.0, 0.0], 80.0, &collision),
        150
    );
}
