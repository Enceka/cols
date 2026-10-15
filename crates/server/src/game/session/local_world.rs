//! Authority for one local gameplay world, separate from connection replication.
//! The current transport owns one instance; sharing it across human connections
//! requires a match registry and one simulation clock, not cloning this value.
use super::prepared::PreparedGameState;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct LocalWorld {
    pub(crate) weapon_drops: crate::game::world::weapon_drops::WeaponDrops,
    pub(crate) game_state: Option<Arc<crate::game::session::prepared::PreparedGameState>>,
    pub(crate) player: Option<crate::game::weapons::held::HeldWeapons>,
    pub(crate) grenades: crate::game::weapons::grenade::Grenades,
    pub(crate) smoke: crate::game::weapons::smoke::Smoke,
    pub(crate) scavenger: crate::game::rewards::scavenger::Bags,
    pub(crate) tactical: crate::game::status::tactical::State,
    pub(crate) gas: crate::game::status::gas::Gas,
    pub(crate) poison: crate::game::status::gas::Poison,
    pub(crate) emp: crate::game::status::emp::State,
    pub(crate) motion: Option<crate::game::player::movement::Motion>,
    pub(crate) combat: Option<crate::game::combat::CombatWorld>,
    pub(crate) bot: Option<crate::game::ai::controller::BotController>,
    pub(crate) tdm: Option<crate::game::modes::tdm::TdmMatch>,
    pub(crate) bot_frame: Option<crate::game::ai::controller::BotFrame>,
    pub(crate) bot_fire_time: Option<u32>,
    pub(crate) bot_fire_toggle: bool,
    pub(crate) vitals: codol_protocol::game::snapshot::PlayerVitals,
    pub(crate) player_birth: u32,
    pub(crate) player_death: Option<u32>,
    pub(crate) health_recovery: crate::game::player::health::HealthRecovery,
}
impl LocalWorld {
    pub(crate) fn from_prepared(state: Arc<PreparedGameState>) -> Self {
        let state = if state.collision.is_some() {
            Arc::new(state.for_local_world())
        } else {
            state
        };
        Self {
            player: state.player(),
            motion: state.motion(),
            combat: state.combat(),
            bot: state.bot(),
            tdm: state.tdm_match().expect("validated TDM state"),
            player_birth: 1000,
            game_state: Some(state),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::world::collision::Collision;

    #[test]
    fn constructing_worlds_directly_isolates_destruction_and_resets_new_matches() {
        let export = serde_json::json!({
            "schema_version": 1, "read_only": true,
            "client_sha256": codol_protocol::CLIENT_SHA256,
            "map": "maps/mp/mp_afghan.d3dbsp", "ncs_crc": 0x12345678,
            "brushes": [{"center": [50, 0, 45], "half": [1, 100, 45],
                "contents": 0x0800_0010, "planes": [], "glass_piece": 4}],
            "triangles": []
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let mut prepared = super::super::prepared::synthetic();
        prepared.collision = Some(Arc::new(Collision::load(file.path()).unwrap()));
        let template = Arc::new(prepared);
        let first = LocalWorld::from_prepared(template.clone());
        let second = LocalWorld::from_prepared(template.clone());
        let first_state = first.game_state.as_ref().unwrap();
        let first_collision = first_state.collision.as_ref().unwrap();
        assert!(first_collision.shatter_glass(4));
        assert_eq!(first_collision.destroyed_glass(), [4]);
        let next_match = LocalWorld::from_prepared(first_state.clone());
        for state in [
            &template,
            second.game_state.as_ref().unwrap(),
            next_match.game_state.as_ref().unwrap(),
        ] {
            let collision = state.collision.as_ref().unwrap();
            assert!(collision.destroyed_glass().is_empty());
            assert_eq!(
                collision
                    .raycast([0.0, 0.0, 40.0], [100.0, 0.0, 40.0])
                    .glass_piece,
                Some(4)
            );
        }
    }
}
