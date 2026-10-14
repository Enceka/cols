//! Authority-owned reward devices. Resource bounds, weapon cadence and damage
//! stages are captured; placement distance, health and lifetime are local rules.
use crate::{
    game_combat::{Actor, ActorScene, Ballistics, DamageFalloff},
    game_killstreaks::{ModelBounds, RadarResources},
    game_motion::Collision,
    game_props::{Blast, Collider, Event},
};
use codol_protocol::game_props::PropSnapshot;
use std::collections::VecDeque;

pub(crate) const FIRST_ENTITY: u16 = 384;
pub(crate) const END_ENTITY: u16 = 448;
const LIFE_MSEC: u32 = 60_000;
const WRECK_MSEC: u32 = 5_000;

#[derive(Clone, Copy)]
pub(crate) struct SentryDefinition {
    model: u16,
    wreck_model: u16,
    bounds: ModelBounds,
    wreck_bounds: ModelBounds,
    pub weapon: u16,
    interval: u32,
    ballistics: Ballistics,
    explosion_fx: u16,
}
impl SentryDefinition {
    pub fn from_resources(r: &RadarResources) -> Option<Self> {
        let weapon = r.weapons.get("sentry_minigun_mp")?;
        let falloff = DamageFalloff::from_stages(weapon.damage_stages, weapon.damage_ranges)?;
        let range = weapon
            .damage_ranges
            .into_iter()
            .fold(0.0, f32::max)
            .min(8192.0);
        let ballistics = Ballistics {
            damage: falloff.damage(0.0),
            range,
            near_range: range,
            location_multipliers: None,
            falloff: Some(falloff),
            pellets: 1,
            spread_degrees: 0.0,
        };
        let bounds = *r.model_bounds.get("sentry_minigun")?;
        let wreck_bounds = *r.model_bounds.get("sentry_minigun_destroyed")?;
        if !ballistics.valid()
            || weapon.fire_interval_msec == 0
            || !bounds.valid()
            || !wreck_bounds.valid()
        {
            return None;
        }
        Some(Self {
            model: *r.models.get("sentry_minigun")?,
            wreck_model: *r.models.get("sentry_minigun_destroyed")?,
            bounds,
            wreck_bounds,
            weapon: weapon.weapon,
            interval: u32::from(weapon.fire_interval_msec),
            ballistics,
            explosion_fx: *r.effects.get("explosions/sentry_gun_explosion_instant")?,
        })
    }
}
pub(crate) struct Placement {
    entity: u16,
    origin: [f32; 3],
    angles: [u16; 3],
    definition: SentryDefinition,
}
#[derive(Debug)]
pub(crate) enum PlacementFailure {
    Resources,
    PoolExhausted,
    Ground,
    UnevenGround,
    Geometry,
    View,
}
struct Device {
    placement: Placement,
    owner: u8,
    team: u8,
    health: u16,
    end: u32,
    next_fire: u32,
    wreck_until: Option<u32>,
    events: codol_protocol::game_events::EventQueue,
}
pub(crate) struct DeviceShot {
    pub owner: u8,
    pub team: u8,
    pub entity: u16,
    pub eye: [f32; 3],
    pub angles: [u16; 3],
    pub weapon: u16,
    pub ballistics: Ballistics,
}
#[derive(Default)]
pub(crate) struct Deployables {
    devices: Vec<Device>,
    events: VecDeque<Event>,
}
impl Deployables {
    /// Validate all placement and allocation conditions before consuming stock.
    pub fn plan_sentry(
        &self,
        r: &RadarResources,
        player: Actor,
        collision: &Collision,
    ) -> Result<Placement, PlacementFailure> {
        let definition = SentryDefinition::from_resources(r).ok_or(PlacementFailure::Resources)?;
        let entity = (FIRST_ENTITY..END_ENTITY)
            .find(|id| !self.devices.iter().any(|d| d.placement.entity == *id))
            .ok_or(PlacementFailure::PoolExhausted)?;
        let yaw = player.yaw.to_radians();
        let mut origin = [
            player.origin[0] + 96.0 * yaw.cos(),
            player.origin[1] + 96.0 * yaw.sin(),
            player.origin[2],
        ];
        let angles = [0, (player.yaw / 360.0 * 65536.0).round() as i32 as u16, 0];
        let footprint = Collider::new(
            entity,
            origin,
            angles,
            definition.bounds.min,
            definition.bounds.max,
        );
        let mut ground = Vec::new();
        for x in [footprint.min[0], footprint.max[0]] {
            for y in [footprint.min[1], footprint.max[1]] {
                let top = [x, y, player.origin[2] + 48.0];
                let bottom = [x, y, player.origin[2] - 96.0];
                let trace = collision.ground_raycast(top, bottom);
                if trace.start_solid
                    || trace.all_solid
                    || trace.fraction >= 1.0
                    || trace.normal[2] < 0.7
                    || trace.prop_entity.is_some()
                {
                    return Err(PlacementFailure::Ground);
                }
                ground.push(top[2] + (bottom[2] - top[2]) * trace.fraction);
            }
        }
        let low = ground.iter().copied().fold(f32::INFINITY, f32::min);
        let high = ground.into_iter().fold(f32::NEG_INFINITY, f32::max);
        if high - low > 8.0 {
            return Err(PlacementFailure::UnevenGround);
        }
        origin[2] = high - definition.bounds.min[2] + 1.0;
        let shape = Collider::new(
            entity,
            origin,
            angles,
            definition.bounds.min,
            definition.bounds.max,
        );
        if !collision.bounds_clear(shape.min, shape.max) {
            return Err(PlacementFailure::Geometry);
        }
        let eye = [
            player.origin[0],
            player.origin[1],
            player.origin[2] + player.height - 10.0,
        ];
        let center = std::array::from_fn(|i| (shape.min[i] + shape.max[i]) * 0.5);
        let trace = collision.raycast(eye, center);
        if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
            return Err(PlacementFailure::View);
        }
        Ok(Placement {
            entity,
            origin,
            angles,
            definition,
        })
    }
    pub fn place(
        &mut self,
        placement: Placement,
        owner: u8,
        team: u8,
        now: u32,
        actors: &[Actor],
        collision: &Collision,
    ) -> bool {
        let shape = Collider::new(
            placement.entity,
            placement.origin,
            placement.angles,
            placement.definition.bounds.min,
            placement.definition.bounds.max,
        );
        if owner >= 12
            || !matches!(team, 1 | 2)
            || actors.iter().any(|a| {
                (0..3).all(|i| {
                    let (min, max) = if i == 2 {
                        (a.origin[i], a.origin[i] + a.height)
                    } else {
                        (a.origin[i] - 15.0, a.origin[i] + 15.0)
                    };
                    min < shape.max[i] && max > shape.min[i]
                })
            })
        {
            return false;
        }
        tracing::info!(entity=placement.entity,owner,team,now,origin=?placement.origin,"sentry deployed");
        self.devices.push(Device {
            placement,
            owner,
            team,
            health: 1000,
            end: now.saturating_add(LIFE_MSEC),
            next_fire: now + 500,
            wreck_until: None,
            events: Default::default(),
        });
        self.sync_collision(collision);
        true
    }
    pub fn snapshots(&self) -> Vec<PropSnapshot> {
        let mut snapshots = self
            .devices
            .iter()
            .map(|d| PropSnapshot {
                entity_number: d.placement.entity,
                model: if d.health == 0 {
                    d.placement.definition.wreck_model
                } else {
                    d.placement.definition.model
                },
                origin: d.placement.origin,
                angles: d.placement.angles,
                present: true,
                weapon: if d.health == 0 {
                    0
                } else {
                    d.placement.definition.weapon
                },
                events: d.events,
                // The GENERAL model path has native DObj evidence. TURRET
                // construction requires additional renderer state before use.
                turret: false,
            })
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|p| p.entity_number);
        snapshots
    }
    fn sync_collision(&self, collision: &Collision) {
        let shapes = self
            .devices
            .iter()
            .map(|d| {
                let b = if d.health == 0 {
                    d.placement.definition.wreck_bounds
                } else {
                    d.placement.definition.bounds
                };
                Collider::new(
                    d.placement.entity,
                    d.placement.origin,
                    d.placement.angles,
                    b.min,
                    b.max,
                )
            })
            .collect::<Vec<_>>();
        collision.replace_prop_range(FIRST_ENTITY..END_ENTITY, &shapes);
    }
    pub fn damage(
        &mut self,
        now: u32,
        entity: u16,
        damage: u16,
        attacker_team: u8,
        collision: &Collision,
    ) -> bool {
        let Some(d) = self
            .devices
            .iter_mut()
            .find(|d| d.placement.entity == entity && d.health > 0 && d.team != attacker_team)
        else {
            return false;
        };
        if damage == 0 {
            return false;
        }
        d.health = d.health.saturating_sub(damage);
        tracing::info!(
            now,
            entity,
            damage,
            health = d.health,
            "reward device damaged"
        );
        if d.health == 0 {
            d.wreck_until = Some(now.saturating_add(WRECK_MSEC));
            self.events.push_back(Event::Fx {
                selector: d.placement.definition.explosion_fx,
                origin: d.placement.origin,
            });
            self.sync_collision(collision);
        }
        true
    }
    pub fn radial(&mut self, now: u32, blast: Blast, attacker_team: u8, collision: &Collision) {
        let targets = self
            .devices
            .iter()
            .filter(|d| d.health > 0)
            .map(|d| {
                let b = d.placement.definition.bounds;
                (
                    d.placement.entity,
                    d.placement.origin,
                    (b.min[2] + b.max[2]) * 0.5,
                )
            })
            .collect::<Vec<_>>();
        for (id, origin, height) in targets {
            let damage =
                crate::game_props::radius_damage(blast, origin, height, Some(id), collision);
            self.damage(now, id, damage, attacker_team, collision);
        }
    }
    pub fn step(
        &mut self,
        now: u32,
        actors: &[Actor],
        undetectable: u16,
        emp_teams: u8,
        finished: bool,
        collision: &Collision,
    ) -> Vec<DeviceShot> {
        let old_len = self.devices.len();
        self.devices
            .retain(|d| !finished && now < d.end && d.wreck_until.is_none_or(|end| now < end));
        if old_len != self.devices.len() {
            self.sync_collision(collision);
        }
        let mut shots = Vec::new();
        for d in &mut self.devices {
            if d.health == 0 || now < d.next_fire {
                continue;
            }
            d.next_fire = now.saturating_add(d.placement.definition.interval);
            if emp_teams & (1 << d.team) != 0 {
                continue;
            }
            let eye = [
                d.placement.origin[0],
                d.placement.origin[1],
                d.placement.origin[2] + d.placement.definition.bounds.max[2] - 8.0,
            ];
            let base = f32::from(d.placement.angles[1]) * 360.0 / 65536.0;
            let target = actors
                .iter()
                .filter(|a| a.team != d.team && undetectable & (1 << a.client) == 0)
                .filter_map(|a| {
                    let delta = [
                        a.origin[0] - eye[0],
                        a.origin[1] - eye[1],
                        a.origin[2] + a.height * 0.5 - eye[2],
                    ];
                    let distance = delta.into_iter().map(|v| v * v).sum::<f32>().sqrt();
                    let yaw = delta[1].atan2(delta[0]).to_degrees();
                    let angle = (yaw - base + 180.0).rem_euclid(360.0) - 180.0;
                    if distance > d.placement.definition.ballistics.range || angle.abs() > 70.0 {
                        return None;
                    }
                    let center = [a.origin[0], a.origin[1], a.origin[2] + a.height * 0.5];
                    let trace =
                        collision.raycast_excluding_prop(eye, center, Some(d.placement.entity));
                    if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
                        return None;
                    }
                    let pitch = -delta[2].atan2(delta[0].hypot(delta[1])).to_degrees();
                    Some((distance, [angle_short(pitch), angle_short(yaw), 0]))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, angles)) = target {
                d.events
                    .push(70, 0)
                    .expect("native EV_FIRE_SENTRY has no parameter");
                shots.push(DeviceShot {
                    owner: d.owner,
                    team: d.team,
                    entity: d.placement.entity,
                    eye,
                    angles,
                    weapon: d.placement.definition.weapon,
                    ballistics: d.placement.definition.ballistics,
                });
            }
        }
        shots
    }
    pub fn drain(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
}
fn angle_short(degrees: f32) -> u16 {
    (degrees / 360.0 * 65536.0).round() as i32 as u16
}
pub(crate) fn resolve_shot(
    shot: &DeviceShot,
    now: u32,
    actors: &[Actor],
    collision: &Collision,
) -> crate::game_combat::ActorShot {
    crate::game_combat::fire_at_actors_with_source(
        shot.eye,
        shot.angles,
        now,
        shot.ballistics,
        (shot.owner, shot.team),
        ActorScene {
            actors,
            regions: None,
            collision,
        },
        Some(shot.entity),
    )
}

