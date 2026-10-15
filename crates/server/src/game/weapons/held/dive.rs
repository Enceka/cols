use super::*;

/// Native WeaponDef+0x784/+0x78c and prone veto+d88. Zero timings are valid.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiveTiming {
    pub in_msec: u16,
    pub out_msec: u16,
    pub prone_blocked: bool,
}
impl DiveTiming {
    pub fn valid(self) -> bool {
        self.in_msec <= 15000 && self.out_msec <= 15000
    }
}

impl HeldWeapons {
    pub fn set_dive_timing(&mut self, timing: DiveTiming) {
        assert!(timing.valid());
        self.current.dive_timing = Some(timing);
    }
    pub fn dive_rules(&self) -> crate::game::player::movement::DiveRules {
        crate::game::player::movement::DiveRules {
            prone_blocked: self.current.dive_timing.is_none_or(|t| t.prone_blocked),
            height_multiplier: self.perks.value("modMultiplyJumpHeight", 1.0),
        }
    }
    pub(crate) fn set_diving(&mut self, active: bool, sliding: bool) {
        self.diving = active;
        self.dive_sliding = active && sliding;
    }
    /// Movement has already visited these subcommands. Replay their dive
    /// phases before advancing the weapon clock to the final command time.
    pub(crate) fn apply_with_dive_frames(
        &mut self,
        cmd: &UserCommand,
        approach: Option<MeleeApproach>,
        frames: &[crate::game::player::movement::dive::Frame],
    ) {
        if cmd.time <= self.last_time {
            return;
        }
        if frames.is_empty() {
            self.set_diving(false, false);
            self.apply_with_melee_approach(cmd, approach);
            return;
        }
        if frames.iter().any(|f| f.active)
            || self.diving
            || self
                .current
                .equipment
                .firing
                .is_some_and(|f| (31..=33).contains(&f.state))
        {
            for frame in frames {
                let mut command = cmd.clone();
                command.time = frame.time;
                self.set_sprinting(frame.sprinting);
                self.set_diving(frame.active, frame.sliding);
                self.apply_with_melee_approach(&command, approach);
            }
        } else {
            if let Some(frame) = frames.last() {
                self.set_diving(frame.active, frame.sliding);
            }
            self.apply_with_melee_approach(cmd, approach);
        }
    }
    ///0x428af0 and0x428df0: dive takes over reload/sprint, and keeps control
    /// through recovery. Landing begins recovery while movement can still slide.
    pub(super) fn apply_dive(&mut self, cmd: &UserCommand, elapsed: u32) -> bool {
        let Some(timing) = self.current.dive_timing else {
            return false;
        };
        let state = self.current.equipment.firing.map_or(0, |f| f.state);
        if (31..=33).contains(&state) {
            let firing = self.current.equipment.firing.as_mut().unwrap();
            firing.remaining_msec = firing
                .remaining_msec
                .saturating_sub(elapsed.min(65535) as u16);
            firing.delay_msec = firing.delay_msec.saturating_sub(elapsed.min(65535) as u16);
            let expired = firing.remaining_msec == 0;
            match state {
                31 if expired => {
                    self.current
                        .animate(32, if self.current.empty() { 39 } else { 36 }, 0, 0)
                }
                32 if !self.diving || self.dive_sliding => self.current.animate(
                    33,
                    if self.current.empty() { 40 } else { 37 },
                    timing.out_msec,
                    0,
                ),
                33 if expired => self.current.ready(),
                _ => {}
            }
            return true;
        }
        if !self.diving
            || self.dive_sliding
            || cmd.weapons[0] == 0
            || matches!(state,1..=7|13..=21)
            || self.selected_offhand.is_some()
        {
            return false;
        }
        if reloading(state) {
            self.emit(2, u32::from(state));
        }
        self.current.cancel_burst();
        self.current.reload_transfer_pending = false;
        self.current.animate(
            31,
            if self.current.empty() { 38 } else { 35 },
            timing.in_msec,
            0,
        );
        true
    }
}
