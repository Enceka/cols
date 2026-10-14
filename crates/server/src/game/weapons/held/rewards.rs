use super::*;

impl HeldWeapons {
    pub fn grant_nuke(&mut self, resources: &crate::game::rewards::killstreaks::NukeResources) {
        self.grant_reward(
            0,
            resources.weapon,
            resources.switch,
            resources.fire_interval_msec,
        );
    }
    pub fn grant_reward(&mut self, slot: usize, id: u16, switch: SwitchTiming, fire_interval: u16) {
        if self.owns_weapon(id) || self.rewards[slot].is_some() {
            return;
        }
        let mut equipment = self.current.equipment;
        equipment.weapon = id;
        equipment.inventory_weapon = None;
        equipment.alternate_weapon = None;
        equipment.alternates = [None; 2];
        equipment.viewmodel = Some(id);
        equipment.ammo = Some(codol_protocol::game::snapshot::Ammo {
            clip_key: id,
            reserve_key: id,
            clip: 1,
            reserve: 0,
        });
        let mut player = Self::new(equipment, Some(fire_interval), None, None);
        player.set_switch_timing(switch);
        self.rewards[slot] = Some(RewardWeapon {
            id,
            weapon: player.current,
            alternate: None,
            alternate_timing: None,
        });
    }
    pub fn grant_marker_reward(
        &mut self,
        slot: usize,
        radio: &crate::game::rewards::killstreaks::Radio,
        definition: crate::game::weapons::grenade::Definition,
    ) {
        assert_eq!(radio.weapon, definition.index);
        self.grant_reward(slot, radio.weapon, radio.switch, radio.fire_interval_msec);
        if let Some(reward) = self.rewards[slot].as_mut() {
            reward.weapon.marker = Some(definition);
        }
    }
    pub fn reward_inventory(&self) -> [Option<EquippedWeapon>; 4] {
        self.rewards.each_ref().map(|r| {
            r.as_ref().map(|r| EquippedWeapon {
                weapon: r.weapon.equipment.weapon,
                ammo: r.weapon.equipment.ammo,
                needs_rechamber: r.weapon.needs_rechamber,
            })
        })
    }
    pub fn remove_nuke(&mut self, id: u16) {
        self.remove_reward(id);
    }
    pub fn return_reward(&mut self) {
        self.restore_reward();
        self.begin_raise();
    }
    pub(super) fn restore_reward(&mut self) {
        self.marker_phase = None;
        self.grenade_time = 0;
        if let Some(slot) = self.active_reward.take() {
            let other = self.rewards[slot].as_mut().unwrap();
            std::mem::swap(&mut self.current, &mut other.weapon);
            std::mem::swap(&mut self.alternate, &mut other.alternate);
            std::mem::swap(&mut self.alternate_timing, &mut other.alternate_timing);
        }
    }
    pub fn remove_reward(&mut self, id: u16) {
        if self.current.equipment.weapon == id && self.active_reward.is_some() {
            self.restore_reward();
            self.begin_raise();
        }
        for reward in &mut self.rewards {
            if reward.as_ref().is_some_and(|r| r.id == id) {
                *reward = None;
            }
        }
    }
}
