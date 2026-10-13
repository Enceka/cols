//! Map-bound barrels and vehicles, with authority-owned damage and destruction.
//! Whole-object health and blast values are local compatibility rules; model
//! poses, bounds, registrations and effect selectors come from native captures.
use anyhow::{Result, ensure};
use codol_protocol::game_props::PropSnapshot;
use serde::Deserialize;
use std::{collections::VecDeque, path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Barrel,
    Vehicle,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Definition {
    pub entity_number: u16,
    pub kind: Kind,
    pub model: u16,
    pub destroyed_model: Option<u16>,
    pub origin: [f32; 3],
    pub angles: [u16; 3],
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub destroyed_bounds: Option<[[f32; 3]; 2]>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resources {
    schema_version: u8,
    client_sha256: String,
    map: String,
    ncs_crc: u32,
    sources_sha256: Vec<String>,
    pub damage_weapon: u16,
    barrel_ignite_fx: u16,
    barrel_burn_fx: u16,
    barrel_explosion_fx: u16,
    vehicle_explosion_fx: u16,
    vehicle_smoke_fx: u16,
    barrel_sound_slot: u16,
    vehicle_sound_slot: u16,
    pub sound_aliases: Vec<(u16, String)>,
    pub props: Vec<Definition>,
    unsupported_models: Vec<String>,
}
fn bounds_valid(low: [f32; 3], high: [f32; 3]) -> bool {
    (0..3).all(|i| {
        low[i].is_finite()
            && high[i].is_finite()
            && low[i] < high[i]
            && low[i].abs() <= 2048.0
            && high[i].abs() <= 2048.0
    })
}
impl Resources {
    pub fn load(path: &Path, map: &str, crc: u32) -> Result<Self> {
        ensure!(path.metadata()?.len() <= 256 * 1024, "prop resource budget");
        let r: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            r.schema_version == 1
                && r.client_sha256 == codol_protocol::CLIENT_SHA256
                && r.map == map
                && r.ncs_crc == crc
                && (3..=5).contains(&r.sources_sha256.len())
                && r.sources_sha256
                    .iter()
                    .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "prop map/build/CRC mismatch"
        );
        let barrel = r.props.iter().any(|p| p.kind == Kind::Barrel);
        let vehicle = r.props.iter().any(|p| p.kind == Kind::Vehicle);
        ensure!(
            (1..900).contains(&r.damage_weapon)
                && [r.barrel_ignite_fx, r.barrel_burn_fx, r.barrel_explosion_fx]
                    .into_iter()
                    .all(|f| if barrel {
                        (1..=399).contains(&f)
                    } else {
                        f <= 399
                    })
                && [r.vehicle_explosion_fx, r.vehicle_smoke_fx]
                    .into_iter()
                    .all(|f| if vehicle {
                        (1..=399).contains(&f)
                    } else {
                        f <= 399
                    }),
            "invalid prop effect resources"
        );
        ensure!(
            r.sound_aliases
                == [
                    (r.barrel_sound_slot, "explo_metal_rand".into()),
                    (r.vehicle_sound_slot, "car_explode".into())
                ]
                && (1..512).contains(&r.barrel_sound_slot)
                && (1..512).contains(&r.vehicle_sound_slot)
                && r.barrel_sound_slot != r.vehicle_sound_slot,
            "invalid prop sound bindings"
        );
        ensure!(
            r.props.len() <= 128 && r.unsupported_models.len() <= 128,
            "prop model budget"
        );
        for (i, p) in r.props.iter().enumerate() {
            ensure!(
                p.entity_number == codol_protocol::game_props::FIRST_ENTITY + i as u16
                    && (1..1024).contains(&p.model)
                    && p.origin
                        .iter()
                        .all(|n| n.is_finite() && (-32768.0..=32767.0).contains(n))
                    && bounds_valid(p.bounds_min, p.bounds_max)
                    && p.destroyed_model.is_some() == p.destroyed_bounds.is_some()
                    && p.destroyed_model
                        .is_none_or(|m| (1..1024).contains(&m) && m != p.model)
                    && p.destroyed_bounds.is_none_or(|b| bounds_valid(b[0], b[1])),
                "invalid map prop definition"
            );
        }
        Ok(r)
    }
    pub fn configstrings(&self) -> Vec<(u16, String)> {
        self.sound_aliases
            .iter()
            .map(|(slot, name)| (3107 + slot, name.clone()))
            .collect()
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Collider {
    pub entity: u16,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub planes: Vec<[f32; 4]>,
}
impl Collider {
    pub fn new(
        entity: u16,
        origin: [f32; 3],
        angles: [u16; 3],
        min: [f32; 3],
        max: [f32; 3],
    ) -> Self {
        let angle = angles.map(|a| f32::from(a) * std::f32::consts::TAU / 65536.0);
        let (sp, cp) = angle[0].sin_cos();
        let (sy, cy) = angle[1].sin_cos();
        let (sr, cr) = angle[2].sin_cos();
        let axes = [
            [cp * cy, cp * sy, -sp],
            [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, sr * cp],
            [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp],
        ];
        let local: [f32; 3] = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let half: [f32; 3] = std::array::from_fn(|i| (max[i] - min[i]) * 0.5);
        let center: [f32; 3] =
            std::array::from_fn(|i| origin[i] + (0..3).map(|j| local[j] * axes[j][i]).sum::<f32>());
        let extent: [f32; 3] =
            std::array::from_fn(|i| (0..3).map(|j| half[j] * axes[j][i].abs()).sum());
        let mut planes = Vec::new();
        for i in 0..3 {
            for sign in [1.0, -1.0] {
                let n = axes[i].map(|x| x * sign);
                let distance = (0..3).map(|j| n[j] * center[j]).sum::<f32>() + half[i];
                planes.push([n[0], n[1], n[2], distance]);
            }
        }
        Self {
            entity,
            min: std::array::from_fn(|i| center[i] - extent[i]),
            max: std::array::from_fn(|i| center[i] + extent[i]),
            planes,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Intact,
    Burning,
    Wreck,
    Gone,
}
struct State {
    health: u16,
    phase: Phase,
    owner: u8,
    burn_time: u32,
    fire_time: u32,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Blast {
    pub owner: u8,
    pub source_entity: Option<u16>,
    pub weapon: u16,
    pub origin: [f32; 3],
    pub radius: f32,
    pub inner: u16,
    pub outer: u16,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    Fx {
        selector: u16,
        origin: [f32; 3],
    },
    Sound {
        slot: u16,
        owner: u8,
        origin: [f32; 3],
    },
    Blast(Blast),
}
#[derive(Default)]
pub(crate) struct Props {
    resources: Option<Arc<Resources>>,
    states: Vec<State>,
    events: VecDeque<Event>,
}
impl Props {
    pub fn new(resources: Arc<Resources>, collision: &crate::game_motion::Collision) -> Self {
        let states = resources
            .props
            .iter()
            .map(|p| State {
                health: match p.kind {
                    Kind::Barrel => 150,
                    Kind::Vehicle => 350,
                },
                phase: Phase::Intact,
                owner: 0,
                burn_time: 0,
                fire_time: 0,
            })
            .collect();
        let p = Self {
            resources: Some(resources),
            states,
            events: VecDeque::new(),
        };
        p.sync_collision(collision);
        p
    }
    pub fn snapshots(&self) -> Vec<PropSnapshot> {
        self.resources
            .iter()
            .flat_map(|r| r.props.iter())
            .zip(&self.states)
            .map(|(p, s)| PropSnapshot {
                entity_number: p.entity_number,
                model: if s.phase == Phase::Wreck {
                    p.destroyed_model.unwrap()
                } else {
                    p.model
                },
                origin: p.origin,
                angles: p.angles,
                present: s.phase != Phase::Gone,
            })
            .collect()
    }
    fn sync_collision(&self, collision: &crate::game_motion::Collision) {
        let geometry = self
            .resources
            .iter()
            .flat_map(|r| r.props.iter())
            .zip(&self.states)
            .filter(|(_, s)| s.phase != Phase::Gone)
            .map(|(p, s)| {
                let bounds = if s.phase == Phase::Wreck {
                    p.destroyed_bounds.unwrap()
                } else {
                    [p.bounds_min, p.bounds_max]
                };
                Collider::new(p.entity_number, p.origin, p.angles, bounds[0], bounds[1])
            })
            .collect::<Vec<_>>();
        collision.replace_props(&geometry);
    }
    pub fn damage(
        &mut self,
        now: u32,
        entity: u16,
        damage: u16,
        owner: u8,
        splash: bool,
        collision: &crate::game_motion::Collision,
    ) -> bool {
        if damage == 0 || owner >= 12 {
            return false;
        }
        let Some(r) = &self.resources else {
            return false;
        };
        let Some(i) = r.props.iter().position(|p| p.entity_number == entity) else {
            return false;
        };
        if matches!(self.states[i].phase, Phase::Gone | Phase::Wreck) {
            return false;
        }
        let p = &r.props[i];
        let s = &mut self.states[i];
        s.owner = owner;
        s.health = s.health.saturating_sub(damage);
        tracing::info!(
            now,
            entity,
            damage,
            health = s.health,
            owner,
            "map prop damaged"
        );
        if s.phase == Phase::Intact {
            s.phase = Phase::Burning;
            s.burn_time = now + 1000;
            s.fire_time = now;
            let selector = if p.kind == Kind::Barrel {
                r.barrel_ignite_fx
            } else {
                r.vehicle_smoke_fx
            };
            self.events.push_back(Event::Fx {
                selector,
                origin: p.origin,
            });
        }
        if s.health == 0 || splash && p.kind == Kind::Barrel {
            self.explode(i, collision);
        }
        true
    }
    fn explode(&mut self, i: usize, collision: &crate::game_motion::Collision) {
        let r = self.resources.as_ref().unwrap();
        let p = &r.props[i];
        let s = &mut self.states[i];
        if matches!(s.phase, Phase::Wreck | Phase::Gone) {
            return;
        }
        s.health = 0;
        s.phase = if p.destroyed_model.is_some() {
            Phase::Wreck
        } else {
            Phase::Gone
        };
        let barrel = p.kind == Kind::Barrel;
        let origin = p.origin;
        self.events.push_back(Event::Fx {
            selector: if barrel {
                r.barrel_explosion_fx
            } else {
                r.vehicle_explosion_fx
            },
            origin,
        });
        self.events.push_back(Event::Sound {
            slot: if barrel {
                r.barrel_sound_slot
            } else {
                r.vehicle_sound_slot
            },
            owner: s.owner,
            origin,
        });
        self.events.push_back(Event::Blast(Blast {
            owner: s.owner,
            source_entity: Some(p.entity_number),
            weapon: r.damage_weapon,
            origin: [origin[0], origin[1], origin[2] + 30.0],
            radius: if barrel { 250.0 } else { 300.0 },
            inner: if barrel { 250 } else { 200 },
            outer: if barrel { 1 } else { 75 },
        }));
        tracing::info!(
            entity = p.entity_number,
            owner = s.owner,
            barrel,
            "map prop exploded"
        );
        self.sync_collision(collision);
    }
    pub fn advance(&mut self, now: u32, collision: &crate::game_motion::Collision) {
        for i in 0..self.states.len() {
            if self.states[i].phase != Phase::Burning {
                continue;
            }
            let p = &self.resources.as_ref().unwrap().props[i];
            if p.kind == Kind::Barrel {
                if now >= self.states[i].fire_time {
                    self.states[i].fire_time = now + 100;
                    self.events.push_back(Event::Fx {
                        selector: self.resources.as_ref().unwrap().barrel_burn_fx,
                        origin: [p.origin[0], p.origin[1], p.origin[2] + 44.0],
                    });
                }
                while now >= self.states[i].burn_time && self.states[i].health > 0 {
                    self.states[i].health = self.states[i].health.saturating_sub(15);
                    self.states[i].burn_time += 1000;
                }
            }
            if self.states[i].health == 0 {
                self.explode(i, collision);
            }
        }
    }
    pub fn radial(&mut self, now: u32, blast: Blast, collision: &crate::game_motion::Collision) {
        let targets = self
            .resources
            .iter()
            .flat_map(|r| r.props.iter())
            .zip(&self.states)
            .filter(|(_, s)| !matches!(s.phase, Phase::Gone | Phase::Wreck))
            .map(|(p, _)| {
                (
                    p.entity_number,
                    p.origin,
                    (p.bounds_min[2] + p.bounds_max[2]) * 0.5,
                )
            })
            .collect::<Vec<_>>();
        for (id, origin, height) in targets {
            let damage = radius_damage(blast, origin, height, Some(id), collision);
            self.damage(now, id, damage, blast.owner, true, collision);
        }
    }
    pub fn drain(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
}
pub(crate) fn radius_damage(
    blast: Blast,
    origin: [f32; 3],
    height: f32,
    target: Option<u16>,
    collision: &crate::game_motion::Collision,
) -> u16 {
    let center = [origin[0], origin[1], origin[2] + height];
    let distance = (0..3)
        .map(|i| (center[i] - blast.origin[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    if distance >= blast.radius {
        return 0;
    }
    let trace = collision.raycast_excluding_prop(blast.origin, center, blast.source_entity);
    if (trace.contents != 0 && trace.fraction < 1.0)
        && !(target.is_some() && trace.prop_entity == target)
    {
        return 0;
    }
    (f32::from(blast.inner)
        + (distance / blast.radius) * (f32::from(blast.outer) - f32::from(blast.inner)))
    .ceil()
    .max(0.0) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    fn collision() -> crate::game_motion::Collision {
        let f = tempfile::NamedTempFile::new().unwrap();
        let data = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,
            "brushes":[{"center":[0,0,-10],"half":[1000,1000,10],"contents":1,"planes":[]}],"triangles":[]});
        std::fs::write(f.path(), serde_json::to_vec(&data).unwrap()).unwrap();
        crate::game_motion::Collision::load(f.path()).unwrap()
    }
    fn definition(entity: u16, kind: Kind, x: f32) -> Definition {
        Definition {
            entity_number: entity,
            kind,
            model: 4,
            destroyed_model: (kind == Kind::Vehicle).then_some(5),
            origin: [x, 0.0, 0.0],
            angles: [0; 3],
            bounds_min: [-10.0, -10.0, 0.0],
            bounds_max: [10.0, 10.0, 40.0],
            destroyed_bounds: (kind == Kind::Vehicle)
                .then_some([[-10.0, -10.0, 0.0], [10.0, 10.0, 10.0]]),
        }
    }
    fn resources(props: Vec<Definition>) -> Arc<Resources> {
        Arc::new(Resources {
            schema_version: 1,
            client_sha256: codol_protocol::CLIENT_SHA256.into(),
            map: "test".into(),
            ncs_crc: 0,
            sources_sha256: vec!["0".repeat(64); 3],
            damage_weapon: 73,
            barrel_ignite_fx: 1,
            barrel_burn_fx: 2,
            barrel_explosion_fx: 3,
            vehicle_explosion_fx: 4,
            vehicle_smoke_fx: 5,
            barrel_sound_slot: 480,
            vehicle_sound_slot: 481,
            sound_aliases: vec![
                (480, "explo_metal_rand".into()),
                (481, "car_explode".into()),
            ],
            props,
            unsupported_models: vec![],
        })
    }
    #[test]
    fn real_shot_hits_the_nearest_prop_and_destruction_removes_its_collision_once() {
        let c = collision();
        let mut props = Props::new(resources(vec![definition(256, Kind::Barrel, 100.0)]), &c);
        let shot = crate::game_combat::fire_at_actors(
            [0.0, 0.0, 20.0],
            [0; 3],
            1000,
            crate::game_combat::Ballistics {
                damage: 160,
                range: 1000.0,
                near_range: 200.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
            (0, 1),
            crate::game_combat::ActorScene {
                actors: &[],
                regions: None,
                collision: &c,
            },
        );
        assert_eq!(shot.props_damage, [(256, 160)]);
        assert_eq!(
            c.raycast([0.0, 0.0, 20.0], [200.0, 0.0, 20.0]).prop_entity,
            Some(256)
        );
        assert!(props.damage(1000, 256, 160, 0, false, &c));
        assert!(!props.snapshots()[0].present);
        assert_eq!(
            c.raycast([0.0, 0.0, 20.0], [200.0, 0.0, 20.0]).prop_entity,
            None
        );
        assert!(!props.damage(1001, 256, 160, 0, false, &c));
        props.advance(99999, &c);
        assert_eq!(
            props
                .drain()
                .iter()
                .filter(|e| matches!(e, Event::Blast(_)))
                .count(),
            1
        );
    }
    #[test]
    fn grenade_starts_a_bounded_chain_and_vehicle_keeps_the_native_wreck_model() {
        let c = collision();
        let mut props = Props::new(
            resources(vec![
                definition(256, Kind::Barrel, 50.0),
                definition(257, Kind::Barrel, 90.0),
                definition(258, Kind::Vehicle, 200.0),
            ]),
            &c,
        );
        props.damage(1000, 256, 1, 0, true, &c);
        let first = props.drain();
        let blast = first
            .into_iter()
            .find_map(|e| {
                if let Event::Blast(b) = e {
                    Some(b)
                } else {
                    None
                }
            })
            .unwrap();
        props.radial(1000, blast, &c);
        assert!(!props.snapshots()[1].present);
        assert!(props.states[2].health < 350);
        props.damage(1001, 258, 350, 0, false, &c);
        assert!(props.snapshots()[2].present);
        assert_eq!(props.snapshots()[2].model, 5);
        let events = props.drain();
        let car = events
            .iter()
            .find_map(|e| {
                if let Event::Blast(b) = e {
                    (b.source_entity == Some(258)).then_some(*b)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(radius_damage(car, [200.0, 60.0, 0.0], 35.0, None, &c) > 0);
        props.advance(10000, &c);
        assert!(props.drain().is_empty());
    }
    #[test]
    fn burning_ticks_do_not_repeat_blasts_and_a_fresh_match_is_intact() {
        let c = collision();
        let r = resources(vec![definition(256, Kind::Barrel, 100.0)]);
        let mut props = Props::new(r.clone(), &c);
        props.damage(1000, 256, 120, 0, false, &c);
        props.drain();
        props.advance(2000, &c);
        assert!(props.snapshots()[0].present);
        props.drain();
        props.advance(3000, &c);
        assert!(!props.snapshots()[0].present);
        assert_eq!(
            props
                .drain()
                .iter()
                .filter(|e| matches!(e, Event::Blast(_)))
                .count(),
            1
        );
        props.advance(3000, &c);
        assert!(props.drain().is_empty());
        let fresh = c.fresh_session();
        let new = Props::new(r, &fresh);
        assert!(new.snapshots()[0].present);
        assert_eq!(
            fresh
                .raycast([0.0, 0.0, 20.0], [200.0, 0.0, 20.0])
                .prop_entity,
            Some(256)
        );
    }
    #[test]
    fn oriented_bounds_block_movement_but_other_rooms_and_world_occlusion_remain_independent() {
        let c = collision();
        let other = c.fresh_session();
        let shape = Collider::new(
            256,
            [100.0, 0.0, 0.0],
            [0, 16384, 0],
            [-20.0, -5.0, 0.0],
            [20.0, 5.0, 60.0],
        );
        c.replace_props(&[shape]);
        let hit = c.trace([0.0; 3], [200.0, 0.0, 0.0], 60.0);
        assert!(hit.fraction < 1.0);
        assert_eq!(hit.prop_entity, Some(256));
        assert_eq!(
            other
                .raycast([0.0, 0.0, 20.0], [200.0, 0.0, 20.0])
                .prop_entity,
            None
        );
        let ray = c.raycast([0.0, 0.0, 30.0], [200.0, 0.0, 30.0]);
        assert!((ray.fraction - 0.475).abs() < 0.001);
        let blast = Blast {
            source_entity: None,
            owner: 0,
            weapon: 73,
            origin: [0.0, 0.0, 30.0],
            radius: 500.0,
            inner: 250,
            outer: 1,
        };
        assert_eq!(radius_damage(blast, [200.0, 0.0, 0.0], 30.0, None, &c), 0);
        assert_eq!(radius_damage(blast, [0.0, 0.0, -100.0], 0.0, None, &c), 0);
    }
}
