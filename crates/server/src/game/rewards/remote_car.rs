//! RC ground motion and detonation. Placement offsets and the 30s timer follow
//! the Alpha compatibility script; driving constants are local compatibility
//! rules, independent of the captured model and weapon registrations.
use super::{
    killstreaks::{ModelBounds, RadarResources},
    strikes::{Hit, Source},
};
use crate::game::world::{
    collision::Collision,
    math::*,
    props::{Blast, Collider, Event},
};
use codol_protocol::game::{channel::usercmd::UserCommand, snapshot::props::PropSnapshot};

pub(crate) const FIRST_ENTITY: u16 = 608;
pub(crate) const END_ENTITY: u16 = 624;
const LIFETIME: u32 = 30_000;
const SKIN: f32 = 0.125;

#[derive(Clone, Copy)]
struct Definition {
    model: u16,
    vehicle: u8,
    bounds: ModelBounds,
    weapon: u16,
    radius: f32,
    inner: u16,
    outer: u16,
    fx: u16,
}
impl Definition {
    fn load(r: &RadarResources, team: u8) -> Option<Self> {
        let model = match team {
            1 => "t5_veh_rcbomb_axis",
            2 => "t5_veh_rcbomb_allies",
            _ => return None,
        };
        let w = r.weapons.get("rcbomb_mp")?;
        let bounds = *r.model_bounds.get(model).filter(|b| b.valid())?;
        if !(1..900).contains(&w.weapon)
            || w.weapon_class != 11
            || w.radius == 0
            || w.radius > 8192
            || w.inner_damage == 0
            || w.inner_damage > u32::from(u16::MAX)
            || w.outer_damage > u32::from(u16::MAX)
        {
            return None;
        }
        Some(Self {
            model: *r.models.get(model).filter(|&&v| (1..1024).contains(&v))?,
            vehicle: *r
                .vehicles
                .get("rc_car_medium_mp")
                .filter(|&&v| (1..32).contains(&v))?,
            bounds,
            weapon: w.weapon,
            radius: w.radius as f32,
            inner: w.inner_damage as u16,
            outer: w.outer_damage as u16,
            fx: *r.effects.get("maps/mp_maps/fx_mp_exp_rc_bomb")?,
        })
    }
}
pub(crate) struct Plan {
    entity: u16,
    slot: u8,
    definition: Definition,
    origin: V3,
    yaw: f32,
}
pub(crate) struct Car {
    pub key: super::remote::Key,
    pub slot: u8,
    definition: Definition,
    pub source: Source,
    origin: V3,
    yaw: f32,
    speed: f32,
    vertical_speed: f32,
    pub last: u32,
    expires: u32,
    input_time: u32,
    direction: u8,
    armed: bool,
    detonate: bool,
    health: u16,
}
impl Car {
    pub(crate) fn resources_present(r: &RadarResources) -> bool {
        Definition::load(r, 1).is_some() && Definition::load(r, 2).is_some()
    }
    pub(crate) fn plan(
        r: &RadarResources,
        entity: u16,
        team: u8,
        origin: V3,
        angles: [u16; 3],
        (occupied, reserved_slots): (&[V3], u16),
        c: &Collision,
    ) -> Option<Plan> {
        let definition = Definition::load(r, team)?;
        let slot = (0..16).find(|slot| reserved_slots & (1 << slot) == 0)?;
        if !(FIRST_ENTITY..END_ENTITY).contains(&entity)
            || origin.iter().any(|v| !v.is_finite() || v.abs() > 30000.0)
        {
            return None;
        }
        let yaw = f32::from(angles[1]) * 360.0 / 65536.0;
        for offset in [0.0f32, 20.0, -20.0, 45.0, -45.0] {
            let a = (yaw + offset).to_radians();
            let above = add(origin, [70.0 * a.cos(), 70.0 * a.sin(), 32.0]);
            let below = add(above, [0.0, 0.0, -132.0]);
            let shape = rotated_bounds(entity, definition.bounds, yaw);
            let floor = c.trace_bounds(above, below, shape.min, shape.max, Some(entity));
            if floor.start_solid
                || floor.all_solid
                || floor.fraction >= 1.0
                || floor.normal[2] < 0.7
            {
                continue;
            }
            let site = add(
                add(above, mul(sub(below, above), floor.fraction)),
                [0.0, 0.0, SKIN],
            );
            let trace = c.trace_bounds(site, site, shape.min, shape.max, Some(entity));
            let visible = c.raycast(add(origin, [0.0, 0.0, 24.0]), add(site, [0.0, 0.0, 8.0]));
            if !trace.start_solid
                && !trace.all_solid
                && !visible.start_solid
                && !visible.all_solid
                && visible.fraction >= 1.0
                && occupied.iter().all(|p| {
                    (p[0] - site[0]).hypot(p[1] - site[1]) > 32.0 || (p[2] - site[2]).abs() > 80.0
                })
            {
                return Some(Plan {
                    entity,
                    slot,
                    definition,
                    origin: site,
                    yaw,
                });
            }
        }
        None
    }
    pub(crate) fn launch(plan: Plan, now: u32, generation: u32, source: Source) -> Self {
        Self {
            slot: plan.slot,
            key: super::remote::Key {
                entity: plan.entity,
                generation,
            },
            definition: plan.definition,
            source,
            origin: plan.origin,
            yaw: plan.yaw,
            speed: 0.0,
            vertical_speed: 0.0,
            last: now,
            expires: now.saturating_add(LIFETIME),
            input_time: now,
            direction: 0,
            armed: false,
            detonate: false,
            health: 100,
        }
    }
    pub(crate) fn snapshot(&self) -> PropSnapshot {
        PropSnapshot {
            actor: None,
            entity_number: self.key.entity,
            model: self.definition.model,
            origin: self.origin,
            angles: [0, (self.yaw * 65536.0 / 360.0).round() as i32 as u16, 0],
            present: true,
            weapon: self.definition.weapon,
            turret: false,
            turret_pose: None,
            vehicle: Some(codol_protocol::game::snapshot::props::VehiclePose {
                definition: self.definition.vehicle,
                slot: self.slot,
                tree: 0,
                gun: [0.0; 2],
                animation: 0,
                animation_parent: 0,
                loop_sound: 0,
                animation_time: self.last,
            }),
            events: Default::default(),
        }
    }
    pub(crate) fn collider(&self) -> Collider {
        Collider::new(
            self.key.entity,
            self.origin,
            self.snapshot().angles,
            self.definition.bounds.min,
            self.definition.bounds.max,
        )
    }
    pub(crate) fn input(&mut self, input: &UserCommand) {
        if input.time <= self.input_time {
            return;
        }
        self.input_time = input.time;
        self.direction = input.direction & 15;
        if input.buttons & 1 == 0 {
            self.armed = true;
        } else if self.armed {
            self.detonate = true;
        }
    }
    pub(crate) fn damage(&mut self, damage: u16, team: u8) {
        if team != self.source.team {
            self.health = self.health.saturating_sub(damage);
        }
    }
    pub(crate) fn radial(&mut self, blast: Blast, team: u8, c: &Collision) {
        if team == self.source.team || blast.radius <= 0.0 {
            return;
        }
        let point = add(self.origin, [0.0, 0.0, 8.0]);
        let distance = length(sub(point, blast.origin));
        let trace = c.raycast(blast.origin, point);
        if distance < blast.radius
            && !trace.start_solid
            && !trace.all_solid
            && (trace.fraction >= 1.0 || trace.prop_entity == Some(self.key.entity))
        {
            let damage = f32::from(blast.inner)
                + (f32::from(blast.outer) - f32::from(blast.inner)) * distance / blast.radius;
            self.damage(damage.max(0.0) as u16, team);
        }
    }
    /// true removes the car. Only detonation/destruction/expiry emit damage;
    /// cancellation is owned by the caller and never creates an explosion.
    pub(crate) fn advance(&mut self, now: u32, c: &Collision) -> bool {
        let through = now.min(self.expires);
        while self.last < through && !self.detonate && self.health > 0 {
            let msec = (through - self.last).min(20);
            self.last += msec;
            let dt = msec as f32 * 0.001;
            let direction = if self.last.saturating_sub(self.input_time) <= 200 {
                self.direction
            } else {
                0
            };
            let throttle = if direction & 1 != 0 {
                1.0
            } else if direction & 2 != 0 {
                -1.0
            } else {
                0.0
            };
            let steer = if direction & 4 != 0 {
                -1.0
            } else if direction & 8 != 0 {
                1.0
            } else {
                0.0
            };
            let target = throttle * if throttle < 0.0 { 180.0 } else { 400.0 };
            let acceleration = if throttle == 0.0 { 800.0 } else { 600.0 };
            self.speed += (target - self.speed).clamp(-acceleration * dt, acceleration * dt);
            self.yaw += steer * self.speed / 400.0 * 120.0 * dt;
            let shape = rotated_bounds(self.key.entity, self.definition.bounds, self.yaw);
            // Reject rotation into a wall; restore the previous orientation.
            let overlap = c.trace_bounds(
                self.origin,
                self.origin,
                shape.min,
                shape.max,
                Some(self.key.entity),
            );
            if overlap.start_solid || overlap.all_solid {
                self.yaw -= steer * self.speed / 400.0 * 120.0 * dt;
                self.speed = 0.0;
            }
            let shape = rotated_bounds(self.key.entity, self.definition.bounds, self.yaw);
            let a = self.yaw.to_radians();
            let horizontal = [a.cos() * self.speed * dt, a.sin() * self.speed * dt, 0.0];
            if self.ground_step(horizontal, &shape, c) {
                self.vertical_speed = 0.0;
                if !self.in_bounds() {
                    return true;
                }
                continue;
            }
            self.vertical_speed -= 800.0 * dt;
            let end = add(
                self.origin,
                [
                    a.cos() * self.speed * dt,
                    a.sin() * self.speed * dt,
                    self.vertical_speed * dt,
                ],
            );
            let trace = c.trace_bounds(
                self.origin,
                end,
                shape.min,
                shape.max,
                Some(self.key.entity),
            );
            if trace.start_solid || trace.all_solid {
                self.speed = 0.0;
                self.vertical_speed = 0.0;
            } else if trace.fraction < 1.0 {
                self.origin = add(
                    add(self.origin, mul(sub(end, self.origin), trace.fraction)),
                    mul(trace.normal, SKIN),
                );
                if trace.normal[2] >= 0.7 {
                    self.vertical_speed = 0.0;
                } else {
                    self.speed = 0.0;
                }
                // Complete the horizontal portion after a floor contact.
                let slide = add(
                    self.origin,
                    [
                        a.cos() * self.speed * dt * (1.0 - trace.fraction),
                        a.sin() * self.speed * dt * (1.0 - trace.fraction),
                        0.0,
                    ],
                );
                let t = c.trace_bounds(
                    self.origin,
                    slide,
                    shape.min,
                    shape.max,
                    Some(self.key.entity),
                );
                if !t.start_solid && !t.all_solid {
                    self.origin = add(
                        self.origin,
                        mul(sub(slide, self.origin), (t.fraction - 0.001).max(0.0)),
                    );
                }
            } else {
                self.origin = end;
            }
            if self
                .origin
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 32700.0)
            {
                return true;
            }
        }
        self.detonate || self.health == 0 || now >= self.expires
    }
    fn ground_step(&mut self, delta: V3, shape: &Collider, c: &Collision) -> bool {
        let trace = |a, b| c.trace_bounds(a, b, shape.min, shape.max, Some(self.key.entity));
        let ground = trace(self.origin, add(self.origin, [0.0, 0.0, -2.0]));
        if ground.start_solid
            || ground.all_solid
            || ground.fraction >= 1.0
            || ground.normal[2] < 0.7
        {
            return false;
        }
        // A short suspension step clears terrain triangle seams. Every leg is
        // a whole-body sweep; a roof, wall or unwalkable landing rejects it.
        let raised = add(self.origin, [0.0, 0.0, 6.0]);
        let up = trace(self.origin, raised);
        if up.start_solid || up.all_solid || up.fraction < 1.0 {
            return false;
        }
        let end = add(raised, delta);
        let across = trace(raised, end);
        if across.start_solid || across.all_solid {
            return false;
        }
        let top = add(raised, mul(delta, (across.fraction - 0.001).max(0.0)));
        let below = add(top, [0.0, 0.0, -14.0]);
        let down = trace(top, below);
        if down.start_solid || down.all_solid || down.fraction >= 1.0 || down.normal[2] < 0.7 {
            return false;
        }
        self.origin = add(
            add(top, mul(sub(below, top), down.fraction)),
            [0.0, 0.0, SKIN],
        );
        if across.fraction < 1.0 {
            self.speed = 0.0;
        }
        true
    }
    pub(crate) fn explosion(&self) -> (Hit, Event) {
        (
            Hit {
                blast: Blast {
                    origin: add(self.origin, [0.0, 0.0, 10.0]),
                    radius: self.definition.radius,
                    inner: self.definition.inner,
                    outer: self.definition.outer,
                    owner: self.source.owner,
                    source_entity: Some(self.key.entity),
                    weapon: self.definition.weapon,
                    reward_life: Some(self.source.life),
                },
                team: self.source.team,
                life: self.source.life,
                airstrike: false,
                area: None,
            },
            Event::Fx {
                selector: self.definition.fx,
                origin: self.origin,
            },
        )
    }
    pub(crate) fn in_bounds(&self) -> bool {
        self.origin
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 32700.0)
    }
}
fn rotated_bounds(entity: u16, bounds: ModelBounds, yaw: f32) -> Collider {
    Collider::new(
        entity,
        [0.0; 3],
        [0, (yaw * 65536.0 / 360.0).round() as i32 as u16, 0],
        bounds.min,
        bounds.max,
    )
}

