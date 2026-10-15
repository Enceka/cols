//! Authoritative death guns, bounded flight, compatible ammo and use swaps.
use super::collision::Collision;
use crate::game::weapons::held::{DroppedWeapon, HeldWeapons};
use codol_protocol::game::snapshot::weapon_drops::{
    END_ENTITY, FIRST_ENTITY, WeaponDropSnapshot, WeaponHint,
};

const LIFETIME: u32 = 60_000;
const MAX_DROPS: usize = 16;

struct GroundWeapon {
    born: u32,
    updated: u32,
    owner: u8,
    snapshot: WeaponDropSnapshot,
    weapon: DroppedWeapon,
    velocity: [f32; 3],
    grounded: bool,
}
#[derive(Default)]
pub(crate) struct WeaponDrops {
    active: Vec<GroundWeapon>,
    generations: std::collections::BTreeMap<u16, u32>,
    use_held: bool,
}
#[derive(Clone, Copy)]
pub(crate) struct User {
    pub actor: u8,
    pub feet: [f32; 3],
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    pub active: bool,
    pub use_button: bool,
}
impl WeaponDrops {
    pub fn clear(&mut self) {
        self.active.clear();
        self.use_held = false;
    }
    pub fn spawn(
        &mut self,
        now: u32,
        owner: u8,
        origin: [f32; 3],
        angles: [u16; 3],
        weapon: DroppedWeapon,
        observers: &[[f32; 3]],
    ) {
        self.active
            .retain(|w| now.saturating_sub(w.born) < LIFETIME);
        if self.active.len() >= MAX_DROPS {
            let farthest = self
                .active
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    let nearest = |w: &GroundWeapon| {
                        observers
                            .iter()
                            .map(|p| distance(*p, w.snapshot.origin))
                            .fold(f32::INFINITY, f32::min)
                    };
                    nearest(a).total_cmp(&nearest(b))
                })
                .map_or(0, |(i, _)| i);
            self.active.remove(farthest);
        }
        let Some(number) = (FIRST_ENTITY..END_ENTITY)
            .find(|n| self.active.iter().all(|w| w.snapshot.entity_number != *n))
        else {
            return;
        };
        let generation = self.generations.entry(number).or_default();
        *generation = generation.wrapping_add(1);
        let yaw = f32::from(angles[1]) * std::f32::consts::TAU / 65536.0;
        // The reference starts at the weapon tag; exported server poses have
        // no live bone matrices, so use a body-relative launch point.
        let start = [origin[0], origin[1], origin[2] + 32.0];
        self.active.push(GroundWeapon {
            born: now,
            updated: now,
            owner,
            weapon: weapon.clone(),
            snapshot: WeaponDropSnapshot {
                entity_number: number,
                origin: start,
                angles: [0, angles[1], 16384],
                weapon: weapon.index(),
                owner: u16::from(owner),
                generation: *generation,
            },
            velocity: [yaw.cos() * 10.0, yaw.sin() * 10.0, 10.0],
            grounded: false,
        });
        tracing::info!(time=now, entity=number, weapon=weapon.index(), owner, ammo=?weapon.ammo(), "death weapon dropped");
    }
    pub fn advance(&mut self, now: u32, collision: &Collision) {
        self.active.retain_mut(|w| {
            if now.saturating_sub(w.born) >= LIFETIME {
                return false;
            }
            if now.saturating_sub(w.born) >= 1000 {
                w.snapshot.owner = 2047;
            }
            while w.updated < now {
                let dt = (now - w.updated).min(25);
                w.updated += dt;
                if w.grounded {
                    let end = [
                        w.snapshot.origin[0],
                        w.snapshot.origin[1],
                        w.snapshot.origin[2] - 2.0,
                    ];
                    if collision
                        .projectile_trace(w.snapshot.origin, end, 1.0)
                        .fraction
                        < 1.0
                    {
                        continue;
                    }
                    w.grounded = false;
                }
                let secs = dt as f32 / 1000.0;
                let start = w.snapshot.origin;
                let end = std::array::from_fn(|i| {
                    start[i] + w.velocity[i] * secs - if i == 2 { 400.0 * secs * secs } else { 0.0 }
                });
                let hit = collision.projectile_trace(start, end, 1.0);
                if hit.start_solid {
                    return false;
                }
                w.snapshot.origin =
                    std::array::from_fn(|i| start[i] + (end[i] - start[i]) * hit.fraction);
                w.velocity[2] -= 800.0 * secs;
                if hit.fraction < 1.0 {
                    for i in 0..3 {
                        w.snapshot.origin[i] += hit.normal[i] * 0.1;
                    }
                    if hit.normal[2] > 0.5 {
                        w.velocity = [0.0; 3];
                        w.grounded = true;
                    } else {
                        let dot: f32 = (0..3).map(|i| w.velocity[i] * hit.normal[i]).sum();
                        for i in 0..3 {
                            w.velocity[i] -= dot * hit.normal[i];
                        }
                    }
                }
            }
            w.snapshot
                .origin
                .iter()
                .all(|v| v.is_finite() && (-32760.0..=32760.0).contains(v))
        });
    }
    pub fn snapshots(&self) -> Vec<WeaponDropSnapshot> {
        let mut states: Vec<_> = self
            .active
            .iter()
            .map(|w| WeaponDropSnapshot {
                origin: w.snapshot.origin.map(f32::round_ties_even),
                ..w.snapshot
            })
            .collect();
        states.sort_by_key(|w| w.entity_number);
        states
    }
    pub fn interact(
        &mut self,
        now: u32,
        user: User,
        held: &mut HeldWeapons,
        collision: &Collision,
    ) -> bool {
        let pressed = user.use_button && !self.use_held;
        self.use_held = user.use_button;
        if !user.active || !held.can_use_ground_weapon() {
            return false;
        }
        if pressed && let Some(index) = self.target(now, user, held, collision) {
            let old = held
                .swap_ground_weapon(self.active[index].weapon.clone())
                .expect("eligible swap");
            let w = &mut self.active[index];
            let new_index = held.equipment().weapon;
            tracing::info!(time=now, entity=w.snapshot.entity_number, picked=new_index, dropped=old.index(), ammo=?held.equipment().ammo, "ground weapon exchanged");
            if old.empty() {
                self.active.remove(index);
            } else {
                w.weapon = old;
                w.born = now;
                w.owner = user.actor;
                w.snapshot.weapon = w.weapon.index();
                w.snapshot.owner = u16::from(user.actor);
                w.snapshot.generation = w.snapshot.generation.wrapping_add(1);
                self.generations
                    .insert(w.snapshot.entity_number, w.snapshot.generation);
            }
            return true;
        }
        self.active.retain_mut(|w| {
            let p = w.snapshot.origin.map(f32::round_ties_even);
            let touching = (0..3).all(|i| {
                let delta = user.feet[i] - p[i];
                if i == 2 { (-88.0..=18.0).contains(&delta) } else { (-36.0..=36.0).contains(&delta) }
            });
            if touching && (w.owner != user.actor || now.saturating_sub(w.born) >= 1000)
                && held.leech_ground_ammo(&mut w.weapon)
            { tracing::info!(time=now, entity=w.snapshot.entity_number, ammo=?held.equipment().ammo, "ground weapon ammunition collected"); }
            !w.weapon.empty()
        });
        false
    }
    pub fn hint(
        &self,
        now: u32,
        user: User,
        held: &HeldWeapons,
        collision: &Collision,
    ) -> Option<WeaponHint> {
        self.target(now, user, held, collision).map(|i| WeaponHint {
            weapon: self.active[i].weapon.index(),
            entity: self.active[i].snapshot.entity_number,
        })
    }
    fn target(
        &self,
        now: u32,
        user: User,
        held: &HeldWeapons,
        collision: &Collision,
    ) -> Option<usize> {
        if !user.active {
            return None;
        }
        // Explicit server use policy until the production scan is recovered:
        // 96 units from the feet, in front of the player and unobstructed.
        self.active
            .iter()
            .enumerate()
            .filter(|(_, w)| {
                let p = w.snapshot.origin.map(f32::round_ties_even);
                let delta: [f32; 3] = std::array::from_fn(|i| p[i] - user.feet[i]);
                let facing = delta[0] * user.forward[0] + delta[1] * user.forward[1];
                held.can_swap_ground_weapon(&w.weapon)
                    && (w.owner != user.actor || now.saturating_sub(w.born) >= 1000)
                    && distance(user.feet, p) <= 96.0 * 96.0
                    && facing >= -8.0
                    && collision.raycast(user.eye, p).fraction >= 0.99
            })
            .min_by(|(_, a), (_, b)| {
                distance(user.feet, a.snapshot.origin)
                    .total_cmp(&distance(user.feet, b.snapshot.origin))
            })
            .map(|(i, _)| i)
    }
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::weapons::held::drop_test_gun;
    fn floor() -> Collision {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(),serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"captured_contents_mask":super::super::collision::PLAYER_MASK|super::super::collision::SHOT_MASK,
            "brushes":[{"center":[0,0,-20],"half":[2000,2000,20],"contents":1,"planes":[]}],"triangles":[]
        })).unwrap()).unwrap();
        Collision::load(file.path()).unwrap()
    }
    fn user() -> User {
        User {
            actor: 0,
            feet: [0.0; 3],
            eye: [0.0, 0.0, 60.0],
            forward: [1.0, 0.0, 0.0],
            active: true,
            use_button: false,
        }
    }
    #[test]
    fn physical_drop_lands_expires_recycles_and_limits_farthest_objects() {
        let mut drops = WeaponDrops::default();
        let c = floor();
        let item = drop_test_gun(353, 164, 7, 72).take_death_weapon().unwrap();
        drops.spawn(1000, 1, [24.0, 0.0, 0.0], [0; 3], item.clone(), &[[0.0; 3]]);
        drops.advance(2000, &c);
        let landed = drops.snapshots()[0];
        assert!((1.0..=2.0).contains(&landed.origin[2]));
        assert_eq!(landed.owner, 2047);
        for i in 1..=MAX_DROPS {
            drops.spawn(
                2000 + i as u32,
                1,
                [100.0 * i as f32, 0.0, 0.0],
                [0; 3],
                item.clone(),
                &[[0.0; 3]],
            );
        }
        assert_eq!(drops.active.len(), MAX_DROPS);
        assert!(
            drops
                .active
                .iter()
                .any(|w| w.snapshot.entity_number == landed.entity_number)
        );
        drops.advance(63000, &c);
        assert!(drops.snapshots().is_empty());
        drops.spawn(64000, 1, [0.0; 3], [0; 3], item, &[]);
        assert!(drops.snapshots()[0].generation > landed.generation);
    }
    #[test]
    fn native_hint_and_swap_require_life_visibility_range_and_a_new_use_edge() {
        let mut drops = WeaponDrops::default();
        let c = floor();
        let mut p = drop_test_gun(352, 163, 7, 72);
        drops.spawn(
            1000,
            1,
            [24.0, 0.0, 0.0],
            [0; 3],
            drop_test_gun(353, 164, 3, 22).take_death_weapon().unwrap(),
            &[],
        );
        drops.advance(2500, &c);
        assert_eq!(drops.hint(2500, user(), &p, &c).unwrap().weapon, 353);
        assert!(
            drops
                .hint(
                    2500,
                    User {
                        active: false,
                        ..user()
                    },
                    &p,
                    &c
                )
                .is_none()
        );
        assert!(
            drops
                .hint(
                    2500,
                    User {
                        feet: [300.0, 0.0, 0.0],
                        ..user()
                    },
                    &p,
                    &c
                )
                .is_none()
        );
        assert!(!drops.interact(
            2500,
            User {
                active: false,
                use_button: true,
                ..user()
            },
            &mut p,
            &c
        ));
        assert!(!drops.interact(
            2550,
            User {
                use_button: true,
                ..user()
            },
            &mut p,
            &c
        ));
        drops.interact(2600, user(), &mut p, &c);
        assert!(drops.interact(
            2650,
            User {
                use_button: true,
                ..user()
            },
            &mut p,
            &c
        ));
        assert_eq!(p.equipment().weapon, 353);
        assert_eq!(p.equipment().ammo.unwrap().clip, 3);
        assert_eq!(drops.snapshots()[0].weapon, 352);
        assert!(!drops.interact(
            2700,
            User {
                use_button: true,
                ..user()
            },
            &mut p,
            &c
        ));
    }
    #[test]
    fn former_owner_is_excluded_for_one_second_and_ammo_cannot_be_collected_while_dead() {
        let mut drops = WeaponDrops::default();
        let c = floor();
        let mut p = drop_test_gun(352, 163, 7, 0);
        drops.spawn(
            1000,
            0,
            [0.0; 3],
            [0; 3],
            drop_test_gun(353, 163, 3, 22).take_death_weapon().unwrap(),
            &[],
        );
        drops.advance(1500, &c);
        assert!(drops.hint(1500, user(), &p, &c).is_none());
        drops.interact(1500, user(), &mut p, &c);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 0);
        drops.advance(2050, &c);
        drops.interact(
            2050,
            User {
                active: false,
                ..user()
            },
            &mut p,
            &c,
        );
        assert_eq!(p.equipment().ammo.unwrap().reserve, 0);
        drops.interact(2100, user(), &mut p, &c);
        assert_eq!(p.equipment().ammo.unwrap().reserve, 25);
        assert!(drops.snapshots().is_empty());
    }
}
