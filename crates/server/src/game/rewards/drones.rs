//! Hunter-killer payloads. Weapon speed, damage and lifetime are captured;
//! launch clearance, acquisition delay and steering are explicit local rules.
use super::{
    killstreaks::RadarResources,
    strikes::{Hit, Source},
};
use crate::game::world::{
    collision::Collision,
    math::*,
    props::{Blast, Event},
};
use codol_protocol::game::snapshot::missile::MissileSnapshot;

pub(crate) use codol_protocol::game::snapshot::missile::{
    END_DRONE_ENTITY as END_ENTITY, FIRST_DRONE_ENTITY as FIRST_ENTITY,
};
#[derive(Clone, Copy)]
struct Weapon {
    index: u16,
    speed: f32,
    lifetime: u32,
    radius: f32,
    inner: u16,
    outer: u16,
}
impl Weapon {
    fn load(r: &RadarResources) -> Option<Self> {
        let w = r.weapons.get("missile_drone_projectile_mp")?;
        let projectile = w.projectile?;
        if w.weapon_type != 2
            || projectile.explosion_kind != 1
            || !(1..=60000).contains(&projectile.lifetime_msec)
            || !(1..=50000).contains(&w.projectile_speed)
            || !(1..=8192).contains(&w.radius)
            || !(1..=65535).contains(&w.inner_damage)
            || w.outer_damage > 65535
            || !r.models.contains_key("veh_t6_drone_hunterkiller")
        {
            return None;
        }
        Some(Self {
            index: w.weapon,
            speed: w.projectile_speed as f32,
            lifetime: projectile.lifetime_msec,
            radius: w.radius as f32,
            inner: w.inner_damage as u16,
            outer: w.outer_damage as u16,
        })
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Target {
    pub actor: u8,
    pub team: u8,
    pub center: V3,
    pub hidden: bool,
    pub born: u32,
}
pub(crate) struct Plan {
    entity: u16,
    origin: V3,
    direction: V3,
    weapon: Weapon,
}
struct Drone {
    snapshot: MissileSnapshot,
    direction: V3,
    weapon: Weapon,
    source: Source,
    last: u32,
    acquired_at: u32,
    expires: u32,
}
#[derive(Default)]
pub(crate) struct Drones {
    active: Vec<Drone>,
    generation: u32,
    events: Vec<Event>,
}
impl Drones {
    pub(crate) fn resources_present(r: &RadarResources) -> bool {
        Weapon::load(r).is_some()
    }
    pub(crate) fn plan(
        &self,
        r: &RadarResources,
        origin: V3,
        angles: [u16; 3],
        c: &Collision,
    ) -> Option<Plan> {
        let weapon = Weapon::load(r)?;
        let entity = (FIRST_ENTITY..END_ENTITY)
            .find(|e| self.active.iter().all(|d| d.snapshot.entity_number != *e))?;
        if origin.iter().any(|v| !v.is_finite() || v.abs() > 30000.0) {
            return None;
        }
        let yaw = f32::from(angles[1]) * std::f32::consts::TAU / 65536.0;
        let direction = normalized([yaw.cos(), yaw.sin(), 0.7]);
        let eye = add(origin, [0.0, 0.0, 60.0]);
        let start = add(eye, mul(direction, 48.0));
        let trace = c.raycast(eye, start);
        if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
            return None;
        }
        Some(Plan {
            entity,
            origin: start,
            direction,
            weapon,
        })
    }
    pub(crate) fn launch(&mut self, now: u32, plan: Plan, source: Source) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("drone generation exhausted");
        self.active.push(Drone {
            snapshot: MissileSnapshot {
                entity_number: plan.entity,
                origin: plan.origin,
                weapon: plan.weapon.index,
                present: true,
                generation: self.generation,
                owner: source.owner,
                angles: angles(plan.direction),
                events: Default::default(),
            },
            direction: plan.direction,
            weapon: plan.weapon,
            source,
            last: now,
            acquired_at: now.saturating_add(1000),
            expires: now.saturating_add(plan.weapon.lifetime),
        });
    }
    pub(crate) fn advance(
        &mut self,
        now: u32,
        finished: bool,
        emp: u8,
        targets: &[Target],
        c: &Collision,
    ) -> Vec<Hit> {
        let mut hits = Vec::new();
        let mut keep = Vec::new();
        for mut drone in self.active.drain(..) {
            if finished || emp & (1 << drone.source.team) != 0 {
                continue;
            }
            let until = now.min(drone.expires);
            let mut impact = None;
            let mut invalid = false;
            while drone.last < until {
                let msec = (until - drone.last).min(20);
                drone.last += msec;
                let dt = msec as f32 * 0.001;
                if drone.last >= drone.acquired_at
                    && let Some(target) = targets
                        .iter()
                        .filter(|t| {
                            if !eligible(**t, drone.source.team, drone.last) {
                                return false;
                            }
                            let trace = c.raycast(drone.snapshot.origin, t.center);
                            !trace.start_solid && !trace.all_solid && trace.fraction >= 1.0
                        })
                        .min_by(|a, b| {
                            length(sub(a.center, drone.snapshot.origin))
                                .total_cmp(&length(sub(b.center, drone.snapshot.origin)))
                                .then_with(|| a.actor.cmp(&b.actor))
                        })
                {
                    let desired = normalized(sub(target.center, drone.snapshot.origin));
                    drone.direction = turn(drone.direction, desired, 90.0f32.to_radians() * dt);
                }
                let end = add(
                    drone.snapshot.origin,
                    mul(drone.direction, drone.weapon.speed * dt),
                );
                let trace = c.raycast(drone.snapshot.origin, end);
                if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
                    let approach = -dot(sub(end, drone.snapshot.origin), trace.normal);
                    let fraction = if approach > 0.0 {
                        (trace.fraction - 0.125 / approach).max(0.0)
                    } else {
                        trace.fraction
                    };
                    drone.snapshot.origin = add(
                        drone.snapshot.origin,
                        mul(sub(end, drone.snapshot.origin), fraction),
                    );
                    impact = Some((trace.normal, ((trace.surface_flags >> 20) & 31) as u8));
                    break;
                }
                drone.snapshot.origin = end;
                drone.snapshot.angles = angles(drone.direction);
                if end.iter().any(|v| !v.is_finite() || v.abs() > 32700.0) {
                    invalid = true;
                    break;
                }
                if targets.iter().any(|t| {
                    eligible(*t, drone.source.team, drone.last)
                        && length(sub(t.center, end)) < drone.weapon.speed * dt + 16.0
                        && c.raycast(end, t.center).fraction >= 1.0
                }) {
                    impact = Some(([0.0, 0.0, 1.0], 0));
                    break;
                }
            }
            if invalid {
                continue;
            }
            if impact.is_none() && now >= drone.expires {
                impact = Some(([0.0, 0.0, 1.0], 0));
            }
            if let Some((normal, surface)) = impact {
                hits.push(Hit {
                    blast: Blast {
                        owner: drone.source.owner,
                        source_entity: Some(drone.snapshot.entity_number),
                        weapon: drone.weapon.index,
                        origin: drone.snapshot.origin,
                        radius: drone.weapon.radius,
                        inner: drone.weapon.inner,
                        outer: drone.weapon.outer,
                        reward_life: Some(drone.source.life),
                    },
                    team: drone.source.team,
                    life: drone.source.life,
                    airstrike: false,
                    area: None,
                });
                self.events.push(Event::Rocket {
                    weapon: drone.weapon.index,
                    origin: drone.snapshot.origin,
                    normal,
                    surface,
                });
            } else {
                keep.push(drone);
            }
        }
        self.active = keep;
        hits
    }
    pub(crate) fn snapshots(&self) -> Vec<MissileSnapshot> {
        let mut states = self.active.iter().map(|d| d.snapshot).collect::<Vec<_>>();
        states.sort_by_key(|s| s.entity_number);
        states
    }
    pub(crate) fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}
