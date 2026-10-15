//! Trusted match-result input at the gameplay/business boundary.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponXp {
    pub item_id: u32,
    pub xp: u32,
}

/// Server-confirmed results only; no client-facing task accepts these values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchAward {
    pub match_id: String,
    pub player_xp: u32,
    /// Canonical ascending marketplace IDs, no duplicates or zero awards.
    pub weapons: Vec<WeaponXp>,
}

/// Explicit local TDM policy, separate from scoreboard points and weapon curves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub kill_player_xp: u32,
    pub kill_weapon_xp: u32,
    pub headshot_player_bonus: u32,
}
impl Policy {
    pub fn valid(self) -> bool {
        (1..=10000).contains(&self.kill_player_xp)
            && (1..=10000).contains(&self.kill_weapon_xp)
            && self.headshot_player_bonus <= 10000
    }
}
