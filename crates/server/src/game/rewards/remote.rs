//! Operator lifetime and predator payload. Native camera/input facts are target
//! captures, including registered speed defaults. Laptop timing and the
//! acceleration/boost, point collision and expiry explosion follow Alpha.
//! The runtime exposes the controller experimentally with matched map resources.
use super::{
    inventory::Ticket,
    killstreaks::RadarResources,
    strikes::{Hit, Source},
};
use crate::game::world::{
    collision::Collision,
    math::*,
    props::{Blast, Event},
};
use codol_protocol::game::{
    channel::usercmd::{BUTTON_REMOTE, UserCommand},
    snapshot::{
        hit_feedback::RemoteExitSnapshot,
        missile::MissileSnapshot,
        remote::{Camera, Links},
    },
};

pub(crate) use codol_protocol::game::snapshot::missile::{
    END_REMOTE_ENTITY as END_ENTITY, FIRST_REMOTE_ENTITY as FIRST_ENTITY,
};

const INTRO_MSEC: u32 = 1800; // reference laptop1.0s and fade0.8s
const NORMAL_SPEED: f32 = 3000.0;
const BOOST_SPEED: f32 = 6000.0;
const ACCELERATION: f32 = 2000.0;
const DECELERATION: f32 = 500.0;
// Alpha CM trace keeps point contacts this far outside the hit plane. This
// is a contact tolerance, not a swept missile radius.
const CONTACT_EPSILON: f32 = 0.125;

