//! Native animal rewards. Captured model/tree/weapon facts are independent of
//! the Alpha-compatible eight-dog pack, four active dogs and 60-second limit.
//! Spawn rings,250u/s walking,18u steps and56u bite reach are local rules.
use super::{
    killstreaks::{AnimationBinding, ModelBounds, RadarResources},
    strikes::Source,
};
use crate::game::{
    ai::navigation::Navigation,
    combat::Actor,
    world::{
        collision::Collision,
        math::*,
        props::{Blast, Collider},
    },
};
use codol_protocol::game::snapshot::props::{ActorPose, PropSnapshot};
use serde::Deserialize;
use std::collections::{BTreeMap, VecDeque};

pub(crate) const FIRST_ENTITY: u16 = 656;
pub(crate) const END_ENTITY: u16 = 688;
const SKIN: f32 = 0.125;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resources {
    pub mask: u8,
    pub selector: u8,
    pub clips: BTreeMap<String, AnimationBinding>,
    pub ancestors: BTreeMap<String, Vec<u16>>,
    pub durations: BTreeMap<String, u32>,
    pub rawfiles_sha256: BTreeMap<String, String>,
}
impl Resources {
    pub fn valid(&self) -> bool {
        self.mask == 2
            && self.selector == 1
            && self.clips.len() == 4
            && ["idle", "run", "bite", "death"].into_iter().all(|n| {
                self.clips.get(n).is_some_and(|a| {
                    (1..16).contains(&a.tree)
                        && (1..2048).contains(&a.node)
                        && a.parent < 2048
                        && a.parent != a.node
                })
            })
            && self
                .clips
                .values()
                .all(|a| a.tree == self.clips["idle"].tree)
            && self.ancestors.len() == 4
            && self.durations.len() == 4
            && self.clips.iter().all(|(name, a)| {
                self.ancestors.get(name).is_some_and(|chain| {
                    chain.len() <= 4
                        && chain.iter().all(|n| *n > 0 && *n < 2048 && *n != a.node)
                        && chain
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            == chain.len()
                        && chain.last().copied().unwrap_or(0) == a.parent
                }) && self
                    .durations
                    .get(name)
                    .is_some_and(|t| (1..=15000).contains(t))
            })
            && self.rawfiles_sha256.len() == 2
            && ["animlists/common_dog.animlist", "animtrees/dog.atr"]
                .into_iter()
                .all(|n| {
                    self.rawfiles_sha256
                        .get(n)
                        .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                })
    }
}
#[derive(Clone)]
struct Definition {
    model: u16,
    weapon: u16,
    damage: u16,
    bite_delay: u32,
    bounds: ModelBounds,
    animation: Resources,
}
impl Definition {
    fn load(r: &RadarResources) -> Option<Self> {
        let animation = r.animals.clone()?;
        let bounds = *r.model_bounds.get("german_sheperd_dog")?;
        let w = r.weapons.get("dog_bite")?;
        if !animation.valid()
            || !bounds.valid()
            || !(1..900).contains(&w.weapon)
            || !(1..=65535).contains(&w.damage_stages[0])
            || w.fire_delay_msec == 0
            || u32::from(w.fire_delay_msec) >= animation.durations["bite"]
        {
            return None;
        }
        Some(Self {
            model: *r.models.get("german_sheperd_dog")?,
            weapon: w.weapon,
            damage: w.damage_stages[0] as u16,
            bite_delay: u32::from(w.fire_delay_msec),
            bounds,
            animation,
        })
    }
    fn hull(&self) -> (V3, V3) {
        let radius = self.bounds.min[0]
            .abs()
            .max(self.bounds.max[0].abs())
            .hypot(self.bounds.min[1].abs().max(self.bounds.max[1].abs()));
        (
            [-radius, -radius, 0.0],
            [radius, radius, self.bounds.max[2] - self.bounds.min[2]],
        )
    }
}
pub(crate) struct Plan {
    definition: Definition,
    spawns: Vec<V3>,
}
struct Pack {
    definition: Definition,
    source: Source,
    spawns: Vec<V3>,
    remaining: u8,
    end: u32,
    next_spawn: u32,
}
struct Dog {
    entity: u16,
    feet: V3,
    yaw: u16,
    health: u16,
    dead_until: u32,
    source: Source,
    definition: Definition,
    animation: &'static str,
    target: Option<(u8, u32)>,
    next_path: u32,
    path: VecDeque<V3>,
    next_bite: u32,
    pending: Option<(u8, u32, u32)>,
    damaged: u32,
    last: u32,
}
#[derive(Clone, Copy)]
pub(crate) struct Target {
    pub actor: Actor,
    pub life: u32,
    pub born: u32,
}
pub(crate) struct Bite {
    pub owner: u8,
    pub life: u32,
    pub weapon: u16,
    pub target: u8,
    pub target_life: u32,
    pub damage: u16,
}
#[derive(Default)]
pub(crate) struct Dogs {
    packs: Vec<Pack>,
    dogs: Vec<Dog>,
}
impl Dogs {
    pub fn clear(&mut self, c: &Collision) {
        self.packs.clear();
        self.dogs.clear();
        self.sync(c);
    }
    pub fn resources_present(r: &RadarResources) -> bool {
        Definition::load(r).is_some()
    }
    pub fn plan(&self, r: &RadarResources, origin: V3, team: u8, c: &Collision) -> Option<Plan> {
        if !matches!(team, 1 | 2)
            || !self.packs.is_empty()
            || origin.iter().any(|v| !v.is_finite() || v.abs() > 32700.0)
        {
            return None;
        }
        let definition = Definition::load(r)?;
        let mut spawns = Vec::new();
        for ring in [144.0, 240.0, 336.0] {
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::TAU / 8.0;
                let candidate = add(origin, [angle.cos() * ring, angle.sin() * ring, 0.0]);
                if let Some(p) = ground(candidate, &definition, None, c) {
                    spawns.push(p);
                }
            }
            if spawns.len() >= 4 {
                break;
            }
        }
        (spawns.len() >= 4).then_some(Plan { definition, spawns })
    }
    pub fn launch(&mut self, now: u32, plan: Plan, source: Source) {
        self.packs.push(Pack {
            definition: plan.definition,
            source,
            spawns: plan.spawns,
            remaining: 8,
            end: now.saturating_add(60000),
            next_spawn: now,
        });
    }
    pub fn damage(&mut self, now: u32, entity: u16, damage: u16, team: u8, c: &Collision) -> bool {
        let Some(d) = self.dogs.iter_mut().find(|d| d.entity == entity) else {
            return false;
        };
        if d.source.team != team && d.health > 0 {
            d.health = d.health.saturating_sub(damage);
            if damage > 0 {
                d.damaged = now;
            }
            if d.health == 0 {
                d.animation = "death";
                d.pending = None;
                d.dead_until = now.saturating_add(d.definition.animation.durations["death"]);
            }
        }
        self.sync(c);
        true
    }
    pub fn radial(&mut self, now: u32, blast: Blast, team: u8, c: &Collision) {
        let hits = self
            .dogs
            .iter()
            .filter(|d| d.health > 0 && d.source.team != team)
            .map(|d| {
                (
                    d.entity,
                    crate::game::world::props::radius_damage(
                        blast,
                        d.feet,
                        20.0,
                        Some(d.entity),
                        c,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (entity, damage) in hits {
            self.damage(now, entity, damage, team, c);
        }
    }
    pub fn advance(
        &mut self,
        now: u32,
        finished: bool,
        targets: &[Target],
        nav: &Navigation,
        c: &Collision,
    ) -> Vec<Bite> {
        if finished {
            self.packs.clear();
            self.dogs.clear();
            self.sync(c);
            return Vec::new();
        }
        self.packs.retain(|p| now < p.end);
        self.dogs.retain(|d| {
            self.packs
                .iter()
                .any(|p| p.source.owner == d.source.owner && p.source.life == d.source.life)
                && (d.health > 0 || now < d.dead_until)
        });
        for p in &mut self.packs {
            let active = self
                .dogs
                .iter()
                .filter(|d| {
                    d.source.owner == p.source.owner
                        && d.source.life == p.source.life
                        && d.health > 0
                })
                .count();
            if active >= 4 || p.remaining == 0 || now < p.next_spawn {
                continue;
            }
            let Some(entity) =
                (FIRST_ENTITY..END_ENTITY).find(|e| self.dogs.iter().all(|d| d.entity != *e))
            else {
                continue;
            };
            let Some(feet) = p.spawns.iter().copied().find(|&pos| {
                self.dogs.iter().all(|d| length(sub(d.feet, pos)) > 96.0)
                    && ground(pos, &p.definition, None, c).is_some()
            }) else {
                continue;
            };
            self.dogs.push(Dog {
                entity,
                feet,
                yaw: 0,
                health: 100,
                dead_until: 0,
                source: p.source,
                definition: p.definition.clone(),
                animation: "idle",
                target: None,
                next_path: 0,
                path: VecDeque::new(),
                next_bite: now,
                pending: None,
                damaged: now,
                last: now,
            });
            p.remaining -= 1;
            p.next_spawn = now.saturating_add(500);
        }
        let mut assigned = BTreeMap::<(u8, u32), usize>::new();
        let mut bites = Vec::new();
        for d in self.dogs.iter_mut().filter(|d| d.health > 0) {
            if now.saturating_sub(d.damaged) >= 5000 {
                d.health = 100;
            }
            if let Some((actor, life, hit_at)) = d.pending
                && now >= hit_at
            {
                if let Some(t) = targets.iter().find(|t| {
                    t.actor.client == actor && t.life == life && t.actor.team != d.source.team
                }) && can_bite(d, t, c)
                {
                    bites.push(Bite {
                        owner: d.source.owner,
                        life: d.source.life,
                        weapon: d.definition.weapon,
                        target: actor,
                        target_life: life,
                        damage: d.definition.damage,
                    });
                }
                d.pending = None;
            }
            if now < d.next_bite {
                if let Some(identity) = d.target {
                    *assigned.entry(identity).or_default() += 1;
                }
                d.animation = "bite";
                d.last = now;
                continue;
            }
            let target = targets
                .iter()
                .filter(|t| {
                    t.actor.team != d.source.team
                        && now.saturating_sub(t.born) >= 2000
                        && assigned
                            .get(&(t.actor.client, t.life))
                            .copied()
                            .unwrap_or(0)
                            < 2
                })
                .min_by(|a, b| {
                    length(sub(a.actor.origin, d.feet))
                        .total_cmp(&length(sub(b.actor.origin, d.feet)))
                });
            let Some(t) = target else {
                d.animation = "idle";
                d.target = None;
                d.path.clear();
                d.last = now;
                continue;
            };
            let identity = (t.actor.client, t.life);
            *assigned.entry(identity).or_default() += 1;
            if d.target != Some(identity) {
                d.target = Some(identity);
                d.next_path = 0;
                d.path.clear();
            }
            let delta = sub(t.actor.origin, d.feet);
            d.yaw = angle(delta[1].atan2(delta[0]).to_degrees());
            if can_bite(d, t, c) {
                d.animation = "bite";
                d.last = now;
                d.pending = Some((
                    identity.0,
                    identity.1,
                    now.saturating_add(d.definition.bite_delay),
                ));
                d.next_bite = now.saturating_add(d.definition.animation.durations["bite"]);
                continue;
            }
            if now >= d.next_path {
                d.path = nav
                    .route_walking(d.feet, t.actor.origin, c, |a, b| {
                        walk(a, b, &d.definition, Some(d.entity), c)
                            .is_some_and(|p| length(sub(p, b)) < 8.0)
                    })
                    .into_iter()
                    .map(|p| p.origin)
                    .collect();
                d.next_path = now.saturating_add(1000);
            }
            while d
                .path
                .front()
                .is_some_and(|p| length(sub(*p, d.feet)) < 12.0)
            {
                d.path.pop_front();
            }
            let mut moving = false;
            while d.last < now {
                let dt = (now - d.last).min(20);
                d.last += dt;
                let Some(&goal) = d.path.front() else {
                    break;
                };
                let delta = sub(goal, d.feet);
                let distance = delta[0].hypot(delta[1]);
                let step = (250.0 * dt as f32 * 0.001).min(distance);
                if distance < 1.0 {
                    d.path.pop_front();
                    continue;
                }
                let end = add(
                    d.feet,
                    [delta[0] / distance * step, delta[1] / distance * step, 0.0],
                );
                if let Some(p) = walk(d.feet, end, &d.definition, Some(d.entity), c) {
                    moving |= length(sub(p, d.feet)) > 0.1;
                    d.feet = p;
                    d.yaw = angle(delta[1].atan2(delta[0]).to_degrees());
                } else {
                    d.path.clear();
                    d.next_path = now.saturating_add(250);
                    break;
                }
            }
            d.last = now;
            d.animation = if moving { "run" } else { "idle" };
        }
        self.sync(c);
        bites
    }
    fn sync(&self, c: &Collision) {
        let shapes = self
            .dogs
            .iter()
            .filter(|d| d.health > 0)
            .map(|d| {
                let (min, max) = d.definition.hull();
                Collider::new(d.entity, d.feet, [0; 3], min, max)
            })
            .collect::<Vec<_>>();
        c.replace_prop_range(FIRST_ENTITY..END_ENTITY, &shapes);
    }
    pub fn snapshots(&self) -> Vec<PropSnapshot> {
        let mut result = self
            .dogs
            .iter()
            .map(|d| {
                let a = d.definition.animation.clips[d.animation];
                let mut ancestors = [0; 4];
                for (slot, &node) in ancestors
                    .iter_mut()
                    .zip(&d.definition.animation.ancestors[d.animation])
                {
                    *slot = node;
                }
                PropSnapshot {
                    entity_number: d.entity,
                    model: d.definition.model,
                    origin: add(d.feet, [0.0, 0.0, -d.definition.bounds.min[2]]),
                    angles: [0, d.yaw, 0],
                    present: true,
                    weapon: d.definition.weapon,
                    events: Default::default(),
                    turret: false,
                    turret_pose: None,
                    vehicle: None,
                    actor: Some(ActorPose {
                        team: d.source.team,
                        slot: (d.entity - FIRST_ENTITY) as u8,
                        selector: 1,
                        animation: a.node,
                        animation_parent: a.parent,
                        animation_ancestors: ancestors,
                    }),
                }
            })
            .collect::<Vec<_>>();
        result.sort_by_key(|p| p.entity_number);
        result
    }
}
fn can_bite(d: &Dog, t: &Target, c: &Collision) -> bool {
    let delta = sub(t.actor.origin, d.feet);
    if delta[0].hypot(delta[1]) > 56.0 || delta[2].abs() > 40.0 {
        return false;
    }
    let trace = c.raycast_excluding_prop(
        add(d.feet, [0.0, 0.0, 20.0]),
        add(t.actor.origin, [0.0, 0.0, t.actor.height * 0.5]),
        Some(d.entity),
    );
    !trace.start_solid && !trace.all_solid && trace.fraction >= 1.0
}
fn angle(v: f32) -> u16 {
    (v / 360.0 * 65536.0).round() as i32 as u16
}
fn ground(p: V3, d: &Definition, ignore: Option<u16>, c: &Collision) -> Option<V3> {
    if p.iter().any(|v| !v.is_finite() || v.abs() > 32700.0) {
        return None;
    }
    let (min, max) = d.hull();
    let from = add(p, [0.0, 0.0, 32.0]);
    let to = add(p, [0.0, 0.0, -128.0]);
    let t = c.trace_actor_bounds(from, to, min, max, ignore);
    (!t.start_solid && !t.all_solid && t.fraction < 1.0 && t.normal[2] >= 0.7).then(|| {
        add(
            add(from, mul(sub(to, from), t.fraction)),
            mul(t.normal, SKIN),
        )
    })
}
fn walk(start: V3, end: V3, d: &Definition, ignore: Option<u16>, c: &Collision) -> Option<V3> {
    if start
        .iter()
        .chain(&end)
        .any(|v| !v.is_finite() || v.abs() > 32700.0)
    {
        return None;
    }
    let samples = (length(sub(end, start)) / 12.0).ceil().max(1.0) as usize;
    if samples > 256 {
        return None;
    }
    let mut position = start;
    for i in 1..=samples {
        let point = add(start, mul(sub(end, start), i as f32 / samples as f32));
        position = walk_step(position, point, d, ignore, c)?;
    }
    Some(position)
}
fn walk_step(start: V3, end: V3, d: &Definition, ignore: Option<u16>, c: &Collision) -> Option<V3> {
    let (min, max) = d.hull();
    let up = add(start, [0.0, 0.0, 18.0]);
    let across = add(end, [0.0, 0.0, 18.0]);
    for (a, b) in [(start, up), (up, across)] {
        let t = c.trace_actor_bounds(a, b, min, max, ignore);
        if t.start_solid || t.all_solid || t.fraction < 1.0 {
            return None;
        }
    }
    let bottom = add(end, [0.0, 0.0, -18.0]);
    let t = c.trace_actor_bounds(across, bottom, min, max, ignore);
    if t.start_solid || t.all_solid || t.fraction >= 1.0 || t.normal[2] < 0.7 {
        return None;
    }
    Some(add(
        add(across, mul(sub(bottom, across), t.fraction)),
        mul(t.normal, SKIN),
    ))
}

#[cfg(test)]
mod tests;