#[cfg(test)]
pub(crate) fn test_resources() -> RadarResources {
    let mut r = super::remote::test_resources();
    let mut w = r.weapons["sentry_minigun_mp"].clone();
    w.weapon = 338;
    w.weapon_class = 11;
    w.radius = 256;
    w.inner_damage = 200;
    w.outer_damage = 50;
    r.weapons.insert("rcbomb_mp".into(), w);
    for (name, id) in [("t5_veh_rcbomb_allies", 131), ("t5_veh_rcbomb_axis", 132)] {
        r.models.insert(name.into(), id);
        r.model_bounds.insert(
            name.into(),
            ModelBounds {
                min: [-14.0, -12.0, -0.5],
                max: [13.0, 12.0, 14.0],
            },
        );
    }
    r.effects
        .insert("maps/mp_maps/fx_mp_exp_rc_bomb".into(), 103);
    r.vehicles.insert("rc_car_medium_mp".into(), 7);
    r
}
#[cfg(test)]
mod tests {
    use super::*;
    fn collision(wall: bool) -> Collision {
        let f = tempfile::NamedTempFile::new().unwrap();
        let mut brushes = vec![
            serde_json::json!({"center":[0,0,-10],"half":[20000,20000,10],"contents":1,"planes":[]}),
        ];
        if wall {
            brushes.push(
                serde_json::json!({"center":[160,0,40],"half":[5,100,40],"contents":1,"planes":[]}),
            );
        }
        std::fs::write(
            f.path(),
            serde_json::to_vec(&serde_json::json!({
                "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
                "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]
            }))
            .unwrap(),
        )
        .unwrap();
        Collision::load(f.path()).unwrap()
    }
    fn car(c: &Collision) -> Car {
        let plan = Car::plan(
            &test_resources(),
            FIRST_ENTITY,
            2,
            [0.0; 3],
            [0; 3],
            (&[], 0),
            c,
        )
        .unwrap();
        Car::launch(
            plan,
            1000,
            1,
            Source {
                owner: 0,
                team: 2,
                life: 7,
            },
        )
    }
    #[test]
    #[ignore = "requires matching target Afghan reward resources and world collision"]
    fn captured_rc_car_drives_real_terrain_with_the_whole_registered_hull() {
        let r = RadarResources::load(
            std::path::Path::new(&std::env::var("CODOL_REWARD_RESOURCES").unwrap()),
            "mp_afghan",
            523897474,
        )
        .unwrap();
        let c = Collision::load(std::path::Path::new(
            &std::env::var("CODOL_COLLISION_FILE").unwrap(),
        ))
        .unwrap();
        assert!(Car::resources_present(&r));
        let mut travel = Vec::new();
        for yaw in [0, 16384, 32768, 49152] {
            let Some(plan) = Car::plan(
                &r,
                FIRST_ENTITY,
                1,
                [4147.0, 2616.0, -8.063736],
                [0, yaw, 0],
                (&[], 0),
                &c,
            ) else {
                continue;
            };
            let mut car = Car::launch(
                plan,
                1000,
                1,
                Source {
                    owner: 0,
                    team: 1,
                    life: 7,
                },
            );
            let start = car.origin;
            for time in (1020..4020).step_by(20) {
                car.input(&UserCommand {
                    time,
                    direction: 1,
                    ..Default::default()
                });
                assert!(!car.advance(time + 20, &c));
                assert!(car.in_bounds());
            }
            travel.push(length(sub(start, car.origin)));
        }
        assert!(
            travel.iter().any(|&distance| distance > 50.0),
            "real terrain travel: {travel:?}"
        );
    }
    #[test]
    fn whole_car_stops_at_wall_stale_input_brakes_and_reverse_works() {
        let c = collision(true);
        let mut car = car(&c);
        for time in (1020..2500).step_by(20) {
            car.input(&UserCommand {
                time,
                direction: 1,
                ..Default::default()
            });
            assert!(!car.advance(time + 20, &c));
        }
        assert!(
            car.origin[0] > 90.0 && car.origin[0] < 143.0,
            "{:?}",
            car.origin
        );
        assert!(car.origin[2] > 0.0 && car.origin[2] < 1.0);
        let before = car.origin;
        car.advance(5000, &c);
        assert_eq!(car.speed, 0.0);
        assert!(length(sub(before, car.origin)) < 1.0);
        for time in (5020..6020).step_by(20) {
            car.input(&UserCommand {
                time,
                direction: 2,
                ..Default::default()
            });
            car.advance(time + 20, &c);
        }
        assert!(car.origin[0] < before[0] - 50.0);
        let (hit, _) = car.explosion();
        assert_eq!(hit.life, 7);
        assert_eq!(hit.blast.reward_life, Some(7));
    }
    #[test]
    fn placement_requires_ground_clearance_visibility_and_unoccupied_site() {
        let c = collision(false);
        let r = test_resources();
        assert!(Car::plan(&r, FIRST_ENTITY, 2, [0.0; 3], [0; 3], (&[], u16::MAX), &c).is_none());
        assert_eq!(
            Car::plan(&r, FIRST_ENTITY, 2, [0.0; 3], [0; 3], (&[], 3), &c)
                .unwrap()
                .slot,
            2
        );
        assert!(
            Car::plan(
                &r,
                FIRST_ENTITY,
                2,
                [0.0, 0.0, 1000.0],
                [0; 3],
                (&[], 0),
                &c
            )
            .is_none()
        );
        let occupied = [
            [70.0, 0.0, 0.0],
            [65.8, 23.9, 0.0],
            [65.8, -23.9, 0.0],
            [49.5, 49.5, 0.0],
            [49.5, -49.5, 0.0],
        ];
        assert!(Car::plan(&r, FIRST_ENTITY, 2, [0.0; 3], [0; 3], (&occupied, 0), &c).is_none());
        assert!(Car::plan(&r, FIRST_ENTITY, 3, [0.0; 3], [0; 3], (&[], 0), &c).is_none());
        let mut missing = r.clone();
        missing.model_bounds.clear();
        assert!(!Car::resources_present(&missing));
    }
    #[test]
    fn held_activation_does_not_detonate_and_friendly_damage_is_ignored() {
        let c = collision(false);
        let mut car = car(&c);
        car.input(&UserCommand {
            time: 1020,
            buttons: 1,
            ..Default::default()
        });
        assert!(!car.advance(1040, &c));
        car.damage(1000, 2);
        assert!(!car.advance(1060, &c));
        car.input(&UserCommand {
            time: 1080,
            ..Default::default()
        });
        car.input(&UserCommand {
            time: 1100,
            buttons: 1,
            ..Default::default()
        });
        assert!(car.advance(1120, &c));
        let mut timed = super::tests::car(&c);
        assert!(!timed.advance(30999, &c));
        assert!(timed.advance(31000, &c));
    }
}
