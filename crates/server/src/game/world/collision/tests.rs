use super::*;
use crate::game::player::movement::Motion;
use crate::game::player::movement::Stance;
use codol_protocol::game::channel::usercmd::UserCommand;
#[test]
#[ignore = "derives a supported ledge and fall route from captured Killhouse collision"]
fn prepare_captured_landing_scene() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("tdm-killhouse-hud-state.json")).unwrap())
            .unwrap();
    let c = Collision::load(std::path::Path::new(
        state["collision_file"].as_str().unwrap(),
    ))
    .unwrap();
    let mut selected = None;
    'search: for x in (2900..4400).step_by(40) {
        for y in (-1000..1300).step_by(40) {
            let start = [x as f32, y as f32, 500.0];
            let hit = c.trace(start, [start[0], start[1], 100.0], 70.0);
            if hit.start_solid || hit.fraction == 1.0 || hit.normal[2] < 0.9 {
                continue;
            }
            let origin = [start[0], start[1], 500.0 - 400.0 * hit.fraction];
            if origin[2] < 180.0 || origin[2] > 400.0 {
                continue;
            }
            for yaw in (0..65536u32).step_by(8192) {
                let mut m = Motion::new(origin);
                m.command_time = 1000;
                let mut ledge = origin;
                for _ in 0..130 {
                    let input = UserCommand {
                        time: m.command_time + 16,
                        direction: 1,
                        angles: [0, yaw as u16, 0],
                        ..Default::default()
                    };
                    m.apply(&input, 1.0, false, &c);
                    if m.grounded() && m.origin[2] >= origin[2] - 2.0 {
                        ledge = m.origin;
                    }
                    for landing in m.take_landings() {
                        if landing.damage >= 30 {
                            selected = Some((ledge, yaw, m.origin, landing));
                            break 'search;
                        }
                    }
                }
            }
        }
    }
    let (origin, yaw, end, landing) = selected.expect("supported ledge route with a damaging fall");
    for team in 0..2 {
        state["match_config"]["spawns"][team][0] = serde_json::json!(origin);
    }
    // These files are diagnostic only; normal map settings are not replaced.
    std::fs::write(
        root.join("landing-diagnostic-state.json"),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("landing-diagnostic-route.json"), serde_json::to_vec_pretty(&serde_json::json!({"origin":origin,"yaw":yaw,"end":end,"time":landing.time,"raw_damage":landing.damage,"event":landing.event.map(|e|(e.id,e.parameter)),"collision_file":state["collision_file"]})).unwrap()).unwrap();
    println!(
        "landing diagnostic: origin={origin:?} yaw={yaw} end={end:?} raw_damage={}",
        landing.damage
    );
}
fn world(brushes: Vec<BrushExport>, triangles: Vec<TriangleExport>) -> Collision {
    Collision::from_export(Export {
        schema_version: 1,
        read_only: true,
        client_sha256: CLIENT_SHA.into(),
        map: "maps/mp/test.d3dbsp".into(),
        ncs_crc: 0,
        captured_contents_mask: PLAYER_MASK | SHOT_MASK,
        brushes,
        triangles,
    })
    .unwrap()
}
fn brush(center: V3, half: V3) -> BrushExport {
    BrushExport {
        glass_piece: None,
        center,
        half,
        contents: 1,
        planes: vec![],
        plane_surface_flags: vec![],
    }
}
fn floor() -> BrushExport {
    brush([0.0, 0.0, -20.0], [2000.0, 2000.0, 20.0])
}
fn pane(x: f32, piece: Option<u16>) -> BrushExport {
    let mut pane = brush([x, 0.0, 45.0], [0.5, 100.0, 45.0]);
    pane.contents = 0x0800_0010;
    pane.glass_piece = piece;
    pane
}