#[derive(Clone, Copy)]
pub(crate) struct Pending {
    pub ticket: Ticket,
    pub life: u32,
    pub weapon: u16,
    began: u32,
    pub angles: [u16; 3],
}
#[derive(Clone, Copy)]
pub(crate) struct Returned {
    pub life: u32,
    pub weapon: u16,
    pub angles: [u16; 3],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub entity: u16,
    pub generation: u32,
}
struct Operation {
    key: Key,
    team: u8,
    body: Returned,
    tag: u8,
    fof_at: u32,
    exit_overlay: Option<RemoteExitSnapshot>,
    exit_until: Option<u32>,
}
#[derive(Clone, Copy)]
struct Weapon {
    index: u16,
    initial_speed: f32,
    radius: f32,
    inner: u16,
    outer: u16,
    lifetime: u32,
}
impl Weapon {
    fn from_resources(r: &RadarResources) -> Option<Self> {
        // The script spawns345;339 is a separate projectile and must not alias.
        let w = r.weapons.get("remotemissile_projectile_mp")?;
        let projectile = w.projectile?;
        if !(1..900).contains(&w.weapon)
            || w.weapon_type != 2
            || projectile.explosion_kind != 1
            || !(1..=60000).contains(&projectile.lifetime_msec)
            || w.projectile_speed == 0
            || w.projectile_speed > 50000
            || w.radius == 0
            || w.radius > 8192
            || w.inner_damage == 0
            || w.inner_damage > u32::from(u16::MAX)
            || w.outer_damage > u32::from(u16::MAX)
            || !r.models.contains_key("projectile_hellfire_missile")
        {
            return None;
        }
        Some(Self {
            index: w.weapon,
            initial_speed: w.projectile_speed as f32,
            radius: w.radius as f32,
            inner: w.inner_damage as u16,
            outer: w.outer_damage as u16,
            lifetime: projectile.lifetime_msec,
        })
    }
}
pub(crate) struct Plan {
    origin: V3,
    angles: V3,
    weapon: Weapon,
    tag: u8,
    vision: Option<&'static str>,
    exit_overlay: Option<RemoteExitSnapshot>,
}
struct Missile {
    state: MissileSnapshot,
    angles: V3,
    speed: f32,
    weapon: Weapon,
    source: Source,
    last: u32,
    expires: u32,
    input_time: u32,
    buttons: u32,
    armed: bool,
    boosted: bool,
}
#[derive(Clone, Copy)]
pub(crate) struct VisionChange {
    key: Key,
    pub life: u32,
    name: &'static str,
    transition_msec: u16,
}
pub(crate) struct WeaponChange {
    key: Key,
    pub life: u32,
    pub weapon: u16,
    pub view_angles: Option<[u16; 3]>,
}
impl VisionChange {
    pub(crate) fn command(self) -> Vec<u8> {
        format!("Q \"{}\" {}", self.name, self.transition_msec).into_bytes()
    }
}
#[derive(Default)]
pub(crate) struct Remotes {
    pending: Option<Pending>,
    operation: Option<Operation>,
    generation: u32,
    missiles: Vec<Missile>,
    cars: Vec<super::remote_car::Car>,
    gunners: super::gunners::Gunners,
    weapon_changes: Vec<WeaponChange>,
    returned: Vec<Returned>,
    hits: Vec<Hit>,
    events: Vec<Event>,
    vision_changes: Vec<VisionChange>,
    exit_hud_used: bool,
    clock: u32,
}
impl Remotes {
    pub(crate) fn resources_present(r: &RadarResources) -> bool {
        Weapon::from_resources(r).is_some()
            && r.tags
                .get("tag_origin")
                .is_some_and(|tag| (1..64).contains(tag))
            && r.remote_spawns
                .as_ref()
                .is_some_and(|spawns| spawns.valid())
            && r.remote_vision
                .as_ref()
                .is_none_or(|vision| vision.flight_name().is_some())
            && r.remote_exit.is_none_or(|exit| exit.valid())
    }
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some() || self.operation.is_some()
    }
    pub(crate) fn pending(&self) -> Option<Pending> {
        self.pending
    }
    pub(crate) fn begin(
        &mut self,
        now: u32,
        ticket: Ticket,
        life: u32,
        weapon: u16,
        angles: [u16; 3],
    ) -> bool {
        if self.busy() || !matches!(ticket.native_id(), 5 | 12 | 13 | 18) {
            return false;
        }
        self.pending = Some(Pending {
            ticket,
            life,
            weapon,
            began: now,
            angles,
        });
        true
    }
    pub(crate) fn ready(&self, now: u32) -> bool {
        self.pending.is_some_and(|p| {
            now.saturating_sub(p.began)
                >= if p.ticket.native_id() == 18 {
                    900
                } else {
                    INTRO_MSEC
                }
        })
    }
    pub(crate) fn cancel_pending(&mut self) -> Option<Ticket> {
        self.pending.take().map(|p| p.ticket)
    }
    pub(crate) fn finish_operator(&mut self) {
        if let Some(active) = self.operation.take() {
            self.returned.push(active.body);
        }
    }
    pub(crate) fn operator_life(&self) -> Option<u32> {
        self.operation.as_ref().map(|s| s.body.life)
    }
    pub(crate) fn links(&self) -> Option<Links> {
        let active = self.operation.as_ref()?;
        if active.exit_until.is_some() {
            return Some(Links {
                camera: Some(Camera {
                    entity: active.key.entity,
                    tag: active.tag,
                    tag_angles: true,
                }),
                controls: None,
                fof_overlay: self.clock >= active.fof_at,
                thermal_vision: true,
                weapon_view: false,
            });
        }
        if self.cars.iter().any(|car| car.key == active.key) {
            return Some(Links {
                camera: Some(Camera {
                    entity: active.key.entity,
                    tag: active.tag,
                    tag_angles: true,
                }),
                controls: None,
                fof_overlay: false,
                thermal_vision: false,
                weapon_view: false,
            });
        }
        if let Some(entity) = self.gunners.camera(active.key) {
            return Some(Links {
                camera: Some(Camera {
                    entity,
                    tag: active.tag,
                    tag_angles: true,
                }),
                controls: None,
                fof_overlay: self.clock >= active.fof_at,
                thermal_vision: true,
                weapon_view: true,
            });
        }
        let missile = self.missiles.iter().find(|m| key(&m.state) == active.key)?;
        Some(Links {
            camera: Some(Camera {
                entity: active.key.entity,
                tag: active.tag,
                tag_angles: true,
            }),
            controls: Some(active.key.entity),
            fof_overlay: missile.last >= active.fof_at,
            thermal_vision: true,
            weapon_view: false,
        })
    }
    pub(crate) fn exit_overlay(&self) -> Option<RemoteExitSnapshot> {
        self.operation
            .as_ref()
            .and_then(|operation| operation.exit_until.and(operation.exit_overlay))
            .or_else(|| self.exit_hud_used.then_some(RemoteExitSnapshot::default()))
    }
    pub(crate) fn body_angles(&self) -> Option<[u16; 3]> {
        self.operation.as_ref().map(|o| o.body.angles)
    }
    pub(crate) fn drain_returns(&mut self) -> Vec<Returned> {
        std::mem::take(&mut self.returned)
    }
    pub(crate) fn plan_car(
        &self,
        r: &RadarResources,
        team: u8,
        origin: V3,
        angles: [u16; 3],
        (occupied, reserved_slots): (&[V3], u16),
        c: &Collision,
    ) -> Option<super::remote_car::Plan> {
        r.tags
            .get("tag_player")
            .filter(|&&tag| (1..64).contains(&tag))?;
        let entity = (super::remote_car::FIRST_ENTITY..super::remote_car::END_ENTITY)
            .find(|e| self.cars.iter().all(|car| car.key.entity != *e))?;
        let reserved_slots = self
            .cars
            .iter()
            .fold(reserved_slots, |mask, car| mask | (1 << car.slot));
        super::remote_car::Car::plan(
            r,
            entity,
            team,
            origin,
            angles,
            (occupied, reserved_slots),
            c,
        )
    }
    pub(crate) fn launch_car(
        &mut self,
        now: u32,
        plan: super::remote_car::Plan,
        source: Source,
        tag: u8,
        c: &Collision,
    ) {
        let p = self.pending.take().expect("committed RC preparation");
        assert_eq!(p.ticket.native_id(), 18);
        assert_eq!(source.life, p.life);
        self.generation = self
            .generation
            .checked_add(1)
            .expect("remote generation exhausted");
        let car = super::remote_car::Car::launch(plan, now, self.generation, source);
        self.operation = Some(Operation {
            key: car.key,
            team: source.team,
            tag,
            fof_at: u32::MAX,
            exit_overlay: None,
            exit_until: None,
            body: Returned {
                life: p.life,
                weapon: p.weapon,
                angles: p.angles,
            },
        });
        self.cars.push(car);
        self.sync_cars(c);
    }
    pub(crate) fn props(&self) -> Vec<codol_protocol::game::snapshot::props::PropSnapshot> {
        let mut p = self
            .cars
            .iter()
            .map(|car| car.snapshot())
            .collect::<Vec<_>>();
        p.extend(self.gunners.props());
        p
    }
    pub(crate) fn vehicle_slots(&self) -> u16 {
        self.cars
            .iter()
            .fold(self.gunners.slots(), |mask, car| mask | (1 << car.slot))
    }
    pub(crate) fn keyboard_driving(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|o| self.cars.iter().any(|car| car.key == o.key))
    }
    pub(crate) fn zero_body_prediction(&self) -> bool {
        self.keyboard_driving() || self.gunner_active()
    }
    fn sync_cars(&self, c: &Collision) {
        c.replace_prop_range(
            super::remote_car::FIRST_ENTITY..super::remote_car::END_ENTITY,
            &self
                .cars
                .iter()
                .map(|car| car.collider())
                .collect::<Vec<_>>(),
        );
    }
    pub(crate) fn damage_car(&mut self, entity: u16, damage: u16, team: u8) -> bool {
        if let Some(car) = self.cars.iter_mut().find(|car| car.key.entity == entity) {
            car.damage(damage, team);
            return true;
        }
        false
    }
    pub(crate) fn radial_cars(&mut self, blast: Blast, team: u8, c: &Collision) {
        for car in &mut self.cars {
            car.radial(blast, team, c);
        }
        self.gunners.radial(blast, team, c);
    }
    pub(crate) fn plan_gunner(
        &self,
        r: &RadarResources,
        id: u8,
        reserved: u16,
        c: &Collision,
    ) -> Option<super::gunners::Plan> {
        self.gunners.plan(r, id, reserved | self.vehicle_slots(), c)
    }
    pub(crate) fn launch_gunner(
        &mut self,
        now: u32,
        plan: super::gunners::Plan,
        source: Source,
        tag: u8,
        c: &Collision,
    ) {
        let p = self.pending.take().expect("committed gunner preparation");
        assert!(matches!(p.ticket.native_id(), 12 | 13));
        assert_eq!(p.ticket.native_id(), plan.id);
        assert_eq!(source.life, p.life);
        self.generation = self
            .generation
            .checked_add(1)
            .expect("remote generation exhausted");
        let (key, vision) = self.gunners.launch(plan, now, self.generation, source, c);
        self.operation = Some(Operation {
            key,
            team: source.team,
            body: Returned {
                life: p.life,
                weapon: p.weapon,
                angles: p.angles,
            },
            tag,
            fof_at: now.saturating_add(150),
            exit_overlay: None,
            exit_until: None,
        });
        self.clock = self.clock.max(now);
        if let Some(name) = vision {
            self.vision_changes.extend([
                VisionChange {
                    key,
                    life: p.life,
                    name: "black_bw",
                    transition_msec: 0,
                },
                VisionChange {
                    key,
                    life: p.life,
                    name,
                    transition_msec: 1000,
                },
            ]);
        }
        if let Some(weapon) = self.gunners.selected_weapon() {
            self.weapon_changes.push(WeaponChange {
                key,
                life: p.life,
                weapon,
                view_angles: self.gunners.angles(),
            });
        }
    }
    pub(crate) fn gunner_equipment(
        &self,
        e: codol_protocol::game::snapshot::Equipment,
    ) -> Option<codol_protocol::game::snapshot::Equipment> {
        if self.gunner_active() {
            self.gunners.equipment(e)
        } else {
            None
        }
    }
    pub(crate) fn gunner_active(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|o| self.gunners.key() == Some(o.key))
    }
    pub(crate) fn gunner_started_at(&self) -> Option<u32> {
        self.gunner_active()
            .then(|| self.operation.as_ref().unwrap().fof_at.saturating_sub(150))
    }
    pub(crate) fn gunner_angles(&self) -> Option<[u16; 3]> {
        self.gunner_active()
            .then(|| self.gunners.angles())
            .flatten()
    }
    pub(crate) fn damage_gunner(&mut self, entity: u16, damage: u16, team: u8) -> bool {
        self.gunners.damage(entity, damage, team)
    }
    pub(crate) fn drain_gunner_shots(&mut self) -> Vec<super::deployables::DeviceShot> {
        self.gunners.drain_shots()
    }
    pub(crate) fn gunner_targets(&self) -> Vec<super::airspace::Target> {
        self.gunners.targets()
    }
    pub(crate) fn drain_weapon_changes(&mut self) -> Vec<WeaponChange> {
        let key = self.operation.as_ref().map(|o| o.key);
        std::mem::take(&mut self.weapon_changes)
            .into_iter()
            .filter(|change| Some(change.key) == key)
            .collect()
    }
    pub(crate) fn plan(
        &self,
        r: &RadarResources,
        origin: V3,
        angles: [u16; 3],
        enemies: &[V3],
        c: &Collision,
    ) -> Option<Plan> {
        if self.missiles.len() >= usize::from(END_ENTITY - FIRST_ENTITY) {
            return None;
        }
        let weapon = Weapon::from_resources(r)?;
        let vision = match &r.remote_vision {
            Some(resources) => Some(resources.flight_name()?),
            None => None,
        };
        let exit_overlay = match r.remote_exit {
            Some(resources) if resources.valid() => Some(resources.snapshot()),
            Some(_) => return None,
            None => None,
        };
        let tag = *r
            .tags
            .get("tag_origin")
            .filter(|&&tag| (1..64).contains(&tag))?;
        let direction = axes(angles.map(|a| f32::from(a as i16) * 360.0 / 65536.0))[0];
        let (vertical, horizontal, target) =
            if matches!(r.map.as_str(), "mp_suburbia" | "mp_mainstreet") {
                (7000.0, 10000.0, 2000.0)
            } else {
                (14000.0, 7000.0, 1500.0)
            };
        let spawns = r.remote_spawns.as_ref()?;
        let (start, aim) = if spawns.points.is_empty() {
            let start = add(
                sub(origin, mul(direction, horizontal)),
                [0.0, 0.0, vertical],
            );
            (start, sub(add(origin, mul(direction, target)), start))
        } else {
            let mut scores = vec![0; spawns.points.len()];
            for enemy in enemies {
                let nearest = spawns
                    .points
                    .iter()
                    .enumerate()
                    .min_by(|(ai, a), (bi, b)| {
                        (a.target[0] - enemy[0])
                            .hypot(a.target[1] - enemy[1])
                            .total_cmp(&(b.target[0] - enemy[0]).hypot(b.target[1] - enemy[1]))
                            .then_with(|| bi.cmp(ai))
                    })
                    .unwrap()
                    .0;
                scores[nearest] += 2;
                for (i, p) in spawns.points.iter().enumerate() {
                    scores[i] += 1;
                    let trace = c.raycast(add(*enemy, [0.0, 0.0, 32.0]), p.origin);
                    if !trace.start_solid && !trace.all_solid && trace.fraction >= 1.0 {
                        scores[i] += 3;
                    }
                }
            }
            let best = scores
                .iter()
                .enumerate()
                .max_by_key(|(i, score)| (**score, std::cmp::Reverse(*i)))
                .unwrap()
                .0;
            let spawn = spawns.points[best];
            let start = add(
                spawn.target,
                mul(normalized(sub(spawn.origin, spawn.target)), 14000.0),
            );
            (start, sub(spawn.target, start))
        };
        if start.iter().any(|v| !v.is_finite() || v.abs() > 32760.0)
            || c.raycast(start, start).start_solid
        {
            return None;
        }
        Some(Plan {
            origin: start,
            angles: direction_angles(aim),
            weapon,
            tag,
            vision,
            exit_overlay,
        })
    }
    /// The caller commits the exact pending ticket immediately before launch.
    pub(crate) fn launch(&mut self, now: u32, plan: Plan, source: Source) {
        let p = self.pending.take().expect("committed remote preparation");
        self.clock = self.clock.max(now);
        assert_eq!(source.life, p.life);
        let entity = (FIRST_ENTITY..END_ENTITY)
            .find(|e| self.missiles.iter().all(|m| m.state.entity_number != *e))
            .expect("planned remote capacity");
        self.generation = self
            .generation
            .checked_add(1)
            .expect("remote generation exhausted");
        let state = MissileSnapshot {
            events: Default::default(),
            entity_number: entity,
            origin: plan.origin,
            weapon: plan.weapon.index,
            present: true,
            generation: self.generation,
            owner: source.owner,
            angles: short_angles(plan.angles),
        };
        if let Some(flight) = plan.vision {
            self.vision_changes.extend([
                VisionChange {
                    key: key(&state),
                    life: p.life,
                    name: "black_bw",
                    transition_msec: 0,
                },
                VisionChange {
                    key: key(&state),
                    life: p.life,
                    name: flight,
                    transition_msec: 1000,
                },
            ]);
        }
        self.operation = Some(Operation {
            key: key(&state),
            team: source.team,
            tag: plan.tag,
            fof_at: now.saturating_add(150),
            exit_overlay: plan.exit_overlay,
            exit_until: None,
            body: Returned {
                life: p.life,
                weapon: p.weapon,
                angles: p.angles,
            },
        });
        self.missiles.push(Missile {
            state,
            angles: plan.angles,
            speed: plan.weapon.initial_speed,
            weapon: plan.weapon,
            source,
            last: now,
            expires: now.saturating_add(plan.weapon.lifetime),
            input_time: now,
            buttons: 0,
            armed: false,
            boosted: false,
        });
    }
    pub(crate) fn input(&mut self, input: &UserCommand, c: &Collision) -> bool {
        let Some(active) = self.operation.as_ref() else {
            return false;
        };
        let id = active.key;
        self.advance(input.time, false, 0, c);
        if self.gunners.key() == Some(id) {
            let before = self.gunners.selected_weapon();
            let vision = self.gunners.vision();
            self.gunners.input(input);
            if self.gunners.vision() != vision
                && let Some((name, transition_msec)) = self.gunners.vision()
                && let Some(o) = self.operation.as_ref()
            {
                self.vision_changes.push(VisionChange {
                    key: id,
                    life: o.body.life,
                    name,
                    transition_msec,
                });
            }
            if (self.gunners.selected_weapon() != before
                || self.gunners.selected_weapon() != Some(input.weapons[0]))
                && let Some(weapon) = self.gunners.selected_weapon()
                && let Some(o) = self.operation.as_ref()
            {
                self.weapon_changes.push(WeaponChange {
                    key: id,
                    life: o.body.life,
                    weapon,
                    view_angles: None,
                });
            }
            return true;
        }
        if let Some(car) = self.cars.iter_mut().find(|car| car.key == id) {
            car.input(input);
            return true;
        }
        let Some(m) = self.missiles.iter_mut().find(|m| key(&m.state) == id) else {
            return true;
        };
        if input.time <= m.input_time {
            return true;
        }
        let elapsed = (input.time - m.input_time).min(200);
        m.input_time = input.time;
        m.buttons = input.buttons & (BUTTON_REMOTE | 1);
        if let Some(axes_input) = input.remote_axes() {
            let old_forward = axes(m.angles)[0];
            m.angles = steer(m.angles, axes_input, elapsed);
            m.speed *= dot(old_forward, axes(m.angles)[0]);
            m.state.angles = short_angles(m.angles);
        }
        true
    }
    pub(crate) fn advance(&mut self, now: u32, finished: bool, emp_teams: u8, c: &Collision) {
        self.clock = self.clock.max(now);
        if let Some(ended) = self.gunners.advance(
            now,
            finished,
            emp_teams,
            self.operation.as_ref().map(|o| o.key),
            c,
        ) && self.operation.as_ref().is_some_and(|o| o.key == ended)
        {
            self.finish_operator();
        }
        let mut cars = Vec::new();
        for mut car in std::mem::take(&mut self.cars) {
            let cancelled = finished
                || emp_teams & (1 << car.source.team) != 0
                || !self.operation.as_ref().is_some_and(|o| o.key == car.key);
            let exploded = !cancelled && car.advance(now, c);
            if cancelled || exploded {
                if exploded && car.in_bounds() {
                    let (hit, event) = car.explosion();
                    self.hits.push(hit);
                    self.events.push(event);
                }
                if self.operation.as_ref().is_some_and(|o| o.key == car.key) {
                    self.finish_operator();
                }
            } else {
                cars.push(car);
            }
        }
        self.cars = cars;
        self.sync_cars(c);
        if self.operation.as_ref().is_some_and(|o| {
            o.exit_until
                .is_some_and(|until| now >= until || finished || emp_teams & (1 << o.team) != 0)
        }) {
            self.finish_operator();
        }
        let mut keep = Vec::new();
        for mut m in self.missiles.drain(..) {
            let mut remove = finished || emp_teams & (1 << m.source.team) != 0;
            let mut impact = None;
            let flight_until = now.min(m.expires);
            let mut ended_at = flight_until;
            while !remove && m.last < flight_until {
                let dt = (flight_until - m.last).min(20) as f32 * 0.001;
                m.last += (dt * 1000.0).round() as u32;
                let buttons = if m.last.saturating_sub(m.input_time) <= 200
                    && m.buttons & BUTTON_REMOTE != 0
                {
                    m.buttons
                } else {
                    0
                };
                if m.armed && !m.boosted && buttons & 1 != 0 {
                    m.boosted = true;
                    m.speed = BOOST_SPEED;
                    m.state
                        .events
                        .push(
                            codol_protocol::game::snapshot::events::MISSILE_REMOTE_BOOST,
                            0,
                        )
                        .expect("verified native boost event");
                } else {
                    if buttons & BUTTON_REMOTE != 0 && buttons & 1 == 0 {
                        m.armed = true;
                    }
                    m.speed = if m.speed < NORMAL_SPEED {
                        (m.speed + ACCELERATION * dt).min(NORMAL_SPEED)
                    } else {
                        (m.speed - DECELERATION * dt).max(NORMAL_SPEED)
                    };
                }
                let end = add(m.state.origin, mul(axes(m.angles)[0], m.speed * dt));
                // Alpha G_RunMissile uses G_LocationalTrace(start, end),
                // without a projectile hull. The visible model is not a hull.
                let trace = c.raycast(m.state.origin, end);
                if trace.fraction < 1.0 || trace.start_solid || trace.all_solid {
                    let approach = -dot(sub(end, m.state.origin), trace.normal);
                    let fraction = if approach > 0.0 {
                        (trace.fraction - CONTACT_EPSILON / approach).max(0.0)
                    } else {
                        trace.fraction
                    };
                    let origin = std::array::from_fn(|i| {
                        m.state.origin[i] + (end[i] - m.state.origin[i]) * fraction
                    });
                    impact = Some((
                        origin,
                        trace.normal,
                        ((trace.surface_flags >> 20) & 31) as u8,
                    ));
                    remove = true;
                    ended_at = m.last;
                } else {
                    m.state.origin = end;
                }
                if m.state.origin.iter().any(|v| v.abs() > 32760.0) {
                    remove = true;
                }
            }
            // Alpha G_FireRocket schedules handler9 at projLifetime; its
            // G_RunThink calls G_ExplodeMissile. Integrate only to that clock,
            // explode once there, and preserve earlier collision/cancellation.
            if !remove && now >= m.expires {
                impact = Some((m.state.origin, [0.0, 0.0, 1.0], 0));
                remove = true;
            }
            if let Some((origin, normal, surface)) = impact {
                self.hits.push(Hit {
                    blast: Blast {
                        origin,
                        radius: m.weapon.radius,
                        inner: m.weapon.inner,
                        outer: m.weapon.outer,
                        owner: m.source.owner,
                        weapon: m.weapon.index,
                        source_entity: Some(m.state.entity_number),
                        reward_life: Some(m.source.life),
                    },
                    team: m.source.team,
                    life: m.source.life,
                    airstrike: false,
                    area: None,
                });
                self.events.push(Event::Rocket {
                    weapon: m.weapon.index,
                    origin,
                    normal,
                    surface,
                });
            }
            if remove {
                if self
                    .operation
                    .as_ref()
                    .is_some_and(|o| o.key == key(&m.state))
                {
                    let active = self.operation.as_mut().unwrap();
                    if !finished
                        && emp_teams & (1 << m.source.team) == 0
                        && active.exit_overlay.is_some()
                    {
                        active.exit_until = Some(ended_at.saturating_add(500));
                        self.exit_hud_used = true;
                    } else {
                        let active = self.operation.take().unwrap();
                        self.returned.push(active.body);
                    }
                }
            } else {
                keep.push(m);
            }
        }
        self.missiles = keep;
        if self
            .operation
            .as_ref()
            .is_some_and(|o| o.exit_until.is_some_and(|until| now >= until))
        {
            self.finish_operator();
        }
    }
    pub(crate) fn missiles(&self) -> Vec<MissileSnapshot> {
        let mut states = self.missiles.iter().map(|m| m.state).collect::<Vec<_>>();
        states.extend(self.gunners.shells());
        states.sort_by_key(|m| m.entity_number);
        states
    }
    pub(crate) fn drain_vision(&mut self) -> Vec<VisionChange> {
        let current = self.operation.as_ref().map(|operation| operation.key);
        std::mem::take(&mut self.vision_changes)
            .into_iter()
            .filter(|change| current == Some(change.key))
            .collect()
    }
    pub(crate) fn drain_hits(&mut self) -> Vec<Hit> {
        let mut hits = std::mem::take(&mut self.hits);
        hits.extend(self.gunners.drain_hits());
        hits
    }
    pub(crate) fn drain(&mut self) -> Vec<Event> {
        let mut events = std::mem::take(&mut self.events);
        events.extend(self.gunners.drain());
        events
    }
}
fn key(s: &MissileSnapshot) -> Key {
    Key {
        entity: s.entity_number,
        generation: s.generation,
    }
}
fn short_angles(a: V3) -> [u16; 3] {
    a.map(|v| (v * 65536.0 / 360.0).round() as i32 as u16)
}
fn direction_angles(d: V3) -> V3 {
    [
        -d[2].atan2(d[0].hypot(d[1])).to_degrees(),
        d[1].atan2(d[0]).to_degrees(),
        0.0,
    ]
}
fn axes(a: V3) -> [V3; 3] {
    let [p, y, r] = a.map(f32::to_radians);
    let (sp, cp) = p.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sr, cr) = r.sin_cos();
    [
        [cp * cy, cp * sy, -sp],
        [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, sr * cp],
        [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp],
    ]
}
fn steer(a: V3, input: [f32; 2], msec: u32) -> V3 {
    let delta = axes([
        input[0] * 35.0 * msec as f32 * 0.001,
        input[1] * 35.0 * msec as f32 * 0.001,
        0.0,
    ]);
    let current = axes(a);
    let composed: [[f32; 3]; 3] = std::array::from_fn(|row| {
        std::array::from_fn(|col| (0..3).map(|i| delta[i][row] * current[i][col]).sum())
    });
    let f = composed[0];
    let p = -f[2].atan2(f[0].hypot(f[1])).to_degrees();
    let y = f[1].atan2(f[0]).to_degrees();
    [
        p.clamp(20.0, 87.0),
        y,
        composed[1][2].atan2(composed[2][2]).to_degrees(),
    ]
}

