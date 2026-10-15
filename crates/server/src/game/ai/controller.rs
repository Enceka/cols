//! Basic server-owned training AI using the recovered movement and world traces.
//!
//! Reaction, spacing and burst pacing are explicit gameplay design choices,
//! rather than claims about the original bot AI. Weapon identity, damage,
//! distance band, cadence and locomotion speed come from captured resources.
use anyhow::{Result, ensure};
use codol_protocol::game::channel::usercmd::UserCommand;

use crate::game::ai::navigation::Waypoint;
use crate::game::ai::navigation::jump_command;
use crate::game::ai::navigation::jump_landed;
use crate::game::ai::navigation::jump_reachable;
use crate::game::combat::Ballistics;
use crate::game::player::movement::Motion;
use crate::game::world::collision::Collision;

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
    let height = crate::game::player::movement::Stance::Stand.height();
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
    anyhow::bail!("no clear walkable standing capsule near bot spawn candidate {origin:?}")
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
    pub(crate) landings: Vec<crate::game::player::landing::Landing>,
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
struct JumpDrive {
    goal: [f32; 3],
    start: u32,
    scale: f32,
    runup_msec: u16,
    launched: bool,
}

#[derive(Debug)]
pub struct BotController {
    landings: Vec<crate::game::player::landing::Landing>,
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
    match_ads_move_scale: f32,
    locomotion: Locomotion,
    control_move_scale: f32,
    control_turn_limit: Option<[f32; 2]>,
    jump_goal: Option<JumpDrive>,
    jump_check_time: u32,
    jumps: u32,
    travel_requested: bool,
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
            landings: Vec::new(),
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
            match_ads_move_scale: 1.0,
            locomotion: Locomotion::Walk,
            control_move_scale: 1.0,
            control_turn_limit: None,
            jump_goal: None,
            jump_check_time: 0,
            jumps: 0,
            travel_requested: false,
        })
    }

    fn frame(&mut self, now: u32, shot: Option<BotShot>) -> BotFrame {
        let physics_time = self.motion.command_time;
        let mut landings = std::mem::take(&mut self.landings);
        for landing in &mut landings {
            landing.time = now.wrapping_sub(physics_time.wrapping_sub(landing.time));
        }
        let mut motion = self.motion.clone();
        // The public frame belongs to the authority clock. The private motion
        // uses a small monotonic physics clock because Motion::apply currently
        // compares ordinary u32 input times and cannot cross a clock wrap.
        motion.command_time = now;
        for time in [
            &mut motion.sprint.last_start,
            &mut motion.sprint.last_end,
            &mut motion.jump_time,
            &mut motion.view_height_lerp_time,
        ] {
            if *time != 0 {
                *time = time.wrapping_add(now.wrapping_sub(physics_time));
            }
        }
        BotFrame {
            landings,
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
        self.motion.speed = crate::game::player::movement::SPEED;
    }
    pub fn set_match_range(&mut self, range: f32, spacing: f32) {
        self.engagement = Some((range, spacing));
    }
    pub fn set_match_move_scale(&mut self, scale: f32) {
        self.match_move_scale = scale;
    }
    pub fn set_match_ads_move_scale(&mut self, scale: f32) {
        self.match_ads_move_scale = scale;
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
        self.step_route(now, target, waypoint.map(Waypoint::walk), weapon, collision)
    }
    pub(crate) fn step_route(
        &mut self,
        now: u32,
        target: Option<BotTarget>,
        waypoint: Option<Waypoint>,
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
        let waypoint =
            waypoint.filter(|p| p.jump_from.is_none() || !jump_landed(&self.motion, p.origin));
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
                let trace = collision.visibility_raycast(self.eye(), t.eye);
                !trace.start_solid && !trace.all_solid && trace.contents == 0
            });
        let hold = visible_before
            && target.is_some_and(|t| horizontal_distance(self.motion.origin, t.origin) <= spacing);
        let mode = if self.jump_goal.is_some() {
            Locomotion::Run
        } else if self.engagement.is_none()
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
        let mut travel = if hold {
            None
        } else {
            waypoint
                .map(|p| {
                    let origin = p.jump_from.unwrap_or(p.origin);
                    BotTarget {
                        origin,
                        eye: [origin[0], origin[1], origin[2] + 60.0],
                        alive: true,
                    }
                })
                .or(target)
        };
        self.travel_requested = travel.is_some() || self.jump_goal.is_some();
        self.move_toward(
            delta,
            travel,
            if waypoint.is_some() { 4.0 } else { spacing },
            collision,
            mode,
            if hold {
                None
            } else {
                waypoint.filter(|p| p.jump_from.is_some())
            },
        );
        if let Some(drive) = &self.jump_goal {
            let origin = drive.goal;
            travel = Some(BotTarget {
                origin,
                eye: [origin[0], origin[1], origin[2] + 60.0],
                alive: true,
            });
        }
        let Some(target) = target else {
            if let Some(travel) = travel {
                self.angles = self.control_turn(aim_angles(self.eye(), travel.eye), delta);
            }
            self.phase = if travel.is_some() {
                BotPhase::Chase
            } else {
                BotPhase::Idle
            };
            self.reset_engagement();
            return self.frame(now, None);
        };
        let eye = self.eye();
        let desired_angles = if self.engagement.is_some() && mode != Locomotion::Walk {
            travel.map_or(self.angles, |t| aim_angles(eye, t.eye))
        } else {
            aim_angles(eye, target.eye)
        };
        self.angles = if self.engagement.is_some() {
            self.control_turn(desired_angles, delta)
        } else {
            desired_angles
        };
        let distance = distance(eye, target.eye);
        let trace = collision.visibility_raycast(eye, target.eye);
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
        if self.engagement.is_some()
            && (mode != Locomotion::Walk
                || aim_error(self.angles, aim_angles(eye, target.eye)) > 3.0)
        {
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

    pub(crate) fn set_tactical_control(&mut self, scale: f32, limit: Option<[f32; 2]>) {
        self.control_move_scale = scale;
        self.control_turn_limit = limit;
        self.motion.flags = (self.motion.flags & !0x8000) | if scale < 1.0 { 0x8000 } else { 0 };
    }
    fn control_turn(&self, desired: [u16; 3], delta: u32) -> [u16; 3] {
        let angles = turn_toward(self.angles, desired, delta);
        let Some(limit) = self.control_turn_limit else {
            return angles;
        };
        std::array::from_fn(|i| {
            if i >= 2 {
                return angles[i];
            }
            let maximum = (limit[i] * delta as f32 * 0.001 * 65536.0 / 360.0).round() as i32;
            self.angles[i].wrapping_add(
                i32::from(angles[i].wrapping_sub(self.angles[i]) as i16).clamp(-maximum, maximum)
                    as i16 as u16,
            )
        })
    }
    pub(crate) fn eye(&self) -> [f32; 3] {
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
        jump: Option<Waypoint>,
    ) {
        while remaining > 0 {
            let dt = remaining.min(self.physics_chop_msec);
            remaining -= dt;
            let moving =
                target.is_some_and(|t| horizontal_distance(self.motion.origin, t.origin) > spacing);
            let angles = target.map_or(self.angles, |t| aim_angles(self.eye(), t.eye));
            if !moving && self.jump_goal.is_none() {
                // Prevent the last chase velocity from carrying through the
                // desired separation. Vertical gravity still uses real motion.
                self.motion.velocity[0] = 0.0;
                self.motion.velocity[1] = 0.0;
            }
            if self.motion.command_time > u32::MAX - dt {
                // Keep jump cooldowns and traversal deadlines relative to the
                // private clock when it is rebased, including an airborne bot.
                let old = self.motion.command_time;
                let base = 1000;
                self.motion.jump_time = base - old.saturating_sub(self.motion.jump_time).min(base);
                if let Some(drive) = &mut self.jump_goal {
                    drive.start = base - old.saturating_sub(drive.start).min(base);
                }
                self.jump_check_time = base;
                self.motion.sprint.last_start =
                    base - old.saturating_sub(self.motion.sprint.last_start).min(base);
                self.motion.sprint.last_end =
                    base - old.saturating_sub(self.motion.sprint.last_end).min(base);
                self.motion.command_time = base;
                self.motion.view_height_lerp_time = 0;
            }
            let command = UserCommand {
                time: self.motion.command_time + dt,
                direction: u8::from(moving), // Recovered forward direction=1.
                angles,
                buttons: if mode == Locomotion::Sprint { 2 } else { 0 },
                ..UserCommand::default()
            };
            let ads = self.engagement.is_some() && mode == Locomotion::Walk;
            let speed_scale = if ads {
                self.match_ads_move_scale
            } else if self.engagement.is_some() {
                self.match_move_scale
            } else {
                self.move_speed / f32::from(self.motion.speed)
            };
            let jump_scale = if self.engagement.is_some() {
                self.match_move_scale
            } else {
                self.move_speed / f32::from(self.motion.speed)
            } * self.control_move_scale;
            let finished_goal = self
                .jump_goal
                .as_ref()
                .filter(|drive| {
                    (drive.launched && jump_landed(&self.motion, drive.goal))
                        || self.motion.command_time.saturating_sub(drive.start) >= 1200
                })
                .map(|drive| drive.goal);
            if finished_goal.is_some() {
                self.jump_goal = None;
            }
            let attempt = self.jump_goal.is_none()
                && jump.is_some_and(|p| {
                    distance(self.motion.origin, p.jump_from.unwrap()) <= 8.0
                        && self.motion.command_time >= self.jump_check_time
                        && self
                            .motion
                            .command_time
                            .saturating_sub(self.motion.jump_time)
                            >= 500
                });
            let begin = attempt
                && jump_reachable(
                    self.motion.clone(),
                    jump.unwrap().origin,
                    jump_scale,
                    jump.unwrap().runup_msec,
                    collision,
                );
            if begin {
                let waypoint = jump.unwrap();
                self.motion.velocity[0] = 0.0;
                self.motion.velocity[1] = 0.0;
                self.jump_goal = Some(JumpDrive {
                    goal: waypoint.origin,
                    start: self.motion.command_time,
                    scale: jump_scale,
                    runup_msec: waypoint.runup_msec,
                    launched: false,
                });
            }
            if attempt && !begin {
                self.jump_check_time = self.motion.command_time + 250;
            }
            let (command, speed_scale, ads) = if let Some(drive) = &mut self.jump_goal {
                let launch = !drive.launched
                    && self.motion.command_time.saturating_sub(drive.start)
                        >= u32::from(drive.runup_msec);
                if launch {
                    drive.launched = true;
                }
                // Retain landing steering even if an enemy appears or a path
                // changes in midair. Sprint/ADS do not alter the checked arc.
                (
                    jump_command(&mut self.motion, drive.goal, dt, launch),
                    drive.scale.min(jump_scale),
                    false,
                )
            } else if finished_goal.is_some_and(|goal| jump.is_some_and(|p| p.origin == goal)) {
                self.motion.velocity[0] = 0.0;
                self.motion.velocity[1] = 0.0;
                (
                    UserCommand {
                        direction: 0,
                        ..command
                    },
                    speed_scale * self.control_move_scale,
                    ads,
                )
            } else {
                (command, speed_scale * self.control_move_scale, ads)
            };
            self.motion.apply(&command, speed_scale, ads, collision);
            if command.buttons & 0x400 != 0 && self.motion.jump_time == command.time {
                self.jumps = self.jumps.saturating_add(1);
            }
            // A controller tick can contain several physics commands. Retain
            // a landing from any substep until the single public frame drains it.
            self.landings.extend(self.motion.take_landings());
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
        self.landings.clear();
        self.motion.velocity = [0.0; 3];
        self.jump_goal = None;
        self.travel_requested = false;
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
        self.jump_goal = None;
        self.jump_check_time = 0;
        self.travel_requested = false;
        self.landings.clear();
        self.motion.speed = if self.engagement.is_some() {
            crate::game::player::movement::SPEED
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
        if self.jump_goal.is_some() || !self.motion.grounded() {
            return;
        }
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
            self.motion
                .move_separation([shift[0] * scale, shift[1] * scale], collision);
        }
    }
    pub(crate) fn traversing(&self) -> bool {
        self.jump_goal.is_some()
    }
    pub(crate) fn has_move_intent(&self) -> bool {
        self.travel_requested
    }
    pub(crate) fn jump_count(&self) -> u32 {
        self.jumps
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

pub(crate) fn aim_angles(eye: [f32; 3], target: [f32; 3]) -> [u16; 3] {
    let delta = std::array::from_fn::<_, 3, _>(|i| f64::from(target[i]) - f64::from(eye[i]));
    let yaw = delta[1].atan2(delta[0]);
    let pitch = (-delta[2]).atan2(delta[0].hypot(delta[1]));
    let short = |radians: f64| {
        ((radians * (65536.0 / std::f64::consts::TAU)).round() as i64 & 0xffff) as u16
    };
    [short(pitch), short(yaw), 0]
}

fn aim_error(a: [u16; 3], b: [u16; 3]) -> f32 {
    (0..2)
        .map(|i| f32::from(b[i].wrapping_sub(a[i]) as i16).abs() * 360.0 / 65536.0)
        .fold(0.0, f32::max)
}
fn turn_toward(from: [u16; 3], to: [u16; 3], delta: u32) -> [u16; 3] {
    // Reference AI eases angular differences by rate * executed dt. Keep this
    // bounded local policy separate from unavailable mounted difficulty data.
    let fraction = (4.0 * delta as f32 * 0.001).min(1.0);
    std::array::from_fn(|i| {
        from[i].wrapping_add(
            (f32::from(to[i].wrapping_sub(from[i]) as i16) * fraction).round() as i16 as u16,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn landing_events_use_authority_time_once_even_at_clock_wrap() {
        let world = collision(false);
        let start = u32::MAX - 500;
        let mut bot = BotController::new([0.0, 0.0, 350.0], [0; 3], 190.0).unwrap();
        let mut seen = Vec::new();
        for i in 0..45 {
            let now = start.wrapping_add(i * 50);
            let frame = bot.step_navigated(now, None, Some([1000.0, 0.0, 0.0]), weapon(), &world);
            for landing in frame.landings {
                assert!(now.wrapping_sub(landing.time) <= 50);
                seen.push(landing);
            }
            assert!(
                bot.step_navigated(now, None, Some([1000.0, 0.0, 0.0]), weapon(), &world)
                    .landings
                    .is_empty()
            );
        }
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].damage, 100);
    }
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
    fn concussion_control_slows_actual_ground_motion_and_caps_turning_then_restores_it() {
        let world = collision(false);
        let target = BotTarget {
            origin: [1000.0, 0.0, 0.0],
            eye: [1000.0, 0.0, 60.0],
            alive: true,
        };
        let mut normal = BotController::new([0.0; 3], [0; 3], 190.0).unwrap();
        let mut stunned = BotController::new([0.0; 3], [0; 3], 190.0).unwrap();
        normal.set_match_engagement();
        stunned.set_match_engagement();
        stunned.set_tactical_control(0.4, Some([75.0, 75.0]));
        let mut last = None;
        for now in (1000..=2000).step_by(50) {
            let a = normal.step(now, Some(target), weapon(), &world);
            let b = stunned.step(now, Some(target), weapon(), &world);
            last = Some((a, b));
        }
        let (a, b) = last.unwrap();
        assert!(a.motion.velocity[0] > 0.0);
        assert!((b.motion.velocity[0] / a.motion.velocity[0] - 0.4).abs() < 0.02);
        assert_ne!(b.motion.flags & 0x8000, 0);
        let side = BotTarget {
            origin: [0.0, 1000.0, 0.0],
            eye: [0.0, 1000.0, 60.0],
            alive: true,
        };
        let previous = b.angles[1];
        let turned = stunned.step(2050, Some(side), weapon(), &world);
        assert!(
            (f32::from(turned.angles[1].wrapping_sub(previous) as i16) * 360.0 / 65536.0).abs()
                <= 3.76
        );
        stunned.set_tactical_control(1.0, None);
        let restored = stunned.step(2100, Some(target), weapon(), &world);
        assert_eq!(restored.motion.flags & 0x8000, 0);
    }

    #[test]
    fn crowd_separation_follows_walkable_slopes_in_both_directions() {
        for terrain in [false, true] {
            let normal = [0.5 / 1.25f32.sqrt(), 0.0, 1.0 / 1.25f32.sqrt()];
            let export = json!({
                "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
                "map":"maps/mp/test.d3dbsp","ncs_crc":0,
                "brushes": if terrain {json!([])} else {json!([{"center":[0,0,-500],"half":[500,500,1000],"contents":1,
                    "planes":[[normal[0],normal[1],normal[2],0]]}])},
                "triangles":if terrain {json!([{"vertices":[[-500,-500,250],[500,-500,-250],[0,500,0]],"contents":1,"surface_flags":0}])} else {json!([])}
            });
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
            let world = Collision::load(file.path()).unwrap();
            let origin = grounded_spawn([0.0, 0.0, 8.0], &world).unwrap();
            for direction in [-1.0, 1.0] {
                let mut bot = BotController::new(origin, [0; 3], 190.0).unwrap();
                bot.step(1000, None, weapon(), &world);
                bot.step(1050, None, weapon(), &world);
                for now in (1100..=1550).step_by(50) {
                    let start = bot.motion.origin;
                    let peer = [start[0] - direction * 8.0, start[1], start[2]];
                    bot.separate_from(1, &[(2, peer)], &world);
                    let frame = bot.step(now, None, weapon(), &world);
                    let end = frame.motion.origin;
                    let down = world.trace(end, [end[0], end[1], end[2] - 9.0], 70.0);
                    assert!(
                        (end[0] - start[0]) * direction > 3.0,
                        "slope avoidance stopped: terrain={terrain}, direction={direction}, {start:?} -> {end:?}"
                    );
                    assert!(
                        !down.start_solid && down.fraction * 9.0 < 0.1,
                        "slope avoidance lifted hull: terrain={terrain}, direction={direction}, {end:?}, {down:?}"
                    );
                    assert!(frame.motion.grounded());
                }
            }
        }
    }

    #[test]
    fn visible_opponents_only_suppress_stuck_recovery_when_the_bot_is_holding_range() {
        let world = collision(false);
        let mut bot = BotController::new([0.0; 3], [0; 3], 190.0).unwrap();
        bot.set_match_engagement();
        let far = BotTarget {
            origin: [1200.0, 0.0, 0.0],
            eye: [1200.0, 0.0, 60.0],
            alive: true,
        };
        bot.step(1000, Some(far), weapon(), &world);
        let advancing = bot.step(1050, Some(far), weapon(), &world);
        assert!(bot.has_move_intent() && advancing.moving());
        let close = BotTarget {
            origin: [100.0, 0.0, 0.0],
            eye: [100.0, 0.0, 60.0],
            alive: true,
        };
        let holding = bot.step(1100, Some(close), weapon(), &world);
        assert!(!bot.has_move_intent() && !holding.moving());
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
    fn stock_mobility_changes_engaged_walk_to_native_ads_speed() {
        let world = collision(false);
        let mut normal = BotController::new([0.0; 3], [0; 3], 94.0).unwrap();
        normal.set_match_engagement();
        normal.set_match_move_scale(0.95);
        let mut stock = BotController::new([0.0; 3], [0; 3], 94.0).unwrap();
        stock.set_match_engagement();
        stock.set_match_move_scale(0.95);
        stock.set_match_ads_move_scale(2.2);
        let mut speeds = [0.0f32; 2];
        for (i, bot) in [&mut normal, &mut stock].into_iter().enumerate() {
            for time in (1000..2000).step_by(50) {
                let frame = bot.step(time, Some(target(1000.0)), weapon(), &world);
                speeds[i] = frame.motion.velocity[0].hypot(frame.motion.velocity[1]);
            }
        }
        assert!((speeds[0] - 76.0).abs() < 0.1);
        assert!((speeds[1] - 167.2).abs() < 0.1);
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
        assert_eq!(frame.motion.speed, crate::game::player::movement::SPEED);
        assert_ne!(frame.motion.flags & 0x4000, 0);
        assert!((frame.motion.velocity[0].hypot(frame.motion.velocity[1]) - 256.5).abs() < 1.0);
        assert!(aim_error(frame.angles, [0, 16384, 0]) < 2.0); // Smoothly faces the route.

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
        assert!((repaired[2] + 75.77473).abs() < 0.001);
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
        assert!((repaired[2] - 0.125).abs() < 1e-5);
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
