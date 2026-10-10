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
    /// WeaponDef+0xd86 shell-by-shell reload; the normal timings are then
    /// one shell (anim13, +0x71c, ammo at +0x730).
    #[serde(default)]
    pub segmented: Option<SegmentedReload>,
}
impl ReloadTiming {
    pub fn valid(&self) -> bool {
        (1..=15000).contains(&self.normal_msec)
            && (1..=15000).contains(&self.empty_msec)
            && (1..=self.normal_msec).contains(&self.add_msec)
            && (1..=self.empty_msec).contains(&self.empty_add_msec)
            && self.segmented.is_none_or(|s| {
                s.start_msec <= 15000 && s.start_add_msec <= s.start_msec && s.end_msec <= 15000
            })
    }
}

/// Target 0x423490/0x424260/0x4243f0: start (state10, anim17, +0x740, one
/// shell at +0x744), shells (state8), end (state12, anim18, +0x748). Fire
/// interrupts as state11/9. Zero times skip that phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SegmentedReload {
    pub start_msec: u16,
    pub start_add_msec: u16,
    pub end_msec: u16,
}

/// Weapon states 8..=12 are reloading, its interrupt, start, start
/// interrupt and end (target 0x424660 groups them).
fn reloading(state: u8) -> bool {
    (8..=12).contains(&state)
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

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AlternateTiming {
    pub drop_msec: u16,
    pub raise_msec: u16,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RechamberTiming {
    pub duration_msec: u16,
    pub eject_msec: u16,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpreadTiming {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
    pub decay: f32,
    pub fire_add: f32,
    pub turn_add: f32,
    pub move_add: f32,
    pub stance_decay: [f32; 3],
    pub ads: f32,
}
impl SpreadTiming {
    pub fn valid(self) -> bool {
        self.minimum
            .into_iter()
            .chain(self.maximum)
            .chain(self.stance_decay)
            .chain([
                self.decay,
                self.fire_add,
                self.turn_add,
                self.move_add,
                self.ads,
            ])
            .all(|v| v.is_finite() && (0.0..=180.0).contains(&v))
            && self
                .minimum
                .into_iter()
                .zip(self.maximum)
                .all(|(min, max)| min <= max)
    }
}

#[derive(Debug, Clone)]
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
    rechamber: Option<RechamberTiming>,
    rechamber_pending: bool,
    needs_rechamber: bool,
    spread: Option<SpreadTiming>,
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
        self.animate(0, 0, 0, 0);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Player {
    current: Weapon,
    secondary: Option<Weapon>,
    alternate: Option<Weapon>,
    secondary_alternate: Option<Weapon>,
    alternate_timing: Option<AlternateTiming>,
    secondary_alternate_timing: Option<AlternateTiming>,
    last_time: u32,
    events: EventQueue,
    switch_target: Option<u16>,
    attack_held: bool,
    shots: Vec<Shot>,
    spread_scale: f32,
    spread_stance: usize,
    spread_speed: f32,
    spread_airborne: bool,
    last_angles: Option<[u16; 3]>,
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
                rechamber: None,
                rechamber_pending: false,
                needs_rechamber: false,
                spread: None,
            },
            secondary: None,
            alternate: None,
            secondary_alternate: None,
            alternate_timing: None,
            secondary_alternate_timing: None,
            last_time: 1000,
            events: EventQueue::default(),
            switch_target: None,
            attack_held: false,
            shots: Vec::new(),
            spread_scale: 0.0,
            spread_stance: 0,
            spread_speed: 0.0,
            spread_airborne: false,
            last_angles: None,
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

    pub fn set_rechamber_timing(&mut self, timing: RechamberTiming) {
        self.current.rechamber = Some(timing);
    }
    pub fn set_spread_timing(&mut self, timing: SpreadTiming) {
        self.current.spread = Some(timing);
    }
    pub fn spread_environment(&mut self, motion: &crate::game_motion::Motion) {
        self.spread_stance = match motion.stance {
            crate::game_motion::Stance::Stand => 0,
            crate::game_motion::Stance::Crouch => 1,
            crate::game_motion::Stance::Prone => 2,
        };
        self.spread_speed =
            motion.velocity[0].hypot(motion.velocity[1]) / f32::from(motion.speed.max(1));
        self.spread_airborne = motion.ground_entity == 2047;
    }

    pub fn move_scale(&self) -> f32 {
        self.current.move_scale
    }
    pub fn set_ballistics(&mut self, stats: crate::game_combat::Ballistics) {
        assert!(stats.valid());
        self.current.ballistics = Some(stats);
    }
    pub fn bot_weapon(&self) -> Option<crate::game_bot::BotWeapon> {
        Some(crate::game_bot::BotWeapon {
            weapon: self.current.equipment.weapon,
            ballistics: self.current.ballistics?,
            fire_interval_msec: u32::from(self.current.fire_interval?),
        })
    }
    pub fn take_shots(&mut self) -> Vec<Shot> {
        std::mem::take(&mut self.shots)
    }

    pub fn with_secondary(mut self, secondary: Player) -> Self {
        if secondary.current.equipment.weapon != self.current.equipment.weapon {
            self.secondary = Some(secondary.current);
            self.secondary_alternate = secondary.alternate;
            self.secondary_alternate_timing = secondary.alternate_timing;
        }
        self
    }

    pub fn with_alternate(mut self, alternate: Player, timing: AlternateTiming) -> Self {
        self.current.equipment.inventory_weapon = Some(self.current.equipment.weapon);
        let mut other = alternate.current;
        other.equipment.inventory_weapon = self.current.equipment.inventory_weapon;
        other.equipment.alternate_weapon = Some(self.current.equipment.weapon);
        self.alternate = Some(other);
        self.alternate_timing = Some(timing);
        self.sync_ammunition();
        self
    }

    pub fn owns_weapon(&self, requested: u16) -> bool {
        std::iter::once(&self.current)
            .chain(&self.secondary)
            .chain(&self.alternate)
            .chain(&self.secondary_alternate)
            .any(|w| w.equipment.weapon == requested)
    }

    fn sync_ammunition(&mut self) {
        let Some(source) = self.current.equipment.ammo else {
            return;
        };
        for other in self
            .secondary
            .iter_mut()
            .chain(&mut self.alternate)
            .chain(&mut self.secondary_alternate)
        {
            if let Some(ammo) = &mut other.equipment.ammo {
                if ammo.clip_key == source.clip_key {
                    ammo.clip = source.clip;
                }
                if ammo.reserve_key == source.reserve_key {
                    ammo.reserve = source.reserve;
                }
            }
        }
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
        equipment.needs_rechamber = self.current.needs_rechamber;
        equipment.spread_scale = self.spread_scale;
        equipment.secondary = self.secondary.as_ref().map(|other| EquippedWeapon {
            weapon: other
                .equipment
                .inventory_weapon
                .unwrap_or(other.equipment.weapon),
            ammo: other.equipment.ammo,
            needs_rechamber: other.needs_rechamber,
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
        self.update_spread(cmd, elapsed);
        if cmd.buttons & 1 == 0 {
            self.current.equipment.fire_count = 0;
        }
        self.apply_weapon(cmd, elapsed);
        self.sync_ammunition();
        self.apply_aim(cmd, elapsed);
        self.attack_held = cmd.buttons & 1 != 0;
        self.last_angles = Some(cmd.angles);
    }

    /// Target0x422aa0/0x425840: PS+0x350 uses0..255. Movement and
    /// turning add spread; stationary recovery and stance affect decay.
    fn update_spread(&mut self, cmd: &UserCommand, elapsed: u32) {
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
    fn shot_spread(&self) -> f32 {
        let Some(timing) = self.current.spread else {
            return self.current.ballistics.map_or(0.0, |b| b.spread_degrees);
        };
        let scale = self.spread_scale / 255.0;
        let min = timing.minimum[self.spread_stance];
        let max = timing.maximum[self.spread_stance];
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
        let reloading = firing.is_some_and(|f| reloading(f.state));
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

    fn needs_shell(&self) -> bool {
        self.current
            .equipment
            .ammo
            .is_some_and(|a| a.clip < self.current.magazine_capacity && a.reserve > 0)
    }

    fn add_shells(&mut self, count: u16) {
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
    fn begin_shell(&mut self) {
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

    fn apply_segmented_reload(
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

    fn begin_drop(&mut self, requested: u16) {
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

    fn begin_alternate_drop(&mut self, requested: u16) {
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
                    .is_some_and(|other| other.equipment.weapon == cmd.weapons[0])
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
            && self.secondary.as_ref().is_some_and(|other| {
                other.equipment.weapon == cmd.weapons[0] && other.switch.is_some()
            })
        {
            self.begin_drop(cmd.weapons[0]);
            return;
        }
        if raising {
            return;
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
        let trigger = cmd.buttons & 1 != 0
            && (self.current.fire_type == 0 || self.current.equipment.fire_count == 0);
        if allowed && trigger && ammo.clip > 0 {
            let remaining_clip = ammo.clip - 1;
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
                self.current.equipment.fire_count = (self.current.equipment.fire_count + 1).min(4);
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
        } else {
            if state != 0 {
                self.current.ready();
            }
            // Target0x424c70 pins weaponTime at1 while a semiautomatic
            // trigger has exhausted its shot count. Preserve that in the
            // authority snapshot so prediction cannot begin another shot.
            if allowed
                && cmd.buttons & 1 != 0
                && self.current.fire_type != 0
                && self.current.equipment.fire_count != 0
                && let Some(firing) = &mut self.current.equipment.firing
            {
                firing.remaining_msec = 1;
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
    fn single_bullets_use_stance_spread_and_ads_and_movement_changes() {
        let mut p = aiming_player();
        p.set_ballistics(crate::game_combat::Ballistics {
            damage: 33,
            range: 8192.0,
            near_range: 500.0,
            location_multipliers: None,
            falloff: None,
            pellets: 1,
            spread_degrees: 0.0,
        });
        p.set_spread_timing(SpreadTiming {
            minimum: [3.0, 2.0, 1.75],
            maximum: [6.0, 4.5, 4.0],
            decay: 4.0,
            fire_add: 0.6,
            turn_add: 0.0,
            move_add: 1.75,
            stance_decay: [1.0, 1.05, 1.1],
            ads: 0.0,
        });
        p.apply(&cmd(1100, 1));
        assert_eq!(p.take_shots()[0].ballistics.spread_degrees, 3.0);
        p.apply(&cmd(1179, 1));
        assert!(p.take_shots()[0].ballistics.spread_degrees > 3.0);
        p.apply(&cmd(1600, 0));
        p.apply(&cmd(1800, 0));
        p.apply(&cmd(2000, 0));
        p.spread_stance = 2;
        assert_eq!(p.shot_spread(), 1.75);
        p.apply(&cmd(2400, 0x800));
        p.apply(&cmd(2600, 0x800));
        assert_eq!(p.shot_spread(), 0.0);
        p.spread_speed = 1.0;
        p.apply(&cmd(2700, 0));
        p.apply(&cmd(2900, 0));
        assert!(p.equipment().spread_scale > 0.0);
    }
    #[test]
    fn pump_weapon_waits_for_fire_and_rechamber_and_ejects_once() {
        let mut p = switchable_player(352, 1.0);
        p.current.fire_interval = Some(192);
        p.set_rechamber_timing(RechamberTiming {
            duration_msec: 550,
            eject_msec: 0,
        });
        p.apply(&select(1100, 352, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        assert!(p.equipment().needs_rechamber);
        p.apply(&select(1291, 352, 1));
        assert_eq!(p.equipment().firing.unwrap().state, 6);
        p.apply(&select(1292, 352, 1));
        assert_eq!(p.equipment().firing.unwrap().state, 7);
        assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 4);
        let sequence = p.equipment().events.unwrap().sequence;
        p.apply(&select(1293, 352, 0));
        assert_eq!(
            p.equipment().events.unwrap().sequence,
            sequence.wrapping_add(1)
        );
        p.apply(&select(1500, 352, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        assert!(p.equipment().needs_rechamber);
        assert_eq!(
            p.equipment().events.unwrap().sequence,
            sequence.wrapping_add(1)
        );
        p.apply(&select(1842, 352, 0));
        assert!(!p.equipment().needs_rechamber);
        p.apply(&select(1843, 352, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
    }
    #[test]
    fn alternate_switch_uses_native_phases_and_keeps_shared_ammo_and_secondary() {
        let mut primary = switchable_player(352, 1.0);
        primary.current.equipment.ammo.as_mut().unwrap().clip_key = 352;
        let mut alternate = switchable_player(353, 0.9);
        alternate.current.equipment.ammo.as_mut().unwrap().clip_key = 352;
        alternate.current.fire_interval = Some(400);
        let mut p = primary
            .with_alternate(
                alternate,
                AlternateTiming {
                    drop_msec: 0,
                    raise_msec: 1399,
                },
            )
            .with_secondary(switchable_player(354, 1.0));
        p.apply(&select(1100, 352, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&select(1200, 353, 0));
        assert_eq!(p.equipment().firing.unwrap().state, 5);
        assert_eq!(p.equipment().firing.unwrap().animation & 0x3ff, 19);
        p.apply(&select(1250, 353, 1));
        assert_eq!(p.equipment().weapon, 353);
        assert_eq!(p.equipment().inventory_weapon, Some(352));
        assert_eq!(p.equipment().firing.unwrap().state, 2);
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&select(2649, 353, 0));
        p.apply(&select(2700, 353, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
        p.apply(&select(2800, 352, 0));
        p.apply(&select(2850, 352, 0));
        p.apply(&select(4249, 352, 0));
        assert_eq!(p.equipment().weapon, 352);
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
        assert_eq!(p.equipment().secondary.unwrap().weapon, 354);
        assert!(p.owns_weapon(353));
        assert!(!p.owns_weapon(355));
    }
    #[test]
    fn only_authorized_shots_are_forwarded_once_to_the_combat_world() {
        let mut p = player();
        p.set_ballistics(crate::game_combat::Ballistics {
            damage: 33,
            range: 8192.0,
            near_range: 500.0,
            location_multipliers: None,
            falloff: None,
            pellets: 0,
            spread_degrees: 0.0,
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
                inventory_weapon: None,
                alternate_weapon: None,
                fire_count: 0,
                spread_scale: 0.0,
                needs_rechamber: false,
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
            assert_eq!(p.equipment().fire_count, 1);
            assert_eq!(p.equipment().events.unwrap().sequence, 1);
            p.apply(&cmd(1301, 0));
            assert_eq!(p.equipment().fire_count, 0);
            p.apply(&cmd(1302, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 28);
            // Native shot-count gating lets a new press wait for cooldown,
            // then fires once; keeping it held does not fire another shot.
            p.apply(&cmd(1303, 0));
            p.apply(&cmd(1304, 1));
            p.apply(&cmd(1400, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 27);
            p.apply(&cmd(1401, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 27);
            p.apply(&cmd(1402, 0));
            p.apply(&cmd(1403, 1));
            p.apply(&cmd(1480, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 26);
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
            segmented: None,
        }
    }
    /// spas12_mp: start1000 (+shell at600), shell550 (+200), end850.
    fn shotgun() -> Player {
        let mut p = player();
        p.current.reload = Some(ReloadTiming {
            normal_msec: 550,
            empty_msec: 550,
            add_msec: 200,
            empty_add_msec: 400,
            segmented: Some(SegmentedReload {
                start_msec: 1000,
                start_add_msec: 600,
                end_msec: 850,
            }),
        });
        p.current.magazine_capacity = 4;
        let ammo = p.current.equipment.ammo.as_mut().unwrap();
        ammo.clip = 1;
        ammo.reserve = 10;
        p
    }
    fn state(p: &Player) -> (u8, u16, u16) {
        let firing = p.equipment().firing.unwrap();
        let ammo = p.equipment().ammo.unwrap();
        (firing.state, firing.animation & 0x3ff, ammo.clip)
    }
    #[test]
    fn shotgun_reload_starts_loads_one_shell_per_cycle_and_ends() {
        let mut p = shotgun();
        p.apply(&cmd(1100, 0x10));
        assert_eq!(state(&p), (10, 17, 1));
        p.apply(&cmd(1700, 0));
        assert_eq!(state(&p), (10, 17, 2)); // Shell inserted during the start.
        p.apply(&cmd(2100, 0));
        assert_eq!(state(&p), (8, 13, 2));
        p.apply(&cmd(2300, 0));
        assert_eq!(state(&p), (8, 13, 3));
        p.apply(&cmd(2650, 0));
        assert_eq!(state(&p), (8, 13, 3)); // Next shell began.
        p.apply(&cmd(2850, 0));
        assert_eq!(state(&p), (8, 13, 4));
        p.apply(&cmd(3200, 0));
        assert_eq!(state(&p), (12, 18, 4)); // Full: reload end.
        p.apply(&cmd(4050, 0));
        assert_eq!(state(&p).0, 0);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 7);
    }
    #[test]
    fn firing_interrupts_a_shotgun_reload_after_the_current_shell() {
        let mut p = shotgun();
        p.apply(&cmd(1100, 0x10));
        p.apply(&cmd(2100, 0));
        assert_eq!(state(&p), (8, 13, 2));
        p.apply(&cmd(2200, 1)); // Fire pressed during the shell.
        assert_eq!(state(&p).0, 9);
        p.apply(&cmd(2650, 0));
        assert_eq!(state(&p), (12, 18, 3)); // Shell completed, then end.
        assert!(p.take_shots().is_empty());
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
    fn quick_wheel_switches_follow_native_interrupts_without_a_second_raise() {
        let mut p = switchable_player(352, 0.9).with_secondary(switchable_player(180, 1.0));
        p.apply(&select(1100, 352, 0));
        // Returning during the drop finishes the change at once (0x423b10).
        p.apply(&select(1200, 180, 0));
        assert_eq!(p.equipment().firing.unwrap().state, 3);
        p.apply(&select(1250, 352, 0));
        assert_eq!(p.equipment().weapon, 352);
        assert_eq!(p.equipment().firing.unwrap().state, 1);
        assert_eq!(p.equipment().firing.unwrap().remaining_msec, 400);
        p.apply(&select(1650, 352, 0));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        // A new request interrupts the raise and drops immediately (0x4236a0).
        p.apply(&select(1700, 180, 0));
        p.apply(&select(2000, 180, 0));
        assert_eq!(
            (p.equipment().weapon, p.equipment().firing.unwrap().state),
            (180, 1)
        );
        p.apply(&select(2100, 352, 0));
        assert_eq!(
            (p.equipment().weapon, p.equipment().firing.unwrap().state),
            (180, 3)
        );
        p.apply(&select(2400, 352, 0));
        assert_eq!(
            (p.equipment().weapon, p.equipment().firing.unwrap().state),
            (352, 1)
        );
        p.apply(&select(2800, 352, 0));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        assert_eq!(p.equipment().secondary.unwrap().weapon, 180);
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
