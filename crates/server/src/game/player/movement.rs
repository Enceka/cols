//! Authoritative player motion against world collision.
use crate::game::world::collision::Collision;
use crate::game::world::collision::NONE;
use crate::game::world::collision::WORLD;
use crate::game::world::math::*;
use codol_protocol::game::channel::usercmd::UserCommand;
pub const SPEED: u16 = 190;
pub const GRAVITY: u16 = 800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stance {
    #[default]
    Stand,
    Crouch,
    Prone,
}
impl Stance {
    pub fn height(self) -> f32 {
        match self {
            Self::Stand => 70.0,
            Self::Crouch => 50.0,
            Self::Prone => 30.0,
        }
    }
    pub fn view_height(self) -> u8 {
        match self {
            Self::Stand => 60,
            Self::Crouch => 40,
            Self::Prone => 11,
        }
    }
    fn flags(self) -> u32 {
        match self {
            Self::Stand => 0,
            Self::Crouch => 2,
            Self::Prone => 1,
        }
    }
    fn scale(self) -> f32 {
        match self {
            Self::Stand => 1.0,
            Self::Crouch => 0.65,
            Self::Prone => 0.15,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Motion {
    pub pm_time: u16,
    landings: Vec<crate::game::player::landing::Landing>,
    pub sprint: codol_protocol::game::snapshot::SprintState,
    sprint_rules: crate::game::player::sprint::Rules,
    pub origin: V3,
    pub velocity: V3,
    pub command_time: u32,
    pub speed: u16,
    pub gravity: u16,
    pub stance: Stance,
    pub flags: u32,
    pub ground_entity: u16,
    pub jump_time: u32,
    pub jump_origin_z: f32,
    pub view_height: f32,
    pub view_height_target: u8,
    pub view_height_lerp_time: u32,
    pub view_height_lerp_target: u8,
    pub view_height_lerp_down: bool,
    pub bob_cycle: u8,
    pub surface_flags: u32,
    melee_charge: Option<(codol_protocol::game::snapshot::MeleeCharge, u16)>,
    previous_buttons: u32,
    ground_normal: V3,
    view_start: f32,
}
impl Motion {
    pub fn new(origin: V3) -> Self {
        Self {
            pm_time: 0,
            landings: Vec::new(),
            sprint: Default::default(),
            sprint_rules: Default::default(),
            origin,
            velocity: [0.0; 3],
            command_time: 0,
            speed: SPEED,
            gravity: GRAVITY,
            stance: Stance::Stand,
            flags: 0,
            ground_entity: NONE,
            jump_time: 0,
            jump_origin_z: 0.0,
            view_height: 60.0,
            view_height_target: 60,
            view_height_lerp_time: 0,
            view_height_lerp_target: 60,
            view_height_lerp_down: false,
            bob_cycle: 0,
            surface_flags: 0,
            melee_charge: None,
            previous_buttons: 0,
            ground_normal: [0.0, 0.0, 1.0],
            view_start: 60.0,
        }
    }
    pub fn grounded(&self) -> bool {
        self.ground_entity != NONE
    }
    pub(crate) fn take_landings(&mut self) -> Vec<crate::game::player::landing::Landing> {
        std::mem::take(&mut self.landings)
    }
    pub(crate) fn set_sprint_rules(&mut self, rules: crate::game::player::sprint::Rules) {
        self.sprint_rules = rules;
    }
    pub(crate) fn cancel_sprint(&mut self) {
        if self.flags & 0x4000 != 0 {
            crate::game::player::sprint::stop(
                &mut self.sprint,
                self.command_time,
                self.previous_buttons & 2 != 0,
            );
            self.flags &= !0x4000;
        }
    }
    pub fn melee_charge(&self) -> Option<codol_protocol::game::snapshot::MeleeCharge> {
        self.melee_charge.map(|(state, _)| state)
    }
    pub(crate) fn start_melee_charge(
        &mut self,
        approach: crate::game::weapons::held::MeleeApproach,
        duration: u16,
    ) {
        if approach.distance == 0 || !(1..=15000).contains(&duration) || self.melee_charge.is_some()
        {
            return;
        }
        self.melee_charge = Some((
            codol_protocol::game::snapshot::MeleeCharge {
                yaw: approach.yaw,
                distance: approach.distance,
                remaining_msec: 0,
            },
            duration,
        ));
        self.cancel_sprint();
        self.flags |= 0x10000;
    }
    pub fn cancel_melee_charge(&mut self) {
        self.melee_charge = None;
        self.flags &= !0x10000;
    }
    fn step_melee_charge(&mut self, dt_ms: u32, c: &Collision) {
        let (mut state, duration) = self.melee_charge.unwrap();
        if state.remaining_msec == 0 {
            let seconds = (f64::from(duration) * 0.001) as f32;
            let speed = 2.0 * f32::from(state.distance) / seconds;
            let yaw = f32::from(state.yaw as i16) * (std::f32::consts::TAU / 65536.0);
            self.velocity[0] = speed * yaw.cos();
            self.velocity[1] = speed * yaw.sin();
            // 0x41cbaa stores seconds as binary32, multiplies by the double
            //1000 constant and truncates when initializing the timer.
            state.remaining_msec = (f64::from(seconds) * 1000.0) as u16;
        }
        let speed = length(self.velocity);
        let direction = normalized(self.velocity);
        let dt = (f64::from(dt_ms) * 0.001) as f32;
        let next = (speed - speed / (f32::from(state.remaining_msec) * 0.001) * dt).max(0.0);
        self.velocity = mul(direction, (speed + next) * 0.5);
        if self.grounded() {
            self.velocity = project_ground(self.velocity, self.ground_normal);
        }
        if self.velocity[0] != 0.0 || self.velocity[1] != 0.0 {
            self.step_slide(c, dt, false);
        }
        self.velocity = mul(direction, next);
        state.remaining_msec = state.remaining_msec.saturating_sub(dt_ms as u16);
        self.flags = self.stance.flags() | 0x10000 | (self.flags & 0x8000);
        if state.remaining_msec == 0 {
            self.velocity[0] = 0.0;
            self.velocity[1] = 0.0;
            self.cancel_melee_charge();
        } else {
            self.melee_charge = Some((state, duration));
        }
        self.ground(c, 0.25);
    }
    /// Apply a small crowd-avoidance displacement through the same capsule
    /// solver as walking. This is not elapsed movement time: preserve the
    /// actor's velocity and clocks, and never apply gravity a second time.
    pub fn move_separation(&mut self, shift: [f32; 2], c: &Collision) {
        if shift == [0.0; 2] {
            return;
        }
        self.ground(c, 0.25);
        let velocity = self.velocity;
        self.velocity = [shift[0], shift[1], 0.0];
        if self.grounded() {
            self.velocity[2] = -(shift[0] * self.ground_normal[0]
                + shift[1] * self.ground_normal[1])
                / self.ground_normal[2];
            self.step_slide(c, 1.0, false);
        } else {
            // Airborne avoidance may slide against a wall but cannot step up
            // or attach itself to a floor below a ledge.
            self.slide(c, 1.0, false);
        }
        self.velocity = velocity;
        self.ground(c, 0.25);
    }
    /// `weapon_scale` is the currently selected weapon's hip/ADS scale.
    /// Command time belongs to input, never the snapshot clock.
    pub fn apply(
        &mut self,
        cmd: &UserCommand,
        weapon_scale: f32,
        ads: bool,
        collision: &Collision,
    ) {
        if cmd.time <= self.command_time || !weapon_scale.is_finite() || weapon_scale < 0.0 {
            return;
        }
        self.landings.clear();
        if cmd.time - self.command_time > 1000 {
            self.command_time = cmd.time - 1000;
        }
        while self.command_time < cmd.time {
            let dt_ms = (cmd.time - self.command_time).min(66);
            self.command_time += dt_ms;
            self.step(cmd, weapon_scale, ads, dt_ms, collision);
            self.previous_buttons = cmd.buttons;
        }
    }
    fn step(&mut self, cmd: &UserCommand, weapon_scale: f32, ads: bool, dt_ms: u32, c: &Collision) {
        self.pm_time = self.pm_time.saturating_sub(dt_ms as u16);
        if self.pm_time == 0 {
            self.flags &= !0x80;
        }
        let entered_airborne = !self.grounded();
        let previous_z = self.origin[2];
        let previous_velocity_z = self.velocity[2];
        let dt = dt_ms as f32 * 0.001;
        let desired = if cmd.buttons & 0x100 != 0 {
            Stance::Prone
        } else if cmd.buttons & 0x200 != 0 {
            Stance::Crouch
        } else {
            Stance::Stand
        };
        if desired != self.stance
            && (desired.height() < self.stance.height()
                || !c
                    .trace(self.origin, self.origin, desired.height())
                    .start_solid)
        {
            self.stance = desired;
            self.view_start = self.view_height;
            self.view_height_target = desired.view_height();
            self.view_height_lerp_target = desired.view_height();
            self.view_height_lerp_time = self.command_time;
            self.view_height_lerp_down = self.view_height > f32::from(self.view_height_target);
        }
        self.update_view_height();
        self.ground(c, 0.25);
        if self.melee_charge.is_some() {
            self.step_melee_charge(dt_ms, c);
            return;
        }
        let jumping = self.grounded()
            && self.stance == Stance::Stand
            && cmd.buttons & 0x400 != 0
            && self.previous_buttons & 0x400 == 0
            && self.command_time.saturating_sub(self.jump_time) >= 500;
        if jumping {
            self.velocity[2] = (2.0 * f32::from(self.gravity) * 39.0).sqrt();
            self.jump_time = self.command_time;
            self.jump_origin_z = self.origin[2];
            self.ground_entity = NONE;
        }
        let locked = cmd.buttons & (0x1000_0000 | 0x0020_0000) != 0;
        let f = if locked {
            0.0
        } else if cmd.direction & 1 != 0 {
            127.0
        } else if cmd.direction & 2 != 0 {
            -127.0
        } else {
            0.0
        };
        let mut r = if locked {
            0.0
        } else if cmd.direction & 4 != 0 {
            127.0
        } else if cmd.direction & 8 != 0 {
            -127.0
        } else {
            0.0
        };
        let sprint = crate::game::player::sprint::update(
            &mut self.sprint,
            self.command_time,
            cmd.buttons,
            self.previous_buttons,
            self.flags & 0x4000 != 0,
            f <= 105.0
                || ads
                || self.stance != Stance::Stand
                || self.flags & (0x8000 | 0x20000) != 0,
            self.sprint_rules,
        );
        self.flags = self.stance.flags()
            | (self.flags & (0x8000 | 0x80))
            | if sprint { 0x4000 } else { 0 }
            | if !self.grounded() && self.jump_time != 0 {
                0x2000
            } else {
                0
            };
        if sprint && self.grounded() {
            // 0x4174ed..0x417502, double constant 0xc862d0.
            r = (r * 0.667f32).trunc();
        }
        let horizontal =
            (self.velocity[0] * self.velocity[0] + self.velocity[1] * self.velocity[1]).sqrt();
        if self.grounded() {
            if horizontal < 1.0 {
                self.velocity = [0.0; 3];
            } else {
                let scale = ((horizontal - horizontal.max(100.0) * 5.5 * dt).max(0.0)) / horizontal;
                // Target PM_Friction 0x416390 measures horizontal speed on
                // ground but applies the resulting scale to all three axes.
                self.velocity = mul(self.velocity, scale);
            }
        }
        let yaw = f32::from(cmd.angles[1]) * (std::f32::consts::TAU / 65536.0);
        let wish = [
            yaw.cos() * f + yaw.sin() * r,
            yaw.sin() * f - yaw.cos() * r,
            0.0,
        ];
        let total = (f * f + r * r).sqrt();
        let biggest = (if f < 0.0 { f * 0.7 } else { f })
            .abs()
            .max((r * 0.8).abs());
        let speed = if total > 0.0 {
            f32::from(self.speed) * biggest / 127.0
                * weapon_scale
                * self.stance.scale()
                * if ads { 0.4 } else { 1.0 }
                * if sprint { 1.5 } else { 1.0 }
                * if self.pm_time != 0 && self.flags & 0x80 != 0 {
                    0.25
                } else {
                    1.0
                }
        } else {
            0.0
        };
        let mut direction = normalized(wish);
        if self.grounded() {
            direction = project_ground(direction, self.ground_normal);
        }
        let acceleration = if !self.grounded() {
            1.0
        } else {
            match self.stance {
                Stance::Stand => 9.0,
                Stance::Crouch => 12.0,
                Stance::Prone => 19.0,
            }
        };
        let amount = (speed - dot(self.velocity, direction))
            .max(0.0)
            .min(dt * acceleration * speed.max(100.0));
        self.velocity = add(self.velocity, mul(direction, amount));
        if self.grounded() {
            self.velocity = project_ground(self.velocity, self.ground_normal);
        }
        let was_grounded = self.grounded();
        if was_grounded && horizontal > 1.0 {
            self.bob_cycle = self
                .bob_cycle
                .wrapping_add((horizontal / f32::from(self.speed) * dt_ms as f32 * 0.4) as u8);
        }
        // Target WalkMove skips step-slide when horizontal velocity is zero.
        // In particular a ground-classification probe must not move an idle
        // player's origin or project its velocity on every input command.
        if !was_grounded || self.velocity[0] != 0.0 || self.velocity[1] != 0.0 {
            self.step_slide(c, dt, !was_grounded);
        }
        self.ground(c, 0.25);
        if entered_airborne
            && self.grounded()
            && let Some(landing) = crate::game::player::landing::calculate(
                self.command_time,
                previous_z,
                previous_velocity_z,
                self.origin[2],
                self.gravity,
                self.surface_flags,
            )
        {
            self.velocity = mul(self.velocity, landing.velocity_scale);
            if landing.pm_time != 0 {
                self.pm_time = landing.pm_time;
                self.flags |= 0x80;
            }
            self.landings.push(landing);
        }
    }
    fn update_view_height(&mut self) {
        if self.view_height_lerp_time == 0 {
            return;
        }
        let duration = if self.view_start < 40.0 || self.view_height_target == 11 {
            400.0
        } else {
            200.0
        };
        let t = (self.command_time - self.view_height_lerp_time) as f32 / duration;
        if t >= 1.0 {
            self.view_height = f32::from(self.view_height_target);
            self.view_height_lerp_time = 0;
        } else {
            let smooth = t * t * (3.0 - 2.0 * t);
            self.view_height =
                self.view_start + (f32::from(self.view_height_target) - self.view_start) * smooth;
        }
    }
    pub(in crate::game) fn ground(&mut self, c: &Collision, distance: f32) {
        // PM_GroundTrace 0x418500 probes +0.25 -> -0.25. Position changes
        // belong to StepSlideMove; the probe only classifies the current hull.
        let trace = c.trace(
            add(self.origin, [0.0, 0.0, distance]),
            add(self.origin, [0.0, 0.0, -distance]),
            self.stance.height(),
        );
        // A slope change can increase world-space vertical velocity while the
        // player remains tangent to the new surface. Test the freshly traced
        // plane, not the preceding frame's normal (which falsely launches the
        // player and lets gravity accumulate sideways drift on Afghan terrain).
        if trace.fraction < 1.0 && self.velocity[2] > 0.0 && dot(self.velocity, trace.normal) > 10.0
        {
            self.ground_entity = NONE;
            return;
        }
        if trace.fraction < 1.0 && !trace.start_solid && trace.normal[2] >= 0.7 {
            self.ground_entity = WORLD;
            self.ground_normal = trace.normal;
            self.surface_flags = trace.surface_flags;
            self.flags &= !0x2000;
        } else {
            self.ground_entity = NONE;
        }
    }
    fn step_slide(&mut self, c: &Collision, dt: f32, gravity: bool) {
        let start = self.origin;
        let velocity = self.velocity;
        let on_ground = self.grounded();
        let blocked = self.slide(c, dt, gravity);
        let down_origin = self.origin;
        let down_velocity = self.velocity;
        if gravity {
            // Slide already swept the complete time interval. Sweeping another
            // velocity*dt here would land on a floor only reached next frame.
            // GroundTrace's 0.25-unit classification includes the sweep skin.
            return;
        }
        let height = if self.stance == Stance::Prone {
            10.0
        } else {
            18.0
        };
        let mut step_up = 0.0;
        // Target 0x421680 attempts a step on a blocked move or a steep
        // ground plane, with a one-unit headroom margin.
        if blocked || (on_ground && self.ground_normal[2] < 0.9) {
            let up = c.trace(
                start,
                add(start, [0.0, 0.0, height + 1.0]),
                self.stance.height(),
            );
            step_up = up.fraction * (height + 1.0) - 1.0;
            if up.start_solid || step_up < 1.0 {
                step_up = 0.0;
            } else {
                self.origin = add(start, [0.0, 0.0, step_up]);
                self.velocity = velocity;
                self.slide(c, dt, gravity);
            }
        }
        if on_ground || step_up != 0.0 {
            let distance = step_up + if on_ground { 9.0 } else { 0.0 };
            let down = c.trace(
                self.origin,
                add(self.origin, [0.0, 0.0, -distance]),
                self.stance.height(),
            );
            if down.start_solid || (down.fraction < 1.0 && down.normal[2] < 0.3) {
                self.origin = down_origin;
                self.velocity = down_velocity;
                return;
            }
            if down.fraction == 1.0 {
                self.origin[2] -= step_up;
            } else {
                self.origin[2] -= distance * down.fraction;
                self.velocity = project_ground(self.velocity, down.normal);
            }
        }
        // Compare progress along the initial velocity, not squared distance:
        // a sideways deflection must not win over a useful forward move.
        let slide_dot =
            (down_origin[0] - start[0]) * velocity[0] + (down_origin[1] - start[1]) * velocity[1];
        let step_dot =
            (self.origin[0] - start[0]) * velocity[0] + (self.origin[1] - start[1]) * velocity[1];
        if slide_dot + 0.001 >= step_dot {
            self.origin = down_origin;
            self.velocity = down_velocity;
            if on_ground {
                let down = c.trace(
                    self.origin,
                    add(self.origin, [0.0, 0.0, -9.0]),
                    self.stance.height(),
                );
                if down.fraction < 1.0 && !down.start_solid {
                    self.origin[2] -= 9.0 * down.fraction;
                    self.velocity = clip(self.velocity, down.normal);
                }
            }
        }
        let dz = self.origin[2] - down_origin[2];
        if on_ground && dz.abs() > 0.5 && dz.round() != 0.0 {
            // The client slows a real step. Omitting this produces recurring
            // prediction corrections when climbing stairs.
            let scale = (1.0 - (self.origin[2] - start[2]).abs() / height) * 0.8 + 0.2;
            self.velocity = mul(self.velocity, scale);
        }
    }
    fn slide(&mut self, c: &Collision, dt: f32, gravity: bool) -> bool {
        let mut end_velocity = self.velocity;
        if gravity {
            end_velocity[2] -= f32::from(self.gravity) * dt;
            self.velocity[2] = (self.velocity[2] + end_velocity[2]) * 0.5;
        }
        let mut planes = Vec::with_capacity(8);
        if self.grounded() {
            planes.push(self.ground_normal);
        }
        let mut time = dt;
        let mut blocked = false;
        for _ in 0..4 {
            let trace = c.trace(
                self.origin,
                add(self.origin, mul(self.velocity, time)),
                self.stance.height(),
            );
            if trace.all_solid {
                self.velocity = [0.0; 3];
                return true;
            }
            self.origin = add(self.origin, mul(self.velocity, time * trace.fraction));
            if trace.fraction == 1.0 {
                break;
            }
            blocked = true;
            time *= 1.0 - trace.fraction;
            if planes.iter().any(|&n| dot(n, trace.normal) > 0.999) {
                self.velocity = add(clip(self.velocity, trace.normal), mul(trace.normal, 0.01));
                continue;
            }
            planes.push(trace.normal);
            self.velocity = clip_planes(self.velocity, &planes);
            end_velocity = clip_planes(end_velocity, &planes);
            if length(self.velocity) < 0.001 {
                break;
            }
        }
        if gravity {
            self.velocity = end_velocity;
        }
        blocked
    }
}
fn clip(v: V3, n: V3) -> V3 {
    let d = dot(v, n);
    // Target subtracts abs(dot)*0.001; dividing a positive dot by 1.001 is
    // close but is a different operation after repeated ground clips.
    sub(v, mul(n, d - d.abs() * 0.001))
}
fn project_ground(v: V3, n: V3) -> V3 {
    let horizontal_squared = v[0] * v[0] + v[1] * v[1];
    if n[2].abs() < 0.001 || horizontal_squared == 0.0 {
        return v;
    }
    let z = -(v[0] * n[0] + v[1] * n[1]) / n[2];
    let result = [v[0], v[1], z];
    let scale = ((v[2] * v[2] + horizontal_squared) / (z * z + horizontal_squared)).sqrt();
    if scale < 1.0 || z < 0.0 || v[2] > 0.0 {
        mul(result, scale)
    } else {
        v
    }
}
fn clip_planes(v: V3, planes: &[V3]) -> V3 {
    let mut result = v;
    for (i, &n) in planes.iter().enumerate() {
        if dot(result, n) >= 0.0 {
            continue;
        }
        result = clip(result, n);
        for (j, &other) in planes.iter().enumerate() {
            if i == j || dot(result, other) >= 0.0 {
                continue;
            }
            result = clip(result, other);
            if dot(result, n) < 0.0 {
                let crease = normalized(cross(n, other));
                result = mul(crease, dot(v, crease));
            }
            if planes.iter().any(|&p| dot(result, p) < -0.01) {
                return [0.0; 3];
            }
        }
    }
    result
}
