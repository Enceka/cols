//! One server-owned training prop with coarse model bounds and real world occlusion.
//!
//! WeaponDef +0xbb4.. holds six damage stages and +0xbcc.. their ranges; damage
//! is interpolated between the used stages. With a captured static NPC pose,
//! the coarse bounds only gate a body-region ray (see game_hit_location).
//! Authorised, de-duplicated weapon shots are supplied by the player simulation.
use anyhow::{Result, ensure};
use codol_protocol::game_target::TargetSnapshot;
use serde::Deserialize;

use crate::game_hit_location::{HitLocation, HitRegions};
use crate::game_motion::Collision;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct WorldImpact {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub surface: u8,
}

#[derive(Default, Debug)]
pub struct ShotOutcome {
    pub feedback: Option<String>,
    /// World hits of the rays that missed the target (one per pellet).
    pub impacts: Vec<WorldImpact>,
    /// Body region of a damaging hit, when a pose capture is loaded.
    pub location: Option<HitLocation>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Actor {
    pub client: u8,
    pub team: u8,
    pub origin: [f32; 3],
    pub height: f32,
    pub yaw: f32,
}

#[derive(Default)]
pub(crate) struct ActorShot {
    pub damage: Vec<(u8, u16, bool)>,
    pub impacts: Vec<WorldImpact>,
    /// Populated only after the match applies damage and protection rules.
    pub committed_feedback: Option<codol_protocol::game_hit_feedback::HitKind>,
}
pub(crate) struct ActorScene<'a> {
    pub actors: &'a [Actor],
    pub regions: Option<&'a HitRegions>,
    pub collision: &'a Collision,
}

/// Each pellet stops at the closest body or world face. A teammate blocks
/// the bullet without receiving friendly-fire damage.
pub(crate) fn fire_at_actors(
    eye: [f32; 3],
    angles: [u16; 3],
    time: u32,
    ballistics: Ballistics,
    shooter: (u8, u8),
    scene: ActorScene<'_>,
) -> ActorShot {
    let ActorScene {
        actors,
        regions,
        collision,
    } = scene;
    let mut outcome = ActorShot::default();
    if !ballistics.valid() || eye.iter().any(|v| !v.is_finite()) {
        return outcome;
    }
    for angles in ballistics.rays(angles, time) {
        let direction = forward(angles);
        let end = std::array::from_fn(|i| eye[i] + direction[i] * ballistics.range);
        let mut nearest: Option<(Actor, f32, Option<HitLocation>)> = None;
        for actor in actors.iter().filter(|a| a.client != shooter.0) {
            let min = [
                actor.origin[0] - 15.0,
                actor.origin[1] - 15.0,
                actor.origin[2],
            ];
            let max = [
                actor.origin[0] + 15.0,
                actor.origin[1] + 15.0,
                actor.origin[2] + actor.height,
            ];
            let Some(coarse) = ray_aabb(eye, end, min, max) else {
                continue;
            };
            let (fraction, location) = if let Some(regions) = regions.filter(|_| actor.client != 0)
            {
                let Some(hit) = regions.classify(eye, end, actor.origin, actor.yaw) else {
                    continue;
                };
                (hit.fraction, Some(hit.location))
            } else {
                (coarse, None)
            };
            if nearest.is_none_or(|(_, previous, _)| fraction < previous) {
                nearest = Some((*actor, fraction, location));
            }
        }
        let world = collision.shot_raycast(eye, end, nearest.map_or(1.0, |(_, f, _)| f));
        if world.start_solid || world.all_solid {
            continue;
        }
        let nearest = nearest.filter(|(_, f, _)| world.contents == 0 || *f < world.fraction);
        if let Some((actor, fraction, location)) = nearest {
            if actor.team != shooter.1 {
                let base = ballistics
                    .damage_at_distance(fraction * ballistics.range)
                    .unwrap_or(0);
                let damage = ballistics.damage_at(base, location);
                if damage != 0 {
                    let headshot = location.is_some_and(HitLocation::is_head_region);
                    if let Some(previous) = outcome.damage.iter_mut().find(|p| p.0 == actor.client)
                    {
                        previous.1 = previous.1.saturating_add(damage);
                        previous.2 |= headshot;
                    } else {
                        outcome.damage.push((actor.client, damage, headshot));
                    }
                }
            }
        } else if let Some(impact) = world_impact(eye, angles, ballistics, collision) {
            outcome.impacts.push(impact);
        }
    }
    outcome
}

impl ShotOutcome {
    pub fn headshot(&self) -> bool {
        self.location.is_some_and(HitLocation::is_head_region)
    }
}

