//! Authoritative held weapons, switching, ammunition, ADS and predictable events.
use codol_protocol::{
    game_events::{self as events, EventQueue},
    game_snapshot::{Aim, Equipment, EquippedWeapon, Firing},
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

/// Target 0x4236a0 / 0x423b10 consume these timings from each weapon's
/// definition (+0x74c/+0x750/+0x768/+0x764), in milliseconds.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SwitchTiming {
    pub drop_msec: u16,
    pub raise_msec: u16,
    pub empty_drop_msec: u16,
    pub empty_raise_msec: u16,
}
impl SwitchTiming {
    pub fn valid(&self) -> bool {
        [
            self.drop_msec,
            self.raise_msec,
            self.empty_drop_msec,
            self.empty_raise_msec,
        ]
        .into_iter()
        .all(|time| (1..=15000).contains(&time))
    }
}

#[derive(Debug)]
struct Weapon {
    equipment: Equipment,
    fire_interval: Option<u16>,
    fire_type: u32,
    reload: Option<ReloadTiming>,
    magazine_capacity: u16,
    reload_transfer_pending: bool,
    ads: Option<AdsTiming>,
    switch: Option<SwitchTiming>,
    move_scale: f32,
    ballistics: Option<crate::game_combat::Ballistics>,
}
impl Weapon {
    fn empty(&self) -> bool {
        self.equipment.ammo.is_some_and(|ammo| ammo.clip == 0)
    }

    fn animate(&mut self, state: u8, animation: u16, remaining_msec: u16, delay_msec: u16) {
        let restart = self
            .equipment
            .firing
            .map_or(0x400, |old| (old.animation ^ 0x400) & 0x400);
        self.equipment.firing = Some(Firing {
            state,
            animation: restart | animation,
            remaining_msec,
            delay_msec,
        });
    }

    fn ready(&mut self) {
        self.reload_transfer_pending = false;
        self.animate(0, 1, 0, 0);
    }
}

#[derive(Debug)]
pub(crate) struct Player {
    current: Weapon,
    secondary: Option<Weapon>,
    last_time: u32,
    events: EventQueue,
    switch_target: Option<u16>,
    attack_held: bool,
    shots: Vec<Shot>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shot {
    pub time: u32,
    pub angles: [u16; 3],
    pub weapon: u16,
    pub ballistics: crate::game_combat::Ballistics,
}

impl Player {
    pub fn new(
        mut equipment: Equipment,
        fire_interval: Option<u16>,
        reload: Option<ReloadTiming>,
        ads: Option<AdsTiming>,
    ) -> Self {
        equipment.aim = ads.map(|_| Aim::default());
        equipment.secondary = None;
        equipment.events = None;
        if fire_interval.is_some() {
            equipment.firing = Some(Firing {
                animation: 0x400,
                remaining_msec: 0,
                delay_msec: 0,
                state: 0,
            });
        }
        Self {
            current: Weapon {
                magazine_capacity: equipment.ammo.map_or(0, |a| a.clip),
                equipment,
                fire_interval,
                fire_type: 0,
                reload,
                reload_transfer_pending: false,
                ads,
                switch: None,
                move_scale: 1.0,
                ballistics: None,
            },
            secondary: None,
            last_time: 1000,
            events: EventQueue::default(),
            switch_target: None,
            attack_held: false,
            shots: Vec::new(),
        }
    }

    pub fn set_switch_timing(&mut self, timing: SwitchTiming) {
        self.current.switch = Some(timing);
    }

