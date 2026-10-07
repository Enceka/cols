//! Minimal single-M4 ammo/reload/ADS ownership. No hit events or physics yet.
use codol_protocol::{
    game_snapshot::{Aim, Equipment, Firing},
    game_usercmd::UserCommand,
};

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReloadTiming {
    pub normal_msec: u16,
    pub empty_msec: u16,
    pub add_msec: u16,
    pub empty_add_msec: u16,
}
impl ReloadTiming {
    pub fn valid(&self) -> bool {
        (1..=15000).contains(&self.normal_msec)
            && (1..=15000).contains(&self.empty_msec)
            && (1..=self.normal_msec).contains(&self.add_msec)
            && (1..=self.empty_msec).contains(&self.empty_add_msec)
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdsTiming {
    pub in_rate: f32,
    pub out_rate: f32,
    pub reload_allowed: bool,
    pub reload_interruptible: bool,
    pub reload_end_msec: u16,
}
impl AdsTiming {
    pub fn valid(&self) -> bool {
        [self.in_rate, self.out_rate]
            .into_iter()
            .all(|r| r.is_finite() && r > 0.0 && r <= 1.0)
            && self.reload_end_msec <= 15000
    }
}

#[derive(Debug)]
pub(crate) struct Player {
    equipment: Equipment,
    fire_interval: Option<u16>,
    last_time: u32,
    reload: Option<ReloadTiming>,
    magazine_capacity: u16,
    reload_transfer_pending: bool,
    ads: Option<AdsTiming>,
}

impl Player {
    pub fn new(
        mut equipment: Equipment,
        fire_interval: Option<u16>,
        reload: Option<ReloadTiming>,
        ads: Option<AdsTiming>,
    ) -> Self {
        equipment.aim = ads.map(|_| Aim::default());
        if fire_interval.is_some() {
            equipment.firing = Some(Firing {
                animation: 0x400,
                remaining_msec: 0,
                delay_msec: 0,
                state: 0,
            });
        }
        Self {
            magazine_capacity: equipment.ammo.map_or(0, |a| a.clip),
            equipment,
            fire_interval,
            last_time: 1000,
            reload,
            reload_transfer_pending: false,
            ads,
        }
    }

    pub fn equipment(&self) -> Equipment {
        self.equipment
    }

    pub fn apply(&mut self, cmd: &UserCommand) {
        if cmd.time <= self.last_time {
            return;
        }
        let elapsed = cmd.time - self.last_time;
        self.last_time = cmd.time;
        self.apply_weapon(cmd, elapsed);
        self.apply_aim(cmd, elapsed);
    }

    fn apply_aim(&mut self, cmd: &UserCommand, elapsed: u32) {
        let (Some(timing), Some(aim)) = (self.ads, self.equipment.aim.as_mut()) else {
            return;
        };
        let firing = self.equipment.firing;
        let reloading = firing.is_some_and(|f| f.state == 8);
        // Verified PMF_ADS_BUTTON0x10 and input BUTTON_ADS0x800. This
        // bootstrap only executes normal, firing and reload weapon states.
        aim.button = cmd.buttons & 0x800 != 0
            && cmd.buttons & (4 | 0x10000000) == 0
            && cmd.weapons[0] == self.equipment.weapon;
        aim.movement_slow = aim.button && !reloading;
        let reload_blocks = reloading
            && !timing.reload_allowed
            && (timing.reload_interruptible
                || firing.is_some_and(|f| f.remaining_msec > timing.reload_end_msec));
        // 0x4224e0: elapsed msec times the registered per-msec rate, clamped
        // to exact0/1. No ability, turret or offhand modifiers are guessed.
        aim.fraction = if aim.button && !reload_blocks {
            (aim.fraction + elapsed as f32 * timing.in_rate).min(1.0)
        } else {
            (aim.fraction - elapsed as f32 * timing.out_rate).max(0.0)
        };
    }

    fn apply_weapon(&mut self, cmd: &UserCommand, elapsed: u32) {
        let (Some(interval), Some(ammo), Some(firing)) = (
            self.fire_interval,
            self.equipment.ammo.as_mut(),
            self.equipment.firing.as_mut(),
        ) else {
            return;
        };
        firing.remaining_msec = firing
            .remaining_msec
            .saturating_sub(elapsed.min(u32::from(u16::MAX)) as u16);
        firing.delay_msec = firing
            .delay_msec
            .saturating_sub(elapsed.min(u32::from(u16::MAX)) as u16);
        if firing.state == 8 {
            if self.reload_transfer_pending && firing.delay_msec == 0 {
                let transfer = self
                    .magazine_capacity
                    .saturating_sub(ammo.clip)
                    .min(ammo.reserve);
                ammo.clip += transfer;
                ammo.reserve -= transfer;
                self.reload_transfer_pending = false;
            }
            if firing.remaining_msec == 0 {
                firing.state = 0;
                firing.animation = (firing.animation ^ 0x400) & 0x400 | 1;
            }
            return;
        }
        // Target 0x45f660: attack=1, menu capture=0x10000000.
        // Only the equipped M4 is supported, not offhand/melee/weapon switching.
        if firing.remaining_msec == 0 {
            if let Some(timing) = self.reload
                && cmd.buttons & (4 | 0x10000000) == 0
                && cmd.weapons[0] == self.equipment.weapon
                && ammo.clip < self.magazine_capacity
                && ammo.reserve > 0
                // 3.3.3.3 VA0x424660: test byte ptr [pmove+8],0x10.
                // pmove+4 is cmd.time, +8 is cmd.buttons. 0x20 is not R.
                && (cmd.buttons & 0x10 != 0 || ammo.clip == 0)
            {
                let empty = ammo.clip == 0;
                // Target0x4232d0: state8, animation13 normal /15 empty;
                // 0x4230a0 gives the earlier ammunition transfer marker.
                firing.state = 8;
                firing.animation = (firing.animation ^ 0x400) & 0x400 | if empty { 15 } else { 13 };
                firing.remaining_msec = if empty {
                    timing.empty_msec
                } else {
                    timing.normal_msec
                };
                firing.delay_msec = if empty {
                    timing.empty_add_msec
                } else {
                    timing.add_msec
                };
                self.reload_transfer_pending = true;
                return;
            }
            if cmd.buttons & (1 | 4 | 0x10000000) == 1
                && cmd.weapons[0] == self.equipment.weapon
                && ammo.clip > 0
            {
                ammo.clip -= 1;
                // 0x4253e0 sets weaponTime from def+0xd10, state6. The zero
                // fire-delay M4 path is the only admitted timing profile.
                firing.remaining_msec = interval;
                firing.state = 6;
                firing.animation =
                    (firing.animation ^ 0x400) & 0x400 | if ammo.clip == 0 { 3 } else { 2 };
            } else if firing.state != 0 {
                firing.state = 0;
                firing.animation = (firing.animation ^ 0x400) & 0x400 | 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codol_protocol::game_snapshot::Ammo;
    fn player() -> Player {
        Player::new(
            Equipment {
                weapon: 352,
                aim: None,
                viewmodel: None,
                firing: None,
                ammo: Some(Ammo {
                    clip_key: 352,
                    reserve_key: 163,
                    clip: 30,
                    reserve: 90,
                }),
            },
            Some(79),
            None,
            None,
        )
    }
    fn cmd(time: u32, buttons: u32) -> UserCommand {
        UserCommand {
            time,
            buttons,
            weapons: [352, 0, 0],
            ..Default::default()
        }
    }
    fn aiming_player() -> Player {
        let mut p = player();
        p.ads = Some(AdsTiming {
            in_rate: 0.004,
            out_rate: 0.004,
            reload_allowed: false,
            reload_interruptible: false,
            reload_end_msec: 349,
        });
        p.equipment.aim = Some(Aim::default());
        p.reload = Some(reload_timing());
        p
    }
    #[test]
    fn ads_progress_survives_frames_reverses_smoothly_and_reaches_exact_endpoints() {
        let mut p = aiming_player();
        p.apply(&cmd(1100, 0x800));
        assert!((p.equipment().aim.unwrap().fraction - 0.4).abs() < 1e-6);
        p.apply(&cmd(1100, 0x800)); // duplicate does not advance ADS
        assert!((p.equipment().aim.unwrap().fraction - 0.4).abs() < 1e-6);
        p.apply(&cmd(1150, 0));
        let aim = p.equipment().aim.unwrap();
        assert!(!aim.button && (aim.fraction - 0.2).abs() < 1e-6);
        p.apply(&cmd(1500, 0x800));
        assert_eq!(p.equipment().aim.unwrap().fraction, 1.0);
        p.apply(&cmd(1800, 0));
        assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
        p.apply(&cmd(2200, 0x10000800));
        assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
    }
    #[test]
    fn ads_leaves_scope_during_reload_and_reenters_only_at_target_threshold() {
        let mut p = aiming_player();
        p.apply(&cmd(1250, 0x800));
        p.apply(&cmd(1300, 0x801)); // make room for a partial reload
        p.apply(&cmd(1400, 0x810));
        assert_eq!(p.equipment().firing.unwrap().state, 8);
        p.apply(&cmd(1700, 0x800));
        assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
        p.apply(&cmd(3079, 0x800)); //350ms left, still blocked
        assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
        p.apply(&cmd(3080, 0x800)); //349ms left, allowed
        assert!((p.equipment().aim.unwrap().fraction - 0.004).abs() < 1e-6);
        p.apply(&cmd(3330, 0x800));
        assert_eq!(p.equipment().aim.unwrap().fraction, 1.0);
        assert_eq!(p.equipment().ammo.unwrap().clip, 30);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 89);
    }
    #[test]
    fn shots_obey_cooldown_and_duplicate_inputs_never_consume_twice() {
        let mut p = player();
        for c in [
            cmd(1010, 1),
            cmd(1010, 1),
            cmd(1005, 1),
            cmd(1050, 1),
            cmd(1088, 1),
        ] {
            p.apply(&c);
        }
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&cmd(1089, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
        p.apply(&cmd(1180, 0));
        for time in (1200..2200).step_by(50) {
            p.apply(&cmd(time, 0));
            assert_eq!(p.equipment().ammo.unwrap().clip, 28);
        }
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 90);
    }
    #[test]
    fn empty_clip_menu_and_weapon_switch_do_not_create_ammo_or_shots() {
        let mut p = player();
        p.apply(&cmd(1100, 0x10000001));
        let mut other = cmd(1200, 1);
        other.weapons[0] = 180;
        p.apply(&other);
        assert_eq!(p.equipment().ammo.unwrap().clip, 30);
        for time in (1300..=6000).step_by(100) {
            p.apply(&cmd(time, 1));
        }
        assert_eq!(p.equipment().ammo.unwrap().clip, 0);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 90);
        assert_eq!(player().equipment().ammo.unwrap().clip, 30); // new match only
    }
    fn reload_timing() -> ReloadTiming {
        ReloadTiming {
            normal_msec: 2029,
            empty_msec: 2799,
            add_msec: 1399,
            empty_add_msec: 1399,
        }
    }
    #[test]
    fn reload_transfers_ammo_once_at_marker_and_blocks_fire_until_finished() {
        let mut p = player();
        p.reload = Some(reload_timing());
        p.apply(&cmd(1100, 1));
        p.apply(&cmd(1200, 0x10));
        assert_eq!(p.equipment().firing.unwrap().state, 8);
        p.apply(&cmd(2598, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&cmd(2599, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 30);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 89);
        p.apply(&cmd(2599, 1)); // repeated marker must not transfer twice
        p.apply(&cmd(3228, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 30);
        assert_eq!(p.equipment().firing.unwrap().state, 8);
        p.apply(&cmd(3229, 0));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        p.apply(&cmd(3230, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 89);
    }
    #[test]
    fn empty_reload_uses_longer_duration_and_cannot_create_missing_reserve() {
        let mut p = player();
        p.reload = Some(reload_timing());
        let ammo = p.equipment.ammo.as_mut().unwrap();
        ammo.clip = 0;
        ammo.reserve = 7;
        p.apply(&cmd(1100, 0)); // empty-magazine automatic reload
        assert_eq!(p.equipment().firing.unwrap().remaining_msec, 2799);
        assert_eq!(p.equipment().firing.unwrap().animation & !0x400, 15);
        p.apply(&cmd(2499, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 7);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 0);
        p.apply(&cmd(3899, 0));
        p.apply(&cmd(4000, 0x10));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        assert_eq!(p.equipment().ammo.unwrap().clip, 7);
    }
    #[test]
    fn target_reload_button_starts_partial_magazine_reload_but_bit32_does_not() {
        let mut p = player();
        p.reload = Some(reload_timing());
        p.apply(&cmd(1100, 1));
        p.apply(&cmd(1200, 0x20));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&cmd(1300, 0x10));
        assert_eq!(p.equipment().firing.unwrap().state, 8);
        assert_eq!(p.equipment().firing.unwrap().animation & !0x400, 13);
        p.apply(&cmd(2699, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 30);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 89);
    }
}
