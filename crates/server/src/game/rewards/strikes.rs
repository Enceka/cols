//! Shared authoritative strike scheduler. Registrations and mortar damage are
//! target facts; airstrike damage/flight parameters follow AlphaBuild GSC.
//! Random offsets and pass spacing are deterministic compatibility choices.
use super::killstreaks::RadarResources;
use super::selection::{Point, ground};
use crate::game::world::{
    collision::Collision,
    props::{Blast, Event},
};
use codol_protocol::game::snapshot::{missile::MissileSnapshot, props::PropSnapshot};
use std::collections::VecDeque;

const FIRST_PLANE: u16 = 544;
const END_PLANE: u16 = 560;
pub(crate) const FIRST_BOMB: u16 = 1600;
pub(crate) const END_BOMB: u16 = 1648;
const GRAVITY: f32 = 800.0;
#[derive(Clone, Copy)]
pub(crate) struct Source {
    pub owner: u8,
    pub team: u8,
    pub life: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct Hit {
    pub blast: Blast,
    pub team: u8,
    pub life: u32,
    pub airstrike: bool,
    pub area: Option<([f32; 2], f32)>,
}
pub(crate) fn coverage_radius(id: u8) -> f32 {
    match id {
        6 | 7 => 1100.0,
        11 => 2600.0,
        _ => 800.0,
    }
}
struct Impact {
    time: u32,
    hit: Hit,
    fx: u16,
}
struct Plane {
    entity: u16,
    model: u16,
    start: u32,
    end: u32,
    from: [f32; 3],
    to: [f32; 3],
    angles: [u16; 3],
}
impl Plane {
    fn snapshot(&self, now: u32) -> Option<PropSnapshot> {
        if now < self.start || now >= self.end {
            return None;
        }
        let f = (now - self.start) as f32 / (self.end - self.start) as f32;
        let origin: [f32; 3] =
            std::array::from_fn(|i| self.from[i] + (self.to[i] - self.from[i]) * f);
        // The original stealth route ends outside16-bit world coordinates.
        // It keeps its full timing and leaves replication after exiting range.
        if origin.iter().any(|v| v.abs() > 32760.0) {
            return None;
        }
        Some(PropSnapshot {
            actor: None,
            entity_number: self.entity,
            model: self.model,
            origin,
            angles: self.angles,
            present: true,
            weapon: 0,
            turret: false,
            turret_pose: None,
            vehicle: None,
            events: Default::default(),
        })
    }
}
struct Bomb {
    snapshot: MissileSnapshot,
    launch: u32,
    last: u32,
    velocity: [f32; 3],
    hit: Hit,
    fx: u16,
}
pub(crate) struct Plan {
    owner: u8,
    team: u8,
    planes: Vec<Plane>,
    bombs: Vec<Bomb>,
    impacts: Vec<Impact>,
}
struct Strike {
    plan: Plan,
}
#[derive(Default)]
pub(crate) struct Strikes {
    active: Vec<Strike>,
    generation: u32,
    events: VecDeque<Event>,
}
impl Strikes {
    pub fn clear(&mut self) {
        self.active.clear();
        self.events.clear();
    }
    pub fn resources_present(r: &RadarResources, id: u8) -> bool {
        let shader = if id == 17 {
            "map_mortar_selector"
        } else {
            "map_artillery_selector"
        };
        if !r.selectors.contains_key(shader) || r.air_route.is_none() || r.radio(id).is_none() {
            return false;
        }
        match id {
            6 | 7 => {
                r.models.contains_key(if id == 7 {
                    "vehicle_av8b_harrier_jet_mp"
                } else {
                    "vehicle_mig29_desert"
                }) && r.weapons.contains_key("artillery_mp")
                    && r.effects.contains_key("explosions/artilleryexp_dirt_brown")
            }
            11 => {
                r.models.contains_key("vehicle_b2_bomber")
                    && r.weapons.contains_key("stealth_bomb_mp")
                    && r.effects.contains_key("explosions/artilleryexp_dirt_brown")
            }
            17 => {
                r.weapons.get("mortar_mp").is_some_and(|w| {
                    w.weapon_type == 4
                        && w.radius > 0
                        && w.radius <= 8192
                        && w.inner_damage > 0
                        && w.inner_damage <= 65535
                        && w.outer_damage <= w.inner_damage
                }) && r.effects.contains_key("explosions/artilleryexp_dirt_brown")
            }
            _ => false,
        }
    }
    pub fn plan(
        &self,
        now: u32,
        id: u8,
        source: Source,
        points: &[Point],
        r: &RadarResources,
        c: &Collision,
    ) -> Option<Plan> {
        let Source { owner, team, life } = source;
        if !Self::resources_present(r, id)
            || owner >= 12
            || !matches!(team, 1 | 2)
            || points.len() != if id == 17 { 3 } else { 1 }
            || points.iter().any(|p| {
                !p.yaw.is_finite() || p.origin.iter().any(|v| !v.is_finite() || v.abs() > 30000.0)
            })
        {
            return None;
        }
        let fx = *r.effects.get("explosions/artilleryexp_dirt_brown")?;
        let height = r.air_route.as_ref()?.center[2];
        let mut plan = Plan {
            owner,
            team,
            planes: Vec::new(),
            bombs: Vec::new(),
            impacts: Vec::new(),
        };
        if id == 17 {
            let slots: Vec<_> = (FIRST_BOMB..END_BOMB)
                .filter(|id| {
                    !self
                        .active
                        .iter()
                        .any(|s| s.plan.bombs.iter().any(|b| b.snapshot.entity_number == *id))
                })
                .take(6)
                .collect();
            if slots.len() != 6 {
                return None;
            }
            let w = r.weapons.get("mortar_mp")?;
            for (i, p) in points.iter().enumerate() {
                for shell in 0..2 {
                    let launch = now + i as u32 * 2500 + shell * 1300;
                    let (origin, velocity) = mortar_trajectory(p.origin, p.yaw);
                    if origin.iter().any(|v| v.abs() > 32760.0) {
                        return None;
                    }
                    let entity = slots[i * 2 + shell as usize];
                    plan.bombs.push(Bomb {
                        snapshot: MissileSnapshot {
                            events: Default::default(),
                            entity_number: entity,
                            weapon: w.weapon,
                            owner,
                            generation: 0,
                            present: true,
                            origin,
                            angles: aim(velocity),
                        },
                        launch,
                        last: launch,
                        velocity,
                        hit: Hit {
                            blast: Blast {
                                reward_life: Some(life),
                                owner,
                                weapon: w.weapon,
                                source_entity: Some(entity),
                                origin,
                                radius: w.radius as f32,
                                inner: w.inner_damage as u16,
                                outer: w.outer_damage as u16,
                            },
                            team,
                            life,
                            airstrike: false,
                            area: None,
                        },
                        fx,
                    });
                }
            }
        } else {
            let p = points[0];
            let coverage = p.coverage.unwrap_or_else(|| coverage_radius(id));
            if !coverage.is_finite() || !(32.0..=16000.0).contains(&coverage) {
                return None;
            }
            let count = match id {
                6 => 3,
                7 => 2,
                _ => 1,
            };
            let slots: Vec<_> = (FIRST_PLANE..END_PLANE)
                .filter(|id| {
                    !self
                        .active
                        .iter()
                        .any(|s| s.plan.planes.iter().any(|p| p.entity == *id))
                })
                .take(count)
                .collect();
            if slots.len() != count {
                return None;
            }
            let (sin, cos) = p.yaw.to_radians().sin_cos();
            let (model, weapon, half, speed, end_scale, radius, inner, outer) =
                if matches!(id, 6 | 7) {
                    (
                        *r.models.get(if id == 7 {
                            "vehicle_av8b_harrier_jet_mp"
                        } else {
                            "vehicle_mig29_desert"
                        })?,
                        r.weapons.get("artillery_mp")?.weapon,
                        24000.0,
                        7000.0,
                        1.0,
                        512.0_f32,
                        200,
                        30,
                    )
                } else {
                    (
                        *r.models.get("vehicle_b2_bomber")?,
                        r.weapons.get("stealth_bomb_mp")?.weapon,
                        12000.0,
                        2000.0,
                        4.0,
                        896.0_f32,
                        300,
                        50,
                    )
                };
            let radius = radius.min(coverage);
            for (pass, entity) in slots.into_iter().enumerate() {
                let start = now + pass as u32 * 2000;
                let from = [p.origin[0] - cos * half, p.origin[1] - sin * half, height];
                let to = [
                    p.origin[0] + cos * half * end_scale,
                    p.origin[1] + sin * half * end_scale,
                    height,
                ];
                let end = start + ((half * (1.0 + end_scale) / speed) * 1000.0) as u32;
                plan.planes.push(Plane {
                    entity,
                    model,
                    start,
                    end,
                    from,
                    to,
                    angles: [0, angle(p.yaw), 0],
                });
                let hit = Hit {
                    blast: Blast {
                        reward_life: Some(life),
                        owner,
                        weapon,
                        source_entity: Some(entity),
                        origin: p.origin,
                        radius,
                        inner,
                        outer,
                    },
                    team,
                    life,
                    airstrike: true,
                    area: Some(([p.origin[0], p.origin[1]], coverage)),
                };
                if matches!(id, 6 | 7) {
                    // Alpha cluster bomb release at bombTime-1, then gravity
                    // travel1.0s, followed by12 descending raycasts55..9deg.
                    let explode_distance = 850.0 / height.max(1.0) * 1500.0;
                    let release = (half + explode_distance) / speed - 1.0;
                    let bomb_forward = -half + release * speed + 7000.0 / 1.5;
                    let bomb = [
                        p.origin[0] + cos * bomb_forward,
                        p.origin[1] + sin * bomb_forward,
                        height - 0.5 * GRAVITY,
                    ];
                    for n in 0..12 {
                        let pitch = (55.0 - 50.0 / 12.0 * n as f32).to_radians();
                        let ray_end = [
                            bomb[0] + cos * pitch.cos() * 10000.0,
                            bomb[1] + sin * pitch.cos() * 10000.0,
                            bomb[2] - pitch.sin() * 10000.0,
                        ];
                        let t = c.raycast(bomb, ray_end);
                        if t.start_solid || t.all_solid || t.fraction >= 1.0 {
                            continue;
                        }
                        let projected: [f32; 3] =
                            std::array::from_fn(|i| bomb[i] + (ray_end[i] - bomb[i]) * t.fraction);
                        let along =
                            (projected[0] - p.origin[0]) * cos + (projected[1] - p.origin[1]) * sin;
                        let extent = (coverage - radius).max(0.0);
                        // Preserve the ordered12-bomb fan within the selected
                        // circle instead of letting its far rays leave the UI.
                        let along = (along / 5500.0).clamp(-1.0, 1.0) * extent;
                        let Some(origin) = ground(
                            [p.origin[0] + cos * along, p.origin[1] + sin * along],
                            height,
                            c,
                        ) else {
                            continue;
                        };
                        let mut impact_hit = hit;
                        impact_hit.blast.origin = [origin[0], origin[1], origin[2] + 16.0];
                        plan.impacts.push(Impact {
                            time: start + ((release + 2.2) * 1000.0) as u32 + n * 50,
                            hit: impact_hit,
                            fx,
                        });
                    }
                } else {
                    // Keep the script's200-unit spacing and radial scatter,
                    // trimming the drop segment to the selected world circle.
                    let scatter_limit = (coverage - radius).clamp(0.0, 512.0);
                    let extent = (coverage - radius - scatter_limit).max(0.0);
                    let count = ((extent * 2.0 / 200.0).floor() as u32 + 1).clamp(1, 45);
                    for n in 0..count {
                        let along = -extent + n as f32 * 200.0;
                        let scatter_yaw = (n as f32 * 137.508 + p.yaw).to_radians();
                        let scatter = (n % 8) as f32 / 8.0 * scatter_limit;
                        let xy = [
                            p.origin[0] + cos * along + scatter_yaw.cos() * scatter,
                            p.origin[1] + sin * along + scatter_yaw.sin() * scatter,
                        ];
                        let Some(origin) = ground(xy, height, c) else {
                            continue;
                        };
                        let fall = height - origin[2];
                        if fall > 5000.0 {
                            continue;
                        }
                        let mut impact_hit = hit;
                        impact_hit.blast.origin = [origin[0], origin[1], origin[2] + 16.0];
                        plan.impacts.push(Impact {
                            time: start
                                + ((half + along) / speed * 1000.0 + 0.85 * fall / 2000.0 * 1000.0)
                                    as u32,
                            hit: impact_hit,
                            fx,
                        });
                    }
                }
            }
            if plan.impacts.is_empty() {
                return None;
            }
        }
        plan.impacts.sort_by_key(|i| i.time);
        Some(plan)
    }
    pub fn launch(&mut self, mut plan: Plan) {
        for b in &mut plan.bombs {
            self.generation = self.generation.wrapping_add(1);
            b.snapshot.generation = self.generation;
        }
        tracing::info!(
            owner = plan.owner,
            team = plan.team,
            planes = plan.planes.len(),
            bombs = plan.bombs.len(),
            impacts = plan.impacts.len(),
            "location strike launched"
        );
        self.active.push(Strike { plan });
    }
    pub fn advance(&mut self, now: u32, finished: bool, emp: u8, c: &Collision) -> Vec<Hit> {
        let mut hits = Vec::new();
        if finished {
            self.clear();
            return hits;
        }
        for s in &mut self.active {
            if emp & (1 << s.plan.team) != 0 {
                s.plan.planes.clear();
                s.plan.impacts.clear();
            }
            s.plan.planes.retain(|p| p.end > now);
            while s.plan.impacts.first().is_some_and(|i| i.time <= now) {
                let impact = s.plan.impacts.remove(0);
                self.events.push_back(Event::Fx {
                    selector: impact.fx,
                    origin: [
                        impact.hit.blast.origin[0],
                        impact.hit.blast.origin[1],
                        impact.hit.blast.origin[2] - 14.0,
                    ],
                });
                hits.push(impact.hit);
            }
            let mut keep = Vec::new();
            for mut b in s.plan.bombs.drain(..) {
                if emp & (1 << s.plan.team) != 0 && now < b.launch {
                    continue;
                }
                if now < b.launch {
                    keep.push(b);
                    continue;
                }
                let mut contact = None;
                while b.last < now.min(b.launch + 30000) {
                    let step = (now.min(b.launch + 30000) - b.last).min(20);
                    let dt = step as f32 * 0.001;
                    let mut next =
                        std::array::from_fn(|i| b.snapshot.origin[i] + b.velocity[i] * dt);
                    next[2] -= 0.5 * GRAVITY * dt * dt;
                    let trace = c.projectile_trace(b.snapshot.origin, next, 2.0);
                    b.last += step;
                    b.velocity[2] -= GRAVITY * dt;
                    if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
                        let origin: [f32; 3] = std::array::from_fn(|i| {
                            b.snapshot.origin[i] + (next[i] - b.snapshot.origin[i]) * trace.fraction
                        });
                        contact = Some(std::array::from_fn(|i| origin[i] + trace.normal[i] * 3.0));
                        break;
                    }
                    b.snapshot.origin = next;
                    b.snapshot.angles = aim(b.velocity);
                }
                if let Some(origin) = contact {
                    b.hit.blast.origin = origin;
                    hits.push(b.hit);
                    self.events.push_back(Event::Fx {
                        selector: b.fx,
                        origin,
                    });
                } else if now < b.launch + 30000
                    && b.snapshot.origin.iter().all(|v| v.abs() < 32760.0)
                {
                    keep.push(b);
                }
            }
            s.plan.bombs = keep;
        }
        self.active.retain(|s| {
            !s.plan.planes.is_empty() || !s.plan.bombs.is_empty() || !s.plan.impacts.is_empty()
        });
        hits
    }
    pub fn planes(&self, now: u32) -> Vec<PropSnapshot> {
        self.active
            .iter()
            .flat_map(|s| s.plan.planes.iter().filter_map(|p| p.snapshot(now)))
            .collect()
    }
    pub fn missiles(&self, now: u32) -> Vec<MissileSnapshot> {
        self.active
            .iter()
            .flat_map(|s| {
                s.plan
                    .bombs
                    .iter()
                    .filter(|b| now >= b.launch)
                    .map(|b| b.snapshot)
            })
            .collect()
    }
    pub fn drain(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
}
fn angle(degrees: f32) -> u16 {
    (degrees * 65536.0 / 360.0).round() as i32 as u16
}
fn aim(v: [f32; 3]) -> [u16; 3] {
    [
        angle(-v[2].atan2(v[0].hypot(v[1])).to_degrees()),
        angle(v[1].atan2(v[0]).to_degrees()),
        0,
    ]
}
fn mortar_trajectory(target: [f32; 3], yaw: f32) -> ([f32; 3], [f32; 3]) {
    let pitch = 75.0f32.to_radians();
    let speed = (GRAVITY * 6000.0 / (2.0 * pitch).sin()).sqrt();
    let horizontal = pitch.cos() * speed;
    let vertical = pitch.sin() * speed;
    let elapsed = 6000.0 / horizontal * 0.3;
    let (sin, cos) = yaw.to_radians().sin_cos();
    (
        [
            target[0] - cos * 4200.0,
            target[1] - sin * 4200.0,
            target[2] + vertical * elapsed - 0.5 * GRAVITY * elapsed * elapsed,
        ],
        [
            cos * horizontal,
            sin * horizontal,
            vertical - GRAVITY * elapsed,
        ],
    )
}
/// Alpha airstrike's indoor advantage: two obstruction tests multiply the
/// distance by4. Mortar uses ordinary native projectile line-of-sight damage.
pub(crate) fn damage(hit: Hit, origin: [f32; 3], height: f32, c: &Collision) -> u16 {
    if hit.area.is_some_and(|(center, radius)| {
        (origin[0] - center[0]).hypot(origin[1] - center[1]) > radius
    }) {
        return 0;
    }
    if !hit.airstrike {
        return crate::game::world::props::radius_damage(hit.blast, origin, height, None, c);
    }
    let center = [origin[0], origin[1], origin[2] + height];
    let mut distance = (0..3)
        .map(|i| (center[i] - hit.blast.origin[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    let roof = [origin[0], origin[1], origin[2] + 130.0];
    let overhead = c.raycast(origin, roof);
    if overhead.start_solid || overhead.fraction < 1.0 {
        let path = c.raycast(
            roof,
            [
                hit.blast.origin[0],
                hit.blast.origin[1],
                hit.blast.origin[2] + 114.0,
            ],
        );
        if path.start_solid || path.fraction < 1.0 {
            distance *= 4.0;
        }
    }
    if distance >= hit.blast.radius {
        return 0;
    }
    (f32::from(hit.blast.inner)
        + (f32::from(hit.blast.outer) - f32::from(hit.blast.inner)) * distance / hit.blast.radius)
        .floor()
        .max(0.0) as u16
}
