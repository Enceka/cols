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

/// Ordinary held-gun melee: WeaponDef+0x714/+0x6f4/+0x75c/+0x6e0.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MeleeTiming {
    pub duration_msec: u16,
    pub hit_msec: u16,
    pub recover_msec: u16,
    pub damage: u16,
    #[serde(default)]
    pub charge: Option<MeleeChargeTiming>,
}
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MeleeChargeTiming {
    pub duration_msec: u16,
    pub hit_msec: u16,
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct MeleeApproach {
    pub yaw: u16,
    pub distance: u8,
}
impl MeleeTiming {
    pub fn valid(self) -> bool {
        (1..=15000).contains(&self.duration_msec)
            && self.hit_msec <= self.duration_msec
            && self.recover_msec <= 15000
            && self.damage > 0
            && self.charge.is_none_or(|c| {
                (1..=15000).contains(&c.duration_msec)
                    && (1..=c.duration_msec).contains(&c.hit_msec)
            })
    }
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
    offhand_switch: Option<AlternateTiming>,
    fast_reload: bool,
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
    melee: Option<MeleeTiming>,
}
impl Weapon {
    fn attack_mask(&self) -> u32 {
        if self.fire_type == 5 { 0x100001 } else { 1 }
    }
    fn empty(&self) -> bool {
        self.equipment.ammo.is_some_and(|ammo| ammo.clip == 0)
    }
    fn is_burst(&self) -> bool {
        (2..=4).contains(&self.fire_type)
    }
    fn burst_pending(&self) -> bool {
        self.is_burst()
            && self.equipment.fire_count > 0
            && u32::from(self.equipment.fire_count) < self.fire_type
            && !self.empty()
            && self.equipment.firing.is_some_and(|f| f.state == 6)
    }
    fn cancel_burst(&mut self) {
        if self.is_burst() {
            self.equipment.fire_count = 0;
            self.equipment.queued_burst = false;
        }
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
    offhands: [Option<crate::game_grenade::Equipped>; 2],
    selected_offhand: Option<usize>,
    grenade_time: u16,
    offhand_held: u32,
    offhand_pending: bool,
    throws: Vec<crate::game_grenade::Throw>,
    perks: crate::game_perks::Perks,
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
    melee_held: bool,
    shots: Vec<Shot>,
    melee_strikes: Vec<MeleeStrike>,
    melee_approach: Option<(MeleeApproach, u16)>,
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct MeleeStrike {
    pub time: u32,
    pub angles: [u16; 3],
    pub weapon: u16,
    pub damage: u16,
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
            offhands: [None, None],
            selected_offhand: None,
            grenade_time: 0,
            offhand_held: 0,
            offhand_pending: false,
            throws: Vec::new(),
            perks: Default::default(),
            current: Weapon {
                offhand_switch: None,
                fast_reload: false,
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
                melee: None,
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
            melee_held: false,
            shots: Vec::new(),
            melee_strikes: Vec::new(),
            melee_approach: None,
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
    pub fn set_perks(&mut self, perks: crate::game_perks::Perks) {
        self.perks = perks;
    }
    pub fn set_fast_reload(&mut self, allowed: bool) {
        self.current.fast_reload = allowed;
    }
    pub fn set_offhand_switch(&mut self, timing: Option<AlternateTiming>) {
        self.current.offhand_switch = timing;
    }
    pub fn set_offhands(&mut self, offhands: [Option<crate::game_grenade::Equipped>; 2]) {
        self.offhands = offhands;
    }
    pub fn take_throws(&mut self) -> Vec<crate::game_grenade::Throw> {
        std::mem::take(&mut self.throws)
    }
    pub fn explosive_damage(&self, damage: u16) -> u16 {
        (f32::from(damage) * self.perks.value("modArmorExplosiveDamageScale", 1.0)).ceil() as u16
    }
    pub fn hides_victim_death_icon(&self) -> bool {
        self.perks.has("modVictimNoDeadIcon")
    }

    /// WeaponDef +0x48c. Target 0x424830 stops fire types 1/5 after one
    /// shot per trigger;0 is automatic;2/3/4 fire that many rounds.
    pub fn set_fire_type(&mut self, fire_type: u32) -> anyhow::Result<()> {
        anyhow::ensure!(
            fire_type <= 5,
            "unsupported special weapon fire type {fire_type}"
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
    pub fn set_melee_timing(&mut self, timing: MeleeTiming) {
        assert!(timing.valid());
        self.current.melee = Some(timing);
    }
    pub fn take_melee_strikes(&mut self) -> Vec<MeleeStrike> {
        std::mem::take(&mut self.melee_strikes)
    }
    pub fn take_melee_approach(&mut self) -> Option<(MeleeApproach, u16)> {
        self.melee_approach.take()
    }
    pub fn disable_melee(&mut self) {
        for weapon in std::iter::once(&mut self.current)
            .chain(&mut self.alternate)
            .chain(&mut self.secondary)
            .chain(&mut self.secondary_alternate)
        {
            weapon.melee = None;
        }
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
        self.current.move_scale * self.perks.value("modMoveSpeedScale", 1.0)
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
        equipment.perks = self.perks.slots;
        equipment.abilities = self.perks.abilities;
        equipment.move_multiplier = self.perks.value("modMoveSpeedScale", 1.0);
        equipment.offhands = self.offhands.each_ref().map(|g| {
            g.as_ref()
                .map(|g| codol_protocol::game_snapshot::OffhandWeapon {
                    weapon: g.definition.index,
                    clip_key: g.definition.clip_key,
                    count: g.count,
                })
        });
        equipment.offhand_weapon = self
            .selected_offhand
            .and_then(|slot| self.offhands[slot].as_ref())
            .map_or(0, |g| g.definition.index);
        equipment.grenade_time = self.grenade_time;
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
        self.apply_with_melee_approach(cmd, None);
    }
    pub fn apply_with_melee_approach(
        &mut self,
        cmd: &UserCommand,
        approach: Option<MeleeApproach>,
    ) {
        if cmd.time <= self.last_time {
            return;
        }
        let elapsed = cmd.time - self.last_time;
        self.last_time = cmd.time;
        self.update_spread(cmd, elapsed);
        if self.current.is_burst() && cmd.buttons & 1 != 0 && !self.attack_held {
            self.current.equipment.queued_burst = true;
        }
        if cmd.buttons & self.current.attack_mask() == 0 && !self.current.is_burst() {
            self.current.equipment.fire_count = 0;
        }
        self.apply_weapon(cmd, elapsed, approach);
        self.sync_ammunition();
        self.apply_aim(cmd, elapsed);
        self.attack_held = cmd.buttons & self.current.attack_mask() != 0;
        self.melee_held = cmd.buttons & 4 != 0;
        self.offhand_held = cmd.buttons & 0xc000;
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
        let melee = firing.is_some_and(|f| (13..=15).contains(&f.state));
        aim.button = cmd.buttons & 0x800 != 0
            && self.selected_offhand.is_none()
            && Self::input_allowed(cmd)
            && !switching
            && !melee
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

    fn begin_raise(&mut self) {
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

    fn begin_alternate_drop(&mut self, requested: u16) {
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

    fn apply_weapon(&mut self, cmd: &UserCommand, elapsed: u32, approach: Option<MeleeApproach>) {
        if self.apply_offhand(cmd, elapsed) {
            return;
        }
        // 0x4249c0 scales the elapsed frame time, not the authored duration.
        let state = self.current.equipment.firing.map_or(0, |f| f.state);
        let scale = if reloading(state) && self.current.fast_reload {
            self.perks.value("modReloadSpeedScale", 1.0)
        } else if (1..=5).contains(&state) {
            self.perks.value("modWeaponSwitchSpeedScale", 1.0)
        } else if matches!(state, 6 | 7) {
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

    fn offhand_time(&self, time: u16) -> u16 {
        (f32::from(time) / self.perks.value("modOffhandSpeedScale", 1.0)).trunc() as u16
    }
    fn finish_offhand(&mut self) {
        self.grenade_time = 0;
        self.offhand_pending = false;
        self.selected_offhand = None;
        let time = self.current.offhand_switch.map_or(0, |t| t.raise_msec);
        self.current.animate(21, 22, self.offhand_time(time), 0);
        self.emit(22, 0); // EV_OFFHAND_END_NOTIFY:0x427c7c
    }
    fn throw_offhand(&mut self, cmd: &UserCommand, in_hand: bool, dropped: bool) {
        let slot = self.selected_offhand.expect("active offhand slot");
        let equipped = self.offhands[slot].as_mut().unwrap();
        if equipped.count == 0 {
            return;
        }
        equipped.count -= 1;
        let definition = equipped.definition.clone();
        self.throws.push(crate::game_grenade::Throw {
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
    fn apply_offhand(&mut self, cmd: &UserCommand, elapsed: u32) -> bool {
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
                18 if cmd.buttons & (if slot == 0 { 0x4000 } else { 0x8000 }) == 0 => {
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
        } else if cmd.buttons & 0x8000 != 0 && self.offhand_held & 0x8000 == 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    fn melee_player() -> Player {
        let mut p = aiming_player();
        p.set_melee_timing(MeleeTiming {
            duration_msec: 699,
            hit_msec: 250,
            recover_msec: 400,
            damage: 135,
            charge: None,
        });
        p
    }
    #[test]
    fn held_gun_melee_hits_once_after_delay_and_never_spends_ammo() {
        let mut p = melee_player();
        let ammo = p.equipment().ammo.unwrap();
        p.apply(&cmd(1100, 4 | 1));
        let swipe = p.equipment().firing.unwrap();
        assert_eq!(
            (swipe.state, swipe.animation & 0x3ff, swipe.delay_msec),
            (13, 8, 250)
        );
        assert!(p.take_shots().is_empty());
        assert!(p.take_melee_strikes().is_empty());
        p.apply(&cmd(1349, 4));
        assert!(p.take_melee_strikes().is_empty());
        let mut hit = cmd(1350, 4);
        hit.angles = [0, 1234, 0];
        p.apply(&hit);
        let strikes = p.take_melee_strikes();
        assert_eq!(strikes.len(), 1);
        assert_eq!(
            (strikes[0].time, strikes[0].angles, strikes[0].damage),
            (1350, hit.angles, 135)
        );
        assert_eq!(p.equipment().firing.unwrap().state, 14);
        p.apply(&hit);
        assert!(p.take_melee_strikes().is_empty());
        p.apply(&cmd(1799, 4));
        let recover = p.equipment().firing.unwrap();
        assert_eq!(
            (
                recover.state,
                recover.animation & 0x3ff,
                recover.remaining_msec
            ),
            (15, 22, 400)
        );
        p.apply(&cmd(2199, 4));
        p.apply(&cmd(2300, 4));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        assert!(p.take_melee_strikes().is_empty());
        assert_eq!(p.equipment().ammo.unwrap().clip, ammo.clip);
        assert_eq!(p.equipment().ammo.unwrap().reserve, ammo.reserve);
        p.apply(&cmd(2301, 0));
        p.apply(&cmd(2302, 4));
        assert_eq!(p.equipment().firing.unwrap().state, 13);
    }
    #[test]
    fn melee_interrupts_reload_without_finishing_its_ammo_transfer() {
        let mut p = melee_player();
        p.current.equipment.ammo.as_mut().unwrap().clip = 5;
        p.apply(&cmd(1100, 16));
        assert_eq!(p.equipment().firing.unwrap().state, 8);
        p.apply(&cmd(1150, 4 | 0x800));
        assert_eq!(p.equipment().firing.unwrap().state, 13);
        assert_eq!(p.equipment().aim.unwrap().fraction, 0.0);
        p.apply(&cmd(1800, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 5);
        assert_eq!(p.take_melee_strikes().len(), 1);
        assert!(!p.current.reload_transfer_pending);
    }
    #[test]
    fn menu_and_weapon_raise_reject_melee_edges_without_buffering_them() {
        let mut p = melee_player();
        p.apply(&cmd(1100, 4 | 0x10000000));
        p.apply(&cmd(1200, 4));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        p.apply(&cmd(1300, 0));
        p.current.animate(1, 11, 500, 0);
        p.apply(&cmd(1400, 4));
        assert_eq!(p.equipment().firing.unwrap().state, 1);
        p.apply(&cmd(1800, 4));
        p.apply(&cmd(1900, 4));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        assert!(p.take_melee_strikes().is_empty());
    }
    #[test]
    fn validated_charge_selects_native_animation_and_queues_only_one_approach() {
        let mut p = melee_player();
        p.current.melee.as_mut().unwrap().charge = Some(MeleeChargeTiming {
            duration_msec: 1159,
            hit_msec: 100,
        });
        let hint = MeleeApproach {
            yaw: 1234,
            distance: 100,
        };
        p.apply_with_melee_approach(&cmd(1100, 4), Some(hint));
        let f = p.equipment().firing.unwrap();
        assert_eq!(
            (f.state, f.animation & 0x3ff, f.remaining_msec, f.delay_msec),
            (13, 9, 1159, 100)
        );
        let (approach, duration) = p.take_melee_approach().unwrap();
        assert_eq!(
            (approach.yaw, approach.distance, duration),
            (1234, 100, 100)
        );
        p.apply_with_melee_approach(&cmd(1150, 4), Some(hint));
        assert!(p.take_melee_approach().is_none());
        assert!(p.take_melee_strikes().is_empty());
        p.apply(&cmd(1200, 0));
        assert_eq!(p.take_melee_strikes().len(), 1);
        let mut unknown = melee_player();
        unknown.apply_with_melee_approach(&cmd(1100, 4), Some(hint));
        assert_eq!(unknown.equipment().firing.unwrap().animation & 0x3ff, 8);
        assert!(unknown.take_melee_approach().is_none());
    }
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
                perks: Default::default(),
                abilities: Default::default(),
                offhands: [None, None],
                offhand_weapon: 0,
                grenade_time: 0,
                move_multiplier: 1.0,
                inventory_weapon: None,
                alternate_weapon: None,
                fire_count: 0,
                queued_burst: false,
                barrel_flags: 0,
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
    fn grenade_player(name: &str) -> Player {
        let mut p = player();
        p.set_offhand_switch(Some(AlternateTiming {
            drop_msec: 80,
            raise_msec: 300,
        }));
        p.set_offhands([
            Some(crate::game_grenade::Equipped {
                definition: crate::game_grenade::captured(name),
                count: 1,
            }),
            None,
        ]);
        p
    }
    #[test]
    fn offhand_pull_hold_throw_delay_consumes_once_and_restores_the_gun() {
        let mut p = grenade_player("frag_grenade_mp");
        let ammo = p.equipment().ammo.unwrap();
        p.apply(&cmd(1100, 0x4000));
        assert_eq!(p.equipment().firing.unwrap().state, 16);
        p.apply(&cmd(1180, 0x4000));
        assert_eq!(p.equipment().firing.unwrap().state, 17);
        p.apply(&cmd(1780, 0x4000));
        assert_eq!(p.equipment().firing.unwrap().state, 18);
        assert_eq!(p.equipment().grenade_time, 3500);
        p.apply(&cmd(1880, 0));
        assert_eq!(p.equipment().firing.unwrap().state, 19);
        p.apply(&cmd(2008, 0));
        assert!(p.take_throws().is_empty());
        p.apply(&cmd(2009, 0));
        let throws = p.take_throws();
        assert_eq!(throws.len(), 1);
        assert_eq!(throws[0].fuse_msec, 3271);
        assert!(!throws[0].in_hand);
        p.apply(&cmd(2009, 0));
        assert!(p.take_throws().is_empty());
        assert_eq!(p.equipment().offhands[0].unwrap().count, 0);
        p.apply(&cmd(2240, 0));
        assert_eq!(p.equipment().firing.unwrap().state, 21);
        p.apply(&cmd(2540, 0));
        assert_eq!(p.equipment().firing.unwrap().state, 0);
        p.apply(&cmd(2600, 0x4000));
        assert!(p.take_throws().is_empty());
        assert_eq!(p.equipment().weapon, 352);
        assert_eq!(p.equipment().ammo.unwrap().clip, ammo.clip);
    }
    #[test]
    fn cooked_expiry_death_drop_and_non_cookable_semtex_keep_distinct_fuses() {
        for name in ["frag_grenade_mp", "semtex_mp"] {
            let mut p = grenade_player(name);
            for t in [1100, 1180, 1780] {
                p.apply(&cmd(t, 0x4000));
            }
            if name == "frag_grenade_mp" {
                p.apply(&cmd(5280, 0x4000));
                let request = p.take_throws().pop().unwrap();
                assert!(request.in_hand);
                assert_eq!(request.fuse_msec, 0);
                p.drop_offhand_on_death(5280, [0; 3]);
                assert!(p.take_throws().is_empty());
            } else {
                p.apply(&cmd(7780, 0x4000));
                assert_eq!(p.equipment().grenade_time, 2000);
                assert!(p.take_throws().is_empty());
                p.drop_offhand_on_death(7800, [0; 3]);
                let request = p.take_throws().pop().unwrap();
                assert!(request.dropped);
                assert_eq!(request.fuse_msec, 2000);
                p.drop_offhand_on_death(7900, [0; 3]);
                assert!(p.take_throws().is_empty());
            }
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
    fn captured_quickdraw_speeds_ads_and_switch_without_changing_weapon_data() {
        let mut p = aiming_player();
        p.set_perks(
            crate::game_perks::Perks::selected([60041, 60066, 0], &[60041, 60066]).unwrap(),
        );
        let rates = p.current.ads.unwrap();
        p.apply(&cmd(1100, 0x800));
        assert!((p.equipment().aim.unwrap().fraction - 0.54).abs() < 1e-6);
        assert_eq!(p.current.ads.unwrap().in_rate, rates.in_rate);
        assert_eq!(p.equipment().perks, [3, 10, 0]);
        assert_eq!(p.equipment().move_multiplier, 1.07);
        assert_eq!(p.move_scale(), 1.07);
        let timing = SwitchTiming {
            drop_msec: 300,
            raise_msec: 300,
            empty_drop_msec: 300,
            empty_raise_msec: 300,
        };
        p.set_switch_timing(timing);
        let mut secondary = player();
        secondary.current.equipment.weapon = 353;
        secondary.set_switch_timing(timing);
        p = p.with_secondary(secondary);
        p.begin_drop(353);
        let request = UserCommand {
            weapons: [353, 0, 0],
            ..cmd(1200, 0)
        };
        p.apply(&request);
        assert_eq!(p.equipment().firing.unwrap().remaining_msec, 145);
        p.apply(&request);
        assert_eq!(p.equipment().firing.unwrap().remaining_msec, 145);
        p.set_perks(Default::default());
        assert_eq!(p.equipment().perks, [0; 3]);
        assert_eq!(p.move_scale(), 1.0);
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
    fn ranger() -> Player {
        let mut p = player();
        p.set_fire_type(5).unwrap();
        p.current.fire_interval = Some(9);
        p.current.equipment.ammo.as_mut().unwrap().clip = 2;
        p.set_ballistics(crate::game_combat::Ballistics {
            damage: 30,
            range: 1200.0,
            near_range: 100.0,
            location_multipliers: None,
            falloff: None,
            pellets: 8,
            spread_degrees: 5.0,
        });
        p
    }
    #[test]
    fn ranger_secondary_trigger_is_semi_automatic_and_switches_barrels_without_release() {
        for first in [1, 0x100000] {
            let mut p = ranger();
            p.apply(&cmd(1001, first));
            assert_eq!(p.equipment().ammo.unwrap().clip, 1);
            assert_eq!(p.equipment().barrel_flags, u8::from(first != 1));
            p.apply(&cmd(1020, first));
            p.apply(&cmd(1030, first));
            assert_eq!(p.equipment().ammo.unwrap().clip, 1);
            let second = if first == 1 { 0x100000 } else { 1 };
            p.apply(&cmd(1040, second));
            assert_eq!(p.equipment().ammo.unwrap().clip, 0);
            assert_eq!(p.equipment().barrel_flags, u8::from(second != 1));
            let shots = p.take_shots();
            assert_eq!(shots.len(), 2);
            assert_eq!(shots[1].time, 1040);
        }
    }
    #[test]
    fn ranger_both_triggers_spend_two_shells_but_never_create_an_absent_shell() {
        for clip in [1, 2] {
            let mut p = ranger();
            p.current.equipment.ammo.as_mut().unwrap().clip = clip;
            p.apply(&cmd(1001, 0x100001));
            assert_eq!(p.equipment().ammo.unwrap().clip, 0);
            assert_eq!(p.equipment().barrel_flags, if clip == 2 { 3 } else { 1 });
            assert_eq!(p.equipment().events.unwrap().sequence, clip as u8);
            assert_eq!(p.equipment().fire_count, clip as u8);
            let shots = p.take_shots();
            assert_eq!(shots.len(), usize::from(clip));
            assert!(
                shots
                    .iter()
                    .all(|s| s.time == 1001 && s.ballistics.pellets == 8)
            );
            p.apply(&cmd(1001, 0x100001));
            assert!(p.take_shots().is_empty());
            p.apply(&cmd(1020, 0));
            assert_eq!(p.equipment().barrel_flags, 1);
        }
        let mut p = player();
        p.apply(&cmd(1001, 0x100000));
        assert_eq!(p.equipment().ammo.unwrap().clip, 30);
    }

    #[test]
    fn special_modes_are_explicitly_rejected_without_changing_fire_mode() {
        let mut p = player();
        p.set_fire_type(1).unwrap();
        for fire_type in [6, 7, u32::MAX] {
            assert!(p.set_fire_type(fire_type).is_err());
        }
        p.apply(&cmd(1100, 1));
        p.apply(&cmd(1200, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
    }
    #[test]
    fn bursts_complete_after_release_and_recover_before_rearming() {
        for count in 2..=4 {
            let mut p = player();
            p.set_fire_type(count).unwrap();
            p.set_ballistics(crate::game_combat::Ballistics {
                damage: 30,
                range: 8192.0,
                near_range: 1000.0,
                location_multipliers: None,
                falloff: None,
                pellets: 0,
                spread_degrees: 0.0,
            });
            p.apply(&cmd(1001, 1));
            for round in 1..count {
                p.apply(&cmd(1001 + round * 79, 0));
                assert_eq!(p.equipment().fire_count, round as u8 + 1);
                let clip = p.equipment().ammo.unwrap().clip;
                p.apply(&cmd(1001 + round * 79, 0));
                assert_eq!(p.equipment().ammo.unwrap().clip, clip);
            }
            assert_eq!(p.equipment().ammo.unwrap().clip, 30 - count as u16);
            let shots = p.take_shots();
            assert_eq!(shots.len(), count as usize);
            assert_eq!(
                shots.iter().map(|s| s.time).collect::<Vec<_>>(),
                (0..count).map(|i| 1001 + i * 79).collect::<Vec<_>>()
            );
            assert!(p.take_shots().is_empty());
            let recovery = 1001 + count * 79;
            p.apply(&cmd(recovery, 0));
            assert_eq!(p.equipment().firing.unwrap().remaining_msec, 200);
            assert_eq!(p.equipment().firing.unwrap().state, 0);
            p.apply(&cmd(recovery + 199, 0));
            assert_eq!(p.equipment().ammo.unwrap().clip, 30 - count as u16);
            p.apply(&cmd(recovery + 200, 0));
            assert_eq!(p.equipment().ammo.unwrap().clip, 30 - count as u16);
            p.apply(&cmd(recovery + 201, 0));
            assert_eq!(p.equipment().fire_count, 0);
            p.apply(&cmd(recovery + 202, 1));
            assert_eq!(p.equipment().ammo.unwrap().clip, 29 - count as u16);
        }
    }
    #[test]
    fn trigger_repress_during_recovery_is_buffered_and_cleared_by_the_next_burst() {
        let mut p = player();
        p.set_fire_type(3).unwrap();
        for time in [1001, 1080, 1159] {
            p.apply(&cmd(time, 1));
        }
        p.apply(&cmd(1200, 0));
        p.apply(&cmd(1201, 1));
        assert!(p.equipment().queued_burst);
        assert_eq!(p.equipment().fire_count, 3);
        p.apply(&cmd(1238, 1));
        assert_eq!(p.equipment().firing.unwrap().remaining_msec, 200);
        p.apply(&cmd(1437, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 27);
        p.apply(&cmd(1438, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 26);
        assert_eq!(p.equipment().fire_count, 1);
        assert!(!p.equipment().queued_burst);
        p.apply(&cmd(1517, 0));
        p.apply(&cmd(1596, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 24);
    }
    #[test]
    fn explicit_reload_interrupts_an_unfinished_burst_before_the_next_round() {
        let mut p = player();
        p.current.reload = Some(reload_timing());
        p.set_fire_type(3).unwrap();
        p.apply(&cmd(1001, 1));
        p.apply(&cmd(1030, 16));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        assert_eq!(p.equipment().fire_count, 0);
        assert_eq!(p.equipment().firing.unwrap().state, 8);
        p.apply(&cmd(1100, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        assert!(!p.equipment().queued_burst);
    }
    #[test]
    fn held_burst_trigger_does_not_repeat_and_empty_clip_truncates_the_burst() {
        let mut p = player();
        p.set_fire_type(3).unwrap();
        for time in (1001..=3001).step_by(79) {
            p.apply(&cmd(time, 1));
        }
        assert_eq!(p.equipment().ammo.unwrap().clip, 27);
        assert_eq!(p.equipment().fire_count, 3);
        assert_eq!(p.equipment().events.unwrap().sequence, 3);
        let mut p = player();
        p.set_fire_type(3).unwrap();
        p.current.equipment.ammo.as_mut().unwrap().clip = 2;
        p.apply(&cmd(1001, 1));
        p.apply(&cmd(1080, 0));
        p.apply(&cmd(1159, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 0);
        assert_eq!(p.equipment().fire_count, 2);
        assert_eq!(p.equipment().firing.unwrap().remaining_msec, 200);
        p.apply(&cmd(1400, 0));
        assert_eq!(p.equipment().fire_count, 0);
        assert_eq!(p.equipment().events.unwrap().sequence, 2);
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
    #[test]
    fn switching_away_cancels_partial_burst_and_does_not_resume_it_on_return() {
        let mut p = switchable_player(352, 1.0);
        p.set_fire_type(3).unwrap();
        let mut p = p.with_secondary(switchable_player(180, 1.0));
        p.apply(&select(1001, 352, 1));
        p.apply(&select(1030, 180, 0));
        assert_eq!(p.equipment().fire_count, 0);
        assert!(!p.equipment().queued_burst);
        p.apply(&select(1330, 180, 0));
        p.apply(&select(1730, 180, 0));
        p.apply(&select(1740, 352, 0));
        p.apply(&select(2040, 352, 0));
        p.apply(&select(2440, 352, 0));
        assert_eq!(p.equipment().weapon, 352);
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&select(2500, 352, 0));
        assert_eq!(p.equipment().ammo.unwrap().clip, 29);
        p.apply(&select(2501, 352, 1));
        assert_eq!(p.equipment().ammo.unwrap().clip, 28);
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
