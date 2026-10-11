//! Opt-in, resource-derived bootstrap for the controlled training playlist.
use anyhow::{Result, ensure};
use codol_protocol::game_state::Gamestate;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
#[path = "game_loadout.rs"]
mod loadout;

#[derive(Default)]
pub(crate) struct SnapshotWorld<'a> {
    pub delta_distance: u8,
    pub corpse_baselines: &'a [codol_protocol::game_corpse::CorpseSnapshot],
    pub impact_baselines: &'a [codol_protocol::game_impact::ImpactSnapshot],
    pub obituary_baselines: &'a [codol_protocol::game_obituary::ObituarySnapshot],
    pub motion: Option<&'a crate::game_motion::Motion>,
    pub target: Option<codol_protocol::game_target::TargetSnapshot>,
    pub impacts: &'a [codol_protocol::game_impact::ImpactSnapshot],
    pub enemy: Option<codol_protocol::game_enemy::EnemySnapshot>,
    pub enemy_baseline: Option<codol_protocol::game_enemy::EnemySnapshot>,
    pub players: &'a [codol_protocol::game_enemy::EnemySnapshot],
    pub player_baselines: &'a [codol_protocol::game_enemy::EnemySnapshot],
    pub hit_feedback: Option<codol_protocol::game_hit_feedback::HitFeedbackSnapshot>,
    pub death_waypoints: Option<&'a [codol_protocol::game_hit_feedback::DeathWaypointSnapshot]>,
    pub replay: Option<codol_protocol::game_snapshot::ReplayView>,
    pub held_weapons_in_baseline: bool,
    pub vitals: Option<codol_protocol::game_snapshot::PlayerVitals>,
    pub local_player: Option<codol_protocol::game_enemy::EnemySnapshot>,
    pub local_player_baseline: Option<codol_protocol::game_enemy::EnemySnapshot>,
    pub corpses: &'a [codol_protocol::game_corpse::CorpseSnapshot],
    pub obituaries: &'a [codol_protocol::game_obituary::ObituarySnapshot],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapWeapon {
    name: String,
    index: u16,
    #[serde(default)]
    ammo: Option<BootstrapAmmo>,
    #[serde(default)]
    base_index: Option<u16>,
    #[serde(default)]
    composed_name: Option<String>,
    #[serde(default)]
    viewmodel_index: Option<u16>,
    #[serde(default)]
    attachments: Vec<u16>,
    #[serde(default)]
    attachment_source_sha256: Option<String>,
    #[serde(default)]
    camo: Option<u8>, // omitted only in legacy diagnostic preparations (camo3)
    #[serde(default)]
    switch: Option<crate::game_player::SwitchTiming>,
    #[serde(default)]
    move_scale: Option<f32>,
    #[serde(default)]
    fire_type: Option<u32>,
    #[serde(default)]
    alternate_base: Option<u16>,
    #[serde(skip)]
    alternate: Option<Box<BootstrapWeapon>>,
    #[serde(default)]
    alternate_timing: Option<crate::game_player::AlternateTiming>,
    #[serde(default)]
    rechamber: Option<crate::game_player::RechamberTiming>,
    #[serde(default)]
    spread: Option<crate::game_player::SpreadTiming>,
    #[serde(default)]
    ballistics: Option<crate::game_combat::Ballistics>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapAmmo {
    clip_key: u16,
    reserve_key: u16,
    clip: u16,
    reserve: u16,
    source_sha256: String,
    #[serde(default)]
    fire_interval_msec: Option<u16>,
    #[serde(default)]
    reload: Option<crate::game_player::ReloadTiming>,
    #[serde(default)]
    ads: Option<crate::game_player::AdsTiming>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedGameState {
    schema_version: u32,
    client_sha256: String,
    capture_sha256: String,
    map: String,
    mode: String,
    zone: String,
    playlist: u32,
    ncs_crc: u32,
    common_crc: u32,
    constant_count: u32,
    asset_count: u32,
    observed_crc_verified: bool,
    #[serde(default)]
    spawn_origin: Option<[f32; 3]>,
    #[serde(default)]
    map_entities_sha256: Option<String>,
    #[serde(default)]
    minimap: Option<crate::game_minimap::Minimap>,
    #[serde(default)]
    feedback_resources: Option<crate::game_feedback::Resources>,
    #[serde(default)]
    replay_resources: Option<crate::game_replay::Resources>,
    #[serde(default)]
    top_players: Option<crate::game_top_players::Resources>,
    #[serde(default)]
    bootstrap_weapon: Option<BootstrapWeapon>,
    #[serde(default)]
    player_loadout: Option<loadout::Loadout>,
    #[serde(default)]
    secondary_weapon: Option<BootstrapWeapon>,
    #[serde(default)]
    training_target: Option<crate::game_combat::TargetConfig>,
    #[serde(default)]
    training_enemy: Option<crate::game_enemy::EnemyConfig>,
    #[serde(default)]
    match_config: Option<crate::game_match::MatchConfig>,
    #[serde(skip)]
    match_weapon: Option<BootstrapWeapon>,
    #[serde(skip)]
    roster_weapons: Vec<BootstrapWeapon>,
    #[serde(skip)]
    navigation: Option<Arc<crate::game_navigation::Navigation>>,
    #[serde(default)]
    arsenal_file: Option<std::path::PathBuf>,
    #[serde(default)]
    collision_file: Option<std::path::PathBuf>,
    #[serde(default)]
    appearance_file: Option<std::path::PathBuf>,
    #[serde(default)]
    appearance_first_model: Option<u16>,
    #[serde(skip)]
    appearance_catalog: Option<Arc<crate::game_appearance::Catalog>>,
    #[serde(skip)]
    player_appearance: Option<crate::game_appearance::Appearance>,
    #[serde(skip)]
    arsenal: Option<Arc<crate::game_arsenal::Arsenal>>,
    #[serde(skip)]
    pub(crate) collision: Option<Arc<crate::game_motion::Collision>>,
    #[serde(skip)]
    pub(crate) profile_hash: Option<[u8; 32]>,
    #[serde(skip)]
    pub(crate) player_card: Option<codol_protocol::game_player_card::Card>,
    #[serde(skip)]
    registry_strings: Vec<(u16, String)>,
    #[serde(skip)]
    snapshot_probe: bool,
    #[serde(skip)]
    snapshot_frames: u32,
}
impl PreparedGameState {
    pub(crate) fn has_tdm(&self) -> bool {
        self.match_config.is_some()
    }
    pub fn map(&self) -> &str {
        &self.map
    }
    pub fn mode(&self) -> &str {
        &self.mode
    }
    pub fn ncs_crc(&self) -> u32 {
        self.ncs_crc
    }
    pub fn prepare_training_enemy(
        state_path: &Path,
        capture_path: &Path,
        output: &Path,
        distance: f32,
        model_name: &str,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Model {
            model: u16,
            name: String,
            bounds_min: [f32; 3],
            bounds_max: [f32; 3],
            bone_names: Vec<String>,
        }
        #[derive(Deserialize)]
        struct Animation {
            index: u16,
            name: String,
            move_speed_bits: u32,
        }
        #[derive(Deserialize)]
        struct Capture {
            schema_version: u32,
            read_only: bool,
            client_sha256: String,
            ncs_crc: u32,
            models: Vec<Model>,
            animations: Vec<Animation>,
        }
        ensure!(
            distance.is_finite() && (64.0..=400.0).contains(&distance),
            "enemy distance must be64..400"
        );
        ensure!(
            std::fs::metadata(capture_path)?.len() <= 4 * 1024 * 1024,
            "character capture too large"
        );
        let raw = std::fs::read(capture_path)?;
        let capture: Capture = serde_json::from_slice(&raw)?;
        let mut state = Self::load(state_path)?;
        ensure!(
            capture.schema_version == 1
                && capture.read_only
                && capture.client_sha256 == codol_protocol::CLIENT_SHA256
                && capture.ncs_crc == state.ncs_crc,
            "character capture identity mismatch"
        );
        let models: Vec<_> = capture
            .models
            .iter()
            .filter(|m| m.name == model_name)
            .collect();
        ensure!(
            models.len() == 1,
            "enemy body must have one registered model identity"
        );
        let model = models[0];
        for bone in [
            "j_mainroot",
            "j_head",
            "j_spinelower",
            "j_wrist_le",
            "j_wrist_ri",
            "tag_weapon_right",
        ] {
            ensure!(
                model.bone_names.iter().any(|b| b == bone),
                "selected enemy model is not a complete human skeleton: missing {bone}"
            );
        }
        let animation = |name: &str| -> Result<&Animation> {
            let found: Vec<_> = capture
                .animations
                .iter()
                .filter(|a| a.name == name)
                .collect();
            ensure!(
                found.len() == 1 && (1..1024).contains(&found[0].index),
                "human animation {name} missing or ambiguous"
            );
            Ok(found[0])
        };
        let collision = state
            .collision
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("enemy requires map collision"))?;
        ensure!(
            collision.captured_contents_mask & crate::game_motion::SHOT_MASK
                == crate::game_motion::SHOT_MASK,
            "enemy requires full shot collision capture"
        );
        let spawn = state
            .spawn_origin
            .ok_or_else(|| anyhow::anyhow!("enemy requires player spawn"))?;
        let mut origin = [spawn[0] + distance, spawn[1], 0.0];
        let start = [origin[0], origin[1], spawn[2] + 512.0];
        let end = [origin[0], origin[1], spawn[2] - 2048.0];
        let floor = collision.raycast(start, end);
        ensure!(
            floor.contents != 0 && !floor.start_solid && floor.normal[2] >= 0.7,
            "no walkable ground under enemy candidate"
        );
        origin[2] = (start[2] + (end[2] - start[2]) * floor.fraction - model.bounds_min[2]).ceil();
        origin = crate::game_bot::grounded_spawn(origin, collision)?;
        let eye = [spawn[0], spawn[1], spawn[2] + 60.0];
        let center =
            std::array::from_fn(|i| origin[i] + (model.bounds_min[i] + model.bounds_max[i]) * 0.5);
        let sight = collision.raycast(eye, center);
        ensure!(
            !sight.start_solid && (sight.contents == 0 || sight.fraction >= 1.0),
            "enemy candidate is behind world geometry"
        );
        let walk = animation("pb_stand_shoot_walk_forward")?;
        let weapon = state
            .bootstrap_weapon
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("enemy requires registered weapon resources"))?;
        let config = crate::game_enemy::EnemyConfig {
            model: model.model,
            origin,
            bounds_min: model.bounds_min,
            bounds_max: model.bounds_max,
            idle_animation: animation("pb_stand_alert")?.index,
            walk_animation: walk.index,
            walk_left_animation: Some(animation("pb_stand_shoot_walk_left")?.index),
            walk_right_animation: Some(animation("pb_stand_shoot_walk_right")?.index),
            walk_back_animation: Some(animation("pb_stand_shoot_walk_back")?.index),
            locomotion: None,
            fire_animation: animation("pt_stand_shoot_auto")?.index,
            reload_animation: None,
            death_animation: animation("pb_stand_death_chest_blowback")?.index,
            move_speed: f32::from_bits(walk.move_speed_bits),
            angles: [0, 32768, 0],
            weapon: weapon.base_index.unwrap_or(weapon.index),
            source_sha256: format!("{:x}", Sha256::digest(&raw)),
            bot: false,
        };
        config.validate()?;
        state.training_target = None;
        state.training_enemy = Some(config.clone());
        state.validate()?;
        let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(state_path)?)?;
        json.as_object_mut().unwrap().remove("training_target");
        json["training_enemy"] = serde_json::json!({"model":config.model,"origin":config.origin,"bounds_min":config.bounds_min,"bounds_max":config.bounds_max,"idle_animation":config.idle_animation,"walk_animation":config.walk_animation,"walk_left_animation":config.walk_left_animation,"walk_right_animation":config.walk_right_animation,"walk_back_animation":config.walk_back_animation,"fire_animation":config.fire_animation,"death_animation":config.death_animation,"move_speed":config.move_speed,"angles":config.angles,"weapon":config.weapon,"source_sha256":config.source_sha256});
        for name in ["arsenal_file", "collision_file"] {
            if let Some(path) = json[name].as_str() {
                json[name] = state_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(path)
                    .canonicalize()?
                    .to_string_lossy()
                    .into_owned()
                    .into();
            }
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        file.write_all(&serde_json::to_vec_pretty(&json)?)?;
        file.write_all(b"\n")?;
        Ok(())
    }
    pub fn prepare_training_target(
        state_path: &Path,
        model_path: &Path,
        output: &Path,
        distance: f32,
        scale: f32,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct ModelCapture {
            schema_version: u32,
            read_only: bool,
            client_sha256: String,
            ncs_crc: u32,
            model_name: String,
            model: u16,
            bounds_min: [f32; 3],
            bounds_max: [f32; 3],
        }
        ensure!(
            distance.is_finite() && (64.0..=400.0).contains(&distance),
            "training target distance must be 64..400 world units"
        );
        ensure!(
            scale.is_finite() && (0.25..=4.0).contains(&scale),
            "target scale outside supported range"
        );
        ensure!(
            std::fs::metadata(model_path)?.len() <= 65536,
            "target model capture too large"
        );
        let raw = std::fs::read(model_path)?;
        let model: ModelCapture = serde_json::from_slice(&raw)?;
        let mut state = Self::load(state_path)?;
        ensure!(
            model.schema_version == 1
                && model.read_only
                && model.client_sha256 == codol_protocol::CLIENT_SHA256
                && model.ncs_crc == state.ncs_crc
                && model.model_name == "com_barrel_russian_fuel_dirt",
            "training model identity mismatch"
        );
        let collision = state
            .collision
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("target needs map collision"))?;
        ensure!(
            collision.captured_contents_mask & crate::game_motion::SHOT_MASK
                == crate::game_motion::SHOT_MASK,
            "capture both movement and shooting contents before target preparation"
        );
        let spawn = state
            .spawn_origin
            .ok_or_else(|| anyhow::anyhow!("target needs player spawn"))?;
        let floor_at = |x: f32, y: f32| -> Result<f32> {
            let start = [x, y, spawn[2] + 512.0];
            let end = [x, y, spawn[2] - 2048.0];
            let hit = collision.raycast(start, end);
            ensure!(
                hit.contents != 0 && hit.fraction < 1.0 && !hit.start_solid && hit.normal[2] >= 0.7,
                "no walkable ground under target candidate"
            );
            Ok(start[2] + (end[2] - start[2]) * hit.fraction)
        };
        let mut config = crate::game_combat::TargetConfig {
            entity_number: 19,
            model: model.model,
            origin: [spawn[0] + distance, spawn[1], 0.0],
            scale,
            bounds_min: model.bounds_min,
            bounds_max: model.bounds_max,
            health: 100,
            respawn_msec: 3000,
            source_sha256: format!("{:x}", Sha256::digest(&raw)),
        };
        config.origin[2] =
            (floor_at(config.origin[0], config.origin[1])? - config.bounds_min[2] * scale).ceil();
        // Entity coordinates use integral world positions; use precisely those
        // positions for the server's hit volume as well as its visible model.
        config.origin = config.origin.map(f32::round_ties_even);
        config.validate()?;
        let eye = [spawn[0], spawn[1], floor_at(spawn[0], spawn[1])? + 60.0];
        let center = std::array::from_fn(|i| {
            config.origin[i] + (config.bounds_min[i] + config.bounds_max[i]) * 0.5 * scale
        });
        let sight = collision.raycast(eye, center);
        ensure!(
            sight.contents == 0 || sight.fraction >= 1.0,
            "training target candidate is behind map geometry"
        );
        state.training_target = Some(config.clone());
        state.validate()?;
        let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(state_path)?)?;
        json["training_target"] = serde_json::json!({"entity_number":config.entity_number,"model":config.model,"origin":config.origin,"scale":config.scale,"bounds_min":config.bounds_min,"bounds_max":config.bounds_max,"health":config.health,"respawn_msec":config.respawn_msec,"source_sha256":config.source_sha256});
        for name in ["arsenal_file", "collision_file"] {
            if let Some(file) = json[name].as_str() {
                json[name] = serde_json::Value::String(
                    state_path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(file)
                        .canonicalize()?
                        .to_string_lossy()
                        .into(),
                );
            }
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        file.write_all(&serde_json::to_vec_pretty(&json)?)?;
        file.write_all(b"\n")?;
        Ok(())
    }
    pub fn with_snapshot_frames(mut self, frames: u32) -> Result<Self> {
        ensure!(
            (1..=18000).contains(&frames),
            "snapshot frame budget out of range"
        );
        self.snapshot_probe = true;
        self.snapshot_frames = frames;
        Ok(self)
    }
    pub fn with_continuous_play(mut self) -> Self {
        self.snapshot_probe = true;
        self.snapshot_frames = u32::MAX;
        self
    }
    pub(crate) fn snapshot_limit(&self) -> u32 {
        self.snapshot_frames
    }
    pub(crate) fn snapshot_probe(&self) -> bool {
        self.snapshot_probe
    }
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= 65536,
            "prepared game state size"
        );
        let mut state: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        let directory = path.parent().unwrap_or(Path::new("."));
        if let Some(file) = &state.arsenal_file {
            state.arsenal = Some(Arc::new(crate::game_arsenal::Arsenal::load(
                directory.join(file),
                state.ncs_crc,
            )?));
        }
        if let Some(file) = &state.collision_file {
            let collision = crate::game_motion::Collision::load(&directory.join(file))?;
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
            state.appearance_catalog = Some(Arc::new(crate::game_appearance::Catalog::load(
                &directory.join(file),
            )?));
        }
        state.validate()?;
        state.ground_first_spawn()?;
        if let Some(config) = &state.match_config {
            state.navigation = Some(Arc::new(crate::game_navigation::Navigation::build(
                config.spawns.iter().flatten().copied(),
                state.collision.as_ref().unwrap(),
            )));
        }
        Ok(state)
    }
    /// Raw spawn entities can sit a few units inside terrain (Crash Axis
    /// starts are 4 units low); the first life must use the same grounded
    /// start as respawns, or every player sweep begins solid.
    fn ground_first_spawn(&mut self) -> Result<()> {
        let (Some(origin), Some(collision)) = (self.spawn_origin, &self.collision) else {
            return Ok(());
        };
        let start = match &self.match_config {
            Some(config) => config.spawns[usize::from(config.teams[0] - 1)][0],
            None => origin,
        };
        let height = crate::game_motion::Stance::Stand.height();
        if self.match_config.is_some() || collision.trace(start, start, height).start_solid {
            self.spawn_origin = Some(crate::game_bot::grounded_spawn(start, collision)?);
        }
        Ok(())
    }
    fn validate(&self) -> Result<()> {
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
                crate::game_bot::grounded_spawn(enemy.origin, state.collision.as_ref().unwrap())?;
            }
        }
        if let Some(config) = &state.match_config {
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
                    && weapon.spread.is_none_or(|s| s.valid())
                    && weapon
                        .move_scale
                        .is_none_or(|s| s.is_finite() && s > 0.0 && s <= 4.0),
                "invalid weapon switching/movement parameters"
            );
            ensure!(
                weapon.fire_type.is_none_or(|v| matches!(v, 0 | 1 | 5)),
                "unsupported burst firing state machine"
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
                            == Some(&codol_protocol::game_state::composed_weapon(
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
                            .is_none_or(|n| (50..=2000).contains(&n))
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
                    .is_some_and(|c| c.captured_contents_mask & crate::game_motion::SHOT_MASK
                        == crate::game_motion::SHOT_MASK),
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
            matches!(
                state.map.as_str(),
                "mp_afghan" | "mp_killhouse_sh" | "mp_crash_tropical"
            ) && state.mode == "war"
                && state.zone == "mp"
                && state.playlist == 0,
            "only the controlled training playlist is supported"
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
            minimap.validate(&state.map)?;
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

    pub(crate) fn uses_player_loadout(&self) -> bool {
        self.player_loadout.is_some()
    }
    pub(crate) fn summary_map_id(&self) -> Result<u32> {
        // IDs from this build's captured mp/coMapList.csv column 3.
        match self.map.as_str() {
            "mp_afghan" => Ok(1),
            "mp_crash_tropical" => Ok(4),
            "mp_killhouse_sh" => Ok(8),
            _ => anyhow::bail!("map has no verified summary ID"),
        }
    }
    pub(crate) fn with_room_teams(mut self, teams: [u8; 12]) -> Result<Self> {
        let config = self
            .match_config
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("native bot room requires TDM preparation"))?;
        config.teams = teams;
        let collision = self
            .collision
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("room team spawns require collision"))?;
        config.validate(collision)?;
        self.spawn_origin = Some(crate::game_bot::grounded_spawn(
            config.spawns[usize::from(teams[0] - 1)][0],
            collision,
        )?);
        Ok(self)
    }
    pub(crate) fn dynamic_loadout(&self) -> bool {
        self.arsenal.is_some()
    }
    pub(crate) fn enemy_snapshot(
        &self,
        alive: bool,
    ) -> Option<codol_protocol::game_enemy::EnemySnapshot> {
        self.training_enemy.as_ref().map(|enemy| {
            let mut snapshot = enemy.snapshot(alive);
            if let Some(weapon) = &self.bootstrap_weapon {
                snapshot.weapon = weapon.index;
            }
            snapshot
        })
    }
    pub(crate) fn local_player_snapshot(
        &self,
        alive: bool,
    ) -> Option<codol_protocol::game_enemy::EnemySnapshot> {
        let mut player = self.enemy_snapshot(alive)?;
        player.entity_number = 0;
        player.client_index = 0;
        player.team = self.match_config.as_ref().map_or(2, |c| c.teams[0]);
        // Captured complete standard male skeleton. This training appearance
        // is independent of the already live first-person backpack equipment.
        player.model = 4;
        player.attachment_models = [3, 2, 5, 6, 0, 0, 0, 0, 0, 0];
        player.attachment_tags = [0; 10];
        if let Some(appearance) = &self.player_appearance {
            player.model = appearance.indices[0];
            player.attachment_models = [0; 10];
            for (slot, &index) in player
                .attachment_models
                .iter_mut()
                .zip(&appearance.indices[1..])
            {
                *slot = index;
            }
        }
        player.origin = self.spawn_origin?;
        player.angles = [0; 3];
        Some(player)
    }
    pub(crate) fn corpse(
        &self,
        player: codol_protocol::game_enemy::EnemySnapshot,
        sequence: u32,
    ) -> codol_protocol::game_corpse::CorpseSnapshot {
        let animation = self.training_enemy.as_ref().unwrap().death_animation;
        codol_protocol::game_corpse::CorpseSnapshot {
            entity_number: if player.client_index == 0 { 20 } else { 19 },
            donor_client: player.client_index,
            origin: player.origin,
            angles: player.angles,
            legs_animation: animation,
            torso_animation: animation,
            weapon: player.weapon,
            present: true,
            phase: codol_protocol::game_corpse::CorpsePhase::Playing,
            death_sequence: sequence,
        }
    }
    pub(crate) fn bot(&self) -> Option<crate::game_bot::BotController> {
        if self.is_match() {
            return None;
        }
        self.training_enemy
            .as_ref()
            .filter(|enemy| enemy.bot)
            .map(|enemy| {
                crate::game_bot::BotController::new(
                    crate::game_bot::grounded_spawn(enemy.origin, self.collision.as_ref().unwrap())
                        .expect("validated bot spawn"),
                    enemy.angles,
                    enemy.move_speed,
                )
                .expect("validated bot configuration")
            })
    }
    pub(crate) fn bot_weapon(&self) -> Option<crate::game_bot::BotWeapon> {
        let weapon = self.bootstrap_weapon.as_ref()?;
        Some(crate::game_bot::BotWeapon {
            weapon: weapon.index,
            ballistics: weapon.ballistics?,
            fire_interval_msec: u32::from(weapon.ammo.as_ref()?.fire_interval_msec?),
        })
    }
    pub(crate) fn animate_enemy(
        &self,
        enemy: codol_protocol::game_enemy::EnemySnapshot,
        frame: &crate::game_bot::BotFrame,
        firing: bool,
        toggle: bool,
    ) -> codol_protocol::game_enemy::EnemySnapshot {
        let config = self
            .training_enemy
            .as_ref()
            .expect("enemy frame requires configuration");
        Self::animate_with_config(config, enemy, frame, firing, toggle)
    }
    pub(crate) fn animate_with_config(
        config: &crate::game_enemy::EnemyConfig,
        mut enemy: codol_protocol::game_enemy::EnemySnapshot,
        frame: &crate::game_bot::BotFrame,
        firing: bool,
        toggle: bool,
    ) -> codol_protocol::game_enemy::EnemySnapshot {
        enemy.origin = frame.motion.origin;
        enemy.angles = frame.angles;
        let moving = frame.moving();
        let direction = if moving {
            let yaw = f32::from(frame.angles[1]) * std::f32::consts::TAU / 65536.0;
            let (sin, cos) = yaw.sin_cos();
            let forward = frame.motion.velocity[0] * cos + frame.motion.velocity[1] * sin;
            let right = -frame.motion.velocity[0] * sin + frame.motion.velocity[1] * cos;
            if right.abs() > forward.abs() {
                if right > 0.0 { 2 } else { 3 }
            } else if forward < 0.0 {
                1
            } else {
                0
            }
        } else {
            0
        };
        enemy.legs_animation = if moving {
            if let Some(a) = &config.locomotion {
                if frame.locomotion == crate::game_bot::Locomotion::Sprint {
                    a.sprint
                } else if frame.locomotion == crate::game_bot::Locomotion::Run {
                    [a.run_forward, a.run_back, a.run_left, a.run_right][direction]
                } else {
                    [
                        Some(config.walk_animation),
                        config.walk_back_animation,
                        config.walk_left_animation,
                        config.walk_right_animation,
                    ][direction]
                        .unwrap_or(config.walk_animation)
                }
            } else {
                [
                    Some(config.walk_animation),
                    config.walk_back_animation,
                    config.walk_left_animation,
                    config.walk_right_animation,
                ][direction]
                    .unwrap_or(config.walk_animation)
            }
        } else {
            config.idle_animation
        };
        // Current playeranim.script's fireweapon/movetype moving branch is
        // empty: the full-body locomotion already holds the gun correctly.
        enemy.torso_animation = if firing && !(moving && config.locomotion.is_some()) {
            config
                .locomotion
                .as_ref()
                .map_or(config.fire_animation, |a| a.fire_standing)
                | if toggle { 0x400 } else { 0 }
        } else {
            0 // The legs node carries the full-body pose; torso0 keeps
            // an additional idle node from overriding its moving legs.
        };
        enemy
    }
    pub(crate) fn feedback_resources(&self) -> Option<&crate::game_feedback::Resources> {
        self.feedback_resources.as_ref()
    }
    pub(crate) fn replay_resources(&self) -> Option<&crate::game_replay::Resources> {
        self.replay_resources.as_ref()
    }
    pub(crate) fn top_players(&self) -> Option<&crate::game_top_players::Resources> {
        self.top_players.as_ref()
    }
    pub(crate) fn top_players_scene(
        &self,
        rows: &[codol_protocol::game_scoreboard::ScoreboardRow],
        mut actors: Vec<codol_protocol::game_enemy::EnemySnapshot>,
    ) -> Result<crate::game_top_players::Scene> {
        let config = self
            .match_config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("podium requires a match"))?;
        for actor in &mut actors {
            let animation = actor
                .client_index
                .checked_sub(1)
                .and_then(|i| config.roster.get(usize::from(i)))
                .map(|b| &b.animation)
                .or(self.training_enemy.as_ref())
                .ok_or_else(|| anyhow::anyhow!("podium idle animation missing"))?;
            actor.legs_animation = animation.idle_animation;
        }
        let seeds: Vec<_> = config.spawns.iter().flatten().copied().collect();
        crate::game_top_players::Scene::build(
            rows,
            &actors,
            &seeds,
            self.collision.as_ref().unwrap(),
        )
    }
    pub(crate) fn next_weapon_index(&self) -> u16 {
        if let Some((index, _)) = self.registry_strings.last() {
            return index - 5443 + 1;
        }
        self.bootstrap_weapon
            .iter()
            .chain(&self.secondary_weapon)
            .chain(&self.match_weapon)
            .chain(&self.roster_weapons)
            .map(|w| w.index + 1)
            .max()
            .unwrap_or(1)
    }

    pub(crate) fn for_profile(&self, public: &[u8], owned: &[u32]) -> Result<Self> {
        let mut state = self.for_profile_at(
            public,
            owned,
            self.bootstrap_weapon
                .as_ref()
                .map_or(self.next_weapon_index(), |w| w.index),
        )?;
        let mut next = self.bootstrap_weapon.as_ref().map_or(1, |w| w.index);
        let mut names = std::collections::BTreeMap::new();
        state.register_weapons(&mut names, &mut next)?;
        state.registry_strings = names
            .into_iter()
            .map(|(name, index)| (5443 + index, name))
            .collect();
        state.registry_strings.sort_by_key(|v| v.0);
        Ok(state)
    }
    pub(crate) fn for_backpack(&self, public: &[u8], slot: u8) -> Result<Self> {
        let mut state = self.clone();
        state.player_loadout = Some(
            self.player_loadout
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing backpack schema"))?
                .selected_for(public, slot)?,
        );
        Ok(state)
    }
    pub(crate) fn selected_backpack_class(&self) -> Option<u8> {
        self.player_loadout
            .as_ref()
            .map(|loadout| loadout.class_index())
    }
    pub(crate) fn update_profile_selection(&mut self, selected: &Self) {
        self.player_loadout = selected.player_loadout.clone();
        self.profile_hash = selected.profile_hash;
        self.player_card = selected.player_card;
    }
    pub(crate) fn preserve_match_appearance(&mut self, current: &Self) {
        // Character changes belong to the next match. Existing actor/corpse
        // baselines and historical replays retain the current model identities.
        self.player_appearance = current.player_appearance.clone();
    }
    pub(crate) fn set_card_rank(&mut self, (rank, prestige): (u8, u8)) {
        if let Some(card) = &mut self.player_card {
            card.rank = rank;
            card.prestige = prestige;
        }
    }
    pub(crate) fn first_weapon_index(&self) -> u16 {
        self.bootstrap_weapon.as_ref().map_or(1, |w| w.index)
    }
    pub(crate) fn for_profile_at(
        &self,
        public: &[u8],
        owned: &[u32],
        next_index: u16,
    ) -> Result<Self> {
        let loadout = self
            .player_loadout
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing loadout catalogue"))?;
        let mut state = self.clone();
        state.player_card = Some(loadout.card(public)?);
        if let Some(catalog) = &self.appearance_catalog
            && self.player_appearance.is_none()
        {
            let snapshot = loadout.profile_snapshot(public)?;
            let mut appearance = catalog.compose(
                &snapshot,
                owned,
                self.match_config.as_ref().map_or(2, |c| c.teams[0]),
            )?;
            appearance.register(
                self.match_config
                    .as_ref()
                    .map_or(&[], |c| c.models.as_slice()),
                self.appearance_first_model.unwrap(),
            )?;
            tracing::info!(package=appearance.package,models=?appearance.models,"resolved saved third-person character");
            state.player_appearance = Some(appearance);
        }
        if let Some(arsenal) = &self.arsenal {
            let selections = loadout.selections(public)?;
            let hands = self
                .bootstrap_weapon
                .as_ref()
                .and_then(|w| w.viewmodel_index);
            state.bootstrap_weapon = Some(Self::resolved_with_alternate(
                arsenal,
                arsenal.resolve(selections[0].0, &selections[0].1, owned, next_index)?,
                hands,
            )?);
            state.secondary_weapon = if selections[1].0 == 0 {
                None
            } else {
                Some(Self::resolved_with_alternate(
                    arsenal,
                    arsenal.resolve(selections[1].0, &selections[1].1, owned, next_index + 2)?,
                    hands,
                )?)
            };
            tracing::info!(primary=?state.bootstrap_weapon.as_ref().and_then(|w|w.composed_name.as_deref()),secondary=?state.secondary_weapon.as_ref().and_then(|w|w.composed_name.as_deref()),"composed current player backpack from resource attributes");
            if let Some(config) = &self.match_config {
                state.match_weapon = Some(Self::resolved_weapon(
                    arsenal.bot_rifle(next_index + 4)?,
                    &arsenal.source_sha256,
                    hands,
                )?);
                state.roster_weapons = config
                    .roster
                    .iter()
                    .enumerate()
                    .map(|(i, bot)| {
                        Self::resolved_weapon(
                            arsenal.bot_weapon(&bot.weapon_name, next_index + 5 + i as u16)?,
                            &arsenal.source_sha256,
                            hands,
                        )
                    })
                    .collect::<Result<_>>()?;
            }
        } else {
            state.bootstrap_weapon = Some(loadout.resolve(self, public, owned)?);
            state.player_loadout = None;
        }
        state.profile_hash = Some(Sha256::digest(public).into());
        state.validate()?;
        Ok(state)
    }
    fn resolved_with_alternate(
        arsenal: &crate::game_arsenal::Arsenal,
        resolved: crate::game_arsenal::ResolvedWeapon,
        hands: Option<u16>,
    ) -> Result<BootstrapWeapon> {
        let alternate = arsenal.resolve_alternate(&resolved)?;
        let mut weapon = Self::resolved_weapon(resolved, &arsenal.source_sha256, hands)?;
        if let Some(resolved) = alternate {
            // Target 0x42c270: +0xdad aliases the parent's magazine key.
            ensure!(
                resolved.definition[0xdad] != 0,
                "alternate with independent clip is not yet supported"
            );
            let mut alternate = Self::resolved_weapon(resolved, &arsenal.source_sha256, hands)?;
            let parent_ammo = weapon.ammo.as_ref().unwrap();
            let ammo = alternate.ammo.as_mut().unwrap();
            ammo.clip_key = parent_ammo.clip_key;
            ammo.clip = parent_ammo.clip;
            ammo.reserve_key = parent_ammo.reserve_key;
            ammo.reserve = parent_ammo.reserve;
            weapon.alternate = Some(Box::new(alternate));
        }
        Ok(weapon)
    }

    fn resolved_weapon(
        w: crate::game_arsenal::ResolvedWeapon,
        source: &str,
        hands: Option<u16>,
    ) -> Result<BootstrapWeapon> {
        let word = |at: usize| u32::from_le_bytes(w.definition[at..at + 4].try_into().unwrap());
        let time = |at: usize| -> Result<u16> {
            let n = word(at);
            ensure!(n <= 15000, "weapon duration exceeds supported range");
            Ok(n as u16)
        };
        let normal = time(0x71c)?;
        let empty = time(0x728)?;
        let add = time(0x730)?;
        let empty_add = time(0x738)?;
        ensure!(
            word(0xd08) > 0 && word(0xd08) <= 511 && word(0x6b8) <= 1023,
            "weapon ammo limits"
        );
        ensure!(
            word(0x6f0) == 0,
            "delayed-fire weapon needs its firing state machine"
        );
        let ammo = BootstrapAmmo {
            clip_key: w.index,
            reserve_key: word(0x6b8) as u16,
            clip: word(0xd08) as u16,
            reserve: (word(0xd08) * 3).min(2047) as u16,
            source_sha256: source.into(),
            fire_interval_msec: Some(time(0xd10)?),
            reload: Some(crate::game_player::ReloadTiming {
                normal_msec: normal,
                empty_msec: empty,
                add_msec: if add == 0 { normal } else { add.min(normal) },
                empty_add_msec: if empty_add == 0 {
                    empty
                } else {
                    empty_add.min(empty)
                },
                segmented: (w.definition[0xd86] != 0).then_some(
                    crate::game_player::SegmentedReload {
                        start_msec: time(0x740)?,
                        start_add_msec: match (time(0x740)?, time(0x744)?) {
                            (start, add) if add != 0 && add < start => add,
                            _ => 0,
                        },
                        end_msec: time(0x748)?,
                    },
                ),
            }),
            ads: if w.definition[0xd76] == 0 {
                None
            } else {
                Some(crate::game_player::AdsTiming {
                    in_rate: f32::from_bits(word(0xbac)),
                    out_rate: f32::from_bits(word(0xbb0)),
                    reload_allowed: w.definition[0xd78] != 0,
                    reload_interruptible: w.definition[0xd86] != 0,
                    reload_end_msec: time(0xb38)?,
                })
            },
        };
        let drop = time(0x74c)?;
        let raise = time(0x750)?;
        let switch = crate::game_player::SwitchTiming {
            drop_msec: drop,
            raise_msec: raise,
            empty_drop_msec: match time(0x768)? {
                0 => drop,
                v => v,
            },
            empty_raise_msec: match time(0x764)? {
                0 => raise,
                v => v,
            },
        };
        let scale = f32::from_bits(word(0x7c8));
        let fire_type = word(0x48c);
        // 0x42c270 / 0x42dff0 register a composed alternate immediately after
        // its parent. This inherited index identifies the base alt definition.
        let alternate_base = match word(0xd24) {
            0 => None,
            n if n < 900 => Some(n as u16),
            _ => anyhow::bail!("alternate weapon base outside registry"),
        };
        // Weapon class +0x480: grenade/rocket launchers (6/7), riot shield (11)
        // and melee (12) are not hitscan; spread (4) fires +0x6c8 pellets.
        let ballistics = if matches!(word(0x480), 6 | 7 | 11 | 12) || w.definition[0xd79] != 0 {
            None
        } else {
            let damage = word(0xbb4);
            let near_range = f32::from_bits(word(0xbcc));
            ensure!(
                damage <= u32::from(u16::MAX),
                "weapon damage outside supported limits"
            );
            let multipliers: [f32; 20] =
                std::array::from_fn(|i| f32::from_bits(word(0xbf0 + i * 4)));
            let value = crate::game_combat::Ballistics {
                damage: damage as u16,
                range: if word(0x480) == 4 {
                    f32::from_bits(word(0xbe0))
                } else {
                    8192.0
                },
                near_range,
                // The block is all zero for knives/launchers; keep base damage.
                location_multipliers: multipliers.iter().any(|m| *m > 0.0).then_some(multipliers),
                pellets: word(0x6c8).clamp(1, 32) as u8,
                // Standing hip spread min/max (+0x810/+0x81c), in degrees.
                spread_degrees: if word(0x6c8) > 1 {
                    f32::from_bits(word(0x810))
                } else {
                    0.0
                },
                falloff: crate::game_combat::DamageFalloff::from_stages(
                    std::array::from_fn(|i| word(0xbb4 + i * 4)),
                    std::array::from_fn(|i| f32::from_bits(word(0xbcc + i * 4))),
                ),
            };
            if value.valid() { Some(value) } else { None }
        };
        let alternate_timing = Some(crate::game_player::AlternateTiming {
            drop_msec: time(0x754)?,
            raise_msec: time(0xd28)?,
        });
        let rechamber = if w.definition[0xd74] != 0 {
            Some(crate::game_player::RechamberTiming {
                duration_msec: time(0x700)?,
                eject_msec: time(0x708)?,
            })
        } else {
            None
        };
        let spread = crate::game_player::SpreadTiming {
            minimum: std::array::from_fn(|i| f32::from_bits(word(0x810 + i * 4))),
            maximum: std::array::from_fn(|i| f32::from_bits(word(0x81c + i * 4))),
            decay: f32::from_bits(word(0x828)),
            fire_add: f32::from_bits(word(0x82c)),
            turn_add: f32::from_bits(word(0x830)),
            move_add: f32::from_bits(word(0x834)),
            stance_decay: [
                1.0,
                f32::from_bits(word(0x838)),
                f32::from_bits(word(0x83c)),
            ],
            ads: f32::from_bits(word(0xac0)),
        };
        ensure!(spread.valid(), "invalid weapon spread parameters");
        tracing::info!(weapon = %w.name, ads_in_msec = word(0xad0), ads_out_msec = word(0xad4),
            fire_msec = word(0xd10), rechamber_msec = rechamber.map_or(0, |t| t.duration_msec),
            hip_min = ?spread.minimum, hip_max = ?spread.maximum, "resolved weapon gameplay attributes");
        Ok(BootstrapWeapon {
            name: w.name,
            index: w.index,
            ammo: Some(ammo),
            base_index: Some(w.base_index),
            composed_name: Some(w.composed_name),
            viewmodel_index: hands,
            attachment_source_sha256: if w.attachments.is_empty() {
                None
            } else {
                Some(source.into())
            },
            attachments: w.attachments,
            camo: Some(0),
            switch: Some(switch),
            move_scale: Some(scale),
            fire_type: Some(fire_type),
            alternate_base,
            alternate: None,
            alternate_timing,
            rechamber,
            spread: Some(spread),
            ballistics,
        })
    }
    pub(crate) fn model_configstrings(&self) -> Vec<(u16, String)> {
        let mut values = self.match_config.as_ref().map_or_else(Vec::new, |config| {
            config
                .models
                .iter()
                .map(|(id, name)| (2051 + id, name.clone()))
                .collect()
        });
        if let Some(appearance) = &self.player_appearance {
            values.extend(
                appearance
                    .registrations
                    .iter()
                    .map(|(id, name)| (2051 + id, name.clone())),
            );
        }
        values
    }
    pub(crate) fn weapon_configstrings(&self) -> Vec<(u16, String)> {
        if !self.registry_strings.is_empty() {
            return self.registry_strings.clone();
        }
        self.bootstrap_weapon
            .iter()
            .chain(&self.secondary_weapon)
            .chain(&self.match_weapon)
            .chain(&self.roster_weapons)
            .filter_map(|w| {
                w.composed_name
                    .as_ref()
                    .map(|n| (5443 + w.index, n.clone()))
            })
            .collect()
    }
    pub(crate) fn register_weapons(
        &mut self,
        known: &mut std::collections::BTreeMap<String, u16>,
        next: &mut u16,
    ) -> Result<Vec<(u16, String)>> {
        let mut added = Vec::new();
        let first = known.values().min().copied().unwrap_or(*next);
        for weapon in self
            .bootstrap_weapon
            .iter_mut()
            .chain(&mut self.secondary_weapon)
            .chain(&mut self.match_weapon)
            .chain(&mut self.roster_weapons)
        {
            let name = weapon
                .composed_name
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing composed identity"))?;
            let index = if let Some(&index) = known.get(name) {
                index
            } else {
                ensure!(*next < 900, "weapon registry exhausted");
                let index = *next;
                *next += 1;
                known.insert(name.clone(), index);
                added.push((5443 + index, name.clone()));
                index
            };
            weapon.index = index;
            if let Some(ammo) = &mut weapon.ammo {
                ammo.clip_key = index;
            }
            if let Some(base) = weapon.alternate_base {
                // Def+0xc is empty for the target's shared-model stock alts.
                // 0x42dff0 retains the camo and parent composition ordinal,
                // followed by the same parts (format at0xc5da1c/0xc5da2c).
                let alternate = format!(
                    "** {base} {} {}{}",
                    weapon.camo.unwrap_or(0),
                    index - first,
                    weapon
                        .attachments
                        .iter()
                        .map(|id| format!(" {id}"))
                        .collect::<String>()
                );
                let alternate_index = if let Some(&id) = known.get(&alternate) {
                    id
                } else {
                    ensure!(*next < 900, "alternate weapon registry exhausted");
                    let id = *next;
                    *next += 1;
                    known.insert(alternate.clone(), id);
                    added.push((5443 + id, alternate.clone()));
                    id
                };
                if let Some(other) = &mut weapon.alternate {
                    other.index = alternate_index;
                    other.composed_name = Some(alternate);
                    if let Some(ammo) = &mut other.ammo {
                        ammo.clip_key = index;
                    }
                }
            }
        }
        Ok(added)
    }
    pub(crate) fn selected_weapon_names(&self) -> Vec<&str> {
        self.bootstrap_weapon
            .iter()
            .chain(&self.secondary_weapon)
            .filter_map(|w| w.composed_name.as_deref())
            .collect()
    }
    pub(crate) fn register_history(
        &mut self,
        registry: &mut crate::game_weapon_registry::WeaponRegistry,
    ) -> Result<()> {
        self.register_weapons(&mut registry.names, &mut registry.next)?;
        self.registry_strings = registry.configstrings();
        self.validate()?;
        Ok(())
    }

    pub(crate) fn matches(&self, build: i32, start: &str) -> bool {
        // This is a comparison with the captured control request, never a
        // console interpreter. Do not apply this map CRC to arbitrary rooms.
        build == -688906341
            && start.split(';').map(str::trim).eq([
                "mmstart sv_resetServer",
                "2486 1",
                "3265 12",
                "2489 1",
                "playlist 0",
                "start_map_rotate_once 0",
            ])
    }

    pub(crate) fn body(&self, command_sequence: u32) -> Result<Vec<u8>> {
        self.body_named(command_sequence, None)
    }

    pub(crate) fn body_named(&self, command_sequence: u32, name: Option<&str>) -> Result<Vec<u8>> {
        // CS0 keys mapname/g_gametype confirmed at 0x465aa2/0x465abf.
        // 0x46d830 parses CS3 into C2S serverId. CS16 is map-center text.
        let mut values = vec![
            (
                0,
                format!(
                    "\\mapname\\{}\\g_gametype\\{}\\scr_gameended\\0",
                    self.map, self.mode
                ),
            ),
            (1, String::new()),
            // 0x445b74 compares CS2 with literal "cod" before weapon init.
            (2, "cod".to_owned()),
            (3, "1".to_owned()),
            (16, "0 0 0".to_owned()),
            // 0x579f30 sscanf(CS6415, "%i %i %g %g", ...). Empty text leaves
            // uninitialized slow-motion values and nearly stops client time.
            (6415, "0 0 1 1".to_owned()),
        ];
        if self.training_enemy.is_some() {
            // Current build reads these raw nickname strings, not info pairs.
            values.extend([
                (
                    codol_protocol::game_held_weapon::HAND_TAG_CONFIGSTRING,
                    "tag_weapon_right".to_owned(),
                ),
                (1975, name.unwrap_or("Player").to_owned()),
                (1976, "Training Bot".to_owned()),
            ]);
            values.sort_by_key(|value| value.0);
        }
        if let Some(minimap) = &self.minimap {
            values.extend(minimap.configstrings());
            values.sort_by_key(|value| value.0);
        }
        if let Some(sound) = self
            .feedback_resources
            .as_ref()
            .and_then(|r| r.sound_configstring())
        {
            values.push(sound);
            values.sort_by_key(|v| v.0);
        }
        if self.match_config.is_some() {
            values.retain(|(index, _)| *index != 1976);
            for client in 1..=11u16 {
                values.push((
                    1975 + client,
                    codol_protocol::room_messages::bot_name(client as u8),
                ));
            }
            values.sort_by_key(|v| v.0);
        }
        // Publish names before cgame/model initialization, not after spawning.
        values.extend(self.model_configstrings());
        values.sort_by_key(|v| v.0);
        for (index, name) in self.weapon_configstrings() {
            // NCS inherited weapon bank starts at5443; unused indices are
            // untyped and accept a regular configstring. Parser0x42e3b0.
            values.push((index, name));
            values.sort_by_key(|v| v.0);
        }
        if self.bootstrap_weapon.is_some() {
            // 0x4435fa / 0x5597d0 iterate 1..<CS6800 and register each
            // CG weapon presentation. Without this the secondary's HUD name
            // stays null until viewmodel registration, racing HUD drawing.
            values.push((6800, self.next_weapon_index().to_string()));
            values.sort_by_key(|v| v.0);
        }
        Ok(Gamestate {
            command_sequence,
            map: &self.map,
            zone: &self.zone,
            ncs_crc: self.ncs_crc,
            configstrings: &values,
            client_number: 0,
            checksum_feed: 0,
        }
        .encode()?)
    }
    pub(crate) fn snapshot(
        &self,
        server_time: u32,
        command_time: u32,
        establish_baseline: bool,
        angles: [u16; 3],
        equipment: Option<codol_protocol::game_snapshot::Equipment>,
        world: SnapshotWorld<'_>,
    ) -> Result<Vec<u8>> {
        let motion = world.motion;
        Ok(codol_protocol::game_snapshot::player_frame_with_world(
            server_time,
            command_time,
            motion.map(|m| m.origin).or(self.spawn_origin),
            establish_baseline,
            equipment,
            angles,
            codol_protocol::game_snapshot::WorldSnapshot {
                delta_distance: world.delta_distance,
                corpse_baselines: world.corpse_baselines,
                impact_baselines: world.impact_baselines,
                obituary_baselines: world.obituary_baselines,
                target: world.target,
                impacts: world.impacts,
                enemy: world.enemy,
                enemy_baseline: world.enemy_baseline,
                players: world.players,
                player_baselines: world.player_baselines,
                hit_feedback: world.hit_feedback,
                death_waypoints: world.death_waypoints,
                replay: world.replay,
                local_card: self.player_card,
                held_weapon_tag: self
                    .training_enemy
                    .as_ref()
                    .map(|_| codol_protocol::game_held_weapon::HAND_TAG_SLOT),
                held_weapons_in_baseline: world.held_weapons_in_baseline,
                vitals: world.vitals,
                local_player: world.local_player,
                local_player_baseline: world.local_player_baseline,
                corpses: world.corpses,
                obituaries: world.obituaries,
                motion: motion.map(|m| codol_protocol::game_snapshot::MotionSnapshot {
                    velocity: m.velocity,
                    flags: m.flags,
                    ground_entity: m.ground_entity,
                    view_height: m.view_height,
                    view_height_target: m.view_height_target,
                    speed: m.speed,
                    gravity: m.gravity,
                    bob_cycle: m.bob_cycle,
                    view_height_lerp_time: m.view_height_lerp_time,
                    view_height_lerp_target: m.view_height_lerp_target,
                    view_height_lerp_down: m.view_height_lerp_down,
                    jump_time: m.jump_time,
                    jump_origin_z: m.jump_origin_z,
                }),
            },
        )?)
    }
    pub(crate) fn player(&self) -> Option<crate::game_player::Player> {
        self.primary_player()
            .map(|p| match self.secondary_player() {
                Some(s) => p.with_secondary(s),
                None => p,
            })
    }
    pub(crate) fn primary_player(&self) -> Option<crate::game_player::Player> {
        self.bootstrap_weapon.as_ref().map(Self::make_player)
    }
    pub(crate) fn secondary_player(&self) -> Option<crate::game_player::Player> {
        self.secondary_weapon.as_ref().map(Self::make_player)
    }
    fn make_player(w: &BootstrapWeapon) -> crate::game_player::Player {
        let mut player = crate::game_player::Player::new(
            codol_protocol::game_snapshot::Equipment {
                inventory_weapon: None,
                alternate_weapon: None,
                fire_count: 0,
                spread_scale: 0.0,
                needs_rechamber: false,
                secondary: None,
                events: None,
                weapon: w.index,
                viewmodel: w.viewmodel_index,
                firing: None,
                aim: None,
                ammo: w
                    .ammo
                    .as_ref()
                    .map(|a| codol_protocol::game_snapshot::Ammo {
                        clip_key: a.clip_key,
                        reserve_key: a.reserve_key,
                        clip: a.clip,
                        reserve: a.reserve,
                    }),
            },
            w.ammo.as_ref().and_then(|a| a.fire_interval_msec),
            w.ammo.as_ref().and_then(|a| a.reload),
            w.ammo.as_ref().and_then(|a| a.ads),
        );
        if let Some(timing) = w.switch {
            player.set_switch_timing(timing);
        }
        if let Some(timing) = w.rechamber {
            player.set_rechamber_timing(timing);
        }
        if let Some(timing) = w.spread {
            player.set_spread_timing(timing);
        }
        if let Some(scale) = w.move_scale {
            player.set_move_scale(scale);
        }
        if let Some(kind) = w.fire_type {
            player.set_fire_type(kind).expect("validated fire type");
        }
        if let Some(stats) = w.ballistics {
            player.set_ballistics(stats);
        }
        if let Some(alternate) = &w.alternate {
            player =
                player.with_alternate(Self::make_player(alternate), w.alternate_timing.unwrap());
        }
        player
    }
    pub(crate) fn motion(&self) -> Option<crate::game_motion::Motion> {
        self.collision
            .as_ref()
            .and(self.spawn_origin)
            .map(|origin| {
                let mut m = crate::game_motion::Motion::new(origin);
                m.command_time = 1000;
                m
            })
    }
    pub(crate) fn combat(&self) -> Option<crate::game_combat::CombatWorld> {
        if self.match_config.is_some() {
            return None;
        }
        self.training_target
            .clone()
            .or_else(|| self.training_enemy.as_ref().map(|enemy| enemy.target()))
            .map(crate::game_combat::CombatWorld::new)
    }
    pub(crate) fn input_base(&self) -> codol_protocol::game_usercmd::UserCommand {
        let mut base = codol_protocol::game_usercmd::UserCommand::default();
        if let Some(weapon) = &self.bootstrap_weapon {
            base.weapons[0] = weapon.index;
            base.melee_entity = 2047; // target 0x5a18e0, POF_PLAYER (0x1000)
        }
        base
    }
    pub(crate) fn has_spawn(&self) -> bool {
        self.spawn_origin.is_some()
    }
    pub(crate) fn tdm_match(&self) -> Result<Option<crate::game_match::TdmMatch>> {
        let Some(config) = &self.match_config else {
            return Ok(None);
        };
        let mut template = self
            .local_player_snapshot(true)
            .ok_or_else(|| anyhow::anyhow!("TDM missing player appearance"))?;
        template.weapon = self
            .match_weapon
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TDM bot rifle missing"))?
            .index;
        let mut tdm = crate::game_match::TdmMatch::with_navigation(
            config.clone(),
            template,
            self.training_enemy.as_ref().unwrap().move_speed,
            self.collision.as_ref().unwrap(),
            self.navigation
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("TDM navigation not prepared"))?
                .clone(),
        )?;
        tdm.configure_roster(self.roster_weapons.iter().map(Self::make_player).collect())?;
        tdm.set_weapon_move_scale(
            self.match_weapon
                .as_ref()
                .and_then(|w| w.move_scale)
                .unwrap_or(1.0),
        );
        Ok(Some(tdm))
    }
    pub(crate) fn match_bot_weapon(&self) -> Option<crate::game_bot::BotWeapon> {
        let weapon = self.match_weapon.as_ref()?;
        Some(crate::game_bot::BotWeapon {
            weapon: weapon.index,
            ballistics: weapon.ballistics?,
            fire_interval_msec: u32::from(weapon.ammo.as_ref()?.fire_interval_msec?),
        })
    }
    pub(crate) fn is_match(&self) -> bool {
        self.match_config.is_some()
    }
}

