use super::*;

impl HeldWeapons {
    pub(super) fn apply_aim(&mut self, cmd: &UserCommand, elapsed: u32) {
        let (Some(timing), Some(aim)) = (self.current.ads, self.current.equipment.aim.as_mut())
        else {
            return;
        };
        let firing = self.current.equipment.firing;
        let reloading = firing.is_some_and(|f| reloading(f.state));
        let switching = firing.is_some_and(|f| (1..=5).contains(&f.state));
        let melee = firing.is_some_and(|f| (13..=15).contains(&f.state));
        let sprint = firing.is_some_and(|f| (23..=25).contains(&f.state));
        aim.button = cmd.buttons & 0x800 != 0
            && self.selected_offhand.is_none()
            && Self::input_allowed(cmd)
            && !switching
            && !melee
            && !sprint
            && cmd.weapons[0] == self.current.equipment.weapon;
        aim.movement_slow = aim.button && !reloading;
        let reload_blocks = reloading
            && !timing.reload_allowed
            && (timing.reload_interruptible
                || firing.is_some_and(|f| f.remaining_msec > timing.reload_end_msec));
        // Target 0x4224e0: elapsed msec times the registered per-msec rate.
        let elapsed = elapsed as f32 * self.perks.value("modADSSpeedScale", 1.0);
        aim.fraction = if aim.button && !reload_blocks {
            (aim.fraction + elapsed * timing.in_rate).min(1.0)
        } else {
            (aim.fraction - elapsed * timing.out_rate).max(0.0)
        };
    }
    pub(super) fn begin_drop(&mut self, requested: u16) {
        self.marker_phase = None;
        self.grenade_time = 0;
        self.current.cancel_burst();
        let timing = self.current.switch.expect("switching timings checked");
        if let Some(state) = self
            .current
            .equipment
            .firing
            .map(|f| f.state)
            .filter(|s| reloading(*s))
        {
            self.emit(2, u32::from(state)); // EV_STOP_WEAPON_SOUND carries the interrupted state.
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
    pub(super) fn begin_raise(&mut self) {
        self.current.cancel_burst();
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
    pub(super) fn begin_alternate_drop(&mut self, requested: u16) {
        self.current.cancel_burst();
        if let Some(firing) = self.current.equipment.firing
            && reloading(firing.state)
        {
            self.emit(2, u32::from(firing.state));
        }
        self.current.reload_transfer_pending = false;
        self.switch_target = Some(requested);
        self.emit(events::WEAPON_SWITCH_STARTED, u32::from(requested));
        self.emit(events::ALT_WEAPON_SWITCH, 0);
        self.current
            .animate(5, 19, self.alternate_timing.unwrap().drop_msec, 0);
    }
    pub(super) fn apply_sprint(&mut self) -> bool {
        let Some(timing) = self.current.sprint_timing else {
            return false;
        };
        let firing = self.current.equipment.firing.unwrap();
        if matches!(firing.state, 23 | 24) {
            if !self.sprinting {
                // 0x4284f2 divides the authored out time by ability11's
                // value, then truncates. Dexterity's +0.5 resolves to1.5.
                let out = (f64::from(timing.out_msec)
                    / f64::from(self.perks.value("modWeaponRecoveryTimeScale", 1.0)))
                    as u16;
                self.current.animate(25, 27, out, 0);
            } else if firing.state == 23 && firing.remaining_msec == 0 {
                self.current.animate(24, 26, 0, 0);
            }
            return true;
        }
        if firing.state == 25 {
            if firing.remaining_msec == 0 {
                self.current.ready();
            }
            return true;
        }
        if !self.sprinting || matches!(firing.state, 1..=7 | 13..=21 | 29..=33) {
            return false;
        }
        if reloading(firing.state) {
            self.emit(2, u32::from(firing.state));
        }
        self.current.cancel_burst();
        self.current.reload_transfer_pending = false;
        self.current.animate(23, 25, timing.in_msec, 0);
        true
    }
    pub(super) fn apply_weapon(
        &mut self,
        cmd: &UserCommand,
        elapsed: u32,
        approach: Option<MeleeApproach>,
    ) {
        if self.apply_offhand(cmd, elapsed) {
            return;
        }
        // 0x4249c0 scales the elapsed frame time, not the authored duration.
        let state = self.current.equipment.firing.map_or(0, |f| f.state);
        let scale = if reloading(state) && self.current.fast_reload {
            self.perks.value("modReloadSpeedScale", 1.0)
        } else if (1..=5).contains(&state) {
            self.perks.value("modWeaponSwitchSpeedScale", 1.0)
        } else if self.current.marker.is_none() && matches!(state, 6 | 7) {
            self.perks.value("modRateOfFire", 1.0)
        } else {
            1.0
        };
        let elapsed = (elapsed as f32 * scale).round_ties_even() as u32;
        let elapsed = elapsed.min(u32::from(u16::MAX)) as u16;
        if let Some(firing) = self.current.equipment.firing.as_mut() {
            firing.remaining_msec = firing.remaining_msec.saturating_sub(elapsed);
            firing.delay_msec = firing.delay_msec.saturating_sub(elapsed);
        }
        let state = self.current.equipment.firing.map_or(0, |f| f.state);
        let mut remaining = self
            .current
            .equipment
            .firing
            .map_or(0, |f| f.remaining_msec);
        if let Some(timing) = self.current.melee {
            if (13..=15).contains(&state) {
                if state == 13 && self.current.equipment.firing.unwrap().delay_msec == 0 {
                    // 0x425f00: one impact transition, without restarting the
                    // swipe animation or consuming a round.
                    self.current.equipment.firing.as_mut().unwrap().state = 14;
                    self.emit(events::FIRE_MELEE, 0);
                    self.melee_strikes.push(MeleeStrike {
                        time: cmd.time,
                        angles: cmd.angles,
                        weapon: self.current.equipment.weapon,
                        damage: timing.damage,
                    });
                } else if state == 14 && remaining == 0 {
                    self.current.animate(15, 22, timing.recover_msec, 0);
                    self.emit(events::MELEE_END, 0);
                } else if state == 15 && remaining == 0 {
                    self.current.ready();
                }
                return;
            }
            // 0x427000 accepts an edge during reload, but not during a
            // weapon change or another pending non-reload action delay.
            if cmd.buttons & 4 != 0
                && !self.melee_held
                && cmd.buttons & 0x10000000 == 0
                && cmd.weapons[0] == self.current.equipment.weapon
                && !(1..=5).contains(&state)
                && (reloading(state)
                    || self
                        .current
                        .equipment
                        .firing
                        .is_none_or(|f| f.delay_msec == 0))
            {
                self.current.cancel_burst();
                self.current.reload_transfer_pending = false;
                if reloading(state) {
                    self.emit(2, u32::from(state));
                }
                self.current.equipment.aim = self.current.ads.map(|_| Aim::default());
                let charge = approach.filter(|a| a.distance > 64).and(timing.charge);
                let duration = charge.map_or(timing.duration_msec, |c| c.duration_msec);
                let delay = charge.map_or(timing.hit_msec, |c| c.hit_msec);
                self.melee_approach = approach
                    .filter(|a| {
                        a.distance > 0 && (a.distance <= 64 || charge.is_some()) && delay > 0
                    })
                    .map(|a| (a, delay));
                self.current
                    .animate(13, if charge.is_some() { 9 } else { 8 }, duration, delay);
                self.emit(events::MELEE_SWIPE, 0);
                self.emit(events::MELEE_INIT, 0);
                return;
            }
        }
        if state == 5 {
            // 0x424fa0 -> 0x423b10: requesting the held weapon again during
            // the alternate drop finishes the change at once (state 2).
            if remaining > 0
                && Self::input_allowed(cmd)
                && cmd.weapons[0] == self.current.equipment.weapon
            {
                self.switch_target = None;
                self.current.equipment.aim = self.current.ads.map(|_| Aim::default());
                self.current
                    .animate(2, 20, self.alternate_timing.unwrap().raise_msec, 0);
                return;
            }
            if remaining == 0 {
                if self.switch_target.take() == self.alternate.as_ref().map(|w| w.equipment.weapon)
                {
                    self.current.ready();
                    std::mem::swap(&mut self.current, self.alternate.as_mut().unwrap());
                }
                self.current.equipment.aim = self.current.ads.map(|_| Aim::default());
                self.current
                    .animate(2, 20, self.alternate_timing.unwrap().raise_msec, 0);
            }
            return;
        }
        if state == 3 {
            // 0x424fa0 calls FinishWeaponChange 0x423b10 as soon as the
            // request returns to the held weapon, so the original gun is
            // raised without waiting for the rest of its drop.
            if Self::input_allowed(cmd) && cmd.weapons[0] == self.current.equipment.weapon {
                self.switch_target = None;
                self.begin_raise();
                return;
            }
            // Otherwise FinishWeaponChange reads the latest owned request.
            if Self::input_allowed(cmd)
                && self
                    .secondary
                    .as_ref()
                    .into_iter()
                    .chain(
                        self.rewards
                            .iter()
                            .flatten()
                            .flat_map(|r| std::iter::once(&r.weapon).chain(&r.alternate)),
                    )
                    .any(|other| other.equipment.weapon == cmd.weapons[0])
            {
                self.switch_target = Some(cmd.weapons[0]);
            }
            if remaining == 0 {
                let requested = self.switch_target.take();
                self.restore_reward();
                if let Some(other) = self.alternate.as_mut()
                    && requested == Some(other.equipment.weapon)
                {
                    self.current.ready();
                    std::mem::swap(&mut self.current, other);
                }
                if let Some(slot) = self
                    .rewards
                    .iter()
                    .position(|r| r.as_ref().is_some_and(|r| requested == Some(r.id)))
                {
                    let other = self.rewards[slot].as_mut().unwrap();
                    self.current.ready();
                    std::mem::swap(&mut self.current, &mut other.weapon);
                    std::mem::swap(&mut self.alternate, &mut other.alternate);
                    std::mem::swap(&mut self.alternate_timing, &mut other.alternate_timing);
                    self.active_reward = Some(slot);
                }
                if let Some(other) = self.secondary.as_mut()
                    && requested == Some(other.equipment.weapon)
                {
                    self.current.ready();
                    std::mem::swap(&mut self.current, other);
                    std::mem::swap(&mut self.alternate, &mut self.secondary_alternate);
                    std::mem::swap(
                        &mut self.alternate_timing,
                        &mut self.secondary_alternate_timing,
                    );
                }
                self.begin_raise();
            }
            return;
        }
        let raising = matches!(state, 1 | 2);
        if raising {
            if remaining == 0 {
                self.current.ready();
                return;
            }
            // 0x424fa0 lets a new request interrupt a raise unless a weapon
            // delay is pending; 0x4236a0 then starts the drop immediately.
            if self.current.equipment.firing.unwrap().delay_msec != 0 {
                return;
            }
        }
        if Self::input_allowed(cmd)
            && self
                .alternate
                .as_ref()
                .is_some_and(|other| other.equipment.weapon == cmd.weapons[0])
        {
            self.begin_alternate_drop(cmd.weapons[0]);
            return;
        }
        if Self::input_allowed(cmd)
            && self.current.switch.is_some()
            && self
                .secondary
                .as_ref()
                .into_iter()
                .chain(
                    self.rewards
                        .iter()
                        .flatten()
                        .flat_map(|r| std::iter::once(&r.weapon).chain(&r.alternate)),
                )
                .any(|other| other.equipment.weapon == cmd.weapons[0] && other.switch.is_some())
        {
            self.begin_drop(cmd.weapons[0]);
            return;
        }
        if raising {
            return;
        }
        if self.current.equipment.firing.is_some() && self.apply_sprint() {
            return;
        }
        if self.apply_marker(cmd) {
            return;
        }
        if self.current.is_burst()
            && state == 6
            && cmd.buttons & 16 != 0
            && Self::input_allowed(cmd)
            && cmd.weapons[0] == self.current.equipment.weapon
            && self.current.reload.is_some()
            && self
                .current
                .equipment
                .ammo
                .is_some_and(|a| a.clip < self.current.magazine_capacity && a.reserve > 0)
        {
            // 0x424660 ->0x423490 accepts an explicit reload from firing6;
            // do not force the remaining rounds of this burst first.
            remaining = 0;
            self.current
                .equipment
                .firing
                .as_mut()
                .unwrap()
                .remaining_msec = 0;
        }
        if self.current.is_burst() && state == 6 && remaining == 0 && !self.current.burst_pending()
        {
            // 0x424d13..0x424d83: even a clip-truncated burst goes ready with
            //200ms recovery after its final round's ordinary fire interval.
            self.current.ready();
            self.current
                .equipment
                .firing
                .as_mut()
                .unwrap()
                .remaining_msec = 200;
            if cmd.buttons & 16 == 0 {
                return;
            }
        }
        if self.current.is_burst()
            && remaining == 0
            && !self.current.burst_pending()
            && (cmd.buttons & 1 == 0 || self.current.equipment.queued_burst)
        {
            self.current.equipment.fire_count = 0;
        }
        if state == 7 {
            if self.current.equipment.firing.unwrap().delay_msec == 0
                && self.current.rechamber_pending
            {
                self.current.rechamber_pending = false;
                self.emit(events::EJECT_BRASS, 0);
            }
            if remaining == 0 {
                self.current.needs_rechamber = false;
                self.current.ready();
            }
            return;
        }
        if self.current.rechamber_pending && remaining == 0 {
            let timing = self.current.rechamber.unwrap();
            let ads = self
                .current
                .equipment
                .aim
                .is_some_and(|a| a.fraction > 0.75);
            self.current.animate(
                7,
                if ads { 7 } else { 4 },
                timing.duration_msec,
                if timing.eject_msec == 0 || timing.eject_msec >= timing.duration_msec {
                    1
                } else {
                    timing.eject_msec
                },
            );
            self.emit(events::RECHAMBER_WEAPON, 0);
            return;
        }
        if let Some(segmented) = self.current.reload.and_then(|r| r.segmented)
            && reloading(state)
        {
            self.apply_segmented_reload(cmd, state, remaining, segmented);
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
            self.current.cancel_burst();
            if self.current.fire_type == 5 {
                self.current.equipment.fire_count = 0;
            }
            let empty = ammo.clip == 0;
            self.emit(events::RESET_ADS, 0);
            self.emit(events::RELOAD_START_NOTIFY, 0);
            if let Some(segmented) = timing.segmented
                && segmented.start_msec != 0
            {
                self.emit(events::RELOAD_START, 0);
                self.current
                    .animate(10, 17, segmented.start_msec, segmented.start_add_msec);
                self.current.reload_transfer_pending = segmented.start_add_msec != 0;
                return;
            }
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
        if self.current.fire_type == 5 {
            self.current.equipment.barrel_flags &= 1;
        }
        let attack = cmd.buttons & self.current.attack_mask();
        let other_barrel = self.current.fire_type == 5
            && if self.current.equipment.barrel_flags & 1 != 0 {
                attack & 1 != 0
            } else {
                attack & 0x100000 != 0
            };
        let trigger = self.current.burst_pending()
            || (attack != 0
                && (self.current.fire_type == 0
                    || self.current.equipment.fire_count == 0
                    || other_barrel));
        if allowed && trigger && ammo.clip > 0 {
            let barrels = if self.current.fire_type == 5 && attack == 0x100001 {
                ammo.clip.min(2)
            } else {
                1
            };
            if self.current.fire_type == 5 {
                // 0x425b20 fires the secondary first, then the ordinary path
                // can fire the primary in the same command. Both spend ammo.
                self.current.equipment.barrel_flags =
                    u8::from(attack & 0x100000 != 0) | if barrels == 2 { 2 } else { 0 };
            }
            for _ in 0..barrels {
                let remaining_clip = self.current.equipment.ammo.unwrap().clip - 1;
                if let Some(mut ballistics) = self.current.ballistics {
                    ballistics.spread_degrees = self.shot_spread();
                    self.shots.push(Shot {
                        time: cmd.time,
                        angles: cmd.angles,
                        weapon: self.current.equipment.weapon,
                        ballistics,
                    });
                }
                self.current.equipment.ammo.as_mut().unwrap().clip = remaining_clip;
                if !self
                    .current
                    .equipment
                    .aim
                    .is_some_and(|a| a.fraction == 1.0)
                    && let Some(timing) = self.current.spread
                {
                    self.spread_scale = (self.spread_scale + timing.fire_add * 255.0).min(255.0);
                }
                if self.current.fire_type != 0 {
                    if self.current.equipment.fire_count == 0 {
                        self.current.equipment.queued_burst = false;
                    }
                    self.current.equipment.fire_count =
                        (self.current.equipment.fire_count + 1).min(4);
                }
                self.current.rechamber_pending = self.current.rechamber.is_some();
                self.current.needs_rechamber = self.current.rechamber_pending;
                let ads = self
                    .current
                    .equipment
                    .aim
                    .is_some_and(|a| a.fraction > 0.75);
                self.current.animate(
                    6,
                    match (ads, remaining_clip == 0) {
                        (false, false) => 2,
                        (false, true) => 3,
                        (true, false) => 5,
                        (true, true) => 6,
                    },
                    interval,
                    0,
                );
                self.emit(
                    if remaining_clip == 0 {
                        events::FIRE_WEAPON_LAST_SHOT
                    } else {
                        events::FIRE_WEAPON
                    },
                    0,
                );
            }
        } else {
            if state != 0 {
                self.current.ready();
            }
            // Target0x424c70 pins weaponTime at1 while a semiautomatic
            // trigger has exhausted its shot count. Preserve that in the
            // authority snapshot so prediction cannot begin another shot.
            if allowed
                && attack != 0
                && self.current.fire_type != 0
                && self.current.equipment.fire_count != 0
                && let Some(firing) = &mut self.current.equipment.firing
            {
                firing.remaining_msec = 1;
            }
            if allowed && attack != 0 && ammo.clip == 0 && !self.attack_held {
                self.emit(events::NO_AMMO, 0);
            }
        }
    }
}
