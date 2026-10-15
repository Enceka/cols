use super::*;

impl PreparedGameState {
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= 256 * 1024,
            "prepared game state size"
        );
        let mut state: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        let directory = path.parent().unwrap_or(Path::new("."));
        if let Some(file) = &state.props_file {
            state.props_resources = Some(Arc::new(crate::game::world::props::Resources::load(
                &directory.join(file),
                &state.map,
                state.ncs_crc,
            )?));
        }
        if let Some(file) = &state.radar_file {
            state.radar_resources = Some(Arc::new(
                crate::game::rewards::killstreaks::RadarResources::load(
                    &directory.join(file),
                    &state.map,
                    state.ncs_crc,
                )?,
            ));
        }
        if let Some(file) = &state.nuke_file {
            state.nuke_resources = Some(Arc::new(
                crate::game::rewards::killstreaks::NukeResources::load(
                    &directory.join(file),
                    &state.map,
                    state.ncs_crc,
                )?,
            ));
        }
        if let Some(file) = &state.voice_file {
            state.voice_resources =
                Some(Arc::new(crate::game::presentation::voice::Resources::load(
                    &directory.join(file),
                    &state.map,
                    &state.mode,
                    state.ncs_crc,
                )?));
        }
        if let Some(file) = &state.arsenal_file {
            state.arsenal = Some(Arc::new(crate::content::weapons::arsenal::Arsenal::load(
                directory.join(file),
                state.ncs_crc,
            )?));
            if let Some(map) = state
                .arsenal
                .as_ref()
                .and_then(|arsenal| arsenal.captured_map.as_ref())
            {
                ensure!(
                    map == &format!("maps/mp/{}.d3dbsp", state.map),
                    "arsenal belongs to another map"
                );
            }
        }
        if let Some(file) = &state.collision_file {
            let collision = crate::game::world::collision::Collision::load(&directory.join(file))?;
            ensure!(
                collision.ncs_crc == state.ncs_crc
                    && collision.map == format!("maps/mp/{}.d3dbsp", state.map),
                "collision identity mismatch"
            );
            state.collision = Some(Arc::new(collision));
        }
        if let Some(file) = &state.appearance_file {
            ensure!(
                state
                    .appearance_first_model
                    .is_some_and(|i| (1..1024).contains(&i)),
                "appearance model range missing"
            );
            state.appearance_catalog = Some(Arc::new(
                crate::content::characters::appearance::Catalog::load(&directory.join(file))?,
            ));
        }
        state.validate()?;
        state.ground_first_spawn()?;
        if let Some(config) = &state.match_config {
            state.navigation = Some(Arc::new(crate::game::ai::navigation::Navigation::build(
                config
                    .spawns
                    .iter()
                    .flatten()
                    .copied()
                    .chain(config.respawn_points.iter().map(|p| p.origin))
                    .chain(
                        config
                            .base_respawns
                            .iter()
                            .flatten()
                            .flatten()
                            .map(|p| p.origin),
                    ),
                state.collision.as_ref().unwrap(),
            )));
        }
        Ok(state)
    }
    /// Raw spawn entities can sit a few units inside terrain (Crash Axis
    /// starts are 4 units low); the first life must use the same grounded
    /// start as respawns, or every player sweep begins solid.
    pub(super) fn ground_first_spawn(&mut self) -> Result<()> {
        let (Some(origin), Some(collision)) = (self.spawn_origin, &self.collision) else {
            return Ok(());
        };
        let start = match &self.match_config {
            Some(config) => config.spawns[usize::from(config.teams[0] - 1)][0],
            None => origin,
        };
        let height = crate::game::player::movement::Stance::Stand.height();
        if self.match_config.is_some() || collision.trace(start, start, height).start_solid {
            self.spawn_origin = Some(crate::game::ai::controller::grounded_spawn(
                start, collision,
            )?);
        }
        Ok(())
    }
    pub(super) fn validate(&self) -> Result<()> {
        let state = self;
        ensure!(
            state.training_target.is_none() || state.training_enemy.is_none(),
            "choose one training target kind"
        );
        if let Some(enemy) = &state.training_enemy {
            enemy.validate()?;
            ensure!(
                state.collision.is_some() && state.arsenal.is_some(),
                "training enemy requires real collision and weapons"
            );
            if enemy.bot {
                crate::game::ai::controller::grounded_spawn(
                    enemy.origin,
                    state.collision.as_ref().unwrap(),
                )?;
            }
        }
        if let Some(config) = &state.match_config {
            ensure!(
                crate::content::maps::registry::maximum_players(state.map())
                    .is_some_and(|max| config.player_count <= max),
                "match exceeds native map capacity"
            );
            ensure!(
                state.training_enemy.is_some() && state.arsenal.is_some(),
                "TDM requires human and weapon assets"
            );
            config.validate(
                state
                    .collision
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("TDM requires map collision"))?,
            )?;
        }
        for weapon in state
            .bootstrap_weapon
            .iter()
            .chain(&state.secondary_weapon)
            .chain(&state.match_weapon)
            .chain(&state.roster_weapons)
        {
            ensure!(
                weapon.ballistics.is_none_or(|b| b.valid()),
                "invalid weapon ballistics"
            );
            ensure!(
                state.spawn_origin.is_some()
                    && !weapon.name.is_empty()
                    && weapon.name.len() <= 64
                    && (1..900).contains(&weapon.index),
                "invalid bootstrap weapon or missing spawn"
            );
            ensure!(
                weapon.switch.is_none_or(|t| t.valid())
                    && weapon.sprint_timing.is_none_or(|t| t.valid())
                    && weapon.spread.is_none_or(|s| s.valid())
                    && weapon
                        .move_scale
                        .is_none_or(|s| s.is_finite() && s > 0.0 && s <= 4.0)
                    && weapon
                        .ads_move_scale
                        .is_none_or(|s| s.is_finite() && (0.0..=4.0).contains(&s))
                    && weapon
                        .sprint_duration_scale
                        .is_none_or(|s| { s.is_finite() && (0.0..=16.0).contains(&s) }),
                "invalid weapon switching/movement parameters"
            );
            ensure!(
                weapon.fire_type.is_none_or(|v| v <= 5),
                "unsupported special firing state machine"
            );
            ensure!(
                weapon.melee.is_none_or(|m| m.valid()),
                "invalid melee timing"
            );
            ensure!(
                weapon.viewmodel_index.is_none_or(|i| i > 0 && i < 1024),
                "invalid viewmodel index"
            );
            ensure!(
                weapon.base_index.is_some() == weapon.composed_name.is_some(),
                "incomplete composed weapon identity"
            );
            ensure!(
                weapon.camo.is_none() || weapon.base_index.is_some(),
                "camo requires composed weapon"
            );
            if let Some(base) = weapon.base_index {
                ensure!(
                    base > 0
                        && base < weapon.index
                        && weapon.composed_name.as_deref()
                            == Some(&codol_protocol::game::gamestate::composed_weapon(
                                base,
                                weapon.camo.unwrap_or(3),
                                &weapon.attachments
                            )?),
                    "unsupported bootstrap weapon composition"
                );
            }
            ensure!(
                weapon.attachments.is_empty() == weapon.attachment_source_sha256.is_none()
                    && (weapon.attachments.is_empty() || weapon.base_index.is_some())
                    && weapon.attachment_source_sha256.as_ref().is_none_or(
                        |hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                    ),
                "missing attachment evidence or composition"
            );
            if let Some(ammo) = &weapon.ammo {
                ensure!(
                    ammo.clip_key <= 1022
                        && ammo.reserve_key <= 1023
                        && ammo.clip <= 511
                        && ammo.reserve <= 2047
                        && ammo
                            .fire_interval_msec
                            .is_none_or(|n| (1..=2000).contains(&n))
                        && ammo
                            .reload
                            .is_none_or(|r| r.valid() && ammo.fire_interval_msec.is_some())
                        && ammo.source_sha256.len() == 64
                        && ammo.ads.is_none_or(|a| a.valid())
                        && ammo.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                    "invalid bootstrap ammo seed"
                );
            }
        }
        if let Some(target) = &state.training_target {
            target.validate()?;
            ensure!(
                state.collision.is_some() && state.arsenal.is_some(),
                "training combat requires real collision and weapon resources"
            );
            ensure!(
                state
                    .collision
                    .as_ref()
                    .is_some_and(|c| c.captured_contents_mask
                        & crate::game::world::collision::SHOT_MASK
                        == crate::game::world::collision::SHOT_MASK),
                "training combat requires collision captured with target shot contents"
            );
        }
        ensure!(
            state.spawn_origin.is_some() == state.map_entities_sha256.is_some(),
            "spawn evidence missing"
        );
        if let Some(origin) = state.spawn_origin {
            ensure!(
                origin
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1_000_000.0),
                "invalid spawn origin"
            );
            let hash = state.map_entities_sha256.as_deref().unwrap_or("");
            ensure!(
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid map entity source hash"
            );
        }
        ensure!(
            state.schema_version == 1 && state.client_sha256 == codol_protocol::CLIENT_SHA256,
            "prepared game state identity mismatch"
        );
        ensure!(
            state.capture_sha256.len() == 64
                && state.capture_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "prepared game state source hash"
        );
        ensure!(
            (state.story_resource_only
                && state.map == codol_protocol::room::story::FIRST_MAP
                && state.mode == "coop"
                && state.clone().resource_initialization().is_ok())
                || (crate::content::maps::registry::supports_mode(&state.map, &state.mode)
                    && state.zone == "mp"
                    && state.playlist == 0),
            "prepared map requires a captured native team mode/map identity"
        );
        ensure!(
            (1..=512).contains(&state.asset_count) && (1..=65536).contains(&state.constant_count),
            "prepared game state resource counts"
        );
        if let Some(loadout) = &state.player_loadout {
            loadout.validate(state)?;
        }
        if let Some(minimap) = &state.minimap {
            ensure!(
                state.map_entities_sha256.is_some(),
                "minimap requires captured map entities"
            );
            minimap.validate(&state.map, state.ncs_crc)?;
        }
        if let Some(resources) = &state.feedback_resources {
            resources.validate(state.ncs_crc)?;
        }
        if let Some(resources) = &state.replay_resources {
            resources.validate(state.ncs_crc)?;
        }
        if let Some(resources) = &state.top_players {
            resources.validate(state.ncs_crc)?;
        }
        tracing::info!(
            ncs_crc = state.ncs_crc,
            common_crc = state.common_crc,
            observed_crc_verified = state.observed_crc_verified,
            "loaded prepared training state"
        );
        Ok(())
    }
}
