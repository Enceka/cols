//! Basic server-owned training AI using the recovered movement and world traces.
//!
//! Reaction, spacing and burst pacing are explicit gameplay design choices,
//! rather than claims about the original bot AI. Weapon identity, damage,
//! distance band, cadence and locomotion speed come from captured resources.
use anyhow::{Result, ensure};
use codol_protocol::game_usercmd::UserCommand;

use crate::{
    game_combat::Ballistics,
    game_motion::{Collision, Motion},
};

const HALF_CLOCK: u32 = 0x8000_0000;
const REACTION_MSEC: u32 = 350;
const BURST_SHOTS: u8 = 3;
const BURST_REST_MSEC: u32 = 750;
const STANDOFF_UNITS: f32 = 160.0;
const PHYSICS_BUDGET_MSEC: u32 = 200;
const PHYSICS_CHOP_MSEC: u32 = 16;

/// Ground an approximate feet position using the same standing capsule as
/// movement. A point/model-bounds trace can leave that capsule inside sloping
/// terrain. Repair only within16 units upward, then sweep down at most256;
/// deeply embedded positions, ceilings and non-walkable floors are rejected.
/// Keep the returned coordinates at full precision: rounding Z downward can
/// recreate the penetration this sweep removed.
pub fn grounded_spawn(origin: [f32; 3], collision: &Collision) -> Result<[f32; 3]> {
    ensure!(valid_position(origin), "invalid bot spawn candidate");
    let height = crate::game_motion::Stance::Stand.height();
    let end = [origin[0], origin[1], (origin[2] - 256.0).max(-32768.0)];
    for lift in [0.0, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0] {
        let start = [origin[0], origin[1], origin[2] + lift];
        if !valid_position(start) {
            continue;
        }
        let hull = collision.trace(start, start, height);
        if hull.start_solid || hull.all_solid {
            continue;
        }
        let floor = collision.trace(start, end, height);
        if floor.start_solid
            || floor.all_solid
            || floor.fraction >= 1.0
            || floor.contents == 0
            || floor.normal[2] < 0.7
        {
            continue;
        }
        let grounded = [
            start[0],
            start[1],
            start[2] + (end[2] - start[2]) * floor.fraction,
        ];
        let hull = collision.trace(grounded, grounded, height);
        if valid_position(grounded) && !hull.start_solid && !hull.all_solid {
            return Ok(grounded);
        }
    }
    anyhow::bail!("no clear walkable standing capsule near bot spawn candidate")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BotPhase {
    #[default]
    Idle,
    Chase,
    Attack,
    Dead,
}

#[derive(Clone, Copy, Debug)]
pub struct BotTarget {
    pub origin: [f32; 3],
    pub eye: [f32; 3],
    pub alive: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct BotWeapon {
    pub weapon: u16,
    pub ballistics: Ballistics,
    pub fire_interval_msec: u32,
}

impl BotWeapon {
    fn valid(self) -> bool {
        (1..900).contains(&self.weapon)
            && self.ballistics.valid()
            && (1..=60_000).contains(&self.fire_interval_msec)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BotShot {
    pub time: u32,
    pub eye: [f32; 3],
    pub angles: [u16; 3],
    pub weapon: u16,
    pub ballistics: Ballistics,
    /// Lifecycle token used to reject a held shot after death or respawn.
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub struct BotFrame {
    pub motion: Motion,
    pub angles: [u16; 3],
    pub phase: BotPhase,
    pub shot: Option<BotShot>,
    pub locomotion: Locomotion,
}

impl BotFrame {
    pub fn moving(&self) -> bool {
        self.motion.velocity[0].hypot(self.motion.velocity[1]) > 8.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Locomotion {
    #[default]
    Walk,
    Run,
    Sprint,
}

#[derive(Debug)]
pub struct BotController {
    motion: Motion,
    angles: [u16; 3],
    phase: BotPhase,
    move_speed: f32,
    last_tick: Option<u32>,
    life_start: Option<u32>,
    seen_since: Option<u32>,
    last_shot: Option<u32>,
    next_shot: Option<u32>,
    burst_left: u8,
    equipped_weapon: Option<u16>,
    generation: u64,
    engagement: Option<(f32, f32)>,
    physics_chop_msec: u32,
    match_move_scale: f32,
    locomotion: Locomotion,
}

impl BotController {
    pub fn new(origin: [f32; 3], angles: [u16; 3], move_speed: f32) -> Result<Self> {
        ensure!(valid_position(origin), "invalid bot origin");
        ensure!(
            move_speed.is_finite() && (20.0..=300.0).contains(&move_speed),
            "invalid captured bot move speed"
        );
        let mut motion = Motion::new(origin);
        motion.speed = move_speed.round() as u16;
        Ok(Self {
            motion,
            angles,
            phase: BotPhase::Idle,
            move_speed,
            last_tick: None,
            life_start: None,
            seen_since: None,
            last_shot: None,
            next_shot: None,
            burst_left: 0,
            equipped_weapon: None,
            generation: 1,
            engagement: None,
            physics_chop_msec: PHYSICS_CHOP_MSEC,
            match_move_scale: 1.0,
            locomotion: Locomotion::Walk,
        })
    }

    fn frame(&self, now: u32, shot: Option<BotShot>) -> BotFrame {
        let mut motion = self.motion.clone();
        // The public frame belongs to the authority clock. The private motion
        // uses a small monotonic physics clock because Motion::apply currently
        // compares ordinary u32 input times and cannot cross a clock wrap.
        motion.command_time = now;
        BotFrame {
            motion,
            angles: self.angles,
            phase: self.phase,
            shot,
            locomotion: self.locomotion,
        }
    }
    pub fn set_match_engagement(&mut self) {
        self.engagement = Some((1500.0, 800.0));
        // Motion itself supports the native66ms maximum; a50ms match tick
        // needs only one capsule movement step instead of four16ms steps.
        self.physics_chop_msec = 50;
        self.motion.speed = crate::game_motion::SPEED;
    }
    pub fn set_match_move_scale(&mut self, scale: f32) {
        self.match_move_scale = scale;
    }

    fn reset_engagement(&mut self) {
        self.seen_since = None;
        self.next_shot = None;
        self.burst_left = 0;
    }

    /// Only strictly newer authority ticks can move or authorise a shot.
    /// Large gaps do not create a catch-up volley or an unbounded movement run.
    pub fn step(
        &mut self,
        now: u32,
        target: Option<BotTarget>,
        weapon: BotWeapon,
        collision: &Collision,
    ) -> BotFrame {
        self.step_navigated(now, target, None, weapon, collision)
    }
    pub fn step_navigated(
        &mut self,
        now: u32,
        target: Option<BotTarget>,
        waypoint: Option<[f32; 3]>,
        weapon: BotWeapon,
        collision: &Collision,
    ) -> BotFrame {
        let delta = match self.last_tick {
            Some(previous) => {
                let elapsed = now.wrapping_sub(previous);
                if elapsed == 0 || elapsed >= HALF_CLOCK {
                    return self.frame(previous, None);
                }
                elapsed.min(PHYSICS_BUDGET_MSEC)
            }
            None => 0,
        };
        self.last_tick = Some(now);
        self.life_start.get_or_insert(now);
        if self.phase == BotPhase::Dead {
            return self.frame(now, None);
        }
        let target =
            target.filter(|t| t.alive && valid_position(t.origin) && valid_position(t.eye));
        if self.equipped_weapon != Some(weapon.weapon) {
            self.equipped_weapon = Some(weapon.weapon);
            self.last_shot = None;
            self.reset_engagement();
        }
        let weapon_valid = weapon.valid();
        let spacing = if let Some((_, spacing)) = self.engagement {
            spacing
        } else if weapon_valid {
            STANDOFF_UNITS
                .min(weapon.ballistics.near_range * 0.75)
                .max(32.0)
        } else {
            STANDOFF_UNITS
        };

        let visible_before = self.engagement.is_some()
            && target.is_some_and(|t| {
                let trace = collision.raycast(self.eye(), t.eye);
                !trace.start_solid && !trace.all_solid && trace.contents == 0
            });
        let hold = visible_before
            && target.is_some_and(|t| horizontal_distance(self.motion.origin, t.origin) <= spacing);
        let mode = if self.engagement.is_none()
            || (visible_before
                && target
                    .is_some_and(|t| distance(self.eye(), t.eye) <= self.engagement.unwrap().0))
        {
            Locomotion::Walk
        } else if target.is_some_and(|t| {
            horizontal_distance(self.motion.origin, t.origin) > self.engagement.unwrap().0
        }) {
            Locomotion::Sprint
        } else {
            Locomotion::Run
        };
        self.locomotion = mode;
        let travel = if hold {
            None
        } else {
            waypoint
                .map(|origin| BotTarget {
                    origin,
                    eye: [origin[0], origin[1], origin[2] + 60.0],
                    alive: true,
                })
                .or(target)
        };
        self.move_toward(
            delta,
            travel,
            if waypoint.is_some() { 24.0 } else { spacing },
            collision,
            mode,
        );
        let Some(target) = target else {
            self.phase = BotPhase::Idle;
            self.reset_engagement();
            return self.frame(now, None);
        };
        let eye = self.eye();
        self.angles = if self.engagement.is_some() && mode != Locomotion::Walk {
            travel.map_or(self.angles, |t| aim_angles(eye, t.eye))
        } else {
            aim_angles(eye, target.eye)
        };
        let distance = distance(eye, target.eye);
        let trace = collision.raycast(eye, target.eye);
        let visible = !trace.start_solid && !trace.all_solid && trace.contents == 0;
        if !visible || !weapon_valid {
            self.reset_engagement();
        } else {
            self.seen_since.get_or_insert(now);
        }
        let separation = horizontal_distance(self.motion.origin, target.origin);
        if (self.engagement.is_none() && separation > spacing + 1.0) || !visible || !weapon_valid {
            self.phase = BotPhase::Chase;
            return self.frame(now, None);
        }
        if distance
            > self
                .engagement
                .map_or(weapon.ballistics.near_range, |e| e.0)
            || distance > weapon.ballistics.range
        {
            self.phase = BotPhase::Idle;
            return self.frame(now, None);
        }
        self.phase = BotPhase::Attack;
        let since = self.seen_since.unwrap_or(now);
        if now.wrapping_sub(since) < REACTION_MSEC {
            return self.frame(now, None);
        }
        if self.engagement.is_some() && mode != Locomotion::Walk {
            return self.frame(now, None);
        }
        if self
            .next_shot
            .is_some_and(|deadline| !reached(now, deadline))
        {
            return self.frame(now, None);
        }
        if self
            .last_shot
            .is_some_and(|previous| now.wrapping_sub(previous) < weapon.fire_interval_msec)
        {
            return self.frame(now, None);
        }
        if self.burst_left == 0 {
            self.burst_left = BURST_SHOTS;
        }
        self.burst_left -= 1;
        self.last_shot = Some(now);
        let wait = if self.burst_left == 0 {
            BURST_REST_MSEC.max(weapon.fire_interval_msec)
        } else {
            weapon.fire_interval_msec
        };
        self.next_shot = Some(now.wrapping_add(wait));
        let shot = BotShot {
            time: now,
            eye,
            angles: self.angles,
            weapon: weapon.weapon,
            ballistics: weapon.ballistics,
            generation: self.generation,
        };
        self.frame(now, Some(shot))
    }

    fn eye(&self) -> [f32; 3] {
        [
            self.motion.origin[0],
            self.motion.origin[1],
            self.motion.origin[2] + self.motion.view_height,
        ]
    }

    fn move_toward(
        &mut self,
        mut remaining: u32,
        target: Option<BotTarget>,
        spacing: f32,
        collision: &Collision,
        mode: Locomotion,
    ) {
        while remaining > 0 {
            let dt = remaining.min(self.physics_chop_msec);
            remaining -= dt;
            let moving =
                target.is_some_and(|t| horizontal_distance(self.motion.origin, t.origin) > spacing);
            let angles = target.map_or(self.angles, |t| aim_angles(self.eye(), t.eye));
            if !moving {
                // Prevent the last chase velocity from carrying through the
                // desired separation. Vertical gravity still uses real motion.
                self.motion.velocity[0] = 0.0;
                self.motion.velocity[1] = 0.0;
            }
            if self.motion.command_time > u32::MAX - dt {
                // This controller never jumps or changes stance; only these
                // otherwise dormant time fields need rebasing for its clock.
                self.motion.command_time = 0;
                self.motion.jump_time = 0;
                self.motion.view_height_lerp_time = 0;
            }
            let command = UserCommand {
                time: self.motion.command_time + dt,
                direction: u8::from(moving), // Recovered forward direction=1.
                angles,
                buttons: if mode == Locomotion::Sprint { 2 } else { 0 },
                ..UserCommand::default()
            };
            let speed_scale = if self.engagement.is_some() {
                self.match_move_scale
            } else {
                self.move_speed / f32::from(self.motion.speed)
            };
            self.motion.apply(
                &command,
                speed_scale,
                self.engagement.is_some() && mode == Locomotion::Walk,
                collision,
            );
        }
    }

    pub fn kill(&mut self, now: u32) -> bool {
        if self.phase == BotPhase::Dead
            || self
                .life_start
                .is_some_and(|birth| now.wrapping_sub(birth) >= HALF_CLOCK)
        {
            return false;
        }
        let effective = match self.last_tick {
            Some(current) => match now.wrapping_sub(current) {
                HALF_CLOCK => return false,
                delta if delta < HALF_CLOCK => now,
                _ => current,
            },
            None => now,
        };
        self.last_tick = Some(effective);
        self.phase = BotPhase::Dead;
        self.motion.velocity = [0.0; 3];
        self.reset_engagement();
        self.generation = self.generation.wrapping_add(1);
        true
    }

    pub fn respawn(&mut self, now: u32, origin: [f32; 3], angles: [u16; 3]) -> Result<()> {
        ensure!(valid_position(origin), "invalid bot respawn origin");
        ensure!(
            self.last_tick
                .is_none_or(|previous| now.wrapping_sub(previous) < HALF_CLOCK),
            "bot respawn time goes backwards"
        );
        self.motion = Motion::new(origin);
        self.motion.speed = if self.engagement.is_some() {
            crate::game_motion::SPEED
        } else {
            self.move_speed.round() as u16
        };
        self.angles = angles;
        self.phase = BotPhase::Idle;
        self.last_tick = Some(now);
        self.life_start = Some(now);
        self.last_shot = None;
        self.reset_engagement();
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    /// Authorised shots are consumed in their issuing frame. Holding a shot
    /// across another tick, death or respawn never grants delayed damage.
    pub fn shot_is_current(&self, shot: &BotShot) -> bool {
        self.phase != BotPhase::Dead
            && shot.generation == self.generation
            && self.last_tick == Some(shot.time)
            && self.last_shot == Some(shot.time)
    }
    pub fn separate_from(&mut self, client: u8, peers: &[(u8, [f32; 3])], collision: &Collision) {
        let mut shift = [0.0f32; 2];
        for &(other, position) in peers {
            if other == client || (position[2] - self.motion.origin[2]).abs() > 40.0 {
                continue;
            }
            let dx = self.motion.origin[0] - position[0];
            let dy = self.motion.origin[1] - position[1];
            let distance = dx.hypot(dy);
            if distance >= 32.0 {
                continue;
            }
            if distance < 0.01 {
                let angle = f32::from(client) * 2.39996;
                shift[0] += angle.cos() * 3.0;
                shift[1] += angle.sin() * 3.0;
            } else {
                shift[0] += dx / distance * (32.0 - distance) * 0.15;
                shift[1] += dy / distance * (32.0 - distance) * 0.15;
            }
        }
        let length = shift[0].hypot(shift[1]);
        if length > 0.0 {
            let scale = (4.0 / length).min(1.0);
            let end = [
                self.motion.origin[0] + shift[0] * scale,
                self.motion.origin[1] + shift[1] * scale,
                self.motion.origin[2],
            ];
            let trace = collision.trace(self.motion.origin, end, 70.0);
            if !trace.start_solid && !trace.all_solid {
                for (i, value) in end.into_iter().enumerate() {
                    self.motion.origin[i] += (value - self.motion.origin[i]) * trace.fraction;
                }
            }
        }
    }
}

fn reached(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) < HALF_CLOCK
}

fn valid_position(position: [f32; 3]) -> bool {
    position
        .iter()
        .all(|n| n.is_finite() && (-32768.0..=32767.0).contains(n))
}

fn horizontal_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    horizontal_distance(a, b).hypot(a[2] - b[2])
}

fn aim_angles(eye: [f32; 3], target: [f32; 3]) -> [u16; 3] {
    let delta = std::array::from_fn::<_, 3, _>(|i| f64::from(target[i]) - f64::from(eye[i]));
    let yaw = delta[1].atan2(delta[0]);
    let pitch = (-delta[2]).atan2(delta[0].hypot(delta[1]));
    let short = |radians: f64| {
        ((radians * (65536.0 / std::f64::consts::TAU)).round() as i64 & 0xffff) as u16
    };
    [short(pitch), short(yaw), 0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn collision(wall: bool) -> Collision {
        let mut brushes = vec![json!({
            "center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]
        })];
        if wall {
            brushes.push(json!({
                "center":[200,0,100],"half":[5,1000,100],"contents":1,"planes":[]
            }));
        }
        let export = json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        Collision::load(file.path()).unwrap()
    }

    fn weapon() -> BotWeapon {
        BotWeapon {
            weapon: 352,
            ballistics: Ballistics {
                damage: 33,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 0,
                spread_degrees: 0.0,
            },
            fire_interval_msec: 80,
        }
    }
    #[test]
    fn match_bot_engages_a_visible_rifle_target_beyond_the_training_distance() {
        let open_world = collision(false);
        let mut bot = BotController::new([0.0; 3], [0; 3], 190.0).unwrap();
        bot.set_match_engagement();
        let target = BotTarget {
            origin: [1000.0, 0.0, 0.0],
            eye: [1000.0, 0.0, 60.0],
            alive: true,
        };
        let mut shots = 0;
        for time in (1000..2000).step_by(50) {
            let frame = bot.step(time, Some(target), weapon(), &open_world);
            if frame.shot.is_some() {
                shots += 1;
            }
        }
        assert!(shots > 0);
        assert_eq!(bot.phase, BotPhase::Attack);
        let blocked = collision(true);
        let mut bot = BotController::new([0.0; 3], [0; 3], 190.0).unwrap();
        bot.set_match_engagement();
        for time in (1000..2000).step_by(50) {
            assert!(
                bot.step(time, Some(target), weapon(), &blocked)
                    .shot
                    .is_none()
            );
        }
    }
    #[test]
    fn native_movement_runs_and_sprints_with_weapon_scale_then_walks_when_engaging() {
        let world = collision(true);
        let mut bot = BotController::new([0.0; 3], [0; 3], 94.0).unwrap();
        bot.set_match_engagement();
        bot.set_match_move_scale(0.9);
        let enemy = target(2500.0);
        let mut last = None;
        for time in (1000..2000).step_by(50) {
            let frame = bot.step_navigated(
                time,
                Some(enemy),
                Some([0.0, 1500.0, 0.0]),
                weapon(),
                &world,
            );
            assert!(frame.shot.is_none());
            last = Some(frame);
        }
        let frame = last.unwrap();
        assert_eq!(frame.motion.speed, crate::game_motion::SPEED);
        assert_ne!(frame.motion.flags & 0x4000, 0);
        assert!((frame.motion.velocity[0].hypot(frame.motion.velocity[1]) - 256.5).abs() < 1.0);
        assert_eq!(frame.angles[1], 16384); // Faces the route rather than an unseen enemy.

        let mut run = BotController::new([0.0; 3], [0; 3], 94.0).unwrap();
        run.set_match_engagement();
        let frame = run.step_navigated(
            1000,
            Some(target(1200.0)),
            Some([0.0, 1500.0, 0.0]),
            weapon(),
            &world,
        );
        assert_eq!(frame.motion.flags & 0x4000, 0);
        let open_world = collision(false);
        let mut walk = BotController::new([0.0; 3], [0; 3], 94.0).unwrap();
        walk.set_match_engagement();
        let mut last = None;
        for time in (1000..2000).step_by(50) {
            last = Some(walk.step(time, Some(target(1000.0)), weapon(), &open_world));
        }
        let frame = last.unwrap();
        assert_eq!(frame.locomotion, Locomotion::Walk);
        assert_eq!(frame.motion.flags & 0x4000, 0);
        assert!(frame.motion.velocity[0].hypot(frame.motion.velocity[1]) <= 76.01);
    }

    fn target(x: f32) -> BotTarget {
        BotTarget {
            origin: [x, 0.0, 0.03125],
            eye: [x, 0.0, 60.03125],
            alive: true,
        }
    }

    fn bot() -> BotController {
        BotController::new([0.0, 0.0, 0.03125], [0; 3], 100.0).unwrap()
    }

    #[test]
    fn capsule_spawn_repairs_the_captured_afghan_slope_without_rounding_back_inside() {
        // Five terrain triangles next to the real stuck spawn [-172,-800,-76],
        // extracted from collision-1791435250791.json, SHA256
        // 82ebedfa5275070f3cd18dcc98cd1d222162c77efac358c00582a92eaef71468.
        // Point grounding cleared this position while the standing hull stayed
        // startSolid/allSolid, preventing every chase movement step.
        let vertices = [
            [
                [-94.0, -794.0, -58.040737],
                [-166.0, -774.0, -79.060585],
                [-120.0, -748.0, -66.35576],
            ],
            [
                [-226.0, -866.0, -82.5224],
                [-204.0, -794.0, -89.19962],
                [-182.0, -844.0, -72.27774],
            ],
            [
                [-182.0, -844.0, -72.27774],
                [-166.0, -774.0, -79.060585],
                [-140.0, -820.0, -65.32214],
            ],
            [
                [-166.0, -774.0, -79.060585],
                [-182.0, -844.0, -72.27774],
                [-204.0, -794.0, -89.19962],
            ],
            [
                [-166.0, -774.0, -79.060585],
                [-94.0, -794.0, -58.040737],
                [-140.0, -820.0, -65.32214],
            ],
        ];
        let triangles: Vec<_> = vertices
            .into_iter()
            .map(|vertices| json!({"vertices":vertices,"contents":1,"surface_flags":18874368}))
            .collect();
        let export = json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/mp_afghan.d3dbsp","ncs_crc":523897474,"brushes":[],"triangles":triangles
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let world = Collision::load(file.path()).unwrap();
        let candidate = [-172.0, -800.0, -76.0];
        assert!(!world.raycast(candidate, candidate).start_solid);
        let stuck = world.trace(candidate, candidate, 70.0);
        assert!(stuck.start_solid && stuck.all_solid);
        let repaired = grounded_spawn(candidate, &world).unwrap();
        assert_eq!(&repaired[..2], &candidate[..2]);
        assert!((repaired[2] + 75.87254).abs() < 0.001);
        let hull = world.trace(repaired, repaired, 70.0);
        assert!(!hull.start_solid && !hull.all_solid);
        let rounded = repaired.map(f32::round_ties_even);
        assert!(world.trace(rounded, rounded, 70.0).all_solid);
        let mut bot = BotController::new(repaired, [0, 32768, 0], 94.48253).unwrap();
        let target = Some(BotTarget {
            origin: [-472.0, -800.0, -141.0],
            eye: [-472.0, -800.0, -81.0],
            alive: true,
        });
        bot.step(1000, target, weapon(), &world);
        let mut frame = bot.step(1020, target, weapon(), &world);
        for now in (1040..=1200).step_by(20) {
            frame = bot.step(now, target, weapon(), &world);
        }
        assert!(frame.motion.origin[0] < repaired[0] - 3.0, "{frame:?}");
        assert!(frame.moving());
    }

    #[test]
    fn capsule_spawn_accepts_clear_ground_and_rejects_ceilings_walls_and_missing_floor() {
        let world = collision(true);
        let repaired = grounded_spawn([0.0, 0.0, -0.01], &world).unwrap();
        assert!((repaired[2] - 0.03125).abs() < 1e-5);
        assert!(!world.trace(repaired, repaired, 70.0).start_solid);
        assert!(grounded_spawn([200.0, 0.0, 0.0], &world).is_err());
        assert!(grounded_spawn([5000.0, 0.0, 0.0], &world).is_err());
        assert!(grounded_spawn([f32::NAN, 0.0, 0.0], &world).is_err());
        let export = json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"triangles":[],
            "brushes":[
                {"center":[0,0,-10],"half":[100,100,10],"contents":1,"planes":[]},
                {"center":[0,0,80],"half":[100,100,20],"contents":1,"planes":[]}
            ]
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let low_ceiling = Collision::load(file.path()).unwrap();
        assert!(grounded_spawn([0.0, 0.0, 0.0], &low_ceiling).is_err());
    }

    #[test]
    fn reaction_cadence_and_burst_rest_prevent_per_tick_or_duplicate_fire() {
        let world = collision(false);
        let mut bot = bot();
        let target = Some(target(100.0));
        assert!(bot.step(1000, target, weapon(), &world).shot.is_none());
        assert!(bot.step(1349, target, weapon(), &world).shot.is_none());
        let frame = bot.step(1350, target, weapon(), &world);
        let shot = frame.shot.unwrap();
        assert!(bot.shot_is_current(&shot));
        assert_eq!(shot.weapon, 352);
        assert_eq!(shot.ballistics.damage, 33);
        assert!(bot.step(1350, target, weapon(), &world).shot.is_none());
        assert!(bot.step(1340, target, weapon(), &world).shot.is_none());
        assert!(bot.step(1429, target, weapon(), &world).shot.is_none());
        assert!(bot.step(1430, target, weapon(), &world).shot.is_some());
        assert!(bot.step(1510, target, weapon(), &world).shot.is_some());
        assert!(bot.step(2000, target, weapon(), &world).shot.is_none());
        assert!(bot.step(2260, target, weapon(), &world).shot.is_some());
    }

    #[test]
    fn a_world_wall_blocks_both_chase_travel_and_shot_authorisation() {
        let world = collision(true);
        let mut bot = bot();
        let target = Some(target(400.0));
        let mut frame = bot.step(0, target, weapon(), &world);
        for now in (16..=6000).step_by(16) {
            frame = bot.step(now, target, weapon(), &world);
            assert!(frame.shot.is_none());
            assert!(frame.motion.origin[0] < 180.1);
        }
        assert!(frame.motion.origin[0] > 150.0); // It really reached the wall.
        assert_eq!(frame.phase, BotPhase::Chase);
        assert!(!frame.moving());
    }

    #[test]
    fn chase_uses_captured_speed_and_stops_without_overlapping_the_player() {
        let world = collision(false);
        let mut bot = bot();
        let target = Some(target(400.0));
        bot.step(0, target, weapon(), &world);
        let mut frame = bot.step(1000, target, weapon(), &world);
        // A long snapshot pause integrates at most the explicit physics budget.
        assert!(frame.motion.origin[0] < 25.0);
        for now in (1016..=7000).step_by(16) {
            frame = bot.step(now, target, weapon(), &world);
        }
        let separation = 400.0 - frame.motion.origin[0];
        assert!((155.0..=165.0).contains(&separation));
        assert_eq!(frame.phase, BotPhase::Attack);
        assert!(!frame.moving());
    }

    #[test]
    fn losing_sight_requires_a_new_reaction_delay_when_the_player_returns() {
        let world = collision(false);
        let mut bot = bot();
        let near = Some(target(100.0));
        bot.step(0, near, weapon(), &world);
        assert!(bot.step(350, near, weapon(), &world).shot.is_some());
        assert_eq!(bot.step(400, None, weapon(), &world).phase, BotPhase::Idle);
        assert!(bot.step(500, near, weapon(), &world).shot.is_none());
        assert!(bot.step(849, near, weapon(), &world).shot.is_none());
        assert!(bot.step(850, near, weapon(), &world).shot.is_some());
    }

    #[test]
    fn death_and_respawn_cancel_old_shots_ticks_and_reaction_state() {
        let world = collision(false);
        let mut bot = bot();
        let near = Some(target(100.0));
        bot.step(1000, near, weapon(), &world);
        let shot = bot.step(1350, near, weapon(), &world).shot.unwrap();
        // Damage commands can lag the latest authority tick.
        assert!(bot.kill(1320));
        assert!(!bot.shot_is_current(&shot));
        let dead = bot.step(2000, near, weapon(), &world);
        assert_eq!(dead.phase, BotPhase::Dead);
        assert!(dead.shot.is_none());
        assert_eq!(dead.motion.velocity, [0.0; 3]);
        bot.respawn(3000, [10.0, 0.0, 0.03125], [0; 3]).unwrap();
        assert!(!bot.shot_is_current(&shot));
        let stale = bot.step(1500, near, weapon(), &world);
        assert_eq!(stale.phase, BotPhase::Idle);
        assert_eq!(stale.motion.origin[0], 10.0);
        assert!(stale.shot.is_none());
        assert!(!bot.kill(1320));
        assert!(bot.step(3010, near, weapon(), &world).shot.is_none());
        assert!(bot.step(3359, near, weapon(), &world).shot.is_none());
        assert!(bot.step(3360, near, weapon(), &world).shot.is_some());
    }

    #[test]
    fn authority_clock_wrap_keeps_movement_reaction_and_cadence_ordered() {
        let world = collision(false);
        let mut bot = bot();
        let near = Some(target(100.0));
        let start = u32::MAX - 200;
        bot.step(start, near, weapon(), &world);
        assert!(
            bot.step(start.wrapping_add(349), near, weapon(), &world)
                .shot
                .is_none()
        );
        let frame = bot.step(start.wrapping_add(350), near, weapon(), &world);
        assert!(frame.shot.is_some());
        assert_eq!(frame.motion.command_time, start.wrapping_add(350));
        assert!(
            bot.step(start.wrapping_add(300), near, weapon(), &world)
                .shot
                .is_none()
        );
        assert!(
            bot.step(start.wrapping_add(430), near, weapon(), &world)
                .shot
                .is_some()
        );
    }

    #[test]
    fn movement_crosses_authority_wrap_and_rebases_its_private_physics_clock() {
        let world = collision(false);
        let mut bot = bot();
        let far = Some(target(400.0));
        let start = u32::MAX - 20;
        bot.step(start, far, weapon(), &world);
        let frame = bot.step(start.wrapping_add(40), far, weapon(), &world);
        assert!(frame.motion.origin[0] > 0.1);
        assert_eq!(frame.motion.command_time, 19);
        let before = frame.motion.origin[0];
        bot.motion.command_time = u32::MAX - 5;
        let frame = bot.step(39, far, weapon(), &world);
        assert!(frame.motion.origin[0] > before);
        assert!(frame.motion.velocity.iter().all(|v| v.is_finite()));
        assert_eq!(frame.motion.command_time, 39);
    }

    #[test]
    fn invalid_resources_or_a_dead_player_cannot_authorise_shooting() {
        let world = collision(false);
        let mut bot = bot();
        let near = Some(target(100.0));
        let bad = BotWeapon {
            fire_interval_msec: 0,
            ..weapon()
        };
        bot.step(1000, near, bad, &world);
        assert!(bot.step(3000, near, bad, &world).shot.is_none());
        assert!(
            bot.step(
                4000,
                Some(BotTarget {
                    alive: false,
                    ..target(100.0)
                }),
                weapon(),
                &world
            )
            .shot
            .is_none()
        );
        assert!(BotController::new([f32::NAN; 3], [0; 3], 100.0).is_err());
        assert!(BotController::new([0.0; 3], [0; 3], f32::NAN).is_err());
    }

    #[test]
    fn atan2_aim_has_downward_positive_pitch_and_all_yaw_quadrants() {
        assert_eq!(aim_angles([0.0; 3], [10.0, 0.0, 0.0]), [0, 0, 0]);
        assert_eq!(aim_angles([0.0; 3], [0.0, 10.0, 0.0]), [0, 16384, 0]);
        assert_eq!(aim_angles([0.0; 3], [-10.0, 0.0, 0.0]), [0, 32768, 0]);
        assert_eq!(aim_angles([0.0; 3], [0.0, -10.0, 0.0]), [0, 49152, 0]);
        assert_eq!(aim_angles([0.0; 3], [10.0, 0.0, -10.0]), [8192, 0, 0]);
        assert_eq!(aim_angles([0.0; 3], [10.0, 0.0, 10.0]), [57344, 0, 0]);
    }
}
