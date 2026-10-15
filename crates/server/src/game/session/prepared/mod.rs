//! Opt-in, resource-derived bootstrap for the controlled training playlist.
mod story_resource_state;
use anyhow::{Result, ensure};
use codol_protocol::game::gamestate::Gamestate;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
mod loadout;

#[derive(Default)]
pub(crate) struct SnapshotWorld<'a> {
    pub weapon_drops: &'a [codol_protocol::game::snapshot::weapon_drops::WeaponDropSnapshot],
    pub weapon_drop_baselines:
        &'a [codol_protocol::game::snapshot::weapon_drops::WeaponDropSnapshot],
    pub weapon_hint: Option<codol_protocol::game::snapshot::weapon_drops::WeaponHint>,
    pub props: &'a [codol_protocol::game::snapshot::props::PropSnapshot],
    pub prop_baselines: &'a [codol_protocol::game::snapshot::props::PropSnapshot],
    pub view_offset: [u16; 3],
    pub streaks: codol_protocol::game::snapshot::killstreaks::StreakSnapshot,
    pub radar_flags: Option<u8>,
    pub reward_inventory: [Option<codol_protocol::game::snapshot::EquippedWeapon>; 4],
    pub nuke_hud: Option<codol_protocol::game::snapshot::hit_feedback::NukeHudSnapshot>,
    pub preparation_hud:
        Option<codol_protocol::game::snapshot::hit_feedback::PreparationHudSnapshot>,
    pub perk_display: bool,
    pub bags: &'a [codol_protocol::game::snapshot::scavenger::BagSnapshot],
    pub bag_baselines: &'a [codol_protocol::game::snapshot::scavenger::BagSnapshot],
    pub shellshock: Option<codol_protocol::game::snapshot::shellshock::ShellShockSnapshot>,
    pub smokes: &'a [codol_protocol::game::snapshot::smoke::SmokeSnapshot],
    pub smoke_baselines: &'a [codol_protocol::game::snapshot::smoke::SmokeSnapshot],
    pub missiles: &'a [codol_protocol::game::snapshot::missile::MissileSnapshot],
    pub missile_baselines: &'a [codol_protocol::game::snapshot::missile::MissileSnapshot],
    pub local_abilities: Option<codol_protocol::game::snapshot::abilities::Abilities>,
    pub local_abilities_baseline: Option<codol_protocol::game::snapshot::abilities::Abilities>,
    pub delta_distance: u8,
    pub corpse_baselines: &'a [codol_protocol::game::snapshot::corpse::CorpseSnapshot],
    pub impact_baselines: &'a [codol_protocol::game::snapshot::impact::ImpactSnapshot],
    pub obituary_baselines: &'a [codol_protocol::game::snapshot::obituary::ObituarySnapshot],
    pub motion: Option<&'a crate::game::player::movement::Motion>,
    pub target: Option<codol_protocol::game::snapshot::target::TargetSnapshot>,
    pub impacts: &'a [codol_protocol::game::snapshot::impact::ImpactSnapshot],
    pub sounds: &'a [codol_protocol::game::snapshot::sound::SoundSnapshot],
    pub sound_baselines: &'a [codol_protocol::game::snapshot::sound::SoundSnapshot],
    pub enemy: Option<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    pub enemy_baseline: Option<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    pub players: &'a [codol_protocol::game::snapshot::enemy::EnemySnapshot],
    pub player_baselines: &'a [codol_protocol::game::snapshot::enemy::EnemySnapshot],
    pub hit_feedback: Option<codol_protocol::game::snapshot::hit_feedback::HitFeedbackSnapshot>,
    pub death_waypoints:
        Option<&'a [codol_protocol::game::snapshot::hit_feedback::DeathWaypointSnapshot]>,
    pub replay: Option<codol_protocol::game::snapshot::ReplayView>,
    pub held_weapons_in_baseline: bool,
    pub vitals: Option<codol_protocol::game::snapshot::PlayerVitals>,
    pub local_player: Option<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    pub local_player_baseline: Option<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    pub corpses: &'a [codol_protocol::game::snapshot::corpse::CorpseSnapshot],
    pub obituaries: &'a [codol_protocol::game::snapshot::obituary::ObituarySnapshot],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapWeapon {
    #[serde(default)]
    marketplace_item_id: Option<u32>,
    #[serde(default)]
    offhand_switch: Option<crate::game::weapons::held::AlternateTiming>,
    #[serde(default)]
    fast_reload: bool,
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
    switch: Option<crate::game::weapons::held::SwitchTiming>,
    #[serde(default)]
    move_scale: Option<f32>,
    #[serde(default)]
    ads_move_scale: Option<f32>,
    #[serde(default)]
    sprint_duration_scale: Option<f32>,
    #[serde(default)]
    sprint_timing: Option<crate::game::weapons::held::SprintTiming>,
    #[serde(default)]
    fire_type: Option<u32>,
    #[serde(default)]
    alternate_base: Option<u16>,
    #[serde(skip)]
    alternate: Option<Box<BootstrapWeapon>>,
    #[serde(default)]
    alternate_timing: Option<crate::game::weapons::held::AlternateTiming>,
    #[serde(default)]
    rechamber: Option<crate::game::weapons::held::RechamberTiming>,
    #[serde(default)]
    melee: Option<crate::game::weapons::held::MeleeTiming>,
    #[serde(default)]
    spread: Option<crate::game::weapons::held::SpreadTiming>,
    #[serde(default)]
    ballistics: Option<crate::game::combat::Ballistics>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapAmmo {
    #[serde(default)]
    limits: Option<crate::game::weapons::held::AmmoLimits>,
    clip_key: u16,
    reserve_key: u16,
    clip: u16,
    reserve: u16,
    source_sha256: String,
    #[serde(default)]
    fire_interval_msec: Option<u16>,
    #[serde(default)]
    reload: Option<crate::game::weapons::held::ReloadTiming>,
    #[serde(default)]
    ads: Option<crate::game::weapons::held::AdsTiming>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedGameState {
    #[serde(skip)]
    story_resource_only: bool,
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
    minimap: Option<crate::content::maps::minimap::Minimap>,
    #[serde(skip)]
    resource_compass_probe: Option<crate::content::maps::minimap::Minimap>,
    #[serde(default)]
    feedback_resources: Option<crate::game::presentation::feedback::Resources>,
    #[serde(default)]
    voice_file: Option<std::path::PathBuf>,
    #[serde(default)]
    nuke_file: Option<std::path::PathBuf>,
    #[serde(default)]
    radar_file: Option<std::path::PathBuf>,
    #[serde(skip)]
    radar_resources: Option<Arc<crate::game::rewards::killstreaks::RadarResources>>,
    #[serde(default)]
    props_file: Option<std::path::PathBuf>,
    #[serde(skip)]
    props_resources: Option<Arc<crate::game::world::props::Resources>>,
    #[serde(skip)]
    nuke_resources: Option<Arc<crate::game::rewards::killstreaks::NukeResources>>,
    #[serde(skip)]
    voice_resources: Option<Arc<crate::game::presentation::voice::Resources>>,
    #[serde(default)]
    replay_resources: Option<crate::game::presentation::replay::Resources>,
    #[serde(default)]
    top_players: Option<crate::game::presentation::top_players::Resources>,
    #[serde(default)]
    bootstrap_weapon: Option<BootstrapWeapon>,
    #[serde(default)]
    player_loadout: Option<loadout::Loadout>,
    #[serde(default)]
    secondary_weapon: Option<BootstrapWeapon>,
    #[serde(default)]
    training_target: Option<crate::game::combat::TargetConfig>,
    #[serde(default)]
    training_enemy: Option<crate::game::ai::enemy::EnemyConfig>,
    #[serde(default)]
    match_config: Option<crate::game::modes::tdm::MatchConfig>,
    #[serde(skip)]
    match_weapon: Option<BootstrapWeapon>,
    #[serde(skip)]
    roster_weapons: Vec<BootstrapWeapon>,
    #[serde(skip)]
    shared_bot_outfit: Option<crate::game::ai::loadout::BotOutfit>,
    #[serde(skip)]
    navigation: Arc<std::sync::OnceLock<Arc<crate::game::ai::navigation::Navigation>>>,
    #[serde(default)]
    arsenal_file: Option<std::path::PathBuf>,
    #[serde(default)]
    collision_file: Option<std::path::PathBuf>,
    #[serde(default)]
    appearance_file: Option<std::path::PathBuf>,
    #[serde(default)]
    appearance_first_model: Option<u16>,
    #[serde(skip)]
    appearance_catalog: Option<Arc<crate::content::characters::appearance::Catalog>>,
    #[serde(skip)]
    player_appearance: Option<crate::content::characters::appearance::Appearance>,
    #[serde(skip)]
    arsenal: Option<Arc<crate::content::weapons::arsenal::Arsenal>>,
    #[serde(skip)]
    pub(crate) collision: Option<Arc<crate::game::world::collision::Collision>>,
    #[serde(skip)]
    pub(crate) profile_hash: Option<[u8; 32]>,
    #[serde(skip)]
    pub(crate) player_card: Option<codol_protocol::game::snapshot::player_card::Card>,
    #[serde(skip)]
    player_perks: crate::game::status::perks::Perks,
    #[serde(skip)]
    player_streak_previews: crate::game::rewards::killstreaks::Previews,
    #[serde(skip)]
    player_grenades: [Option<crate::game::weapons::grenade::Equipped>; 2],
    #[serde(skip)]
    registry_strings: Vec<(u16, String)>,
    #[serde(skip)]
    snapshot_probe: bool,
    #[serde(skip)]
    snapshot_frames: u32,
}
impl PreparedGameState {
    pub(crate) fn has_nuke_diagnostics(&self) -> bool {
        self.nuke_resources
            .as_ref()
            .is_some_and(|r| r.acceptance_seed != 0 || r.acceptance_stationary)
    }
    pub(crate) fn nuke_resources(
        &self,
    ) -> Option<&crate::game::rewards::killstreaks::NukeResources> {
        self.nuke_resources.as_deref()
    }
    pub(crate) fn radar_resources(
        &self,
    ) -> Option<&crate::game::rewards::killstreaks::RadarResources> {
        self.radar_resources.as_deref()
    }
    pub(crate) fn streak_previews(&self) -> crate::game::rewards::killstreaks::Previews {
        self.player_streak_previews
    }
    pub(crate) fn spawn_angles(&self) -> [u16; 3] {
        self.match_config
            .as_ref()
            .and_then(|c| {
                c.spawn_angles
                    .as_ref()
                    .map(|a| a[usize::from(c.teams[0] - 1)][0])
            })
            .unwrap_or([0; 3])
    }
    pub(crate) fn for_local_world(&self) -> Self {
        let mut state = self.clone();
        state.collision = self.collision.as_ref().map(|c| Arc::new(c.fresh_session()));
        state
    }
    pub(crate) fn preserve_match_collision(&mut self, current: &Self) {
        assert_eq!(
            (self.map(), self.ncs_crc()),
            (current.map(), current.ncs_crc())
        );
        self.collision = current.collision.clone();
    }
    pub(crate) fn match_player_count(&self) -> u8 {
        self.match_config.as_ref().map_or(1, |c| c.player_count)
    }
    pub(crate) fn has_tdm(&self) -> bool {
        self.match_config.is_some()
    }
    pub(crate) fn room_bot_loadouts(
        &self,
    ) -> Option<Vec<[codol_protocol::room::messages::Loadout; 2]>> {
        self.match_config
            .as_ref()?
            .roster
            .iter()
            .map(|b| b.room_loadouts)
            .collect()
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
    pub(crate) fn resource_initialization(mut self) -> Result<Self> {
        ensure!(
            self.bootstrap_weapon.is_none()
                && self.player_loadout.is_none()
                && self.secondary_weapon.is_none()
                && self.training_enemy.is_none()
                && self.training_target.is_none()
                && self.match_config.is_none()
                && self.spawn_origin.is_none()
                && self.collision_file.is_none()
                && self.arsenal_file.is_none()
                && self.appearance_file.is_none(),
            "resource initialization requires metadata without a gameplay world"
        );
        self.snapshot_probe = false;
        self.snapshot_frames = 0;
        Ok(self)
    }
    pub(crate) fn snapshot_probe(&self) -> bool {
        self.snapshot_probe
    }
    pub(crate) fn with_resource_compass_probe(
        self,
        probe: &crate::content::maps::minimap::CompassProbe,
    ) -> Result<Self> {
        let mut state = self.resource_initialization()?;
        ensure!(
            state.map == probe.map && state.ncs_crc == probe.ncs_crc,
            "compass probe map/CRC differs from resource initialization"
        );
        state.resource_compass_probe = Some(probe.minimap.clone());
        Ok(state)
    }

    pub(crate) fn uses_player_loadout(&self) -> bool {
        self.player_loadout.is_some()
    }
    pub(crate) fn summary_map_id(&self) -> Result<u32> {
        crate::content::maps::registry::native_id(&self.map)
            .ok_or_else(|| anyhow::anyhow!("map has no verified summary ID"))
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
        self.spawn_origin = Some(crate::game::ai::controller::grounded_spawn(
            config.spawns[usize::from(teams[0] - 1)][0],
            collision,
        )?);
        Ok(self)
    }
    pub(crate) fn dynamic_loadout(&self) -> bool {
        self.arsenal.is_some()
    }
    pub(crate) fn feedback_resources(
        &self,
    ) -> Option<&crate::game::presentation::feedback::Resources> {
        self.feedback_resources.as_ref()
    }
    pub(crate) fn voice_resources(&self) -> Option<&crate::game::presentation::voice::Resources> {
        self.voice_resources.as_deref()
    }
    pub(crate) fn voice_actor(
        &self,
        snapshot: codol_protocol::game::snapshot::enemy::EnemySnapshot,
    ) -> crate::game::presentation::voice::Actor {
        let character = if snapshot.client_index == 0 {
            self.player_appearance.as_ref().and_then(|a| a.character)
        } else {
            self.match_config
                .as_ref()
                .and_then(|c| c.roster.get(usize::from(snapshot.client_index - 1)))
                .and_then(|b| b.outfits.as_ref())
                .and_then(|outfits| outfits[usize::from(snapshot.team - 1)].character)
        };
        crate::game::presentation::voice::Actor {
            life: 0,
            client: snapshot.client_index,
            team: snapshot.team,
            origin: snapshot.origin,
            alive: snapshot.alive,
            events: snapshot.events,
            prefix: character.and_then(|c| self.voice_resources().and_then(|r| r.prefix(c))),
        }
    }
    pub(crate) fn replay_resources(&self) -> Option<&crate::game::presentation::replay::Resources> {
        self.replay_resources.as_ref()
    }
    pub(crate) fn top_players(&self) -> Option<&crate::game::presentation::top_players::Resources> {
        self.top_players.as_ref()
    }
    pub(crate) fn top_players_scene(
        &self,
        rows: &[codol_protocol::game::snapshot::scoreboard::ScoreboardRow],
        mut actors: Vec<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    ) -> Result<crate::game::presentation::top_players::Scene> {
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
        crate::game::presentation::top_players::Scene::build(
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

    pub(crate) fn has_spawn(&self) -> bool {
        self.spawn_origin.is_some()
    }
    pub(crate) fn for_bot_validation(&self) -> Result<Self> {
        let mut state = self.for_local_world();
        let arsenal = self
            .arsenal
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("map arsenal missing"))?;
        let config = self
            .match_config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("TDM configuration missing"))?;
        let first = arsenal.next_index();
        let hands = self
            .bootstrap_weapon
            .as_ref()
            .and_then(|w| w.viewmodel_index);
        state.match_weapon = Some(Self::resolved_weapon(
            arsenal.bot_rifle(first + 4)?,
            &arsenal.source_sha256,
            hands,
        )?);
        state.roster_weapons = config
            .roster
            .iter()
            .enumerate()
            .map(|(i, bot)| {
                Self::resolved_weapon(
                    arsenal.bot_weapon(&bot.weapon_name, first + 5 + i as u16)?,
                    &arsenal.source_sha256,
                    hands,
                )
            })
            .collect::<Result<_>>()?;
        Ok(state)
    }
    pub(crate) fn match_bot_weapon(&self) -> Option<crate::game::ai::controller::BotWeapon> {
        let weapon = self.match_weapon.as_ref()?;
        Some(crate::game::ai::controller::BotWeapon {
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
        story_resource_only: false,
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
        resource_compass_probe: None,
        feedback_resources: None,
        voice_file: None,
        nuke_file: None,
        radar_file: None,
        radar_resources: None,
        props_file: None,
        props_resources: None,
        nuke_resources: None,
        voice_resources: None,
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
        shared_bot_outfit: None,
        navigation: Default::default(),
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
        player_perks: Default::default(),
        player_streak_previews: Default::default(),
        player_grenades: [None, None],
        registry_strings: Vec::new(),
        snapshot_probe: false,
        snapshot_frames: 0,
    }
}

#[cfg(test)]
mod tests;

mod loading;

mod preparation;

mod profile;

mod replication;

mod actors;
mod humans;
#[cfg(test)]
pub(crate) use humans::test_state as human_test_state;