#[cfg(test)]
pub(crate) fn test_resources() -> RadarResources {
    let mut r = super::deployables::test_resources();
    let mut weapon = r.weapons["sentry_minigun_mp"].clone();
    weapon.weapon = 345;
    weapon.weapon_type = 2;
    weapon.projectile_speed = 10;
    weapon.projectile = Some(super::killstreaks::ProjectileFacts {
        lifetime_msec: 12000,
        explosion_kind: 1,
    });
    weapon.radius = 450;
    weapon.inner_damage = 1000;
    weapon.outer_damage = 10;
    r.weapons
        .insert("remotemissile_projectile_mp".into(), weapon);
    r.models.insert("projectile_hellfire_missile".into(), 111);
    r.tags.insert("tag_origin".into(), 18);
    r.tags.insert("tag_player".into(), 19);
    r.remote_spawns = Some(super::killstreaks::RemoteSpawns {
        sources_sha256: ["0".repeat(64)],
        points: vec![],
    });
    r.effects.insert("explosions/aerial_explosion".into(), 5);
    r
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::rewards::inventory::Inventory;
    #[test]
    fn predator_availability_checks_full_payload_spawns_and_optional_native_overlays() {
        let mut resources = test_resources();
        assert!(Remotes::resources_present(&resources));
        resources.remote_spawns = None;
        assert!(!Remotes::resources_present(&resources));
        resources = test_resources();
        resources.tags.insert("tag_origin".into(), 0);
        assert!(!Remotes::resources_present(&resources));
        resources = test_resources();
        resources.remote_exit = Some(super::super::killstreaks::RemoteExitResources {
            white: 1,
            grain: 1001,
            material_bank_count: 2,
        });
        assert!(!Remotes::resources_present(&resources));
    }
    fn collision() -> Collision {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[20000,20000,10],"contents":1,"planes":[]}],"triangles":[]})).unwrap()).unwrap();
        Collision::load(f.path()).unwrap()
    }
    #[test]
    fn active_payload_numbers_never_overlap_full_smoke_or_scavenger_pools() {
        use codol_protocol::game::snapshot::{scavenger, smoke};
        let reserved = (smoke::FIRST_ENTITY..smoke::END_ENTITY)
            .chain(scavenger::FIRST_ENTITY..scavenger::END_ENTITY)
            .collect::<std::collections::BTreeSet<_>>();
        for number in (FIRST_ENTITY..END_ENTITY)
            .chain(super::super::drones::FIRST_ENTITY..super::super::drones::END_ENTITY)
        {
            assert!(
                !reserved.contains(&number),
                "payload reuses live world entity {number}"
            );
        }
    }
    fn launch_test_car(r: &mut Remotes, c: &Collision) {
        let resources = super::super::remote_car::test_resources();
        let mut inventory = Inventory::default();
        assert!(inventory.grant(0, 18, 3));
        let ticket = inventory.reserve(0, 18, 3).unwrap();
        assert!(r.begin(1000, ticket, 3, 338, [0; 3]));
        assert!(!r.ready(1899));
        assert!(r.ready(1900));
        let plan = r
            .plan_car(&resources, 2, [0.0; 3], [0; 3], (&[], 0), c)
            .unwrap();
        assert!(inventory.commit(ticket, 0, 3, true));
        r.launch_car(
            1900,
            plan,
            Source {
                owner: 0,
                team: 2,
                life: 3,
            },
            18,
            c,
        );
        assert!(!inventory.contains(18));
        assert_eq!(r.links().unwrap().controls, None);
        assert!(r.keyboard_driving());
        assert!(c.raycast([0.0, 0.0, 8.0], [100.0, 0.0, 8.0]).prop_entity == Some(608));
    }
    #[test]
    fn rc_detonation_is_once_and_clears_camera_body_and_collision() {
        let c = collision();
        let mut r = Remotes::default();
        launch_test_car(&mut r, &c);
        assert!(r.input(
            &UserCommand {
                time: 1920,
                ..Default::default()
            },
            &c
        ));
        assert!(r.input(
            &UserCommand {
                time: 1940,
                buttons: 1,
                ..Default::default()
            },
            &c
        ));
        r.advance(1960, false, 0, &c);
        let hits = r.drain_hits();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].blast.weapon, 338);
        assert_eq!(hits[0].blast.reward_life, Some(3));
        assert_eq!(r.drain().len(), 1);
        assert_eq!(r.drain_returns().len(), 1);
        assert!(r.links().is_none() && r.props().is_empty());
        assert_eq!(c.raycast([0.0, 0.0, 8.0], [100.0, 0.0, 8.0]).fraction, 1.0);
        r.advance(32000, false, 0, &c);
        assert!(r.drain_hits().is_empty() && r.drain().is_empty() && r.drain_returns().is_empty());
    }
    #[test]
    fn rc_death_emp_end_and_old_instance_never_damage_or_unlink_a_new_life() {
        for cause in 0..3 {
            let c = collision();
            let mut r = Remotes::default();
            launch_test_car(&mut r, &c);
            if cause == 0 {
                r.finish_operator();
            }
            r.advance(1920, cause == 1, if cause == 2 { 1 << 2 } else { 0 }, &c);
            assert!(r.drain_hits().is_empty() && r.drain().is_empty());
            assert!(r.links().is_none() && r.props().is_empty());
            assert_eq!(r.drain_returns().len(), 1);
            launch(&mut r, 4, 2000, [0.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
            r.advance(2020, false, 0, &c);
            assert_eq!(r.operator_life(), Some(4));
            assert!(r.drain_returns().is_empty());
        }
    }
    fn launch(remote: &mut Remotes, life: u32, now: u32, origin: V3, angles: V3) {
        let mut inventory = Inventory::default();
        assert!(inventory.grant(0, 5, life));
        let ticket = inventory.reserve(0, 5, life).unwrap();
        assert!(remote.begin(now, ticket, life, 325, [0, 1234, 0]));
        let plan = Plan {
            origin,
            angles,
            weapon: Weapon::from_resources(&test_resources()).unwrap(),
            tag: 18,
            vision: None,
            exit_overlay: None,
        };
        assert!(inventory.commit(ticket, 0, life, true));
        remote.launch(
            now,
            plan,
            Source {
                owner: 0,
                team: 2,
                life,
            },
        );
    }
    #[test]
    #[ignore = "requires matching captured remote resources and map collision"]
    fn captured_remote_spawn_routes_hit_real_map_and_keep_original_source() {
        let resources_path =
            std::path::PathBuf::from(std::env::var_os("CODOL_REMOTE_RESOURCES").unwrap());
        let collision_path =
            std::path::PathBuf::from(std::env::var_os("CODOL_REMOTE_COLLISION").unwrap());
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&resources_path).unwrap()).unwrap();
        let resources = RadarResources::load(
            &resources_path,
            raw["map"].as_str().unwrap(),
            raw["ncs_crc"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let c = Collision::load(&collision_path).unwrap();
        let points = &resources.remote_spawns.as_ref().unwrap().points;
        assert!(!points.is_empty());
        for (i, point) in points.iter().enumerate() {
            let mut remote = Remotes::default();
            let plan = remote
                .plan(&resources, [0.0; 3], [0; 3], &[point.target], &c)
                .unwrap();
            eprintln!(
                "captured remote route {i}: origin={:?} angles={:?}",
                plan.origin, plan.angles
            );
            let mut inventory = Inventory::default();
            inventory.grant(0, 5, 3);
            let ticket = inventory.reserve(0, 5, 3).unwrap();
            remote.begin(1000, ticket, 3, 325, [0; 3]);
            assert!(inventory.commit(ticket, 0, 3, true));
            remote.launch(
                2800,
                plan,
                Source {
                    owner: 0,
                    team: 2,
                    life: 3,
                },
            );
            let mut hits = Vec::new();
            let mut ended = None;
            for now in (2850..32800).step_by(50) {
                remote.advance(now, false, 0, &c);
                hits.extend(remote.drain_hits());
                if remote.missiles().is_empty() {
                    ended = Some(now);
                    break;
                }
            }
            assert_eq!(
                hits.len(),
                1,
                "captured route {i} must hit actual geometry before timeout"
            );
            assert_eq!(hits[0].life, 3);
            assert_eq!(hits[0].blast.weapon, 345);
            assert_eq!(hits[0].blast.owner, 0);
            assert!(
                ended.unwrap() < 2800 + 12000,
                "route {i} reached expiry instead of map geometry"
            );
            assert_eq!(remote.drain_returns().len(), 1);
            assert!(remote.links().is_none());
        }
    }
    #[test]
    fn authored_spawn_uses_enemy_target_score_and_missing_capture_cannot_invent_fallback() {
        let c = collision();
        let mut resources = test_resources();
        let remote = Remotes::default();
        resources.remote_spawns = None;
        assert!(remote.plan(&resources, [0.0; 3], [0; 3], &[], &c).is_none());
        resources.remote_spawns = Some(super::super::killstreaks::RemoteSpawns {
            sources_sha256: ["0".repeat(64)],
            points: vec![
                super::super::killstreaks::RemoteSpawn {
                    origin: [-1000.0, 0.0, 2000.0],
                    target: [-1000.0, 0.0, 0.0],
                },
                super::super::killstreaks::RemoteSpawn {
                    origin: [1000.0, 0.0, 2000.0],
                    target: [1000.0, 0.0, 0.0],
                },
            ],
        });
        let plan = remote
            .plan(&resources, [0.0; 3], [0; 3], &[[1000.0, 0.0, 0.0]], &c)
            .unwrap();
        assert_eq!(plan.origin, [1000.0, 0.0, 14000.0]);
        assert_eq!(plan.angles[0], 90.0);
        let plan = remote
            .plan(&resources, [0.0; 3], [0; 3], &[[-1000.0, 0.0, 0.0]], &c)
            .unwrap();
        assert_eq!(plan.origin, [-1000.0, 0.0, 14000.0]);
    }
    #[test]
    fn preparation_is_cancelable_and_payload_bindings_do_not_alias339() {
        let c = collision();
        let mut r = test_resources();
        let mut inventory = Inventory::default();
        inventory.grant(0, 5, 3);
        let ticket = inventory.reserve(0, 5, 3).unwrap();
        let mut remote = Remotes::default();
        assert!(remote.begin(1000, ticket, 3, 325, [0; 3]));
        assert!(!remote.ready(2799));
        assert!(remote.ready(2800));
        assert_eq!(remote.cancel_pending(), Some(ticket));
        assert!(inventory.cancel(ticket));
        assert!(inventory.contains(5));
        assert!(remote.links().is_none());
        let rocket = r.weapons.remove("remotemissile_projectile_mp").unwrap();
        r.weapons.insert("remote_missile".into(), rocket.clone());
        assert!(remote.plan(&r, [0.0; 3], [0; 3], &[], &c).is_none());
        r.weapons
            .insert("remotemissile_projectile_mp".into(), rocket);
        let plan = remote.plan(&r, [0.0; 3], [0; 3], &[], &c).unwrap();
        assert_eq!(plan.weapon.index, 345);
        assert_eq!(plan.origin, [-7000.0, 0.0, 14000.0]);
        r.models.remove("projectile_hellfire_missile");
        assert!(remote.plan(&r, [0.0; 3], [0; 3], &[], &c).is_none());
        for id in [20, 23] {
            inventory.grant(0, id, 3);
            let t = inventory.reserve(0, id, 3).unwrap();
            assert!(!remote.begin(3000, t, 3, 325, [0; 3]));
            inventory.cancel(t);
        }
    }
    #[test]
    fn captured_vision_uses_ordered_missilecam_commands_and_rejects_unbound_names() {
        use super::super::killstreaks::RemoteVisionResources;
        let c = collision();
        let mut resources = test_resources();
        let mut r = Remotes::default();
        assert!(
            r.plan(&resources, [0.0; 3], [0; 3], &[], &c)
                .unwrap()
                .vision
                .is_none()
        );
        let mut vision = RemoteVisionResources {
            arena_absent: false,
            flight: "thermal_snowlevel_mp".into(),
            rawfiles_sha256: [
                "mp/basemaps.arena",
                "vision/black_bw.vision",
                "vision/thermal_snowlevel_mp.vision",
            ]
            .into_iter()
            .map(|name| (name.into(), "a".repeat(64)))
            .collect(),
        };
        assert_eq!(vision.flight_name(), Some("thermal_snowlevel_mp"));
        let absent = RemoteVisionResources {
            flight: "thermal_mp".into(),
            arena_absent: true,
            rawfiles_sha256: ["vision/black_bw.vision", "vision/thermal_mp.vision"]
                .into_iter()
                .map(|name| (name.into(), "b".repeat(64)))
                .collect(),
        };
        assert_eq!(absent.flight_name(), Some("thermal_mp"));
        let mut invalid_absence = absent;
        invalid_absence.flight = "thermal_snowlevel_mp".into();
        assert!(invalid_absence.flight_name().is_none());

        vision.rawfiles_sha256.remove("mp/basemaps.arena");
        assert!(vision.flight_name().is_none());
        vision
            .rawfiles_sha256
            .insert("mp/basemaps.arena".into(), "a".repeat(64));
        vision.flight = "thermal_mp\"; other-command".into();
        resources.remote_vision = Some(vision.clone());
        assert!(r.plan(&resources, [0.0; 3], [0; 3], &[], &c).is_none());
        vision.flight = "thermal_snowlevel_mp".into();
        resources.remote_vision = Some(vision);
        let plan = r.plan(&resources, [0.0; 3], [0; 3], &[], &c).unwrap();
        let mut inventory = Inventory::default();
        inventory.grant(0, 5, 3);
        let ticket = inventory.reserve(0, 5, 3).unwrap();
        r.begin(1000, ticket, 3, 325, [0; 3]);
        assert!(r.drain_vision().is_empty());
        assert!(inventory.commit(ticket, 0, 3, true));
        r.launch(
            2800,
            plan,
            Source {
                owner: 0,
                team: 2,
                life: 3,
            },
        );
        let changes = r.drain_vision();
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|change| change.life == 3));
        assert_eq!(changes[0].command(), b"Q \"black_bw\" 0");
        assert_eq!(changes[1].command(), b"Q \"thermal_snowlevel_mp\" 1000");
        // A queued command must not outlive its operator instance.
        r.finish_operator();
        r.vision_changes.extend(changes);

        assert!(r.drain_vision().is_empty());
    }
    fn launch_exit(r: &mut Remotes) {
        launch(r, 3, 1000, [0.0, 0.0, 10.0], [87.0, 0.0, 0.0]);
        r.operation.as_mut().unwrap().exit_overlay = Some(RemoteExitSnapshot {
            visible: true,
            white: 1000,
            grain: 1001,
        });
    }
    #[test]
    fn static_exit_unlinks_controls_keeps_body_frozen_and_returns_after_500ms() {
        let c = collision();
        let mut r = Remotes::default();
        launch_exit(&mut r);
        let body = r.body_angles();
        r.advance(1200, false, 0, &c);
        assert_eq!(r.drain_hits().len(), 1);
        assert!(r.missiles().is_empty());
        assert!(r.busy());
        assert_eq!(r.operator_life(), Some(3));
        assert_eq!(r.body_angles(), body);
        assert_eq!(r.links().unwrap().controls, None);
        assert_eq!(r.links().unwrap().camera.unwrap().entity, FIRST_ENTITY);
        assert!(r.exit_overlay().is_some_and(|exit| exit.visible));
        assert!(r.drain_returns().is_empty());
        let until = r.operation.as_ref().unwrap().exit_until.unwrap();
        assert!(until > 1200);
        assert!(r.input(
            &UserCommand {
                time: 1250,
                buttons: BUTTON_REMOTE | 1,
                ..Default::default()
            },
            &c
        ));
        r.advance(until - 1, false, 0, &c);
        assert!(r.exit_overlay().is_some_and(|exit| exit.visible));
        r.advance(until, false, 0, &c);
        assert!(!r.busy());
        assert!(r.links().is_none());
        assert!(r.exit_overlay().is_some_and(|exit| !exit.visible));
        assert_eq!(r.drain_returns()[0].life, 3);
        r.advance(until + 100, false, 0, &c);
        assert!(r.drain_returns().is_empty());
        assert!(r.drain_hits().is_empty());
    }
    #[test]
    fn lifetime_exit_uses_expiry_clock_and_early_impact_retains_delayed_fof() {
        let c = collision();
        let mut r = Remotes::default();
        launch_exit(&mut r);
        r.advance(1100, false, 0, &c);
        assert!(r.exit_overlay().is_some_and(|exit| exit.visible));
        assert!(!r.links().unwrap().fof_overlay);
        r.advance(1150, false, 0, &c);
        assert!(r.links().unwrap().fof_overlay);
        let mut r = Remotes::default();
        launch_exit(&mut r);
        r.missiles[0].state.origin = [0.0, 0.0, 10000.0];
        r.missiles[0].expires = 1100;
        r.advance(1100, false, 0, &c);
        assert_eq!(r.drain_hits().len(), 1);
        assert_eq!(r.drain().len(), 1);
        assert_eq!(r.operation.as_ref().unwrap().exit_until, Some(1600));
        r.advance(1600, false, 0, &c);
        assert!(!r.busy());
        assert!(r.exit_overlay().is_some_and(|exit| !exit.visible));
    }
    #[test]
    fn static_exit_cancels_on_death_emp_and_end_and_never_replays_after_delay() {
        let c = collision();
        for cause in 0..3 {
            let mut r = Remotes::default();
            launch_exit(&mut r);
            r.advance(1200, false, 0, &c);
            if cause == 0 {
                r.finish_operator();
            }
            r.advance(1201, cause == 1, if cause == 2 { 1 << 2 } else { 0 }, &c);
            assert!(r.exit_overlay().is_some_and(|exit| !exit.visible));
            assert!(r.links().is_none());
            assert_eq!(r.drain_returns().len(), 1);
        }
        let mut r = Remotes::default();
        launch_exit(&mut r);
        r.advance(2000, false, 0, &c);
        assert_eq!(r.drain_hits().len(), 1);
        assert!(r.exit_overlay().is_some_and(|exit| !exit.visible));
        assert!(!r.busy());
        assert_eq!(r.drain_returns().len(), 1);
    }
    #[test]
    fn exit_resources_require_reserved_dynamic_slots_and_do_not_invent_materials() {
        use super::super::killstreaks::RemoteExitResources;
        let c = collision();
        let r = Remotes::default();
        let mut resources = test_resources();
        assert!(
            r.plan(&resources, [0.0; 3], [0; 3], &[], &c)
                .unwrap()
                .exit_overlay
                .is_none()
        );
        resources.remote_exit = Some(RemoteExitResources {
            white: 1000,
            grain: 1001,
            material_bank_count: 27,
        });
        assert_eq!(
            r.plan(&resources, [0.0; 3], [0; 3], &[], &c)
                .unwrap()
                .exit_overlay,
            Some(RemoteExitSnapshot {
                visible: true,
                white: 1000,
                grain: 1001
            })
        );
        resources.remote_exit.as_mut().unwrap().material_bank_count = 1000;
        assert!(r.plan(&resources, [0.0; 3], [0; 3], &[], &c).is_none());
    }
    #[test]
    fn fof_overlay_waits_150ms_and_clears_with_the_original_operator() {
        let c = collision();
        let mut r = Remotes::default();
        launch(&mut r, 3, 1000, [0.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
        assert!(!r.links().unwrap().fof_overlay);
        r.advance(1149, false, 0, &c);
        assert!(!r.links().unwrap().fof_overlay);
        r.advance(1150, false, 0, &c);
        assert!(r.links().unwrap().fof_overlay);
        r.finish_operator();
        assert!(r.links().is_none());
        // The old missile remains, but a new life starts its own delay.
        launch(&mut r, 4, 1200, [1000.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
        assert!(!r.links().unwrap().fof_overlay);
        r.advance(1350, false, 0, &c);
        assert!(r.links().unwrap().fof_overlay);
        r.advance(1360, false, 1 << 2, &c);
        assert!(r.links().is_none());
        assert!(r.missiles().is_empty());
    }
    #[test]
    fn remote_input_moves_only_payload_and_boost_requires_release_then_press() {
        let c = collision();
        let mut r = Remotes::default();
        launch(&mut r, 3, 1000, [0.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
        let body = r.body_angles();
        let command = |time, buttons, remote| UserCommand {
            time,
            buttons,
            remote,
            ..Default::default()
        };
        r.input(&command(1020, BUTTON_REMOTE | 1, [0, 0]), &c);
        r.advance(1040, false, 0, &c);
        assert!(!r.missiles[0].boosted);
        assert!(r.missiles[0].speed < 200.0);
        r.input(&command(1060, BUTTON_REMOTE, [0, 0]), &c);
        r.advance(1080, false, 0, &c);
        r.input(&command(1100, BUTTON_REMOTE | 1, [129, 0]), &c);
        r.advance(1120, false, 0, &c);
        assert!(r.missiles[0].boosted);
        assert!(r.missiles[0].speed > 5900.0);
        let angle = r.missiles[0].angles;
        assert!(angle[0] < 45.0);
        r.input(&command(1100, BUTTON_REMOTE, [127, 127]), &c);
        assert_eq!(r.missiles[0].angles, angle);
        r.input(&command(1140, 0, [127, 127]), &c);
        assert_eq!(r.missiles[0].angles, angle);
        assert_eq!(r.body_angles(), body);
        assert!(r.missiles()[0].origin[2] < 10000.0);
        assert_eq!(r.links().unwrap().controls, Some(FIRST_ENTITY));
    }
    #[test]
    fn detached_old_life_payload_hits_once_without_unlinking_new_operation() {
        let c = collision();
        let mut r = Remotes::default();
        launch(&mut r, 3, 1000, [0.0, 0.0, 10.0], [87.0, 0.0, 0.0]);
        r.finish_operator();
        assert_eq!(r.drain_returns()[0].life, 3);
        assert!(r.links().is_none());
        launch(&mut r, 4, 1000, [1000.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
        let current = r.links().unwrap();
        assert_eq!(current.controls, Some(FIRST_ENTITY + 1));
        r.advance(1200, false, 0, &c);
        let hits = r.drain_hits();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].life, 3);
        assert_eq!(hits[0].blast.reward_life, Some(3));
        assert_eq!(hits[0].blast.weapon, 345);
        assert!((hits[0].blast.origin[2] - CONTACT_EPSILON).abs() < 0.0001);
        assert!(super::super::strikes::damage(hits[0], hits[0].blast.origin, 1.0, &c) > 0);
        assert_eq!(
            r.links(),
            Some(Links {
                fof_overlay: true,
                ..current
            })
        );
        assert!(r.drain_returns().is_empty());
        assert_eq!(r.drain().len(), 1);
        r.advance(1220, false, 0, &c);
        assert!(r.drain_hits().is_empty());
        assert!(r.drain().is_empty());
    }
    #[test]
    fn delayed_tick_keeps_pre_expiry_collision_and_input_window() {
        let c = collision();
        let mut stepped = Remotes::default();
        let mut delayed = Remotes::default();
        for r in [&mut stepped, &mut delayed] {
            launch(r, 3, 1000, [0.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
            // A delayed tick still consumes fire during its valid input window.
            r.input(
                &UserCommand {
                    time: 1020,
                    buttons: BUTTON_REMOTE,
                    ..Default::default()
                },
                &c,
            );
            r.input(
                &UserCommand {
                    time: 1100,
                    buttons: BUTTON_REMOTE | 1,
                    ..Default::default()
                },
                &c,
            );
        }
        for now in (1120..=2000).step_by(20) {
            stepped.advance(now, false, 0, &c);
        }
        delayed.advance(2000, false, 0, &c);
        assert!(stepped.missiles[0].boosted);
        assert!(delayed.missiles[0].boosted);
        assert_eq!(stepped.missiles()[0].origin, delayed.missiles()[0].origin);
        for now in (2020..=14000).step_by(20) {
            stepped.advance(now, false, 0, &c);
        }
        delayed.advance(14000, false, 0, &c);
        let a = stepped.drain_hits();
        let b = delayed.drain_hits();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].blast.origin, b[0].blast.origin);
        assert_eq!(b[0].blast.reward_life, Some(3));
        assert!(delayed.missiles().is_empty());
        assert!(delayed.links().is_none());
        assert_eq!(delayed.drain_returns().len(), 1);
        assert_eq!(delayed.drain().len(), 1);
        delayed.advance(15000, false, 0, &c);
        assert!(delayed.drain_hits().is_empty());
        assert!(delayed.drain_returns().is_empty());
        assert!(delayed.drain().is_empty());
    }
    #[test]
    fn expiry_never_integrates_collision_beyond_lifetime() {
        let c = collision();
        let mut r = Remotes::default();
        launch(&mut r, 3, 1000, [0.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
        r.missiles[0].expires = 1100;
        let mut timely = Remotes::default();
        launch(&mut timely, 3, 1000, [0.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
        timely.missiles[0].expires = 1100;
        timely.advance(1100, false, 0, &c);
        r.advance(14000, false, 0, &c);
        let hits = r.drain_hits();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].blast.origin, timely.drain_hits()[0].blast.origin);
        assert!(hits[0].blast.origin[2] > 9900.0);
        assert_eq!(hits[0].blast.weapon, 345);
        assert_eq!(hits[0].blast.reward_life, Some(3));
        assert_eq!(r.drain().len(), 1);
        assert!(r.missiles().is_empty());
        assert_eq!(r.drain_returns().len(), 1);
        r.advance(15000, false, 0, &c);
        assert!(r.drain_hits().is_empty());
        assert!(r.drain().is_empty());
        assert!(r.drain_returns().is_empty());
    }
    #[test]
    fn point_flight_clears_nearby_surfaces_and_expires_once_in_air() {
        let c = collision();
        let mut r = Remotes::default();
        launch(&mut r, 3, 1000, [0.0, 0.0, 1.0], [0.0; 3]);
        r.missiles[0].expires = 2000;
        assert!(
            c.projectile_trace([0.0, 0.0, 1.0], [10.0, 0.0, 1.0], 2.0)
                .start_solid
        );
        r.advance(1100, false, 0, &c);
        assert_eq!(r.missiles().len(), 1);
        assert!(r.drain_hits().is_empty());
        r.advance(13000, false, 0, &c);
        let hits = r.drain_hits();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].blast.origin[2], 1.0);
        assert!(matches!(
            r.drain().as_slice(),
            [Event::Rocket {
                weapon: 345,
                normal: [0.0, 0.0, 1.0],
                surface: 0,
                ..
            }]
        ));
        r.advance(13100, false, 0, &c);
        assert!(r.drain_hits().is_empty());
        assert!(r.drain().is_empty());
    }
    #[test]
    fn emp_match_end_and_timeout_release_each_operator_exactly_once() {
        let c = collision();
        for (now, finished, emp) in [(1100, false, 1 << 2), (1100, true, 0), (13000, false, 0)] {
            let mut r = Remotes::default();
            launch(&mut r, 3, 1000, [0.0, 0.0, 10000.0], [45.0, 0.0, 0.0]);
            r.advance(now, finished, emp, &c);
            assert!(r.links().is_none());
            assert!(r.missiles().is_empty());
            assert_eq!(r.drain_returns().len(), 1);
            assert_eq!(r.drain_hits().len(), usize::from(!finished && emp == 0));
            r.drain();
            r.advance(now + 100, finished, emp, &c);
            assert!(r.drain_returns().is_empty());
            assert!(r.drain_hits().is_empty());
        }
        assert!((steer([86.0, 0.0, 0.0], [-1.0, 0.0], 200)[0] - 87.0).abs() < 0.001);
    }
}
