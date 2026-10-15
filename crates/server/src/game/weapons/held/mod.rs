//! Authoritative held weapons, switching, ammunition, ADS and predictable events.
use codol_protocol::game::channel::usercmd::UserCommand;
use codol_protocol::game::snapshot::Aim;
use codol_protocol::game::snapshot::Equipment;
use codol_protocol::game::snapshot::EquippedWeapon;
use codol_protocol::game::snapshot::Firing;
use codol_protocol::game::snapshot::events;
use codol_protocol::game::snapshot::events::EventQueue;

mod dive;
mod parameters;
pub(crate) use dive::DiveTiming;
pub(crate) use dropped::DroppedWeapon;
#[cfg(test)]
pub(crate) use dropped::tests::gun as drop_test_gun;
use parameters::reloading;
pub(crate) use parameters::{
    AdsTiming, AlternateTiming, AmmoLimits, MeleeApproach, MeleeChargeTiming, MeleeTiming,
    RechamberTiming, ReloadTiming, SegmentedReload, SpreadTiming, SprintTiming, SwitchTiming,
};

#[derive(Debug, Clone)]
struct Weapon {
    ammo_limits: Option<AmmoLimits>,
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
    ads_move_scale: f32,
    sprint_duration_scale: f32,
    sprint_timing: Option<SprintTiming>,
    dive_timing: Option<DiveTiming>,
    ballistics: Option<crate::game::combat::Ballistics>,
    rechamber: Option<RechamberTiming>,
    rechamber_pending: bool,
    needs_rechamber: bool,
    spread: Option<SpreadTiming>,
    melee: Option<MeleeTiming>,
    marker: Option<crate::game::weapons::grenade::Definition>,
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
struct RewardWeapon {
    id: u16,
    weapon: Weapon,
    alternate: Option<Weapon>,
    alternate_timing: Option<AlternateTiming>,
}

#[derive(Debug, Clone)]
pub(crate) struct HeldWeapons {
    death_weapon_taken: bool,
    offhands: [Option<crate::game::weapons::grenade::Equipped>; 2],
    selected_offhand: Option<usize>,
    grenade_time: u16,
    offhand_held: u32,
    offhand_pending: bool,
    throws: Vec<crate::game::weapons::grenade::Throw>,
    marker_phase: Option<u8>,
    perks: crate::game::status::perks::Perks,
    sprinting: bool,
    diving: bool,
    dive_sliding: bool,
    support_arrow: bool,
    current: Weapon,
    secondary: Option<Weapon>,
    alternate: Option<Weapon>,
    secondary_alternate: Option<Weapon>,
    rewards: [Option<RewardWeapon>; 4],
    active_reward: Option<usize>,
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
    pub ballistics: crate::game::combat::Ballistics,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MeleeStrike {
    pub time: u32,
    pub angles: [u16; 3],
    pub weapon: u16,
    pub damage: u16,
}

impl HeldWeapons {
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
            death_weapon_taken: false,
            offhands: [None, None],
            selected_offhand: None,
            grenade_time: 0,
            offhand_held: 0,
            offhand_pending: false,
            throws: Vec::new(),
            marker_phase: None,
            perks: Default::default(),
            sprinting: false,
            diving: false,
            dive_sliding: false,
            support_arrow: false,
            current: Weapon {
                offhand_switch: None,
                fast_reload: false,
                ammo_limits: None,
                magazine_capacity: equipment.ammo.map_or(0, |a| a.clip),
                equipment,
                fire_interval,
                fire_type: 0,
                reload,
                reload_transfer_pending: false,
                ads,
                switch: None,
                move_scale: 1.0,
                ads_move_scale: 1.0,
                sprint_duration_scale: 1.0,
                sprint_timing: None,
                dive_timing: None,
                ballistics: None,
                rechamber: None,
                rechamber_pending: false,
                needs_rechamber: false,
                spread: None,
                melee: None,
                marker: None,
            },
            secondary: None,
            alternate: None,
            secondary_alternate: None,
            rewards: std::array::from_fn(|_| None),
            active_reward: None,
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
    pub fn set_perks(&mut self, perks: crate::game::status::perks::Perks) {
        self.perks = perks;
    }
    pub fn set_fast_reload(&mut self, allowed: bool) {
        self.current.fast_reload = allowed;
    }

