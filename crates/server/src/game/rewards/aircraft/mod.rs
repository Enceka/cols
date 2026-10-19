//! Attack flight authority; reward availability stays gated until attack and
//! native presentation verification are complete.
use super::killstreaks::{ModelBounds, RadarResources};
use crate::game::world::{
    collision::Collision,
    props::{Collider, Event},
};
use codol_protocol::game::snapshot::props::PropSnapshot;
pub(super) mod routes;
#[cfg(test)]
mod tests;
mod weapons;
pub(crate) use weapons::{AircraftShot, TargetActor};
#[derive(Clone, Copy, Debug)]
pub(crate) struct Source {
    pub owner: u8,
    pub team: u8,
    pub life: u32,
    pub serial: u32,
}
use routes::{Motion, Movement, Path};
pub(crate) const FIRST_ENTITY: u16 = 560;
pub(crate) const END_ENTITY: u16 = 576;
pub(crate) struct Plan {
    entity: u16,
    model: u16,
    gun_model: Option<u16>,
    loop_sound: u8,
    bounds: ModelBounds,
    ingress: Path,
    patrol: Path,
    graph: super::paths::FlightNodes,
    fx: u16,
    id: u8,
    gun: weapons::Gun,
    vehicle: u8,
    rotor: Option<super::killstreaks::AnimationBinding>,
    smoke: [u16; 2],
    flare: Option<u16>,
    rocket: Option<super::sam::Weapon>,
    delay: u32,
}
struct Flight {
    plan: Plan,
    motion: Motion,
    last: u32,
    patrol_end: Option<u32>,
    leaving: bool,
    source: Source,
    health: u16,
    gun_state: weapons::GunState,
    attacker: Option<u8>,
    damage_stage: u8,
    next_smoke: u32,
    flare_available: bool,
    crash: Option<(u32, [f32; 3])>,
    crash_pending: bool,
    crash_route: bool,
    fire_events: codol_protocol::game::snapshot::events::EventQueue,
    next_rocket: u32,
    activated_at: u32,
    ingress_done: bool,
    rockets_remaining: u8,
    gun_angles: [f32; 2],
}
#[derive(Default)]
pub(crate) struct Aircraft {
    reserved_slots: u16,
    flights: Vec<Flight>,
    events: Vec<Event>,
    clock: u32,
}
impl Aircraft {
    pub(crate) fn resources_present(r: &RadarResources, id: u8) -> bool {
        let model = match id {
            8 => "vehicle_mi24p_hind_mp",
            10 => "vehicle_pavelow",
            _ => return false,
        };
        r.model_bounds.get(model).is_some_and(|b| b.valid())
            && r.models.contains_key(model)
            && r.vehicles
                .contains_key(if id == 10 { "pavelow_mp" } else { "cobra_mp" })
            && r.animations.get("bh_rotors").is_some_and(|a| a.parent > 0)
            && r.air_paths.as_ref().is_some_and(|g| {
                g.valid()
                    && g.entries.contains_key("heli_start")
                    && g.entries.contains_key("heli_loop_start")
                    && g.entries.contains_key("heli_leave")
            })
            && weapons::Gun::from_resources(r, id).is_some()
            && r.effects.contains_key("explosions/aerial_explosion")
            && r.effects
                .contains_key("smoke/smoke_trail_white_heli_emitter")
            && r.effects
                .contains_key("smoke/smoke_trail_black_heli_emitter")
            && (id != 10
                || r.models.contains_key("weapon_minigun")
                    && r.effects.contains_key("misc/flares_cobra"))
    }
    pub(crate) fn harrier_resources_present(r: &RadarResources) -> bool {
        r.model_bounds
            .get("vehicle_av8b_harrier_jet_mp")
            .is_some_and(|bounds| bounds.valid())
            && r.models.contains_key("vehicle_av8b_harrier_jet_mp")
            && r.vehicles.contains_key("harrier_mp")
            && r.air_route.is_some()
            && r.air_paths
                .as_ref()
                .is_some_and(|paths| paths.valid() && paths.entries.contains_key("heli_leave"))
            && weapons::Gun::from_resources(r, 7).is_some()
            && [
                "explosions/aerial_explosion_harrier",
                "smoke/smoke_trail_white_heli_emitter",
                "smoke/smoke_trail_black_heli_emitter",
            ]
            .into_iter()
            .all(|name| r.effects.contains_key(name))
            && super::sam::Weapon::aircraft(r, 7).is_some()
    }
    pub(crate) fn vehicle_slots(&self) -> u16 {
        self.flights.iter().fold(0, |mask, flight| {
            mask | (1 << (flight.plan.entity - FIRST_ENTITY))
        })
    }
    pub(crate) fn reserve_slots(&mut self, mask: u16) {
        self.reserved_slots = mask;
    }
    pub(crate) fn plan(&self, r: &RadarResources, id: u8, c: &Collision) -> Option<Plan> {
        // Alpha shares one chopper slot between the normal and flare variants.
        // Keep the new inventory instance until the previous aircraft leaves
        // or finishes its crash, instead of launching intersecting routes.
        if self.flights.iter().any(|f| matches!(f.plan.id, 8 | 10)) {
            return None;
        }
        let model = match id {
            8 => "vehicle_mi24p_hind_mp",
            10 => "vehicle_pavelow",
            _ => return None,
        };
        let gun_model = if id == 10 {
            Some(*r.models.get("weapon_minigun")?)
        } else {
            None
        };
        let bounds = *r.model_bounds.get(model)?;
        let gun = weapons::Gun::from_resources(r, id)?;
        let vehicle = *r
            .vehicles
            .get(if id == 10 { "pavelow_mp" } else { "cobra_mp" })?;
        let rotor = Some(*r.animations.get("bh_rotors").filter(|a| a.parent > 0)?);
        let smoke = [
            *r.effects.get("smoke/smoke_trail_white_heli_emitter")?,
            *r.effects.get("smoke/smoke_trail_black_heli_emitter")?,
        ];
        let flare = (id == 10)
            .then(|| r.effects.get("misc/flares_cobra").copied())
            .flatten();
        if id == 10 && flare.is_none() {
            return None;
        }
        let graph = r.air_paths.as_ref()?;
        let entity = (FIRST_ENTITY..END_ENTITY).find(|e| {
            self.reserved_slots & (1 << (*e - FIRST_ENTITY)) == 0
                && !self.flights.iter().any(|f| f.plan.entity == *e)
        })?;
        for &start in graph.entries.get("heli_start")? {
            let mut ingress = routes::authored(graph, start, false)?;
            if !r.experimental_air_routes && !routes::fit(c, bounds, &mut ingress, None, None) {
                continue;
            }
            let end = ingress.nodes.last()?.origin;
            for &loop_start in graph.entries.get("heli_loop_start")? {
                let mut patrol = routes::authored(graph, loop_start, id == 10)?;
                if !routes::fit(c, bounds, &mut patrol, Some(end), None)
                    && !r.experimental_air_routes
                {
                    continue;
                }
                if !r.experimental_air_routes
                    && routes::exit_path(patrol.nodes[0].origin, graph, bounds, c, None).is_none()
                {
                    continue;
                }
                return Some(Plan {
                    entity,
                    model: *r.models.get(model)?,
                    gun_model,
                    loop_sound: r
                        .loop_sounds
                        .get("weap_cobra_20mm_fire_npc")
                        .copied()
                        .filter(|_| id == 8)
                        .unwrap_or(0),
                    bounds,
                    ingress,
                    patrol,
                    graph: graph.clone(),
                    fx: *r.effects.get("explosions/aerial_explosion")?,
                    id,
                    gun,
                    vehicle,
                    rotor,
                    smoke,
                    flare,
                    rocket: super::sam::Weapon::aircraft(r, id),
                    delay: 0,
                });
            }
        }
        None
    }
    pub(crate) fn launch(&mut self, now: u32, plan: Plan, source: Source, c: &Collision) {
        self.clock = self.clock.max(now);
        let motion = Motion::new(plan.ingress.clone());
        let activated_at = now.saturating_add(plan.delay);
        self.flights.push(Flight {
            health: if matches!(plan.id, 7 | 10) {
                3000
            } else {
                1500
            },
            flare_available: plan.flare.is_some(),
            plan,
            motion,
            last: activated_at,
            patrol_end: None,
            leaving: false,
            source,
            gun_state: Default::default(),
            attacker: None,
            damage_stage: 0,
            next_smoke: now,
            crash: None,
            crash_pending: false,
            crash_route: false,
            fire_events: Default::default(),
            next_rocket: now,
            activated_at,
            ingress_done: false,
            rockets_remaining: 6,
            gun_angles: [0.0; 2],
        });
        self.sync(c);
    }
    pub(crate) fn advance(&mut self, now: u32, finished: bool, c: &Collision) {
        self.clock = now;
        if finished {
            self.flights.clear();
            self.events.clear();
            self.sync(c);
            return;
        }
        let mut keep = Vec::new();
        for mut f in self.flights.drain(..) {
            let mut remove = false;
            while f.last < now {
                let step = (now - f.last).min(20);
                f.last += step;
                if let Some((start, mut velocity)) = f.crash {
                    let dt = step as f32 * 0.001;
                    if f.crash_pending {
                        f.crash_pending = false;
                        if let Some(path) = routes::crash_path(
                            f.motion.origin,
                            &f.plan.graph,
                            f.plan.bounds,
                            c,
                            f.plan.entity,
                        ) {
                            f.motion.replace(path);
                            f.crash_route = true;
                        }
                    }
                    if f.crash_route {
                        let yaw = f.motion.angles[1];
                        let movement = f.motion.step(f.last, dt, f.plan.bounds, f.plan.entity, c);
                        f.motion.angles[1] = yaw.wrapping_add(short(180.0 * dt));
                        f.motion.angles[2] = short(25.0);
                        if matches!(movement, Movement::Moving)
                            && f.last.saturating_sub(start) < 60000
                        {
                            continue;
                        }
                        self.events.push(Event::Fx {
                            selector: f.plan.fx,
                            origin: f.motion.origin,
                        });
                        remove = true;
                        break;
                    }
                    velocity[2] -= 400.0 * dt;
                    let next = std::array::from_fn(|i| f.motion.origin[i] + velocity[i] * dt);
                    let (min, max) = routes::envelope(f.plan.bounds);
                    let trace =
                        c.trace_bounds(f.motion.origin, next, min, max, Some(f.plan.entity));
                    f.motion.angles[1] = f.motion.angles[1].wrapping_add(short(180.0 * dt));
                    f.motion.angles[2] = short(25.0);
                    if trace.start_solid
                        || trace.fraction < 1.0
                        || f.last.saturating_sub(start) > 12000
                    {
                        self.events.push(Event::Fx {
                            selector: f.plan.fx,
                            origin: f.motion.origin,
                        });
                        remove = true;
                        break;
                    }
                    f.motion.origin = next;
                    f.crash = Some((start, velocity));
                    continue;
                }
                if !f.leaving && f.patrol_end.is_some_and(|end| f.last >= end) {
                    if let Some(path) = routes::exit_path(
                        f.motion.origin,
                        &f.plan.graph,
                        f.plan.bounds,
                        c,
                        Some(f.plan.entity),
                    ) {
                        f.motion.replace(path);
                        f.leaving = true;
                    } else {
                        remove = true;
                        break;
                    }
                }
                match f
                    .motion
                    .step(f.last, step as f32 * 0.001, f.plan.bounds, f.plan.entity, c)
                {
                    Movement::Moving => {}
                    Movement::Done if !f.ingress_done => {
                        f.ingress_done = true;
                        f.patrol_end = Some(if f.plan.id == 7 {
                            f.activated_at.saturating_add(45000)
                        } else {
                            f.last.saturating_add(60000)
                        });
                        f.motion.replace(f.plan.patrol.clone());
                    }
                    Movement::Done => {
                        remove = true;
                        break;
                    }
                    Movement::Blocked => {
                        self.events.push(Event::Fx {
                            selector: f.plan.fx,
                            origin: f.motion.origin,
                        });
                        remove = true;
                        break;
                    }
                }
            }
            if !remove {
                keep.push(f);
            }
        }
        self.flights = keep;
        self.sync(c);
    }
    fn sync(&self, c: &Collision) {
        let shapes = self
            .flights
            .iter()
            .filter(|f| self.clock >= f.activated_at)
            .map(|f| {
                Collider::new(
                    f.plan.entity,
                    f.motion.origin,
                    f.motion.angles,
                    f.plan.bounds.min,
                    f.plan.bounds.max,
                )
            })
            .collect::<Vec<_>>();
        c.replace_prop_range(FIRST_ENTITY..END_ENTITY, &shapes);
    }
    pub(crate) fn snapshots(&self) -> Vec<PropSnapshot> {
        let mut snapshots = self
            .flights
            .iter()
            .filter(|f| self.clock >= f.activated_at)
            .flat_map(|f| {
                let aircraft = PropSnapshot {
                    actor: None,
                    entity_number: f.plan.entity,
                    model: f.plan.model,
                    origin: f.motion.origin,
                    angles: f.motion.angles,
                    present: true,
                    weapon: f.plan.gun.weapon,
                    events: f.fire_events,
                    turret: false,
                    turret_pose: None,
                    vehicle: Some(codol_protocol::game::snapshot::props::VehiclePose {
                        definition: f.plan.vehicle,
                        slot: (f.plan.entity - FIRST_ENTITY) as u8,
                        tree: f.plan.rotor.map_or(0, |r| r.tree),
                        gun: f.gun_angles,
                        animation: f.plan.rotor.map_or(0, |r| r.node),
                        animation_parent: f.plan.rotor.map_or(0, |r| r.parent),
                        loop_sound: if f.gun_state.firing(
                            0,
                            self.clock,
                            f.crash.is_none() && !f.leaving,
                            f.plan.gun.interval,
                        ) {
                            f.plan.loop_sound
                        } else {
                            0
                        },
                        animation_time: f.activated_at,
                    }),
                };
                let mut states = vec![aircraft];
                if let Some(model) = f.plan.gun_model {
                    for barrel in 0..2 {
                        let (origin, angles) =
                            weapons::heavy_attachment(f.motion.origin, f.motion.angles, barrel);
                        states.push(PropSnapshot {
                            actor: None,
                            entity_number: END_ENTITY
                                + (f.plan.entity - FIRST_ENTITY) * 2
                                + barrel as u16,
                            model,
                            origin,
                            angles,
                            present: true,
                            weapon: f.plan.gun.weapon,
                            events: f.gun_state.events(barrel),
                            turret: false,
                            turret_pose: Some(f.gun_state.presentation(
                                barrel,
                                self.clock,
                                angles,
                                f.crash.is_none() && !f.leaving,
                                f.plan.gun.interval,
                            )),
                            vehicle: None,
                        });
                    }
                }
                states
            })
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|p| p.entity_number);
        snapshots
    }
    pub(crate) fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
    pub(crate) fn shots(
        &mut self,
        now: u32,
        actors: &[TargetActor],
        emp_teams: u8,
        c: &Collision,
    ) -> Vec<AircraftShot> {
        let mut shots = Vec::new();
        for f in &mut self.flights {
            if now < f.activated_at
                || f.crash.is_some()
                || f.leaving
                || emp_teams & (1 << f.source.team) != 0
            {
                continue;
            }
            if f.damage_stage > 0 && now >= f.next_smoke {
                f.next_smoke = now.saturating_add(1000);
                self.events.push(Event::Fx {
                    selector: f.plan.smoke[usize::from(f.damage_stage > 1)],
                    origin: f.motion.origin,
                });
            }
            let fired = f.gun_state.fire(
                now,
                &weapons::FireContext {
                    source: f.source,
                    entity: f.plan.entity,
                    origin: f.motion.origin,
                    angles: f.motion.angles,
                    gun: f.plan.gun,
                    attacker: f.attacker,
                },
                actors,
                c,
            );
            if !fired.is_empty() {
                let angles = fired[0].angles;
                f.gun_angles = [
                    f32::from(angles[0] as i16) * (360.0 / 65536.0),
                    f32::from(angles[1].wrapping_sub(f.motion.angles[1]) as i16)
                        * (360.0 / 65536.0),
                ];
                if f.plan.gun_model.is_none() {
                    f.fire_events
                        .push(codol_protocol::game::snapshot::events::FIRE_WEAPON, 0)
                        .expect("verified aircraft gun event");
                }
            }
            shots.extend(fired);
        }
        shots
    }
    pub(crate) fn damage(
        &mut self,
        now: u32,
        entity: u16,
        damage: u16,
        attacker: (u8, u8),
        explosive: bool,
        c: &Collision,
    ) -> bool {
        let Some(f) = self
            .flights
            .iter_mut()
            .find(|f| f.plan.entity == entity && f.source.team != attacker.1 && f.health > 0)
        else {
            return false;
        };
        let damage = if explosive {
            damage
        } else {
            (u32::from(damage) * 3).div_ceil(10) as u16
        };
        if damage == 0 {
            return false;
        }
        f.health = f.health.saturating_sub(damage);
        f.attacker = Some(attacker.0);
        let max = if matches!(f.plan.id, 7 | 10) {
            3000.0
        } else {
            1500.0
        };
        f.damage_stage = if f32::from(f.health) < max * 0.33 {
            2
        } else if f32::from(f.health) < max * 0.67 {
            1
        } else {
            0
        };
        tracing::info!(
            now,
            entity,
            damage,
            health = f.health,
            "attack aircraft damaged"
        );
        if f.health == 0 {
            start_crash(f, now);
        }
        self.sync(c);
        true
    }
    pub(crate) fn radial(
        &mut self,
        now: u32,
        blast: crate::game::world::props::Blast,
        team: u8,
        c: &Collision,
    ) {
        let hits = self
            .flights
            .iter()
            .filter(|f| f.health > 0)
            .map(|f| {
                (
                    f.plan.entity,
                    crate::game::world::props::radius_damage(
                        blast,
                        f.motion.origin,
                        0.0,
                        Some(f.plan.entity),
                        c,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (entity, damage) in hits {
            self.damage(now, entity, damage, (blast.owner, team), true, c);
        }
    }
    pub(crate) fn emp(&mut self, now: u32, team: u8) {
        for f in &mut self.flights {
            if f.source.team != team && f.health > 0 {
                f.health = 0;
                start_crash(f, now);
            }
        }
    }
    pub(crate) fn targets(&self) -> Vec<super::airspace::Target> {
        self.flights
            .iter()
            .filter(|f| f.health > 0 && !f.leaving && self.clock >= f.activated_at)
            .map(|f| super::airspace::Target {
                entity: f.plan.entity,
                source: super::killstreaks::SupportFlight {
                    actor: f.source.owner,
                    id: f.plan.id,
                    team: f.source.team,
                    start: f.source.serial,
                    end: u32::MAX,
                },
                center: f.motion.origin,
            })
            .collect()
    }
    /// The first guided interception spends the heavy helicopter's single
    /// flare deployment. The target identity includes the call serial, so a
    /// reused entity cannot steal a missile or receive a late callback.
    pub(crate) fn decoy(&mut self, source: super::airspace::Target) -> bool {
        let Some(f) = self.flights.iter_mut().find(|f| {
            f.plan.entity == source.entity
                && f.source.serial == source.source.start
                && f.health > 0
                && f.flare_available
        }) else {
            return false;
        };
        f.flare_available = false;
        self.events.push(Event::Fx {
            selector: f.plan.flare.unwrap(),
            origin: f.motion.origin,
        });
        true
    }
    pub(crate) fn launches(
        &mut self,
        now: u32,
        targets: &[super::airspace::Target],
        emp_teams: u8,
        capacity: usize,
        c: &Collision,
    ) -> Vec<super::sam::Launch> {
        let mut out = Vec::new();
        for f in &mut self.flights {
            let Some(weapon) = f.plan.rocket else {
                continue;
            };
            if f.health == 0
                || now < f.activated_at
                || (f.plan.id == 7 && f.rockets_remaining == 0)
                || f.leaving
                || now < f.next_rocket
                || emp_teams & (1 << f.source.team) != 0
                || out.len() >= capacity
            {
                continue;
            }
            let yaw = f32::from(f.motion.angles[1]) * std::f32::consts::TAU / 65536.0;
            let target = targets
                .iter()
                .filter(|t| t.source.team != f.source.team)
                .filter(|t| {
                    let d: [f32; 3] = std::array::from_fn(|i| t.center[i] - f.motion.origin[i]);
                    let range = distance(t.center, f.motion.origin);
                    let facing = (d[0] * yaw.cos() + d[1] * yaw.sin()) / range.max(1.0);
                    let trace =
                        c.raycast_excluding_prop(f.motion.origin, t.center, Some(f.plan.entity));
                    range <= 3500.0
                        && facing >= 0.3
                        && !trace.start_solid
                        && (trace.fraction >= 1.0 || trace.prop_entity == Some(t.entity))
                })
                .min_by(|a, b| {
                    distance(a.center, f.motion.origin)
                        .total_cmp(&distance(b.center, f.motion.origin))
                });
            if let Some(&target) = target {
                if f.plan.id == 7 {
                    f.rockets_remaining -= 1;
                }
                f.next_rocket = now.saturating_add(weapon.interval);
                out.push(super::sam::Launch {
                    reward_life: Some(f.source.life),
                    owner: f.source.owner,
                    team: f.source.team,
                    device: f.plan.entity,
                    origin: f.motion.origin,
                    target,
                    weapon,
                });
            }
        }
        out
    }
    pub(crate) fn plan_harrier(
        &self,
        r: &RadarResources,
        point: super::selection::Point,
        c: &Collision,
    ) -> Option<Plan> {
        let bounds = *r.model_bounds.get("vehicle_av8b_harrier_jet_mp")?;
        let graph = r.air_paths.as_ref()?;
        let height = r.air_route.as_ref()?.center[2];
        let entity = (FIRST_ENTITY..END_ENTITY).find(|e| {
            self.reserved_slots & (1 << (*e - FIRST_ENTITY)) == 0
                && !self.flights.iter().any(|f| f.plan.entity == *e)
        })?;
        let (sin, cos) = point.yaw.to_radians().sin_cos();
        let goal = [point.origin[0], point.origin[1], height];
        let node = routes::Waypoint {
            origin: goal,
            speed: 250.0 * 17.6,
            accel: 175.0 * 17.6,
            delay: 0,
            yaw: Some(point.yaw),
        };
        let mut ingress = Path {
            nodes: vec![
                routes::Waypoint {
                    origin: [goal[0] - cos * 5000.0, goal[1] - sin * 5000.0, height],
                    ..node.clone()
                },
                node.clone(),
            ],
            repeat: None,
        };
        if !r.experimental_air_routes && !routes::fit(c, bounds, &mut ingress, None, None) {
            return None;
        }
        let center = ingress.nodes[1].origin;
        let hover = routes::Waypoint {
            origin: center,
            speed: 15.0 * 17.6,
            accel: 5.0 * 17.6,
            delay: 3000,
            ..node
        };
        let mut patrol = Path {
            nodes: vec![
                hover.clone(),
                routes::Waypoint {
                    origin: [center[0] + 200.0, center[1], center[2]],
                    ..hover.clone()
                },
                routes::Waypoint {
                    origin: [center[0], center[1] + 200.0, center[2]],
                    ..hover
                },
            ],
            repeat: Some(0),
        };
        if !routes::fit(c, bounds, &mut patrol, Some(center), None)
            || routes::exit_path(center, graph, bounds, c, None).is_none()
        {
            return None;
        }
        Some(Plan {
            entity,
            model: *r.models.get("vehicle_av8b_harrier_jet_mp")?,
            gun_model: None,
            loop_sound: 0,
            bounds,
            ingress,
            patrol,
            graph: graph.clone(),
            fx: *r.effects.get("explosions/aerial_explosion_harrier")?,
            id: 7,
            gun: weapons::Gun::from_resources(r, 7)?,
            vehicle: *r.vehicles.get("harrier_mp")?,
            rotor: None,
            smoke: [
                *r.effects.get("smoke/smoke_trail_white_heli_emitter")?,
                *r.effects.get("smoke/smoke_trail_black_heli_emitter")?,
            ],
            flare: None,
            rocket: super::sam::Weapon::aircraft(r, 7),
            delay: 4000,
        })
    }
}
fn start_crash(f: &mut Flight, now: u32) {
    f.crash_pending = true;
    let yaw = f32::from(f.motion.angles[1]) * std::f32::consts::TAU / 65536.0;
    f.crash = Some((
        now,
        [
            yaw.cos() * f.motion.speed,
            yaw.sin() * f.motion.speed,
            -80.0,
        ],
    ));
}
fn short(a: f32) -> u16 {
    (a * 65536.0 / 360.0).round() as i32 as u16
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}

#[cfg(test)]
mod captured_tests {
    use super::*;
    #[test]
    #[ignore = "requires matching local AFP collision and aircraft graph"]
    fn captured_paths_clear_whole_aircraft_and_leave_without_looping_forever() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let state: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("research-output/tdm-afghan-hud-state.json")).unwrap(),
        )
        .unwrap();
        let c = Collision::load(std::path::Path::new(
            state["collision_file"].as_str().unwrap(),
        ))
        .unwrap();
        let r = RadarResources::load(
            &root.join("research-output/all-killstreaks-20261014/resources-afghan-mechanisms.json"),
            "mp_afghan",
            523897474,
        )
        .unwrap();
        for id in [8, 10] {
            let mut a = Aircraft::default();
            let plan = a
                .plan(&r, id, &c)
                .expect("at least one complete clear route");
            println!(
                "id={id} ingress={} patrol={} repeat={:?}",
                plan.ingress.nodes.len(),
                plan.patrol.nodes.len(),
                plan.patrol.repeat
            );
            let start = plan.ingress.nodes[0].origin;
            a.launch(
                1000,
                plan,
                Source {
                    owner: 0,
                    team: 2,
                    life: 0,
                    serial: 1000,
                },
                &c,
            );
            let mut moved = false;
            for now in (1050..240000).step_by(50) {
                a.advance(now, false, &c);
                for s in a.snapshots() {
                    moved |= distance(s.origin, start) > 100.0;
                    assert!(s.origin.iter().all(|v| v.is_finite()));
                }
                assert!(a.drain().is_empty(), "validated flight must not collide");
            }
            assert!(moved);
            assert!(a.snapshots().is_empty(), "patrol must terminate and depart");
        }
    }
    #[test]
    fn invalid_or_unbound_reward_cannot_create_an_aircraft_plan() {
        let r = crate::game::rewards::deployables::test_resources();
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[1000,1000,10],"contents":1,"planes":[]}],"triangles":[]})).unwrap()).unwrap();
        let c = Collision::load(f.path()).unwrap();
        let a = Aircraft::default();
        for id in [1, 7, 8, 10, 12] {
            assert!(a.plan(&r, id, &c).is_none());
        }
    }
}
