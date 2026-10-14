//! Authority-owned aircraft and their support identity. Model bounds are native
//! facts; route speed/height and health remain explicit local simulation rules.
use crate::{
    game_killstreaks::{ModelBounds, RadarResources, SupportFlight},
    game_motion::Collision,
    game_props::{Blast, Collider, Event},
};
use codol_protocol::game_props::PropSnapshot;
use std::collections::VecDeque;

pub(crate) const FIRST_ENTITY: u16 = 448;
pub(crate) const END_ENTITY: u16 = 464;
#[derive(Clone, Copy)]
struct Definition {
    model: u16,
    bounds: ModelBounds,
    blast_fx: u16,
    health: u16,
}
impl Definition {
    fn radar(r: &RadarResources, id: u8) -> Option<Self> {
        let (name, health) = match id {
            1 => ("vehicle_uav_static02_mp", 400),
            3 => ("vehicle_counter_uav_static02_mp", 500),
            _ => return None,
        };
        let bounds = *r.model_bounds.get(name)?;
        if !bounds.valid() {
            return None;
        }
        Some(Self {
            model: *r.models.get(name)?,
            bounds,
            health,
            blast_fx: *r.effects.get("explosions/aerial_explosion")?,
        })
    }
}
const RADAR_FLIGHT_MSEC: u32 = 30_000;
const RADAR_HEIGHT_ABOVE_ANCHOR: f32 = 3000.0;
const RADAR_HALF_LENGTH: f32 = 3000.0;

