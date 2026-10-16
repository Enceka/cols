//! Shared marker-driven transport, falling crates and sustained pickup.
use crate::game::rewards::killstreaks::ModelBounds;
use crate::game::rewards::killstreaks::RadarResources;
use crate::game::weapons::grenade::Definition as Marker;
use crate::game::weapons::grenade::Explosion;
use crate::game::world::collision::Collision;
use crate::game::world::props::Collider;
use codol_protocol::game::snapshot::props::PropSnapshot;
const FIRST_CARRIER: u16 = 464;
const END_CARRIER: u16 = 480;
const FIRST_CRATE: u16 = 480;
const END_CRATE: u16 = 544;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Contents {
    Ammo,
    Reward(u8),
}
#[derive(Clone, Copy)]
pub(crate) struct PickupUser {
    pub actor: u8,
    pub life: u32,
    pub feet: [f32; 3],
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    pub active: bool,
    pub holding: bool,
    pub steal_msec: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PickupHint {
    pub entity: u16,
    pub contents: Contents,
    pub elapsed: u32,
    pub duration: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Claim {
    entity: u16,
    generation: u64,
    actor: u8,
    life: u32,
    start: u32,
    pub contents: Contents,
}
#[derive(Clone, Copy)]
struct Hold {
    actor: u8,
    life: u32,
    start: u32,
    last: u32,
    duration: u32,
}
#[derive(Clone)]
struct Definition {
    marker: Marker,
    carrier: u16,
    carrier_bounds: ModelBounds,
    friendly: u16,
    enemy: u16,
    bounds: ModelBounds,
    count: usize,
    height: f32,
}
impl Definition {
    fn load(r: &RadarResources, id: u8) -> Option<Self> {
        let marker = Marker::marker(r, id)?;
        let name = if id == 9 {
            "vehicle_ac130_low_mp"
        } else {
            "vehicle_little_bird_armed"
        };
        let bounds = *r.model_bounds.get("com_plasticcase_friendly")?;
        let enemy_bounds = *r.model_bounds.get("com_plasticcase_enemy")?;
        let carrier_bounds = *r.model_bounds.get(name)?;
        if !bounds.valid() || !enemy_bounds.valid() || !carrier_bounds.valid() {
            return None;
        }
        let bounds = ModelBounds {
            min: std::array::from_fn(|i| bounds.min[i].min(enemy_bounds.min[i])),
            max: std::array::from_fn(|i| bounds.max[i].max(enemy_bounds.max[i])),
        };
        Some(Self {
            marker,
            carrier: *r.models.get(name)?,
            carrier_bounds,
            friendly: *r.models.get("com_plasticcase_friendly")?,
            enemy: *r.models.get("com_plasticcase_enemy")?,
            bounds,
            count: if id == 9 { 4 } else { 1 },
            height: r.air_route.as_ref()?.center[2],
        })
    }
}
#[derive(Clone, Copy)]
struct Route {
    start: [f32; 3],
    drop: [f32; 3],
    end: [f32; 3],
    inbound: u32,
    outbound: u32,
}
impl Route {
    fn duration(self) -> u32 {
        self.inbound.saturating_add(self.outbound)
    }
    fn pose(self, elapsed: u32) -> ([f32; 3], [u16; 3]) {
        let (a, b, f) = if elapsed <= self.inbound {
            (self.start, self.drop, elapsed as f32 / self.inbound as f32)
        } else {
            (
                self.drop,
                self.end,
                elapsed.saturating_sub(self.inbound).min(self.outbound) as f32
                    / self.outbound as f32,
            )
        };
        let origin = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f);
        let yaw = ((b[1] - a[1]).atan2(b[0] - a[0]) * 65536.0 / std::f32::consts::TAU).round()
            as i32 as u16;
        (origin, [0, yaw, 0])
    }
    fn plan(
        site: [f32; 3],
        height: f32,
        mega: bool,
        entity: u16,
        bounds: ModelBounds,
        collision: &Collision,
    ) -> Option<Self> {
        let distance = if mega { 24000.0 } else { 15000.0 };
        let (inbound, outbound) = if mega { (12000, 12000) } else { (11364, 2841) };
        for direction in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
            let drop = [site[0], site[1], height];
            let start = [
                site[0] - direction[0] * distance,
                site[1] - direction[1] * distance,
                height,
            ];
            let exit = if mega {
                direction
            } else {
                [-direction[1], direction[0]]
            };
            let end = [
                site[0] + exit[0] * distance,
                site[1] + exit[1] * distance,
                height,
            ];
            let route = Self {
                start,
                drop,
                end,
                inbound,
                outbound,
            };
            let clear = [(start, drop, 0), (drop, end, inbound + 1)]
                .into_iter()
                .all(|(a, b, t)| {
                    let (_, angles) = route.pose(t);
                    let shape = Collider::new(entity, [0.0; 3], angles, bounds.min, bounds.max);
                    let trace = collision.trace_bounds(a, b, shape.min, shape.max, Some(entity));
                    trace.fraction >= 1.0
                        && !trace.start_solid
                        && !trace.all_solid
                        && [a, b]
                            .into_iter()
                            .flatten()
                            .all(|v| v.is_finite() && v.abs() < 30000.0)
                });
            if clear {
                return Some(route);
            }
        }
        None
    }
}
struct Delivery {
    entity: u16,
    crates: Vec<u16>,
    owner: u8,
    team: u8,
    definition: Definition,
    launch: Option<u32>,
    start: Option<u32>,
    site: [f32; 3],
    released: usize,
    route: Option<Route>,
}
struct Crate {
    entity: u16,
    owner: u8,
    team: u8,
    definition: Definition,
    contents: Contents,
    origin: [f32; 3],
    velocity: f32,
    last: u32,
    expires: u32,
    landed: bool,
    hold: Option<Hold>,
    generation: u64,
}
#[derive(Default)]
pub(crate) struct Airdrops {
    deliveries: Vec<Delivery>,
    crates: Vec<Crate>,
    random: u32,
    now: u32,
    generation: u64,
    pickup_hints: [Option<PickupHint>; 12],
}
impl Airdrops {
    pub fn resources_present(r: &RadarResources, id: u8) -> bool {
        Definition::load(r, id).is_some()
    }
    pub fn reserve(&mut self, r: &RadarResources, id: u8, owner: u8, team: u8) -> Option<u16> {
        if owner >= 12 || !matches!(team, 1 | 2) {
            return None;
        }
        let d = Definition::load(r, id)?;
        if self
            .deliveries
            .iter()
            .filter(|d| d.definition.marker.marker_reward != Some(9))
            .count()
            >= 3
            && id != 9
        {
            return None;
        }
        let entity = (FIRST_CARRIER..END_CARRIER)
            .find(|e| !self.deliveries.iter().any(|d| d.entity == *e))?;
        let crates = (FIRST_CRATE..END_CRATE)
            .filter(|e| {
                !self.crates.iter().any(|c| c.entity == *e)
                    && !self.deliveries.iter().any(|d| d.crates.contains(e))
            })
            .take(d.count)
            .collect::<Vec<_>>();
        if crates.len() != d.count {
            return None;
        }
        self.deliveries.push(Delivery {
            entity,
            crates,
            owner,
            team,
            definition: d,
            launch: None,
            start: None,
            site: [0.0; 3],
            released: 0,
            route: None,
        });
        Some(entity)
    }
    pub fn cancel(&mut self, entity: u16) {
        self.deliveries.retain(|d| d.entity != entity);
    }
    pub fn launch(&mut self, entity: u16, time: u32) -> bool {
        let Some(d) = self
            .deliveries
            .iter_mut()
            .find(|d| d.entity == entity && d.launch.is_none())
        else {
            return false;
        };
        d.launch = Some(time);
        true
    }
    pub fn marker_landed(&mut self, e: &Explosion, collision: &Collision) -> bool {
        let Some(id) = e.definition.marker_reward else {
            return false;
        };
        let Some(index) = self.deliveries.iter().position(|d| {
            d.launch == Some(e.definition.marker_call)
                && d.definition.marker.marker_reward == Some(id)
                && d.start.is_none()
        }) else {
            return false;
        };
        let definition = &self.deliveries[index].definition;
        let route = Route::plan(
            e.origin,
            definition.height,
            id == 9,
            self.deliveries[index].entity,
            definition.carrier_bounds,
            collision,
        );
        let Some(route) = route else {
            let d = self.deliveries.remove(index);
            tracing::info!(owner=d.owner,id,origin=?e.origin,"airdrop transport rejected: complete route obstructed");
            return false;
        };
        let d = &mut self.deliveries[index];
        d.site = e.origin;
        d.start = Some(e.time);
        d.route = Some(route);
        tracing::info!(owner=d.owner,id,start=?route.start,drop=?route.drop,end=?route.end,"airdrop route accepted");
        true
    }
    fn draw(random: &mut u32, mega: bool) -> Contents {
        // AlphaBuild's configuration defaults, preserved as a complete pool.
        let choices = if mega {
            vec![
                (Contents::Ammo, 12),
                (Contents::Reward(1), 12),
                (Contents::Reward(3), 16),
                (Contents::Reward(16), 16),
                (Contents::Reward(5), 14),
                (Contents::Reward(6), 10),
                (Contents::Reward(7), 5),
                (Contents::Reward(8), 5),
                (Contents::Reward(10), 3),
                (Contents::Reward(11), 3),
                (Contents::Reward(12), 2),
                (Contents::Reward(13), 2),
            ]
        } else {
            vec![
                (Contents::Ammo, 17),
                (Contents::Reward(1), 17),
                (Contents::Reward(3), 15),
                (Contents::Reward(16), 12),
                (Contents::Reward(5), 12),
                (Contents::Reward(6), 11),
                (Contents::Reward(7), 7),
                (Contents::Reward(8), 7),
                (Contents::Reward(10), 5),
                (Contents::Reward(11), 5),
                (Contents::Reward(12), 3),
                (Contents::Reward(13), 3),
                (Contents::Reward(14), 1),
            ]
        };
        *random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        let mut value = *random % choices.iter().map(|(_, w)| w).sum::<u32>();
        for (c, w) in choices {
            if value < w {
                return c;
            }
            value -= w;
        }
        unreachable!()
    }
    pub fn advance(&mut self, now: u32, finished: bool, collision: &Collision) {
        self.now = now;
        if finished {
            self.deliveries.clear();
            self.crates.clear();
            self.pickup_hints = [None; 12];
            self.sync(collision);
            return;
        }
        let mut spawned = Vec::new();
        for d in &mut self.deliveries {
            let Some(start) = d.start else {
                continue;
            };
            let route = d.route.unwrap();
            while d.released < d.definition.count {
                let release_time = start
                    .saturating_add(route.inbound)
                    .saturating_add(d.released as u32 * 100);
                if now < release_time {
                    break;
                }
                let entity = d.crates[d.released];
                let contents = match d.definition.marker.marker_reward {
                    Some(4) => Contents::Reward(16),
                    Some(22) => Contents::Reward(21),
                    _ => Self::draw(&mut self.random, d.definition.count == 4),
                };
                let (carrier, angles) = route.pose(release_time.saturating_sub(start));
                let b = d.definition.bounds;
                let release = [
                    carrier[0],
                    carrier[1],
                    carrier[2] + d.definition.carrier_bounds.min[2] - b.max[2] - 10.0,
                ];
                let trace = collision.trace_bounds(release, release, b.min, b.max, Some(entity));
                if !trace.start_solid && !trace.all_solid {
                    self.generation = self
                        .generation
                        .checked_add(1)
                        .expect("airdrop generation exhausted");
                    spawned.push(Crate {
                        entity,
                        owner: d.owner,
                        team: d.team,
                        definition: d.definition.clone(),
                        contents,
                        origin: release,
                        velocity: 0.0,
                        last: release_time,
                        expires: release_time.saturating_add(120000),
                        landed: false,
                        hold: None,
                        generation: self.generation,
                    });
                    tracing::info!(now,entity,owner=d.owner,?contents,angles=?angles,"airdrop crate released");
                } else {
                    tracing::info!(now, entity, "airdrop crate release obstructed");
                }
                d.released += 1;
            }
        }
        self.crates.extend(spawned);
        self.deliveries.retain(|d| {
            d.start.map_or(
                d.launch.is_none_or(|t| now < t.saturating_add(30000)),
                |t| now < t.saturating_add(d.route.unwrap().duration()),
            )
        });
        for c in &mut self.crates {
            if c.hold.is_some_and(|h| now.saturating_sub(h.last) > 250) {
                c.hold = None;
            }
            while !c.landed && c.last < now {
                let msec = (now - c.last).min(10);
                let dt = msec as f32 * 0.001;
                c.last += msec;
                let mut next = c.origin;
                next[2] -= c.velocity * dt + 400.0 * dt * dt;
                c.velocity += 800.0 * dt;
                let trace = collision.trace_bounds(
                    c.origin,
                    next,
                    c.definition.bounds.min,
                    c.definition.bounds.max,
                    Some(c.entity),
                );
                if trace.start_solid || trace.all_solid {
                    c.expires = now;
                    c.hold = None;
                    break;
                }
                c.origin[2] += (next[2] - c.origin[2]) * trace.fraction;
                if trace.fraction < 1.0 {
                    if trace.normal[2] < 0.5 {
                        c.expires = now;
                        break;
                    }
                    c.origin[2] += 0.05;
                    c.landed = true;
                    c.velocity = 0.0;
                    c.expires = now.saturating_add(90000);
                    tracing::info!(now,entity=c.entity,origin=?c.origin,"airdrop crate landed");
                }
            }
        }
        self.crates
            .retain(|c| now < c.expires || (c.landed && c.hold.is_some()));
        self.sync(collision);
    }
    fn sync(&self, collision: &Collision) {
        let shapes = self
            .crates
            .iter()
            .map(|c| {
                Collider::new(
                    c.entity,
                    c.origin,
                    [0; 3],
                    c.definition.bounds.min,
                    c.definition.bounds.max,
                )
            })
            .collect::<Vec<_>>();
        collision.replace_prop_range(FIRST_CRATE..END_CRATE, &shapes);
        let carriers = self
            .deliveries
            .iter()
            .filter_map(|d| {
                d.start.map(|start| {
                    let (origin, angles) = d.route.unwrap().pose(self.now.saturating_sub(start));
                    Collider::new(
                        d.entity,
                        origin,
                        angles,
                        d.definition.carrier_bounds.min,
                        d.definition.carrier_bounds.max,
                    )
                })
            })
            .collect::<Vec<_>>();
        collision.replace_prop_range(FIRST_CARRIER..END_CARRIER, &carriers);
    }
    pub fn snapshots(&self, view_team: u8, now: u32) -> Vec<PropSnapshot> {
        let mut out = Vec::new();
        for d in &self.deliveries {
            let Some(start) = d.start else {
                continue;
            };
            let (origin, angles) = d.route.unwrap().pose(now.saturating_sub(start));
            out.push(PropSnapshot {
                actor: None,
                entity_number: d.entity,
                model: d.definition.carrier,
                origin,
                angles,
                present: true,
                weapon: 0,
                events: Default::default(),
                turret: false,
                turret_pose: None,
                vehicle: None,
            });
        }
        out.extend(self.crates.iter().map(|c| PropSnapshot {
            actor: None,
            entity_number: c.entity,
            model: if c.team == view_team {
                c.definition.friendly
            } else {
                c.definition.enemy
            },
            origin: c.origin,
            angles: [0; 3],
            present: true,
            weapon: 0,
            events: Default::default(),
            turret: false,
            turret_pose: None,
            vehicle: None,
        }));
        out.sort_by_key(|s| s.entity_number);
        out
    }
    pub fn hint(&self, actor: u8) -> Option<PickupHint> {
        self.pickup_hints.get(usize::from(actor)).copied().flatten()
    }
    /// A ready claim does not remove the box. The mode must first grant its
    /// contents and then consume this exact generation, so a failed grant
    /// cannot silently lose the reward.
    pub fn pickup(&mut self, now: u32, user: PickupUser, collision: &Collision) -> Option<Claim> {
        if user.actor >= 12 {
            return None;
        }
        self.pickup_hints[usize::from(user.actor)] = None;
        let nearest = if user.active
            && [user.feet, user.eye, user.forward]
                .into_iter()
                .flatten()
                .all(f32::is_finite)
        {
            self.crates
                .iter()
                .filter(|c| c.landed && now < c.expires)
                .filter_map(|c| {
                    let center = std::array::from_fn(|i| {
                        c.origin[i]
                            + (c.definition.bounds.min[i] + c.definition.bounds.max[i]) * 0.5
                    });
                    let distance = (0..3)
                        .map(|i| (user.feet[i] - center[i]).powi(2))
                        .sum::<f32>();
                    let toward: [f32; 3] = std::array::from_fn(|i| center[i] - user.eye[i]);
                    let length = toward.into_iter().map(|v| v * v).sum::<f32>().sqrt();
                    let cosine = (0..3).map(|i| toward[i] * user.forward[i]).sum::<f32>()
                        / length.max(0.001);
                    let trace = collision.raycast_excluding_prop(user.eye, center, None);
                    (distance <= 128.0 * 128.0
                        && cosine >= 0.3
                        && !trace.start_solid
                        && !trace.all_solid
                        && (trace.fraction >= 1.0 || trace.prop_entity == Some(c.entity)))
                    .then_some((c.entity, distance))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|p| p.0)
        } else {
            None
        };
        for c in &mut self.crates {
            if c.hold.is_some_and(|h| {
                now.saturating_sub(h.last) > 250
                    || (h.actor == user.actor
                        && (h.life != user.life
                            || !user.active
                            || !user.holding
                            || Some(c.entity) != nearest))
            }) {
                c.hold = None;
            }
            if Some(c.entity) != nearest {
                continue;
            }
            let duration = if user.actor == c.owner {
                500
            } else {
                user.steal_msec.clamp(500, 8000)
            };
            if user.holding && c.hold.is_none() {
                c.hold = Some(Hold {
                    actor: user.actor,
                    life: user.life,
                    start: now,
                    last: now,
                    duration,
                });
            }
            let mut elapsed = 0;
            if let Some(hold) = &mut c.hold
                && hold.actor == user.actor
                && hold.life == user.life
            {
                if now < hold.last {
                    return None;
                }
                hold.last = now;
                elapsed = now.saturating_sub(hold.start).min(hold.duration);
            }
            self.pickup_hints[usize::from(user.actor)] = Some(PickupHint {
                entity: c.entity,
                contents: c.contents,
                elapsed,
                duration,
            });
            if user.holding
                && c.hold
                    .is_some_and(|h| h.actor == user.actor && elapsed >= h.duration)
            {
                return Some(Claim {
                    entity: c.entity,
                    generation: c.generation,
                    actor: user.actor,
                    life: user.life,
                    start: c.hold.unwrap().start,
                    contents: c.contents,
                });
            }
        }
        None
    }
    pub fn claim(
        &mut self,
        claim: Claim,
        collision: &Collision,
        grant: impl FnOnce(Contents) -> bool,
    ) -> bool {
        let Some(index) = self.crates.iter().position(|c| {
            c.entity == claim.entity
                && c.generation == claim.generation
                && c.contents == claim.contents
                && c.hold.is_some_and(|h| {
                    h.actor == claim.actor
                        && h.life == claim.life
                        && h.start == claim.start
                        && h.last.saturating_sub(h.start) >= h.duration
                })
        }) else {
            return false;
        };
        if !grant(claim.contents) {
            return false;
        }
        let c = self.crates.remove(index);
        for hint in &mut self.pickup_hints {
            if hint.is_some_and(|h| h.entity == c.entity) {
                *hint = None;
            }
        }
        self.sync(collision);
        tracing::info!(entity=c.entity,?claim.contents,"airdrop crate pickup committed");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resources() -> RadarResources {
        let mut r = crate::game::weapons::grenade::marker_resources();
        for (name, id) in [
            ("vehicle_little_bird_armed", 142),
            ("vehicle_ac130_low_mp", 138),
            ("com_plasticcase_friendly", 103),
            ("com_plasticcase_enemy", 102),
        ] {
            r.models.insert(name.into(), id);
            r.model_bounds.insert(
                name.into(),
                ModelBounds {
                    min: [-10.0, -10.0, 0.0],
                    max: [10.0, 10.0, 20.0],
                },
            );
        }
        r.air_route = Some(crate::game::rewards::killstreaks::AirRoute {
            center: [0.0, 0.0, 500.0],
            sources_sha256: ["0".repeat(64)],
        });
        r
    }
    fn user(feet: [f32; 3], eye: [f32; 3]) -> PickupUser {
        PickupUser {
            actor: 0,
            life: 0,
            feet,
            eye,
            forward: [-0.4, 0.0, -0.92],
            active: true,
            holding: true,
            steal_msec: 8000,
        }
    }
    fn collision() -> Collision {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[10000,10000,10],"contents":1,"planes":[]}],"triangles":[]})).unwrap()).unwrap();
        Collision::load(f.path()).unwrap()
    }
    #[test]
    fn dedicated_device_drop_lands_and_hold_pickup_consumes_once() {
        let r = resources();
        let c = collision();
        let mut a = Airdrops::default();
        let entity = a.reserve(&r, 22, 0, 1).unwrap();
        assert!(a.launch(entity, 1000));
        assert!(!a.launch(entity, 1000));
        let mut d = Marker::marker(&r, 22).unwrap();
        d.marker_call = 1000;
        let e = Explosion {
            time: 1700,
            origin: [0.0, 0.0, 2.0],
            normal: [0.0, 0.0, 1.0],
            definition: d,
        };
        assert!(a.marker_landed(&e, &c));
        assert!(!a.marker_landed(&e, &c));
        a.advance(16000, false, &c);
        let p = a
            .snapshots(1, 16000)
            .into_iter()
            .find(|p| p.entity_number >= FIRST_CRATE)
            .unwrap();
        assert_eq!(p.model, 103);
        let feet = [p.origin[0] + 30.0, p.origin[1], 0.0];
        let eye = [feet[0], feet[1], 65.0];
        for now in (16100..16600).step_by(100) {
            assert_eq!(a.pickup(now, user(feet, eye), &c), None);
        }
        let claim = a.pickup(16600, user(feet, eye), &c).unwrap();
        assert_eq!(claim.contents, Contents::Reward(21));
        assert!(a.claim(claim, &c, |_| true));
        assert!(!a.claim(claim, &c, |_| panic!("replayed grant")));
        assert_eq!(a.pickup(17100, user(feet, eye), &c), None);
        a.advance(17200, true, &c);
        assert!(a.snapshots(1, 17200).is_empty());
    }
    fn landed_drop() -> (Airdrops, Collision, PickupUser) {
        let r = resources();
        let c = collision();
        let mut a = Airdrops::default();
        let entity = a.reserve(&r, 4, 0, 1).unwrap();
        assert!(a.launch(entity, 1000));
        let mut d = Marker::marker(&r, 4).unwrap();
        d.marker_call = 1000;
        assert!(a.marker_landed(
            &Explosion {
                time: 1700,
                origin: [0.0, 0.0, 2.0],
                normal: [0.0, 0.0, 1.0],
                definition: d
            },
            &c
        ));
        a.advance(16000, false, &c);
        let p = a
            .snapshots(1, 16000)
            .into_iter()
            .find(|p| p.entity_number >= FIRST_CRATE)
            .unwrap();
        let feet = [p.origin[0] + 30.0, p.origin[1], 0.0];
        let eye = [feet[0], feet[1], 65.0];
        (a, c, user(feet, eye))
    }
    #[test]
    fn stale_input_death_release_and_look_away_reset_continuous_pickup() {
        for failure in 0..4 {
            let (mut a, c, u) = landed_drop();
            for now in (16100..16500).step_by(100) {
                assert!(a.pickup(now, u, &c).is_none());
            }
            let mut changed = u;
            match failure {
                0 => changed.active = false,
                1 => changed.holding = false,
                2 => changed.life += 1,
                _ => changed.forward = [1.0, 0.0, 0.0],
            }
            assert!(a.pickup(16500, changed, &c).is_none());
            assert!(a.pickup(16600, u, &c).is_none());
            for now in (16700..17100).step_by(100) {
                assert!(a.pickup(now, u, &c).is_none());
            }
            assert!(a.pickup(17100, u, &c).is_some());
        }
        let (mut a, c, u) = landed_drop();
        assert!(a.pickup(16100, u, &c).is_none());
        assert!(a.pickup(16700, u, &c).is_none());
        assert_eq!(a.hint(0).unwrap().elapsed, 0);
    }
    #[test]
    fn failed_grant_keeps_box_and_expiry_clears_stale_claim_and_collision() {
        let (mut a, c, u) = landed_drop();
        for now in (16100..16600).step_by(100) {
            assert!(a.pickup(now, u, &c).is_none());
        }
        let claim = a.pickup(16600, u, &c).unwrap();
        assert!(!a.claim(claim, &c, |_| false));
        assert_eq!(
            a.snapshots(1, 16600)
                .iter()
                .filter(|p| p.entity_number >= FIRST_CRATE)
                .count(),
            1
        );
        a.advance(200000, false, &c);
        assert!(!a.claim(claim, &c, |_| panic!("expired grant")));
        assert!(a.snapshots(1, 200000).is_empty());
    }
    #[test]
    fn thief_requires_full_hold_and_a_prior_life_cannot_apply_a_ready_claim() {
        let (mut a, c, mut u) = landed_drop();
        u.actor = 1;
        for now in (16100..24100).step_by(100) {
            assert!(a.pickup(now, u, &c).is_none());
        }
        let claim = a.pickup(24100, u, &c).unwrap();
        u.life += 1;
        assert!(a.pickup(24200, u, &c).is_none());
        assert!(!a.claim(claim, &c, |_| panic!("old life received a new claim")));
        assert_eq!(a.hint(1).unwrap().elapsed, 0);
        assert_eq!(a.hint(1).unwrap().duration, 8000);
    }
    #[test]
    #[ignore = "requires matching target-build transport models and map collision capture"]
    fn captured_afghan_transports_have_complete_routes_and_crates_land_with_full_bounds() {
        let r = RadarResources::load(
            std::path::Path::new(&std::env::var("CODOL_MARKER_RESOURCES").unwrap()),
            "mp_afghan",
            523897474,
        )
        .unwrap();
        let c = Collision::load(std::path::Path::new(
            &std::env::var("CODOL_MARKER_COLLISION").unwrap(),
        ))
        .unwrap();
        for id in [2, 4, 9, 22] {
            let mut a = Airdrops::default();
            let entity = a.reserve(&r, id, 0, 1).unwrap();
            assert!(a.launch(entity, 1000));
            let mut d = Marker::marker(&r, id).unwrap();
            d.marker_call = 1000;
            let accepted = a.marker_landed(
                &Explosion {
                    time: 1720,
                    origin: [4101.2944, 2208.2507, 67.6967],
                    normal: [0.0, 0.0, 1.0],
                    definition: d,
                },
                &c,
            );
            assert!(accepted, "no transport route for id{id}");
            for now in (1800..40000).step_by(50) {
                a.advance(now, false, &c);
            }
            assert_eq!(a.crates.len(), if id == 9 { 4 } else { 1 });
            assert!(a.crates.iter().all(|c| c.landed));
            assert!(
                a.crates.iter().all(|c| c.origin[2] < 200.0),
                "boxes landed on an invisible airspace clip for id{id}"
            );
            println!(
                "id={id} boxes={:?}",
                a.crates.iter().map(|c| c.origin).collect::<Vec<_>>()
            );
            let boxes = a
                .crates
                .iter()
                .map(|b| (b.origin, b.contents))
                .collect::<Vec<_>>();
            for (index, (origin, contents)) in boxes.into_iter().enumerate() {
                let feet = [origin[0] + 60.0, origin[1], origin[2] - 15.0];
                let eye = [feet[0], feet[1], feet[2] + 60.0];
                let direction = [origin[0] - eye[0], origin[1] - eye[1], origin[2] - eye[2]];
                let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
                let mut u = user(feet, eye);
                u.forward = direction.map(|v| v / length);
                let start = 40100 + index as u32 * 1000;
                for now in (start..start + 500).step_by(100) {
                    assert!(a.pickup(now, u, &c).is_none());
                }
                let claim = a
                    .pickup(start + 500, u, &c)
                    .expect("landed crate cannot be picked up through full bounds");
                assert_eq!(claim.contents, contents);
                assert!(a.claim(claim, &c, |_| true));
                assert!(!a.claim(claim, &c, |_| panic!("real-map pickup replayed")));
            }
            assert!(a.crates.is_empty());
            a.advance(45000, true, &c);
        }
    }
    #[test]
    fn cancelled_reservations_release_transport_and_crate_capacity() {
        let r = resources();
        let mut a = Airdrops::default();
        for _ in 0..20 {
            let entity = a.reserve(&r, 4, 0, 1).unwrap();
            a.cancel(entity);
        }
        for _ in 0..3 {
            assert!(a.reserve(&r, 4, 0, 1).is_some());
        }
        assert!(a.reserve(&r, 4, 0, 1).is_none());
        assert!(a.reserve(&r, 9, 0, 1).is_some());
    }
}
