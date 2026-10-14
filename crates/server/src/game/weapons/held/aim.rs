use super::*;

impl HeldWeapons {
    pub fn set_spread_timing(&mut self, timing: SpreadTiming) {
        self.current.spread = Some(timing);
    }
    pub fn spread_environment(&mut self, motion: &crate::game::player::movement::Motion) {
        self.spread_stance = match motion.stance {
            crate::game::player::movement::Stance::Stand => 0,
            crate::game::player::movement::Stance::Crouch => 1,
            crate::game::player::movement::Stance::Prone => 2,
        };
        self.spread_speed =
            motion.velocity[0].hypot(motion.velocity[1]) / f32::from(motion.speed.max(1));
        self.spread_airborne = motion.ground_entity == 2047;
    }
    /// Target0x422aa0/0x425840: PS+0x350 uses0..255. Movement and
    /// turning add spread; stationary recovery and stance affect decay.
    pub(super) fn update_spread(&mut self, cmd: &UserCommand, elapsed: u32) {
        let Some(timing) = self.current.spread else {
            return;
        };
        let seconds = elapsed.min(200) as f32 * 0.001;
        let ads = self
            .current
            .equipment
            .aim
            .is_some_and(|a| a.fraction == 1.0);
        let turn = self.last_angles.map_or(0.0, |old| {
            (0..2)
                .map(|i| {
                    f32::from(cmd.angles[i].wrapping_sub(old[i]) as i16).abs() * 360.0 / 65536.0
                })
                .sum::<f32>()
        }) * timing.turn_add
            * 0.01;
        let movement = if !ads {
            timing.move_add * self.spread_speed * seconds + turn
        } else {
            0.0
        };
        let change = if self.spread_airborne && !ads {
            movement + 2.56 * seconds
        } else if movement > 0.0 {
            movement
        } else {
            -timing.decay * timing.stance_decay[self.spread_stance] * seconds
        };
        self.spread_scale = (self.spread_scale + change * 255.0).clamp(0.0, 255.0);
    }
    pub(super) fn shot_spread(&self) -> f32 {
        let Some(timing) = self.current.spread else {
            return self.current.ballistics.map_or(0.0, |b| b.spread_degrees);
        };
        let scale = self.spread_scale / 255.0;
        let multiplier = self.perks.value("modBulletSpreadScale", 1.0);
        let min = timing.minimum[self.spread_stance] * multiplier;
        let max = timing.maximum[self.spread_stance] * multiplier;
        if self
            .current
            .equipment
            .aim
            .is_some_and(|a| a.fraction == 1.0)
        {
            timing.ads + (max - timing.ads) * scale
        } else {
            min + (max - min) * scale
        }
    }
}