    /// WeaponDef +0x48c. Target 0x424830 stops fire types 1/5 after one
    /// shot per trigger; 0 is automatic. Burst modes require their complete
    /// burst/pause state machine, not ordinary automatic-fire timing.
    pub fn set_fire_type(&mut self, fire_type: u32) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(fire_type, 0 | 1 | 5),
            "unsupported weapon fire type {fire_type}: burst/special firing state machine required"
        );
        self.current.fire_type = fire_type;
        Ok(())
    }

    pub fn set_move_scale(&mut self, scale: f32) {
        self.current.move_scale = scale;
    }

    pub fn move_scale(&self) -> f32 {
        self.current.move_scale
    }
    pub fn set_ballistics(&mut self, stats: crate::game_combat::Ballistics) {
        assert!(stats.valid());
        self.current.ballistics = Some(stats);
    }
    pub fn take_shots(&mut self) -> Vec<Shot> {
        std::mem::take(&mut self.shots)
    }

    pub fn with_secondary(mut self, secondary: Player) -> Self {
        if secondary.current.equipment.weapon != self.current.equipment.weapon {
            self.secondary = Some(secondary.current);
        }
        self
    }

    /// Called after a saved backpack has been resolved and published. Preserve
    /// the connection's command clock and event counter across that replacement.
    pub fn replace_loadout(&mut self, primary: Player, secondary: Option<Player>) {
        let old_weapon = self.current.equipment.weapon;
        let sequence = self.events;
        let command_time = self.last_time;
        *self = if let Some(secondary) = secondary {
            primary.with_secondary(secondary)
        } else {
            primary
        };
        self.events = sequence;
        self.last_time = command_time;
        self.emit(
            events::WEAPON_SWITCH_STARTED,
            u32::from(self.current.equipment.weapon),
        );
        if old_weapon != 0 {
            self.emit(events::PUT_AWAY_WEAPON, 0);
        }
        self.begin_raise();
    }

    pub fn equipment(&self) -> Equipment {
        let mut equipment = self.current.equipment;
        equipment.secondary = self.secondary.as_ref().map(|other| EquippedWeapon {
            weapon: other.equipment.weapon,
            ammo: other.equipment.ammo,
        });
        equipment.events = Some(self.events);
        equipment
    }

    pub fn apply(&mut self, cmd: &UserCommand) {
        if cmd.time <= self.last_time {
            return;
        }
        let elapsed = cmd.time - self.last_time;
        self.last_time = cmd.time;
        self.apply_weapon(cmd, elapsed);
        self.apply_aim(cmd, elapsed);
        self.attack_held = cmd.buttons & 1 != 0;
    }

    fn emit(&mut self, id: u8, parameter: u32) {
        self.events
            .push(id, parameter)
            .expect("validated gameplay event");
    }

    /// Movement and weapon events share the player's predictable event ring.
    pub fn movement_event(&mut self, id: u8, parameter: u32) {
        self.emit(id, parameter);
    }

    fn input_allowed(cmd: &UserCommand) -> bool {
        cmd.buttons & (4 | 0x10000000) == 0
    }

    fn apply_aim(&mut self, cmd: &UserCommand, elapsed: u32) {
        let (Some(timing), Some(aim)) = (self.current.ads, self.current.equipment.aim.as_mut())
        else {
            return;
        };
        let firing = self.current.equipment.firing;
        let reloading = firing.is_some_and(|f| f.state == 8);
        let switching = firing.is_some_and(|f| (1..=5).contains(&f.state));
        aim.button = cmd.buttons & 0x800 != 0
            && Self::input_allowed(cmd)
            && !switching
            && cmd.weapons[0] == self.current.equipment.weapon;
        aim.movement_slow = aim.button && !reloading;
        let reload_blocks = reloading
            && !timing.reload_allowed
            && (timing.reload_interruptible
                || firing.is_some_and(|f| f.remaining_msec > timing.reload_end_msec));
        // Target 0x4224e0: elapsed msec times the registered per-msec rate.
        aim.fraction = if aim.button && !reload_blocks {
            (aim.fraction + elapsed as f32 * timing.in_rate).min(1.0)
        } else {
            (aim.fraction - elapsed as f32 * timing.out_rate).max(0.0)
        };
    }

    fn begin_drop(&mut self, requested: u16) {
        let timing = self.current.switch.expect("switching timings checked");
        if self.current.equipment.firing.is_some_and(|f| f.state == 8) {
            self.emit(2, 8); // EV_STOP_WEAPON_SOUND carries the interrupted state.
        }
        self.current.reload_transfer_pending = false;
        self.switch_target = Some(requested);
        self.emit(events::WEAPON_SWITCH_STARTED, u32::from(requested));
        self.emit(events::PUT_AWAY_WEAPON, 0);
        let empty = self.current.empty();
        self.current.animate(
            3,
            if empty { 23 } else { 10 },
            if empty {
                timing.empty_drop_msec
            } else {
                timing.drop_msec
            },
            0,
        );
    }

    fn begin_raise(&mut self) {
        self.current.reload_transfer_pending = false;
        if let Some(timing) = self.current.switch {
            let empty = self.current.empty();
            self.current.animate(
                1,
                if empty { 24 } else { 11 },
                if empty {
                    timing.empty_raise_msec
                } else {
                    timing.raise_msec
                },
                0,
            );
        } else {
            self.current.ready();
        }
        self.current.equipment.aim = self.current.ads.map(|_| Aim::default());
        self.emit(events::RAISE_WEAPON, 0);
    }

    fn apply_weapon(&mut self, cmd: &UserCommand, elapsed: u32) {
        let elapsed = elapsed.min(u32::from(u16::MAX)) as u16;
        if let Some(firing) = self.current.equipment.firing.as_mut() {
            firing.remaining_msec = firing.remaining_msec.saturating_sub(elapsed);
            firing.delay_msec = firing.delay_msec.saturating_sub(elapsed);
        }
        let state = self.current.equipment.firing.map_or(0, |f| f.state);
        let remaining = self
            .current
            .equipment
            .firing
            .map_or(0, |f| f.remaining_msec);
        if state == 3 {
            // FinishWeaponChange reads the latest owned requested weapon. A
            // request to return during the drop raises the original gun again.
            if Self::input_allowed(cmd)
                && (cmd.weapons[0] == self.current.equipment.weapon
                    || self
                        .secondary
                        .as_ref()
                        .is_some_and(|other| other.equipment.weapon == cmd.weapons[0]))
            {
                self.switch_target = Some(cmd.weapons[0]);
            }
            if remaining == 0 {
                let requested = self.switch_target.take();
                if let Some(other) = self.secondary.as_mut()
                    && requested == Some(other.equipment.weapon)
                {
                    self.current.ready();
                    std::mem::swap(&mut self.current, other);
                }
                self.begin_raise();
            }
            return;
        }
        if state == 1 {
            if remaining == 0 {
                self.current.ready();
            }
            return;
        }
        if Self::input_allowed(cmd)
            && self.current.switch.is_some()
            && self.secondary.as_ref().is_some_and(|other| {
                other.equipment.weapon == cmd.weapons[0] && other.switch.is_some()
            })
        {
            self.begin_drop(cmd.weapons[0]);
            return;
        }
        if state == 8 {
            let delay = self.current.equipment.firing.unwrap().delay_msec;
            if self.current.reload_transfer_pending && delay == 0 {
                if let Some(ammo) = self.current.equipment.ammo.as_mut() {
                    let transfer = self
                        .current
                        .magazine_capacity
                        .saturating_sub(ammo.clip)
                        .min(ammo.reserve);
                    ammo.clip += transfer;
                    ammo.reserve -= transfer;
                    if transfer != 0 {
                        self.emit(events::RELOAD_ADD_AMMO, 0);
                    }
                }
                self.current.reload_transfer_pending = false;
            }
            if remaining == 0 {
                self.current.ready();
                self.emit(events::RELOAD_END, 0);
            }
            return;
        }
        if remaining != 0 {
            return;
        }
        let (Some(interval), Some(ammo)) =
            (self.current.fire_interval, self.current.equipment.ammo)
        else {
            return;
        };
        let allowed = Self::input_allowed(cmd) && cmd.weapons[0] == self.current.equipment.weapon;
        if let Some(timing) = self.current.reload
            && allowed && ammo.clip < self.current.magazine_capacity && ammo.reserve > 0
            // Target 0x424660 reads BUTTON_RELOAD 0x10, not 0x20.
            && (cmd.buttons & 0x10 != 0 || ammo.clip == 0)
        {
            let empty = ammo.clip == 0;
            self.emit(events::RESET_ADS, 0);
            self.emit(events::RELOAD_START_NOTIFY, 0);
            self.emit(
                if empty {
                    events::RELOAD_FROM_EMPTY
                } else {
                    events::RELOAD
                },
                0,
            );
            self.current.animate(
                8,
                if empty { 15 } else { 13 },
                if empty {
                    timing.empty_msec
                } else {
                    timing.normal_msec
                },
                if empty {
                    timing.empty_add_msec
                } else {
                    timing.add_msec
                },
            );
            self.current.reload_transfer_pending = true;
            return;
        }
        let trigger = cmd.buttons & 1 != 0 && (self.current.fire_type == 0 || !self.attack_held);
        if allowed && trigger && ammo.clip > 0 {
            let remaining_clip = ammo.clip - 1;
            if let Some(ballistics) = self.current.ballistics {
                self.shots.push(Shot {
                    time: cmd.time,
                    angles: cmd.angles,
                    weapon: self.current.equipment.weapon,
                    ballistics,
                });
            }
            self.current.equipment.ammo.as_mut().unwrap().clip = remaining_clip;
            self.current
                .animate(6, if remaining_clip == 0 { 3 } else { 2 }, interval, 0);
            self.emit(
                if remaining_clip == 0 {
                    events::FIRE_WEAPON_LAST_SHOT
                } else {
                    events::FIRE_WEAPON
                },
                0,
            );
        } else {
            if state != 0 {
                self.current.ready();
            }
            if allowed && cmd.buttons & 1 != 0 && ammo.clip == 0 && !self.attack_held {
                self.emit(events::NO_AMMO, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_authorized_shots_are_forwarded_once_to_the_combat_world() {
        let mut p = player();
        p.set_ballistics(crate::game_combat::Ballistics {
            damage: 33,
            range: 8192.0,
            near_range: 500.0,
            location_multipliers: None,
            falloff: None,
        });
        for c in [
            cmd(1100, 1),
            cmd(1100, 1),
            cmd(1150, 1),
            cmd(1179, 1),
            cmd(1200, 0x10000001),
        ] {
            p.apply(&c);
        }
        let shots = p.take_shots();
        assert_eq!(
            shots.iter().map(|s| s.time).collect::<Vec<_>>(),
            [1100, 1179]
        );
        assert_eq!(shots[0].ballistics.damage, 33);
        assert!(p.take_shots().is_empty());
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
    }
    use codol_protocol::game_snapshot::Ammo;
    fn player() -> Player {
        Player::new(
            Equipment {
                weapon: 352,
                secondary: None,
                events: None,
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
        p.current.ads = Some(AdsTiming {
            in_rate: 0.004,
            out_rate: 0.004,
            reload_allowed: false,
            reload_interruptible: false,
            reload_end_msec: 349,
        });
        p.current.equipment.aim = Some(Aim::default());
        p.current.reload = Some(reload_timing());
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
    fn semiautomatic_modes_require_release_and_a_new_press_for_each_shot() {
        for fire_type in [1, 5] {
            let mut p = player();
            p.set_fire_type(fire_type).unwrap();
            p.apply(&cmd(1100, 1));
            p.apply(&cmd(1100, 1)); // retried input cannot rearm the trigger
            p.apply(&cmd(1200, 1));
            p.apply(&cmd(1300, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 29);
            assert_eq!(p.equipment().events.unwrap().sequence, 1);
            p.apply(&cmd(1301, 0));
            p.apply(&cmd(1302, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 28);
            // An early press must not bypass the firing interval or become
            // repeated fire after it expires while the button stays held.
            p.apply(&cmd(1303, 0));
            p.apply(&cmd(1304, 1));
            p.apply(&cmd(1400, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 28);
            p.apply(&cmd(1401, 0));
            p.apply(&cmd(1402, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 27);
        }
    }

    #[test]
    fn burst_and_special_modes_are_explicitly_rejected_without_changing_fire_mode() {
        let mut p = player();
        p.set_fire_type(1).unwrap();
        for fire_type in [2, 3, 4, 6, 7, u32::MAX] {
            assert!(p.set_fire_type(fire_type).is_err());
        }
        p.apply(&cmd(1100, 1));
        p.apply(&cmd(1200, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
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
        p.current.reload = Some(reload_timing());
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
        p.current.reload = Some(reload_timing());
        let ammo = p.current.equipment.ammo.as_mut().unwrap();
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
        p.current.reload = Some(reload_timing());
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

    fn switchable_player(weapon: u16, scale: f32) -> Player {
        let mut player = player();
        player.current.equipment.weapon = weapon;
        player.current.equipment.ammo.as_mut().unwrap().clip_key = weapon;
        player.current.equipment.ammo.as_mut().unwrap().reserve_key = weapon;
        player.set_switch_timing(SwitchTiming {
            drop_msec: 300,
            raise_msec: 400,
            empty_drop_msec: 500,
            empty_raise_msec: 600,
        });
        player.set_move_scale(scale);
        player
    }

    fn select(time: u32, weapon: u16, buttons: u32) -> UserCommand {
        UserCommand {
            weapons: [weapon, 0, 0],
            ..cmd(time, buttons)
        }
    }

    #[test]
    fn switching_drops_old_weapon_then_raises_owned_weapon_and_preserves_each_magazine() {
        let primary = switchable_player(352, 0.9);
        let secondary = switchable_player(180, 1.0);
        let mut player = primary.with_secondary(secondary);
        player.apply(&select(1100, 352, 1));
        assert_eq!(player.equipment().ammo.unwrap().clip, 29);
        player.apply(&select(1200, 180, 1));
        assert_eq!(player.equipment().weapon, 352);
        assert_eq!(player.equipment().firing.unwrap().state, 3);
        assert_eq!(player.equipment().firing.unwrap().animation & !0x400, 10);
        assert_eq!(player.move_scale(), 0.9);
        let sequence = player.equipment().events.unwrap().sequence;
        player.apply(&select(1200, 180, 1));
        assert_eq!(player.equipment().events.unwrap().sequence, sequence);
        player.apply(&select(1499, 180, 1));
        assert_eq!(player.equipment().weapon, 352);
        player.apply(&select(1500, 180, 1));
        assert_eq!(player.equipment().weapon, 180);
        assert_eq!(player.equipment().secondary.unwrap().ammo.unwrap().clip, 29);
        assert_eq!(player.equipment().firing.unwrap().state, 1);
        assert_eq!(player.equipment().firing.unwrap().animation & !0x400, 11);
        assert_eq!(player.move_scale(), 1.0);
        player.apply(&select(1899, 180, 1));
        assert_eq!(player.equipment().ammo.unwrap().clip, 30);
        player.apply(&select(1900, 180, 1));
        assert_eq!(player.equipment().firing.unwrap().state, 0);
        player.apply(&select(1901, 180, 1));
        assert_eq!(player.equipment().ammo.unwrap().clip, 29);
        player.apply(&select(2000, 352, 0));
        player.apply(&select(2300, 352, 0));
        player.apply(&select(2700, 352, 0));
        assert_eq!(player.equipment().weapon, 352);
        assert_eq!(player.equipment().ammo.unwrap().clip, 29);
        assert_eq!(player.equipment().secondary.unwrap().ammo.unwrap().clip, 29);
    }

    #[test]
    fn switching_preserves_each_weapons_fire_type() {
        let primary = switchable_player(352, 0.9);
        let mut secondary = switchable_player(180, 1.0);
        secondary.set_fire_type(1).unwrap();
        let mut p = primary.with_secondary(secondary);
        p.apply(&select(1100, 352, 1));
        p.apply(&select(1200, 352, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
        p.apply(&select(1300, 180, 0));
        p.apply(&select(1600, 180, 0));
        p.apply(&select(2000, 180, 0));
        p.apply(&select(2001, 180, 1));
        p.apply(&select(2101, 180, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&select(2200, 352, 0));
        p.apply(&select(2500, 352, 0));
        p.apply(&select(2900, 352, 0));
        p.apply(&select(2901, 352, 1));
        p.apply(&select(3001, 352, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 26);
    }

    #[test]
    fn movement_events_share_the_predictable_sequence_with_weapon_events() {
        let mut p = player();
        p.movement_event(142, 4); // Target EV_FOOTSTEP_RUN, surface 4.
        p.apply(&cmd(1100, 1));
        let events = p.equipment().events.unwrap();
        assert_eq!(events.sequence, 2);
        assert_eq!(events.slots[0].id, 142);
        assert_eq!(events.slots[0].parameter, 4);
        assert_eq!(events.slots[1].id, events::FIRE_WEAPON);
    }

    #[test]
    fn empty_magazines_select_measured_empty_switch_animations_and_timings() {
        let mut primary = switchable_player(352, 1.0);
        primary.current.equipment.ammo.as_mut().unwrap().clip = 0;
        let mut secondary = switchable_player(180, 1.0);
        secondary.current.equipment.ammo.as_mut().unwrap().clip = 0;
        let mut player = primary.with_secondary(secondary);
        player.apply(&select(1100, 180, 0));
        let drop = player.equipment().firing.unwrap();
        assert_eq!((drop.animation & !0x400, drop.remaining_msec), (23, 500));
        player.apply(&select(1600, 180, 0));
        let raise = player.equipment().firing.unwrap();
        assert_eq!((raise.animation & !0x400, raise.remaining_msec), (24, 600));
        assert_eq!(player.equipment().ammo.unwrap().clip, 0);
    }

    #[test]
    fn unowned_and_menu_captured_switches_are_ignored_and_drop_can_be_cancelled() {
        let mut player = switchable_player(352, 1.0).with_secondary(switchable_player(180, 1.0));
        player.apply(&select(1100, 800, 0));
        player.apply(&select(1200, 180, 0x10000000));
        assert_eq!(player.equipment().firing.unwrap().state, 0);
        assert_eq!(player.equipment().events.unwrap().sequence, 0);
        player.apply(&select(1300, 180, 0));
        player.apply(&select(1600, 352, 0));
        assert_eq!(player.equipment().weapon, 352);
        assert_eq!(player.equipment().firing.unwrap().state, 1);
    }

    #[test]
    fn switching_cancels_unfinished_reload_without_moving_reserve() {
        let mut primary = switchable_player(352, 1.0);
        primary.current.reload = Some(reload_timing());
        let mut player = primary.with_secondary(switchable_player(180, 1.0));
        player.apply(&select(1100, 352, 1));
        player.apply(&select(1200, 352, 0x10));
        player.apply(&select(1300, 180, 0));
        player.apply(&select(1600, 180, 0));
        player.apply(&select(2000, 180, 0));
        let old = player.equipment().secondary.unwrap().ammo.unwrap();
        assert_eq!((old.clip, old.reserve), (29, 90));
        player.apply(&select(2100, 352, 0));
        player.apply(&select(2400, 352, 0));
        player.apply(&select(2800, 352, 0));
        player.apply(&select(5000, 352, 0));
        let old = player.equipment().ammo.unwrap();
        assert_eq!((old.clip, old.reserve), (29, 90));
    }

    #[test]
    fn reload_events_fire_once_and_loadout_replacement_preserves_event_and_command_clocks() {
        let mut player = player();
        player.current.reload = Some(reload_timing());
        player.apply(&cmd(1100, 1));
        player.apply(&cmd(1200, 0x10));
        let queue = player.equipment().events.unwrap();
        assert_eq!(queue.sequence, 4);
        assert_eq!(queue.slots.map(|event| event.id), [36, 23, 28, 24]);
        player.apply(&cmd(2599, 0));
        player.apply(&cmd(2599, 0));
        assert_eq!(player.equipment().events.unwrap().sequence, 5);
        assert_eq!(player.equipment().events.unwrap().slots[0].id, 29);
        player.apply(&cmd(3229, 0));
        assert_eq!(player.equipment().events.unwrap().sequence, 6);
        assert_eq!(player.equipment().events.unwrap().slots[1].id, 27);
        player.replace_loadout(
            switchable_player(353, 0.95),
            Some(switchable_player(180, 1.0)),
        );
        assert_eq!(player.equipment().events.unwrap().sequence, 9);
        player.apply(&select(3229, 353, 1));
        player.apply(&select(3000, 353, 1));
        assert_eq!(player.equipment().events.unwrap().sequence, 9);
        assert_eq!(player.equipment().firing.unwrap().remaining_msec, 400);
        player.apply(&select(3629, 353, 0));
        assert_eq!(player.equipment().firing.unwrap().state, 0);
    }
}