#[derive(Clone, Copy)]
struct StraightFlight {
    start: [f32; 3],
    end: [f32; 3],
}
impl StraightFlight {
    fn pose(self, elapsed: u32) -> ([f32; 3], [u16; 3]) {
        let fraction = elapsed.min(RADAR_FLIGHT_MSEC) as f32 / RADAR_FLIGHT_MSEC as f32;
        let delta: [f32; 3] = std::array::from_fn(|i| self.end[i] - self.start[i]);
        let origin = std::array::from_fn(|i| self.start[i] + delta[i] * fraction);
        let yaw =
            (delta[1].atan2(delta[0]) * 65536.0 / std::f32::consts::TAU).round() as i32 as u16;
        (origin, [0, yaw, 0])
    }
}
pub(crate) struct FlightPlan {
    entity: u16,
    definition: Definition,
    route: StraightFlight,
}
struct Flight {
    plan: FlightPlan,
    source: SupportFlight,
    health: u16,
    origin: [f32; 3],
    angles: [u16; 3],
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Target {
    pub entity: u16,
    pub source: SupportFlight,
    pub center: [f32; 3],
}
#[derive(Debug)]
pub(crate) enum Failure {
    Resources,
    Capacity,
    FlightPath,
}
#[derive(Default)]
pub(crate) struct Airspace {
    flights: Vec<Flight>,
    events: VecDeque<Event>,
}
impl Airspace {
    pub fn resources_present(r: &RadarResources, id: u8) -> bool {
        Definition::radar(r, id).is_some()
    }
    pub fn plan_radar(
        &self,
        r: &RadarResources,
        id: u8,
        origin: [f32; 3],
        collision: &Collision,
    ) -> Result<FlightPlan, Failure> {
        if origin.iter().any(|v| !v.is_finite() || v.abs() > 30000.0) {
            return Err(Failure::FlightPath);
        }
        let definition = Definition::radar(r, id).ok_or(Failure::Resources)?;
        let entity = (FIRST_ENTITY..END_ENTITY)
            .find(|id| !self.flights.iter().any(|f| f.plan.entity == *id))
            .ok_or(Failure::Capacity)?;
        // The map anchor is an airstrike datum, not a recovered UAV altitude.
        // High, level passes implement the required flight shape; the numeric
        // offset, speed and headings remain explicit compatibility parameters.
        let anchor = r.air_route.as_ref().map_or(origin, |r| r.center);
        let height = anchor[2] + RADAR_HEIGHT_ABOVE_ANCHOR;
        let lane = f32::from(entity - FIRST_ENTITY) * 450.0;
        for direction in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
            let center = [
                anchor[0] - direction[1] * lane,
                anchor[1] + direction[0] * lane,
                height,
            ];
            let route = StraightFlight {
                start: [
                    center[0] - direction[0] * RADAR_HALF_LENGTH,
                    center[1] - direction[1] * RADAR_HALF_LENGTH,
                    height,
                ],
                end: [
                    center[0] + direction[0] * RADAR_HALF_LENGTH,
                    center[1] + direction[1] * RADAR_HALF_LENGTH,
                    height,
                ],
            };
            let (start, angles) = route.pose(0);
            let (end, _) = route.pose(RADAR_FLIGHT_MSEC);
            let a = Collider::new(
                entity,
                start,
                angles,
                definition.bounds.min,
                definition.bounds.max,
            );
            let b = Collider::new(
                entity,
                end,
                angles,
                definition.bounds.min,
                definition.bounds.max,
            );
            let min = std::array::from_fn(|i| a.min[i].min(b.min[i]));
            let max = std::array::from_fn(|i| a.max[i].max(b.max[i]));
            // A constant orientation makes this conservative swept volume
            // cover the entire pass, rather than isolated sampled positions.
            if collision.bounds_clear(min, max) {
                return Ok(FlightPlan {
                    entity,
                    definition,
                    route,
                });
            }
        }
        Err(Failure::FlightPath)
    }
    pub fn launch(&mut self, plan: FlightPlan, source: SupportFlight, collision: &Collision) {
        let (origin, angles) = plan.route.pose(0);
        let health = plan.definition.health;
        tracing::info!(entity=plan.entity,owner=source.actor,id=source.id,team=source.team,origin=?origin,"radar aircraft launched");
        self.flights.push(Flight {
            plan,
            source,
            health,
            origin,
            angles,
        });
        self.sync_collision(collision);
    }
    pub fn advance(
        &mut self,
        now: u32,
        finished: bool,
        live_support: &[SupportFlight],
        collision: &Collision,
    ) {
        self.flights
            .retain(|f| !finished && f.source.end > now && live_support.contains(&f.source));
        for flight in &mut self.flights {
            let (origin, angles) = flight
                .plan
                .route
                .pose(now.saturating_sub(flight.source.start));
            flight.origin = origin;
            flight.angles = angles;
        }
        self.sync_collision(collision);
    }
    fn sync_collision(&self, c: &Collision) {
        let shapes = self
            .flights
            .iter()
            .map(|f| {
                Collider::new(
                    f.plan.entity,
                    f.origin,
                    f.angles,
                    f.plan.definition.bounds.min,
                    f.plan.definition.bounds.max,
                )
            })
            .collect::<Vec<_>>();
        c.replace_prop_range(FIRST_ENTITY..END_ENTITY, &shapes);
    }
    pub fn snapshots(&self) -> Vec<PropSnapshot> {
        let mut snapshots = self
            .flights
            .iter()
            .map(|f| PropSnapshot {
                entity_number: f.plan.entity,
                model: f.plan.definition.model,
                origin: f.origin,
                angles: f.angles,
                present: true,
                weapon: 0,
                events: Default::default(),
                turret: false,
            })
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|s| s.entity_number);
        snapshots
    }
    pub fn targets(&self) -> Vec<Target> {
        self.flights
            .iter()
            .map(|f| {
                let b = f.plan.definition.bounds;
                let shape = Collider::new(f.plan.entity, f.origin, f.angles, b.min, b.max);
                Target {
                    entity: f.plan.entity,
                    source: f.source,
                    center: std::array::from_fn(|i| (shape.min[i] + shape.max[i]) * 0.5),
                }
            })
            .collect()
    }
    pub fn damage(
        &mut self,
        now: u32,
        entity: u16,
        damage: u16,
        attacker_team: u8,
        c: &Collision,
    ) -> Option<SupportFlight> {
        let index = self
            .flights
            .iter()
            .position(|f| f.plan.entity == entity && f.source.team != attacker_team)?;
        if damage == 0 {
            return None;
        }
        let flight = &mut self.flights[index];
        flight.health = flight.health.saturating_sub(damage);
        tracing::info!(
            now,
            entity,
            damage,
            health = flight.health,
            "aircraft damaged"
        );
        if flight.health > 0 {
            return None;
        }
        let flight = self.flights.remove(index);
        self.events.push_back(Event::Fx {
            selector: flight.plan.definition.blast_fx,
            origin: flight.origin,
        });
        self.sync_collision(c);
        Some(flight.source)
    }
    pub fn radial(
        &mut self,
        now: u32,
        blast: Blast,
        attacker_team: u8,
        c: &Collision,
    ) -> Vec<SupportFlight> {
        let hits = self
            .flights
            .iter()
            .map(|f| {
                let b = f.plan.definition.bounds;
                let damage = crate::game_props::radius_damage(
                    blast,
                    f.origin,
                    (b.min[2] + b.max[2]) * 0.5,
                    Some(f.plan.entity),
                    c,
                );
                (f.plan.entity, damage)
            })
            .collect::<Vec<_>>();
        hits.into_iter()
            .filter_map(|(entity, damage)| self.damage(now, entity, damage, attacker_team, c))
            .collect()
    }
    pub fn drain(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
    pub fn emp(&mut self, now: u32, team: u8, c: &Collision) -> Vec<SupportFlight> {
        let entities = self
            .flights
            .iter()
            .filter(|f| f.source.team != team)
            .map(|f| f.plan.entity)
            .collect::<Vec<_>>();
        entities
            .into_iter()
            .filter_map(|entity| self.damage(now, entity, u16::MAX, team, c))
            .collect()
    }
}