fn eligible(t: Target, team: u8, now: u32) -> bool {
    t.actor < 12
        && matches!(t.team, 1 | 2)
        && t.team != team
        && !t.hidden
        && now.saturating_sub(t.born) >= 2000
        && t.center.iter().all(|v| v.is_finite() && v.abs() <= 32700.0)
}
fn angles(direction: V3) -> [u16; 3] {
    [
        -direction[2]
            .atan2(direction[0].hypot(direction[1]))
            .to_degrees(),
        direction[1].atan2(direction[0]).to_degrees(),
        0.0,
    ]
    .map(|a| (a * 65536.0 / 360.0).round() as i32 as u16)
}
fn turn(from: V3, to: V3, limit: f32) -> V3 {
    let cosine = dot(from, to).clamp(-1.0, 1.0);
    let angle = cosine.acos();
    if angle <= limit {
        return to;
    }
    let mut side = sub(to, mul(from, cosine));
    if length(side) < 0.0001 {
        side = cross(
            from,
            if from[2].abs() < 0.9 {
                [0.0, 0.0, 1.0]
            } else {
                [0.0, 1.0, 0.0]
            },
        );
    }
    normalized(add(
        mul(from, limit.cos()),
        mul(normalized(side), limit.sin()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resources() -> RadarResources {
        let mut r = super::super::remote::test_resources();
        let mut w = r.weapons["remotemissile_projectile_mp"].clone();
        w.weapon = 332;
        w.projectile_speed = 1500;
        w.projectile.as_mut().unwrap().lifetime_msec = 25000;
        r.weapons.insert("missile_drone_projectile_mp".into(), w);
        r.models.insert("veh_t6_drone_hunterkiller".into(), 137);
        r
    }
    fn collision() -> Collision {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,
            "brushes":[{"center":[0,0,-10],"half":[30000,30000,10],"contents":1,"planes":[]}],"triangles":[]
        })).unwrap()).unwrap();
        Collision::load(f.path()).unwrap()
    }
    fn launch(d: &mut Drones, c: &Collision) {
        let plan = d.plan(&resources(), [0.0; 3], [0; 3], c).unwrap();
        d.launch(
            3000,
            plan,
            Source {
                owner: 0,
                team: 2,
                life: 7,
            },
        );
    }
    #[test]
    fn target_requires_enemy_visibility_spawn_age_and_finite_position() {
        let c = collision();
        let mut d = Drones::default();
        launch(&mut d, &c);
        let target = Target {
            actor: 6,
            team: 1,
            center: [5000.0, 0.0, 35.0],
            hidden: false,
            born: 0,
        };
        let mut all = Vec::new();
        for now in (3020..15000).step_by(20) {
            all.extend(d.advance(now, false, 0, &[target], &c));
            if !all.is_empty() {
                break;
            }
        }
        assert_eq!(all.len(), 1);
        assert!(
            length(sub(all[0].blast.origin, target.center)) < 80.0,
            "{:?}",
            all[0].blast.origin
        );
        assert_eq!(all[0].blast.weapon, 332);
        assert_eq!(all[0].blast.reward_life, Some(7));
        assert_eq!(d.drain().len(), 1);
        assert!(d.advance(30000, false, 0, &[target], &c).is_empty());
        for invalid in [
            Target { team: 2, ..target },
            Target {
                hidden: true,
                ..target
            },
            Target {
                born: 5000,
                ..target
            },
            Target {
                center: [f32::NAN, 0.0, 0.0],
                ..target
            },
        ] {
            assert!(!eligible(invalid, 2, 6000));
        }
    }
    #[test]
    fn expiry_collision_and_cancellation_each_resolve_once_with_original_life() {
        let c = collision();
        for cause in 0..3 {
            let mut d = Drones::default();
            launch(&mut d, &c);
            let hits = d.advance(
                if cause == 0 { 28000 } else { 3020 },
                cause == 1,
                if cause == 2 { 1 << 2 } else { 0 },
                &[],
                &c,
            );
            assert_eq!(hits.len(), usize::from(cause == 0));
            assert!(d.snapshots().is_empty());
            assert!(d.advance(40000, false, 0, &[], &c).is_empty());
        }
        let mut d = Drones::default();
        launch(&mut d, &c);
        d.active[0].direction = [0.0, 0.0, -1.0];
        let hits = d.advance(4000, false, 0, &[], &c);
        assert_eq!(hits.len(), 1);
        assert!((hits[0].blast.origin[2] - 0.125).abs() < 0.001);
        assert_eq!(hits[0].life, 7);
        assert!(d.snapshots().is_empty());
    }
    #[test]
    fn resource_or_spawn_failure_never_creates_payload_and_reverse_turn_stays_finite() {
        let c = collision();
        let mut r = resources();
        let d = Drones::default();
        r.weapons
            .get_mut("missile_drone_projectile_mp")
            .unwrap()
            .projectile = None;
        assert!(!Drones::resources_present(&r));
        assert!(d.plan(&r, [0.0; 3], [0; 3], &c).is_none());
        assert!(
            d.plan(&resources(), [0.0, 0.0, -100.0], [0; 3], &c)
                .is_none()
        );
        let turned = turn([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], 0.1);
        assert!((length(turned) - 1.0).abs() < 0.0001);
        assert!((dot(turned, [1.0, 0.0, 0.0]).acos() - 0.1).abs() < 0.0001);
    }
}
