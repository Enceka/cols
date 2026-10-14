use crate::game::ai::loadout::BotLoadout;
use crate::game::world::collision::Collision;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatchConfig {
    #[serde(default = "default_player_count")]
    pub player_count: u8,
    /// Axis and allies starts from this map's captured entity string.
    pub spawns: [Vec<[f32; 3]>; 2],
    #[serde(default)]
    pub spawn_angles: Option<[Vec<[u16; 3]>; 2]>,
    pub score_limit: u32,
    pub time_limit_msec: u32,
    #[serde(default = "default_teams")]
    pub teams: [u8; 12],
    #[serde(default)]
    pub roster: Vec<BotLoadout>,
    #[serde(default)]
    pub models: Vec<(u16, String)>,
}
fn default_player_count() -> u8 {
    12
}
pub(crate) fn default_teams() -> [u8; 12] {
    [2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1]
}

impl MatchConfig {
    pub fn validate(&self, collision: &Collision) -> Result<()> {
        ensure!(
            self.spawn_angles.as_ref().is_none_or(|angles| angles
                .iter()
                .zip(&self.spawns)
                .all(|(a, p)| a.len() == p.len())),
            "TDM spawn headings must match every captured start"
        );
        ensure!(
            matches!(self.player_count, 4 | 12)
                && self.teams[..usize::from(self.player_count)]
                    .iter()
                    .all(|t| matches!(t, 1 | 2))
                && self.teams[..usize::from(self.player_count)]
                    .iter()
                    .filter(|&&t| t == 1)
                    .count()
                    == usize::from(self.player_count / 2)
                && self.teams[usize::from(self.player_count)..]
                    .iter()
                    .all(|&t| t == 0),
            "TDM requires a balanced active roster with empty unused slots"
        );
        ensure!(
            (100..=100_000).contains(&self.score_limit) && self.score_limit.is_multiple_of(100),
            "invalid TDM score limit"
        );
        ensure!(
            (60_000..=3_600_000).contains(&self.time_limit_msec),
            "invalid TDM duration"
        );
        ensure!(
            self.roster.is_empty() || self.roster.len() == usize::from(self.player_count - 1),
            "TDM roster must match the configured player count"
        );
        for entry in &self.roster {
            entry.animation.validate()?;
            for (faction, outfit) in entry.outfits.iter().flatten().enumerate() {
                if let Some(character) = outfit.character {
                    character.fields()?;
                    ensure!(
                        character.faction == 1 - faction as u8,
                        "Bot character faction mismatch"
                    );
                }
                ensure!(
                    (1..1024).contains(&outfit.model)
                        && outfit.attachment_models.iter().all(|&i| i < 1024),
                    "invalid faction-specific Bot outfit"
                );
                let mut ended = false;
                for model in outfit.attachment_models {
                    ensure!(!ended || model == 0, "sparse Bot outfit attachments");
                    ended |= model == 0;
                }
            }
            ensure!(
                entry.class.starts_with("botclass") && entry.class.len() <= 32,
                "invalid native Bot class"
            );
            ensure!(
                matches!(
                    entry.personality.as_str(),
                    "default" | "camper" | "run_and_gun"
                ),
                "invalid bot personality"
            );
            ensure!(
                entry.weapon_name.ends_with("_mp")
                    && entry
                        .weapon_name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "invalid bot weapon identity"
            );
            ensure!(
                entry.attachment_models.iter().all(|&i| i < 1024),
                "invalid bot part index"
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        for (id, name) in &self.models {
            ensure!(
                (1..1024).contains(id)
                    && ids.insert(*id)
                    && !name.is_empty()
                    && name.len() < 64
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "invalid bot model registration"
            );
        }
        for (team_index, team) in self.spawns.iter().enumerate() {
            ensure!(
                (usize::from(self.player_count / 2)..=128).contains(&team.len()),
                "TDM needs a captured start for every active player"
            );
            for (index, &spawn) in team.iter().enumerate() {
                crate::game::ai::controller::grounded_spawn(spawn, collision).with_context(
                    || {
                        format!(
                            "invalid TDM spawn: team={} index={index} origin={spawn:?}",
                            team_index + 1
                        )
                    },
                )?;
            }
        }
        Ok(())
    }
}
