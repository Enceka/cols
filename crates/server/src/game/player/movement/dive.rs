//! Native3.3.3.3 dive:0x40dca0..0x40e340, timed by the executing subcommand.
use super::*;
use codol_protocol::game::snapshot::events::{DTP_LAND, DTP_LAUNCH, Event};
#[cfg(test)]
mod tests;

pub(crate) const FLAG: u32 = 0x4000_0000;
const MIN_SPEED: f32 = 3.16;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Rules {
    pub prone_blocked: bool,
    pub height_multiplier: f32,
}
impl Default for Rules {
    fn default() -> Self {
        Self {
            prone_blocked: false,
            height_multiplier: 1.0,
        }
    }
}

/// Motion/weapon ordering within a long command. Ordinary commands retain the
/// existing weapon update; a dive visits every native66ms movement substep.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frame {
    pub time: u32,
    pub active: bool,
    pub sliding: bool,
    pub sprinting: bool,
}

impl Motion {
    pub(crate) fn set_dive_rules(&mut self, rules: Rules) {
        self.dive_rules = rules;
    }
    pub(crate) fn diving(&self) -> bool {
        self.flags & FLAG != 0
    }
    pub(crate) fn dive_sliding(&self) -> bool {
        self.diving() && (self.jump_time as i32) < 0
    }
    pub(crate) fn dive_locked(&self) -> bool {
        !self.diving()
            && self.dive_end_time != 0
            && self.command_time.wrapping_sub(self.dive_end_time) < 100
    }
    pub(crate) fn take_dive_frames(&mut self) -> Vec<Frame> {
        std::mem::take(&mut self.dive_frames)
    }
    pub(crate) fn take_movement_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.movement_events)
    }
    fn dive_surface(&self) -> u32 {
        const MAP: [u32; 31] = [
            0, 3, 2, 4, 4, 2, 5, 2, 7, 5, 7, 5, 4, 1, 8, 2, 2, 2, 5, 4, 8, 3, 2, 2, 2, 2, 5, 5, 1,
            1, 8,
        ];
        let index = if self.surface_flags & 0x2000 != 0 {
            0
        } else {
            (self.surface_flags >> 20) & 31
        };
        MAP.get(index as usize).copied().unwrap_or(0)
    }
    fn finish_dive(&mut self) {
        self.flags &= !FLAG;
        self.velocity = [0.0; 3];
        self.jump_time = 0;
        self.dive_end_time = self.command_time;
        self.sprint.button_up_required = false;
    }
    /// Death, linked/remote movement and frozen match states cannot carry a
    /// live dive into the next lifecycle.0x40dd80 bypasses the normal lockout.
    pub(crate) fn cancel_dive(&mut self) {
        if self.diving() {
            self.finish_dive();
            self.dive_end_time = self.command_time.saturating_sub(101);
        }
        self.dive_frames.clear();
        self.movement_events.clear();
    }
    pub(super) fn finish_slow_dive(&mut self) {
        if self.dive_sliding() && dot(self.velocity, self.velocity) < MIN_SPEED * MIN_SPEED {
            self.finish_dive();
        }
    }
    pub(super) fn check_dive(&mut self, cmd: &UserCommand) {
        if self.diving() {
            if (self.jump_time as i32) > 0 {
                self.jump_time = (-(self.command_time as i32)) as u32;
                self.movement_events.push(Event {
                    id: DTP_LAND,
                    parameter: self.dive_surface(),
                });
            }
            return;
        }
        let v = self.velocity;
        let horizontal = v[0] * v[0] + v[1] * v[1];
        let elapsed = self.command_time.wrapping_sub(self.dive_end_time) as i32;
        if self.dive_rules.prone_blocked
            || self.flags & 0x0200_0000 != 0
            || cmd.buttons & 0x0080_0000 == 0
            || self.sprint.last_end != self.command_time
            || horizontal <= MIN_SPEED * MIN_SPEED
            || (0..=1500).contains(&elapsed)
            || self.command_time.wrapping_sub(self.sprint.last_start) <= 250
        {
            return;
        }
        let yaw = f32::from(cmd.angles[1]) * (std::f32::consts::TAU / 65536.0);
        if yaw.cos() * (v[0] / horizontal.sqrt()) + yaw.sin() * (v[1] / horizontal.sqrt()) < 0.98 {
            return;
        }
        self.ground_entity = NONE;
        self.jump_time = self.command_time;
        self.jump_origin_z = self.origin[2];
        self.velocity[2] =
            ((f64::from(self.gravity) * f64::from(self.dive_height())).sqrt() as f32) * 1.5;
        self.flags = (self.flags | FLAG) & !0x2180;
        self.pm_time = 0;
        self.movement_events.push(Event {
            id: DTP_LAUNCH,
            parameter: self.dive_surface(),
        });
    }
    fn dive_height(&self) -> f32 {
        39.0 * self.dive_rules.height_multiplier
    }
    pub(super) fn dive_apex(&mut self) {
        if self.diving() && self.origin[2] > self.jump_origin_z + self.dive_height() {
            if (self.command_time as i32).wrapping_sub(self.jump_time as i32) < 400 {
                self.velocity[2] = 0.0;
            } else {
                self.velocity[2] = self.velocity[2].min(0.0);
            }
        }
    }
    pub(super) fn dive_friction(&mut self, collision: &Collision) -> f32 {
        if !self.dive_sliding() {
            return 1.0;
        }
        let elapsed = (self.command_time as i32).wrapping_add(self.jump_time as i32);
        let tr = collision.trace_mask(
            add(self.origin, [0.0, 0.0, 1.0]),
            add(self.origin, [0.0, 0.0, -1.0]),
            self.stance.height(),
            0x0100_0000,
        );
        if elapsed < 300 {
            if tr.fraction < 1.0 {
                0.0
            } else {
                elapsed.max(0) as f32 / 300.0
            }
        } else {
            self.finish_dive();
            1.0
        }
    }
}
