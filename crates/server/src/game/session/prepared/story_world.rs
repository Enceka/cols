//! Story world preparation: the first level resources plus its mission.
//! A story world is never a TDM world; it carries no match configuration and
//! uses the grounded first spawn instead of a team formation.
use super::*;
use crate::game::modes::story::Mission;

impl PreparedGameState {
    pub(crate) fn load_story_world(path: &Path, mission_path: &Path) -> Result<Self> {
        let state = Self::read(path)?;
        let mission = Mission::load(mission_path)?;
        ensure!(
            state.map == mission.map && state.mode == mission.mode && state.match_config.is_none(),
            "story world preparation differs from its mission"
        );
        ensure!(
            state.spawn_origin.is_some(),
            "story world needs a start origin"
        );
        // The mission has to be attached before the resources load: a story
        // level reads its geometry and weapons from the base map captures.
        let mut state = state;
        state.story_mission = Some(mission);
        let mut state = Self::attach(path, state, false)?;
        let spawn = state
            .spawn_origin
            .ok_or_else(|| anyhow::anyhow!("story world needs a start origin"))?;
        if let Some(mission) = &mut state.story_mission {
            mission.spawn.origin = spawn;
        }
        state.validate()?;
        Ok(state)
    }
}