#[test]
fn glass_hit_updates_bullet_capsule_projectile_and_support_traces_only_for_its_piece() {
    let world = world(
        vec![floor(), pane(50.0, Some(168)), pane(100.0, None)],
        vec![],
    );
    let start = [0.0, 0.0, 40.0];
    let end = [150.0, 0.0, 40.0];
    assert_eq!(world.raycast(start, end).glass_piece, Some(168));
    assert_eq!(
        world.trace([0.0; 3], [150.0, 0.0, 0.0], 70.0).glass_piece,
        Some(168)
    );
    assert_eq!(
        world.projectile_trace(start, end, 2.0).glass_piece,
        Some(168)
    );
    let hit = world.shot_raycast(start, end, 1.0);
    assert_eq!(world.destroyed_glass(), vec![168]);
    assert_eq!(hit.glass_piece, None);
    assert!((hit.fraction - 99.5 / 150.0).abs() < 0.0001);
    assert_eq!(world.projectile_trace(start, end, 2.0).glass_piece, None);
    assert_eq!(
        world.trace([0.0; 3], [150.0, 0.0, 0.0], 70.0).glass_piece,
        None
    );
    assert_eq!(
        world
            .ground_raycast([50.0, 0.0, 100.0], [50.0, 0.0, -10.0])
            .contents,
        1
    );
    assert!(!world.shatter_glass(168));
    assert!(!world.shatter_glass(167));
    assert!(!world.shatter_glass(1024));
}

#[test]
fn glass_cannot_be_destroyed_behind_a_body_wall_or_from_inside_a_solid() {
    let world = world(
        vec![floor(), pane(50.0, Some(0)), pane(100.0, Some(1))],
        vec![],
    );
    let start = [0.0, 0.0, 40.0];
    let end = [150.0, 0.0, 40.0];
    world.shot_raycast(start, end, 0.25);
    assert!(world.destroyed_glass().is_empty());
    world.shot_raycast([50.0, 0.0, 40.0], end, 1.0);
    assert!(world.destroyed_glass().is_empty());
    world.shot_raycast(start, end, 0.5);
    assert_eq!(world.destroyed_glass(), vec![0]);
    assert_eq!(world.raycast(start, end).glass_piece, Some(1));
    let wall = self::world(
        vec![floor(), pane(50.0, None), pane(100.0, Some(1))],
        vec![],
    );
    wall.shot_raycast(start, end, 1.0);
    assert!(wall.destroyed_glass().is_empty());
}

