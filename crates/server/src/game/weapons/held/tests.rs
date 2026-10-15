use super::*;
fn melee_player() -> HeldWeapons {
    let mut p = aiming_player();
    p.set_melee_timing(MeleeTiming {
        duration_msec: 699,
        hit_msec: 250,
        recover_msec: 400,
        damage: 135,
        charge: None,
    });
    p
}
#[test]
fn primary_reward_marker_holds_then_throws_once_after_the_authored_delay() {
    let r = crate::game::weapons::grenade::marker_resources();
    let mut p = player();
    let d = crate::game::weapons::grenade::Definition::marker(&r, 2).unwrap();
    p.current.equipment.weapon = 293;
    p.current.equipment.ammo.as_mut().unwrap().clip = 1;
    p.current.marker = Some(d);
    let command = |time, buttons| UserCommand {
        time,
        buttons,
        weapons: [293, 0, 0],
        ..Default::default()
    };
    p.apply(&command(1100, 1));
    assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 3);
    p.apply(&command(1800, 1));
    assert!(p.take_throws().is_empty());
    assert_eq!(p.equipment().ammo.unwrap().clip, 1);
    p.apply(&command(1810, 0));
    p.apply(&command(1938, 0));
    assert!(p.take_throws().is_empty());
    p.apply(&command(1939, 0));
    let t = p.take_throws();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].definition.marker_reward, Some(2));
    assert_eq!(p.equipment().ammo.unwrap().clip, 0);
    assert!(p.take_shots().is_empty());
    p.apply(&command(2300, 1));
    assert!(p.take_throws().is_empty());
}
#[test]
fn switching_away_from_a_pulled_marker_keeps_the_marker_ammo() {
    let r = crate::game::weapons::grenade::marker_resources();
    let mut p = player();
    p.set_switch_timing(SwitchTiming {
        drop_msec: 10,
        raise_msec: 10,
        empty_drop_msec: 10,
        empty_raise_msec: 10,
    });
    let radio = r.radio(2).unwrap();
    p.grant_marker_reward(
        1,
        radio,
        crate::game::weapons::grenade::Definition::marker(&r, 2).unwrap(),
    );
    let command = |time, weapon, buttons| UserCommand {
        time,
        buttons,
        weapons: [weapon, 0, 0],
        ..Default::default()
    };
    p.apply(&command(1100, 293, 0));
    p.apply(&command(1110, 293, 0));
    p.apply(&command(1120, 293, 0));
    p.apply(&command(1130, 293, 1));
    assert!(p.marker_phase.is_some());
    p.apply(&command(1140, 352, 0));
    assert!(p.marker_phase.is_none());
    assert!(p.take_throws().is_empty());
    p.apply(&command(1150, 352, 0));
    assert_eq!(p.reward_inventory()[1].unwrap().ammo.unwrap().clip, 1);
}
#[test]
fn unthrown_primary_marker_is_cancelled_on_death_without_spending_ammo() {
    let r = crate::game::weapons::grenade::marker_resources();
    let mut p = player();
    p.current.equipment.weapon = 293;
    p.current.equipment.ammo.as_mut().unwrap().clip = 1;
    p.current.marker = crate::game::weapons::grenade::Definition::marker(&r, 2);
    p.apply(&UserCommand {
        time: 1100,
        buttons: 1,
        weapons: [293, 0, 0],
        ..Default::default()
    });
    p.drop_offhand_on_death(1200, [0; 3]);
    assert!(p.take_throws().is_empty());
    assert_eq!(p.equipment().ammo.unwrap().clip, 1);
}
#[test]
fn held_gun_melee_hits_once_after_delay_and_never_spends_ammo() {
    let mut p = melee_player();
    let ammo = p.equipment().ammo.unwrap();
    p.apply(&cmd(1100, 4 | 1));
    let swipe = p.equipment().firing.unwrap();
    assert_eq!(
        (swipe.state, swipe.animation & 0x3ff, swipe.delay_msec),
        (13, 8, 250)
    );
    assert!(p.take_shots().is_empty());
    assert!(p.take_melee_strikes().is_empty());
    p.apply(&cmd(1349, 4));
    assert!(p.take_melee_strikes().is_empty());
    let mut hit = cmd(1350, 4);
    hit.angles = [0, 1234, 0];
    p.apply(&hit);
    let strikes = p.take_melee_strikes();
    assert_eq!(strikes.len(), 1);
    assert_eq!(
        (strikes[0].time, strikes[0].angles, strikes[0].damage),
        (1350, hit.angles, 135)
    );
    assert_eq!(p.equipment().firing.unwrap().state, 14);
    p.apply(&hit);
    assert!(p.take_melee_strikes().is_empty());
    p.apply(&cmd(1799, 4));
    let recover = p.equipment().firing.unwrap();
    assert_eq!(
        (
            recover.state,
            recover.animation & 0x3ff,
            recover.remaining_msec
        ),
        (15, 22, 400)
    );
    p.apply(&cmd(2199, 4));
    p.apply(&cmd(2300, 4));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert!(p.take_melee_strikes().is_empty());
    assert_eq!(p.equipment().ammo.unwrap().clip, ammo.clip);
    assert_eq!(p.equipment().ammo.unwrap().reserve, ammo.reserve);
    p.apply(&cmd(2301, 0));
    p.apply(&cmd(2302, 4));
    assert_eq!(p.equipment().firing.unwrap().state, 13);
}
#[test]
fn melee_interrupts_reload_without_finishing_its_ammo_transfer() {
    let mut p = melee_player();
    p.current.equipment.ammo.as_mut().unwrap().clip = 5;
    p.apply(&cmd(1100, 16));
    assert_eq!(p.equipment().firing.unwrap().state, 8);
    p.apply(&cmd(1150, 4 | 0x800));
    assert_eq!(p.equipment().firing.unwrap().state, 13);
    assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
    p.apply(&cmd(1800, 0));
    assert_eq!(p.equipment().ammo.unwrap().clip, 5);
    assert_eq!(p.take_melee_strikes().len(), 1);
    assert!(!p.current.reload_transfer_pending);
}
#[test]
fn menu_and_weapon_raise_reject_melee_edges_without_buffering_them() {
    let mut p = melee_player();
    p.apply(&cmd(1100, 4 | 0x10000000));
    p.apply(&cmd(1200, 4));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    p.apply(&cmd(1300, 0));
    p.current.animate(1, 11, 500, 0);
    p.apply(&cmd(1400, 4));
    assert_eq!(p.equipment().firing.unwrap().state, 1);
    p.apply(&cmd(1800, 4));
    p.apply(&cmd(1900, 4));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert!(p.take_melee_strikes().is_empty());
}
#[test]
fn validated_charge_selects_native_animation_and_queues_only_one_approach() {
    let mut p = melee_player();
    p.current.melee.as_mut().unwrap().charge = Some(MeleeChargeTiming {
        duration_msec: 1159,
        hit_msec: 100,
    });
    let hint = MeleeApproach {
        yaw: 1234,
        distance: 100,
    };
    p.apply_with_melee_approach(&cmd(1100, 4), Some(hint));
    let f = p.equipment().firing.unwrap();
    assert_eq!(
        (f.state, f.animation & 0x3ff, f.remaining_msec, f.delay_msec),
        (13, 9, 1159, 100)
    );
    let (approach, duration) = p.take_melee_approach().unwrap();
    assert_eq!(
        (approach.yaw, approach.distance, duration),
        (1234, 100, 100)
    );
    p.apply_with_melee_approach(&cmd(1150, 4), Some(hint));
    assert!(p.take_melee_approach().is_none());
    assert!(p.take_melee_strikes().is_empty());
    p.apply(&cmd(1200, 0));
    assert_eq!(p.take_melee_strikes().len(), 1);
    let mut unknown = melee_player();
    unknown.apply_with_melee_approach(&cmd(1100, 4), Some(hint));
    assert_eq!(unknown.equipment().firing.unwrap().animation & 0x3ff, 8);
    assert!(unknown.take_melee_approach().is_none());
}
#[test]
fn single_bullets_use_stance_spread_and_ads_and_movement_changes() {
    let mut p = aiming_player();
    p.set_ballistics(crate::game::combat::Ballistics {
        damage: 33,
        range: 8192.0,
        near_range: 500.0,
        location_multipliers: None,
        falloff: None,
        pellets: 1,
        spread_degrees: 0.0,
    });
    p.set_spread_timing(SpreadTiming {
        minimum: [3.0, 2.0, 1.75],
        maximum: [6.0, 4.5, 4.0],
        decay: 4.0,
        fire_add: 0.6,
        turn_add: 0.0,
        move_add: 1.75,
        stance_decay: [1.0, 1.05, 1.1],
        ads: 0.0,
    });
    p.apply(&cmd(1100, 1));
    assert_eq!(p.take_shots()[0].ballistics.spread_degrees, 3.0);
    p.apply(&cmd(1179, 1));
    assert!(p.take_shots()[0].ballistics.spread_degrees > 3.0);
    p.apply(&cmd(1600, 0));
    p.apply(&cmd(1800, 0));
    p.apply(&cmd(2000, 0));
    p.spread_stance = 2;
    assert_eq!(p.shot_spread(), 1.75);
    p.apply(&cmd(2400, 0x800));
    p.apply(&cmd(2600, 0x800));
    assert_eq!(p.shot_spread(), 0.0);
    p.spread_speed = 1.0;
    p.apply(&cmd(2700, 0));
    p.apply(&cmd(2900, 0));
    assert!(p.equipment().spread_scale > 0.0);
}
#[test]
fn pump_weapon_waits_for_fire_and_rechamber_and_ejects_once() {
    let mut p = switchable_player(352, 1.0);
    p.current.fire_interval = Some(192);
    p.set_rechamber_timing(RechamberTiming {
        duration_msec: 550,
        eject_msec: 0,
    });
    p.apply(&select(1100, 352, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    assert!(p.equipment().needs_rechamber);
    p.apply(&select(1291, 352, 1));
    assert_eq!(p.equipment().firing.unwrap().state, 6);
    p.apply(&select(1292, 352, 1));
    assert_eq!(p.equipment().firing.unwrap().state, 7);
    assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 4);
    let sequence = p.equipment().events.unwrap().sequence;
    p.apply(&select(1293, 352, 0));
    assert_eq!(
        p.equipment().events.unwrap().sequence,
        sequence.wrapping_add(1)
    );
    p.apply(&select(1500, 352, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    assert!(p.equipment().needs_rechamber);
    assert_eq!(
        p.equipment().events.unwrap().sequence,
        sequence.wrapping_add(1)
    );
    p.apply(&select(1842, 352, 0));
    assert!(!p.equipment().needs_rechamber);
    p.apply(&select(1843, 352, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 28);
}
#[test]
fn alternate_switch_uses_native_phases_and_keeps_shared_ammo_and_secondary() {
    let mut primary = switchable_player(352, 1.0);
    primary.current.equipment.ammo.as_mut().unwrap().clip_key = 352;
    let mut alternate = switchable_player(353, 0.9);
    alternate.current.equipment.ammo.as_mut().unwrap().clip_key = 352;
    alternate.current.fire_interval = Some(400);
    let mut p = primary
        .with_alternate(
            alternate,
            AlternateTiming {
                drop_msec: 0,
                raise_msec: 1399,
            },
        )
        .with_secondary(switchable_player(354, 1.0));
    p.apply(&select(1100, 352, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    p.apply(&select(1200, 353, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 5);
    assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 19);
    p.apply(&select(1250, 353, 1));
    assert_eq!(p.equipment().weapon, 353);
    assert_eq!(p.equipment().inventory_weapon, Some(352));
    assert_eq!(p.equipment().alternates[0].unwrap().weapon, 353);
    assert_eq!(p.equipment().alternates[0].unwrap().ammo.unwrap().clip, 29);
    assert_eq!(p.equipment().firing.unwrap().state, 2);
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    p.apply(&select(2649, 353, 0));
    p.apply(&select(2700, 353, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 28);
    p.apply(&select(2800, 352, 0));
    p.apply(&select(2850, 352, 0));
    p.apply(&select(4249, 352, 0));
    assert_eq!(p.equipment().weapon, 352);
    assert_eq!(p.equipment().alternates[0].unwrap().weapon, 353);
    assert_eq!(p.equipment().alternates[0].unwrap().ammo.unwrap().clip, 28);
    assert_eq!(p.equipment().ammo.unwrap().clip, 28);
    assert_eq!(p.equipment().secondary.unwrap().weapon, 354);
    assert!(p.owns_weapon(353));
    assert!(!p.owns_weapon(355));
}
#[test]
fn only_authorized_shots_are_forwarded_once_to_the_combat_world() {
    let mut p = player();
    p.set_ballistics(crate::game::combat::Ballistics {
        damage: 33,
        range: 8192.0,
        near_range: 500.0,
        location_multipliers: None,
        falloff: None,
        pellets: 0,
        spread_degrees: 0.0,
    });
    for c in [
        cmd(1100, 1),
        cmd(1100, 1),
        cmd(1150, 1),
        cmd(1179, 1),
        cmd(1200, 0x10000001),
    ] {
        p.apply(&c);
    }
    let shots = p.take_shots();
    assert_eq!(
        shots.iter().map(|s| s.time).collect::<Vec<_>>(),
        [1100, 1179]
    );
    assert_eq!(shots[0].ballistics.damage, 33);
    assert!(p.take_shots().is_empty());
    assert_eq!(p.equipment().ammo.unwrap().clip, 28);
}
use codol_protocol::game::snapshot::Ammo;
fn player() -> HeldWeapons {
    HeldWeapons::new(
        Equipment {
            perks: Default::default(),
            abilities: Default::default(),
            offhands: [None, None],
            offhand_weapon: 0,
            grenade_time: 0,
            move_multiplier: 1.0,
            inventory_weapon: None,
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
            ammo: Some(Ammo {
                clip_key: 352,
                reserve_key: 163,
                clip: 30,
                reserve: 90,
            }),
        },
        Some(79),
        None,
        None,
    )
}
#[test]
fn ordinary_radios_and_nuke_switch_without_overwriting_rewards_or_rifle_forms() {
    let mut p = switchable_player(352, 0.9).with_alternate(
        switchable_player(353, 0.9),
        AlternateTiming {
            drop_msec: 100,
            raise_msec: 100,
        },
    );
    let timing = crate::game::rewards::killstreaks::test_resources().switch;
    p.grant_nuke(&crate::game::rewards::killstreaks::test_resources());
    p.grant_reward(1, 301, timing, 800);
    p.grant_reward(2, 302, timing, 800);
    assert_eq!(
        p.reward_inventory().map(|w| w.map(|w| w.weapon)),
        [Some(323), Some(301), Some(302), None]
    );
    let mut time = 1000;
    for id in [301, 302, 323, 353, 352, 302] {
        for _ in 0..30 {
            time += 100;
            p.apply(&UserCommand {
                time,
                weapons: [id, 0, 0],
                ..Default::default()
            });
        }
        assert_eq!(p.equipment().weapon, id);
        for owned in [301, 302, 323, 352, 353] {
            assert!(p.owns_weapon(owned));
        }
    }
    p.remove_reward(302);
    assert_eq!(p.equipment().weapon, 352);
    assert!(!p.owns_weapon(302));
    assert!(p.owns_weapon(301) && p.owns_weapon(323) && p.owns_weapon(353));
    p.remove_reward(301);
    assert!(p.owns_weapon(323));
}
#[test]
fn nuclear_reward_switches_through_native_inventory_and_returns_the_original_weapon() {
    let mut p = player();
    p.set_switch_timing(SwitchTiming {
        drop_msec: 233,
        raise_msec: 300,
        empty_drop_msec: 233,
        empty_raise_msec: 300,
    });
    let resources = crate::game::rewards::killstreaks::test_resources();
    let ammunition = p.equipment().ammo;
    p.grant_nuke(&resources);
    assert!(p.owns_weapon(323));
    assert_eq!(p.reward_inventory()[0].unwrap().ammo.unwrap().clip, 1);
    let mut command = cmd(1100, 0);
    command.weapons[0] = 323;
    p.apply(&command);
    assert_eq!(p.equipment().weapon, 352);
    command.time = 1333;
    p.apply(&command);
    assert_eq!(p.equipment().weapon, 323);
    assert_eq!(p.reward_inventory()[0].unwrap().weapon, 352);
    p.remove_nuke(323);
    assert_eq!(p.equipment().weapon, 352);
    assert_eq!(
        p.equipment()
            .ammo
            .map(|a| (a.clip_key, a.reserve_key, a.clip, a.reserve)),
        ammunition.map(|a| (a.clip_key, a.reserve_key, a.clip, a.reserve))
    );
    assert!(!p.owns_weapon(323));
    assert!(p.reward_inventory().iter().all(Option::is_none));
}
fn cmd(time: u32, buttons: u32) -> UserCommand {
    UserCommand {
        time,
        buttons,
        weapons: [352, 0, 0],
        ..Default::default()
    }
}
pub(crate) fn scavenger_player(stock: u16, cap: u32) -> HeldWeapons {
    let mut p = player();
    p.set_perks(crate::game::status::perks::Perks::selected([0, 60040, 0], &[60040]).unwrap());
    p.set_ammo_limits(AmmoLimits {
        contribution: cap,
        shared: None,
        clip_only: false,
        scavenger_allowed: true,
    });
    p.current.equipment.ammo.as_mut().unwrap().reserve = stock;
    p
}
#[test]
fn scavenger_requires_the_flag_and_current_guns_native_remaining_capacity() {
    let mut p = scavenger_player(90, 100);
    p.set_perks(Default::default());
    let before = p.equipment().ammo;
    assert!(!p.scavenge(229));
    assert_eq!(p.equipment().ammo.unwrap().reserve, before.unwrap().reserve);
    p.set_perks(crate::game::status::perks::Perks::selected([0, 60040, 0], &[60040]).unwrap());
    assert_eq!(p.equipment().perks, [4, 0, 0]);
    assert!(p.scavenge(229));
    assert_eq!(p.equipment().ammo.unwrap().reserve, 100);
    assert_eq!(p.events.sequence, 1);
    assert_eq!(
        p.events.slots[0],
        codol_protocol::game::snapshot::events::Event {
            id: 16,
            parameter: 229
        }
    );
    assert!(!p.scavenge(229));
    assert_eq!(p.events.sequence, 1);
    let mut other = scavenger_player(0, 100);
    other.current.equipment.weapon = 354;
    other.current.equipment.ammo.as_mut().unwrap().reserve_key = 164;
    p = p.with_secondary(other);
    assert!(!p.can_scavenge());
}
#[test]
fn scavenger_shared_reserve_aliases_receive_one_clip_and_remain_synchronized() {
    let mut other = scavenger_player(80, 100);
    other.current.equipment.weapon = 354;
    let mut p = scavenger_player(80, 100).with_secondary(other);
    // Distinct carried guns add native cap contributions to the same pool.
    p.secondary.as_mut().unwrap().equipment.weapon = 354;
    p.secondary
        .as_mut()
        .unwrap()
        .equipment
        .ammo
        .as_mut()
        .unwrap()
        .clip_key = 354;
    let mut alt = scavenger_player(80, 100);
    alt.current.equipment.weapon = 353;
    p = p.with_alternate(
        alt,
        AlternateTiming {
            drop_msec: 100,
            raise_msec: 100,
        },
    );
    assert_eq!(p.ammo_space(&p.current), 120);
    assert!(p.scavenge(229));
    assert!(
        p.weapons()
            .all(|w| w.equipment.ammo.unwrap().reserve == 110)
    );
    assert!(p.weapons().all(|w| w.equipment.ammo.unwrap().clip == 30));
    p.sync_ammunition();
    assert!(
        p.weapons()
            .all(|w| w.equipment.ammo.unwrap().reserve == 110)
    );
}
#[test]
fn scavenger_group_margin_counts_distinct_reserve_pools_and_clamps_the_combined_total() {
    let mut p = scavenger_player(40, 100);
    p.current.equipment.ammo.as_mut().unwrap().reserve_key = 200;
    let mut other = scavenger_player(10, 100);
    other.current.equipment.weapon = 354;
    other.current.equipment.ammo.as_mut().unwrap().reserve_key = 164;
    for w in [&mut p.current, &mut other.current] {
        w.ammo_limits.as_mut().unwrap().shared = Some([7, 60]);
    }
    p = p.with_secondary(other);
    assert_eq!(p.ammo_space(&p.current), 10);
    assert!(p.scavenge(229));
    assert_eq!(p.equipment().ammo.unwrap().reserve, 50);
    assert_eq!(
        p.secondary
            .as_ref()
            .unwrap()
            .equipment
            .ammo
            .unwrap()
            .reserve,
        10
    );
    assert!(!p.can_scavenge());
}
#[test]
fn scavenger_restores_tactical_counts_with_native_cap_but_keeps_excluded_recipes_empty() {
    let mut p = scavenger_player(0, 100);
    p.set_offhands([
        Some(crate::game::weapons::grenade::Equipped {
            definition: crate::game::weapons::grenade::captured("frag_grenade_mp"),
            count: 0,
        }),
        Some(crate::game::weapons::grenade::Equipped {
            definition: crate::game::weapons::grenade::captured("concussion_grenade_mp"),
            count: 0,
        }),
    ]);
    assert!(p.scavenge(229));
    assert_eq!(p.equipment().offhands[0].unwrap().count, 0);
    assert_eq!(p.equipment().offhands[1].unwrap().count, 1);
    assert!(p.scavenge(229));
    assert!(p.scavenge(229));
    assert_eq!(p.equipment().offhands[1].unwrap().count, 2);
    assert_eq!(p.equipment().offhand_weapon, 0);
}
fn grenade_player(name: &str) -> HeldWeapons {
    let mut p = player();
    p.set_offhand_switch(Some(AlternateTiming {
        drop_msec: 80,
        raise_msec: 300,
    }));
    p.set_offhands([
        Some(crate::game::weapons::grenade::Equipped {
            definition: crate::game::weapons::grenade::captured(name),
            count: 1,
        }),
        None,
    ]);
    p
}
#[test]
fn offhand_pull_hold_throw_delay_consumes_once_and_restores_the_gun() {
    let mut p = grenade_player("frag_grenade_mp");
    let ammo = p.equipment().ammo.unwrap();
    p.apply(&cmd(1100, 0x4000));
    assert_eq!(p.equipment().firing.unwrap().state, 16);
    p.apply(&cmd(1180, 0x4000));
    assert_eq!(p.equipment().firing.unwrap().state, 17);
    p.apply(&cmd(1780, 0x4000));
    assert_eq!(p.equipment().firing.unwrap().state, 18);
    assert_eq!(p.equipment().grenade_time, 3500);
    p.apply(&cmd(1880, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 19);
    p.apply(&cmd(2008, 0));
    assert!(p.take_throws().is_empty());
    p.apply(&cmd(2009, 0));
    let throws = p.take_throws();
    assert_eq!(throws.len(), 1);
    assert_eq!(throws[0].fuse_msec, 3271);
    assert!(!throws[0].in_hand);
    p.apply(&cmd(2009, 0));
    assert!(p.take_throws().is_empty());
    assert_eq!(p.equipment().offhands[0].unwrap().count, 0);
    p.apply(&cmd(2240, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 21);
    p.apply(&cmd(2540, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    p.apply(&cmd(2600, 0x4000));
    assert!(p.take_throws().is_empty());
    assert_eq!(p.equipment().weapon, 352);
    assert_eq!(p.equipment().ammo.unwrap().clip, ammo.clip);
}
#[test]
fn tactical_quick_throw_preserves_the_frag_and_does_not_cook_smoke_while_held() {
    let mut p = grenade_player("frag_grenade_mp");
    let ammo = p.equipment().ammo.unwrap();
    p.offhands[1] = Some(crate::game::weapons::grenade::Equipped {
        definition: crate::game::weapons::grenade::captured("smoke_grenade_mp"),
        count: 1,
    });
    for time in [1100, 1180, 1780, 5000] {
        p.apply(&cmd(time, 0x10000));
    }
    assert_eq!(p.equipment().firing.unwrap().state, 18);
    assert_eq!(p.equipment().offhand_weapon, 236);
    assert_eq!(p.equipment().grenade_time, 1000);
    assert!(p.take_throws().is_empty());
    p.apply(&cmd(5100, 0));
    p.apply(&cmd(5229, 0));
    let thrown = p.take_throws();
    assert_eq!(thrown.len(), 1);
    assert_eq!(thrown[0].fuse_msec, 1000);
    assert_eq!(thrown[0].definition.index, 236);
    assert!(!thrown[0].in_hand);
    p.apply(&cmd(5229, 0));
    assert!(p.take_throws().is_empty());
    assert_eq!(p.equipment().offhands[1].unwrap().count, 0);
    assert_eq!(p.equipment().offhands[0].unwrap().count, 1);
    assert_eq!(p.equipment().ammo.unwrap().clip, ammo.clip);
}
#[test]
fn cooked_expiry_death_drop_and_non_cookable_semtex_keep_distinct_fuses() {
    for name in ["frag_grenade_mp", "semtex_mp"] {
        let mut p = grenade_player(name);
        for t in [1100, 1180, 1780] {
            p.apply(&cmd(t, 0x4000));
        }
        if name == "frag_grenade_mp" {
            p.apply(&cmd(5280, 0x4000));
            let request = p.take_throws().pop().unwrap();
            assert!(request.in_hand);
            assert_eq!(request.fuse_msec, 0);
            p.drop_offhand_on_death(5280, [0; 3]);
            assert!(p.take_throws().is_empty());
        } else {
            p.apply(&cmd(7780, 0x4000));
            assert_eq!(p.equipment().grenade_time, 2000);
            assert!(p.take_throws().is_empty());
            p.drop_offhand_on_death(7800, [0; 3]);
            let request = p.take_throws().pop().unwrap();
            assert!(request.dropped);
            assert_eq!(request.fuse_msec, 2000);
            p.drop_offhand_on_death(7900, [0; 3]);
            assert!(p.take_throws().is_empty());
        }
    }
}
fn aiming_player() -> HeldWeapons {
    let mut p = player();
    p.current.ads = Some(AdsTiming {
        in_rate: 0.004,
        out_rate: 0.004,
        reload_allowed: false,
        reload_interruptible: false,
        reload_end_msec: 349,
    });
    p.current.equipment.aim = Some(Aim::default());
    p.current.reload = Some(reload_timing());
    p
}

#[test]
fn captured_quickdraw_speeds_ads_and_switch_without_changing_weapon_data() {
    let mut p = aiming_player();
    p.set_perks(
        crate::game::status::perks::Perks::selected([60041, 60066, 0], &[60041, 60066]).unwrap(),
    );
    let rates = p.current.ads.unwrap();
    p.apply(&cmd(1100, 0x800));
    assert!((p.equipment().aim.unwrap().fraction - 0.54).abs() < 1e-6);
    assert_eq!(p.current.ads.unwrap().in_rate, rates.in_rate);
    assert_eq!(p.equipment().perks, [0x408, 0, 0]);
    assert_eq!(p.equipment().move_multiplier, 1.07);
    assert_eq!(p.move_scale(), 1.07);
    let timing = SwitchTiming {
        drop_msec: 300,
        raise_msec: 300,
        empty_drop_msec: 300,
        empty_raise_msec: 300,
    };
    p.set_switch_timing(timing);
    let mut secondary = player();
    secondary.current.equipment.weapon = 353;
    secondary.set_switch_timing(timing);
    p = p.with_secondary(secondary);
    p.begin_drop(353);
    let request = UserCommand {
        weapons: [353, 0, 0],
        ..cmd(1200, 0)
    };
    p.apply(&request);
    assert_eq!(p.equipment().firing.unwrap().remaining_msec, 145);
    p.apply(&request);
    assert_eq!(p.equipment().firing.unwrap().remaining_msec, 145);
    p.set_perks(Default::default());
    assert_eq!(p.equipment().perks, [0; 3]);
    assert_eq!(p.move_scale(), 1.0);
}
#[test]
fn ads_progress_survives_frames_reverses_smoothly_and_reaches_exact_endpoints() {
    let mut p = aiming_player();
    p.apply(&cmd(1100, 0x800));
    assert!((p.equipment().aim.unwrap().fraction - 0.4).abs() < 1e-6);
    p.apply(&cmd(1100, 0x800)); // duplicate does not advance ADS
    assert!((p.equipment().aim.unwrap().fraction - 0.4).abs() < 1e-6);
    p.apply(&cmd(1150, 0));
    let aim = p.equipment().aim.unwrap();
    assert!(!aim.button && (aim.fraction - 0.2).abs() < 1e-6);
    p.apply(&cmd(1500, 0x800));
    assert_eq!(p.equipment().aim.unwrap().fraction, 1.0);
    p.apply(&cmd(1800, 0));
    assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
    p.apply(&cmd(2200, 0x10000800));
    assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
}
#[test]
fn ads_leaves_scope_during_reload_and_reenters_only_at_target_threshold() {
    let mut p = aiming_player();
    p.apply(&cmd(1250, 0x800));
    p.apply(&cmd(1300, 0x801)); // make room for a partial reload
    p.apply(&cmd(1400, 0x810));
    assert_eq!(p.equipment().firing.unwrap().state, 8);
    p.apply(&cmd(1700, 0x800));
    assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
    p.apply(&cmd(3079, 0x800)); //350ms left, still blocked
    assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
    p.apply(&cmd(3080, 0x800)); //349ms left, allowed
    assert!((p.equipment().aim.unwrap().fraction - 0.004).abs() < 1e-6);
    p.apply(&cmd(3330, 0x800));
    assert_eq!(p.equipment().aim.unwrap().fraction, 1.0);
    assert_eq!(p.equipment().ammo.unwrap().clip, 30);
    assert_eq!(p.equipment().ammo.unwrap().reserve, 89);
}
#[test]
fn shots_obey_cooldown_and_duplicate_inputs_never_consume_twice() {
    let mut p = player();
    for c in [
        cmd(1010, 1),
        cmd(1010, 1),
        cmd(1005, 1),
        cmd(1050, 1),
        cmd(1088, 1),
    ] {
        p.apply(&c);
    }
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    p.apply(&cmd(1089, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 28);
    p.apply(&cmd(1180, 0));
    for time in (1200..2200).step_by(50) {
        p.apply(&cmd(time, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
    }
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert_eq!(p.equipment().ammo.unwrap().reserve, 90);
}

#[test]
fn semiautomatic_modes_require_release_and_a_new_press_for_each_shot() {
    for fire_type in [1, 5] {
        let mut p = player();
        p.set_fire_type(fire_type).unwrap();
        p.apply(&cmd(1100, 1));
        p.apply(&cmd(1100, 1)); // retried input cannot rearm the trigger
        p.apply(&cmd(1200, 1));
        p.apply(&cmd(1300, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        assert_eq!(p.equipment().fire_count, 1);
        assert_eq!(p.equipment().events.unwrap().sequence, 1);
        p.apply(&cmd(1301, 0));
        assert_eq!(p.equipment().fire_count, 0);
        p.apply(&cmd(1302, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
        // Native shot-count gating lets a new press wait for cooldown,
        // then fires once; keeping it held does not fire another shot.
        p.apply(&cmd(1303, 0));
        p.apply(&cmd(1304, 1));
        p.apply(&cmd(1400, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 27);
        p.apply(&cmd(1401, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 27);
        p.apply(&cmd(1402, 0));
        p.apply(&cmd(1403, 1));
        p.apply(&cmd(1480, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 26);
    }
}
fn ranger() -> HeldWeapons {
    let mut p = player();
    p.set_fire_type(5).unwrap();
    p.current.fire_interval = Some(9);
    p.current.equipment.ammo.as_mut().unwrap().clip = 2;
    p.set_ballistics(crate::game::combat::Ballistics {
        damage: 30,
        range: 1200.0,
        near_range: 100.0,
        location_multipliers: None,
        falloff: None,
        pellets: 8,
        spread_degrees: 5.0,
    });
    p
}
#[test]
fn ranger_secondary_trigger_is_semi_automatic_and_switches_barrels_without_release() {
    for first in [1, 0x100000] {
        let mut p = ranger();
        p.apply(&cmd(1001, first));
        assert_eq!(p.equipment().ammo.unwrap().clip, 1);
        assert_eq!(p.equipment().barrel_flags, u8::from(first != 1));
        p.apply(&cmd(1020, first));
        p.apply(&cmd(1030, first));
        assert_eq!(p.equipment().ammo.unwrap().clip, 1);
        let second = if first == 1 { 0x100000 } else { 1 };
        p.apply(&cmd(1040, second));
        assert_eq!(p.equipment().ammo.unwrap().clip, 0);
        assert_eq!(p.equipment().barrel_flags, u8::from(second != 1));
        let shots = p.take_shots();
        assert_eq!(shots.len(), 2);
        assert_eq!(shots[1].time, 1040);
    }
}
#[test]
fn ranger_both_triggers_spend_two_shells_but_never_create_an_absent_shell() {
    for clip in [1, 2] {
        let mut p = ranger();
        p.current.equipment.ammo.as_mut().unwrap().clip = clip;
        p.apply(&cmd(1001, 0x100001));
        assert_eq!(p.equipment().ammo.unwrap().clip, 0);
        assert_eq!(p.equipment().barrel_flags, if clip == 2 { 3 } else { 1 });
        assert_eq!(p.equipment().events.unwrap().sequence, clip as u8);
        assert_eq!(p.equipment().fire_count, clip as u8);
        let shots = p.take_shots();
        assert_eq!(shots.len(), usize::from(clip));
        assert!(
            shots
                .iter()
                .all(|s| s.time == 1001 && s.ballistics.pellets == 8)
        );
        p.apply(&cmd(1001, 0x100001));
        assert!(p.take_shots().is_empty());
        p.apply(&cmd(1020, 0));
        assert_eq!(p.equipment().barrel_flags, 1);
    }
    let mut p = player();
    p.apply(&cmd(1001, 0x100000));
    assert_eq!(p.equipment().ammo.unwrap().clip, 30);
}

#[test]
fn special_modes_are_explicitly_rejected_without_changing_fire_mode() {
    let mut p = player();
    p.set_fire_type(1).unwrap();
    for fire_type in [6, 7, u32::MAX] {
        assert!(p.set_fire_type(fire_type).is_err());
    }
    p.apply(&cmd(1100, 1));
    p.apply(&cmd(1200, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
}
#[test]
fn bursts_complete_after_release_and_recover_before_rearming() {
    for count in 2..=4 {
        let mut p = player();
        p.set_fire_type(count).unwrap();
        p.set_ballistics(crate::game::combat::Ballistics {
            damage: 30,
            range: 8192.0,
            near_range: 1000.0,
            location_multipliers: None,
            falloff: None,
            pellets: 0,
            spread_degrees: 0.0,
        });
        p.apply(&cmd(1001, 1));
        for round in 1..count {
            p.apply(&cmd(1001 + round * 79, 0));
            assert_eq!(p.equipment().fire_count, round as u8 + 1);
            let clip = p.equipment().ammo.unwrap().clip;
            p.apply(&cmd(1001 + round * 79, 0));
            assert_eq!(p.equipment().ammo.unwrap().clip, clip);
        }
        assert_eq!(p.equipment().ammo.unwrap().clip, 30 - count as u16);
        let shots = p.take_shots();
        assert_eq!(shots.len(), count as usize);
        assert_eq!(
            shots.iter().map(|s| s.time).collect::<Vec<_>>(),
            (0..count).map(|i| 1001 + i * 79).collect::<Vec<_>>()
        );
        assert!(p.take_shots().is_empty());
        let recovery = 1001 + count * 79;
        p.apply(&cmd(recovery, 0));
        assert_eq!(p.equipment().firing.unwrap().remaining_msec, 200);
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        p.apply(&cmd(recovery + 199, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 30 - count as u16);
        p.apply(&cmd(recovery + 200, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 30 - count as u16);
        p.apply(&cmd(recovery + 201, 0));
        assert_eq!(p.equipment().fire_count, 0);
        p.apply(&cmd(recovery + 202, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29 - count as u16);
    }
}
#[test]
fn trigger_repress_during_recovery_is_buffered_and_cleared_by_the_next_burst() {
    let mut p = player();
    p.set_fire_type(3).unwrap();
    for time in [1001, 1080, 1159] {
        p.apply(&cmd(time, 1));
    }
    p.apply(&cmd(1200, 0));
    p.apply(&cmd(1201, 1));
    assert!(p.equipment().queued_burst);
    assert_eq!(p.equipment().fire_count, 3);
    p.apply(&cmd(1238, 1));
    assert_eq!(p.equipment().firing.unwrap().remaining_msec, 200);
    p.apply(&cmd(1437, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 27);
    p.apply(&cmd(1438, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 26);
    assert_eq!(p.equipment().fire_count, 1);
    assert!(!p.equipment().queued_burst);
    p.apply(&cmd(1517, 0));
    p.apply(&cmd(1596, 0));
    assert_eq!(p.equipment().ammo.unwrap().clip, 24);
}
#[test]
fn explicit_reload_interrupts_an_unfinished_burst_before_the_next_round() {
    let mut p = player();
    p.current.reload = Some(reload_timing());
    p.set_fire_type(3).unwrap();
    p.apply(&cmd(1001, 1));
    p.apply(&cmd(1030, 16));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    assert_eq!(p.equipment().fire_count, 0);
    assert_eq!(p.equipment().firing.unwrap().state, 8);
    p.apply(&cmd(1100, 0));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    assert!(!p.equipment().queued_burst);
}
#[test]
fn held_burst_trigger_does_not_repeat_and_empty_clip_truncates_the_burst() {
    let mut p = player();
    p.set_fire_type(3).unwrap();
    for time in (1001..=3001).step_by(79) {
        p.apply(&cmd(time, 1));
    }
    assert_eq!(p.equipment().ammo.unwrap().clip, 27);
    assert_eq!(p.equipment().fire_count, 3);
    assert_eq!(p.equipment().events.unwrap().sequence, 3);
    let mut p = player();
    p.set_fire_type(3).unwrap();
    p.current.equipment.ammo.as_mut().unwrap().clip = 2;
    p.apply(&cmd(1001, 1));
    p.apply(&cmd(1080, 0));
    p.apply(&cmd(1159, 0));
    assert_eq!(p.equipment().ammo.unwrap().clip, 0);
    assert_eq!(p.equipment().fire_count, 2);
    assert_eq!(p.equipment().firing.unwrap().remaining_msec, 200);
    p.apply(&cmd(1400, 0));
    assert_eq!(p.equipment().fire_count, 0);
    assert_eq!(p.equipment().events.unwrap().sequence, 2);
}
#[test]
fn empty_clip_menu_and_weapon_switch_do_not_create_ammo_or_shots() {
    let mut p = player();
    p.apply(&cmd(1100, 0x10000001));
    let mut other = cmd(1200, 1);
    other.weapons[0] = 180;
    p.apply(&other);
    assert_eq!(p.equipment().ammo.unwrap().clip, 30);
    for time in (1300..=6000).step_by(100) {
        p.apply(&cmd(time, 1));
    }
    assert_eq!(p.equipment().ammo.unwrap().clip, 0);
    assert_eq!(p.equipment().ammo.unwrap().reserve, 90);
    assert_eq!(player().equipment().ammo.unwrap().clip, 30); // new match only
}
fn reload_timing() -> ReloadTiming {
    ReloadTiming {
        normal_msec: 2029,
        empty_msec: 2799,
        add_msec: 1399,
        empty_add_msec: 1399,
        segmented: None,
    }
}
/// spas12_mp: start1000 (+shell at600), shell550 (+200), end850.
fn shotgun() -> HeldWeapons {
    let mut p = player();
    p.current.reload = Some(ReloadTiming {
        normal_msec: 550,
        empty_msec: 550,
        add_msec: 200,
        empty_add_msec: 400,
        segmented: Some(SegmentedReload {
            start_msec: 1000,
            start_add_msec: 600,
            end_msec: 850,
        }),
    });
    p.current.magazine_capacity = 4;
    let ammo = p.current.equipment.ammo.as_mut().unwrap();
    ammo.clip = 1;
    ammo.reserve = 10;
    p
}
fn state(p: &HeldWeapons) -> (u8, u16, u16) {
    let firing = p.equipment().firing.unwrap();
    let ammo = p.equipment().ammo.unwrap();
    (firing.state, firing.animation & 0x3ff, ammo.clip)
}
#[test]
fn shotgun_reload_starts_loads_one_shell_per_cycle_and_ends() {
    let mut p = shotgun();
    p.apply(&cmd(1100, 0x10));
    assert_eq!(state(&p), (10, 17, 1));
    p.apply(&cmd(1700, 0));
    assert_eq!(state(&p), (10, 17, 2)); // Shell inserted during the start.
    p.apply(&cmd(2100, 0));
    assert_eq!(state(&p), (8, 13, 2));
    p.apply(&cmd(2300, 0));
    assert_eq!(state(&p), (8, 13, 3));
    p.apply(&cmd(2650, 0));
    assert_eq!(state(&p), (8, 13, 3)); // Next shell began.
    p.apply(&cmd(2850, 0));
    assert_eq!(state(&p), (8, 13, 4));
    p.apply(&cmd(3200, 0));
    assert_eq!(state(&p), (12, 18, 4)); // Full: reload end.
    p.apply(&cmd(4050, 0));
    assert_eq!(state(&p).0, 0);
    assert_eq!(p.equipment().ammo.unwrap().reserve, 7);
}
#[test]
fn firing_interrupts_a_shotgun_reload_after_the_current_shell() {
    let mut p = shotgun();
    p.apply(&cmd(1100, 0x10));
    p.apply(&cmd(2100, 0));
    assert_eq!(state(&p), (8, 13, 2));
    p.apply(&cmd(2200, 1)); // Fire pressed during the shell.
    assert_eq!(state(&p).0, 9);
    p.apply(&cmd(2650, 0));
    assert_eq!(state(&p), (12, 18, 3)); // Shell completed, then end.
    assert!(p.take_shots().is_empty());
}
#[test]
fn reload_transfers_ammo_once_at_marker_and_blocks_fire_until_finished() {
    let mut p = player();
    p.current.reload = Some(reload_timing());
    p.apply(&cmd(1100, 1));
    p.apply(&cmd(1200, 0x10));
    assert_eq!(p.equipment().firing.unwrap().state, 8);
    p.apply(&cmd(2598, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    p.apply(&cmd(2599, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 30);
    assert_eq!(p.equipment().ammo.unwrap().reserve, 89);
    p.apply(&cmd(2599, 1)); // repeated marker must not transfer twice
    p.apply(&cmd(3228, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 30);
    assert_eq!(p.equipment().firing.unwrap().state, 8);
    p.apply(&cmd(3229, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    p.apply(&cmd(3230, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    assert_eq!(p.equipment().ammo.unwrap().reserve, 89);
}
#[test]
fn empty_reload_uses_longer_duration_and_cannot_create_missing_reserve() {
    let mut p = player();
    p.current.reload = Some(reload_timing());
    let ammo = p.current.equipment.ammo.as_mut().unwrap();
    ammo.clip = 0;
    ammo.reserve = 7;
    p.apply(&cmd(1100, 0)); // empty-magazine automatic reload
    assert_eq!(p.equipment().firing.unwrap().remaining_msec, 2799);
    assert_eq!(p.equipment().firing.unwrap().animation & !0x400, 15);
    p.apply(&cmd(2499, 0));
    assert_eq!(p.equipment().ammo.unwrap().clip, 7);
    assert_eq!(p.equipment().ammo.unwrap().reserve, 0);
    p.apply(&cmd(3899, 0));
    p.apply(&cmd(4000, 0x10));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert_eq!(p.equipment().ammo.unwrap().clip, 7);
}
#[test]
fn target_reload_button_starts_partial_magazine_reload_but_bit32_does_not() {
    let mut p = player();
    p.current.reload = Some(reload_timing());
    p.apply(&cmd(1100, 1));
    p.apply(&cmd(1200, 0x20));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    p.apply(&cmd(1300, 0x10));
    assert_eq!(p.equipment().firing.unwrap().state, 8);
    assert_eq!(p.equipment().firing.unwrap().animation & !0x400, 13);
    p.apply(&cmd(2699, 0));
    assert_eq!(p.equipment().ammo.unwrap().clip, 30);
    assert_eq!(p.equipment().ammo.unwrap().reserve, 89);
}

fn switchable_player(weapon: u16, scale: f32) -> HeldWeapons {
    let mut player = player();
    player.current.equipment.weapon = weapon;
    player.current.equipment.ammo.as_mut().unwrap().clip_key = weapon;
    player.current.equipment.ammo.as_mut().unwrap().reserve_key = weapon;
    player.set_switch_timing(SwitchTiming {
        drop_msec: 300,
        raise_msec: 400,
        empty_drop_msec: 500,
        empty_raise_msec: 600,
    });
    player.set_move_scale(scale);
    player
}
#[test]
fn captured_marathon_changes_the_current_weapons_sprint_rules() {
    let mut p = switchable_player(352, 1.0);
    p.set_sprint_duration_scale(0.5);
    assert_eq!(p.sprint_rules().maximum(), 2000);
    p.set_perks(crate::game::status::perks::Perks::selected([0, 0, 60038], &[60038]).unwrap());
    assert_eq!(p.sprint_rules().duration_seconds, -1.0);
    assert_eq!(p.sprint_rules().maximum(), 0);
    p.set_perks(Default::default());
    assert_eq!(p.sprint_rules().maximum(), 2000);
}
#[test]
fn stock_ads_mobility_follows_the_selected_weapon_without_changing_hip_speed() {
    let mut primary = switchable_player(352, 0.95);
    primary.set_ads_move_scale(2.2);
    let mut secondary = switchable_player(354, 1.0);
    secondary.set_ads_move_scale(0.8);
    let mut p = primary.with_secondary(secondary);
    assert_eq!(p.move_scale(), 0.95);
    assert_eq!(p.ads_move_scale(), 2.2);
    p.apply(&select(1100, 354, 0));
    p.apply(&select(2100, 354, 0));
    assert_eq!(p.move_scale(), 1.0);
    assert_eq!(p.ads_move_scale(), 0.8);
    p.apply(&select(2200, 352, 0));
    p.apply(&select(3200, 352, 0));
    assert_eq!(p.move_scale(), 0.95);
    assert_eq!(p.ads_move_scale(), 2.2);
    p.set_ads_move_scale(0.0);
    assert_eq!(p.ads_move_scale(), 1.0);
}
#[test]
fn captured_lightweight_removes_landing_damage_without_affecting_explosions() {
    let mut p = switchable_player(352, 1.0);
    assert_eq!(p.fall_damage(100), 100);
    p.set_perks(crate::game::status::perks::Perks::selected([60041, 0, 0], &[60041]).unwrap());
    assert_eq!(p.fall_damage(100), 0);
    assert_eq!(p.fall_damage(50), 0);
    assert_eq!(p.explosive_damage(50), 50);
    p.set_perks(Default::default());
    assert_eq!(p.fall_damage(50), 50);
}
#[test]
fn sprint_uses_native_in_loop_out_and_blocks_shots_until_recovery() {
    let mut p = aiming_player();
    p.set_sprint_timing(SprintTiming {
        in_msec: 300,
        out_msec: 450,
    });
    let clip = p.equipment().ammo.unwrap().clip;
    p.set_sprinting(true);
    p.apply(&cmd(1100, 2));
    let entered = p.equipment().firing.unwrap();
    assert_eq!(
        (
            entered.state,
            entered.animation & 0x3ff,
            entered.remaining_msec
        ),
        (23, 25, 300)
    );
    p.apply(&cmd(1399, 2));
    assert_eq!(p.equipment().firing.unwrap().state, 23);
    p.apply(&cmd(1400, 2));
    assert_eq!(p.equipment().firing.unwrap().state, 24);
    assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 26);
    p.apply(&cmd(1500, 2));
    assert_eq!(p.equipment().ammo.unwrap().clip, clip);
    p.set_sprinting(false);
    p.apply(&cmd(1501, 1));
    let out = p.equipment().firing.unwrap();
    assert_eq!(
        (out.state, out.animation & 0x3ff, out.remaining_msec),
        (25, 27, 450)
    );
    p.apply(&cmd(1950, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, clip);
    p.apply(&cmd(1951, 1));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    p.apply(&cmd(1952, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, clip - 1);
}
#[test]
fn dexterity_divides_sprint_recovery_without_changing_authored_times() {
    let mut p = aiming_player();
    p.set_sprint_timing(SprintTiming {
        in_msec: 300,
        out_msec: 500,
    });
    p.set_perks(crate::game::status::perks::Perks::selected([0, 0, 60073], &[60073]).unwrap());
    p.set_sprinting(true);
    p.apply(&cmd(1100, 2));
    p.set_sprinting(false);
    p.apply(&cmd(1101, 0));
    let out = p.equipment().firing.unwrap();
    assert_eq!((out.state, out.remaining_msec), (25, 333));
    assert_eq!(p.current.sprint_timing.unwrap().out_msec, 500);
    let unchanged = p.equipment().firing.unwrap();
    p.apply(&cmd(1101, 0));
    assert_eq!(
        p.equipment().firing.unwrap().remaining_msec,
        unchanged.remaining_msec
    );
}
#[test]
fn switching_away_cancels_partial_burst_and_does_not_resume_it_on_return() {
    let mut p = switchable_player(352, 1.0);
    p.set_fire_type(3).unwrap();
    let mut p = p.with_secondary(switchable_player(180, 1.0));
    p.apply(&select(1001, 352, 1));
    p.apply(&select(1030, 180, 0));
    assert_eq!(p.equipment().fire_count, 0);
    assert!(!p.equipment().queued_burst);
    p.apply(&select(1330, 180, 0));
    p.apply(&select(1730, 180, 0));
    p.apply(&select(1740, 352, 0));
    p.apply(&select(2040, 352, 0));
    p.apply(&select(2440, 352, 0));
    assert_eq!(p.equipment().weapon, 352);
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    p.apply(&select(2500, 352, 0));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    p.apply(&select(2501, 352, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 28);
}

fn select(time: u32, weapon: u16, buttons: u32) -> UserCommand {
    UserCommand {
        weapons: [weapon, 0, 0],
        ..cmd(time, buttons)
    }
}

#[test]
fn switching_drops_old_weapon_then_raises_owned_weapon_and_preserves_each_magazine() {
    let primary = switchable_player(352, 0.9);
    let secondary = switchable_player(180, 1.0);
    let mut player = primary.with_secondary(secondary);
    player.apply(&select(1100, 352, 1));
    assert_eq!(player.equipment().ammo.unwrap().clip, 29);
    player.apply(&select(1200, 180, 1));
    assert_eq!(player.equipment().weapon, 352);
    assert_eq!(player.equipment().firing.unwrap().state, 3);
    assert_eq!(player.equipment().firing.unwrap().animation & !0x400, 10);
    assert_eq!(player.move_scale(), 0.9);
    let sequence = player.equipment().events.unwrap().sequence;
    player.apply(&select(1200, 180, 1));
    assert_eq!(player.equipment().events.unwrap().sequence, sequence);
    player.apply(&select(1499, 180, 1));
    assert_eq!(player.equipment().weapon, 352);
    player.apply(&select(1500, 180, 1));
    assert_eq!(player.equipment().weapon, 180);
    assert_eq!(player.equipment().secondary.unwrap().ammo.unwrap().clip, 29);
    assert_eq!(player.equipment().firing.unwrap().state, 1);
    assert_eq!(player.equipment().firing.unwrap().animation & !0x400, 11);
    assert_eq!(player.move_scale(), 1.0);
    player.apply(&select(1899, 180, 1));
    assert_eq!(player.equipment().ammo.unwrap().clip, 30);
    player.apply(&select(1900, 180, 1));
    assert_eq!(player.equipment().firing.unwrap().state, 0);
    player.apply(&select(1901, 180, 1));
    assert_eq!(player.equipment().ammo.unwrap().clip, 29);
    player.apply(&select(2000, 352, 0));
    player.apply(&select(2300, 352, 0));
    player.apply(&select(2700, 352, 0));
    assert_eq!(player.equipment().weapon, 352);
    assert_eq!(player.equipment().ammo.unwrap().clip, 29);
    assert_eq!(player.equipment().secondary.unwrap().ammo.unwrap().clip, 29);
}

#[test]
fn quick_wheel_switches_follow_native_interrupts_without_a_second_raise() {
    let mut p = switchable_player(352, 0.9).with_secondary(switchable_player(180, 1.0));
    p.apply(&select(1100, 352, 0));
    // Returning during the drop finishes the change at once (0x423b10).
    p.apply(&select(1200, 180, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 3);
    p.apply(&select(1250, 352, 0));
    assert_eq!(p.equipment().weapon, 352);
    assert_eq!(p.equipment().firing.unwrap().state, 1);
    assert_eq!(p.equipment().firing.unwrap().remaining_msec, 400);
    p.apply(&select(1650, 352, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    // A new request interrupts the raise and drops immediately (0x4236a0).
    p.apply(&select(1700, 180, 0));
    p.apply(&select(2000, 180, 0));
    assert_eq!(
        (p.equipment().weapon, p.equipment().firing.unwrap().state),
        (180, 1)
    );
    p.apply(&select(2100, 352, 0));
    assert_eq!(
        (p.equipment().weapon, p.equipment().firing.unwrap().state),
        (180, 3)
    );
    p.apply(&select(2400, 352, 0));
    assert_eq!(
        (p.equipment().weapon, p.equipment().firing.unwrap().state),
        (352, 1)
    );
    p.apply(&select(2800, 352, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert_eq!(p.equipment().secondary.unwrap().weapon, 180);
}

#[test]
fn switching_preserves_each_weapons_fire_type() {
    let primary = switchable_player(352, 0.9);
    let mut secondary = switchable_player(180, 1.0);
    secondary.set_fire_type(1).unwrap();
    let mut p = primary.with_secondary(secondary);
    p.apply(&select(1100, 352, 1));
    p.apply(&select(1200, 352, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 28);
    p.apply(&select(1300, 180, 0));
    p.apply(&select(1600, 180, 0));
    p.apply(&select(2000, 180, 0));
    p.apply(&select(2001, 180, 1));
    p.apply(&select(2101, 180, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    p.apply(&select(2200, 352, 0));
    p.apply(&select(2500, 352, 0));
    p.apply(&select(2900, 352, 0));
    p.apply(&select(2901, 352, 1));
    p.apply(&select(3001, 352, 1));
    assert_eq!(p.equipment().ammo.unwrap().clip, 26);
}

#[test]
fn movement_events_share_the_predictable_sequence_with_weapon_events() {
    let mut p = player();
    p.movement_event(142, 4); // Target EV_FOOTSTEP_RUN, surface 4.
    p.apply(&cmd(1100, 1));
    let events = p.equipment().events.unwrap();
    assert_eq!(events.sequence, 2);
    assert_eq!(events.slots[0].id, 142);
    assert_eq!(events.slots[0].parameter, 4);
    assert_eq!(events.slots[1].id, events::FIRE_WEAPON);
}

#[test]
fn empty_magazines_select_measured_empty_switch_animations_and_timings() {
    let mut primary = switchable_player(352, 1.0);
    primary.current.equipment.ammo.as_mut().unwrap().clip = 0;
    let mut secondary = switchable_player(180, 1.0);
    secondary.current.equipment.ammo.as_mut().unwrap().clip = 0;
    let mut player = primary.with_secondary(secondary);
    player.apply(&select(1100, 180, 0));
    let drop = player.equipment().firing.unwrap();
    assert_eq!((drop.animation & !0x400, drop.remaining_msec), (23, 500));
    player.apply(&select(1600, 180, 0));
    let raise = player.equipment().firing.unwrap();
    assert_eq!((raise.animation & !0x400, raise.remaining_msec), (24, 600));
    assert_eq!(player.equipment().ammo.unwrap().clip, 0);
}

#[test]
fn unowned_and_menu_captured_switches_are_ignored_and_drop_can_be_cancelled() {
    let mut player = switchable_player(352, 1.0).with_secondary(switchable_player(180, 1.0));
    player.apply(&select(1100, 800, 0));
    player.apply(&select(1200, 180, 0x10000000));
    assert_eq!(player.equipment().firing.unwrap().state, 0);
    assert_eq!(player.equipment().events.unwrap().sequence, 0);
    player.apply(&select(1300, 180, 0));
    player.apply(&select(1600, 352, 0));
    assert_eq!(player.equipment().weapon, 352);
    assert_eq!(player.equipment().firing.unwrap().state, 1);
}

#[test]
fn switching_cancels_unfinished_reload_without_moving_reserve() {
    let mut primary = switchable_player(352, 1.0);
    primary.current.reload = Some(reload_timing());
    let mut player = primary.with_secondary(switchable_player(180, 1.0));
    player.apply(&select(1100, 352, 1));
    player.apply(&select(1200, 352, 0x10));
    player.apply(&select(1300, 180, 0));
    player.apply(&select(1600, 180, 0));
    player.apply(&select(2000, 180, 0));
    let old = player.equipment().secondary.unwrap().ammo.unwrap();
    assert_eq!((old.clip, old.reserve), (29, 90));
    player.apply(&select(2100, 352, 0));
    player.apply(&select(2400, 352, 0));
    player.apply(&select(2800, 352, 0));
    player.apply(&select(5000, 352, 0));
    let old = player.equipment().ammo.unwrap();
    assert_eq!((old.clip, old.reserve), (29, 90));
}

#[test]
fn reload_events_fire_once_and_loadout_replacement_preserves_event_and_command_clocks() {
    let mut player = player();
    player.current.reload = Some(reload_timing());
    player.apply(&cmd(1100, 1));
    player.apply(&cmd(1200, 0x10));
    let queue = player.equipment().events.unwrap();
    assert_eq!(queue.sequence, 4);
    assert_eq!(queue.slots.map(|event| event.id), [36, 23, 28, 24]);
    player.apply(&cmd(2599, 0));
    player.apply(&cmd(2599, 0));
    assert_eq!(player.equipment().events.unwrap().sequence, 5);
    assert_eq!(player.equipment().events.unwrap().slots[0].id, 29);
    player.apply(&cmd(3229, 0));
    assert_eq!(player.equipment().events.unwrap().sequence, 6);
    assert_eq!(player.equipment().events.unwrap().slots[1].id, 27);
    player.replace_loadout(
        switchable_player(353, 0.95),
        Some(switchable_player(180, 1.0)),
    );
    assert_eq!(player.equipment().events.unwrap().sequence, 9);
    player.apply(&select(3229, 353, 1));
    player.apply(&select(3000, 353, 1));
    assert_eq!(player.equipment().events.unwrap().sequence, 9);
    assert_eq!(player.equipment().firing.unwrap().remaining_msec, 400);
    player.apply(&select(3629, 353, 0));
    assert_eq!(player.equipment().firing.unwrap().state, 0);
}

#[test]
fn dive_weapon_phases_use_authored_times_and_block_actions_until_recovery() {
    let mut p = melee_player();
    p.set_dive_timing(DiveTiming {
        in_msec: 150,
        out_msec: 250,
        prone_blocked: false,
    });
    let cmd = |time, buttons| UserCommand {
        time,
        buttons,
        weapons: [352, 0, 0],
        ..Default::default()
    };
    p.set_diving(true, false);
    p.apply(&cmd(1100, 0x800));
    assert_eq!(p.equipment().firing.unwrap().state, 31);
    assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 35);
    for (time, state) in [(1249, 31), (1250, 32), (1300, 32)] {
        p.apply(&cmd(time, 1 | 4 | 16 | 0x4000));
        assert_eq!(p.equipment().firing.unwrap().state, state);
        assert!(p.take_shots().is_empty());
        assert!(p.take_melee_strikes().is_empty());
        assert!(p.take_throws().is_empty());
    }
    p.set_diving(true, true);
    p.apply(&cmd(1320, 0));
    assert_eq!(p.equipment().firing.unwrap().state, 33);
    assert_eq!(p.equipment().firing.unwrap().remaining_msec, 250);
    p.apply(&cmd(1569, 1));
    assert_eq!(p.equipment().firing.unwrap().state, 33);
    p.set_diving(false, false);
    p.apply(&cmd(1570, 1));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert!(p.take_shots().is_empty());
    assert_eq!(p.equipment().ammo.unwrap().clip, 30);
}
#[test]
fn dive_empty_variants_zero_times_and_reload_interruption_keep_ammunition() {
    let mut p = player();
    p.set_dive_timing(DiveTiming {
        in_msec: 0,
        out_msec: 0,
        prone_blocked: false,
    });
    p.current.equipment.ammo.as_mut().unwrap().clip = 0;
    let cmd = |time| UserCommand {
        time,
        weapons: [352, 0, 0],
        ..Default::default()
    };
    p.set_diving(true, false);
    p.apply(&cmd(1100));
    assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 38);
    p.apply(&cmd(1101));
    assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 39);
    p.set_diving(true, true);
    p.apply(&cmd(1102));
    assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 40);
    p.apply(&cmd(1103));
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert_eq!(p.equipment().ammo.unwrap().clip, 0);
    p.current.equipment.ammo.as_mut().unwrap().clip = 3;
    p.current.animate(8, 13, 1000, 500);
    p.current.reload_transfer_pending = true;
    p.set_diving(true, false);
    p.apply(&cmd(1200));
    assert_eq!(p.equipment().firing.unwrap().state, 31);
    assert!(!p.current.reload_transfer_pending);
    assert_eq!(p.equipment().ammo.unwrap().clip, 3);
    p.freeze_at(1300);
    assert_eq!(p.equipment().firing.unwrap().state, 0);
    assert!(!p.diving);
}
