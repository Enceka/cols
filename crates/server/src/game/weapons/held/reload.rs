use super::*;

impl HeldWeapons {
    pub(super) fn needs_shell(&self) -> bool {
        self.current
            .equipment
            .ammo
            .is_some_and(|a| a.clip < self.current.magazine_capacity && a.reserve > 0)
    }
    pub(super) fn add_shells(&mut self, count: u16) {
        if let Some(ammo) = self.current.equipment.ammo.as_mut() {
            let transfer = self
                .current
                .magazine_capacity
                .saturating_sub(ammo.clip)
                .min(ammo.reserve)
                .min(count);
            ammo.clip += transfer;
            ammo.reserve -= transfer;
            if transfer != 0 {
                self.emit(events::RELOAD_ADD_AMMO, 0);
            }
        }
    }
    /// Target 0x4232d0: one shell, normal or empty variant.
    pub(super) fn begin_shell(&mut self) {
        let timing = self.current.reload.expect("segmented reload timing");
        let empty = self.current.empty();
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
        self.emit(
            if empty {
                events::RELOAD_FROM_EMPTY
            } else {
                events::RELOAD
            },
            0,
        );
    }
    pub(super) fn apply_segmented_reload(
        &mut self,
        cmd: &UserCommand,
        mut state: u8,
        remaining: u16,
        segmented: SegmentedReload,
    ) {
        let delay = self.current.equipment.firing.unwrap().delay_msec;
        if self.current.reload_transfer_pending && delay == 0 {
            self.add_shells(1);
            self.current.reload_transfer_pending = false;
        }
        if state == 12 {
            if remaining == 0 {
                self.current.ready();
            }
            return;
        }
        // Pressing fire with a loaded shell interrupts after this phase.
        let loaded = self.current.equipment.ammo.is_some_and(|a| a.clip > 0);
        if Self::input_allowed(cmd) && cmd.buttons & 1 != 0 && loaded && matches!(state, 8 | 10) {
            state += 1;
            if let Some(firing) = self.current.equipment.firing.as_mut() {
                firing.state = state;
            }
        }
        if remaining != 0 {
            return;
        }
        let interrupted = matches!(state, 9 | 11) && loaded;
        if !interrupted && self.needs_shell() {
            self.begin_shell();
        } else if segmented.end_msec != 0 {
            self.current.reload_transfer_pending = false;
            self.current.animate(12, 18, segmented.end_msec, 0);
            self.emit(events::RELOAD_END, 0);
        } else {
            self.current.ready();
        }
    }
}
