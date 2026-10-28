//! Version-specific progression model and authoritative account transactions.
//! Gameplay policy and native presentation are supplied at their own boundaries.
use anyhow::{Result, ensure};
use codol_protocol::lobby::profile::PlayerProgress;
use serde::{Deserialize, Serialize};

const MAX_XP: u32 = i32::MAX as u32;
pub(crate) mod service;
pub use crate::contracts::progression::{MatchAward, WeaponXp};
pub(crate) use service::Archived;
pub use service::{Commit, Receipt, WeaponResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TablesFile {
    schema_version: u32,
    client_sha256: String,
    rank_version: u32,
    player: Vec<u32>,
    primary: Vec<u32>,
    secondary: Vec<u32>,
}

pub struct Tables {
    pub player: Curve,
    pub primary: Curve,
    pub secondary: Curve,
}

impl Tables {
    pub fn native() -> Result<Self> {
        Self::from_json(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/progression-3.3.3.3.json"
        )))
    }
    /// Numeric thresholds extracted by tools/prepare_progression.py.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let file: TablesFile = serde_json::from_slice(bytes)?;
        ensure!(
            file.schema_version == 1
                && file.client_sha256 == codol_protocol::CLIENT_SHA256
                && file.rank_version == 4
                && file.player.len() == 70
                && file.primary.len() == 31
                && file.secondary.len() == 20,
            "progression table identity/version/count mismatch"
        );
        Ok(Self {
            player: Curve::new(file.player)?,
            primary: Curve::new(file.primary)?,
            secondary: Curve::new(file.secondary)?,
        })
    }

    /// Do not reinterpret a different rank-table version or malformed stored XP.
    /// A fresh native seed has version zero, rank one and zero experience.
    pub fn player_total(&self, stored: PlayerProgress) -> Result<u32> {
        ensure!(
            stored.rank_version == 4
                || (stored.rank_version == 0 && stored.rank == 1 && stored.experience == 0),
            "unsupported stored rank-table version"
        );
        self.player.total(stored.rank, stored.experience)
    }

    pub fn plan_player_award(&self, stored: PlayerProgress, xp: u32) -> Result<Award> {
        self.player.award(self.player_total(stored)?, xp)
    }
}

pub struct Curve {
    thresholds: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    /// Displayed level; native clientState / weapon-table index is level - 1.
    pub level: u8,
    pub total_xp: u32,
    pub xp_within_level: u32,
    /// None at max level, so UI callers cannot invent a level beyond the table.
    pub xp_to_next: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Award {
    pub before: Progress,
    pub after: Progress,
    pub credited_xp: u32,
    /// Every crossed displayed level, including multi-level awards.
    pub levels_gained: Vec<u8>,
}

impl Curve {
    pub fn new(thresholds: Vec<u32>) -> Result<Self> {
        ensure!(
            (1..=255).contains(&thresholds.len())
                && thresholds.first() == Some(&0)
                && thresholds.last().is_some_and(|v| *v <= MAX_XP)
                && thresholds.windows(2).all(|pair| pair[0] < pair[1]),
            "invalid cumulative progression thresholds"
        );
        Ok(Self { thresholds })
    }

    pub fn progress(&self, xp: u32) -> Result<Progress> {
        ensure!(xp <= MAX_XP, "XP exceeds native signed lookup range");
        let index = self.thresholds.partition_point(|&need| need <= xp) - 1;
        Ok(Progress {
            level: (index + 1) as u8,
            total_xp: xp,
            xp_within_level: xp - self.thresholds[index],
            xp_to_next: self.thresholds.get(index + 1).map(|next| next - xp),
        })
    }

    pub fn total(&self, level: u8, within: u32) -> Result<u32> {
        let index = usize::from(level).checked_sub(1);
        let base = index.and_then(|i| self.thresholds.get(i));
        ensure!(base.is_some(), "stored level outside progression curve");
        let total = base.unwrap().checked_add(within);
        ensure!(total.is_some_and(|v| v <= MAX_XP), "stored XP overflow");
        let total = total.unwrap();
        ensure!(
            self.progress(total)?.level == level,
            "stored within-level XP crosses the next threshold"
        );
        Ok(total)
    }

    pub fn award(&self, total: u32, amount: u32) -> Result<Award> {
        let before = self.progress(total)?;
        let after = self.progress(total.saturating_add(amount).min(MAX_XP))?;
        let levels_gained = (u16::from(before.level) + 1..=u16::from(after.level))
            .map(|level| level as u8)
            .collect();
        Ok(Award {
            before,
            after,
            credited_xp: after.total_xp - before.total_xp,
            levels_gained,
        })
    }
}

/// A separately verified attachment requirement. Readiness does not imply
/// ownership: crossing a level only makes an item eligible, never grants it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlockRequirement {
    pub item_id: u32,
    pub displayed_level: u8,
}

