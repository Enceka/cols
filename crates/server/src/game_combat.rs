//! One server-owned training prop with coarse model bounds and real world occlusion.
//!
//! WeaponDef +0xbb4 supplies damage only inside its first distance band (+0xbcc).
//! Later damage bands and skeletal hit locations are not recovered by this path.
//! Authorised, de-duplicated weapon shots are supplied by the player simulation.
use anyhow::{Result, ensure};
use codol_protocol::game_target::TargetSnapshot;
use serde::Deserialize;

use crate::game_motion::Collision;

#[derive(Clone, Copy, Debug)]
pub struct WorldImpact {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub surface: u8,
}

#[derive(Default, Debug)]
pub struct ShotOutcome {
    pub feedback: Option<String>,
    pub impact: Option<WorldImpact>,
}

/// Trace ordinary bullets even without a damageable training prop. A live
/// prop stops the ray, including outside the currently supported damage band.
fn trace_shot(
    eye: [f32; 3],
    angles: [u16; 3],
    range: f32,
    collision: &Collision,
    target: Option<&TargetConfig>,
) -> (Option<f32>, Option<WorldImpact>) {
    let direction = forward(angles);
    let end = std::array::from_fn(|i| eye[i] + direction[i] * range);
    let world = collision.raycast(eye, end);
    if world.start_solid || world.all_solid {
        return (None, None);
    }
    let target_fraction = target.and_then(|config| {
        let min = std::array::from_fn(|i| config.origin[i] + config.bounds_min[i] * config.scale);
        let max = std::array::from_fn(|i| config.origin[i] + config.bounds_max[i] * config.scale);
        ray_aabb(eye, end, min, max)
    });
    if let Some(fraction) = target_fraction
        && (world.contents == 0 || fraction < world.fraction)
    {
        return (Some(fraction), None);
    }
    // Current surfaceparm table: sky4, noimpact16, nomarks32, nodraw128. The
    // effect's material columns0..30 exclude flesh-only columns31..34.
    // No decal-only event flag is recovered; nomarks conservatively suppresses
    // this whole event rather than creating a forbidden world mark.
    let impact = (world.contents != 0
        && world.surface_flags & 0xb4 == 0
        && ((world.surface_flags >> 20) & 31) <= 30)
        .then(|| WorldImpact {
            origin: std::array::from_fn(|i| eye[i] + (end[i] - eye[i]) * world.fraction),
            normal: world.normal,
            surface: ((world.surface_flags >> 20) & 31) as u8,
        });
    (None, impact)
}

pub fn world_impact(
    eye: [f32; 3],
    angles: [u16; 3],
    ballistics: Ballistics,
    collision: &Collision,
) -> Option<WorldImpact> {
    if !ballistics.valid() || eye.iter().any(|n| !n.is_finite() || n.abs() > 1_000_000.0) {
        return None;
    }
    trace_shot(eye, angles, ballistics.range, collision, None).1
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ballistics {
    pub damage: u16,
    /// Full world ray length, in world units; ordinary target traces use 8192.
    pub range: f32,
    /// First damage band, in world units. No later falloff is invented.
    pub near_range: f32,
}

impl Ballistics {
    pub fn valid(self) -> bool {
        self.damage > 0
            && self.range.is_finite()
            && self.range > 0.0
            && self.range <= 8192.0
            && self.near_range.is_finite()
            && self.near_range > 0.0
            && self.near_range <= self.range
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub entity_number: u16,
    pub model: u16,
    pub origin: [f32; 3],
    #[serde(default = "unit_scale")]
    pub scale: f32,
    /// Coarse local model bounds, translated by `origin` for hit testing.
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub health: u16,
    pub respawn_msec: u32,
    pub source_sha256: String,
}

fn unit_scale() -> f32 {
    1.0
}

impl TargetConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.entity_number == 19,
            "training target must use entity 19"
        );
        ensure!(
            (1..1024).contains(&self.model),
            "invalid target model index"
        );
        ensure!(
            self.scale.is_finite() && (0.25..=4.0).contains(&self.scale),
            "target scale outside supported range"
        );
        ensure!(
            self.origin
                .iter()
                .all(|n| n.is_finite() && (-32768.0..=32767.0).contains(n)),
            "target origin cannot be represented in the entity snapshot"
        );
        ensure!(
            (0..3).all(|i| {
                self.bounds_min[i].is_finite()
                    && self.bounds_max[i].is_finite()
                    && self.bounds_min[i].abs() <= 4096.0
                    && self.bounds_max[i].abs() <= 4096.0
                    && self.bounds_min[i] < self.bounds_max[i]
            }),
            "invalid coarse target bounds"
        );
        ensure!(self.health > 0, "target health must be positive");
        ensure!(
            (1..=60_000).contains(&self.respawn_msec),
            "target respawn must be between 1 and 60000 ms"
        );
        ensure!(
            self.source_sha256.len() == 64
                && self.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid target source SHA256"
        );
        Ok(())
    }
}

