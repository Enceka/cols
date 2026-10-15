//! Accumulate confirmed local TDM kills; networking and account storage are
//! outside this module. Only a finished match exposes a final reward batch.
use crate::contracts::progression::{MatchAward, Policy, WeaponXp};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct MatchProgress {
    match_id: String,
    policy: Policy,
    owned: BTreeSet<u32>,
    deaths: [u32; 12],
    player_xp: u32,
    weapons: BTreeMap<u32, u32>,
}
impl MatchProgress {
    pub(crate) fn new(match_id: String, policy: Policy, owned: &[u32]) -> Self {
        Self {
            match_id,
            policy,
            owned: owned.iter().copied().collect(),
            deaths: [0; 12],
            player_xp: 0,
            weapons: BTreeMap::new(),
        }
    }
    pub(crate) fn credit(
        &mut self,
        kill: crate::game::modes::tdm::Kill,
        teams: &[u8; 12],
        item: Option<u32>,
    ) {
        let victim = usize::from(kill.victim.client_index);
        if victim >= self.deaths.len() || kill.sequence <= self.deaths[victim] {
            return;
        }
        self.deaths[victim] = kill.sequence;
        if kill.falling
            || kill.attacker != 0
            || victim == 0
            || teams[0] == teams[victim]
            || teams[victim] == 0
        {
            return;
        }
        self.player_xp = self
            .player_xp
            .saturating_add(self.policy.kill_player_xp)
            .saturating_add(if kill.headshot {
                self.policy.headshot_player_bonus
            } else {
                0
            });
        // Melee, grenades and reward devices do not borrow the equipped rifle's
        // identity. An actual mapped held-weapon kill must also be owned.
        if !kill.melee
            && let Some(id) = item.filter(|id| self.owned.contains(id))
        {
            let xp = self.weapons.entry(id).or_default();
            *xp = xp.saturating_add(self.policy.kill_weapon_xp);
        }
    }
    pub(crate) fn final_award(&self) -> Option<MatchAward> {
        (self.player_xp > 0).then(|| MatchAward {
            match_id: self.match_id.clone(),
            player_xp: self.player_xp,
            weapons: self
                .weapons
                .iter()
                .map(|(&item_id, &xp)| WeaponXp { item_id, xp })
                .collect(),
        })
    }
    pub(crate) fn earned_player_xp(&self) -> u32 {
        self.player_xp
    }
    pub(crate) fn update_ownership(&mut self, owned: &[u32]) {
        self.owned = owned.iter().copied().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::modes::tdm::Kill;
    use codol_protocol::game::snapshot::enemy::EnemySnapshot;
    fn kill(victim: u8, sequence: u32) -> Kill {
        Kill {
            falling: false,
            time: 1000,
            attacker: 0,
            weapon: 352,
            headshot: false,
            melee: false,
            sequence,
            victim: EnemySnapshot {
                entity_number: u16::from(victim),
                client_index: victim,
                team: 1,
                model: 4,
                attachment_models: [0; 10],
                attachment_tags: [0; 10],
                origin: [0.; 3],
                angles: [0; 3],
                legs_animation: 0,
                torso_animation: 0,
                weapon: 352,
                events: Default::default(),
                alive: false,
            },
        }
    }
    fn progress() -> MatchProgress {
        MatchProgress::new(
            "1".repeat(32),
            Policy {
                kill_player_xp: 100,
                kill_weapon_xp: 50,
                headshot_player_bonus: 25,
            },
            &[10001, 10049],
        )
    }
    const TEAMS: [u8; 12] = [2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1];
    #[test]
    fn authoritative_enemy_kills_are_unique_and_bind_the_actual_weapon_item() {
        let mut p = progress();
        p.credit(kill(6, 1), &TEAMS, Some(10001));
        p.credit(kill(6, 1), &TEAMS, Some(10049));
        let mut next = kill(6, 2);
        next.headshot = true;
        p.credit(next, &TEAMS, Some(10049));
        let award = p.final_award().unwrap();
        assert_eq!(award.player_xp, 225);
        assert_eq!(
            award.weapons,
            [
                WeaponXp {
                    item_id: 10001,
                    xp: 50
                },
                WeaponXp {
                    item_id: 10049,
                    xp: 50
                }
            ]
        );
        assert_eq!(p.final_award(), Some(award));
    }
    #[test]
    fn self_friendly_environmental_and_bot_kills_award_nothing() {
        let mut p = progress();
        p.credit(kill(0, 1), &TEAMS, Some(10001));
        p.credit(kill(1, 1), &TEAMS, Some(10001));
        let mut fall = kill(6, 1);
        fall.falling = true;
        p.credit(fall, &TEAMS, Some(10001));
        let mut bot = kill(7, 1);
        bot.attacker = 1;
        p.credit(bot, &TEAMS, Some(10001));
        assert!(p.final_award().is_none());
    }
    #[test]
    fn unmapped_unowned_and_melee_kills_cannot_level_the_equipped_rifle() {
        let mut p = progress();
        p.credit(kill(6, 1), &TEAMS, None);
        p.credit(kill(7, 1), &TEAMS, Some(99999));
        let mut melee = kill(8, 1);
        melee.melee = true;
        p.credit(melee, &TEAMS, Some(10001));
        assert_eq!(p.final_award().unwrap().player_xp, 300);
        assert!(p.final_award().unwrap().weapons.is_empty());
    }
    #[test]
    fn authenticated_ownership_refresh_only_credits_later_kills_to_new_items() {
        let mut p = progress();
        p.credit(kill(6, 1), &TEAMS, Some(10002));
        p.update_ownership(&[10001, 10049, 10002]);
        p.credit(kill(7, 1), &TEAMS, Some(10002));
        assert_eq!(
            p.final_award().unwrap().weapons,
            [WeaponXp {
                item_id: 10002,
                xp: 50
            }]
        );
    }
}
