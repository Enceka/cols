use super::*;

impl HeldWeapons {
    pub fn set_offhand_switch(&mut self, timing: Option<AlternateTiming>) {
        self.current.offhand_switch = timing;
    }
    pub fn set_offhands(&mut self, offhands: [Option<crate::game::weapons::grenade::Equipped>; 2]) {
        self.offhands = offhands;
    }
    pub fn take_throws(&mut self) -> Vec<crate::game::weapons::grenade::Throw> {
        std::mem::take(&mut self.throws)
    }
    pub(super) fn apply_marker(&mut self, cmd: &UserCommand) -> bool {
        let Some(d) = self.current.marker.clone() else {
            return false;
        };
        if let Some(phase) = self.marker_phase {
            let f = self.current.equipment.firing.unwrap();
            if phase == 0 && f.remaining_msec == 0 && cmd.buttons & 1 == 0 {
                self.current.animate(6, 2, d.throw_msec, d.throw_delay_msec);
                self.marker_phase = Some(1);
            } else if phase == 1 {
                if f.delay_msec == 0 {
                    let ammo = self.current.equipment.ammo.as_mut().unwrap();
                    if ammo.clip > 0 {
                        ammo.clip -= 1;
                        self.throws.push(crate::game::weapons::grenade::Throw {
                            time: cmd.time,
                            angles: cmd.angles,
                            definition: d.clone(),
                            fuse_msec: d.fuse_msec,
                            in_hand: false,
                            dropped: false,
                        });
                        self.emit(events::FIRE_WEAPON_LAST_SHOT, 0);
                    }
                }
                if f.remaining_msec == 0 {
                    self.marker_phase = None;
                    self.grenade_time = 0;
                    self.current.ready();
                }
            }
            return true;
        }
        if Self::input_allowed(cmd)
            && cmd.weapons[0] == d.index
            && cmd.buttons & 1 != 0
            && !self.attack_held
            && !self.current.empty()
        {
            self.current.animate(6, 3, d.prepare_msec, 0);
            self.emit(35, 0); // Native EV_PULLBACK_WEAPON, not EV_PREP_OFFHAND.
            self.marker_phase = Some(0);
        }
        true
    }
    pub(super) fn offhand_time(&self, time: u16) -> u16 {
        (f32::from(time) / self.perks.value("modOffhandSpeedScale", 1.0)).trunc() as u16
    }
    pub(super) fn finish_offhand(&mut self) {
        self.grenade_time = 0;
        self.offhand_pending = false;
        self.selected_offhand = None;
        let time = self.current.offhand_switch.map_or(0, |t| t.raise_msec);
        self.current.animate(21, 22, self.offhand_time(time), 0);
        self.emit(22, 0); // EV_OFFHAND_END_NOTIFY:0x427c7c
    }
    pub(super) fn throw_offhand(&mut self, cmd: &UserCommand, in_hand: bool, dropped: bool) {
        let slot = self.selected_offhand.expect("active offhand slot");
        let equipped = self.offhands[slot].as_mut().unwrap();
        if equipped.count == 0 {
            return;
        }
        equipped.count -= 1;
        let definition = equipped.definition.clone();
        self.throws.push(crate::game::weapons::grenade::Throw {
            time: cmd.time,
            angles: cmd.angles,
            definition: definition.clone(),
            fuse_msec: if in_hand { 0 } else { self.grenade_time },
            in_hand,
            dropped,
        });
        self.offhand_pending = false;
        self.emit(if in_hand { 126 } else { 64 }, u32::from(definition.index));
        tracing::info!(
            time = cmd.time,
            weapon = definition.index,
            count = self.offhands[slot].as_ref().unwrap().count,
            fuse = self.grenade_time,
            in_hand,
            dropped,
            "native grenade throw committed"
        );
    }
    pub fn drop_offhand_on_death(&mut self, time: u32, angles: [u16; 3]) {
        self.set_diving(false, false);
        self.marker_phase = None;
        if self.selected_offhand.is_some() && self.offhand_pending && self.grenade_time != 0 {
            self.throw_offhand(
                &UserCommand {
                    time,
                    angles,
                    ..Default::default()
                },
                false,
                true,
            );
        }
        self.selected_offhand = None;
        self.grenade_time = 0;
        self.offhand_pending = false;
        self.current.ready();
    }
    pub(super) fn apply_offhand(&mut self, cmd: &UserCommand, elapsed: u32) -> bool {
        let state = self.current.equipment.firing.map_or(0, |f| f.state);
        if state == 21 {
            if let Some(f) = self.current.equipment.firing.as_mut() {
                f.remaining_msec = f.remaining_msec.saturating_sub(elapsed.min(65535) as u16);
                if f.remaining_msec == 0 {
                    self.current.ready();
                }
            }
            return true;
        }
        if let Some(slot) = self.selected_offhand {
            let definition = self.offhands[slot].as_ref().unwrap().definition.clone();
            let f = self.current.equipment.firing.as_mut().unwrap();
            f.remaining_msec = f.remaining_msec.saturating_sub(elapsed.min(65535) as u16);
            f.delay_msec = f.delay_msec.saturating_sub(elapsed.min(65535) as u16);
            let (remaining, delay) = (f.remaining_msec, f.delay_msec);
            if matches!(state, 18 | 19) && definition.cook && self.offhand_pending {
                if elapsed >= u32::from(self.grenade_time) {
                    self.throw_offhand(cmd, true, false);
                    self.finish_offhand();
                    return true;
                }
                self.grenade_time -= elapsed as u16;
            }
            match state {
                16 if remaining == 0 => {
                    self.current
                        .animate(17, 31, self.offhand_time(definition.prepare_msec), 0);
                    self.emit(63, u32::from(definition.index));
                }
                17 if remaining == 0 => {
                    self.current.equipment.firing.as_mut().unwrap().state = 18;
                    self.grenade_time = definition.fuse_msec;
                }
                18 if cmd.buttons & (if slot == 0 { 0x4000 } else { 0x10000 }) == 0 => {
                    self.current.animate(
                        19,
                        2,
                        self.offhand_time(definition.throw_msec),
                        self.offhand_time(definition.throw_delay_msec),
                    );
                }
                19 => {
                    if delay == 0 && self.offhand_pending {
                        self.throw_offhand(cmd, false, false);
                    }
                    if remaining == 0 {
                        self.finish_offhand();
                    }
                }
                _ => (),
            }
            return true;
        }
        if !Self::input_allowed(cmd)
            || cmd.buttons & 0x10000000 != 0
            || cmd.buttons & 2 != 0
            || cmd.weapons[0] != self.current.equipment.weapon
            || (1..=5).contains(&state)
            || (13..=15).contains(&state)
            || self.current.offhand_switch.is_none()
        {
            return false;
        }
        let slot = if cmd.buttons & 0x4000 != 0 && self.offhand_held & 0x4000 == 0 {
            0
        } else if cmd.buttons & 0x10000 != 0 && self.offhand_held & 0x10000 == 0 {
            1
        } else {
            return false;
        };
        let Some(definition) = self.offhands[slot]
            .as_ref()
            .filter(|g| g.count != 0)
            .map(|g| g.definition.clone())
        else {
            return false;
        };
        self.current.cancel_burst();
        self.current.reload_transfer_pending = false;
        self.selected_offhand = Some(slot);
        self.offhand_pending = true;
        self.grenade_time = 0;
        self.current.equipment.aim = self.current.ads.map(|_| Aim::default());
        self.emit(65, u32::from(definition.index)); // EV_SWITCH_OFFHAND
        self.emit(34, u32::from(definition.index)); // EV_WEAPON_SWITCH_STARTED
        let drop = self.current.offhand_switch.unwrap().drop_msec;
        self.current.animate(16, 21, self.offhand_time(drop), 0);
        true
    }
}
