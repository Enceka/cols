//! One real PLAYER/client1 assembled from captured human assets and animations.
use anyhow::{Result, ensure};
use codol_protocol::game_enemy::EnemySnapshot;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnemyConfig {
    pub model: u16,
    pub origin: [f32; 3],
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub idle_animation: u16,
    pub walk_animation: u16,
    #[serde(default)]
    pub walk_left_animation: Option<u16>,
    #[serde(default)]
    pub walk_right_animation: Option<u16>,
    #[serde(default)]
    pub walk_back_animation: Option<u16>,
    #[serde(default)]
    pub locomotion: Option<NativeLocomotion>,
    pub fire_animation: u16,
    pub death_animation: u16,
    pub move_speed: f32,
    pub angles: [u16; 3],
    pub weapon: u16,
    pub source_sha256: String,
    #[serde(default)]
    pub bot: bool,
}

/// Bound to the captured3.3 script's autorifle rules and human registry.
/// Physical speed still comes from Pmove/WeaponDef rather than root motion.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeLocomotion {
    pub run_forward: u16,
    pub run_back: u16,
    pub run_left: u16,
    pub run_right: u16,
    pub sprint: u16,
    pub fire_standing: u16,
    pub script_sha256: String,
}

impl EnemyConfig {
    pub fn validate(&self) -> Result<()> {
        self.target().validate()?;
        if let Some(a) = &self.locomotion {
            ensure!(
                [
                    a.run_forward,
                    a.run_back,
                    a.run_left,
                    a.run_right,
                    a.sprint,
                    a.fire_standing
                ]
                .into_iter()
                .all(|i| (1..1024).contains(&i))
                    && a.script_sha256.len() == 64
                    && a.script_sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid native locomotion capture"
            );
        }
        ensure!(
            [
                self.walk_left_animation,
                self.walk_right_animation,
                self.walk_back_animation
            ]
            .into_iter()
            .flatten()
            .all(|a| (1..1024).contains(&a)),
            "invalid directional walk animation"
        );
        ensure!(
            (1..900).contains(&self.weapon),
            "enemy weapon outside registry"
        );
        ensure!(
            [
                self.idle_animation,
                self.walk_animation,
                self.fire_animation,
                self.death_animation
            ]
            .into_iter()
            .all(|a| (1..1024).contains(&a)),
            "invalid captured human animation"
        );
        ensure!(
            self.move_speed.is_finite() && (20.0..=300.0).contains(&self.move_speed),
            "invalid captured enemy movement speed"
        );
        Ok(())
    }

    pub fn target(&self) -> crate::game_combat::TargetConfig {
        crate::game_combat::TargetConfig {
            entity_number: 1,
            model: self.model,
            origin: self.origin,
            scale: 1.0,
            bounds_min: self.bounds_min,
            bounds_max: self.bounds_max,
            health: 100,
            respawn_msec: 3000,
            source_sha256: self.source_sha256.clone(),
        }
    }

    pub fn snapshot(&self, alive: bool) -> EnemySnapshot {
        EnemySnapshot {
            entity_number: 1,
            client_index: 1,
            team: 1,
            model: self.model,
            attachment_models: [0; 10],
            attachment_tags: [0; 10],
            origin: self.origin,
            angles: self.angles,
            legs_animation: self.idle_animation,
            torso_animation: self.idle_animation,
            weapon: self.weapon,
            alive,
            events: Default::default(),
        }
    }
}
