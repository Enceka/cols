use super::*;

impl PreparedGameState {
    pub(crate) fn human_profile(
        &self,
        team: u8,
        ordinal: usize,
        public: &[u8],
        owned: &[u32],
    ) -> Result<Self> {
        ensure!(matches!(team, 1 | 2), "invalid human team");
        let mut state = self.clone();
        let config = state
            .match_config
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("human match requires TDM map"))?;
        let spawns = &config.spawns[usize::from(team - 1)];
        state.spawn_origin = Some(crate::game::ai::controller::grounded_spawn(
            spawns[ordinal % spawns.len()],
            self.collision.as_ref().unwrap(),
        )?);
        if let Some(angles) = config.spawn_angles.as_mut() {
            let selected =
                angles[usize::from(team - 1)][ordinal % angles[usize::from(team - 1)].len()];
            angles[usize::from(team - 1)][0] = selected;
        }
        if config.teams[0] != team {
            for team in &mut config.teams {
                *team = 3 - *team;
            }
        }
        state.player_appearance = None;
        state.appearance_first_model = Some(500 + ordinal as u16 * 32);
        let selected = state.for_initial_backpack(public)?;
        selected.for_profile_at(public, owned, self.first_weapon_index())
    }
    pub(crate) fn shared_bot_profile(&self, team: u8, client: u8, ordinal: usize) -> Result<Self> {
        ensure!(
            matches!(team, 1 | 2) && (1..=11).contains(&client),
            "invalid shared Bot"
        );
        let mut state = self.clone();
        let config = state
            .match_config
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Bot requires TDM map"))?;
        let entry = config
            .roster
            .get(usize::from(client - 1))
            .ok_or_else(|| anyhow::anyhow!("Bot roster missing"))?
            .clone();
        let spawns = &config.spawns[usize::from(team - 1)];
        state.spawn_origin = Some(crate::game::ai::controller::grounded_spawn(
            spawns[ordinal % spawns.len()],
            self.collision.as_ref().unwrap(),
        )?);
        if config.teams[0] != team {
            for t in &mut config.teams {
                *t = 3 - *t;
            }
        }
        let arsenal = self
            .arsenal
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Bot arsenal missing"))?;
        let hands = self
            .bootstrap_weapon
            .as_ref()
            .and_then(|w| w.viewmodel_index);
        state.bootstrap_weapon = Some(Self::resolved_weapon(
            arsenal.bot_weapon(&entry.weapon_name, self.first_weapon_index())?,
            &arsenal.source_sha256,
            hands,
        )?);
        state.secondary_weapon = None;
        state.player_appearance = None;
        state.player_loadout = None;
        state.training_enemy = Some(entry.animation.clone());
        state.shared_bot_outfit = Some(entry.outfit(team));
        Ok(state)
    }
    pub(crate) fn shared_bot_controller(
        &self,
    ) -> Result<crate::game::ai::controller::BotController> {
        crate::game::ai::controller::BotController::new(
            self.spawn_origin.unwrap(),
            self.spawn_angles(),
            self.training_enemy.as_ref().unwrap().move_speed,
        )
    }
    pub(crate) fn shared_navigation(&self) -> Option<Arc<crate::game::ai::navigation::Navigation>> {
        self.prepared_navigation().ok()
    }
    pub(crate) fn include_human_models(&mut self, models: &[(u16, String)]) -> Result<()> {
        let own = self
            .player_appearance
            .as_ref()
            .map(|a| a.registrations.clone())
            .unwrap_or_default();
        let config = self
            .match_config
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("human map missing roster"))?;
        for (slot, name) in models {
            if own.iter().any(|(id, n)| 2051 + id == *slot && n == name) {
                continue;
            }
            if let Some((_, old)) = config.models.iter().find(|(id, _)| 2051 + id == *slot) {
                ensure!(old == name, "human model registration conflict");
            } else {
                config.models.push((slot - 2051, name.clone()));
            }
        }
        Ok(())
    }
    pub(crate) fn human_character(
        &self,
    ) -> Option<codol_protocol::game::snapshot::character::Character> {
        self.shared_bot_outfit
            .and_then(|o| o.character)
            .or_else(|| self.player_appearance.as_ref().and_then(|a| a.character))
    }
    pub(crate) fn human_limits(&self) -> (u32, u32) {
        self.match_config
            .as_ref()
            .map_or((7500, 600000), |c| (c.score_limit, c.time_limit_msec))
    }
    pub(crate) fn human_snapshot(
        &self,
        view: &crate::game::session::shared::View,
        view_offset: [u16; 3],
        previous: Option<&crate::game::session::shared::View>,
        delta_distance: u8,
        held_weapons_in_baseline: bool,
    ) -> Result<Vec<u8>> {
        Ok(codol_protocol::game::snapshot::player_frame_with_world(
            view.time,
            view.command_time,
            Some(view.motion.origin),
            previous.is_none(),
            Some(view.equipment),
            view.angles,
            codol_protocol::game::snapshot::WorldSnapshot {
                motion: Some(view.motion.wire_snapshot()),
                vitals: Some(view.vitals),
                local_player: Some(view.local),
                players: &view.players,
                local_player_baseline: previous.map(|v| v.local).filter(|p| p.alive),
                player_baselines: previous.map_or(&[], |v| v.players.as_slice()),
                corpse_baselines: previous
                    .filter(|_| held_weapons_in_baseline)
                    .map_or(&[], |v| v.corpses.as_slice()),
                impact_baselines: previous
                    .filter(|_| held_weapons_in_baseline)
                    .map_or(&[], |v| v.impacts.as_slice()),
                obituary_baselines: previous
                    .filter(|_| held_weapons_in_baseline)
                    .map_or(&[], |v| v.obituaries.as_slice()),
                missile_baselines: previous.map_or(&[], |v| v.missiles.as_slice()),
                smoke_baselines: previous.map_or(&[], |v| v.smokes.as_slice()),
                delta_distance,
                held_weapons_in_baseline,
                characters: &view.characters,
                held_weapon_tag: Some(codol_protocol::game::snapshot::held_weapon::HAND_TAG_SLOT),
                local_card: self.player_card,
                view_offset,
                impacts: &view.impacts,
                obituaries: &view.obituaries,
                corpses: &view.corpses,
                hit_feedback: view.feedback,
                missiles: &view.missiles,
                smokes: &view.smokes,
                shellshock: view.shellshock,
                destroyed_glass: &self.collision.as_ref().unwrap().destroyed_glass(),
                ..Default::default()
            },
        )?)
    }
}

#[cfg(test)]
pub(crate) fn test_state(
    origin: [f32; 3],
    collision: Arc<crate::game::world::collision::Collision>,
) -> PreparedGameState {
    let mut state = super::synthetic();
    state.spawn_origin = Some(origin);
    state.collision = Some(collision);
    state.bootstrap_weapon=Some(serde_json::from_value(serde_json::json!({
        "name":"test","index":352,"ammo":{"clip_key":352,"reserve_key":352,"clip":30,"reserve":90,"source_sha256":"a".repeat(64),"fire_interval_msec":79},
        "ballistics":{"damage":34,"range":8192.0,"near_range":8192.0}
    })).unwrap());
    state.training_enemy=Some(serde_json::from_value(serde_json::json!({
        "model":4,"origin":[0,0,0],"bounds_min":[-15,-15,0],"bounds_max":[15,15,70],
        "idle_animation":395,"walk_animation":243,"fire_animation":616,"death_animation":47,"move_speed":94.0,"angles":[0,0,0],"weapon":352,"source_sha256":"a".repeat(64)
    })).unwrap());
    state
}