#[derive(Debug)]
pub struct CombatWorld {
    config: TargetConfig,
    health: u16,
    death_time: Option<u32>,
    current_time: Option<u32>,
    last_shot_time: Option<u32>,
    life_start_time: Option<u32>,
}

impl CombatWorld {
    /// `config` must have passed `TargetConfig::validate` during preparation.
    pub fn new(config: TargetConfig) -> Self {
        Self {
            health: config.health,
            config,
            death_time: None,
            current_time: None,
            last_shot_time: None,
            life_start_time: None,
        }
    }

    pub fn health(&self) -> u16 {
        self.health
    }

    pub fn snapshot(&self) -> TargetSnapshot {
        TargetSnapshot {
            entity_number: self.config.entity_number,
            model: self.config.model,
            origin: self.config.origin,
            scale: self.config.scale,
            alive: self.health > 0,
        }
    }

    fn advance_clock(&mut self, now: u32) -> bool {
        // Native snapshot time is a wrapping u32 clock. A delta of half its
        // space or more is stale/ambiguous, not forward progress.
        if self
            .current_time
            .is_some_and(|previous| now.wrapping_sub(previous) >= 0x8000_0000)
        {
            return false;
        }
        self.current_time = Some(now);
        true
    }

    pub fn advance(&mut self, now: u32) -> Option<String> {
        if !self.advance_clock(now) {
            return None;
        }
        self.life_start_time.get_or_insert(now);
        let elapsed = now.wrapping_sub(self.death_time?);
        if elapsed < self.config.respawn_msec || elapsed >= 0x8000_0000 {
            return None;
        }
        self.health = self.config.health;
        self.death_time = None;
        self.life_start_time = Some(now);
        Some("Target ready".into())
    }

    pub fn fire(
        &mut self,
        time: u32,
        eye: [f32; 3],
        angles: [u16; 3],
        ballistics: Ballistics,
        collision: &Collision,
    ) -> ShotOutcome {
        if !ballistics.valid() || eye.iter().any(|n| !n.is_finite() || n.abs() > 1_000_000.0) {
            return ShotOutcome::default();
        }
        if self.last_shot_time.is_some_and(|previous| {
            let delta = time.wrapping_sub(previous);
            delta == 0 || delta >= 0x8000_0000
        }) {
            return ShotOutcome::default();
        }
        // A delayed shot belongs to the lifetime it was fired in. Do not
        // apply bullets from before this target appeared to its new health.
        if self
            .life_start_time
            .is_some_and(|birth| time.wrapping_sub(birth) >= 0x8000_0000)
        {
            return ShotOutcome::default();
        }
        // Commands normally lag the latest snapshot. Preserve their distinct
        // shot times for deduplication while scheduling damage on the latest
        // authoritative clock. Half-space separation cannot be ordered.
        let effective_time = match self.current_time {
            Some(current) => match time.wrapping_sub(current) {
                0x8000_0000 => return ShotOutcome::default(),
                delta if delta < 0x8000_0000 => time,
                _ => current,
            },
            None => time,
        };
        self.current_time = Some(effective_time);
        self.life_start_time.get_or_insert(time);
        // Misses and shots at a dead target also consume their shot identity.
        self.last_shot_time = Some(time);
        let (fraction, impact) = trace_shot(
            eye,
            angles,
            ballistics.range,
            collision,
            (self.health > 0).then_some(&self.config),
        );
        let mut outcome = ShotOutcome {
            feedback: None,
            impact,
        };
        let Some(fraction) = fraction else {
            return outcome;
        };
        if fraction * ballistics.range > ballistics.near_range {
            return outcome;
        }

        self.health = self.health.saturating_sub(ballistics.damage);
        outcome.feedback = Some(if self.health == 0 {
            self.death_time = Some(effective_time);
            "Training target destroyed".into()
        } else {
            format!("Training target: {} HP", self.health)
        });
        outcome
    }
}

