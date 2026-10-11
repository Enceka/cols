//! Target 3.3.3.3 sprint meter and input gates, verified at0x414bd0/0x4160f0.
use codol_protocol::game_snapshot::SprintState;

#[derive(Debug, Clone, Copy)]
pub struct Rules {
    pub duration_seconds: f32,
    pub weapon_duration_scale: f32,
    pub force_recover_seconds: Option<f32>,
    pub always: bool,
    pub weapon_state: u8,
}
impl Default for Rules {
    fn default() -> Self {
        Self {
            duration_seconds: 4.0,
            weapon_duration_scale: 1.0,
            force_recover_seconds: None,
            always: false,
            weapon_state: 0,
        }
    }
}
impl Rules {
    pub fn maximum(self) -> u16 {
        if self.duration_seconds < 0.0 || self.always {
            return 0;
        }
        // Native x87 stores the product as float32 before adding the double
        // epsilon0xc857b8 and FISTP (ties-to-even). WeaponDef+0x7d0 scales it.
        let product = (f64::from(self.duration_seconds)
            * 1000.0
            * f64::from(self.weapon_duration_scale)) as f32;
        (f64::from(product) + 9.313225746154785e-10)
            .round_ties_even()
            .clamp(0.0, f64::from(u16::MAX)) as u16
    }
}

pub fn remaining(state: SprintState, now: u32, rules: Rules) -> u16 {
    let maximum = rules.maximum();
    if maximum == 0 {
        return 0x3fff;
    }
    if state.last_start == 0 {
        return maximum;
    }
    let value = if state.last_start > state.last_end {
        u32::from(state.start_max_length)
            .wrapping_sub(now)
            .wrapping_add(state.last_start)
    } else {
        // This build's exhausted-meter recharge pause is zero. Ability33
        // makes the meter full once the recovered amount exceeds its threshold.
        let value = u32::from(state.start_max_length)
            .wrapping_sub(state.last_end.wrapping_mul(2))
            .wrapping_add(state.last_start)
            .wrapping_add(now);
        if rules
            .force_recover_seconds
            .is_some_and(|threshold| f64::from(value as i32 as f32) > f64::from(threshold) * 1000.0)
        {
            return maximum;
        }
        value
    };
    (value as i32).clamp(0, i32::from(maximum)) as u16
}

pub fn stop(state: &mut SprintState, now: u32, pressed: bool) {
    state.delay = false;
    state.last_end = now;
    state.button_up_required |= pressed;
}

/// Forward movement, stance, ADS and shock gates are classified by Motion.
pub fn update(
    state: &mut SprintState,
    now: u32,
    buttons: u32,
    previous_buttons: u32,
    active: bool,
    blocked: bool,
    rules: Rules,
) -> bool {
    let pressed = buttons & 2 != 0;
    if !pressed {
        state.button_up_required = false;
    }
    if active {
        if rules.maximum() != 0
            && now.wrapping_sub(state.last_start) >= u32::from(state.start_max_length)
        {
            stop(state, now, pressed);
            state.delay = true;
            return false;
        }
        if blocked
            || buttons & 0x1cf35 != 0
            || (13..=21).contains(&rules.weapon_state)
            || matches!(rules.weapon_state, 29 | 30)
            || (!rules.always && pressed && previous_buttons & 2 == 0)
        {
            stop(state, now, pressed);
            return false;
        }
        return true;
    }
    if blocked
        || (!rules.always && (!pressed || state.button_up_required))
        || buttons & 0x1cc35 != 0
        || (13..=21).contains(&rules.weapon_state)
    {
        return false;
    }
    let left = remaining(*state, now, rules);
    if left <= 1000 {
        return false;
    }
    state.start_max_length = left;
    state.last_start = now;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_meter_expires_and_requires_release_before_recovery() {
        let mut s = SprintState::default();
        let rules = Rules::default();
        assert!(update(&mut s, 1000, 2, 0, false, false, rules));
        assert_eq!((s.last_start, s.start_max_length), (1000, 4000));
        assert!(update(&mut s, 4999, 2, 2, true, false, rules));
        assert!(!update(&mut s, 5000, 2, 2, true, false, rules));
        assert!(s.delay && s.button_up_required);
        assert_eq!(remaining(s, 5500, rules), 500);
        assert!(!update(&mut s, 9000, 2, 2, false, false, rules));
        assert!(!update(&mut s, 9001, 0, 2, false, false, rules));
        assert!(update(&mut s, 9002, 2, 0, false, false, rules));
        assert_eq!(s.start_max_length, 4000);
    }
    #[test]
    fn sprint_is_toggled_and_actions_interrupt_it() {
        for action in [1, 4, 0x10, 0x200, 0x400, 0x4000, 0x10000] {
            let mut s = SprintState::default();
            let rules = Rules::default();
            assert!(update(&mut s, 1000, 2, 0, false, false, rules));
            assert!(update(&mut s, 1100, 0, 2, true, false, rules));
            assert!(!update(&mut s, 1200, action, 0, true, false, rules));
            assert_eq!(s.last_end, 1200);
            assert_eq!(remaining(s, 1250, rules), 3850);
        }
        let mut s = SprintState::default();
        assert!(update(&mut s, 1000, 2, 0, false, false, Rules::default()));
        assert!(!update(&mut s, 1100, 2, 0, true, false, Rules::default()));
    }
    #[test]
    fn marathon_is_unlimited_but_keeps_action_and_press_gates() {
        let rules = Rules {
            duration_seconds: -1.0,
            ..Default::default()
        };
        let mut s = SprintState::default();
        assert_eq!(rules.maximum(), 0);
        assert!(update(&mut s, 1000, 2, 0, false, false, rules));
        assert!(update(&mut s, 61_000, 2, 2, true, false, rules));
        assert_eq!(remaining(s, 61_000, rules), 0x3fff);
        assert!(!update(&mut s, 61_001, 3, 2, true, false, rules));
        let always = Rules {
            always: true,
            ..Default::default()
        };
        assert!(update(&mut s, 62_000, 0, 0, false, false, always));
    }
    #[test]
    fn partial_recharge_threshold_and_weapon_scaling_match_native_limits() {
        let rules = Rules {
            weapon_duration_scale: 0.5,
            force_recover_seconds: Some(1.0),
            ..Default::default()
        };
        let mut s = SprintState::default();
        assert_eq!(rules.maximum(), 2000);
        assert!(update(&mut s, 1000, 2, 0, false, false, rules));
        assert!(!update(&mut s, 3000, 2, 2, true, false, rules));
        assert_eq!(remaining(s, 4000, rules), 1000);
        assert_eq!(remaining(s, 4001, rules), 2000);
        assert_eq!(
            Rules {
                weapon_duration_scale: 0.0,
                ..Default::default()
            }
            .maximum(),
            0
        );
        for phase in 13..=21 {
            assert!(!update(
                &mut s,
                5000,
                2,
                0,
                false,
                false,
                Rules {
                    weapon_state: phase,
                    ..rules
                }
            ));
        }
    }
}
