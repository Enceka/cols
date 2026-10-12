//! Captured offhand definitions, throw requests and server-owned grenade flight.
use crate::game_motion::Collision;
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Definition {
    pub name: String,
    pub index: u16,
    items: Vec<u32>,
    #[serde(rename = "type")]
    weapon_type: u32,
    #[serde(rename = "class")]
    weapon_class: u32,
    offhand_class: u32,
    pub clip_key: u16,
    capacity: u16,
    pub prepare_msec: u16,
    pub throw_msec: u16,
    pub throw_delay_msec: u16,
    pub fuse_msec: u16,
    pub cook: bool,
    explosion_kind: u32,
    #[serde(default)]
    pub smoke: Option<crate::game_smoke::Definition>,
    #[serde(default)]
    pub tactical: Option<crate::game_tactical::Kind>,
    #[serde(default)]
    pub gas: bool,
    #[serde(default)]
    pub emp: bool,
    pub radius: u16,
    pub inner_damage: u16,
    pub outer_damage: u16,
    speed: u16,
    speed_up: u16,
    speed_forward: u16,
    stickiness: u32,
    parallel: [f32; 31],
    perpendicular: [f32; 31],
}
#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    client_sha256: String,
    capture_sha256: String,
    double_ammo_offsets: Vec<[[usize; 2]; 2]>,
    grenades: Vec<Definition>,
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let c: Catalog = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/throwables-3.3.3.3.json"
    ))
    .expect("captured offhand definitions");
    assert_eq!(c.schema_version, 1);
    assert_eq!(c.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(c.capture_sha256.len(), 64);
    assert_eq!(c.double_ammo_offsets.len(), 60);
    c
});
impl Definition {
    pub fn capacity(&self) -> u16 {
        self.capacity
    }
    fn supported(&self) -> bool {
        self.weapon_type == 1
            && self.weapon_class == 6
            && (((self.tactical == Some(crate::game_tactical::Kind::Flash)
                && self.explosion_kind == 2
                && self.offhand_class == 3
                && self.name == "flash_grenade_mp")
                || (self.tactical == Some(crate::game_tactical::Kind::Concussion)
                    && self.explosion_kind == 3
                    && self.offhand_class == 2
                    && self.name == "concussion_grenade_mp"))
                || (self.explosion_kind == 5
                    && self.offhand_class == 2
                    && self.name == "smoke_grenade_mp"
                    && self.smoke.as_ref().is_some_and(|s| s.valid()))
                || (self.gas
                    && self.name == "tabun_gas_mp"
                    && self.explosion_kind == 5
                    && self.offhand_class == 2
                    && self.smoke.is_none()
                    && self.tactical.is_none())
                || (self.emp
                    && self.name == "emp_grenade_mp"
                    && self.explosion_kind == 3
                    && self.offhand_class == 2
                    && self.smoke.is_none()
                    && self.tactical.is_none())
                || (self.explosion_kind == 0
                    && matches!(self.offhand_class, 1 | 5)
                    && self.smoke.is_none()
                    && matches!(
                        self.name.as_str(),
                        "frag_grenade_mp"
                            | "fragholiday_grenade_mp"
                            | "fragshort_grenade_mp"
                            | "fragtech_grenade_mp"
                            | "semtex_mp"
                            | "stielhandgranate_mp"
                    )))
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.supported()
                && self.clip_key == self.index
                && self.capacity > 0
                && self.capacity <= 511
                && (1..900).contains(&self.index)
                && self.throw_delay_msec <= self.throw_msec
                && self.throw_msec > 0
                && self.prepare_msec > 0
                && self.fuse_msec > 0
                && (self.smoke.is_some() || self.gas || self.radius > 0)
                && self.inner_damage >= self.outer_damage
                && self
                    .parallel
                    .iter()
                    .chain(&self.perpendicular)
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "invalid captured grenade definition"
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Equipped {
    pub definition: Definition,
    pub count: u16,
}
pub(crate) fn resolve(snapshot: &[u8], class: u8, owned: &[u32]) -> Result<[Option<Equipped>; 2]> {
    let layout: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/passive-skills-3.3.3.3.json"
    ))?;
    let mut result = [None, None];
    for (slot, output) in result.iter_mut().enumerate() {
        let offset = layout["class_offsets"][usize::from(class)][6 + slot]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("invalid grenade class layout"))?
            as usize;
        let raw = i32::from_le_bytes(
            snapshot
                .get(offset..offset + 4)
                .ok_or_else(|| anyhow::anyhow!("grenade profile bounds"))?
                .try_into()
                .unwrap(),
        );
        ensure!(raw >= 0, "negative grenade reference");
        let id = (raw as u32).saturating_sub(1);
        if id == 0 {
            continue;
        }
        ensure!(owned.contains(&id), "unowned offhand item {id}");
        let Some(def) = CATALOG.grenades.iter().find(|d| d.items.contains(&id)) else {
            tracing::warn!(item = id, "offhand equipment has no grenade definition");
            continue;
        };
        if !def.supported() {
            tracing::warn!(item=id, weapon=%def.name, "selected offhand effect is not available yet");
            continue;
        }
        ensure!(
            slot == usize::from(matches!(def.offhand_class, 2 | 3)),
            "offhand equipped in the wrong slot"
        );
        def.validate()?;
        let [offset, bit] = CATALOG.double_ammo_offsets[usize::from(class)][slot];
        let double = snapshot
            .get(offset)
            .ok_or_else(|| anyhow::anyhow!("grenade count flag bounds"))?
            & (1 << bit)
            != 0;
        *output = Some(Equipped {
            definition: def.clone(),
            count: if double { 2 } else { 1 },
        });
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub(crate) struct Throw {
    pub time: u32,
    pub angles: [u16; 3],
    pub definition: Definition,
    pub fuse_msec: u16,
    pub in_hand: bool,
    pub dropped: bool,
}
#[cfg(test)]
pub(crate) fn captured(name: &str) -> Definition {
    CATALOG
        .grenades
        .iter()
        .find(|d| d.name == name)
        .unwrap()
        .clone()
}
#[cfg(test)]
#[path = "game_grenade_tests.rs"]
mod tests;
#[derive(Clone)]
struct Missile {
    snapshot: codol_protocol::game_missile::MissileSnapshot,
    velocity: [f32; 3],
    definition: Definition,
    expires: u32,
    time: u32,
    resting: bool,
}
#[derive(Clone)]
pub(crate) struct Explosion {
    pub time: u32,
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub definition: Definition,
}
#[derive(Default)]
pub(crate) struct Grenades {
    missiles: Vec<Missile>,
    sequence: u32,
}
impl Grenades {
    pub fn clear(&mut self) {
        self.missiles.clear();
    }
    pub fn launch(
        &mut self,
        request: Throw,
        eye: [f32; 3],
        inherited: [f32; 3],
        collision: &Collision,
    ) -> bool {
        if self.missiles.len() >= 8 {
            return false;
        }
        let yaw = f32::from(request.angles[1]) * std::f32::consts::TAU / 65536.0;
        let pitch = f32::from(request.angles[0] as i16) * std::f32::consts::TAU / 65536.0;
        let forward = [
            pitch.cos() * yaw.cos(),
            pitch.cos() * yaw.sin(),
            -pitch.sin(),
        ];
        let desired: [f32; 3] = std::array::from_fn(|i| eye[i] + forward[i] * 8.0);
        let trace = collision.projectile_trace(eye, desired, 2.0);
        let origin = std::array::from_fn(|i| eye[i] + (desired[i] - eye[i]) * trace.fraction);
        let def = request.definition;
        let mut velocity =
            std::array::from_fn(|i| forward[i] * f32::from(def.speed) + inherited[i]);
        velocity[0] += yaw.cos() * f32::from(def.speed_forward);
        velocity[1] += yaw.sin() * f32::from(def.speed_forward);
        velocity[2] += f32::from(def.speed_up);
        if request.in_hand || request.dropped {
            velocity = [0.0; 3];
        }
        self.sequence = self.sequence.wrapping_add(1);
        let entity_number = (1700..1708)
            .find(|number| {
                self.missiles
                    .iter()
                    .all(|m| m.snapshot.entity_number != *number)
            })
            .unwrap();
        self.missiles.push(Missile {
            snapshot: codol_protocol::game_missile::MissileSnapshot {
                entity_number,
                origin,
                weapon: def.index,
                present: true,
                generation: self.sequence,
            },
            velocity,
            definition: def,
            expires: request.time + u32::from(request.fuse_msec),
            time: request.time,
            resting: request.in_hand,
        });
        true
    }
    pub fn advance(&mut self, now: u32, collision: &Collision) -> Vec<Explosion> {
        let mut explosions = Vec::new();
        for missile in &mut self.missiles {
            let through = now.min(missile.expires);
            while missile.time < through {
                let dt_ms = (through - missile.time).min(8);
                let dt = dt_ms as f32 * 0.001;
                missile.time += dt_ms;
                if missile.resting {
                    continue;
                }
                let start = missile.snapshot.origin;
                let mut end = std::array::from_fn(|i| start[i] + missile.velocity[i] * dt);
                end[2] -= 400.0 * dt * dt;
                let trace = collision.projectile_trace(start, end, 2.0);
                missile.velocity[2] -= 800.0 * dt;
                missile.snapshot.origin =
                    std::array::from_fn(|i| start[i] + (end[i] - start[i]) * trace.fraction);
                if trace.fraction < 1.0 {
                    let n = trace.normal;
                    let incoming = (0..3).map(|i| missile.velocity[i] * n[i]).sum::<f32>();
                    let surface = ((trace.surface_flags >> 20) & 31).min(30) as usize;
                    missile.velocity = std::array::from_fn(|i| {
                        (missile.velocity[i] - incoming * n[i])
                            * missile.definition.parallel[surface]
                            - incoming * n[i] * missile.definition.perpendicular[surface]
                    });
                    missile.snapshot.origin =
                        std::array::from_fn(|i| missile.snapshot.origin[i] + n[i] * 0.05);
                    if missile.definition.stickiness != 0
                        || trace.start_solid
                        || (n[2] > 0.7 && missile.velocity[2].abs() < 40.0)
                    {
                        missile.resting = true;
                        missile.velocity = [0.0; 3];
                    }
                }
            }
            if now >= missile.expires {
                explosions.push(Explosion {
                    time: missile.expires,
                    origin: missile.snapshot.origin,
                    normal: [0.0, 0.0, 1.0],
                    definition: missile.definition.clone(),
                });
                missile.snapshot.present = false;
            }
        }
        self.missiles.retain(|m| m.snapshot.present);
        explosions
    }
    pub fn snapshots(&self) -> Vec<codol_protocol::game_missile::MissileSnapshot> {
        let mut snapshots: Vec<_> = self.missiles.iter().map(|m| m.snapshot).collect();
        snapshots.sort_by_key(|m| m.entity_number);
        snapshots
    }
}

pub(crate) fn radius_damage(
    explosion: &Explosion,
    feet: [f32; 3],
    height: f32,
    collision: &Collision,
) -> u16 {
    let target = [feet[0], feet[1], feet[2] + height * 0.5];
    let distance = (0..3)
        .map(|i| (target[i] - explosion.origin[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    let def = &explosion.definition;
    if distance >= f32::from(def.radius) {
        return 0;
    }
    if collision.raycast(explosion.origin, target).fraction < 1.0 {
        return 0;
    }
    let fraction = distance / f32::from(def.radius);
    (f32::from(def.inner_damage)
        + fraction * (f32::from(def.outer_damage) - f32::from(def.inner_damage)))
    .ceil() as u16
}
