use crate::game::ai::loadout::BotLoadout;
use crate::game::world::collision::Collision;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatchConfig {
    /// Bound from the prepared state's native gametype, never a JSON override.
    #[serde(skip)]
    pub mode: crate::content::maps::team_modes::TeamMode,
    #[serde(default = "default_preparation")]
    pub preparation_msec: u32,
    #[serde(default)]
    pub preparation_text: Option<u16>,
    #[serde(default = "default_grace_period")]
    pub grace_period_msec: u32,
    /// Shared mp_tdm_spawn pool, distinct from the two opening formations.
    #[serde(default)]
    pub respawn_points: Vec<SpawnPoint>,
    /// mp_cha_spawn_axis/allies, distinct from optional opening _start groups.
    #[serde(default)]
    pub base_respawns: Option<[Vec<SpawnPoint>; 2]>,
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
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpawnPoint {
    pub origin: [f32; 3],
    #[serde(default)]
    pub angles: [u16; 3],
}
fn default_preparation() -> u32 {
    5000
}
fn default_grace_period() -> u32 {
    15000
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
            self.preparation_msec <= 30_000,
            "invalid match preparation duration"
        );
        ensure!(
            self.grace_period_msec <= 60_000,
            "invalid match grace duration"
        );
        ensure!(
            self.preparation_text.is_none_or(|t| (1..1023).contains(&t)),
            "invalid preparation text slot"
        );
        ensure!(
            self.respawn_points.len() <= 1024,
            "too many shared respawn points"
        );
        for point in &self.respawn_points {
            ensure!(
                point
                    .origin
                    .iter()
                    .all(|v| v.is_finite() && (-32768.0..=32767.0).contains(v)),
                "shared respawn position outside native coordinate bounds"
            );
            crate::game::ai::controller::grounded_spawn(point.origin, collision)
                .with_context(|| format!("invalid shared respawn point {:?}", point.origin))?;
        }
        if let Some(bases) = &self.base_respawns {
            for base in bases {
                ensure!(
                    (1..=128).contains(&base.len()),
                    "invalid fixed-base respawn pool"
                );
                for point in base {
                    ensure!(
                        point
                            .origin
                            .iter()
                            .all(|v| v.is_finite() && (-32768.0..=32767.0).contains(v)),
                        "fixed-base spawn outside native coordinates"
                    );
                    crate::game::ai::controller::grounded_spawn(point.origin, collision)?;
                }
            }
        }
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
            for loadout in entry.room_loadouts.iter().flatten() {
                loadout.validate()?;
            }
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
