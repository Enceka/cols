//! Captured offhand definitions, throw requests and server-owned grenade flight.
use crate::game::world::collision::Collision;
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Definition {
    pub name: String,
    #[serde(default)]
    pub marker_reward: Option<u8>,
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
    pub smoke: Option<crate::game::weapons::smoke::Definition>,
    #[serde(default)]
    pub tactical: Option<crate::game::status::tactical::Kind>,
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
    let c: Catalog = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/throwables-3.3.3.3.json"
    )))
    .expect("captured offhand definitions");
    assert_eq!(c.schema_version, 1);
    assert_eq!(c.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(c.capture_sha256.len(), 64);
    assert_eq!(c.double_ammo_offsets.len(), 60);
    c
});
impl Definition {
    pub(crate) fn marker(
        resources: &crate::game::rewards::killstreaks::RadarResources,
        id: u8,
    ) -> Option<Self> {
        let name = match id {
            2 => "airdrop_marker_mp",
            4 => "airdrop_sentry_marker_mp",
            9 => "airdrop_mega_marker_mp",
            22 => "airdrop_sam_marker_mp",
            _ => return None,
        };
        let w = resources.weapons.get(name)?;
        let m = w.marker.as_ref()?;
        if !m.valid()
            || w.weapon_type != 1
            || w.weapon_class != 6
            || w.radius != 0
            || resources.radio(id)?.weapon != w.weapon
            || w.projectile_speed == 0
            || w.projectile_speed > 50000
        {
            return None;
        }
        let d = Self {
            name: name.into(),
            marker_reward: Some(id),
            index: w.weapon,
            items: vec![],
            weapon_type: 1,
            weapon_class: 6,
            offhand_class: 0,
            clip_key: w.clip_key?,
            capacity: 1,
            prepare_msec: m.prepare_msec,
            throw_msec: m.throw_msec,
            throw_delay_msec: m.throw_delay_msec,
            fuse_msec: m.fuse_msec,
            cook: m.cook,
            explosion_kind: 0,
            smoke: None,
            tactical: None,
            gas: false,
            emp: false,
            radius: 0,
            inner_damage: 0,
            outer_damage: 0,
            speed: w.projectile_speed as u16,
            speed_up: m.speed_up,
            speed_forward: m.speed_forward,
            stickiness: m.stickiness,
            parallel: m.parallel,
            perpendicular: m.perpendicular,
        };
        d.validate().ok()?;
        Some(d)
    }
    pub fn capacity(&self) -> u16 {
        self.capacity
    }
    fn supported(&self) -> bool {
        if let Some(id) = self.marker_reward {
            return !self.cook
                && self.weapon_type == 1
                && self.weapon_class == 6
                && self.offhand_class == 0
                && self.capacity == 1
                && self.radius == 0
                && self.inner_damage == 0
                && self.outer_damage == 0
                && matches!(
                    (id, self.name.as_str()),
                    (2, "airdrop_marker_mp")
                        | (4, "airdrop_sentry_marker_mp")
                        | (9, "airdrop_mega_marker_mp")
                        | (22, "airdrop_sam_marker_mp")
                );
        }
        self.weapon_type == 1
            && self.weapon_class == 6
            && (((self.tactical == Some(crate::game::status::tactical::Kind::Flash)
                && self.explosion_kind == 2
                && self.offhand_class == 3
                && self.name == "flash_grenade_mp")
                || (self.tactical == Some(crate::game::status::tactical::Kind::Concussion)
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
                && (self.smoke.is_some()
                    || self.gas
                    || self.radius > 0
                    || self.marker_reward.is_some())
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
    let layout: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/passive-skills-3.3.3.3.json"
    )))?;
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
pub(crate) fn marker_resources() -> crate::game::rewards::killstreaks::RadarResources {
    use crate::game::rewards::killstreaks::MarkerFacts;
    use crate::game::rewards::killstreaks::Radio;
    let mut r = crate::game::rewards::deployables::test_resources();
    let original = r.weapons["sentry_minigun_mp"].clone();
    for (id, index, name) in [
        (2, 293, "airdrop_marker_mp"),
        (9, 294, "airdrop_mega_marker_mp"),
        (22, 295, "airdrop_sam_marker_mp"),
        (4, 296, "airdrop_sentry_marker_mp"),
    ] {
        let mut w = original.clone();
        w.weapon = index;
        w.weapon_type = 1;
        w.weapon_class = 6;
        w.clip_key = Some(index);
        w.ammo_key = Some(index);
        w.radius = 0;
        w.projectile_speed = 960;
        w.marker = Some(MarkerFacts {
            prepare_msec: 600,
            throw_msec: 360,
            throw_delay_msec: 129,
            fuse_msec: 1000,
            cook: false,
            speed_up: 120,
            speed_forward: 0,
            stickiness: 0,
            parallel: [0.5; 31],
            perpendicular: [0.25; 31],
        });
        r.radios.push(Radio {
            native_id: id,
            weapon: index,
            switch: w.switch,
            fire_interval_msec: 360,
        });
        r.weapons.insert(name.into(), w);
    }
    r
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
mod tests;
#[derive(Clone)]
struct Missile {
    snapshot: codol_protocol::game::snapshot::missile::MissileSnapshot,
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
        let expires = request.time.saturating_add(if def.marker_reward.is_some() {
            30_000
        } else {
            u32::from(request.fuse_msec)
        });
        self.missiles.push(Missile {
            snapshot: codol_protocol::game::snapshot::missile::MissileSnapshot {
                owner: 0,
                angles: [0; 3],
                entity_number,
                origin,
                weapon: def.index,
                present: true,
                generation: self.sequence,
            },
            velocity,
            definition: def,
            expires,
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
            if missile.definition.marker_reward.is_some() && missile.resting {
                missile.expires = missile.time;
            }
            if now >= missile.expires {
                if missile.definition.marker_reward.is_some() && !missile.resting {
                    missile.snapshot.present = false;
                    continue;
                }
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
    pub fn snapshots(&self) -> Vec<codol_protocol::game::snapshot::missile::MissileSnapshot> {
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
