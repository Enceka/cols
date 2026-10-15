use super::*;

fn floor(contents: u32) -> Collision {
    let file = tempfile::NamedTempFile::new().unwrap();
    let value = serde_json::json!({
        "schema_version":1,"read_only":true,
        "client_sha256":"e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
        "map":"maps/mp/test.d3dbsp","ncs_crc":0,"captured_contents_mask":0x0381_4031u32,
        "brushes":[{"center":[0,0,-50],"half":[2000,2000,50],"contents":contents,
            "planes":[],"plane_surface_flags":vec![3<<20;6]}],"triangles":[]});
    std::fs::write(file.path(), serde_json::to_vec(&value).unwrap()).unwrap();
    Collision::load(file.path()).unwrap()
}
fn cmd(time: u32, buttons: u32) -> UserCommand {
    UserCommand {
        time,
        buttons,
        direction: 1,
        weapons: [352, 0, 0],
        ..Default::default()
    }
}
fn eligible(now: u32) -> Motion {
    let mut m = Motion::new([0.0, 0.0, 0.03125]);
    m.command_time = now;
    m.velocity = [285.0, 0.0, 0.0];
    m.sprint.last_start = 1000;
    m.sprint.last_end = now;
    m
}
#[test]
fn dive_eligibility_uses_same_command_strict_times_and_current_view_direction() {
    let request = 0x800100;
    for (now, start, end, last_end, yaw, blocked, expected) in [
        (2000, 1749, 499, 2000, 0, false, true),
        (2000, 1750, 499, 2000, 0, false, false),
        (2000, 1000, 500, 2000, 0, false, false),
        (2000, 1000, 499, 1999, 0, false, false),
        (2000, 1000, 499, 2000, 2000, false, true), //10.99 degrees
        (2000, 1000, 499, 2000, 2200, false, false),
        (2000, 1000, 499, 2000, 0, true, false),
    ] {
        let mut m = eligible(now);
        m.sprint.last_start = start;
        m.sprint.last_end = last_end;
        m.dive_end_time = end;
        m.dive_rules.prone_blocked = blocked;
        let mut c = cmd(now, request);
        c.angles[1] = yaw;
        m.check_dive(&c);
        assert_eq!(
            m.diving(),
            expected,
            "{start}/{end}/{last_end}/{yaw}/{blocked}"
        );
        if expected {
            assert_eq!(m.velocity[0], 285.0);
            assert!((m.velocity[2] - 264.95282).abs() < 0.001);
        }
    }
}
#[test]
fn dive_sprint_launch_land_slide_lockout_and_reset_share_one_motion_clock() {
    let c = floor(1);
    let mut m = Motion::new([0.0, 0.0, 0.03125]);
    m.command_time = 2000;
    for time in (2020..=2600).step_by(20) {
        m.apply(&cmd(time, 2), 1.0, false, &c);
    }
    let start = m.origin;
    m.apply(&cmd(2620, 0x800100), 1.0, false, &c);
    assert!(m.diving());
    assert_eq!(m.jump_time, 2620);
    assert_eq!(m.stance, Stance::Prone);
    assert!(m.origin[0] > start[0] + 5.0 && m.origin[2] > start[2]);
    let mut events = m.take_movement_events();
    let mut saw_slide = false;
    for time in (2640..=4300).step_by(20) {
        m.apply(&cmd(time, 0), 1.0, false, &c);
        events.extend(m.take_movement_events());
        if m.dive_sliding() {
            saw_slide = true;
            assert!(m.grounded());
        }
        if !m.diving() {
            break;
        }
    }
    assert!(saw_slide);
    assert!(!m.diving());
    assert!(m.dive_end_time > 2620);
    assert_eq!(events.iter().filter(|e| e.id == DTP_LAUNCH).count(), 1);
    assert_eq!(events.iter().filter(|e| e.id == DTP_LAND).count(), 1);
    assert!(events.iter().all(|e| e.parameter == 4)); //native surface3 ->dive material4
    let end = m.dive_end_time;
    m.apply(&cmd(end + 99, 0), 1.0, false, &c);
    assert_eq!(m.stance, Stance::Prone);
    assert!(m.dive_locked());
    m.apply(&cmd(end + 100, 0), 1.0, false, &c);
    assert!(!m.dive_locked());
    assert_eq!(m.stance, Stance::Stand);
    let fresh = Motion::new(start);
    assert_eq!(fresh.dive_end_time, 0);
    assert_eq!(fresh.jump_time, 0);
}
#[test]
fn dive_slide_mask_zeroes_friction_without_becoming_a_player_obstacle() {
    let ordinary = floor(1);
    let special = floor(0x0100_0000);
    let mut m = eligible(2150);
    m.flags |= FLAG;
    m.jump_time = (-2000i32) as u32;
    m.stance = Stance::Prone;
    assert_eq!(m.dive_friction(&ordinary), 0.5);
    assert_eq!(m.dive_friction(&special), 0.0);
    assert_eq!(
        special
            .trace([0.0, 0.0, 2.0], [0.0, 0.0, -2.0], 30.0)
            .fraction,
        1.0
    );
    m.command_time = 2300;
    assert_eq!(m.dive_friction(&special), 1.0);
    assert!(!m.diving());
    assert_eq!(m.dive_end_time, 2300);
}
#[test]
fn dive_apex_and_jump_height_ability_do_not_overwrite_horizontal_velocity() {
    let mut m = eligible(2000);
    m.dive_rules.height_multiplier = 2.0;
    m.check_dive(&cmd(2000, 0x800100));
    assert!((m.velocity[2] - (800.0f32 * 78.0).sqrt() * 1.5).abs() < 0.001);
    m.origin[2] = m.jump_origin_z + 79.0;
    m.command_time = 2399;
    m.velocity[2] = -50.0;
    m.dive_apex();
    assert_eq!(m.velocity, [285.0, 0.0, 0.0]);
    m.command_time = 2400;
    m.velocity[2] = -50.0;
    m.dive_apex();
    assert_eq!(m.velocity[2], -50.0);
    m.cancel_dive();
    assert!(!m.diving());
    assert!(!m.dive_locked());
}

