use super::*;

impl HeldWeapons {
    pub(crate) fn refill_supply(&mut self) {
        let pools = self
            .inventory()
            .filter_map(|w| w.equipment.ammo.map(|a| a.reserve_key))
            .collect::<std::collections::BTreeSet<_>>();
        for key in pools {
            let Some(w) = self
                .inventory()
                .find(|w| w.equipment.ammo.is_some_and(|a| a.reserve_key == key))
            else {
                continue;
            };
            let stock = w
                .equipment
                .ammo
                .unwrap()
                .reserve
                .saturating_add(self.ammo_space(w).min(2047) as u16);
            for w in std::iter::once(&mut self.current)
                .chain(&mut self.secondary)
                .chain(&mut self.alternate)
                .chain(&mut self.secondary_alternate)
            {
                if let Some(a) = &mut w.equipment.ammo
                    && a.reserve_key == key
                {
                    a.reserve = stock;
                }
            }
        }
        for g in self.offhands.iter_mut().flatten() {
            g.count = g.definition.capacity();
        }
        self.sync_ammunition();
    }

    pub fn set_ammo_limits(&mut self, limits: AmmoLimits) {
        self.current.ammo_limits = Some(limits);
    }
    pub(super) fn weapons(&self) -> impl Iterator<Item = &Weapon> {
        std::iter::once(&self.current)
            .chain(&self.secondary)
            .chain(&self.alternate)
            .chain(&self.secondary_alternate)
            .chain(
                self.rewards
                    .iter()
                    .flatten()
                    .flat_map(|r| std::iter::once(&r.weapon).chain(&r.alternate)),
            )
    }
    pub(super) fn inventory(&self) -> impl Iterator<Item = &Weapon> {
        self.weapons().filter(|w| {
            w.equipment
                .inventory_weapon
                .is_none_or(|parent| parent == w.equipment.weapon)
        })
    }
    pub(super) fn ammo_space(&self, weapon: &Weapon) -> u32 {
        let (Some(ammo), Some(limits)) = (weapon.equipment.ammo, weapon.ammo_limits) else {
            return 0;
        };
        if limits.clip_only {
            return 0;
        }
        if let Some([group, cap]) = limits.shared {
            let mut pools = std::collections::BTreeSet::new();
            let used: u32 = self
                .inventory()
                .filter_map(|w| {
                    let a = w.equipment.ammo?;
                    let l = w.ammo_limits?;
                    if l.shared.map(|g| g[0]) != Some(group) {
                        return None;
                    }
                    if l.clip_only {
                        Some(u32::from(a.clip))
                    } else if pools.insert(a.reserve_key) {
                        Some(u32::from(a.reserve))
                    } else {
                        None
                    }
                })
                .sum();
            cap.saturating_sub(used).min(2047 - u32::from(ammo.reserve))
        } else {
            let mut cap: u32 = 0;
            for w in self.inventory() {
                if w.equipment
                    .ammo
                    .is_some_and(|a| a.reserve_key == ammo.reserve_key)
                    && let Some(l) = w.ammo_limits
                {
                    if let Some([_, shared_cap]) = l.shared {
                        cap = shared_cap;
                        break;
                    }
                    cap = cap.saturating_add(l.contribution);
                }
            }
            cap.min(2047).saturating_sub(u32::from(ammo.reserve))
        }
    }
    pub fn can_scavenge(&self) -> bool {
        self.perks.has("modScavenger")
            && self
                .current
                .ammo_limits
                .is_some_and(|l| l.scavenger_allowed && !l.clip_only)
            && self.ammo_space(&self.current) > 0
    }
    pub fn scavenge(&mut self, bag_weapon: u16) -> bool {
        if !self.can_scavenge() {
            return false;
        }
        let mut pools: Vec<(u16, u16)> = Vec::new();
        for w in self.weapons().filter(|w| {
            w.ammo_limits
                .is_some_and(|l| l.scavenger_allowed && !l.clip_only)
        }) {
            if let Some(a) = w.equipment.ammo {
                if let Some((_, amount)) = pools.iter_mut().find(|(key, _)| *key == a.reserve_key) {
                    *amount = (*amount).max(w.magazine_capacity);
                } else {
                    pools.push((a.reserve_key, w.magazine_capacity));
                }
            }
        }
        for (key, amount) in pools {
            let weapon = self
                .weapons()
                .find(|w| w.equipment.ammo.is_some_and(|a| a.reserve_key == key))
                .unwrap();
            let space = self.ammo_space(weapon);
            let stock = weapon
                .equipment
                .ammo
                .unwrap()
                .reserve
                .saturating_add(amount.min(space as u16));
            for w in std::iter::once(&mut self.current)
                .chain(&mut self.secondary)
                .chain(&mut self.alternate)
                .chain(&mut self.secondary_alternate)
            {
                if let Some(a) = &mut w.equipment.ammo
                    && a.reserve_key == key
                {
                    a.reserve = stock;
                }
            }
        }
        for offhand in self.offhands.iter_mut().flatten() {
            if !matches!(
                offhand.definition.name.as_str(),
                "frag_grenade_mp" | "fragshort_grenade_mp" | "semtex_mp"
            ) {
                offhand.count = offhand
                    .count
                    .saturating_add(1)
                    .min(offhand.definition.capacity());
            }
        }
        self.emit(events::ITEM_PICKUP, u32::from(bag_weapon));
        true
    }
    pub(super) fn sync_ammunition(&mut self) {
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
}
