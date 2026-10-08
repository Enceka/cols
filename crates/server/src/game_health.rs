//! Training recovery adapted from the supplied MP healthoverlay/tweakables
//! behavior. Delay5s; severe wounds heal10% every50ms; light wounds heal fully.
#[derive(Default, Debug)]
pub(crate) struct HealthRecovery {
    last_time: Option<u32>,
    next_tick: Option<u32>,
    severe: bool,
}
impl HealthRecovery {
    fn clock(&mut self, now: u32) -> bool {
        if self
            .last_time
            .is_some_and(|last| now.wrapping_sub(last) >= 0x8000_0000)
        {
            return false;
        }
        self.last_time = Some(now);
        true
    }
    pub fn damaged(&mut self, now: u32, health: u16) {
        if !self.clock(now) {
            return;
        }
        if health == 0 {
            self.next_tick = None;
            self.severe = false;
            return;
        }
        self.severe |= health <= 55;
        self.next_tick = Some(now.wrapping_add(5000));
    }
    pub fn recover(&mut self, now: u32, health: u16) -> u16 {
        if !self.clock(now) || health == 0 {
            return health;
        }
        let Some(next) = self.next_tick else {
            return health;
        };
        let elapsed = now.wrapping_sub(next);
        if elapsed >= 0x8000_0000 {
            return health;
        }
        let ticks = elapsed / 50 + 1;
        let healed = if self.severe {
            u32::from(health)
                .saturating_add(ticks.saturating_mul(10))
                .min(100) as u16
        } else {
            100
        };
        if healed == 100 {
            self.next_tick = None;
            self.severe = false;
        } else {
            self.next_tick = Some(next.wrapping_add(ticks * 50));
        }
        healed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delay_and_repeated_frames_do_not_restore_extra_health() {
        let mut r = HealthRecovery::default();
        r.damaged(1000, 34);
        assert_eq!(r.recover(5999, 34), 34);
        assert_eq!(r.recover(6000, 34), 44);
        assert_eq!(r.recover(6000, 44), 44);
        assert_eq!(r.recover(6150, 44), 74);
        assert_eq!(r.recover(6300, 74), 100);
    }
    #[test]
    fn new_damage_restarts_delay_and_death_never_heals() {
        let mut r = HealthRecovery::default();
        r.damaged(1000, 67);
        assert_eq!(r.recover(5999, 67), 67);
        r.damaged(5999, 34);
        assert_eq!(r.recover(6000, 34), 34);
        assert_eq!(r.recover(10999, 34), 44);
        r.damaged(11000, 0);
        assert_eq!(r.recover(30000, 0), 0);
    }
    #[test]
    fn light_wounds_and_clock_wrap_keep_the_same_life_rules() {
        let mut r = HealthRecovery::default();
        let hit = u32::MAX - 1000;
        r.damaged(hit, 67);
        assert_eq!(r.recover(hit.wrapping_add(4999), 67), 67);
        assert_eq!(r.recover(hit.wrapping_add(5000), 67), 100);
        assert_eq!(r.recover(hit.wrapping_add(5000) - 1, 67), 67);
    }
}