#[test]
fn dive_backlogged_command_drives_both_motion_and_weapon_through_touchdown() {
    let collision = std::sync::Arc::new(floor(1));
    let state =
        crate::game::session::prepared::human_test_state([0.0, 0.0, 0.03125], collision.clone());
    let mut player = state.player().unwrap();
    player.set_dive_timing(crate::game::weapons::held::DiveTiming {
        in_msec: 150,
        out_msec: 250,
        prone_blocked: false,
    });
    player.freeze_at(2600);
    let mut m = eligible(2600);
    m.flags |= 0x4000;
    m.sprint.last_start = 2000;
    let command = cmd(3600, 0x800100);
    m.apply(&command, 1.0, false, &collision);
    let frames = m.take_dive_frames();
    assert!(frames.iter().any(|f| f.active && !f.sliding));
    assert!(frames.iter().any(|f| f.sliding));
    let touchdown = frames.iter().position(|f| f.sliding).unwrap();
    let touch_command = cmd(frames[touchdown].time, 0x800100);
    player.apply_with_dive_frames(&touch_command, None, &frames[..=touchdown]);
    assert_eq!(player.equipment().firing.unwrap().state, 33);
    assert_eq!(player.equipment().firing.unwrap().remaining_msec, 250);
    player.apply_with_dive_frames(&command, None, &frames[touchdown + 1..]);
    assert_eq!(player.equipment().firing.unwrap().state, 0);
    assert_eq!(player.equipment().ammo.unwrap().clip, 30);
    assert!(player.take_shots().is_empty());
}

#[test]
fn grounded_dive_slide_follows_the_slope_without_accumulating_downward_gravity() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let value = serde_json::json!({
        "schema_version":1,"read_only":true,
        "client_sha256":"e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b",
        "map":"maps/mp/test.d3dbsp","ncs_crc":0,
        "brushes":[],"triangles":[{"vertices":[[-500,-500,-125],[500,-500,125],[0,500,0]],
            "contents":1,"surface_flags":0}]});
    std::fs::write(file.path(), serde_json::to_vec(&value).unwrap()).unwrap();
    let collision = Collision::load(file.path()).unwrap();
    let offset = 15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0);
    let mut m = Motion::new([0.0, 0.0, offset + 0.05]);
    m.command_time = 2000;
    m.flags = FLAG | 1;
    m.stance = Stance::Prone;
    m.jump_time = (-2000i32) as u32;
    m.velocity = [184.0, 0.0, 46.0];
    for time in (2020..=2240).step_by(20) {
        m.apply(&cmd(time, 0x100), 1.0, false, &collision);
        assert!(m.dive_sliding() && m.grounded(), "{m:?}");
        assert!((m.origin[2] - m.origin[0] * 0.25 - offset).abs() < 0.1);
        assert!(
            (m.velocity[2] - m.velocity[0] * 0.25).abs() <= 1.0,
            "slide velocity must remain tangent to the slope: {:?}",
            m.velocity
        );
    }
    assert!(m.origin[0] > 20.0);
}
