//! Operator-controlled helicopter and AC130. Native registrations, camera
//! mounts, ammunition, cadence and shell physics come from the target capture.
//! Authored helicopter motion and the 40s ride/70s rig revolution follow Alpha.
use super::{
    aircraft::routes::{self, Motion, Movement, Path},
    deployables::DeviceShot,
    killstreaks::{CameraMount, ModelBounds, RadarResources, WeaponFacts},
    remote::Key,
    strikes::{Hit, Source},
};
use crate::game::{
    combat::{Ballistics, DamageFalloff},
    world::{
        collision::Collision,
        math::*,
        props::{Blast, Collider, Event},
    },
};
use codol_protocol::game::{
    channel::usercmd::UserCommand,
    snapshot::{
        Ammo, Equipment, EquippedWeapon, Firing,
        events::EventQueue,
        missile::MissileSnapshot,
        props::{PropSnapshot, VehiclePose},
    },
};

pub(crate) const FIRST_ENTITY: u16 = 624;
pub(crate) const END_ENTITY: u16 = 640;
const FIRST_CAMERA: u16 = 640;
use codol_protocol::game::snapshot::missile::{
    END_GUNNER_ENTITY as END_SHELL, FIRST_GUNNER_ENTITY as FIRST_SHELL,
};
const RIDE_MSEC: u32 = 40_000;

