//! Target selection follows Alpha's range, five-second spawn protection and
//! body recognition. Gun cadence and damage stages use target weapon facts.
use super::*;
use crate::game::combat::{Actor, Ballistics, DamageFalloff};
use crate::game::rewards::deployables::DeviceShot;

#[derive(Clone, Copy)]
pub(crate) struct TargetActor {
    pub actor: Actor,
    pub life: u32,
    pub born: u32,
    pub score: i32,
    pub hidden: bool,
    pub airplane_hidden: bool,
}
#[derive(Clone, Copy)]
pub(super) struct Gun {
    pub weapon: u16,
    pub ballistics: Ballistics,
    pub(super) interval: u32,
    heavy: bool,
    airplane: bool,
}
impl Gun {
    pub fn from_resources(r: &RadarResources, id: u8) -> Option<Self> {
        let name = match id {
            7 => "harrier_20mm_mp",
            8 => "cobra_20mm_mp",
            10 => "pavelow_minigun_mp",
            _ => return None,
        };
        let w = r.weapons.get(name)?;
        if w.weapon_type != 0 || w.fire_interval_msec == 0 {
            return None;
        }
        let falloff = DamageFalloff::from_stages(w.damage_stages, w.damage_ranges)?;
        let range = w
            .damage_ranges
            .into_iter()
            .fold(0.0, f32::max)
            .min(if id == 7 { 8192.0 } else { 3500.0 });
        let ballistics = Ballistics {
            damage: falloff.damage(0.0),
            range,
            near_range: range,
            location_multipliers: None,
            falloff: Some(falloff),
            pellets: 1,
            spread_degrees: 0.0,
        };
        ballistics.valid().then_some(Self {
            weapon: w.weapon,
            ballistics,
            interval: if id == 10 {
                100
            } else {
                u32::from(w.fire_interval_msec)
            },
            heavy: id == 10,
            airplane: id == 7,
        })
    }
}
#[derive(Default)]
struct BarrelState {
    target: Option<(u8, u32)>,
    acquired: u32,
    next_fire: u32,
    rounds: u8,
    scan: u32,
    sequence: u32,
    last: Option<(u32, [u16; 3])>,
    events: codol_protocol::game::snapshot::events::EventQueue,
}
pub(crate) type AircraftShot = DeviceShot;
#[derive(Default)]
pub(super) struct GunState {
    barrels: [BarrelState; 2],
}
impl GunState {
    pub fn fire(
        &mut self,
        now: u32,
        f: &FireContext,
        actors: &[TargetActor],
        c: &Collision,
    ) -> Vec<AircraftShot> {
        (0..if f.gun.heavy { 2 } else { 1 })
            .flat_map(|barrel| {
                let shots = self.barrels[barrel].fire(now, f, actors, c, barrel);
                if let Some(shot) = shots.first() {
                    self.barrels[barrel].last = Some((now, shot.angles));
                    self.barrels[barrel]
                        .events
                        .push(codol_protocol::game::snapshot::events::FIRE_WEAPON, 0)
                        .expect("verified turret fire event");
                }
                shots
            })
            .collect()
    }
    pub fn firing(&self, barrel: usize, now: u32, active: bool, interval: u32) -> bool {
        active
            && self.barrels[barrel]
                .last
                .is_some_and(|(time, _)| now.saturating_sub(time) < interval)
    }
    pub fn events(&self, barrel: usize) -> codol_protocol::game::snapshot::events::EventQueue {
        self.barrels[barrel].events
    }
    pub fn presentation(
        &self,
        barrel: usize,
        now: u32,
        body: [u16; 3],
        active: bool,
        interval: u32,
    ) -> codol_protocol::game::snapshot::props::TurretPose {
        let last = self.barrels[barrel].last;
        codol_protocol::game::snapshot::props::TurretPose {
            gun: last.map_or([0; 3], |(_, angles)| local_aim(angles, body)),
            firing: self.firing(barrel, now, active, interval),
        }
    }
}

