//! Story world preparation: the first level resources plus its mission.
//! A story world is never a TDM world; it carries no match configuration and
//! uses the grounded first spawn instead of a team formation.
use super::*;
use crate::game::modes::story::Mission;

impl PreparedGameState {
    pub(crate) fn load_story_world(path: &Path, mission_path: &Path) -> Result<Self> {
        let mut state = Self::load(path)?;
        let mut mission = Mission::load(mission_path)?;
        ensure!(
            state.map == mission.map && state.mode == mission.mode && state.match_config.is_none(),
            "story world preparation differs from its mission"
        );
        let spawn = state
            .spawn_origin
            .ok_or_else(|| anyhow::anyhow!("story world needs a start origin"))?;
        mission.spawn.origin = spawn;
        state.story_mission = Some(mission);
        state.validate()?;
        Ok(state)
    }
}
