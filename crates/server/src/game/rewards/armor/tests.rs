use super::*;
use codol_protocol::game::channel::usercmd::UserCommand;

fn resources() -> RadarResources {
    let mut r = crate::game::rewards::gunner_test_resources();
    r.models.extend([
        ("mp_fullbody_juggernaut_explosive_so".into(), 109),
        ("viewhands_juggernaut_ally".into(), 150),
        ("c_kp_juggernaut_tf_00".into(), 99),
        ("c_kp_juggernaut_sc_00".into(), 98),
        ("viewhands_player_kp_juggernaut_tf".into(), 152),
        ("viewhands_player_kp_juggernaut_sc".into(), 151),
    ]);
    for (name, id, kind, clip) in [
        ("mg4juggernaut_mp", 187, 0, 100),
        ("minigungunship_mp", 188, 6, 300),
    ] {
        let mut w = r.weapons["cobra_player_minigun_mp"].clone();
        w.weapon = id;
        w.clip_key = Some(id);
        w.ammo_key = Some(id);
        w.clip_size = clip;
        w.fire_interval_msec = if kind == 0 { 96 } else { 70 };
        w.operator = Some(OperatorFacts {
            fire_type: kind,
            move_scale: 1.0,
            ads_move_scale: 1.0,
            reload: ReloadTiming {
                normal_msec: 8000,
                empty_msec: 8000,
                add_msec: 5539,
                empty_add_msec: 5539,
                segmented: None,
            },
            ads: None,
            location_multipliers: [1.0; 20],
        });
        r.weapons.insert(name.into(), w);
    }
    r
}
fn rifle() -> HeldWeapons {
    crate::game::weapons::held::drop_test_gun(352, 352, 19, 77)
}
fn source() -> Source {
    Source {
        owner: 0,
        team: 1,
        life: 3,
    }
}

#[test]
fn variants_have_distinct_registered_models_hands_guns_and_rotary_cadence() {
    let r = resources();
    for (id, team, model, hands, weapon, clip) in [
        (23, 1, 109, 150, 187, 100),
        (24, 1, 99, 152, 188, 300),
        (24, 2, 98, 151, 188, 300),
    ] {
        let mut suit = Armor::default();
        let mut player = rifle();
        let mut v = PlayerVitals::default();
        let p = Plan::new(&r, id, team, player.equipment()).unwrap();
        assert_eq!(p.model, model);
        assert!(suit.start(p, source(), 1000, &mut player, &mut v));
        assert_eq!((v.health, v.max_health), (1000, 1000));
        assert_eq!(player.equipment().weapon, weapon);
        assert_eq!(player.equipment().viewmodel, Some(hands));
        assert!((player.move_scale() - 0.65).abs() < 0.0001);
        assert!((player.equipment().move_multiplier - 0.65).abs() < 0.0001);
        player.apply(&UserCommand {
            time: 2000,
            weapons: [weapon, 0, 0],
            ..Default::default()
        });
        player.apply(&UserCommand {
            time: 2200,
            buttons: 1,
            weapons: [weapon, 0, 0],
            ..Default::default()
        });
        player.apply(&UserCommand {
            time: 2400,
            buttons: 1,
            weapons: [weapon, 0, 0],
            ..Default::default()
        });
        assert!(player.equipment().ammo.unwrap().clip < clip);
        assert!(!player.take_shots().is_empty());
        assert!(player.take_death_weapon().is_none());
        assert_eq!(suit.source().unwrap().life, 3);
    }
}

#[test]
fn live_exit_restores_exact_saved_rifle_and_ammo_without_healing_or_double_return() {
    let mut suit = Armor::default();
    let mut player = rifle();
    let mut v = PlayerVitals {
        health: 37,
        ..Default::default()
    };
    let plan = Plan::new(&resources(), 23, 1, player.equipment()).unwrap();
    assert!(suit.start(plan, source(), 1000, &mut player, &mut v));
    let duplicate = Plan::new(&resources(), 24, 1, player.equipment()).unwrap();
    assert!(!suit.start(duplicate, source(), 1100, &mut player, &mut v));
    v.health = 200;
    assert!(suit.validate(3000, 3, true, &mut player, &mut v));
    assert_eq!((v.health, v.max_health), (20, 100));
    let e = player.equipment();
    let a = e.ammo.unwrap();
    assert_eq!((e.weapon, a.clip, a.reserve), (352, 19, 77));
    assert_eq!(e.move_multiplier, 1.0);
    assert!(!suit.validate(3100, 3, true, &mut player, &mut v));
}

#[test]
fn death_and_different_life_discard_saved_state_and_never_restore_over_a_respawn() {
    for dead in [true, false] {
        let mut suit = Armor::default();
        let mut player = rifle();
        let mut v = PlayerVitals::default();
        assert!(suit.start(
            Plan::new(&resources(), 23, 1, player.equipment()).unwrap(),
            source(),
            1000,
            &mut player,
            &mut v
        ));
        if dead {
            v.health = 0;
        } else {
            player = crate::game::weapons::held::drop_test_gun(353, 353, 7, 21);
            v = Default::default();
        }
        assert!(!suit.validate(3000, if dead { 3 } else { 4 }, false, &mut player, &mut v));
        assert!(!suit.active());
        if dead {
            assert_eq!(v.health, 0);
        } else {
            assert_eq!(player.equipment().weapon, 353);
            assert_eq!(v.max_health, 100);
        }
    }
}

#[test]
fn missing_capture_rejects_the_plan_and_mode_identity_is_never_aliased() {
    let mut r = resources();
    r.models.remove("c_kp_juggernaut_tf_00");
    assert!(Plan::new(&r, 24, 1, rifle().equipment()).is_none());
    assert!(Plan::new(&r, 23, 1, rifle().equipment()).is_some());
    r.weapons.get_mut("mg4juggernaut_mp").unwrap().operator = None;
    assert!(Plan::new(&r, 23, 1, rifle().equipment()).is_none());
    assert!(Plan::new(&r, 22, 1, rifle().equipment()).is_none());
}

#[test]
fn death_sequence_change_keeps_the_suit_on_the_corpse_without_covering_a_new_life() {
    let mut suit = Armor::default();
    let mut player = rifle();
    let mut v = PlayerVitals::default();
    suit.start(
        Plan::new(&resources(), 23, 1, player.equipment()).unwrap(),
        source(),
        1000,
        &mut player,
        &mut v,
    );
    v.health = 0;
    suit.validate(2000, 4, false, &mut player, &mut v);
    let mut actor = codol_protocol::game::snapshot::enemy::EnemySnapshot {
        entity_number: 0,
        client_index: 0,
        team: 1,
        model: 4,
        attachment_models: [0; 10],
        attachment_tags: [0; 10],
        origin: [0.0; 3],
        angles: [0; 3],
        legs_animation: 0,
        torso_animation: 0,
        weapon: 352,
        events: Default::default(),
        alive: false,
    };
    suit.appearance(&mut actor, 4);
    assert_eq!(actor.model, 109);
    actor.model = 4;
    actor.alive = true;
    suit.appearance(&mut actor, 4);
    assert_eq!(actor.model, 4);
    assert!(suit.source().is_none());
}