fn forward(angles: [u16; 3]) -> [f32; 3] {
    let radians = std::f64::consts::TAU / 65536.0;
    let (sin_pitch, cos_pitch) = (f64::from(angles[0]) * radians).sin_cos();
    let (sin_yaw, cos_yaw) = (f64::from(angles[1]) * radians).sin_cos();
    [
        (cos_pitch * cos_yaw) as f32,
        (cos_pitch * sin_yaw) as f32,
        -sin_pitch as f32,
    ]
}

/// Segment slab intersection. A ray beginning inside the bounds enters at0;
/// parallel axes must already lie inside their slab. Ties include the face.
fn ray_aabb(start: [f32; 3], end: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Option<f32> {
    let (mut enter, mut leave) = (0.0f64, 1.0f64);
    for i in 0..3 {
        let start = f64::from(start[i]);
        let delta = f64::from(end[i]) - start;
        if delta.abs() < 1e-8 {
            if start < f64::from(min[i]) || start > f64::from(max[i]) {
                return None;
            }
            continue;
        }
        let a = (f64::from(min[i]) - start) / delta;
        let b = (f64::from(max[i]) - start) / delta;
        enter = enter.max(a.min(b));
        leave = leave.min(a.max(b));
        if enter > leave {
            return None;
        }
    }
    Some(enter as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn target() -> TargetConfig {
        TargetConfig {
            entity_number: 19,
            model: 156,
            origin: [100.0, 0.0, 30.0],
            scale: 1.0,
            bounds_min: [-10.0; 3],
            bounds_max: [10.0; 3],
            health: 100,
            respawn_msec: 1000,
            source_sha256: "a".repeat(64),
        }
    }

    fn ballistics() -> Ballistics {
        Ballistics {
            damage: 33,
            range: 8192.0,
            near_range: 500.0,
        }
    }

    fn collision(wall: Option<(f32, u32)>) -> Collision {
        let mut brushes = vec![json!({
            "center":[0,0,-20], "half":[2000,2000,20], "contents":1, "planes":[]
        })];
        if let Some((x, contents)) = wall {
            brushes.push(json!({
                "center":[x,0,30], "half":[5,100,100], "contents":contents, "planes":[]
            }));
        }
        let export = json!({
            "schema_version":1, "read_only":true,
            "client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp", "ncs_crc":0,
            "brushes":brushes, "triangles":[]
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        Collision::load(file.path()).unwrap()
    }

    fn shoot(
        world: &mut CombatWorld,
        time: u32,
        damage: u16,
        collision: &Collision,
    ) -> Option<String> {
        world
            .fire(
                time,
                [0.0, 0.0, 30.0],
                [0; 3],
                Ballistics {
                    damage,
                    ..ballistics()
                },
                collision,
            )
            .feedback
    }

    #[test]
    fn config_and_weapon_limits_reject_invalid_geometry_and_unbounded_time() {
        let config = target();
        config.validate().unwrap();
        for malformed in [
            TargetConfig {
                entity_number: 18,
                ..config.clone()
            },
            TargetConfig {
                model: 1024,
                ..config.clone()
            },
            TargetConfig {
                origin: [f32::NAN; 3],
                ..config.clone()
            },
            TargetConfig {
                bounds_max: [-10.0; 3],
                ..config.clone()
            },
            TargetConfig {
                health: 0,
                ..config.clone()
            },
            TargetConfig {
                respawn_msec: u32::MAX,
                ..config.clone()
            },
            TargetConfig {
                source_sha256: "z".repeat(64),
                ..config
            },
        ] {
            assert!(malformed.validate().is_err());
        }
        for malformed in [
            Ballistics {
                damage: 0,
                ..ballistics()
            },
            Ballistics {
                range: f32::NAN,
                ..ballistics()
            },
            Ballistics {
                range: 8193.0,
                ..ballistics()
            },
            Ballistics {
                near_range: 8193.0,
                ..ballistics()
            },
            Ballistics {
                near_range: 0.0,
                ..ballistics()
            },
        ] {
            assert!(!malformed.valid());
        }
        assert!(
            serde_json::from_value::<Ballistics>(json!({
                "damage":33,"range":8192,"near_range":500,"ignored":true
            }))
            .is_err()
        );
    }

    #[test]
    fn short_angles_use_standard_yaw_and_downward_positive_pitch() {
        for (angles, expected) in [
            ([0, 0, 0], [1.0, 0.0, 0.0]),
            ([0, 16384, 0], [0.0, 1.0, 0.0]),
            ([0, 32768, 1234], [-1.0, 0.0, 0.0]),
            ([0, 49152, 0], [0.0, -1.0, 0.0]),
            ([16384, 0, 0], [0.0, 0.0, -1.0]),
            ([49152, 0, 0], [0.0, 0.0, 1.0]),
        ] {
            let got = forward(angles);
            assert!((0..3).all(|i| (got[i] - expected[i]).abs() < 1e-6));
        }
    }

    #[test]
    fn slabs_find_nearest_entry_and_reject_parallel_or_backward_misses() {
        let min = [10.0, -2.0, -2.0];
        let max = [20.0, 2.0, 2.0];
        assert_eq!(ray_aabb([0.0; 3], [100.0, 0.0, 0.0], min, max), Some(0.1));
        assert_eq!(
            ray_aabb([15.0, 0.0, 0.0], [100.0, 0.0, 0.0], min, max),
            Some(0.0)
        );
        assert_eq!(ray_aabb([0.0, 3.0, 0.0], [100.0, 3.0, 0.0], min, max), None);
        assert_eq!(ray_aabb([0.0; 3], [-100.0, 0.0, 0.0], min, max), None);
        assert_eq!(ray_aabb([0.0; 3], [10.0, 0.0, 0.0], min, max), Some(1.0));
    }

    #[test]
    fn shots_damage_once_and_dead_targets_wait_for_authoritative_respawn() {
        let collision = collision(None);
        let mut world = CombatWorld::new(target());
        assert_eq!(
            shoot(&mut world, 100, 33, &collision).as_deref(),
            Some("Training target: 67 HP")
        );
        assert!(shoot(&mut world, 100, 33, &collision).is_none());
        assert!(shoot(&mut world, 99, 33, &collision).is_none());
        assert_eq!(world.health(), 67);
        assert_eq!(
            shoot(&mut world, 101, 1000, &collision).as_deref(),
            Some("Training target destroyed")
        );
        assert_eq!(world.health(), 0);
        assert!(!world.snapshot().alive);
        assert!(shoot(&mut world, 102, 33, &collision).is_none());
        assert!(world.advance(1100).is_none());
        assert_eq!(world.advance(1101).as_deref(), Some("Target ready"));
        assert_eq!(world.health(), 100);
        assert!(world.snapshot().alive);
        assert!(world.advance(1101).is_none());
        assert!(world.advance(1100).is_none());
        assert_eq!(
            shoot(&mut world, 1101, 33, &collision).as_deref(),
            Some("Training target: 67 HP")
        );
    }

    #[test]
    fn timer_wrap_preserves_respawn_and_rejects_old_shots_after_wrap() {
        let collision = collision(None);
        let mut world = CombatWorld::new(target());
        let death = u32::MAX - 20;
        assert!(shoot(&mut world, death, 100, &collision).is_some());
        assert!(world.advance(death.wrapping_add(999)).is_none());
        assert!(world.advance(death.wrapping_add(1000)).is_some());
        assert!(shoot(&mut world, death.wrapping_add(10), 100, &collision).is_none());
        assert_eq!(world.health(), 100);
        assert!(shoot(&mut world, death.wrapping_add(1001), 33, &collision).is_some());
    }

    #[test]
    fn commands_can_lag_the_snapshot_clock_without_repeating_a_shot() {
        let collision = collision(None);
        let mut world = CombatWorld::new(target());
        assert!(world.advance(1000).is_none());
        assert!(world.advance(1100).is_none());
        assert!(shoot(&mut world, 1060, 33, &collision).is_some());
        assert_eq!(world.health(), 67);
        assert_eq!(world.current_time, Some(1100));
        assert!(shoot(&mut world, 1060, 33, &collision).is_none());
        assert_eq!(world.health(), 67);
    }

    #[test]
    fn distinct_command_shots_share_a_snapshot_clock_and_death_uses_that_clock() {
        let collision = collision(None);
        let mut world = CombatWorld::new(target());
        world.advance(1000);
        world.advance(1100);
        for (time, health) in [(1060, 67), (1061, 34), (1062, 1), (1063, 0)] {
            assert!(shoot(&mut world, time, 33, &collision).is_some());
            assert_eq!(world.health(), health);
            assert_eq!(world.current_time, Some(1100));
        }
        assert_eq!(world.death_time, Some(1100));
        assert!(world.advance(2099).is_none());
        assert_eq!(world.advance(2100).as_deref(), Some("Target ready"));
    }

    #[test]
    fn delayed_shots_from_the_dead_lifetime_do_not_hurt_a_respawned_target() {
        let collision = collision(None);
        let mut world = CombatWorld::new(target());
        world.advance(1000);
        world.advance(1100);
        assert!(shoot(&mut world, 1060, 100, &collision).is_some());
        assert!(world.advance(2100).is_some());
        // This is later than the last processed shot but before the new life.
        assert!(shoot(&mut world, 1500, 100, &collision).is_none());
        assert_eq!(world.health(), 100);
        assert_eq!(world.current_time, Some(2100));
        assert!(shoot(&mut world, 2100, 33, &collision).is_some());
        assert_eq!(world.health(), 67);
    }

    #[test]
    fn first_band_includes_its_boundary_and_does_not_invent_distant_damage() {
        let collision = collision(None);
        let mut config = target();
        config.origin[0] = 510.0;
        let mut world = CombatWorld::new(config.clone());
        assert!(shoot(&mut world, 1, 33, &collision).is_some());
        config.origin[0] = 510.1;
        let mut outside = CombatWorld::new(config);
        assert!(shoot(&mut outside, 1, 33, &collision).is_none());
        assert_eq!(outside.health(), 100);
    }

    #[test]
    fn nearest_world_geometry_occludes_target_but_geometry_behind_it_does_not() {
        // The current mask includes 0x8000, even though the player mask does not.
        let before = collision(Some((50.0, 0x8000)));
        let after = collision(Some((150.0, 1)));
        let inside = collision(Some((0.0, 1)));
        let mut world = CombatWorld::new(target());
        assert!(shoot(&mut world, 1, 33, &before).is_none());
        assert!(shoot(&mut world, 2, 33, &inside).is_none());
        assert_eq!(world.health(), 100);
        assert!(shoot(&mut world, 3, 33, &after).is_some());
        assert_eq!(world.health(), 67);
    }

    #[test]
    fn translated_model_bounds_and_yaw_are_used_for_target_hits() {
        let collision = collision(None);
        let mut config = target();
        config.origin = [0.0, 100.0, 30.0];
        let mut world = CombatWorld::new(config);
        assert!(shoot(&mut world, 1, 33, &collision).is_none());
        assert!(
            world
                .fire(2, [0.0, 0.0, 30.0], [0, 16384, 0], ballistics(), &collision)
                .feedback
                .is_some()
        );
        assert_eq!(world.health(), 67);
        assert_eq!(world.snapshot().origin, [0.0, 100.0, 30.0]);
        assert_eq!(world.snapshot().entity_number, 19);
    }

    #[test]
    fn wall_shots_emit_impacts_even_when_the_target_is_missed_or_dead() {
        let wall = collision(Some((150.0, 1)));
        let mut world = CombatWorld::new(target());
        let miss = world.fire(1, [0.0, 50.0, 30.0], [0; 3], ballistics(), &wall);
        assert!(miss.feedback.is_none());
        let impact = miss.impact.unwrap();
        assert_eq!(impact.origin, [145.0, 50.0, 30.0]);
        assert_eq!(impact.normal, [-1.0, 0.0, 0.0]);
        assert!(
            world
                .fire(1, [0.0, 50.0, 30.0], [0; 3], ballistics(), &wall)
                .impact
                .is_none()
        );
        let kill = world.fire(
            2,
            [0.0, 0.0, 30.0],
            [0; 3],
            Ballistics {
                damage: 100,
                ..ballistics()
            },
            &wall,
        );
        assert!(kill.feedback.is_some());
        assert!(
            kill.impact.is_none(),
            "live target blocks the wall behind it"
        );
        let dead = world.fire(3, [0.0, 0.0, 30.0], [0; 3], ballistics(), &wall);
        assert!(dead.feedback.is_none());
        assert_eq!(dead.impact.unwrap().origin, [145.0, 0.0, 30.0]);
        let before = collision(Some((50.0, 1)));
        let mut occluded = CombatWorld::new(target());
        let blocked = occluded.fire(1, [0.0, 0.0, 30.0], [0; 3], ballistics(), &before);
        assert!(blocked.feedback.is_none());
        assert_eq!(blocked.impact.unwrap().origin, [45.0, 0.0, 30.0]);
        assert_eq!(occluded.health(), 100);
    }

    #[test]
    fn world_impacts_work_without_a_target_and_do_not_use_movement_skin() {
        let wall = collision(Some((50.0, 1)));
        let hit = world_impact([0.0, 0.0, 30.0], [0; 3], ballistics(), &wall).unwrap();
        assert_eq!(hit.origin, [45.0, 0.0, 30.0]);
        assert!(world_impact([0.0, 0.0, 30.0], [0, 32768, 0], ballistics(), &wall).is_none());
        assert!(world_impact([50.0, 0.0, 30.0], [0; 3], ballistics(), &wall).is_none());
    }

    #[test]
    fn enlarged_targets_use_the_same_scaled_bounds_as_the_visible_model() {
        let mut config = target();
        config.scale = 2.0;
        let mut world = CombatWorld::new(config);
        let result = world.fire(1, [0.0, 15.0, 30.0], [0; 3], ballistics(), &collision(None));
        assert!(result.feedback.is_some());
        assert_eq!(world.health(), 67);
        assert_eq!(world.snapshot().scale, 2.0);
    }

    #[test]
    fn physical_materials_reach_impacts_and_noimpact_surfaces_still_block_bullets() {
        for flags in [17 << 20, 4, 16, 32, 128, 31 << 20] {
            let export = json!({
                "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
                "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[],
                "triangles":[{"vertices":[[-100,-100,0],[100,-100,0],[0,100,0]],"contents":1,"surface_flags":flags}]
            });
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
            let c = Collision::load(file.path()).unwrap();
            let hit = world_impact([0.0, 0.0, 30.0], [16384, 0, 0], ballistics(), &c);
            if flags == 17 << 20 {
                assert_eq!(hit.unwrap().surface, 17);
            } else {
                assert!(hit.is_none(), "flags {flags:#x}");
                assert_ne!(c.raycast([0.0, 0.0, 30.0], [0.0, 0.0, -30.0]).contents, 0);
            }
        }
    }
}
