//! Guided ground-to-air projectiles use captured TOW weapon facts, normal
//! world collision and the same native MISSILE snapshots as other projectiles.
use crate::game::rewards::airspace::Target;
use crate::game::rewards::killstreaks::RadarResources;
use crate::game::world::collision::Collision;
use crate::game::world::props::Blast;
use crate::game::world::props::Event;
use codol_protocol::game::snapshot::missile::MissileSnapshot;
use std::collections::VecDeque;
pub(crate) const FIRST_ENTITY: u16 = 1648;
pub(crate) const END_ENTITY: u16 = 1664;
#[derive(Clone, Copy)]
pub(crate) struct Weapon {
    pub index: u16,
    pub speed: f32,
    pub interval: u32,
    pub direct: u16,
    radius: f32,
    inner: u16,
    outer: u16,
    fx: u16,
}
impl Weapon {
    pub fn from_resources(r: &RadarResources) -> Option<Self> {
        Self::registered(r, "tow_turret_mp", false)
    }
    pub(crate) fn aircraft(r: &RadarResources, id: u8) -> Option<Self> {
        match id {
            8 => Self::registered(r, "cobra_ffar_mp", true),
            7 => Self::registered(r, "harrier_missile_mp", true),
            _ => None,
        }
    }
    fn registered(r: &RadarResources, name: &str, aircraft: bool) -> Option<Self> {
        let w = r.weapons.get(name)?;
        if w.weapon_type != 2
            || w.projectile_speed == 0
            || w.projectile_speed > 50000
            || (!aircraft && w.fire_interval_msec == 0)
            || w.damage_stages[0] == 0
            || w.damage_stages[0] > u32::from(u16::MAX)
            || w.radius == 0
            || w.radius > 8192
            || w.inner_damage > u32::from(u16::MAX)
            || w.outer_damage > u32::from(u16::MAX)
        {
            return None;
        }
        Some(Self {
            index: w.weapon,
            speed: w.projectile_speed as f32,
            interval: if aircraft {
                5000
            } else {
                u32::from(w.fire_interval_msec)
            },
            direct: w.damage_stages[0] as u16,
            radius: w.radius as f32,
            inner: w.inner_damage as u16,
            outer: w.outer_damage as u16,
            fx: *r.effects.get("explosions/aerial_explosion")?,
        })
    }
}
pub(crate) struct Launch {
    pub owner: u8,
    pub team: u8,
    pub device: u16,
    pub origin: [f32; 3],
    pub target: Target,
    pub weapon: Weapon,
    pub reward_life: Option<u32>,
}
struct Missile {
    snapshot: MissileSnapshot,
    device: u16,
    target: Target,
    weapon: Weapon,
    last_time: u32,
    end: u32,
    decoy: Option<[f32; 3]>,
    reward_life: Option<u32>,
}
pub(crate) struct Hit {
    pub entity: u16,
    pub damage: u16,
    pub owner: u8,
}
#[derive(Default)]
pub(crate) struct Missiles {
    active: Vec<Missile>,
    generation: u32,
    events: VecDeque<Event>,
}
impl Missiles {
    pub fn capacity(&self) -> usize {
        usize::from(END_ENTITY - FIRST_ENTITY) - self.active.len()
    }
    pub fn launch(&mut self, now: u32, request: Launch) -> bool {
        let Some(entity) = (FIRST_ENTITY..END_ENTITY)
            .find(|id| !self.active.iter().any(|m| m.snapshot.entity_number == *id))
        else {
            return false;
        };
        if request.owner >= 12
            || !matches!(request.team, 1 | 2)
            || request.target.source.team == request.team
            || request
                .origin
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 32700.0)
        {
            return false;
        }
        let angles = aim(request.origin, request.target.center);
        self.generation = self.generation.wrapping_add(1);
        self.active.push(Missile {
            snapshot: MissileSnapshot {
                events: Default::default(),
                entity_number: entity,
                origin: request.origin,
                weapon: request.weapon.index,
                generation: self.generation,
                present: true,
                owner: request.owner,
                angles,
            },
            device: request.device,
            target: request.target,
            weapon: request.weapon,
            last_time: now,
            end: now.saturating_add(5000),
            decoy: None,
            reward_life: request.reward_life,
        });
        tracing::info!(
            now,
            entity,
            device = request.device,
            target = request.target.entity,
            owner = request.owner,
            "SAM missile launched"
        );
        true
    }
    pub fn advance(
        &mut self,
        now: u32,
        finished: bool,
        targets: &[Target],
        collision: &Collision,
    ) -> Vec<Hit> {
        if finished {
            self.active.clear();
            return Vec::new();
        }
        let mut hits = Vec::new();
        let mut keep = Vec::new();
        for mut missile in self.active.drain(..) {
            // Entity slots may be reused. A lost support identity never becomes
            // permission to acquire a replacement plane with the same number.
            let target = if let Some(center) = missile.decoy {
                Target {
                    center,
                    ..missile.target
                }
            } else {
                let Some(target) = targets.iter().find(|t| {
                    t.entity == missile.target.entity && t.source == missile.target.source
                }) else {
                    continue;
                };
                *target
            };
            if now >= missile.end {
                continue;
            }
            let end_time = now.min(missile.end);
            let mut exploded = None;
            while missile.last_time < end_time {
                let msec = (end_time - missile.last_time).min(20);
                missile.last_time += msec;
                let delta: [f32; 3] =
                    std::array::from_fn(|i| target.center[i] - missile.snapshot.origin[i]);
                let distance = delta.into_iter().map(|v| v * v).sum::<f32>().sqrt();
                if distance < 1.0 {
                    exploded = Some((
                        target.center,
                        missile.decoy.is_none().then_some(target.entity),
                    ));
                    break;
                }
                let travel = (missile.weapon.speed * msec as f32 * 0.001).min(distance);
                let next = std::array::from_fn(|i| {
                    missile.snapshot.origin[i] + delta[i] * travel / distance
                });
                let trace = collision.projectile_trace_excluding_prop(
                    missile.snapshot.origin,
                    next,
                    2.0,
                    Some(missile.device),
                );
                if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
                    let impact = std::array::from_fn(|i| {
                        missile.snapshot.origin[i]
                            + (next[i] - missile.snapshot.origin[i]) * trace.fraction
                    });
                    exploded = Some((impact, trace.prop_entity));
                    break;
                }
                missile.snapshot.angles = aim(missile.snapshot.origin, next);
                missile.snapshot.origin = next;
            }
            if let Some((origin, entity)) = exploded {
                if let Some(entity) = entity {
                    hits.push(Hit {
                        entity,
                        damage: missile.weapon.direct,
                        owner: missile.snapshot.owner,
                    });
                }
                self.events.push_back(Event::Fx {
                    selector: missile.weapon.fx,
                    origin,
                });
                self.events.push_back(Event::Blast(Blast {
                    reward_life: missile.reward_life,
                    owner: missile.snapshot.owner,
                    source_entity: Some(missile.device),
                    weapon: missile.weapon.index,
                    origin,
                    radius: missile.weapon.radius,
                    inner: missile.weapon.inner,
                    outer: missile.weapon.outer,
                }));
                tracing::info!(now,entity=missile.snapshot.entity_number,target=missile.target.entity,hit=?entity,"SAM missile impact");
            } else {
                keep.push(missile);
            }
        }
        self.active = keep;
        hits
    }
    pub fn snapshots(&self) -> Vec<MissileSnapshot> {
        let mut s = self.active.iter().map(|m| m.snapshot).collect::<Vec<_>>();
        s.sort_by_key(|s| s.entity_number);
        s
    }
    pub(crate) fn divert(&mut self, target: Target) {
        if let Some(m) = self
            .active
            .last_mut()
            .filter(|m| m.target.entity == target.entity && m.target.source == target.source)
        {
            let delta: [f32; 3] = std::array::from_fn(|i| m.snapshot.origin[i] - target.center[i]);
            let distance = delta.iter().map(|d| d * d).sum::<f32>().sqrt().max(1.0);
            m.decoy = Some(std::array::from_fn(|i| {
                target.center[i] + delta[i] / distance * 1000.0
            }));
        }
    }
    pub fn drain(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
}
fn aim(from: [f32; 3], to: [f32; 3]) -> [u16; 3] {
    let d: [f32; 3] = std::array::from_fn(|i| to[i] - from[i]);
    let yaw = d[1].atan2(d[0]);
    let pitch = -d[2].atan2(d[0].hypot(d[1]));
    [pitch, yaw, 0.0].map(|a| (a * 65536.0 / std::f32::consts::TAU).round() as i32 as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::rewards::killstreaks::SupportFlight;

    fn target(start: u32) -> Target {
        Target {
            entity: 448,
            source: SupportFlight {
                actor: 6,
                id: 1,
                team: 2,
                start,
                end: start + 30000,
            },
            center: [300.0, 0.0, 100.0],
        }
    }
    fn launch(target: Target) -> Launch {
        Launch {
            reward_life: None,
            owner: 0,
            team: 1,
            device: 384,
            origin: [0.0, 0.0, 100.0],
            target,
            weapon: Weapon {
                index: 351,
                speed: 3000.0,
                interval: 750,
                direct: 1000,
                radius: 300.0,
                inner: 300,
                outer: 200,
                fx: 5,
            },
        }
    }
    fn collision() -> Collision {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[5000,5000,10],"contents":1,"planes":[]}],"triangles":[]})).unwrap()).unwrap();
        Collision::load(file.path()).unwrap()
    }
    #[test]
    fn guided_hit_emits_one_blast_and_removes_projectile() {
        let mut missiles = Missiles::default();
        let t = target(0);
        assert!(missiles.launch(0, launch(t)));
        assert!(missiles.advance(60, false, &[t], &collision()).is_empty());
        assert!((missiles.snapshots()[0].origin[0] - 180.0).abs() < 0.001);
        let hits = missiles.advance(120, false, &[t], &collision());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].entity, hits[0].damage, hits[0].owner),
            (448, 1000, 0)
        );
        assert!(missiles.snapshots().is_empty());
        assert_eq!(missiles.drain().len(), 2);
        assert!(missiles.advance(200, false, &[t], &collision()).is_empty());
        assert!(missiles.drain().is_empty());
    }
    #[test]
    fn diverted_missile_reaches_flare_without_direct_damage_or_requiring_live_plane() {
        let mut missiles = Missiles::default();
        let mut t = target(1000);
        t.center = [3000.0, 0.0, 100.0];
        assert!(missiles.launch(1000, launch(t)));
        missiles.divert(t);
        let c = collision();
        assert!(missiles.advance(2000, false, &[], &c).is_empty());
        assert!(missiles.snapshots().is_empty());
        let events = missiles.drain();
        assert_eq!(events.len(), 2);
        let Event::Blast(blast) = events[1] else {
            panic!("missing flare detonation")
        };
        assert!((blast.origin[0] - 2000.0).abs() < 0.01);
        assert_eq!(
            crate::game::world::props::radius_damage(blast, t.center, 0.0, None, &c),
            0
        );
    }
    #[test]
    fn projectile_excludes_launcher_but_hits_aircraft_or_other_live_geometry() {
        use crate::game::world::props::Collider;
        let c = collision();
        c.replace_prop_range(
            384..448,
            &[Collider {
                entity: 384,
                min: [-10.0, -10.0, 90.0],
                max: [10.0, 10.0, 110.0],
                planes: vec![],
            }],
        );
        c.replace_prop_range(
            448..464,
            &[Collider {
                entity: 448,
                min: [290.0, -10.0, 90.0],
                max: [310.0, 10.0, 110.0],
                planes: vec![],
            }],
        );
        let mut missiles = Missiles::default();
        assert!(missiles.launch(0, launch(target(0))));
        let hits = missiles.advance(120, false, &[target(0)], &c);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entity, 448);
        assert_eq!(missiles.drain().len(), 2);
        c.replace_props(&[Collider {
            entity: 256,
            min: [100.0, -10.0, 90.0],
            max: [110.0, 10.0, 110.0],
            planes: vec![],
        }]);
        assert!(missiles.launch(200, launch(target(0))));
        let hits = missiles.advance(320, false, &[target(0)], &c);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].entity, hits[0].owner), (256, 0));
        assert!(missiles.snapshots().is_empty());
        assert_eq!(missiles.drain().len(), 2);
    }
    #[test]
    fn lost_support_identity_never_acquires_reused_aircraft_slot() {
        let mut missiles = Missiles::default();
        assert!(missiles.launch(0, launch(target(0))));
        assert!(
            missiles
                .advance(20, false, &[target(1)], &collision())
                .is_empty()
        );
        assert!(missiles.snapshots().is_empty());
        assert!(missiles.drain().is_empty());
    }
    #[test]
    fn capacity_friendlies_and_match_end_are_authoritative() {
        let mut missiles = Missiles::default();
        let mut friendly = launch(target(0));
        friendly.team = 2;
        assert!(!missiles.launch(0, friendly));
        for _ in FIRST_ENTITY..END_ENTITY {
            assert!(missiles.launch(0, launch(target(0))));
        }
        assert_eq!(missiles.capacity(), 0);
        assert!(!missiles.launch(0, launch(target(0))));
        assert_eq!(missiles.snapshots().len(), 16);
        assert!(
            missiles
                .advance(20, true, &[target(0)], &collision())
                .is_empty()
        );
        assert_eq!(missiles.capacity(), 16);
        assert!(missiles.drain().is_empty());
    }
}