#[cfg(test)]
pub(crate) fn test_resources() -> RadarResources {
    let mut r = crate::game_deployables::test_resources();
    for (name, model) in [
        ("vehicle_uav_static02_mp", 149),
        ("vehicle_counter_uav_static02_mp", 141),
    ] {
        r.models.insert(name.into(), model);
        r.model_bounds.insert(
            name.into(),
            ModelBounds {
                min: [-30.0, -30.0, -10.0],
                max: [30.0, 30.0, 10.0],
            },
        );
    }
    r.effects.insert("explosions/aerial_explosion".into(), 5);
    let radio = r.radios[0].clone();
    r.radios.extend([
        crate::game_killstreaks::Radio {
            native_id: 1,
            weapon: 329,
            ..radio.clone()
        },
        crate::game_killstreaks::Radio {
            native_id: 3,
            weapon: 314,
            ..radio
        },
    ]);
    r
}
#[cfg(test)]
mod tests {
    use super::*;
    fn collision(sky: bool) -> Collision {
        let f = tempfile::NamedTempFile::new().unwrap();
        let mut brushes = vec![
            serde_json::json!({"center":[0,0,-10],"half":[5000,5000,10],"contents":1,"planes":[]}),
        ];
        if sky {
            brushes.push(serde_json::json!({"center":[0,0,3000],"half":[10000,10000,1000],"contents":1,"planes":[]}));
        }
        std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]})).unwrap()).unwrap();
        Collision::load(f.path()).unwrap()
    }
    fn source(actor: u8, id: u8, start: u32) -> SupportFlight {
        SupportFlight {
            actor,
            id,
            start,
            team: 1,
            end: start + 30000,
        }
    }
    #[test]
    fn flight_validates_resources_and_whole_route_then_moves_and_expires() {
        let r = test_resources();
        let c = collision(false);
        let mut air = Airspace::default();
        assert!(air.plan_radar(&r, 19, [0.0; 3], &c).is_err());
        assert!(air.plan_radar(&r, 1, [f32::NAN, 0.0, 0.0], &c).is_err());
        assert!(air.plan_radar(&r, 1, [0.0; 3], &collision(true)).is_err());
        let key = source(0, 1, 1000);
        let plan = air.plan_radar(&r, 1, [0.0; 3], &c).unwrap();
        air.launch(plan, key, &c);
        let initial = air.snapshots()[0];
        assert_eq!(initial.model, 149);
        assert_eq!(initial.entity_number, 448);
        air.advance(2000, false, &[key], &c);
        let moving = air.snapshots()[0];
        assert_ne!(moving.origin, initial.origin);
        assert_eq!(moving.angles, initial.angles);
        assert_eq!(moving.origin[1], initial.origin[1]);
        assert!((moving.origin[0] - initial.origin[0] - 200.0).abs() < 0.01);
        assert_eq!(moving.origin[2], 3000.0);
        let trace = c.raycast(
            [moving.origin[0], moving.origin[1], 2500.0],
            [moving.origin[0], moving.origin[1], 3500.0],
        );
        assert_eq!(trace.prop_entity, Some(448));
        air.advance(31000, false, &[key], &c);
        assert!(air.snapshots().is_empty());
        assert_eq!(
            c.raycast(
                [moving.origin[0], moving.origin[1], 2500.0],
                [moving.origin[0], moving.origin[1], 3500.0]
            )
            .prop_entity,
            None
        );
    }
    #[test]
    fn whole_straight_pass_rejects_an_obstacle_between_clear_endpoints() {
        let c = collision(false);
        c.replace_props(&[Collider::new(
            256,
            [0.0, 0.0, 3000.0],
            [0; 3],
            [-10.0; 3],
            [10.0; 3],
        )]);
        assert!(
            Airspace::default()
                .plan_radar(&test_resources(), 1, [0.0; 3], &c)
                .is_err()
        );
        c.replace_props(&[]);
        assert!(
            Airspace::default()
                .plan_radar(&test_resources(), 1, [0.0; 3], &c)
                .is_ok()
        );
    }
    #[test]
    fn enemy_damage_returns_only_destroyed_support_and_emits_once_while_friendlies_block() {
        let r = test_resources();
        let c = collision(false);
        let mut air = Airspace::default();
        let key = source(0, 1, 1000);
        let plan = air.plan_radar(&r, 1, [0.0; 3], &c).unwrap();
        air.launch(plan, key, &c);
        assert_eq!(air.damage(2000, 448, 400, 1, &c), None);
        assert_eq!(air.damage(2000, 448, 399, 2, &c), None);
        assert_eq!(air.damage(2050, 448, 1, 2, &c), Some(key));
        assert!(air.snapshots().is_empty());
        assert_eq!(air.drain().len(), 1);
        assert_eq!(air.damage(2051, 448, 1, 2, &c), None);
        assert!(air.drain().is_empty());
    }
    #[test]
    fn support_removal_emp_match_end_and_map_collider_refresh_keep_ownership_isolated() {
        let r = test_resources();
        let c = collision(false);
        let mut air = Airspace::default();
        let a = source(0, 1, 1000);
        let mut b = source(6, 3, 1001);
        b.team = 2;
        let plan = air.plan_radar(&r, 1, [0.0; 3], &c).unwrap();
        air.launch(plan, a, &c);
        let plan = air.plan_radar(&r, 3, [3000.0, 0.0, 0.0], &c).unwrap();
        air.launch(plan, b, &c);
        c.replace_props(&[]);
        assert_eq!(
            c.raycast([-3000.0, 0.0, 2500.0], [-3000.0, 0.0, 3500.0])
                .prop_entity,
            Some(448)
        );
        assert_eq!(air.emp(2000, 1, &c), [b]);
        assert_eq!(air.snapshots().len(), 1);
        air.advance(2100, false, &[], &c);
        assert!(air.snapshots().is_empty());
        let plan = air.plan_radar(&r, 1, [0.0; 3], &c).unwrap();
        air.launch(plan, a, &c);
        air.advance(2200, true, &[a], &c);
        assert!(air.snapshots().is_empty());
    }
    #[test]
    #[ignore = "requires target-build aircraft resources and collision capture"]
    fn captured_air_support_anchor_has_a_clear_flight_path() {
        let r = RadarResources::load(
            std::path::Path::new(&std::env::var("CODOL_AIR_RESOURCES").unwrap()),
            "mp_afghan",
            523897474,
        )
        .unwrap();
        let c = Collision::load(std::path::Path::new(
            &std::env::var("CODOL_AIR_COLLISION").unwrap(),
        ))
        .unwrap();
        let air = Airspace::default();
        for id in [1, 3] {
            let plan = air.plan_radar(&r, id, [4144.0, 2580.0, -12.0], &c).unwrap();
            let anchor = r.air_route.as_ref().unwrap().center;
            assert_eq!(plan.route.start[2], anchor[2] + RADAR_HEIGHT_ABOVE_ANCHOR);
            assert_eq!(plan.route.end[2], plan.route.start[2]);
            println!(
                "id={id} start={:?} end={:?}",
                plan.route.start, plan.route.end
            );
        }
    }
}