/// Default held-gun knife geometry:0x4b42f0 uses a64-unit forward endpoint
/// and the ordered center/four-corner offsets at0xc66a54,10 units on each
/// view axis (0x42da50/0x42da80). Actor bounds remain the server's coarse
/// bodies; original bone traces and the separate knife-table rows are not
/// represented by these defaults.
pub(crate) fn melee_at_actors(
    eye: [f32; 3],
    angles: [u16; 3],
    damage: u16,
    shooter: (u8, u8),
    scene: ActorScene<'_>,
) -> ActorShot {
    let mut outcome = ActorShot::default();
    if damage == 0 || eye.iter().any(|n| !n.is_finite()) {
        return outcome;
    }
    let [forward, right, up] = view_axes(angles);
    for [horizontal, vertical] in [
        [0.0, 0.0],
        [1.0, 1.0],
        [-1.0, 1.0],
        [1.0, -1.0],
        [-1.0, -1.0],
    ] {
        let end = std::array::from_fn(|i| {
            eye[i] + 64.0 * forward[i] + 10.0 * horizontal * right[i] + 10.0 * vertical * up[i]
        });
        let world = scene.collision.raycast(eye, end);
        if world.start_solid || world.all_solid {
            return outcome;
        }
        let hit = scene
            .actors
            .iter()
            .filter(|a| a.client != shooter.0)
            .filter_map(|a| actor_fraction(eye, end, a).map(|fraction| (a, fraction)))
            .filter(|(_, fraction)| world.contents == 0 || *fraction < world.fraction)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if hit.is_none()
            && let Some(piece) = world.glass_piece
        {
            scene.collision.shatter_glass(piece);
            return outcome;
        }
        if let Some((actor, _)) = hit {
            if actor.team != shooter.1 {
                outcome.damage.push((actor.client, damage, false));
            }
            return outcome;
        }
    }
    outcome
}

fn actor_fraction(start: [f32; 3], end: [f32; 3], actor: &Actor) -> Option<f32> {
    ray_aabb(
        start,
        end,
        [
            actor.origin[0] - 15.0,
            actor.origin[1] - 15.0,
            actor.origin[2],
        ],
        [
            actor.origin[0] + 15.0,
            actor.origin[1] + 15.0,
            actor.origin[2] + actor.height,
        ],
    )
}

/// Validate a native approach against a real visible body along its hint yaw.
/// 0x50a640 caps the default knife-less assist at96. The screen region in
/// 0x50a150 has virtual half-sizes320/240; combining its FOV scaling with
/// projection gives tangents tan(cg_fov/2)/2 and3*tan(cg_fov/2)/8. Usercmd
/// does not carry cg_fov, so use the native allowed maximum90 (0x43f390),
/// retaining endpoint, world and nearest-body checks for every accepted hint.
pub(crate) fn melee_approach_at_actors(
    eye: [f32; 3],
    angles: [u16; 3],
    approach: crate::game_player::MeleeApproach,
    shooter: (u8, u8),
    scene: ActorScene<'_>,
) -> Option<Actor> {
    if !(1..=96).contains(&approach.distance) || eye.iter().any(|n| !n.is_finite()) {
        return None;
    }
    let yaw = f32::from(approach.yaw as i16) * (std::f32::consts::TAU / 65536.0);
    let distance = f32::from(approach.distance);
    let endpoint = [eye[0] + distance * yaw.cos(), eye[1] + distance * yaw.sin()];
    let end = [
        eye[0] + (distance + 15.0) * yaw.cos(),
        eye[1] + (distance + 15.0) * yaw.sin(),
        eye[2],
    ];
    let world = scene.collision.raycast(eye, end);
    if world.start_solid || world.all_solid {
        return None;
    }
    let (actor, _) = scene
        .actors
        .iter()
        .filter(|a| a.client != shooter.0)
        .filter_map(|a| actor_fraction(eye, end, a).map(|fraction| (a, fraction)))
        .filter(|(_, fraction)| world.contents == 0 || *fraction < world.fraction)
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    if actor.team == shooter.1
        || (endpoint[0] - actor.origin[0]).hypot(endpoint[1] - actor.origin[1]) > 15.0
    {
        return None;
    }
    let [forward, right, up] = view_axes(angles);
    // Native0x50a9d0 projects all eight actor-bound corners. Check overlap
    // with the largest legal assist frustum in world space, avoiding any
    // trust in client-provided projected bounds or its predicted position.
    let center = [
        actor.origin[0] - eye[0],
        actor.origin[1] - eye[1],
        actor.origin[2] + actor.height * 0.5 - eye[2],
    ];
    let half = [15.0, 15.0, actor.height * 0.5];
    for normal in [
        forward,
        std::array::from_fn(|i| 0.5 * forward[i] + right[i]),
        std::array::from_fn(|i| 0.5 * forward[i] - right[i]),
        std::array::from_fn(|i| 0.375 * forward[i] + up[i]),
        std::array::from_fn(|i| 0.375 * forward[i] - up[i]),
    ] {
        let support: f32 = (0..3)
            .map(|i| center[i] * normal[i] + half[i] * normal[i].abs())
            .sum();
        if support <= 0.0 {
            return None;
        }
    }
    Some(*actor)
}

