//! A ground weapon carries its own authored behavior and live ammunition.
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct DroppedWeapon {
    primary: Weapon,
    alternate: Option<Weapon>,
    timing: Option<AlternateTiming>,
}
impl DroppedWeapon {
    pub fn index(&self) -> u16 {
        self.primary.equipment.weapon
    }
    pub fn ammo(&self) -> Option<codol_protocol::game::snapshot::Ammo> {
        self.primary.equipment.ammo
    }
    pub fn empty(&self) -> bool {
        std::iter::once(&self.primary)
            .chain(&self.alternate)
            .all(|w| {
                w.equipment
                    .ammo
                    .is_none_or(|a| a.clip == 0 && a.reserve == 0)
            })
    }
    fn reset(&mut self) {
        for w in std::iter::once(&mut self.primary).chain(&mut self.alternate) {
            w.ready();
            w.cancel_burst();
            w.rechamber_pending = false;
            w.equipment.aim = w.ads.map(|_| Aim::default());
            w.equipment.events = None;
            w.equipment.secondary = None;
            w.equipment.alternates = [None; 2];
        }
    }
}

impl HeldWeapons {
    fn ordinary_bundle(&self) -> DroppedWeapon {
        // While a reward is selected it holds the previously active ordinary
        // gun; the radio/marker itself must never become a death weapon.
        let (current, alternate, timing) = self.active_reward.map_or(
            (&self.current, &self.alternate, self.alternate_timing),
            |slot| {
                let saved = self.rewards[slot].as_ref().unwrap();
                (&saved.weapon, &saved.alternate, saved.alternate_timing)
            },
        );
        let mut bundle = if current
            .equipment
            .inventory_weapon
            .is_some_and(|parent| parent != current.equipment.weapon)
        {
            DroppedWeapon {
                primary: alternate.as_ref().expect("owned alternate parent").clone(),
                alternate: Some(current.clone()),
                timing,
            }
        } else {
            DroppedWeapon {
                primary: current.clone(),
                alternate: alternate.clone(),
                timing,
            }
        };
        bundle.reset();
        if let Some(a) = bundle.primary.equipment.ammo
            && let Some(other) = bundle
                .alternate
                .as_mut()
                .and_then(|w| w.equipment.ammo.as_mut())
        {
            if other.clip_key == a.clip_key {
                other.clip = a.clip;
            }
            if other.reserve_key == a.reserve_key {
                other.reserve = a.reserve;
            }
        }
        bundle
    }
    pub fn take_death_weapon(&mut self) -> Option<DroppedWeapon> {
        if self.reward_loadout {
            return None;
        }
        if std::mem::replace(&mut self.death_weapon_taken, true) {
            return None;
        }
        let mut bundle = self.ordinary_bundle();
        let ammo = bundle.primary.equipment.ammo?;
        if ammo.clip == 0 || bundle.primary.marker.is_some() {
            return None;
        }
        if let Some(limits) = bundle.primary.ammo_limits {
            bundle.primary.equipment.ammo.as_mut().unwrap().reserve =
                ammo.reserve.min(limits.contribution.min(2047) as u16);
        }
        if let Some(a) = bundle.primary.equipment.ammo
            && let Some(other) = bundle
                .alternate
                .as_mut()
                .and_then(|w| w.equipment.ammo.as_mut())
            && other.reserve_key == a.reserve_key
        {
            other.reserve = a.reserve;
        }
        Some(bundle)
    }
    pub fn can_use_ground_weapon(&self) -> bool {
        self.active_reward.is_none()
            && self.selected_offhand.is_none()
            && self.switch_target.is_none()
            && self
                .current
                .equipment
                .firing
                .is_none_or(|f| !matches!(f.state, 1..=5 | 13..=15 | 17..=21))
    }
    pub fn can_swap_ground_weapon(&self, incoming: &DroppedWeapon) -> bool {
        self.can_use_ground_weapon()
            && !self
                .weapons()
                .any(|w| w.equipment.weapon == incoming.index())
            && incoming.primary.equipment.ammo.is_some()
    }
    pub fn swap_ground_weapon(&mut self, mut incoming: DroppedWeapon) -> Option<DroppedWeapon> {
        if !self.can_swap_ground_weapon(&incoming) {
            return None;
        }
        self.sync_ammunition();
        let mut old = self.ordinary_bundle();
        // A reserve pool belongs to the held inventory. Only stock exceeding
        // the remaining gun's capacity leaves with the replaced weapon.
        if let Some(ammo) = old.primary.equipment.ammo {
            let remaining = self.secondary.as_ref().filter(|w| {
                w.equipment
                    .ammo
                    .is_some_and(|a| a.reserve_key == ammo.reserve_key)
            });
            if let Some(remaining) = remaining {
                let cap = remaining
                    .ammo_limits
                    .map_or(0, |l| l.shared.map_or(l.contribution, |g| g[1]));
                let kept = ammo.reserve.min(cap.min(2047) as u16);
                old.primary.equipment.ammo.as_mut().unwrap().reserve -= kept;
                if let Some(a) = self
                    .secondary
                    .as_mut()
                    .and_then(|w| w.equipment.ammo.as_mut())
                {
                    a.reserve = kept;
                }
            }
        }
        if let Some(a) = old.primary.equipment.ammo
            && let Some(other) = old
                .alternate
                .as_mut()
                .and_then(|w| w.equipment.ammo.as_mut())
            && other.reserve_key == a.reserve_key
        {
            other.reserve = a.reserve;
        }
        incoming.reset();
        if let Some(a) = &mut incoming.primary.equipment.ammo
            && let Some(other) = self.secondary.as_ref().and_then(|w| w.equipment.ammo)
            && a.reserve_key == other.reserve_key
        {
            a.reserve = a.reserve.saturating_add(other.reserve).min(2047);
        }
        let reserve = incoming.primary.equipment.ammo.unwrap().reserve;
        self.current = incoming.primary;
        self.alternate = incoming.alternate;
        self.alternate_timing = incoming.timing;
        self.current.fast_reload = old.primary.fast_reload;
        if let Some(other) = &mut self.alternate {
            other.fast_reload = self.current.fast_reload;
        }
        self.switch_target = None;
        self.attack_held = true;
        self.melee_held = true;
        self.spread_scale = 0.0;
        self.current.equipment.ammo.as_mut().unwrap().reserve = 0;
        if let Some(other) = self
            .secondary
            .as_mut()
            .and_then(|w| w.equipment.ammo.as_mut())
            && other.reserve_key == self.current.equipment.ammo.unwrap().reserve_key
        {
            other.reserve = 0;
        }
        let cap = self.ammo_space(&self.current);
        self.current.equipment.ammo.as_mut().unwrap().reserve = reserve.min(cap.min(2047) as u16);
        self.sync_ammunition();
        self.emit(
            events::WEAPON_SWITCH_STARTED,
            u32::from(self.current.equipment.weapon),
        );
        self.emit(
            events::ITEM_PICKUP,
            u32::from(self.current.equipment.weapon),
        );
        self.begin_raise();
        Some(old)
    }
    /// Consume only ammo that a compatible held reserve pool can accept.
    pub fn leech_ground_ammo(&mut self, item: &mut DroppedWeapon) -> bool {
        if !self.can_use_ground_weapon() {
            return false;
        }
        let Some(source) = item.primary.equipment.ammo else {
            return false;
        };
        let Some(target) = self.inventory().find(|w| {
            w.equipment
                .ammo
                .is_some_and(|a| a.reserve_key == source.reserve_key)
        }) else {
            return false;
        };
        let amount = (u32::from(source.clip) + u32::from(source.reserve))
            .min(self.ammo_space(target)) as u16;
        if amount == 0 {
            return false;
        }
        let new_stock = target.equipment.ammo.unwrap().reserve + amount;
        for w in std::iter::once(&mut self.current)
            .chain(&mut self.secondary)
            .chain(&mut self.alternate)
            .chain(&mut self.secondary_alternate)
        {
            if let Some(a) = &mut w.equipment.ammo
                && a.reserve_key == source.reserve_key
            {
                a.reserve = new_stock;
            }
        }
        let a = item.primary.equipment.ammo.as_mut().unwrap();
        let stock = amount.min(a.reserve);
        a.reserve -= stock;
        a.clip -= amount - stock;
        if let Some(other) = item
            .alternate
            .as_mut()
            .and_then(|w| w.equipment.ammo.as_mut())
        {
            other.clip = a.clip;
            other.reserve = a.reserve;
        }
        self.emit(events::ITEM_PICKUP, u32::from(item.index()));
        true
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use codol_protocol::game::snapshot::Ammo;
    pub(crate) fn gun(id: u16, key: u16, clip: u16, reserve: u16) -> HeldWeapons {
        let mut p = HeldWeapons::new(
            Equipment {
                weapon: id,
                inventory_weapon: None,
                inventory_override: None,
                alternate_weapon: None,
                alternates: [None; 2],
                ammo: Some(Ammo {
                    clip_key: id,
                    reserve_key: key,
                    clip,
                    reserve,
                }),
                viewmodel: Some(54),
                firing: None,
                aim: None,
                secondary: None,
                events: None,
                offhands: [None; 2],
                offhand_weapon: 0,
                grenade_time: 0,
                move_multiplier: 1.0,
                perks: [0; 3],
                abilities: Default::default(),
                fire_count: 0,
                queued_burst: false,
                barrel_flags: 0,
                spread_scale: 0.0,
                needs_rechamber: false,
            },
            Some(79),
            None,
            None,
        );
        p.set_ammo_limits(AmmoLimits {
            contribution: 180,
            shared: None,
            clip_only: false,
            scavenger_allowed: true,
        });
        p.current.magazine_capacity = 30;
        p.set_switch_timing(SwitchTiming {
            drop_msec: 100,
            raise_msec: 200,
            empty_drop_msec: 100,
            empty_raise_msec: 200,
        });
        p
    }
    #[test]
    fn death_preserves_partial_ammo_is_once_per_life_and_empty_magazine_does_not_drop() {
        let mut p = gun(352, 163, 7, 72);
        let item = p.take_death_weapon().unwrap();
        assert_eq!(item.index(), 352);
        assert_eq!(
            (item.ammo().unwrap().clip, item.ammo().unwrap().reserve),
            (7, 72)
        );
        assert!(p.take_death_weapon().is_none());
        assert_eq!(p.equipment().ammo.unwrap().clip, 7);
        assert!(gun(352, 163, 0, 72).take_death_weapon().is_none());
    }
    #[test]
    fn alternate_and_active_reward_drop_the_saved_parent_with_actual_ammunition() {
        let mut alt = gun(353, 163, 7, 72);
        alt.current.equipment.ammo.as_mut().unwrap().clip_key = 352;
        let mut p = gun(352, 163, 7, 72).with_alternate(
            alt,
            AlternateTiming {
                drop_msec: 100,
                raise_msec: 100,
            },
        );
        std::mem::swap(&mut p.current, p.alternate.as_mut().unwrap());
        p.current.equipment.ammo.as_mut().unwrap().clip = 3;
        p.sync_ammunition();
        let item = p.clone().take_death_weapon().unwrap();
        assert_eq!(item.index(), 352);
        assert_eq!(item.ammo().unwrap().clip, 3);
        p.grant_reward(
            1,
            300,
            SwitchTiming {
                drop_msec: 100,
                raise_msec: 100,
                empty_drop_msec: 100,
                empty_raise_msec: 100,
            },
            100,
        );
        let r = p.rewards[1].as_mut().unwrap();
        std::mem::swap(&mut p.current, &mut r.weapon);
        std::mem::swap(&mut p.alternate, &mut r.alternate);
        p.active_reward = Some(1);
        assert_eq!(p.take_death_weapon().unwrap().index(), 352);
    }
    #[test]
    fn swap_transfers_weapon_behavior_and_preserves_secondary_and_player_state() {
        let mut p = gun(352, 163, 7, 72).with_secondary(gun(354, 164, 10, 40));
        let mut incoming = gun(355, 165, 3, 22);
        incoming.current.fire_interval = Some(1500);
        incoming.current.move_scale = 0.72;
        let old = p
            .swap_ground_weapon(incoming.take_death_weapon().unwrap())
            .unwrap();
        assert_eq!(
            (
                old.index(),
                old.ammo().unwrap().clip,
                old.ammo().unwrap().reserve
            ),
            (352, 7, 72)
        );
        assert_eq!(p.current.fire_interval, Some(1500));
        assert_eq!(p.current.magazine_capacity, 30);
        assert_eq!(p.move_scale(), 0.72);
        assert_eq!(
            (
                p.equipment().weapon,
                p.equipment().ammo.unwrap().clip,
                p.equipment().ammo.unwrap().reserve
            ),
            (355, 3, 22)
        );
        assert_eq!(p.equipment().secondary.unwrap().weapon, 354);
        assert_eq!(p.equipment().secondary.unwrap().ammo.unwrap().reserve, 40);
        assert!(!p.owns_weapon(352));
        assert!(
            p.swap_ground_weapon(old).is_none(),
            "raising forbids a second swap"
        );
    }
    #[test]
    fn shared_pool_stock_is_partitioned_on_swap_without_duplication() {
        let mut second = gun(354, 163, 10, 90);
        second.current.ammo_limits.as_mut().unwrap().contribution = 60;
        let mut p = gun(352, 163, 7, 90).with_secondary(second);
        let item = gun(355, 165, 3, 22).take_death_weapon().unwrap();
        let old = p.swap_ground_weapon(item).unwrap();
        assert_eq!(old.ammo().unwrap().reserve, 30);
        assert_eq!(p.equipment().secondary.unwrap().ammo.unwrap().reserve, 60);
    }
    #[test]
    fn compatible_ammo_only_consumes_real_stock_up_to_capacity() {
        let mut p = gun(352, 163, 7, 178);
        let mut item = gun(355, 163, 3, 22).take_death_weapon().unwrap();
        assert!(p.leech_ground_ammo(&mut item));
        assert_eq!(p.equipment().ammo.unwrap().reserve, 180);
        assert_eq!(
            (item.ammo().unwrap().clip, item.ammo().unwrap().reserve),
            (3, 20)
        );
        assert!(!p.leech_ground_ammo(&mut item));
        assert!(!p.leech_ground_ammo(&mut gun(356, 166, 3, 22).take_death_weapon().unwrap()));
    }
}