#[cfg(test)]
pub(crate) fn synthetic() -> PreparedGameState {
    PreparedGameState {
        schema_version: 1,
        client_sha256: codol_protocol::CLIENT_SHA256.into(),
        capture_sha256: "0".repeat(64),
        map: "mp_afghan".into(),
        mode: "war".into(),
        zone: "mp".into(),
        playlist: 0,
        ncs_crc: 0x12345678,
        common_crc: 1,
        constant_count: 2,
        asset_count: 1,
        observed_crc_verified: false,
        spawn_origin: None,
        map_entities_sha256: None,
        minimap: None,
        feedback_resources: None,
        replay_resources: None,
        top_players: None,
        bootstrap_weapon: None,
        player_loadout: None,
        secondary_weapon: None,
        training_target: None,
        training_enemy: None,
        match_config: None,
        match_weapon: None,
        roster_weapons: Vec::new(),
        navigation: None,
        arsenal_file: None,
        collision_file: None,
        appearance_file: None,
        appearance_first_model: None,
        appearance_catalog: None,
        player_appearance: None,
        arsenal: None,
        collision: None,
        profile_hash: None,
        player_card: None,
        registry_strings: Vec::new(),
        snapshot_probe: false,
        snapshot_frames: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires independently captured map states in CODOL_PRESENTATION_STATES"]
    fn captured_maps_have_grounded_podiums_within_snapshot_budget() {
        let paths =
            std::env::var_os("CODOL_PRESENTATION_STATES").expect("set captured map state paths");
        for path in std::env::split_paths(&paths) {
            let mut state = PreparedGameState::load(&path).unwrap();
            let config = state.match_config.as_ref().unwrap();
            let mut actors = vec![state.local_player_snapshot(true).unwrap()];
            for (i, entry) in config.roster.iter().enumerate() {
                let mut actor = actors[0];
                actor.client_index = i as u8 + 1;
                actor.entity_number = i as u16 + 1;
                actor.model = entry.animation.model;
                actor.attachment_models = entry.attachment_models;
                actor.team = config.teams[i + 1];
                actor.weapon = 352 + i as u16;
                actors.push(actor);
            }
            let rows: Vec<_> = (0..12)
                .map(
                    |client_number| codol_protocol::game_scoreboard::ScoreboardRow {
                        client_number,
                        score: 1200 - i32::from(client_number) * 100,
                        ..Default::default()
                    },
                )
                .collect();
            let scene = state.top_players_scene(&rows, actors.clone()).unwrap();
            state.player_card = Some(codol_protocol::game_player_card::Card {
                rank: 69,
                prestige: 2,
                icon: 138,
                title: 214,
                background: 396,
                nameplate: 0,
            });
            let live = state
                .snapshot(
                    10000,
                    10000,
                    true,
                    [0; 3],
                    state.player().map(|p| p.equipment()),
                    SnapshotWorld {
                        motion: state.motion().as_ref(),
                        local_player: Some(actors[0]),
                        players: &actors[1..],
                        vitals: Some(Default::default()),
                        ..Default::default()
                    },
                )
                .unwrap();
            eprintln!(
                "{}: live characters plus held weapons {} bytes",
                state.map,
                live.len()
            );
            assert!(live.len() <= codol_protocol::game_wire::FRAGMENT_SIZE - 5);
            let mut firing = actors.clone();
            for actor in &mut firing {
                for _ in 0..2 {
                    actor
                        .events
                        .push(codol_protocol::game_events::FIRE_WEAPON, 31)
                        .unwrap();
                    actor
                        .events
                        .push(codol_protocol::game_events::EJECT_BRASS, 0)
                        .unwrap();
                }
            }
            let impacts: Vec<_> = (54..62)
                .map(
                    |entity_number| codol_protocol::game_impact::ImpactSnapshot {
                        entity_number,
                        origin: [123.0, 456.0, 70.0],
                        normal_byte: 0,
                        surface: 1,
                        weapon: 352,
                    },
                )
                .collect();
            let delta = state
                .snapshot(
                    10050,
                    10050,
                    false,
                    [0; 3],
                    state.player().map(|p| p.equipment()),
                    SnapshotWorld {
                        delta_distance: 1,
                        motion: state.motion().as_ref(),
                        local_player: Some(firing[0]),
                        players: &firing[1..],
                        local_player_baseline: Some(actors[0]),
                        player_baselines: &actors[1..],
                        impacts: &impacts,
                        held_weapons_in_baseline: false,
                        vitals: Some(Default::default()),
                        ..Default::default()
                    },
                )
                .unwrap();
            eprintln!(
                "{}: first weapon delta with firing/events {} bytes",
                state.map,
                delta.len()
            );
            assert!(delta.len() <= codol_protocol::game_wire::FRAGMENT_SIZE - 5);
            let snapshot = state
                .snapshot(
                    10000,
                    10000,
                    true,
                    scene.angles,
                    state.player().map(|p| p.equipment()),
                    SnapshotWorld {
                        motion: Some(&scene.camera),
                        local_player: Some(scene.actors[0]),
                        players: &scene.actors[1..],
                        vitals: Some(codol_protocol::game_snapshot::PlayerVitals {
                            intermission: true,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )
                .unwrap();
            eprintln!(
                "{}: camera={:?}, actors={:?}, bytes={}",
                state.map,
                scene.camera.origin,
                scene.actors[12..]
                    .iter()
                    .map(|a| a.origin)
                    .collect::<Vec<_>>(),
                snapshot.len()
            );
            assert!(snapshot.len() <= codol_protocol::game_wire::FRAGMENT_SIZE - 5);
        }
    }
    #[test]
    fn first_spawn_below_terrain_is_grounded_like_a_respawn() {
        let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,62],"half":[4000,4000,10],"contents":1,"planes":[]}],"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let collision = crate::game_motion::Collision::load(file.path()).unwrap();
        let mut state = synthetic();
        // Floor top is z=72; the raw entity origin is four units inside it.
        state.spawn_origin = Some([0.0, 0.0, 68.0]);
        state.collision = Some(Arc::new(collision));
        state.ground_first_spawn().unwrap();
        let origin = state.spawn_origin.unwrap();
        assert!((72.0..72.5).contains(&origin[2]), "{origin:?}");
        let collision = state.collision.clone().unwrap();
        let mut motion = state.motion().unwrap();
        for _ in 0..30 {
            let cmd = codol_protocol::game_usercmd::UserCommand {
                time: motion.command_time + 16,
                direction: 1,
                ..Default::default()
            };
            motion.apply(&cmd, 1.0, false, &collision);
        }
        assert!(motion.origin[0] > 50.0, "{:?}", motion.origin);
    }
    #[test]
    fn match_summary_map_ids_follow_the_current_map_table() {
        let mut state = synthetic();
        for (map, id) in [
            ("mp_afghan", 1),
            ("mp_crash_tropical", 4),
            ("mp_killhouse_sh", 8),
        ] {
            state.map = map.into();
            assert_eq!(state.summary_map_id().unwrap(), id);
        }
        state.map = "mp_unknown".into();
        assert!(state.summary_map_id().is_err());
    }
    #[test]
    fn walking_full_body_animation_is_not_overridden_by_a_standing_torso_pose() {
        let mut state = synthetic();
        let config:crate::game_enemy::EnemyConfig=serde_json::from_value(serde_json::json!({
            "model":4,"origin":[0,0,0],"bounds_min":[-15,-15,0],"bounds_max":[15,15,70],
            "idle_animation":395,"walk_animation":243,"walk_left_animation":241,"walk_right_animation":240,"walk_back_animation":242,
            "fire_animation":616,"death_animation":47,"move_speed":94.0,"angles":[0,0,0],"weapon":352,"source_sha256":"a".repeat(64)
        })).unwrap();
        state.training_enemy = Some(config.clone());
        let mut motion = crate::game_motion::Motion::new([0.0; 3]);
        motion.velocity = [94.0, 0.0, 0.0];
        let mut frame = crate::game_bot::BotFrame {
            motion,
            angles: [0; 3],
            phase: crate::game_bot::BotPhase::Chase,
            shot: None,
            locomotion: crate::game_bot::Locomotion::Walk,
        };
        let current = state.animate_enemy(config.snapshot(true), &frame, false, false);
        assert_eq!((current.legs_animation, current.torso_animation), (243, 0));
        frame.motion.velocity = [-94.0, 0.0, 0.0];
        assert_eq!(
            state
                .animate_enemy(current, &frame, false, false)
                .legs_animation,
            242
        );
        frame.motion.velocity = [0.0, 94.0, 0.0];
        assert_eq!(
            state
                .animate_enemy(current, &frame, false, false)
                .legs_animation,
            241
        );
        frame.motion.velocity = [0.0, -94.0, 0.0];
        assert_eq!(
            state
                .animate_enemy(current, &frame, false, false)
                .legs_animation,
            240
        );
        assert_eq!(
            state
                .animate_enemy(current, &frame, true, true)
                .torso_animation,
            616 | 0x400
        );
        state.training_enemy.as_mut().unwrap().locomotion =
            Some(crate::game_enemy::NativeLocomotion {
                run_forward: 328,
                run_back: 325,
                run_left: 328,
                run_right: 328,
                sprint: 309,
                fire_standing: 617,
                script_sha256: "a".repeat(64),
            });
        frame.motion.velocity = [190.0, 0.0, 0.0];
        frame.motion.flags = 0;
        frame.locomotion = crate::game_bot::Locomotion::Run;
        let running = state.animate_enemy(current, &frame, true, false);
        assert_eq!((running.legs_animation, running.torso_animation), (328, 0));
        frame.motion.flags = 0x4000;
        frame.locomotion = crate::game_bot::Locomotion::Sprint;
        assert_eq!(
            state
                .animate_enemy(current, &frame, false, false)
                .legs_animation,
            309
        );
        frame.motion.flags = 0x10;
        frame.locomotion = crate::game_bot::Locomotion::Walk;
        assert_eq!(
            state
                .animate_enemy(current, &frame, false, false)
                .legs_animation,
            243
        );
        frame.motion.velocity = [0.0; 3];
        let standing = state.animate_enemy(current, &frame, true, false);
        assert_eq!(
            (standing.legs_animation, standing.torso_animation),
            (395, 617)
        );
    }
    #[test]
    fn prepared_training_state_cannot_match_another_playlist_or_console_suffix() {
        let state = synthetic();
        let start =
            "mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist 0; start_map_rotate_once 0";
        assert!(state.matches(-688906341, start));
        assert!(!state.matches(0, start));
        assert!(!state.matches(-688906341, &start.replace("playlist 0", "playlist 1")));
        assert!(!state.matches(-688906341, &format!("{start}; map other")));
    }
    #[test]
    fn bootstrap_supplies_a_complete_normal_speed_slowmotion_config() {
        let body = synthetic().body(1).unwrap();
        let mut r = codol_protocol::game_wire::Reader::new(&body);
        assert_eq!(r.u8().unwrap(), 3);
        r.u32().unwrap();
        r.string().unwrap();
        r.string().unwrap();
        r.u32().unwrap();
        assert_eq!(r.u8().unwrap(), 4);
        let count = r.u16().unwrap();
        let mut index = -1i32;
        let mut slowmo = None;
        for _ in 0..count {
            index = if r.bits(1).unwrap() == 1 {
                index + 1
            } else {
                r.bits(13).unwrap() as i32
            };
            let value = r.string().unwrap();
            if index == 6415 {
                slowmo = Some(std::str::from_utf8(value).unwrap().to_owned());
            }
        }
        let fields = slowmo
            .unwrap()
            .split_whitespace()
            .map(|v| v.parse::<f32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(fields, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(r.bits(10).unwrap(), 0);
        assert_eq!(r.bits(10).unwrap(), 0);
        assert_eq!(r.u8().unwrap(), 6);
        r.u32().unwrap();
        r.u32().unwrap();
        assert_eq!(r.u8().unwrap(), 6);
        r.finish().unwrap();
    }
}