impl Award {
    /// Candidate reminders; persistence and delivery deduplication belong to the
    /// eventual authoritative match transaction, not a transient HUD queue.
    pub fn crossed_unlocks(&self, requirements: &[UnlockRequirement]) -> Vec<u32> {
        let mut items = Vec::new();
        for rule in requirements {
            if rule.item_id != 0
                && self.before.level < rule.displayed_level
                && rule.displayed_level <= self.after.level
                && !requirements.iter().any(|earlier| {
                    earlier.item_id == rule.item_id && earlier.displayed_level <= self.before.level
                })
                && !items.contains(&rule.item_id)
            {
                items.push(rule.item_id);
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> Tables {
        Tables::from_json(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/progression-3.3.3.3.json"
        )))
        .unwrap()
    }

    #[test]
    fn native_curves_round_trip_every_boundary_and_never_show_an_extra_level() {
        let t = tables();
        for (curve, expected_max) in [(&t.player, 70), (&t.primary, 31), (&t.secondary, 20)] {
            for (index, &threshold) in curve.thresholds.iter().enumerate() {
                let at = curve.progress(threshold).unwrap();
                assert_eq!(usize::from(at.level), index + 1);
                assert_eq!(at.xp_within_level, 0);
                assert_eq!(curve.total(at.level, 0).unwrap(), threshold);
                if threshold > 0 {
                    let below = curve.progress(threshold - 1).unwrap();
                    assert_eq!(usize::from(below.level), index);
                    assert_eq!(below.xp_to_next, Some(1));
                    assert_eq!(
                        curve.total(below.level, below.xp_within_level).unwrap(),
                        threshold - 1
                    );
                }
            }
            let capped = curve.progress(MAX_XP).unwrap();
            assert_eq!(capped.level, expected_max);
            assert_eq!(capped.xp_to_next, None);
        }
        assert_eq!(t.player.progress(3_582_550).unwrap().level, 70);
        assert_eq!(t.primary.progress(121_230).unwrap().level, 31);
        assert_eq!(t.secondary.progress(71_200).unwrap().level, 20);
    }

    #[test]
    fn rejects_wrong_versions_noncanonical_stored_xp_and_overflow() {
        let t = tables();
        assert!(t.player.total(0, 0).is_err());
        assert!(t.player.total(71, 0).is_err());
        assert!(t.player.total(1, 2000).is_err());
        assert!(t.player.total(70, MAX_XP).is_err());
        assert!(t.primary.progress(u32::MAX).is_err());
        for thresholds in [
            vec![],
            vec![1],
            vec![0, 500, 500],
            vec![0, 500, 100],
            vec![0, u32::MAX],
        ] {
            assert!(Curve::new(thresholds).is_err());
        }
        for version in [2, 3, 5] {
            assert!(
                t.player_total(PlayerProgress {
                    rank: 1,
                    experience: 0,
                    rank_version: version
                })
                .is_err()
            );
        }
        let mut file: serde_json::Value = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/progression-3.3.3.3.json"
        )))
        .unwrap();
        for (key, value) in [
            ("rank_version", serde_json::json!(2)),
            ("client_sha256", serde_json::json!("wrong")),
            ("primary", serde_json::json!([0])),
        ] {
            let original = file[key].clone();
            file[key] = value;
            assert!(Tables::from_json(&serde_json::to_vec(&file).unwrap()).is_err());
            file[key] = original;
        }
    }

    #[test]
    fn awards_include_all_crossed_levels_and_only_new_unlock_candidates() {
        let t = tables();
        let award = t.primary.award(499, 1601).unwrap();
        assert_eq!(award.before.level, 1);
        assert_eq!(award.after.level, 3);
        assert_eq!(award.levels_gained, [2, 3]);
        let rules = [
            UnlockRequirement {
                item_id: 10,
                displayed_level: 1,
            },
            UnlockRequirement {
                item_id: 10,
                displayed_level: 2,
            },
            UnlockRequirement {
                item_id: 11,
                displayed_level: 2,
            },
            UnlockRequirement {
                item_id: 12,
                displayed_level: 3,
            },
            UnlockRequirement {
                item_id: 11,
                displayed_level: 3,
            },
            UnlockRequirement {
                item_id: 13,
                displayed_level: 4,
            },
        ];
        assert_eq!(award.crossed_unlocks(&rules), [11, 12]);
        assert!(
            t.primary
                .award(2100, 0)
                .unwrap()
                .crossed_unlocks(&rules)
                .is_empty()
        );
        let max = t.player.award(MAX_XP - 5, u32::MAX).unwrap();
        assert_eq!(max.credited_xp, 5);
        assert!(max.levels_gained.is_empty());
    }

    #[test]
    fn player_award_can_prepare_an_owner_checked_profile_without_touching_disk() {
        let t = tables();
        let mut blank = vec![0; codol_protocol::lobby::profile::SNAPSHOT_BYTES];
        let crc = crc32fast::hash(&blank[4..]);
        blank[..4].copy_from_slice(&crc.to_le_bytes());
        let old = codol_protocol::lobby::profile::prepare_private(&blank, 123, true).unwrap();
        let stored = codol_protocol::lobby::profile::player_progress(&old, 123).unwrap();
        let award = t.plan_player_award(stored, 2001).unwrap();
        let prepared = codol_protocol::lobby::profile::with_player_progress(
            &old,
            123,
            award.after.level,
            award.after.xp_within_level,
        )
        .unwrap();
        let stored = codol_protocol::lobby::profile::player_progress(&prepared, 123).unwrap();
        assert_eq!(
            stored,
            PlayerProgress {
                rank: 2,
                experience: 1,
                rank_version: 4
            }
        );
        assert_eq!(t.player_total(stored).unwrap(), 2001);
        assert!(codol_protocol::lobby::profile::with_player_progress(&old, 124, 2, 1).is_err());
    }
}