fn view_axes(angles: [u16; 3]) -> [[f32; 3]; 3] {
    let radians = std::f64::consts::TAU / 65536.0;
    let (sp, cp) = (f64::from(angles[0]) * radians).sin_cos();
    let (sy, cy) = (f64::from(angles[1]) * radians).sin_cos();
    let (sr, cr) = (f64::from(angles[2]) * radians).sin_cos();
    [
        [cp * cy, cp * sy, -sp],
        [-sr * sp * cy + cr * sy, -sr * sp * sy - cr * cy, -sr * cp],
        [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp],
    ]
    .map(|a| a.map(|n| n as f32))
}

/// A captured body pose placed at the target's current origin and yaw.
struct BodyPose<'a> {
    regions: &'a HitRegions,
    yaw_degrees: f32,
}

/// Trace ordinary bullets even without a damageable training prop. A live
/// prop stops the ray, including outside the currently supported damage band.
fn trace_shot(
    eye: [f32; 3],
    angles: [u16; 3],
    range: f32,
    collision: &Collision,
    target: Option<(&TargetConfig, Option<BodyPose<'_>>)>,
    break_glass: bool,
) -> (Option<(f32, Option<HitLocation>)>, Option<WorldImpact>) {
    let direction = forward(angles);
    let end = std::array::from_fn(|i| eye[i] + direction[i] * range);
    let target_hit = target.and_then(|(config, pose)| {
        let min = std::array::from_fn(|i| config.origin[i] + config.bounds_min[i] * config.scale);
        let max = std::array::from_fn(|i| config.origin[i] + config.bounds_max[i] * config.scale);
        let fraction = ray_aabb(eye, end, min, max)?;
        match pose {
            // Inside the coarse bounds but between body regions, the bullet
            // continues to the world instead of damaging the target.
            Some(pose) => pose
                .regions
                .classify(eye, end, config.origin, pose.yaw_degrees)
                .map(|hit| (hit.fraction, Some(hit.location))),
            None => Some((fraction, None)),
        }
    });
    let world = if break_glass {
        collision.shot_raycast(eye, end, target_hit.map_or(1.0, |hit| hit.0))
    } else {
        collision.raycast(eye, end)
    };
    if world.start_solid || world.all_solid {
        return (None, None);
    }
    if let Some(hit) = target_hit
        && (world.contents == 0 || hit.0 < world.fraction)
    {
        return (Some(hit), None);
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
    trace_shot(eye, angles, ballistics.range, collision, None, false).1
}

/// World impacts of every ray of a shot that has no damageable target.
pub fn world_impacts(
    eye: [f32; 3],
    angles: [u16; 3],
    time: u32,
    ballistics: Ballistics,
    collision: &Collision,
) -> Vec<WorldImpact> {
    if !ballistics.valid() || eye.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0) {
        return Vec::new();
    }
    ballistics
        .rays(angles, time)
        .into_iter()
        .filter_map(|ray| trace_shot(eye, ray, ballistics.range, collision, None, true).1)
        .collect()
}

/// Coarse player bounds use the recovered collision radius and stance height.
/// Returns the distance damage; the closest world face wins.
pub(crate) fn hits_player(
    eye: [f32; 3],
    angles: [u16; 3],
    ballistics: Ballistics,
    origin: [f32; 3],
    height: f32,
    collision: &Collision,
) -> Option<u16> {
    let direction = forward(angles);
    let end = std::array::from_fn(|i| eye[i] + direction[i] * ballistics.range);
    let min = [origin[0] - 15.0, origin[1] - 15.0, origin[2]];
    let max = [origin[0] + 15.0, origin[1] + 15.0, origin[2] + height];
    let fraction = ray_aabb(eye, end, min, max)?;
    let damage = ballistics.damage_at_distance(fraction * ballistics.range)?;
    let world = collision.raycast(eye, end);
    (!world.start_solid && !world.all_solid && (world.contents == 0 || world.fraction > fraction))
        .then_some(damage)
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ballistics {
    pub damage: u16,
    /// Full world ray length, in world units; ordinary target traces use 8192.
    pub range: f32,
    /// First damage band, in world units. No later falloff is invented.
    pub near_range: f32,
    /// WeaponDef+0xbf0, 20 floats in the current hit-location enum order.
    /// Inferred from consistent resource values, not a named field table.
    #[serde(default)]
    pub location_multipliers: Option<[f32; 20]>,
    /// Used (range, damage) stages in increasing range, from the WeaponDef.
    /// Without it, damage stops after `near_range` as before.
    #[serde(default)]
    pub falloff: Option<DamageFalloff>,
    /// WeaponDef+0x6c8 rays per shot (shotgun pellets); 0 means one.
    #[serde(default)]
    pub pellets: u8,
    /// Pellet cone half-angle in degrees, from the standing hip spread.
    #[serde(default)]
    pub spread_degrees: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DamageFalloff {
    points: [(f32, u16); 6],
    len: u8,
}

impl DamageFalloff {
    /// Stages with a zero range are unused; ranges must then increase.
    pub fn from_stages(damage: [u32; 6], range: [f32; 6]) -> Option<Self> {
        let mut points = [(0.0, 0); 6];
        let mut len = 0usize;
        for (damage, range) in damage.into_iter().zip(range) {
            if range == 0.0 {
                continue;
            }
            let previous = len.checked_sub(1).map(|i| points[i].0);
            if !(0.0..=65536.0).contains(&range)
                || damage > u32::from(u16::MAX)
                || previous.is_some_and(|p| range <= p)
            {
                return None;
            }
            points[len] = (range, damage as u16);
            len += 1;
        }
        (len > 0).then_some(Self {
            points,
            len: len as u8,
        })
    }

    /// First stage up to its range, linear between stages, last beyond.
    pub fn damage(self, distance: f32) -> u16 {
        let points = &self.points[..usize::from(self.len)];
        if distance <= points[0].0 {
            return points[0].1;
        }
        for pair in points.windows(2) {
            let ((near, a), (far, b)) = (pair[0], pair[1]);
            if distance <= far {
                let t = (distance - near) / (far - near);
                return (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u16;
            }
        }
        points[points.len() - 1].1
    }
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
            && self.pellets <= 32
            && self.spread_degrees.is_finite()
            && (0.0..=89.0).contains(&self.spread_degrees)
            && self
                .location_multipliers
                .is_none_or(|m| m.iter().all(|v| v.is_finite() && (0.0..=10.0).contains(v)))
    }

    /// View angles of each ray. Pellets are spread uniformly over the cone;
    /// the pattern is derived from the shot time, never from client input.
    pub fn rays(self, angles: [u16; 3], seed: u32) -> Vec<[u16; 3]> {
        let count = self.pellets.max(1);
        if self.spread_degrees <= 0.0 {
            return vec![angles; usize::from(count)];
        }
        let mut state = seed.wrapping_mul(0x9e37_79b9) | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            f64::from(state) / f64::from(u32::MAX)
        };
        let direction = forward(angles).map(f64::from);
        let yaw = f64::from(angles[1]) * std::f64::consts::TAU / 65536.0;
        let right = [-yaw.sin(), yaw.cos(), 0.0];
        let up = [
            direction[1] * right[2] - direction[2] * right[1],
            direction[2] * right[0] - direction[0] * right[2],
            direction[0] * right[1] - direction[1] * right[0],
        ];
        let units = 65536.0 / std::f64::consts::TAU;
        (0..count)
            .map(|_| {
                let radius = f64::from(self.spread_degrees).to_radians().tan() * next();
                let theta = next() * std::f64::consts::TAU;
                let ray: [f64; 3] = std::array::from_fn(|i| {
                    direction[i] + radius * (theta.cos() * right[i] + theta.sin() * up[i])
                });
                let pitch = (-ray[2].atan2(ray[0].hypot(ray[1])) * units).round() as i32;
                let yaw = (ray[1].atan2(ray[0]) * units).round() as i32;
                [pitch as u16, yaw as u16, angles[2]]
            })
            .collect()
    }

    /// Base damage at a distance, or None outside a weapon without stages.
    pub fn damage_at_distance(self, distance: f32) -> Option<u16> {
        if !distance.is_finite() || distance < 0.0 || distance > self.range {
            return None;
        }
        match self.falloff {
            Some(falloff) => Some(falloff.damage(distance)),
            None => (distance <= self.near_range).then_some(self.damage),
        }
    }

    /// Integer damage after the location multiplier; the fraction truncates.
    pub fn damage_at(self, damage: u16, location: Option<HitLocation>) -> u16 {
        match (location, self.location_multipliers) {
            (Some(location), Some(multipliers)) => {
                (f32::from(damage) * multipliers[location as usize]) as u16
            }
            _ => damage,
        }
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
            matches!(self.entity_number, 1 | 19),
            "training target must use reserved enemy1 or prop19"
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
    regions: Option<Arc<HitRegions>>,
    yaw_degrees: f32,
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
            regions: None,
            yaw_degrees: 0.0,
            death_time: None,
            current_time: None,
            last_shot_time: None,
            life_start_time: None,
        }
    }

    pub fn health(&self) -> u16 {
        self.health
    }
    pub(crate) fn move_target(&mut self, origin: [f32; 3]) {
        if self.health > 0 {
            self.config.origin = origin;
        }
    }
    pub(crate) fn turn_target(&mut self, yaw_degrees: f32) {
        if self.health > 0 && yaw_degrees.is_finite() {
            self.yaw_degrees = yaw_degrees;
        }
    }
    /// Only the reserved humanoid enemy uses the captured body pose.
    pub(crate) fn set_hit_regions(&mut self, regions: Arc<HitRegions>) {
        if self.config.entity_number == 1 {
            self.regions = Some(regions);
        }
    }
    pub(crate) fn has_hit_regions(&self) -> bool {
        self.regions.is_some()
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
        let mut outcome = ShotOutcome::default();
        let mut damage = 0u16;
        let mut pellets_hit = 0u8;
        let mut nearest_hit = f32::MAX;
        for ray in ballistics.rays(angles, time) {
            let pose = self.regions.as_deref().map(|regions| BodyPose {
                regions,
                yaw_degrees: self.yaw_degrees,
            });
            let (hit, impact) = trace_shot(
                eye,
                ray,
                ballistics.range,
                collision,
                (self.health > 0).then_some((&self.config, pose)),
                true,
            );
            outcome.impacts.extend(impact);
            let Some((fraction, location)) = hit else {
                continue;
            };
            let Some(base) = ballistics.damage_at_distance(fraction * ballistics.range) else {
                continue;
            };
            let pellet = ballistics.damage_at(base, location);
            if pellet == 0 {
                continue;
            }
            damage = damage.saturating_add(pellet);
            pellets_hit += 1;
            nearest_hit = nearest_hit.min(fraction * ballistics.range);
            // Any head pellet makes the shot a headshot.
            if outcome.location.is_none_or(|l| !l.is_head_region()) {
                outcome.location = location;
            }
        }
        let location = outcome.location;
        if damage == 0 {
            return outcome;
        }
        tracing::info!(
            damage,
            pellets_hit,
            pellets = ballistics.pellets.max(1),
            distance = nearest_hit,
            "authoritative bullet damage"
        );
        self.health = self.health.saturating_sub(damage);
        let name = if self.config.entity_number == 1 {
            "Training enemy"
        } else {
            "Training target"
        };
        let part = location.map_or(String::new(), |l| format!(" ({})", l.name()));
        outcome.feedback = Some(if self.health == 0 {
            self.death_time = Some(effective_time);
            format!("{name} destroyed{part}")
        } else {
            format!("{name}: {} HP{part}", self.health)
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
            location_multipliers: None,
            falloff: None,
            pellets: 0,
            spread_degrees: 0.0,
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
    #[test]
    fn actor_bullets_break_front_glass_but_stop_at_teammates_and_permanent_windows() {
        let export = json!({
            "schema_version":1, "read_only":true,
            "client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp", "ncs_crc":0,
            "brushes":[
                {"center":[50,0,45],"half":[0.5,100,45],"contents":0x0800_0010,"planes":[],"glass_piece":0},
                {"center":[200,0,45],"half":[0.5,100,45],"contents":0x0800_0010,"planes":[],"glass_piece":1},
                {"center":[250,0,45],"half":[0.5,100,45],"contents":0x0800_0010,"planes":[]},
                {"center":[300,0,45],"half":[0.5,100,45],"contents":0x0800_0010,"planes":[],"glass_piece":2}
            ], "triangles":[]
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let collision = Collision::load(file.path()).unwrap();
        let actor = |client, team, x| Actor {
            client,
            team,
            origin: [x, 0.0, 0.0],
            height: 70.0,
            yaw: 0.0,
        };
        for (time, actors, expected_damage, pieces) in [
            (
                1000,
                vec![actor(1, 2, 100.0), actor(6, 1, 150.0)],
                false,
                vec![0],
            ),
            (2000, vec![actor(6, 1, 150.0)], true, vec![0]),
            (3000, vec![actor(6, 1, 350.0)], false, vec![0, 1]),
        ] {
            let outcome = fire_at_actors(
                [0.0, 0.0, 60.0],
                [0; 3],
                time,
                ballistics(),
                (0, 2),
                ActorScene {
                    actors: &actors,
                    regions: None,
                    collision: &collision,
                },
            );
            assert_eq!(!outcome.damage.is_empty(), expected_damage);
            assert_eq!(collision.destroyed_glass(), pieces);
        }
    }

    #[test]
    fn melee_glass_hit_consumes_the_strike_before_a_body_behind_it() {
        let export = json!({
            "schema_version":1, "read_only":true,
            "client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp", "ncs_crc":0,
            "brushes":[{"center":[30,0,45],"half":[0.5,100,45],"contents":0x0800_0010,"planes":[],"glass_piece":9}],
            "triangles":[]
        });
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        let collision = Collision::load(file.path()).unwrap();
        let actors = [Actor {
            client: 6,
            team: 1,
            origin: [55.0, 0.0, 0.0],
            height: 70.0,
            yaw: 0.0,
        }];
        let strike = || {
            melee_at_actors(
                [0.0, 0.0, 60.0],
                [0; 3],
                100,
                (0, 2),
                ActorScene {
                    actors: &actors,
                    regions: None,
                    collision: &collision,
                },
            )
        };
        assert!(strike().damage.is_empty());
        assert_eq!(collision.destroyed_glass(), vec![9]);
        assert_eq!(strike().damage, vec![(6, 100, false)]);
    }
    #[test]
    fn multiplayer_pellets_stop_at_teammates_and_the_nearest_enemy() {
        let collision = collision(None);
        let actor = |client, team, x| Actor {
            client,
            team,
            origin: [x, 0.0, 0.0],
            height: 70.0,
            yaw: 0.0,
        };
        let mut b = ballistics();
        b.pellets = 8;
        let eye = [0.0, 0.0, 60.0];
        let blocked = fire_at_actors(
            eye,
            [0; 3],
            1000,
            b,
            (0, 2),
            ActorScene {
                actors: &[actor(1, 2, 100.0), actor(6, 1, 200.0)],
                regions: None,
                collision: &collision,
            },
        );
        assert!(blocked.damage.is_empty());
        let hit = fire_at_actors(
            eye,
            [0; 3],
            1000,
            b,
            (0, 2),
            ActorScene {
                actors: &[actor(6, 1, 100.0), actor(7, 1, 200.0)],
                regions: None,
                collision: &collision,
            },
        );
        assert_eq!(hit.damage, vec![(6, 264, false)]);
    }
    #[test]
    fn melee_approach_frustum_has_native_range_angle_and_world_bounds() {
        let actor = |x, y, z| Actor {
            client: 1,
            team: 1,
            origin: [x, y, z],
            height: 70.0,
            yaw: 0.0,
        };
        let world = collision(None);
        let validate = |a: Actor, angles, collision: &Collision| {
            let yaw = (a.origin[1].atan2(a.origin[0]) * 65536.0 / std::f32::consts::TAU).round()
                as i32 as u16;
            let distance = a.origin[0].hypot(a.origin[1]) as u8;
            melee_approach_at_actors(
                [0.0, 0.0, 60.0],
                angles,
                crate::game_player::MeleeApproach { yaw, distance },
                (0, 2),
                ActorScene {
                    actors: &[a],
                    regions: None,
                    collision,
                },
            )
            .is_some()
        };
        assert!(validate(actor(75.0, 24.0, 0.0), [0; 3], &world));
        assert!(validate(actor(75.0, -24.0, 0.0), [0; 3], &world));
        assert!(!validate(actor(40.0, 60.0, 0.0), [0; 3], &world));
        assert!(!validate(actor(75.0, 24.0, 0.0), [0, 32768, 0], &world));
        assert!(!validate(actor(97.0, 0.0, 0.0), [0; 3], &world));
        assert!(!validate(actor(75.0, 24.0, 0.0), [16384, 0, 0], &world));
        assert!(!validate(
            actor(75.0, 24.0, 0.0),
            [0; 3],
            &collision(Some((30.0, 1)))
        ));
    }
    #[test]
    fn melee_fan_reaches_side_bodies_without_extending_forward_range() {
        let world = collision(None);
        let actor = |x, y, z, height| Actor {
            client: 1,
            team: 1,
            origin: [x, y, z],
            height,
            yaw: 0.0,
        };
        for (body, hit) in [
            (actor(60.0, 20.0, 0.0, 70.0), true),
            (actor(60.0, -20.0, 0.0, 70.0), true),
            (actor(60.0, 26.0, 0.0, 70.0), false),
            (actor(80.0, 0.0, 0.0, 70.0), false),
            (actor(60.0, 0.0, 64.0, 6.0), true),
            (actor(60.0, 0.0, 71.0, 6.0), false),
        ] {
            let actors = [body];
            let result = melee_at_actors(
                [0.0, 0.0, 60.0],
                [0; 3],
                135,
                (0, 2),
                ActorScene {
                    actors: &actors,
                    regions: None,
                    collision: &world,
                },
            );
            assert_eq!(!result.damage.is_empty(), hit, "{body:?}");
            assert!(result.impacts.is_empty());
        }
    }
    #[test]
    fn melee_fan_preserves_center_priority_corner_order_and_teammate_blocking() {
        let world = collision(None);
        let mut actors = [
            Actor {
                client: 1,
                team: 1,
                origin: [60.0, 20.0, 0.0],
                height: 70.0,
                yaw: 0.0,
            },
            Actor {
                client: 2,
                team: 1,
                origin: [60.0, -20.0, 0.0],
                height: 70.0,
                yaw: 0.0,
            },
            Actor {
                client: 3,
                team: 2,
                origin: [100.0, 0.0, 0.0],
                height: 70.0,
                yaw: 0.0,
            },
        ];
        let attack = |actors: &[Actor]| {
            melee_at_actors(
                [0.0, 0.0, 60.0],
                [0; 3],
                135,
                (0, 2),
                ActorScene {
                    actors,
                    regions: None,
                    collision: &world,
                },
            )
        };
        assert_eq!(attack(&actors).damage, vec![(2, 135, false)]);
        actors[2].origin[0] = 40.0;
        assert!(attack(&actors).damage.is_empty());
        actors[2].team = 1;
        assert_eq!(attack(&actors).damage, vec![(3, 135, false)]);
    }
    #[test]
    fn melee_offsets_follow_rotated_pitch_yaw_and_roll() {
        let world = collision(None);
        let actors = [Actor {
            client: 1,
            team: 1,
            origin: [-20.0, 60.0, 0.0],
            height: 70.0,
            yaw: 0.0,
        }];
        let hit = melee_at_actors(
            [0.0, 0.0, 60.0],
            [0, 16384, 16384],
            135,
            (0, 2),
            ActorScene {
                actors: &actors,
                regions: None,
                collision: &world,
            },
        );
        assert_eq!(hit.damage, vec![(1, 135, false)]);
        let low = [Actor {
            client: 2,
            team: 1,
            origin: [-20.0, 45.0, 0.0],
            height: 15.0,
            yaw: 0.0,
        }];
        let hit = melee_at_actors(
            [0.0, 0.0, 60.0],
            [8192, 16384, 0],
            135,
            (0, 2),
            ActorScene {
                actors: &low,
                regions: None,
                collision: &world,
            },
        );
        assert_eq!(hit.damage, vec![(2, 135, false)]);
    }
    #[test]
    fn melee_cannot_hit_through_a_world_face_or_create_a_bullet_impact() {
        let actor = Actor {
            client: 1,
            team: 1,
            origin: [60.0, 0.0, 0.0],
            height: 70.0,
            yaw: 0.0,
        };
        let actors = [actor];
        for wall in [Some((30.0, 1)), None] {
            let world = collision(wall);
            let hit = melee_at_actors(
                [0.0, 0.0, 60.0],
                [0; 3],
                135,
                (0, 2),
                ActorScene {
                    actors: &actors,
                    regions: None,
                    collision: &world,
                },
            );
            assert_eq!(hit.damage.is_empty(), wall.is_some());
            assert!(hit.impacts.is_empty());
        }
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

    fn humanoid() -> CombatWorld {
        let mut world = CombatWorld::new(TargetConfig {
            entity_number: 1,
            origin: [100.0, 0.0, 0.0],
            bounds_min: [-20.0, -20.0, 0.0],
            bounds_max: [20.0, 20.0, 80.0],
            ..target()
        });
        world.set_hit_regions(Arc::new(crate::game_hit_location::synthetic()));
        world
    }

    fn located() -> Ballistics {
        let mut multipliers = [1.0; 20];
        multipliers[HitLocation::Head as usize] = 1.2;
        multipliers[HitLocation::Gun as usize] = 0.0;
        Ballistics {
            location_multipliers: Some(multipliers),
            ..ballistics()
        }
    }

    /// Level shot from x=0 at height z toward the humanoid at x=100.
    fn shot_at(world: &mut CombatWorld, time: u32, z: f32, y: f32) -> ShotOutcome {
        let yaw = (y / 100.0).atan().to_degrees();
        let angles = [0, (yaw.rem_euclid(360.0) * 65536.0 / 360.0) as u16, 0];
        world.fire(time, [0.0, 0.0, z], angles, located(), &collision(None))
    }

    #[test]
    fn captured_body_regions_select_location_damage_and_headshots() {
        let mut world = humanoid();
        let head = shot_at(&mut world, 1, 70.0, 0.0);
        assert_eq!(head.location, Some(HitLocation::Head));
        assert!(head.headshot());
        assert_eq!(world.health(), 61); // 33 * 1.2 truncates to 39.
        let torso = shot_at(&mut world, 2, 40.0, 0.0);
        assert_eq!(torso.location, Some(HitLocation::TorsoUpper));
        assert!(!torso.headshot());
        assert_eq!(world.health(), 28);
        assert_eq!(located().damage_at(33, Some(HitLocation::Gun)), 0);
        assert_eq!(ballistics().damage_at(33, Some(HitLocation::Head)), 33);
    }

    #[test]
    fn a_ray_through_the_coarse_bounds_but_between_regions_reaches_the_world() {
        let mut world = humanoid();
        // Inside the 40-wide box, beside the 20-wide torso: no damage.
        let miss = shot_at(&mut world, 1, 25.0, 17.0);
        assert!(miss.feedback.is_none() && miss.location.is_none());
        assert_eq!(world.health(), 100);
        // Without a capture the same ray damages the coarse bounds.
        let mut coarse = CombatWorld::new(TargetConfig {
            entity_number: 1,
            origin: [100.0, 0.0, 0.0],
            bounds_min: [-20.0, -20.0, 0.0],
            bounds_max: [20.0, 20.0, 80.0],
            ..target()
        });
        let hit = shot_at(&mut coarse, 1, 25.0, 17.0);
        assert!(hit.feedback.is_some() && hit.location.is_none());
        // The static prop never takes the humanoid pose.
        let mut prop = CombatWorld::new(target());
        prop.set_hit_regions(Arc::new(crate::game_hit_location::synthetic()));
        assert!(!prop.has_hit_regions());
    }

    #[test]
    fn pellets_spread_inside_the_cone_and_sum_their_damage() {
        let shotgun = Ballistics {
            damage: 50,
            pellets: 8,
            spread_degrees: 8.0,
            ..ballistics()
        };
        let rays = shotgun.rays([0, 0, 0], 1234);
        assert_eq!(rays.len(), 8);
        assert_eq!(rays, shotgun.rays([0, 0, 0], 1234)); // Server-chosen, repeatable.
        assert_ne!(rays, shotgun.rays([0, 0, 0], 1235));
        let limit = (8.0 * 65536.0 / 360.0) as i32 + 1;
        for ray in &rays {
            let pitch = ray[0] as i16 as i32;
            let yaw = ray[1] as i16 as i32;
            assert!(pitch * pitch + yaw * yaw <= limit * limit);
        }
        assert_eq!(ballistics().rays([1, 2, 3], 9), vec![[1, 2, 3]]);
        // Point blank, every pellet lands on the 20-unit target box.
        let mut world = CombatWorld::new(TargetConfig {
            health: 1000,
            ..target()
        });
        world.config.origin = [100.0, 0.0, 30.0];
        world.config.bounds_min = [-10.0, -40.0, -40.0];
        world.config.bounds_max = [10.0, 40.0, 40.0];
        let outcome = world.fire(1, [0.0, 0.0, 30.0], [0; 3], shotgun, &collision(None));
        assert_eq!(world.health(), 600);
        assert!(outcome.impacts.is_empty());
    }

    #[test]
    fn captured_damage_stages_interpolate_and_keep_the_last_stage_beyond() {
        // m9_mp WeaponDef+0xbb4/+0xbcc from the current arsenal capture.
        let m9 = DamageFalloff::from_stages(
            [50, 45, 45, 33, 24, 19],
            [275.0, 276.0, 700.0, 701.0, 900.0, 1200.0],
        )
        .unwrap();
        for (distance, damage) in [
            (0.0, 50),
            (275.0, 50),
            (276.0, 45),
            (500.0, 45),
            (701.0, 33),
            (800.5, 29),
            (900.0, 24),
            (1050.0, 22),
            (1200.0, 19),
            (8000.0, 19),
        ] {
            assert_eq!(m9.damage(distance), damage, "{distance}");
        }
        // m4_mp leaves stages 3/4 unused: 33 to 1600 units, then 24.
        let m4 = DamageFalloff::from_stages(
            [33, 33, 33, 0, 0, 24],
            [500.0, 501.0, 1600.0, 0.0, 0.0, 1601.0],
        )
        .unwrap();
        assert_eq!((m4.damage(1200.0), m4.damage(3000.0)), (33, 24));
        // spas12_mp: 50 to 300, falling to 10 at 450 units.
        let spas = DamageFalloff::from_stages(
            [50, 0, 0, 10, 10, 10],
            [300.0, 0.0, 0.0, 450.0, 451.0, 650.0],
        )
        .unwrap();
        assert_eq!((spas.damage(375.0), spas.damage(1000.0)), (30, 10));
        for (damage, range) in [
            ([0; 6], [0.0; 6]),
            ([33; 6], [500.0, 400.0, 0.0, 0.0, 0.0, 0.0]),
            ([33; 6], [f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ([70_000, 0, 0, 0, 0, 0], [500.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        ] {
            assert!(DamageFalloff::from_stages(damage, range).is_none());
        }
        let staged = Ballistics {
            falloff: Some(m4),
            ..ballistics()
        };
        assert_eq!(staged.damage_at_distance(2000.0), Some(24));
        assert_eq!(ballistics().damage_at_distance(2000.0), None);
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
        let impact = miss.impacts[0];
        assert_eq!(impact.origin, [145.0, 50.0, 30.0]);
        assert_eq!(impact.normal, [-1.0, 0.0, 0.0]);
        assert!(
            world
                .fire(1, [0.0, 50.0, 30.0], [0; 3], ballistics(), &wall)
                .impacts
                .is_empty()
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
            kill.impacts.is_empty(),
            "live target blocks the wall behind it"
        );
        let dead = world.fire(3, [0.0, 0.0, 30.0], [0; 3], ballistics(), &wall);
        assert!(dead.feedback.is_none());
        assert_eq!(dead.impacts[0].origin, [145.0, 0.0, 30.0]);
        let before = collision(Some((50.0, 1)));
        let mut occluded = CombatWorld::new(target());
        let blocked = occluded.fire(1, [0.0, 0.0, 30.0], [0; 3], ballistics(), &before);
        assert!(blocked.feedback.is_none());
        assert_eq!(blocked.impacts[0].origin, [45.0, 0.0, 30.0]);
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