#[derive(Clone, Copy)]
struct Gun {
    weapon: u16,
    clip_key: u16,
    ammo_key: u16,
    magazine: u16,
    reload: u32,
    interval: u32,
    bullet: Option<Ballistics>,
    shell: Option<ShellWeapon>,
}
#[derive(Clone, Copy)]
struct ShellWeapon {
    speed: f32,
    lifetime: u32,
    radius: f32,
    inner: u16,
    outer: u16,
}
impl Gun {
    fn load(w: &WeaponFacts, shell: bool) -> Option<Self> {
        if !(1..900).contains(&w.weapon) || w.fire_interval_msec == 0 || w.clip_size > 511 {
            return None;
        }
        let bullet = if shell {
            None
        } else {
            if w.weapon_type != 0 {
                return None;
            }
            let falloff = DamageFalloff::from_stages(w.damage_stages, w.damage_ranges)?;
            let range = w
                .damage_ranges
                .iter()
                .copied()
                .fold(0.0, f32::max)
                .min(32768.0);
            let b = Ballistics {
                damage: falloff.damage(0.0),
                range,
                near_range: range,
                location_multipliers: None,
                falloff: Some(falloff),
                pellets: 1,
                spread_degrees: 0.0,
            };
            if !b.valid() {
                return None;
            }
            Some(b)
        };
        let shell = if shell {
            let p = w.projectile?;
            if w.weapon_type != 2
                || !(1..=50000).contains(&w.projectile_speed)
                || p.explosion_kind != 1
                || !(1..=60000).contains(&p.lifetime_msec)
                || !(1..=8192).contains(&w.radius)
                || !(1..=65535).contains(&w.inner_damage)
                || w.outer_damage > 65535
                || w.clip_size == 0
                || w.reload_msec == 0
            {
                return None;
            }
            Some(ShellWeapon {
                speed: w.projectile_speed as f32,
                lifetime: p.lifetime_msec,
                radius: w.radius as f32,
                inner: w.inner_damage as u16,
                outer: w.outer_damage as u16,
            })
        } else {
            None
        };
        Some(Self {
            weapon: w.weapon,
            clip_key: w.clip_key?,
            ammo_key: w.ammo_key?,
            magazine: w.clip_size,
            reload: u32::from(w.reload_msec),
            interval: u32::from(w.fire_interval_msec),
            bullet,
            shell,
        })
    }
}
enum Flight {
    Helicopter {
        motion: Motion,
        patrol: Path,
        ingress: bool,
        vehicle: u8,
        rotor: super::killstreaks::AnimationBinding,
    },
    Orbit {
        center: V3,
        mount: CameraMount,
        phase: f32,
        began: u32,
        heading: f32,
    },
}
pub(crate) struct Plan {
    pub entity: u16,
    pub id: u8,
    model: u16,
    camera_model: u16,
    bounds: ModelBounds,
    mount: CameraMount,
    slot: u8,
    flight: Flight,
    guns: Vec<Gun>,
    fx: u16,
    vision: Option<&'static str>,
    inverted_vision: Option<&'static str>,
}
struct Ride {
    key: Key,
    source: Source,
    plan: Plan,
    origin: V3,
    body_angles: [u16; 3],
    camera: V3,
    aim: V3,
    selected: usize,
    clips: Vec<u16>,
    ready_at: Vec<u32>,
    reload_at: Vec<u32>,
    events: EventQueue,
    last: u32,
    expires: u32,
    health: u16,
    identity_end: u32,
    input_time: u32,
    buttons: u32,
    armed: bool,
    thermal: bool,
    input_angles: Option<[u16; 3]>,
}
struct Shell {
    state: MissileSnapshot,
    source: Source,
    weapon: ShellWeapon,
    velocity: V3,
    last: u32,
    expires: u32,
    carrier: u16,
}
#[derive(Default)]
pub(crate) struct Gunners {
    ride: Option<Ride>,
    generation: u32,
    shells: Vec<Shell>,
    shots: Vec<DeviceShot>,
    hits: Vec<Hit>,
    events: Vec<Event>,
}
impl Gunners {
    pub(crate) fn targets(&self) -> Vec<super::airspace::Target> {
        self.ride
            .iter()
            .map(|r| super::airspace::Target {
                entity: r.key.entity,
                center: r.origin,
                source: super::killstreaks::SupportFlight {
                    actor: r.source.owner,
                    id: r.plan.id,
                    team: r.source.team,
                    start: r.key.generation,
                    end: r.identity_end,
                },
            })
            .collect()
    }
    fn guns(r: &RadarResources, id: u8) -> Option<Vec<Gun>> {
        let names: &[&str] = match id {
            12 => &["cobra_player_minigun_mp"],
            13 => &["ac130_105mm_mp", "ac130_40mm_mp", "ac130_25mm_mp"],
            _ => return None,
        };
        names
            .iter()
            .map(|n| Gun::load(r.weapons.get(*n)?, id == 13))
            .collect()
    }
    pub(crate) fn resources_present(r: &RadarResources, id: u8) -> bool {
        let model = if id == 12 {
            "vehicle_mi-28_mp"
        } else {
            "vehicle_ac130_low_mp"
        };
        let mount = if id == 12 { model } else { "c130_zoomrig" };
        matches!(id, 12 | 13)
            && Self::guns(r, id).is_some()
            && r.models.contains_key(model)
            && r.model_bounds.get(model).is_some_and(|b| b.valid())
            && r.models.contains_key("tag_origin")
            && r.models.contains_key("c130_zoomrig")
            && r.camera_mounts.get(mount).is_some_and(|m| m.valid())
            && r.tags.contains_key("tag_origin")
            && (id != 12
                || r.air_paths.as_ref().is_some_and(|paths| {
                    paths.valid()
                        && paths.entries.contains_key("heli_start")
                        && paths.entries.contains_key("heli_loop_start")
                }))
            && (id != 13
                || r.air_route.as_ref().is_some_and(|route| {
                    route
                        .center
                        .iter()
                        .all(|value| value.is_finite() && value.abs() <= 30000.0)
                }))
            && (id != 12
                || r.vehicles.contains_key("cobra_minigun_mp")
                    && r.animations.contains_key("bh_rotors"))
            && r.effects.contains_key("explosions/aerial_explosion")
    }
    pub(crate) fn plan(
        &self,
        r: &RadarResources,
        id: u8,
        reserved: u16,
        c: &Collision,
    ) -> Option<Plan> {
        if self.ride.is_some() || !Self::resources_present(r, id) {
            return None;
        }
        let slot = (0..16u8).find(|s| reserved & (1 << s) == 0)?;
        let entity = FIRST_ENTITY + u16::from(slot);
        let model = if id == 12 {
            "vehicle_mi-28_mp"
        } else {
            "vehicle_ac130_low_mp"
        };
        let mount = r.camera_mounts[if id == 12 { model } else { "c130_zoomrig" }];
        let bounds = r.model_bounds[model];
        let flight = if id == 12 {
            let g = r.air_paths.as_ref()?;
            let mut accepted = None;
            for &start in g.entries.get("heli_start")? {
                let mut path = routes::authored(g, start, false)?;
                if !routes::fit(c, bounds, &mut path, None, Some(entity)) {
                    continue;
                }
                for &root in g.entries.get("heli_loop_start")? {
                    let mut patrol = routes::authored(g, root, false)?;
                    if routes::fit(
                        c,
                        bounds,
                        &mut patrol,
                        path.nodes.last().map(|n| n.origin),
                        Some(entity),
                    ) {
                        accepted = Some((path.clone(), patrol));
                        break;
                    }
                }
                if accepted.is_some() {
                    break;
                }
            }
            let (path, patrol) = accepted?;
            Flight::Helicopter {
                motion: Motion::new(path),
                patrol,
                ingress: true,
                vehicle: r.vehicles["cobra_minigun_mp"],
                rotor: r.animations["bh_rotors"],
            }
        } else {
            let center = r.air_route.as_ref()?.center;
            let radius = length(std::array::from_fn(|i| {
                bounds.min[i].abs().max(bounds.max[i].abs())
            }));
            let (min, max) = ([-radius; 3], [radius; 3]);
            // A complete revolution checks the true rig's radius and height,
            // not an invented map route or another map's vehicle resources.
            for i in 0..64 {
                let a = add(
                    center,
                    rotate_yaw(mount.translation, 115.0 + i as f32 * 360.0 / 64.0),
                );
                let b = add(
                    center,
                    rotate_yaw(mount.translation, 115.0 + (i + 1) as f32 * 360.0 / 64.0),
                );
                if !valid(a) || !valid(b) {
                    return None;
                }
                let t = c.trace_bounds(a, b, min, max, Some(entity));
                if t.start_solid || t.all_solid || t.fraction < 1.0 {
                    return None;
                }
            }
            Flight::Orbit {
                center,
                mount,
                phase: 115.0,
                began: 0,
                heading: 115.0,
            }
        };
        Some(Plan {
            entity,
            id,
            model: r.models[model],
            camera_model: r.models["c130_zoomrig"],
            bounds,
            mount,
            slot,
            flight,
            guns: Self::guns(r, id)?,
            fx: r.effects["explosions/aerial_explosion"],
            inverted_vision: r.remote_vision.as_ref().and_then(|v| v.inverted_name()),
            vision: match &r.remote_vision {
                Some(v) => Some(v.flight_name()?),
                None => None,
            },
        })
    }
    pub(crate) fn launch(
        &mut self,
        mut plan: Plan,
        now: u32,
        generation: u32,
        source: Source,
        c: &Collision,
    ) -> (Key, Option<&'static str>) {
        let vision = plan.vision;
        if let Flight::Orbit { began, .. } = &mut plan.flight {
            *began = now;
        }
        let clips = plan.guns.iter().map(|g| g.magazine).collect::<Vec<_>>();
        let guns = clips.len();
        let key = Key {
            entity: plan.entity,
            generation,
        };
        let mut ride = Ride {
            key,
            source,
            origin: [0.0; 3],
            body_angles: [0; 3],
            camera: [0.0; 3],
            aim: [45.0, 0.0, 0.0],
            selected: 0,
            clips,
            ready_at: vec![now; guns],
            reload_at: vec![0; guns],
            events: Default::default(),
            last: now,
            expires: now.saturating_add(if plan.id == 12 { 120_000 } else { RIDE_MSEC }),
            identity_end: now.saturating_add(120_000),
            health: if plan.id == 12 { 1500 } else { 1000 },
            input_time: now,
            buttons: 0,
            armed: false,
            thermal: true,
            input_angles: None,
            plan,
        };
        ride.pose(now);
        let yaw = match &ride.plan.flight {
            Flight::Orbit { phase, .. } => *phase,
            _ => f32::from(ride.body_angles[1]) * 360.0 / 65536.0,
        };
        ride.aim = direction_angles(rotate_yaw(mount_forward(ride.plan.mount), yaw));
        ride.aim[0] = ride.aim[0].clamp(if ride.plan.id == 13 { 25.0 } else { 0.0 }, 85.0);
        self.ride = Some(ride);
        self.sync(c);
        (key, vision)
    }
    pub(crate) fn key(&self) -> Option<Key> {
        self.ride.as_ref().map(|r| r.key)
    }
    pub(crate) fn selected_weapon(&self) -> Option<u16> {
        self.ride.as_ref().map(|r| r.plan.guns[r.selected].weapon)
    }
    pub(crate) fn vision(&self) -> Option<(&'static str, u16)> {
        let r = self.ride.as_ref()?;
        if r.thermal {
            Some((r.plan.vision?, 510))
        } else {
            Some((r.plan.inverted_vision?, 620))
        }
    }
    pub(crate) fn camera(&self, key: Key) -> Option<u16> {
        self.ride.as_ref().filter(|r| r.key == key).map(|r| {
            if r.plan.id == 12 {
                r.key.entity
            } else {
                FIRST_CAMERA + r.key.entity - FIRST_ENTITY
            }
        })
    }
    pub(crate) fn angles(&self) -> Option<[u16; 3]> {
        self.ride.as_ref().map(|r| short_angles(r.aim))
    }
    pub(crate) fn slots(&self) -> u16 {
        self.ride
            .as_ref()
            .filter(|r| r.plan.id == 12)
            .map_or(0, |r| 1 << r.plan.slot)
    }
    pub(crate) fn input(&mut self, input: &UserCommand) -> bool {
        let Some(r) = &mut self.ride else {
            return false;
        };
        if input.time <= r.input_time {
            return true;
        }
        let dt = (input.time - r.input_time).min(200) as f32 * 0.001;
        r.input_time = input.time;
        if input.buttons & codol_protocol::game::channel::usercmd::BUTTON_USE != 0
            && r.buttons & codol_protocol::game::channel::usercmd::BUTTON_USE == 0
            && r.plan.inverted_vision.is_some()
        {
            r.thermal = !r.thermal;
        }
        r.buttons = input.buttons;
        if input.buttons & 1 == 0 {
            r.armed = true;
        }
        if let Some(axes) = input.remote_axes() {
            r.aim[0] = (r.aim[0] + axes[0] * 60.0 * dt)
                .clamp(if r.plan.id == 13 { 25.0 } else { 0.0 }, 85.0);
            r.aim[1] = (r.aim[1] + axes[1] * 60.0 * dt).rem_euclid(360.0);
        } else if let Some(old) = r.input_angles {
            let delta = std::array::from_fn::<_, 3, _>(|i| {
                f32::from(input.angles[i].wrapping_sub(old[i]) as i16) * 360.0 / 65536.0
            });
            r.aim[0] = (r.aim[0] + delta[0]).clamp(if r.plan.id == 13 { 25.0 } else { 0.0 }, 85.0);
            r.aim[1] = (r.aim[1] + delta[1]).rem_euclid(360.0);
        }
        r.input_angles = Some(input.angles);
        if let Some(i) = r
            .plan
            .guns
            .iter()
            .position(|g| g.weapon == input.weapons[0])
        {
            r.selected = i;
        }
        true
    }
    pub(crate) fn equipment(&self, mut e: Equipment) -> Option<Equipment> {
        let r = self.ride.as_ref()?;
        let g = r.plan.guns[r.selected];
        let equipped = |i: usize| EquippedWeapon {
            weapon: r.plan.guns[i].weapon,
            ammo: r.ammo(i),
            needs_rechamber: false,
        };
        e.weapon = g.weapon;
        e.viewmodel = None;
        e.inventory_weapon = None;
        let mut slots = [None; 15];
        for (i, slot) in slots.iter_mut().enumerate().take(r.plan.guns.len()) {
            *slot = Some(equipped(i));
        }
        e.inventory_override = Some(slots);
        e.alternate_weapon = None;
        e.alternates = [None, None];
        e.secondary = None;
        e.ammo = r.ammo(r.selected);
        e.offhands = [None, None];
        e.offhand_weapon = 0;
        e.grenade_time = 0;
        e.events = Some(r.events);
        e.aim = None;
        e.fire_count = 0;
        e.queued_burst = false;
        e.needs_rechamber = false;
        let reloading = r.reload_at[r.selected] > r.last;
        let remaining = if reloading {
            r.reload_at[r.selected]
        } else {
            r.ready_at[r.selected]
        }
        .saturating_sub(r.last)
        .min(15000) as u16;
        e.firing = Some(Firing {
            state: if reloading {
                8
            } else if remaining > 0 {
                6
            } else {
                0
            },
            animation: if reloading { 0x400 | 15 } else { 0x400 },
            remaining_msec: remaining,
            delay_msec: 0,
        });
        Some(e)
    }
    pub(crate) fn damage(&mut self, entity: u16, damage: u16, team: u8) -> bool {
        if let Some(r) = &mut self.ride
            && r.key.entity == entity
        {
            if team != r.source.team {
                r.health = r.health.saturating_sub(damage);
            }
            return true;
        }
        false
    }
    pub(crate) fn radial(&mut self, blast: Blast, team: u8, c: &Collision) {
        if let Some(r) = &mut self.ride
            && team != r.source.team
        {
            let d = crate::game::world::props::radius_damage(
                blast,
                r.origin,
                0.0,
                Some(r.key.entity),
                c,
            );
            r.health = r.health.saturating_sub(d);
        }
    }
    pub(crate) fn advance(
        &mut self,
        now: u32,
        finished: bool,
        emp: u8,
        operator: Option<Key>,
        c: &Collision,
    ) -> Option<Key> {
        let mut ended = None;
        if let Some(mut r) = self.ride.take() {
            let cancelled = finished || emp & (1 << r.source.team) != 0 || operator != Some(r.key);
            let mut blocked = false;
            if !cancelled {
                while r.last < now.min(r.expires) && r.health > 0 {
                    let msec = (now.min(r.expires) - r.last).min(20);
                    r.last += msec;
                    if let Flight::Helicopter {
                        motion,
                        patrol,
                        ingress,
                        ..
                    } = &mut r.plan.flight
                    {
                        match motion.step(
                            r.last,
                            msec as f32 * 0.001,
                            r.plan.bounds,
                            r.key.entity,
                            c,
                        ) {
                            Movement::Blocked => {
                                blocked = true;
                                break;
                            }
                            Movement::Done if *ingress => {
                                *ingress = false;
                                motion.replace(patrol.clone());
                                r.expires = r.last.saturating_add(RIDE_MSEC);
                            }
                            _ => {}
                        }
                    }
                    r.pose(r.last);
                    for i in 0..r.clips.len() {
                        if r.reload_at[i] > 0 && r.last >= r.reload_at[i] {
                            r.clips[i] = r.plan.guns[i].magazine;
                            r.reload_at[i] = 0;
                        }
                    }
                    let i = r.selected;
                    let g = r.plan.guns[i];
                    if !r.armed
                        || r.buttons & 1 == 0
                        || r.last < r.input_time
                        || r.last.saturating_sub(r.input_time) > 200
                        || r.last < r.ready_at[i]
                        || r.reload_at[i] != 0
                        || self.shots.len() >= 64
                    {
                        continue;
                    }
                    if let Some(w) = g.shell {
                        let Some(entity) = (FIRST_SHELL..END_SHELL)
                            .find(|e| self.shells.iter().all(|s| s.state.entity_number != *e))
                        else {
                            continue;
                        };
                        self.generation = self
                            .generation
                            .checked_add(1)
                            .expect("gunner shell generation exhausted");
                        self.shells.push(Shell {
                            state: MissileSnapshot {
                                entity_number: entity,
                                origin: r.camera,
                                weapon: g.weapon,
                                present: true,
                                generation: self.generation,
                                owner: r.source.owner,
                                angles: short_angles(r.aim),
                                events: Default::default(),
                            },
                            source: r.source,
                            weapon: w,
                            velocity: mul(forward(r.aim), w.speed),
                            last: r.last,
                            expires: r.last.saturating_add(w.lifetime),
                            carrier: r.key.entity,
                        });
                    } else if let Some(ballistics) = g.bullet {
                        self.shots.push(DeviceShot {
                            owner: r.source.owner,
                            team: r.source.team,
                            life: r.source.life,
                            entity: r.key.entity,
                            eye: r.camera,
                            angles: short_angles(r.aim),
                            weapon: g.weapon,
                            ballistics,
                        });
                    }
                    r.events
                        .push(codol_protocol::game::snapshot::events::FIRE_WEAPON, 0)
                        .expect("native gunner fire event");
                    r.ready_at[i] = r.last.saturating_add(g.interval);
                    if g.magazine > 0 {
                        r.clips[i] -= 1;
                        if r.clips[i] == 0 {
                            r.reload_at[i] = r.last.saturating_add(g.reload);
                        }
                    }
                }
            }
            if cancelled || blocked || r.health == 0 || now >= r.expires {
                ended = Some(r.key);
                if !cancelled && (blocked || r.health == 0) {
                    self.events.push(Event::Fx {
                        selector: r.plan.fx,
                        origin: r.origin,
                    });
                }
            } else {
                self.ride = Some(r);
            }
        }
        self.sync(c);
        let mut keep = Vec::new();
        for mut s in self.shells.drain(..) {
            if finished || emp & (1 << s.source.team) != 0 {
                continue;
            }
            let mut impact = None;
            let mut invalid = false;
            while s.last < now.min(s.expires) {
                let msec = (now.min(s.expires) - s.last).min(20);
                s.last += msec;
                let end = add(s.state.origin, mul(s.velocity, msec as f32 * 0.001));
                let t = c.raycast_excluding_prop(s.state.origin, end, Some(s.carrier));
                if t.start_solid || t.all_solid || t.fraction < 1.0 {
                    let approach = -dot(sub(end, s.state.origin), t.normal);
                    let f = if approach > 0.0 {
                        (t.fraction - 0.125 / approach).max(0.0)
                    } else {
                        t.fraction
                    };
                    s.state.origin = add(s.state.origin, mul(sub(end, s.state.origin), f));
                    impact = Some((t.normal, ((t.surface_flags >> 20) & 31) as u8));
                    break;
                }
                s.state.origin = end;
                if !valid(end) {
                    invalid = true;
                    break;
                }
            }
            if invalid {
                continue;
            }
            if impact.is_none() && now >= s.expires {
                impact = Some(([0.0, 0.0, 1.0], 0));
            }
            if let Some((normal, surface)) = impact {
                self.hits.push(Hit {
                    blast: Blast {
                        owner: s.source.owner,
                        source_entity: Some(s.state.entity_number),
                        weapon: s.state.weapon,
                        origin: s.state.origin,
                        radius: s.weapon.radius,
                        inner: s.weapon.inner,
                        outer: s.weapon.outer,
                        reward_life: Some(s.source.life),
                    },
                    team: s.source.team,
                    life: s.source.life,
                    airstrike: false,
                    area: None,
                });
                self.events.push(Event::Rocket {
                    weapon: s.state.weapon,
                    origin: s.state.origin,
                    normal,
                    surface,
                });
            } else {
                keep.push(s);
            }
        }
        self.shells = keep;
        ended
    }
    fn sync(&self, c: &Collision) {
        let shapes = self
            .ride
            .iter()
            .map(|r| {
                Collider::new(
                    r.key.entity,
                    r.origin,
                    r.body_angles,
                    r.plan.bounds.min,
                    r.plan.bounds.max,
                )
            })
            .collect::<Vec<_>>();
        c.replace_prop_range(FIRST_ENTITY..END_ENTITY, &shapes);
    }
    pub(crate) fn props(&self) -> Vec<PropSnapshot> {
        let Some(r) = &self.ride else {
            return Vec::new();
        };
        let vehicle = match &r.plan.flight {
            Flight::Helicopter { vehicle, rotor, .. } => Some(VehiclePose {
                definition: *vehicle,
                slot: r.plan.slot,
                tree: rotor.tree,
                gun: [
                    r.aim[0],
                    (r.aim[1] - f32::from(r.body_angles[1]) * 360.0 / 65536.0 + 180.0)
                        .rem_euclid(360.0)
                        - 180.0,
                ],
                animation: rotor.node,
                animation_parent: rotor.parent,
                loop_sound: 0,
                animation_time: r.last,
            }),
            _ => None,
        };
        let body = PropSnapshot {
            actor: None,
            entity_number: r.key.entity,
            model: r.plan.model,
            origin: r.origin,
            angles: r.body_angles,
            present: true,
            weapon: r.plan.guns[r.selected].weapon,
            events: r.events,
            turret: false,
            turret_pose: None,
            vehicle,
        };
        if r.plan.id == 12 {
            return vec![body];
        }
        let (origin, angles) = match &r.plan.flight {
            Flight::Orbit {
                center, heading, ..
            } => (*center, short_angles([0.0, *heading, 0.0])),
            _ => unreachable!(),
        };
        vec![
            body,
            PropSnapshot {
                actor: None,
                entity_number: FIRST_CAMERA + r.key.entity - FIRST_ENTITY,
                model: r.plan.camera_model,
                origin,
                angles,
                weapon: 0,
                events: Default::default(),
                vehicle: None,
                ..body
            },
        ]
    }
    pub(crate) fn shells(&self) -> Vec<MissileSnapshot> {
        let mut s = self.shells.iter().map(|s| s.state).collect::<Vec<_>>();
        s.sort_by_key(|s| s.entity_number);
        s
    }
    pub(crate) fn drain_hits(&mut self) -> Vec<Hit> {
        std::mem::take(&mut self.hits)
    }
    pub(crate) fn drain_shots(&mut self) -> Vec<DeviceShot> {
        std::mem::take(&mut self.shots)
    }
    pub(crate) fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}