#[test]
fn glass_planning_and_visibility_keep_real_collision_and_other_sessions_intact() {
    let template = world(vec![floor(), pane(50.0, Some(1023))], vec![]);
    let first = template.fresh_session();
    let second = template.fresh_session();
    let start = [0.0, 0.0, 40.0];
    let end = [100.0, 0.0, 40.0];
    assert_eq!(first.visibility_raycast(start, end).contents, 0);
    let planning = first.navigation_view();
    assert_eq!(planning.raycast(start, end).contents, 0);
    assert!(!planning.shatter_glass(1023));
    assert_eq!(first.raycast(start, end).glass_piece, Some(1023));
    first.shot_raycast(start, end, 1.0);
    assert_eq!(first.raycast(start, end).contents, 0);
    for other in [&template, &second, &first.fresh_session()] {
        assert_eq!(other.raycast(start, end).glass_piece, Some(1023));
        assert!(other.destroyed_glass().is_empty());
        assert!(Arc::ptr_eq(&first.primitives, &other.primitives));
    }
}
fn cmd(time: u32, direction: u8, buttons: u32) -> UserCommand {
    UserCommand {
        time,
        direction,
        buttons,
        ..Default::default()
    }
}
fn tick(m: &mut Motion, c: &Collision, direction: u8, buttons: u32, n: usize) {
    for _ in 0..n {
        m.apply(&cmd(m.command_time + 16, direction, buttons), 1.0, false, c);
    }
}
#[test]
fn marathon_changes_real_movement_after_the_normal_meter_runs_out() {
    let c = world(vec![floor()], vec![]);
    let mut normal = Motion::new([0.0, 0.0, 0.03125]);
    normal.apply(&cmd(1000, 0, 0), 1.0, false, &c);
    let mut marathon = normal.clone();
    marathon.set_sprint_rules(crate::game::player::sprint::Rules {
        duration_seconds: -1.0,
        ..Default::default()
    });
    for time in (1016..=6616).step_by(16) {
        let input = cmd(time, 1, 2);
        normal.apply(&input, 1.0, false, &c);
        marathon.apply(&input, 1.0, false, &c);
    }
    assert_eq!(normal.flags & 0x4000, 0);
    assert!(normal.sprint.button_up_required && normal.sprint.delay);
    assert_ne!(marathon.flags & 0x4000, 0);
    assert!(marathon.origin[0] > normal.origin[0] + 100.0);
    let before = normal.sprint;
    normal.apply(&cmd(normal.command_time, 1, 0), 1.0, false, &c);
    assert_eq!(normal.sprint, before);
    normal.apply(&cmd(6632, 1, 0), 1.0, false, &c);
    normal.apply(&cmd(6648, 1, 2), 1.0, false, &c);
    assert_ne!(normal.flags & 0x4000, 0);
    normal.cancel_sprint();
    assert_eq!(normal.flags & 0x4000, 0);
    assert_eq!(normal.sprint.last_end, normal.command_time);
    let stopped = normal.sprint;
    normal.cancel_sprint();
    assert_eq!(normal.sprint, stopped);
}
#[test]
fn separation_respects_walls_and_preserves_airborne_velocity_and_time() {
    let c = world(
        vec![floor(), brush([50.0, 0.0, 100.0], [10.0, 1000.0, 100.0])],
        vec![],
    );
    let mut m = Motion::new([0.0, 0.0, 0.03125]);
    tick(&mut m, &c, 0, 0, 1);
    let time = m.command_time;
    for _ in 0..20 {
        m.move_separation([4.0, 0.0], &c);
        assert!(m.grounded());
        assert!(!c.trace(m.origin, m.origin, 70.0).start_solid);
    }
    assert!(m.origin[0] > 24.0 && m.origin[0] < 25.0);
    assert_eq!(m.command_time, time);
    for vertical in [-50.0, 50.0] {
        let mut air = Motion::new([0.0, 0.0, 5.0]);
        air.velocity = [10.0, 20.0, vertical];
        air.move_separation([4.0, 0.0], &c);
        assert_eq!(air.origin, [4.0, 0.0, 5.0]);
        assert_eq!(air.velocity, [10.0, 20.0, vertical]);
        assert_eq!(air.command_time, 0);
        assert!(!air.grounded());
    }
}
#[test]
fn melee_charge_uses_native_midpoint_deceleration_and_clears_at_expiry() {
    let c = world(vec![floor()], vec![]);
    let mut m = Motion::new([0.0, 0.0, 0.03125]);
    m.apply(&cmd(1000, 0, 0), 1.0, false, &c);
    m.start_melee_charge(
        crate::game::weapons::held::MeleeApproach {
            yaw: 0,
            distance: 50,
        },
        250,
    );
    assert_eq!(m.origin[0], 0.0);
    assert_eq!(m.melee_charge().unwrap().remaining_msec, 0);
    for (i, x) in [18.0, 32.0, 42.0, 48.0, 50.0].into_iter().enumerate() {
        let input = cmd(1050 + i as u32 * 50, 0, 0);
        m.apply(&input, 1.0, false, &c);
        assert!((m.origin[0] - x).abs() < 0.002, "{:?}", m.origin);
        let before = m.origin;
        m.apply(&input, 1.0, false, &c);
        assert_eq!(m.origin, before);
    }
    assert!(m.melee_charge().is_none());
    assert_eq!(m.flags & 0x10000, 0);
    assert_eq!(m.velocity, [0.0; 3]);
}
#[test]
fn melee_charge_matches_native_expiry_during_a_long_command() {
    let c = world(vec![floor()], vec![]);
    let mut m = Motion::new([0.0, 0.0, 0.03125]);
    m.apply(&cmd(1509, 0, 0), 1.0, false, &c);
    m.start_melee_charge(
        crate::game::weapons::held::MeleeApproach {
            yaw: 466,
            distance: 75,
        },
        100,
    );
    m.apply(&cmd(1739, 0, 0), 1.0, false, &c);
    // Native command1509->1739, client-1791694333310: the last
    //66ms subdivision spans timer expiry and still integrates its full dt.
    assert!((m.origin[0] - 83.07715).abs() < 0.002, "{:?}", m.origin);
    assert!((m.origin[1] - 3.7141).abs() < 0.002, "{:?}", m.origin);
    assert!(m.melee_charge().is_none());
    assert_eq!(m.velocity, [0.0; 3]);
}
#[test]
fn melee_charge_sweeps_the_capsule_and_cannot_cross_a_wall() {
    let c = world(
        vec![floor(), brush([60.0, 0.0, 100.0], [5.0, 1000.0, 100.0])],
        vec![],
    );
    let mut m = Motion::new([0.0, 0.0, 0.03125]);
    m.apply(&cmd(1000, 0, 0), 1.0, false, &c);
    m.start_melee_charge(
        crate::game::weapons::held::MeleeApproach {
            yaw: 0,
            distance: 200,
        },
        100,
    );
    for time in (1010..=1100).step_by(10) {
        m.apply(&cmd(time, 0, 0), 1.0, false, &c);
    }
    assert!(m.origin[0] > 39.0 && m.origin[0] < 40.0, "{:?}", m.origin);
    assert!(!c.trace(m.origin, m.origin, 70.0).start_solid);
    assert!(m.melee_charge().is_none());
    assert_eq!(m.velocity, [0.0; 3]);
}
#[test]
fn separation_leaves_a_ledge_without_snapping_to_a_distant_floor() {
    let c = world(
        vec![
            brush([0.0, 0.0, -20.0], [32.0, 1000.0, 20.0]),
            brush([0.0, 0.0, -120.0], [1000.0, 1000.0, 20.0]),
        ],
        vec![],
    );
    let mut m = Motion::new([30.0, 0.0, 0.03125]);
    tick(&mut m, &c, 0, 0, 1);
    for _ in 0..10 {
        m.move_separation([4.0, 0.0], &c);
    }
    assert!(!m.grounded());
    assert!(m.origin[0] > 60.0 && m.origin[2] > -18.0, "{:?}", m.origin);
    tick(&mut m, &c, 0, 0, 100);
    assert!(m.grounded());
    assert!((m.origin[2] + 100.0 - 0.125).abs() < 0.1, "{m:?}");
}
#[test]
fn wall_stops_forward_motion_but_allows_sliding() {
    let c = world(
        vec![floor(), brush([100.0, 0.0, 100.0], [10.0, 1000.0, 100.0])],
        vec![],
    );
    let mut m = Motion::new([0.0, 0.0, 2.0]);
    tick(&mut m, &c, 5, 0, 150);
    assert!(m.origin[0] <= 75.01, "{:?}", m.origin);
    assert!(m.origin[1] < -100.0);
    assert!(m.grounded());
}
#[test]
fn falling_only_advances_the_command_interval_before_landing() {
    let c = world(vec![floor()], vec![]);
    let mut m = Motion::new([0.0, 0.0, 5.0]);
    m.command_time = 1000;
    m.velocity = [0.0, 0.0, -40.0];
    m.apply(&cmd(1050, 0, 0), 1.0, false, &c);
    assert!((m.origin[2] - 2.0).abs() < 0.001, "{:?}", m);
    assert_eq!(m.velocity[2], -80.0);
    assert!(!m.grounded());
    m.apply(&cmd(1100, 0, 0), 1.0, false, &c);
    assert!((0.0..=0.25).contains(&m.origin[2]), "{:?}", m);
    assert!(m.grounded());
}
#[test]
fn real_collision_landing_records_one_hit_and_expires_the_movement_timer() {
    let mut f = floor();
    f.plane_surface_flags = vec![3 << 20; 6];
    let c = world(vec![f], vec![]);
    let mut m = Motion::new([0.0, 0.0, 220.0]);
    m.command_time = 1000;
    let mut hits = Vec::new();
    for _ in 0..80 {
        m.apply(&cmd(m.command_time + 16, 0, 0), 1.0, false, &c);
        hits.extend(m.take_landings());
    }
    assert_eq!(hits.len(), 1);
    assert!((52..=54).contains(&hits[0].damage), "{:?}", hits);
    assert!(m.grounded());
    assert!(m.pm_time > 0);
    assert_ne!(m.flags & 0x80, 0);
    for _ in 0..150 {
        m.apply(&cmd(m.command_time + 16, 0, 0), 1.0, false, &c);
        assert!(m.take_landings().is_empty());
    }
    assert_eq!(m.pm_time, 0);
    assert_eq!(m.flags & 0x80, 0);
}
#[test]
fn jump_returns_to_real_ground_and_does_not_repeat_while_held() {
    let c = world(vec![floor()], vec![]);
    let mut m = Motion::new([0.0, 0.0, 2.0]);
    tick(&mut m, &c, 0, 0, 40);
    let base = m.origin[2];
    tick(&mut m, &c, 0, 0x400, 12);
    assert!(m.origin[2] > base + 20.0, "{:?}", m.origin);
    tick(&mut m, &c, 0, 0x400, 100);
    assert!(m.grounded());
    // GroundTrace classifies contact within 0.25 units without snapping
    // the origin. Check that actual tolerance, rather than requiring the
    // old second integration to force exactly the previous resting point.
    assert!((0.0..=0.25).contains(&m.origin[2]), "{m:?}");
    assert!(
        (m.origin[2] - base).abs() <= 0.25,
        "base={base}, motion={m:?}"
    );
}
#[test]
fn crouch_cannot_stand_through_a_ceiling() {
    let c = world(
        vec![floor(), brush([0.0, 0.0, 65.0], [80.0, 80.0, 5.0])],
        vec![],
    );
    let mut m = Motion::new([0.0, 0.0, 1.0]);
    tick(&mut m, &c, 0, 0x200, 40);
    assert_eq!(m.stance, Stance::Crouch);
    tick(&mut m, &c, 0, 0, 40);
    assert_eq!(m.stance, Stance::Crouch);
    assert_eq!(m.view_height, 40.0);
    tick(&mut m, &c, 1, 0x200, 100);
    tick(&mut m, &c, 0, 0, 40);
    assert_eq!(m.stance, Stance::Stand);
}
#[test]
fn can_step_over_a_twelve_unit_obstacle() {
    let c = world(
        vec![floor(), brush([80.0, 0.0, 6.0], [15.0, 80.0, 6.0])],
        vec![],
    );
    let mut m = Motion::new([0.0, 0.0, 1.0]);
    tick(&mut m, &c, 1, 0, 100);
    assert!(m.origin[0] > 150.0, "{:?}", m.origin);
}
#[test]
fn triangle_slope_supports_a_falling_hull() {
    let c = world(
        vec![],
        vec![TriangleExport {
            vertices: [
                [-500.0, -500.0, -125.0],
                [500.0, -500.0, 125.0],
                [0.0, 500.0, 0.0],
            ],
            contents: 1,
            surface_flags: 7,
        }],
    );
    let mut m = Motion::new([0.0, 0.0, 100.0]);
    tick(&mut m, &c, 0, 0, 100);
    assert!(m.grounded(), "{:?}", m);
    let slope_offset =
        15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0) + 0.125 * (1.0f32 + 0.25 * 0.25).sqrt();
    assert!(
        (m.origin[2] - m.origin[0] * 0.25 - slope_offset).abs() < 0.1,
        "{:?}",
        m.origin
    );
    assert_eq!(m.surface_flags, 7);
    tick(&mut m, &c, 1, 0, 40);
    assert!(m.origin[0] > 20.0);
    assert!(m.origin[2] > 8.0);
}
#[test]
fn changing_slope_uses_the_new_ground_normal() {
    let c = world(
        vec![],
        vec![TriangleExport {
            vertices: [
                [-500.0, -500.0, -125.0],
                [500.0, -500.0, 125.0],
                [0.0, 500.0, 0.0],
            ],
            contents: 1,
            surface_flags: 0,
        }],
    );
    let slope_offset = 15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0);
    let mut m = Motion::new([0.0, 0.0, slope_offset + 0.05]);
    m.velocity = [184.0, 0.0, 46.0];
    m.ground_entity = WORLD;
    m.ground(&c, 0.25);
    assert!(m.grounded());
    assert!(m.velocity[1].abs() < 0.001);
}
#[test]
fn commands_are_deduplicated_and_air_does_not_invent_a_floor() {
    let c = world(vec![brush([1000.0, 1000.0, -20.0], [1.0; 3])], vec![]);
    let mut m = Motion::new([0.0, 0.0, 20.0]);
    m.apply(&cmd(66, 1, 0), 1.0, false, &c);
    let once = m.origin;
    m.apply(&cmd(66, 1, 0), 1.0, false, &c);
    assert_eq!(m.origin, once);
    tick(&mut m, &c, 0, 0, 100);
    assert!(m.origin[2] < -100.0);
    assert!(!m.grounded());
}
#[test]
fn continuous_sweep_blocks_fast_motion_and_respects_slanted_brush_plane() {
    let mut b = brush([100.0, 0.0, 50.0], [100.0, 100.0, 50.0]);
    let s = 2f32.sqrt().recip();
    b.planes.push([-s, 0.0, s, 0.0]);
    let c = world(vec![b], vec![]);
    let trace = c.trace([-100.0, 0.0, 20.0], [1000.0, 0.0, 20.0], 70.0);
    assert!(trace.fraction < 0.2);
    assert!(
        trace.normal[0] < -0.5 && trace.normal[2] > 0.5,
        "{:?}",
        trace
    );
}
#[test]
fn capsule_support_uses_a_round_cap_instead_of_box_corners() {
    let normal = normalized([-0.25, 0.0, 1.0]);
    for height in [70.0, 50.0, 30.0] {
        let capsule = Capsule::new(height);
        let expected = 15.0 + (height * 0.5 - 15.0) * normal[2];
        assert!((capsule.support(normal) - expected).abs() < 0.00001);
        let foot_offset = capsule.support(normal) / normal[2] - height * 0.5;
        let expected_offset = 15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0);
        assert!((foot_offset - expected_offset).abs() < 0.00001);
    }
}
#[test]
fn capsule_brush_slope_uses_the_same_round_cap_as_terrain() {
    let normal = normalized([-0.25, 0.0, 1.0]);
    let planes = [[normal[0], normal[1], normal[2], 0.0]];
    let capsule = Capsule::new(70.0);
    let trace = sweep_brush(&planes, &[], [0.0, 0.0, 45.0], [0.0, 0.0, 25.0], capsule);
    let foot = 10.0 - 20.0 * trace.fraction;
    let expected = 15.0 * ((1.0f32 + 0.25 * 0.25).sqrt() - 1.0) + SKIN / normal[2];
    assert!((foot - expected).abs() < 0.0001, "{trace:?}, foot={foot}");
}
#[test]
fn capsule_terrain_sweep_covers_edges_vertices_and_cylinder() {
    let vertices = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
    let capsule = Capsule::new(70.0);
    let edge = sweep_triangle(vertices, [50.0, -100.0, 20.0], [50.0, 100.0, 20.0], capsule);
    assert!(
        (edge.fraction - (100.0 - 15.0 - SKIN) / 200.0).abs() < 0.00001,
        "{edge:?}"
    );
    assert!(edge.normal[1] < -0.999, "{edge:?}");
    let vertex = sweep_triangle(
        vertices,
        [-100.0, -100.0, 20.0],
        [100.0, 100.0, 20.0],
        capsule,
    );
    let expected = (100.0 - (15.0 + SKIN) / 2.0f32.sqrt()) / 200.0;
    assert!((vertex.fraction - expected).abs() < 0.00001, "{vertex:?}");
    assert!(
        vertex.normal[0] < -0.7 && vertex.normal[1] < -0.7,
        "{vertex:?}"
    );
    // A box corner overlaps this triangle, but the round capsule does not.
    let clear = sweep_triangle(
        vertices,
        [-14.0, -14.0, 20.0],
        [-14.0, -14.0, 20.0],
        capsule,
    );
    assert!(!clear.start_solid && clear.fraction == 1.0, "{clear:?}");
    // Contact halfway along the vertical segment is outside either end
    // sphere, proving that the cylinder is included in position tests.
    let upright = [[0.0, 0.0, 30.0], [0.0, 0.0, 40.0], [0.0, 1.0, 35.0]];
    let center = [-14.0, 0.0, 20.0];
    let solid = sweep_triangle(upright, center, center, capsule);
    assert!(solid.start_solid && solid.all_solid, "{solid:?}");
}
#[test]
fn capsule_terrain_sweep_does_not_tunnel_at_high_speed() {
    let vertices = [
        [-1000.0, -1000.0, 0.0],
        [1000.0, -1000.0, 0.0],
        [0.0, 1000.0, 0.0],
    ];
    let capsule = Capsule::new(70.0);
    let face = sweep_triangle(vertices, [0.0, 0.0, 10035.0], [0.0, 0.0, -9965.0], capsule);
    assert!(
        (face.fraction - (10000.0 - SKIN) / 20000.0).abs() < 0.000001,
        "{face:?}"
    );
    assert!(face.normal[2] > 0.999, "{face:?}");
    let vertices = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
    let edge = sweep_triangle(
        vertices,
        [50.0, -10000.0, 20.0],
        [50.0, 10000.0, 20.0],
        capsule,
    );
    assert!(
        (edge.fraction - (10000.0 - 15.0 - SKIN) / 20000.0).abs() < 0.000001,
        "{edge:?}"
    );
    assert!(edge.normal[1] < -0.999, "{edge:?}");
}
#[test]
fn capsule_does_not_drift_at_rest_or_catch_a_coplanar_terrain_seam() {
    let vertices = [
        [-1000.0, -1000.0, -250.0],
        [1000.0, -1000.0, 250.0],
        [1000.0, 1000.0, 250.0],
        [-1000.0, 1000.0, -250.0],
    ];
    let c = world(
        vec![],
        vec![
            TriangleExport {
                vertices: [vertices[0], vertices[1], vertices[2]],
                contents: 1,
                surface_flags: 0,
            },
            TriangleExport {
                vertices: [vertices[0], vertices[2], vertices[3]],
                contents: 1,
                surface_flags: 0,
            },
        ],
    );
    let mut m = Motion::new([-100.0, -50.0, 100.0]);
    tick(&mut m, &c, 0, 0, 150);
    assert!(m.grounded(), "{m:?}");
    let resting = m.origin;
    tick(&mut m, &c, 0, 0, 600);
    assert_eq!(m.origin, resting, "{m:?}");
    for _ in 0..100 {
        let previous = m.origin;
        tick(&mut m, &c, 1, 0, 1);
        assert!(m.grounded(), "{m:?}");
        assert!(m.origin[0] >= previous[0], "{m:?}");
    }
    assert!(m.origin[0] > 100.0, "{m:?}");
    tick(&mut m, &c, 0, 0, 100);
    let resting = m.origin;
    tick(&mut m, &c, 0, 0, 600);
    assert_eq!(m.origin, resting, "{m:?}");
}
#[test]
fn raycast_returns_the_nearest_world_brush_without_player_radius_or_skin() {
    let mut brushes = (0..12)
        .map(|i| brush([500.0 + i as f32 * 100.0, 0.0, 50.0], [10.0, 10.0, 50.0]))
        .collect::<Vec<_>>();
    brushes.push(brush([100.0, 0.0, 50.0], [10.0, 10.0, 50.0]));
    let c = world(brushes, vec![]);
    let hit = c.raycast([0.0, 0.0, 30.0], [2000.0, 0.0, 30.0]);
    assert!((hit.fraction - 90.0 / 2000.0).abs() < 1e-7, "{hit:?}");
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
    assert_eq!(hit.contents, 1);
    assert!(!hit.start_solid && !hit.all_solid);
    let clear = c.raycast([0.0, 11.0, 30.0], [200.0, 11.0, 30.0]);
    assert_eq!(clear.fraction, 1.0, "{clear:?}");
    assert_eq!(clear.contents, 0);
    assert!(c.trace([0.0, 11.0, 0.0], [200.0, 11.0, 0.0], 70.0).fraction < 1.0);
    let short = c.raycast([0.0, 0.0, 30.0], [89.0, 0.0, 30.0]);
    assert_eq!(short.contents, 0, "{short:?}");
    let endpoint = c.raycast([0.0, 0.0, 30.0], [90.0, 0.0, 30.0]);
    assert_eq!(endpoint.fraction, 1.0);
    assert_eq!(endpoint.contents, 1, "{endpoint:?}");
}
#[test]
fn raycast_brush_start_solid_blocks_a_shot_even_when_its_endpoint_escapes() {
    let c = world(vec![brush([0.0; 3], [10.0; 3])], vec![]);
    let escape = c.raycast([0.0; 3], [100.0, 0.0, 0.0]);
    assert_eq!(escape.fraction, 0.0);
    assert!(escape.start_solid && !escape.all_solid, "{escape:?}");
    assert_eq!(escape.contents, 1);
    let inside = c.raycast([0.0; 3], [5.0, 0.0, 0.0]);
    assert!(inside.start_solid && inside.all_solid, "{inside:?}");
    assert_eq!(inside.fraction, 0.0);
    let stationary = c.raycast([0.0; 3], [0.0; 3]);
    assert!(stationary.start_solid && stationary.all_solid);
}
#[test]
fn raycast_terrain_is_double_sided_and_preserves_surface_metadata() {
    let c = world(
        vec![],
        vec![TriangleExport {
            vertices: [[0.0, 0.0, 10.0], [100.0, 0.0, 10.0], [0.0, 100.0, 10.0]],
            contents: 1,
            surface_flags: 0x0050_0100,
        }],
    );
    let front = c.raycast([25.0, 25.0, 100.0], [25.0, 25.0, -100.0]);
    assert!((front.fraction - 0.45).abs() < 1e-7, "{front:?}");
    assert_eq!(front.normal, [0.0, 0.0, 1.0]);
    assert_eq!(front.surface_flags, 0x0050_0100);
    assert_eq!(front.contents, 1);
    assert!(!front.start_solid && !front.all_solid);
    let back = c.raycast([25.0, 25.0, -100.0], [25.0, 25.0, 100.0]);
    assert!((back.fraction - 0.55).abs() < 1e-7, "{back:?}");
    assert_eq!(back.normal, [0.0, 0.0, -1.0]);
    let parallel = c.raycast([25.0, 25.0, 11.0], [35.0, 25.0, 11.0]);
    assert_eq!(parallel.contents, 0, "{parallel:?}");
    let beyond_face = c.raycast([60.0, 60.0, 20.0], [60.0, 60.0, 0.0]);
    assert_eq!(beyond_face.contents, 0, "{beyond_face:?}");
}
#[test]
fn raycast_terrain_includes_edges_vertices_and_coplanar_entry() {
    let c = world(
        vec![],
        vec![TriangleExport {
            vertices: [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]],
            contents: 1,
            surface_flags: 7,
        }],
    );
    for point in [[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [100.0, 0.0]] {
        let hit = c.raycast([point[0], point[1], 10.0], [point[0], point[1], -10.0]);
        assert_eq!(hit.fraction, 0.5, "point={point:?}, {hit:?}");
        assert_eq!(hit.contents, 1);
    }
    let coplanar = c.raycast([-25.0, 25.0, 0.0], [75.0, 25.0, 0.0]);
    assert_eq!(coplanar.fraction, 0.25, "{coplanar:?}");
    assert_eq!(coplanar.contents, 1);
    let stationary = c.raycast([25.0, 25.0, 0.0], [25.0, 25.0, 0.0]);
    assert_eq!(stationary.fraction, 0.0);
    assert_eq!(stationary.contents, 1);
    assert!(!stationary.start_solid && !stationary.all_solid);
    let outside = c.raycast([60.0, 60.0, 0.0], [70.0, 70.0, 0.0]);
    assert_eq!(outside.contents, 0, "{outside:?}");
}
#[test]
fn raycast_uses_shot_contents_while_player_sweeps_keep_their_own_mask() {
    // Exercise each shot-only category separately: otherwise the older
    // C1 mask's missing 0x8000 bit is hidden by an accompanying clipshot bit.
    for shot_contents in [0x0000_2000, 0x0000_8000] {
        let mut player_clip = brush([20.0, 0.0, 35.0], [5.0, 100.0, 35.0]);
        player_clip.contents = 0x0001_0000;
        let mut shot_clip = brush([100.0, 0.0, 35.0], [5.0, 100.0, 35.0]);
        shot_clip.contents = shot_contents;
        let c = world(vec![player_clip, shot_clip], vec![]);
        let hit = c.raycast([0.0, 0.0, 30.0], [200.0, 0.0, 30.0]);
        assert!((hit.fraction - 95.0 / 200.0).abs() < 1e-7, "{hit:?}");
        assert_eq!(hit.contents, shot_contents);
        let player = c.trace([50.0, 0.0, 0.0], [200.0, 0.0, 0.0], 70.0);
        assert_eq!(player.fraction, 1.0, "{player:?}");
        let player = c.trace([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0], 70.0);
        assert!(player.fraction < 1.0, "{player:?}");
        assert_eq!(player.contents, 0x0001_0000);
    }
}