#[cfg(test)]
pub(crate) fn test_resources() -> RadarResources {
    serde_json::from_value(serde_json::json!({
        "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"sources_sha256":["0".repeat(64),"1".repeat(64)],"map":"test","ncs_crc":0,
        "radios":[{"native_id":16,"weapon":327,"switch":{"drop_msec":100,"raise_msec":100,"empty_drop_msec":100,"empty_raise_msec":100},"fire_interval_msec":800}],
        "models":{"sentry_minigun":125,"sentry_minigun_destroyed":126},
        "model_bounds":{"sentry_minigun":{"min":[-25,-25,0],"max":[25,25,60]},"sentry_minigun_destroyed":{"min":[-25,-25,0],"max":[25,25,30]}},
        "effects":{"explosions/sentry_gun_explosion_instant":78},
        "weapons":{"sentry_minigun_mp":{"weapon":347,"weapon_type":0,"weapon_class":8,"clip_key":347,"ammo_key":347,
        "switch":{"drop_msec":0,"raise_msec":0,"empty_drop_msec":0,"empty_raise_msec":0},"fire_interval_msec":50,"radius":0,"inner_damage":0,"outer_damage":0,
        "damage_stages":[20,0,0,0,0,20],"damage_ranges":[1200,0,0,0,0,1800]}}
    })).unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn collision(wall: bool) -> Collision {
        let f = tempfile::NamedTempFile::new().unwrap();
        let mut brushes = vec![
            serde_json::json!({"center":[0,0,-10],"half":[3000,3000,10],"contents":1,"planes":[]}),
        ];
        if wall {
            brushes.push(
                serde_json::json!({"center":[96,0,50],"half":[5,200,50],"contents":1,"planes":[]}),
            );
        }
        std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,
            "client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]})).unwrap()).unwrap();
        Collision::load(f.path()).unwrap()
    }
    fn actor(client: u8, team: u8, x: f32, y: f32) -> Actor {
        Actor {
            client,
            team,
            origin: [x, y, 0.0],
            height: 70.0,
            yaw: 0.0,
        }
    }
    fn placed(c: &Collision) -> Deployables {
        let mut d = Deployables::default();
        let owner = actor(0, 1, 0.0, 0.0);
        let plan = d.plan_sentry(&test_resources(), owner, c).unwrap();
        assert!(d.place(plan, 0, 1, 1000, &[owner], c));
        d
    }
    #[test]
    fn placement_requires_resources_world_clearance_ground_and_unoccupied_body() {
        let c = collision(false);
        let d = Deployables::default();
        let owner = actor(0, 1, 0.0, 0.0);
        let mut r = test_resources();
        r.model_bounds.clear();
        assert!(d.plan_sentry(&r, owner, &c).is_err());
        assert!(
            d.plan_sentry(&test_resources(), owner, &collision(true))
                .is_err()
        );
        assert!(
            d.plan_sentry(&test_resources(), actor(0, 1, 4000.0, 0.0), &c)
                .is_err()
        );
        let mut d = d;
        let plan = d.plan_sentry(&test_resources(), owner, &c).unwrap();
        assert!(!d.place(plan, 0, 1, 1000, &[owner, actor(6, 2, 96.0, 0.0)], &c));
        assert!(d.snapshots().is_empty());
    }
    #[test]
    fn device_uses_registered_cadence_damage_and_own_hull_exclusion() {
        let c = collision(false);
        let mut d = placed(&c);
        let actors = [actor(0, 1, 0.0, 0.0), actor(6, 2, 300.0, 0.0)];
        assert!(d.step(1499, &actors, 0, 0, false, &c).is_empty());
        let shots = d.step(1500, &actors, 0, 0, false, &c);
        assert_eq!(shots.len(), 1);
        assert_eq!(shots[0].weapon, 347);
        assert_eq!(
            resolve_shot(&shots[0], 1500, &actors, &c).damage,
            [(6, 20, false)]
        );
        assert!(d.step(1549, &actors, 0, 0, false, &c).is_empty());
        assert_eq!(d.step(1550, &actors, 0, 0, false, &c).len(), 1);
        let mut teammate = actors.to_vec();
        teammate.push(actor(1, 1, 220.0, 0.0));
        let shot = d.step(1600, &teammate, 0, 0, false, &c).pop().unwrap();
        assert!(resolve_shot(&shot, 1600, &teammate, &c).damage.is_empty());
    }
    #[test]
    fn enemy_detection_respects_front_arc_wall_cold_blooded_and_emp() {
        let c = collision(false);
        let mut d = placed(&c);
        let enemy = actor(6, 2, 300.0, 0.0);
        assert!(d.step(1500, &[enemy], 1 << 6, 0, false, &c).is_empty());
        assert!(d.step(1550, &[enemy], 0, 1 << 1, false, &c).is_empty());
        assert!(
            d.step(1600, &[actor(6, 2, -300.0, 0.0)], 0, 0, false, &c)
                .is_empty()
        );
        assert!(
            d.step(1650, &[actor(6, 2, 3000.0, 0.0)], 0, 0, false, &c)
                .is_empty()
        );
        c.replace_props(&[Collider::new(
            256,
            [220.0, 0.0, 0.0],
            [0; 3],
            [-10.0, -100.0, 0.0],
            [10.0, 100.0, 100.0],
        )]);
        assert!(d.step(1700, &[enemy], 0, 0, false, &c).is_empty());
        c.replace_props(&[]);
        assert_eq!(d.step(1750, &[enemy], 0, 0, false, &c).len(), 1);
    }
    #[test]
    fn map_collision_updates_preserve_devices_and_wrecks_expire_once() {
        let c = collision(false);
        let mut d = placed(&c);
        let ray = || c.raycast([0.0, 0.0, 30.0], [200.0, 0.0, 30.0]).prop_entity;
        assert_eq!(ray(), Some(384));
        c.replace_props(&[]);
        assert_eq!(ray(), Some(384));
        assert!(!d.damage(2000, 384, 1000, 1, &c));
        assert!(d.damage(2000, 384, 500, 2, &c));
        assert_eq!(d.snapshots()[0].model, 125);
        assert!(d.damage(2100, 384, 500, 2, &c));
        assert_eq!(d.snapshots()[0].model, 126);
        assert!(!d.damage(2150, 384, 500, 2, &c));
        assert_eq!(d.drain().len(), 1);
        assert!(d.drain().is_empty());
        d.step(7099, &[], 0, 0, false, &c);
        assert_eq!(d.snapshots().len(), 1);
        d.step(7100, &[], 0, 0, false, &c);
        assert!(d.snapshots().is_empty());
        assert_eq!(ray(), None);
    }
    #[test]
    fn splash_respects_occlusion_and_match_end_removes_collision() {
        let c = collision(false);
        let mut d = placed(&c);
        let blast = Blast {
            owner: 6,
            source_entity: None,
            weapon: 352,
            origin: [180.0, 0.0, 30.0],
            radius: 200.0,
            inner: 2000,
            outer: 1000,
        };
        c.replace_props(&[Collider::new(
            256,
            [145.0, 0.0, 0.0],
            [0; 3],
            [-5.0, -100.0, 0.0],
            [5.0, 100.0, 100.0],
        )]);
        d.radial(2000, blast, 2, &c);
        assert_eq!(d.devices[0].health, 1000);
        c.replace_props(&[]);
        d.radial(2100, blast, 2, &c);
        assert_eq!(d.devices[0].health, 0);
        d.step(2200, &[], 0, 0, true, &c);
        assert!(d.snapshots().is_empty());
        assert!(c.bounds_clear([71.0, -25.0, 1.0], [121.0, 25.0, 61.0]));
    }
    #[test]
    #[ignore = "requires matching ignored target-build collision and reward resources"]
    fn target_build_placement_grid() {
        let resources = std::env::var("CODOL_SENTRY_RESOURCES").unwrap();
        let r =
            RadarResources::load(std::path::Path::new(&resources), "mp_afghan", 523897474).unwrap();
        let collision = Collision::load(std::path::Path::new(
            &std::env::var("CODOL_SENTRY_COLLISION").unwrap(),
        ))
        .unwrap();
        let d = Deployables::default();
        let mut placements = Vec::new();
        for x in (4020..=4180).step_by(20) {
            for y in (2300..=2720).step_by(20) {
                let Ok(origin) =
                    crate::game_bot::grounded_spawn([x as f32, y as f32, 76.0], &collision)
                else {
                    continue;
                };
                for yaw in [0.0, 90.0, 180.0, 270.0] {
                    let player = Actor {
                        origin,
                        yaw,
                        ..actor(0, 2, 0.0, 0.0)
                    };
                    if let Ok(plan) = d.plan_sentry(&r, player, &collision) {
                        placements.push(
                            serde_json::json!({"player":origin,"yaw":yaw,"device":plan.origin}),
                        );
                    }
                }
            }
        }
        let original = Actor {
            origin: [4_001.201_2, 2_696.371_3, 30.63998],
            yaw: -47.373_047,
            ..actor(0, 2, 0.0, 0.0)
        };
        let failure = d
            .plan_sentry(&r, original, &collision)
            .err()
            .map(|e| format!("{e:?}"));
        let report = serde_json::json!({"placements":placements,"sample_failure":failure});
        std::fs::write(
            std::env::var("CODOL_SENTRY_GRID_OUTPUT").unwrap(),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        assert!(!placements.is_empty());
    }
}