impl Ride {
    fn pose(&mut self, now: u32) {
        match &mut self.plan.flight {
            Flight::Helicopter { motion, .. } => {
                self.origin = motion.origin;
                self.body_angles = motion.angles;
                self.camera = add(
                    self.origin,
                    rotate_yaw(
                        self.plan.mount.translation,
                        f32::from(self.body_angles[1]) * 360.0 / 65536.0,
                    ),
                );
            }
            Flight::Orbit {
                center,
                mount,
                phase,
                began,
                heading,
            } => {
                let yaw = *phase + now.saturating_sub(*began) as f32 * 360.0 / 70000.0;
                self.aim[1] = (self.aim[1] + yaw - *heading).rem_euclid(360.0);
                *heading = yaw;
                self.camera = add(*center, rotate_yaw(mount.translation, yaw));
                self.origin = add(self.camera, rotate_yaw([0.0, 80.0, 32.0], yaw));
                self.body_angles = short_angles([-25.0, yaw, 0.0]);
            }
        }
    }
    fn ammo(&self, i: usize) -> Option<Ammo> {
        let g = self.plan.guns[i];
        (g.magazine > 0).then_some(Ammo {
            clip_key: g.clip_key,
            reserve_key: g.ammo_key,
            clip: self.clips[i],
            // The script replenishes magazines throughout the ride. A spare
            // magazine keeps native prediction on its reload path rather
            // than auto-selecting another cannon before the fire command.
            reserve: if g.shell.is_some() { g.magazine } else { 0 },
        })
    }
}
fn valid(p: V3) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 32700.0)
}
fn rotate_yaw(p: V3, yaw: f32) -> V3 {
    let (s, c) = yaw.to_radians().sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]]
}
fn mount_forward(m: CameraMount) -> V3 {
    let n = m.rotation.iter().map(|v| v * v).sum::<f32>().sqrt();
    let [x, y, z, w] = m.rotation.map(|v| v / n);
    [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + w * z),
        2.0 * (x * z - w * y),
    ]
}
fn direction_angles(d: V3) -> V3 {
    [
        -d[2].atan2(d[0].hypot(d[1])).to_degrees(),
        d[1].atan2(d[0]).to_degrees(),
        0.0,
    ]
}
fn short_angles(a: V3) -> [u16; 3] {
    a.map(|a| (a * 65536.0 / 360.0).round() as i32 as u16)
}
fn forward(a: V3) -> V3 {
    let (p, y) = (a[0].to_radians(), a[1].to_radians());
    [p.cos() * y.cos(), p.cos() * y.sin(), -p.sin()]
}

#[cfg(test)]
pub(super) mod tests;