    pub(crate) fn sentry_undetectable(&self) -> bool {
        self.perks.has("modNoSentryGuns")
    }
    pub fn explosive_damage(&self, damage: u16) -> u16 {
        (f32::from(damage) * self.perks.value("modArmorExplosiveDamageScale", 1.0)).ceil() as u16
    }
    pub fn fall_damage(&self, damage: u16) -> u16 {
        // Defensive scaling follows the supplied MP perk script. The native
        // movement event carries raw landing energy; immunity remains zero.
        (f32::from(damage) * self.perks.value("modFallDamageScale", 1.0)) as u16
    }
    pub fn hides_victim_death_icon(&self) -> bool {
        self.perks.has("modVictimNoDeadIcon")
    }
    pub fn counter_uav_protected(&self) -> bool {
        self.perks.has("modInvalidCounterUav")
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
    pub fn set_ads_move_scale(&mut self, scale: f32) {
        self.current.ads_move_scale = if scale > 0.0 { scale } else { 1.0 };
    }
    pub fn set_sprint_duration_scale(&mut self, scale: f32) {
        self.current.sprint_duration_scale = scale;
    }
    pub fn set_sprint_timing(&mut self, timing: SprintTiming) {
        assert!(timing.valid());
        self.current.sprint_timing = Some(timing);
    }
    pub fn set_sprinting(&mut self, active: bool) {
        self.sprinting = active;
    }
    pub fn sprint_rules(&self) -> crate::game::player::sprint::Rules {
        crate::game::player::sprint::Rules {
            duration_seconds: self.perks.value("modSprintTime", 4.0),
            weapon_duration_scale: self.current.sprint_duration_scale,
            force_recover_seconds: self
                .perks
                .has("modSprintForceRecoverTime")
                .then(|| self.perks.value("modSprintForceRecoverTime", 1.0)),
            always: self.perks.has("modAlwaysSprint"),
            weapon_state: self.current.equipment.firing.map_or(0, |f| f.state),
        }
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

    pub fn move_scale(&self) -> f32 {
        self.current.move_scale * self.perks.value("modMoveSpeedScale", 1.0)
    }
    /// PM_CmdScale0x416c16..0x416cc4 selects WeaponDef+0x7cc while
    /// aiming, replacing +0x7c8. Motion applies the separate native0.4 factor.
    pub fn ads_move_scale(&self) -> f32 {
        self.current.ads_move_scale * self.perks.value("modMoveSpeedScale", 1.0)
    }
    pub fn set_ballistics(&mut self, stats: crate::game::combat::Ballistics) {
        assert!(stats.valid());
        self.current.ballistics = Some(stats);
    }
    pub fn bot_weapon(&self) -> Option<crate::game::ai::controller::BotWeapon> {
        Some(crate::game::ai::controller::BotWeapon {
            weapon: self.current.equipment.weapon,
            ballistics: self.current.ballistics?,
            fire_interval_msec: u32::from(self.current.fire_interval?),
        })
    }
    pub fn take_shots(&mut self) -> Vec<Shot> {
        std::mem::take(&mut self.shots)
    }

    pub fn with_secondary(mut self, secondary: HeldWeapons) -> Self {
        if secondary.current.equipment.weapon != self.current.equipment.weapon {
            self.secondary = Some(secondary.current);
            self.secondary_alternate = secondary.alternate;
            self.secondary_alternate_timing = secondary.alternate_timing;
        }
        self
    }

    pub fn with_alternate(mut self, alternate: HeldWeapons, timing: AlternateTiming) -> Self {
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
        self.weapons().any(|w| w.equipment.weapon == requested)
    }

    /// Called after a saved backpack has been resolved and published. Preserve
    /// the connection's command clock and event counter across that replacement.
    pub fn replace_loadout(&mut self, primary: HeldWeapons, secondary: Option<HeldWeapons>) {
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

    pub fn tactical_protection(&self, kind: crate::game::status::tactical::Kind) -> f32 {
        if matches!(
            kind,
            crate::game::status::tactical::Kind::Gas
                | crate::game::status::tactical::Kind::GasNoKick
        ) {
            return if self.gas_protected() { 0.0 } else { 1.0 };
        }
        self.perks.value(
            match kind {
                crate::game::status::tactical::Kind::Flash => "modFlashProtectionScale",
                crate::game::status::tactical::Kind::Concussion => "modStunProtectionScale",
                crate::game::status::tactical::Kind::Gas
                | crate::game::status::tactical::Kind::GasNoKick => {
                    unreachable!()
                }
            },
            1.0,
        )
    }
    pub fn gas_protected(&self) -> bool {
        self.perks.has("modGasMask")
    }
    pub fn emp_protection(&self) -> f32 {
        // The native table's default is zero even without this modifier.
        // Hardwire activates it with zero; only an active modifier is immunity.
        if self.perks.has("modEmpProtectionScale") {
            self.perks.value("modEmpProtectionScale", 1.0)
        } else {
            1.0
        }
    }
    pub fn asym_robot(&self) -> bool {
        self.perks.has("modAsymRobot")
    }
    pub fn set_support_arrow(&mut self, active: bool) {
        self.support_arrow = active;
    }
    pub fn equipment(&self) -> Equipment {
        let mut equipment = self.current.equipment;
        let mut alternates = self.weapons().filter(|w| {
            w.equipment
                .inventory_weapon
                .is_some_and(|parent| parent != w.equipment.weapon)
        });
        equipment.alternates = std::array::from_fn(|_| {
            alternates.next().map(|w| EquippedWeapon {
                weapon: w.equipment.weapon,
                ammo: w.equipment.ammo,
                needs_rechamber: w.needs_rechamber,
            })
        });
        equipment.perks = self.perks.native_masks();
        equipment.abilities = self.perks.abilities;
        if self.support_arrow {
            equipment.abilities.masks[0] |= 0x80;
            equipment.abilities.values[13] = 1.0;
        }
        equipment.move_multiplier = self.perks.value("modMoveSpeedScale", 1.0);
        equipment.offhands = self.offhands.each_ref().map(|g| {
            g.as_ref()
                .map(|g| codol_protocol::game::snapshot::OffhandWeapon {
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
    /// Frozen controls advance the authority clock without producing weapon
    /// actions or consuming ammunition. Unlock cannot replay the frozen gap.
    pub(crate) fn freeze_at(&mut self, time: u32) {
        self.set_diving(false, false);
        if self
            .current
            .equipment
            .firing
            .is_some_and(|f| (31..=33).contains(&f.state))
        {
            self.current.ready();
        }
        self.last_time = self.last_time.max(time);
        self.attack_held = false;
        self.melee_held = false;
        self.offhand_held = 0;
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
        self.offhand_held = cmd.buttons & 0x14000;
        self.last_angles = Some(cmd.angles);
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
}

#[cfg(test)]
pub(crate) mod tests;

mod reload;

mod actions;

mod offhand;

mod rewards;

mod ammunition;

mod aim;

mod dropped;