impl BarrelState {
    pub fn fire(
        &mut self,
        now: u32,
        f: &FireContext,
        actors: &[TargetActor],
        c: &Collision,
        barrel: usize,
    ) -> Vec<AircraftShot> {
        let yaw = f32::from(f.angles[1]) * std::f32::consts::TAU / 65536.0;
        let muzzle = if f.gun.heavy {
            heavy_pivot(f.origin, f.angles, barrel)
        } else {
            [
                f.origin[0] + yaw.cos() * 144.0,
                f.origin[1] + yaw.sin() * 144.0,
                f.origin[2] - 160.0,
            ]
        };
        if now >= self.scan {
            self.scan = now.saturating_add(500);
            let target = actors
                .iter()
                .filter(|a| eligible(a, now, f, muzzle, c))
                .max_by(|a, b| {
                    threat(a, muzzle, f.attacker).total_cmp(&threat(b, muzzle, f.attacker))
                });
            let identity = target.map(|a| (a.actor.client, a.life));
            if identity != self.target {
                self.target = identity;
                self.acquired = now;
                self.rounds = 0;
            }
        }
        if now < self.next_fire || now.saturating_sub(self.acquired) < 500 {
            return Vec::new();
        }
        let Some(target) = actors.iter().find(|a| {
            Some((a.actor.client, a.life)) == self.target && eligible(a, now, f, muzzle, c)
        }) else {
            self.target = None;
            return Vec::new();
        };
        let clip = if f.gun.heavy { 60 } else { 40 };
        if self.rounds >= clip {
            self.rounds = 0;
            self.next_fire = now.saturating_add(1500);
            return Vec::new();
        }
        let convergence = if f.gun.heavy {
            0.0
        } else {
            (1.0 - f32::from(self.rounds) * 0.05).max(0.0)
        };
        self.rounds += 1;
        self.sequence = self.sequence.wrapping_add(1);
        self.next_fire = now.saturating_add(f.gun.interval);
        let scatter = ((self.sequence.wrapping_mul(1664525) >> 8) % 13) as f32 - 6.0;
        let center = [
            target.actor.origin[0] + 96.0 * convergence + scatter,
            target.actor.origin[1] + scatter,
            target.actor.origin[2] + target.actor.height * 0.5,
        ];
        let d: [f32; 3] = std::array::from_fn(|i| center[i] - muzzle[i]);
        let angles = [
            short(-d[2].atan2(d[0].hypot(d[1])).to_degrees()),
            short(d[1].atan2(d[0]).to_degrees()),
            0,
        ];
        std::iter::once(barrel)
            .map(|barrel| {
                let eye = if f.gun.heavy {
                    heavy_muzzle(f.origin, f.angles, barrel, center)
                } else {
                    muzzle
                };
                AircraftShot {
                    life: f.source.life,
                    owner: f.source.owner,
                    team: f.source.team,
                    entity: f.entity,
                    eye,
                    angles: if f.gun.heavy {
                        let d: [f32; 3] = std::array::from_fn(|i| center[i] - eye[i]);
                        [
                            short(-d[2].atan2(d[0].hypot(d[1])).to_degrees()),
                            short(d[1].atan2(d[0]).to_degrees()),
                            0,
                        ]
                    } else {
                        angles
                    },
                    weapon: f.gun.weapon,
                    ballistics: f.gun.ballistics,
                }
            })
            .collect()
    }
}
// Target148 tag_gunner_left/right and model153 tag_aim/tag_flash bind
// positions, captured in aircraft-bindings-afghan-3.3.3.3.json. The model's
// attachment frame turns left/right by +/-90 degrees. Aim advances the flash
// from the pivot toward the selected target; parent remains the shot collider.
fn axes(angles: [u16; 3]) -> [[f32; 3]; 3] {
    let [pitch, yaw, roll] = angles.map(|v| f32::from(v) * std::f32::consts::TAU / 65536.0);
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let (sr, cr) = roll.sin_cos();
    let forward = [cp * cy, cp * sy, -sp];
    let left = [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, sr * cp];
    let up = [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp];
    [forward, left, up]
}
pub(super) fn heavy_attachment(
    origin: [f32; 3],
    angles: [u16; 3],
    barrel: usize,
) -> ([f32; 3], [u16; 3]) {
    let mount = if barrel == 0 {
        [105.149_92, 80.416_63, -17.796_448]
    } else {
        [102.984_245, -79.775_345, -20.136_963]
    };
    let basis = axes(angles);
    let point =
        std::array::from_fn(|i| origin[i] + (0..3).map(|j| basis[j][i] * mount[j]).sum::<f32>());
    let sign = if barrel == 0 { 1.0 } else { -1.0 };
    let forward = basis[1].map(|v| sign * v);
    let left = basis[0].map(|v| -sign * v);
    let up = basis[2];
    let body = [
        short(-forward[2].atan2(forward[0].hypot(forward[1])).to_degrees()),
        short(forward[1].atan2(forward[0]).to_degrees()),
        short(left[2].atan2(up[2]).to_degrees()),
    ];
    (point, body)
}
fn local_aim(world: [u16; 3], body: [u16; 3]) -> [u16; 3] {
    let direction = axes(world)[0];
    let local: [f32; 3] =
        axes(body).map(|axis| axis.iter().zip(direction).map(|(a, b)| a * b).sum());
    [
        short(-local[2].atan2(local[0].hypot(local[1])).to_degrees()),
        short(local[1].atan2(local[0]).to_degrees()),
        0,
    ]
}
fn heavy_pivot(origin: [f32; 3], angles: [u16; 3], barrel: usize) -> [f32; 3] {
    let mount = if barrel == 0 {
        [105.149_92, 80.416_63 - 1.791_409, -17.796_448 + 8.020_691]
    } else {
        [
            102.984_245,
            -79.775_345 + 1.791_409,
            -20.136_963 + 8.020_691,
        ]
    };
    let [forward, left, up] = axes(angles);
    std::array::from_fn(|i| {
        origin[i] + forward[i] * mount[0] + left[i] * mount[1] + up[i] * mount[2]
    })
}
fn heavy_muzzle(origin: [f32; 3], angles: [u16; 3], barrel: usize, target: [f32; 3]) -> [f32; 3] {
    let pivot = heavy_pivot(origin, angles, barrel);
    let d: [f32; 3] = std::array::from_fn(|i| target[i] - pivot[i]);
    let length = d.iter().map(|v| v * v).sum::<f32>().sqrt();
    // Distance from the weapon aim pivot to tag_flash along the barrel.
    let advance = 37.109_512_f32.min(length);
    std::array::from_fn(|i| pivot[i] + d[i] / length.max(f32::EPSILON) * advance)
}
pub(super) struct FireContext {
    pub source: super::Source,
    pub entity: u16,
    pub origin: [f32; 3],
    pub angles: [u16; 3],
    pub gun: Gun,
    pub attacker: Option<u8>,
}
fn threat(a: &TargetActor, eye: [f32; 3], attacker: Option<u8>) -> f32 {
    (3500.0 - distance(a.actor.origin, eye)) / 35.0
        + a.score as f32 * 4.0
        + if attacker == Some(a.actor.client) {
            100.0
        } else {
            0.0
        }
}
fn eligible(a: &TargetActor, now: u32, f: &FireContext, eye: [f32; 3], c: &Collision) -> bool {
    if a.actor.team == f.source.team
        || a.actor.client == f.source.owner
        || if f.gun.airplane {
            a.airplane_hidden
        } else {
            a.hidden
        }
        || now.saturating_sub(a.born) <= 5000
        || distance(eye, a.actor.origin) > f.gun.ballistics.range
    {
        return false;
    }
    if f.gun.airplane
        && (a.actor.origin[0] - f.origin[0]).hypot(a.actor.origin[1] - f.origin[1]) < 768.0
    {
        return false;
    }
    let visible = [0.2, 0.5, 0.85]
        .into_iter()
        .filter(|height| {
            let p = [
                a.actor.origin[0],
                a.actor.origin[1],
                a.actor.origin[2] + a.actor.height * height,
            ];
            let t = c.raycast_excluding_prop(eye, p, Some(f.entity));
            !t.start_solid && !t.all_solid && t.fraction >= 1.0
        })
        .count();
    visible >= if f.gun.airplane { 3 } else { 2 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heavy_flash_rays_follow_captured_mounts_and_body_rotation() {
        let binding: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/aircraft-bindings-afghan-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(binding["client_sha256"], codol_protocol::CLIENT_SHA256);
        let aim = &binding["models"]["weapon_minigun"]["attachment_bind_matrices"]["tag_aim"];
        for (barrel, name, sign) in [(0, "tag_gunner_left", 1.0), (1, "tag_gunner_right", -1.0)] {
            let mount = &binding["models"]["vehicle_pavelow"]["attachment_bind_matrices"][name];
            let pivot = [
                mount[4].as_f64().unwrap() as f32,
                mount[5].as_f64().unwrap() as f32 + sign * aim[4].as_f64().unwrap() as f32,
                1000.0 + mount[6].as_f64().unwrap() as f32 + aim[6].as_f64().unwrap() as f32,
            ];
            let target = [1000.0, 300.0, 40.0];
            let flash = heavy_muzzle([0.0, 0.0, 1000.0], [0; 3], barrel, target);
            assert!((distance(pivot, flash) - 37.109_512).abs() < 0.001);
            assert!((distance(pivot, target) - distance(flash, target) - 37.109_512).abs() < 0.001);
            let rotated = heavy_muzzle(
                [0.0, 0.0, 1000.0],
                [0, 16384, 0],
                barrel,
                [-300.0, 1000.0, 40.0],
            );
            assert!((rotated[0] + flash[1]).abs() < 0.001);
            assert!((rotated[1] - flash[0]).abs() < 0.001);
            assert!((rotated[2] - flash[2]).abs() < 0.001);
        }
    }
    fn collision(wall: bool) -> Collision {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut brushes = vec![
            serde_json::json!({"center":[0,0,-10],"half":[20000,20000,10],"contents":1,"planes":[]}),
        ];
        if wall {
            brushes.push(serde_json::json!({"center":[500,0,1000],"half":[10,2000,1000],"contents":1,"planes":[]}));
        }
        std::fs::write(file.path(),serde_json::to_vec(&serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]})).unwrap()).unwrap();
        Collision::load(file.path()).unwrap()
    }
    fn target() -> TargetActor {
        TargetActor {
            actor: Actor {
                client: 1,
                team: 1,
                origin: [1000.0, 0.0, 0.0],
                height: 80.0,
                yaw: 0.0,
            },
            life: 1,
            born: 1000,
            score: 100,
            hidden: false,
            airplane_hidden: false,
        }
    }
    fn context(heavy: bool) -> FireContext {
        FireContext {
            source: Source {
                owner: 0,
                team: 2,
                life: 3,
                serial: 1,
            },
            entity: 560,
            origin: [0.0, 0.0, 1000.0],
            angles: [0; 3],
            gun: Gun {
                weapon: 302,
                ballistics: Ballistics {
                    damage: 30,
                    range: 3500.0,
                    near_range: 3500.0,
                    location_multipliers: None,
                    falloff: None,
                    pellets: 1,
                    spread_degrees: 0.0,
                },
                interval: 50,
                heavy,
                airplane: false,
            },
            attacker: None,
        }
    }
    #[test]
    fn heavy_barrels_acquire_different_enemies_and_reacquire_lives_separately() {
        let c = collision(false);
        let ctx = context(true);
        let mut left = target();
        left.actor.origin[1] = 500.0;
        let mut right = target();
        right.actor.client = 2;
        right.actor.origin[1] = -500.0;
        let mut state = GunState::default();
        assert!(state.fire(7000, &ctx, &[left, right], &c).is_empty());
        assert_eq!(state.barrels[0].target, Some((1, 1)));
        assert_eq!(state.barrels[1].target, Some((2, 1)));
        let shots = state.fire(7500, &ctx, &[left, right], &c);
        assert_eq!(shots.len(), 2);
        assert!(shots.iter().all(|s| s.entity == 560 && s.life == 3));
        left.life += 1;
        let shots = state.fire(8000, &ctx, &[left, right], &c);
        assert_eq!(shots.len(), 1);
        assert!(shots[0].eye[1] < 0.0);
        assert_eq!(state.events(0).sequence, 1);
        assert_eq!(state.events(1).sequence, 2);
        assert_eq!(state.fire(8500, &ctx, &[left, right], &c).len(), 2);
        assert!(!state.presentation(0, 8600, [0; 3], false, 100).firing);
    }
    #[test]
    fn protection_occlusion_team_and_server_only_stealth_all_block_acquisition() {
        let c = collision(false);
        let ctx = context(false);
        let t = target();
        let mut state = GunState::default();
        for time in [2000, 5999, 6000] {
            assert!(state.fire(time, &ctx, &[t], &c).is_empty());
        }
        for variant in [
            TargetActor { hidden: true, ..t },
            TargetActor {
                actor: Actor { team: 2, ..t.actor },
                ..t
            },
            TargetActor {
                actor: Actor {
                    client: 0,
                    ..t.actor
                },
                ..t
            },
        ] {
            let mut state = GunState::default();
            assert!(state.fire(7000, &ctx, &[variant], &c).is_empty());
            assert!(state.fire(8000, &ctx, &[variant], &c).is_empty());
        }
        let mut state = GunState::default();
        assert!(state.fire(7000, &ctx, &[t], &collision(true)).is_empty());
        assert!(state.fire(8000, &ctx, &[t], &collision(true)).is_empty());
    }
    #[test]
    fn acquired_burst_converges_and_uses_real_collision_without_harming_allies() {
        let c = collision(false);
        let ctx = context(false);
        let t = target();
        let mut state = GunState::default();
        assert!(state.fire(7000, &ctx, &[t], &c).is_empty());
        let mut damage = 0;
        let mut shot_count = 0;
        for now in (7500..9500).step_by(50) {
            for s in state.fire(now, &ctx, &[t], &c) {
                shot_count += 1;
                assert_eq!(s.life, 3);
                let outcome =
                    crate::game::rewards::deployables::resolve_shot(&s, now, &[t.actor], &c);
                damage += outcome
                    .damage
                    .iter()
                    .map(|(_, d, _)| u32::from(*d))
                    .sum::<u32>();
            }
        }
        assert_eq!(shot_count, 40);
        assert!(damage > 100);
        assert!(state.fire(9500, &ctx, &[t], &c).is_empty());
        assert!(state.fire(10500, &ctx, &[t], &c).is_empty());
        assert_eq!(state.fire(11000, &ctx, &[t], &c).len(), 1);
    }
    #[test]
    fn stale_target_life_requires_new_recognition_and_heavy_barrels_aim_independently() {
        let c = collision(false);
        let ctx = context(true);
        let t = target();
        let mut state = GunState::default();
        state.fire(7000, &ctx, &[t], &c);
        let shots = state.fire(7500, &ctx, &[t], &c);
        assert_eq!(shots.len(), 2);
        assert_ne!(shots[0].angles, shots[1].angles);
        for s in shots {
            assert!(
                !crate::game::rewards::deployables::resolve_shot(&s, 7500, &[t.actor], &c)
                    .damage
                    .is_empty()
            );
        }
        let t = TargetActor {
            life: 2,
            born: 7600,
            ..t
        };
        assert!(state.fire(8000, &ctx, &[t], &c).is_empty());
        assert!(state.fire(12601, &ctx, &[t], &c).is_empty());
        assert!(!state.fire(13101, &ctx, &[t], &c).is_empty());
    }
}