#[test]
fn brush_material_comes_from_the_actual_entry_face() {
    let mut wall = brush([100.0, 0.0, 30.0], [10.0, 10.0, 10.0]);
    wall.plane_surface_flags = vec![1 << 20, 3 << 20, 11 << 20, 12 << 20, 18 << 20, 23 << 20];
    let c = world(vec![wall], vec![]);
    for (start, end, surface) in [
        ([0.0, 0.0, 30.0], [200.0, 0.0, 30.0], 3),
        ([200.0, 0.0, 30.0], [0.0, 0.0, 30.0], 1),
        ([100.0, 0.0, 100.0], [100.0, 0.0, 0.0], 18),
    ] {
        assert_eq!(c.raycast(start, end).surface_flags, surface << 20);
    }
    let sweep = c.trace([0.0, 0.0, 30.0], [200.0, 0.0, 30.0], 20.0);
    assert_eq!(sweep.surface_flags, 3 << 20);
    let mut slope = brush([0.0; 3], [100.0; 3]);
    slope.planes = vec![[0.70710677, 0.0, 0.70710677, 0.0]];
    slope.plane_surface_flags = vec![7 << 20, 0, 0, 0, 0, 0, 0];
    let c = world(vec![slope], vec![]);
    assert_eq!(
        c.raycast([0.0, 0.0, 80.0], [0.0, 0.0, -80.0]).surface_flags,
        7 << 20
    );
}

#[test]
fn native_capsule_contact_retains_the_recorded_eighth_unit_margin() {
    let c = world(vec![brush([0.0, 0.0, -50.0], [500.0, 500.0, 50.0])], vec![]);
    let tr = c.trace([0.0, 0.0, 50.0], [0.0, 0.0, -50.0], 70.0);
    let contact_z = 50.0 - 100.0 * tr.fraction;
    assert!((contact_z - 0.125).abs() < 0.00001);
}
