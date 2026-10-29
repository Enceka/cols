//! First-level cooperative mission definition.
//! Stages come from authored preparation data for this map, so the runtime never
//! invents objectives that the preparation did not carry.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

const MAX_FILE_BYTES: u64 = 65536;
const MAX_STAGES: usize = 64;
const MIN_RADIUS: f32 = 32.0;
const MAX_RADIUS: f32 = 4096.0;
const MAX_COORDINATE: f32 = 16384.0;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mission {
    pub(crate) schema_version: u32,
    pub(crate) client_sha256: String,
    pub(crate) map: String,
    pub(crate) mode: String,
    pub(crate) spawn: Spawn,
    pub(crate) stages: Vec<Stage>,
    pub(crate) completion: Completion,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Spawn {
    pub(crate) origin: [f32; 3],
    #[serde(default)]
    pub(crate) angles: [u16; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Stage {
    pub(crate) id: String,
    /// The next attempt restarts here instead of at the opening spawn.
    #[serde(default)]
    pub(crate) checkpoint: bool,
    pub(crate) objective: Objective,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Objective {
    Reach { origin: [f32; 3], radius: f32 },
    Eliminate { count: u16 },
    Survive { seconds: u16 },
    Finish { origin: [f32; 3], radius: f32 },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Completion {
    pub(crate) unlock_flag: u16,
    pub(crate) native_flag: String,
}

impl Mission {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= MAX_FILE_BYTES,
            "story mission file too large"
        );
        let raw = std::fs::read(path)?;
        let mission: Self =
            serde_json::from_slice(&raw).context("invalid story mission definition")?;
        mission.validate()?;
        Ok(mission)
    }

    /// Elimination stages need an enemy with health the build can simulate.
    pub(crate) fn needs_enemy(&self) -> bool {
        self.stages
            .iter()
            .any(|stage| matches!(stage.objective, Objective::Eliminate { .. }))
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.client_sha256 == codol_protocol::CLIENT_SHA256
                && self.map == codol_protocol::room::story::FIRST_MAP
                && self.mode == "coop",
            "story mission identity mismatch"
        );
        ensure!(
            self.completion.unlock_flag == codol_protocol::room::story::FIRST_COMPLETION_FLAG
                && self.completion.native_flag
                    == codol_protocol::room::story::FIRST_COMPLETION_NAME,
            "story mission completion flag is not the first level marker"
        );
        coordinate(self.spawn.origin)?;
        ensure!(
            (1..=MAX_STAGES).contains(&self.stages.len()),
            "story mission stage count out of range"
        );
        let mut ids = std::collections::HashSet::new();
        let mut finishes = 0;
        for (index, stage) in self.stages.iter().enumerate() {
            ensure!(
                valid_id(&stage.id) && ids.insert(&stage.id),
                "story mission stage ids must be canonical and unique"
            );
            match stage.objective {
                Objective::Reach { origin, radius } | Objective::Finish { origin, radius } => {
                    coordinate(origin)?;
                    ensure!(
                        radius.is_finite() && (MIN_RADIUS..=MAX_RADIUS).contains(&radius),
                        "story mission objective radius out of range"
                    );
                }
                Objective::Eliminate { count } => ensure!(
                    (1..=64).contains(&count),
                    "story mission elimination count out of range"
                ),
                Objective::Survive { seconds } => ensure!(
                    (1..=3600).contains(&seconds),
                    "story mission survival budget out of range"
                ),
            }
            if matches!(stage.objective, Objective::Finish { .. }) {
                finishes += 1;
                ensure!(
                    index + 1 == self.stages.len(),
                    "story mission must finish on its last stage"
                );
            }
        }
        ensure!(
            finishes == 1,
            "story mission needs exactly one finish stage"
        );
        Ok(())
    }
}

fn coordinate(origin: [f32; 3]) -> Result<()> {
    ensure!(
        origin.iter().all(|v| v.is_finite())
            && origin[0].abs() <= MAX_COORDINATE
            && origin[1].abs() <= MAX_COORDINATE
            && origin[2].abs() <= MAX_COORDINATE,
        "story mission coordinate out of range"
    );
    Ok(())
}

fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
