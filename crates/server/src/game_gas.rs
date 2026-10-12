//! Captured Nova center presentation and reference-script poison exposure.
//! Area touch uses current server body bounds, not unrecovered native triggers.
use crate::{game_grenade::Explosion, game_motion::Collision, game_tactical};
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    client_sha256: String,
    capture_sha256: String,
    native_effect: Effect,
    server_rules: Rules,
}
#[derive(Deserialize)]
pub(crate) struct Effect {
    pub weapon: u16,
    pub lifetime_msec: u32,
    name: String,
    effect: String,
    definition_sha256: String,
    elements_sha256: String,
}
#[derive(Deserialize)]
struct Rules {
    source: String,
    sha256: String,
    cloud_msec: u32,
    tick_msec: u32,
    clear_msec: u32,
    damage: u16,
    radius: f32,
    height: f32,
    spoke_length: f32,
    radii: [f32; 4],
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let c: Catalog = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/gas-status-3.3.3.3.json"
    ))
    .expect("captured gas binding and reference rules");
    assert_eq!(c.schema_version, 1);
    assert_eq!(c.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(c.capture_sha256.len(), 64);
    assert_eq!(c.native_effect.weapon, 250);
    assert_eq!(c.native_effect.name, "tabun_center_mp");
    assert_eq!(c.native_effect.effect, "weapon/grenade/fx_gas_nova_mp");
    assert_eq!(c.native_effect.lifetime_msec, 10000);
    assert_eq!(c.native_effect.definition_sha256.len(), 64);
    assert_eq!(c.native_effect.elements_sha256.len(), 64);
    assert!(c.server_rules.source.contains("reference script port"));
    assert_eq!(c.server_rules.sha256.len(), 64);
    assert_eq!(c.server_rules.cloud_msec, 8000);
    assert_eq!(c.server_rules.tick_msec, 1000);
    assert_eq!(c.server_rules.clear_msec, 1000);
    assert_eq!(c.server_rules.damage, 13);
    c
});

pub(crate) fn effect() -> &'static Effect {
    &CATALOG.native_effect
}

#[derive(Clone, Copy)]
struct Area {
    origin: [f32; 3],
    radius: f32,
    height: f32,
}
struct Cloud {
    owner: u8,
    team: u8,
    weapon: u16,
    start: u32,
    end: u32,
    areas: Vec<Area>,
}
#[derive(Default)]
pub(crate) struct Gas {
    clouds: Vec<Cloud>,
}
#[derive(Clone, Copy)]
pub(crate) struct Recipient {
    pub client: u8,
    pub team: u8,
    pub origin: [f32; 3],
    pub height: f32,
    pub alive: bool,
    pub protected: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub attacker: u8,
    pub weapon: u16,
    pub damage: u16,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Poison {
    next_damage: Option<u32>,
    clear_at: Option<u32>,
    no_kick: bool,
}
impl Poison {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn advance(
        &mut self,
        now: u32,
        gas: &Gas,
        target: Recipient,
        collision: &Collision,
        shock: &mut game_tactical::State,
    ) -> Option<Hit> {
        if !target.alive || target.protected {
            self.clear();
            shock.clear_gas();
            return None;
        }
        let Some(hit) = gas.exposure(now, target, collision) else {
            if self.next_damage.is_some() {
                let clear = *self
                    .clear_at
                    .get_or_insert(now.saturating_add(CATALOG.server_rules.clear_msec));
                if now >= clear {
                    self.clear();
                    // Do not erase a flash/concussion that replaced gas in the
                    // client's single Named screen-effect slot after leaving.
                    shock.clear_gas();
                }
            }
            return None;
        };
        self.clear_at = None;
        if self.next_damage.is_some_and(|next| now < next) {
            return None;
        }
        self.next_damage = Some(now.saturating_add(CATALOG.server_rules.tick_msec));
        shock.apply(
            now,
            game_tactical::Hit {
                kind: if self.no_kick {
                    game_tactical::Kind::GasNoKick
                } else {
                    game_tactical::Kind::Gas
                },
                duration_msec: CATALOG.server_rules.tick_msec as u16,
            },
        );
        self.no_kick = !self.no_kick;
        Some(hit)
    }
}
impl Gas {
    pub fn clear(&mut self) {
        self.clouds.clear();
    }
    pub fn expire(&mut self, now: u32) {
        self.clouds.retain(|c| now < c.end);
    }
    pub fn deploy(
        &mut self,
        explosion: &Explosion,
        owner: u8,
        team: u8,
        collision: &Collision,
        persistent: &mut crate::game_smoke::Smoke,
    ) -> bool {
        if !explosion.definition.gas {
            return false;
        }
        self.expire(explosion.time);
        let effect = effect();
        if !persistent.spawn_effect(
            explosion.time,
            explosion.origin,
            effect.weapon,
            effect.lifetime_msec,
            None,
        ) {
            return false;
        }
        self.clouds.push(Cloud {
            owner,
            team,
            weapon: explosion.definition.index,
            start: explosion.time,
            end: explosion
                .time
                .saturating_add(CATALOG.server_rules.cloud_msec),
            areas: areas(explosion.origin, collision),
        });
        true
    }
    fn exposure(&self, now: u32, target: Recipient, collision: &Collision) -> Option<Hit> {
        if !target.alive || target.protected {
            return None;
        }
        self.clouds.iter().find_map(|c| {
            if now < c.start || now >= c.end || (target.client != c.owner && target.team == c.team)
            {
                return None;
            }
            c.areas.iter().find_map(|a| {
                // Server capsule radius15 and posture height approximate the
                // reference trigger's isTouching test. LOS is still required.
                let distance = (target.origin[0] - a.origin[0]).powi(2)
                    + (target.origin[1] - a.origin[1]).powi(2);
                if distance >= (a.radius + 15.0).powi(2)
                    || target.origin[2] >= a.origin[2] + a.height
                    || target.origin[2] + target.height <= a.origin[2]
                {
                    return None;
                }
                let trace = collision.raycast(
                    a.origin,
                    [target.origin[0], target.origin[1], target.origin[2] + 12.0],
                );
                (!trace.start_solid && !trace.all_solid && trace.fraction == 1.0).then_some(Hit {
                    attacker: c.owner,
                    weapon: c.weapon,
                    damage: CATALOG.server_rules.damage,
                })
            })
        })
    }
}
fn areas(origin: [f32; 3], collision: &Collision) -> Vec<Area> {
    let rules = &CATALOG.server_rules;
    let start = [origin[0], origin[1], origin[2] + 12.0];
    let dirs = [
        [1.0, 0.0],
        [0.707, 0.707],
        [0.0, 1.0],
        [-0.707, 0.707],
        [-1.0, 0.0],
        [-0.707, -0.707],
        [0.0, -1.0],
        [0.707, -0.707],
    ];
    let mut points = [[0.0; 3]; 8];
    let mut spans = [0.0; 8];
    for (i, dir) in dirs.iter().enumerate() {
        let end = [
            start[0] + dir[0] * rules.spoke_length,
            start[1] + dir[1] * rules.spoke_length,
            start[2],
        ];
        let trace = collision.raycast(start, end);
        let stop: [f32; 3] =
            std::array::from_fn(|j| start[j] + (end[j] - start[j]) * trace.fraction);
        spans[i] = (0..3).map(|j| (stop[j] - start[j]).powi(2)).sum();
        points[i] = std::array::from_fn(|j| (start[j] + stop[j]) * 0.5);
        points[i][2] -= 12.0;
    }
    let sizes: [Option<usize>; 8] = std::array::from_fn(|i| {
        let (span, prev, next) = (spans[i], spans[(i + 7) % 8], spans[(i + 1) % 8]);
        for k in 0..3 {
            if span > rules.radii[k].powi(2)
                && prev > rules.radii[k + 1].powi(2)
                && next > rules.radii[k + 1].powi(2)
            {
                return Some(k);
            }
        }
        // Retain the reference script's unsquared adjacent-radius comparison.
        (span > rules.radii[3].powi(2) && prev > rules.radii[3] && next > rules.radii[3])
            .then_some(3)
    });
    if sizes.iter().all(|s| matches!(s, Some(0) | Some(1)))
        && sizes.iter().filter(|&&s| s == Some(1)).count() <= 1
    {
        return vec![Area {
            origin,
            radius: rules.radius,
            height: rules.height,
        }];
    }
    sizes
        .iter()
        .enumerate()
        .filter_map(|(i, size)| {
            size.map(|k| Area {
                origin: points[i],
                radius: rules.radii[k],
                height: rules.radii[k],
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world(wall: bool) -> Collision {
        let mut brushes = vec![
            serde_json::json!({"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}),
        ];
        if wall {
            brushes.push(serde_json::json!({"center":[64,0,100],"half":[2,1000,100],"contents":1,"planes":[]}));
        }
        let data = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&data).unwrap()).unwrap();
        Collision::load(file.path()).unwrap()
    }
    fn target() -> Recipient {
        Recipient {
            client: 0,
            team: 1,
            origin: [0.0; 3],
            height: 60.0,
            alive: true,
            protected: false,
        }
    }
    fn deploy(
        gas: &mut Gas,
        persistent: &mut crate::game_smoke::Smoke,
        world: &Collision,
        time: u32,
    ) {
        assert!(gas.deploy(
            &Explosion {
                time,
                origin: [0.0, 0.0, 2.0],
                normal: [0.0, 0.0, 1.0],
                definition: crate::game_grenade::captured("tabun_gas_mp")
            },
            0,
            1,
            world,
            persistent
        ));
    }
    #[test]
    fn mask_team_self_height_wall_and_cloud_expiry_have_independent_semantics() {
        let collision = world(false);
        let mut gas = Gas::default();
        let mut persistent = crate::game_smoke::Smoke::default();
        deploy(&mut gas, &mut persistent, &collision, 1000);
        let t = target();
        assert_eq!(gas.exposure(1000, t, &collision).unwrap().damage, 13);
        for t in [
            Recipient {
                protected: true,
                ..t
            },
            Recipient { client: 1, ..t },
            Recipient {
                origin: [0.0, 0.0, 30.0],
                ..t
            },
            Recipient {
                origin: [250.0, 0.0, 0.0],
                ..t
            },
        ] {
            assert!(gas.exposure(1000, t, &collision).is_none());
        }
        assert!(
            gas.exposure(
                1000,
                Recipient {
                    client: 2,
                    team: 2,
                    ..t
                },
                &collision
            )
            .is_some()
        );
        assert!(gas.exposure(999, t, &collision).is_none());
        assert!(gas.exposure(8999, t, &collision).is_some());
        assert!(gas.exposure(9000, t, &collision).is_none());
        assert_eq!(persistent.snapshots(9000)[0].weapon, 250);
        assert!(persistent.visible(5000, [-100.0, 0.0, 12.0], [100.0, 0.0, 12.0]));
        let collision = world(true);
        let mut gas = Gas::default();
        deploy(&mut gas, &mut persistent, &collision, 1000);
        assert!(gas.exposure(1000, t, &collision).is_some());
        assert!(
            gas.exposure(
                1000,
                Recipient {
                    origin: [128.0, 0.0, 0.0],
                    ..t
                },
                &collision
            )
            .is_none()
        );
    }
    #[test]
    fn poison_ticks_once_without_stacking_alternates_and_clears_without_erasing_new_flash() {
        let collision = world(false);
        let mut gas = Gas::default();
        let mut persistent = crate::game_smoke::Smoke::default();
        deploy(&mut gas, &mut persistent, &collision, 1000);
        deploy(&mut gas, &mut persistent, &collision, 1000);
        let mut poison = Poison::default();
        let mut shock = game_tactical::State::default();
        let t = target();
        assert!(
            poison
                .advance(1000, &gas, t, &collision, &mut shock)
                .is_some()
        );
        assert_eq!(shock.snapshot(1000).unwrap().index, 7);
        assert!(
            poison
                .advance(1000, &gas, t, &collision, &mut shock)
                .is_none()
        );
        assert!(
            poison
                .advance(1999, &gas, t, &collision, &mut shock)
                .is_none()
        );
        assert!(
            poison
                .advance(2000, &gas, t, &collision, &mut shock)
                .is_some()
        );
        assert_eq!(shock.snapshot(2000).unwrap().index, 8);
        let away = Recipient {
            origin: [1000.0, 0.0, 0.0],
            ..t
        };
        assert!(
            poison
                .advance(2100, &gas, away, &collision, &mut shock)
                .is_none()
        );
        shock.apply(
            2200,
            game_tactical::Hit {
                kind: game_tactical::Kind::Flash,
                duration_msec: 4000,
            },
        );
        poison.advance(3100, &gas, away, &collision, &mut shock);
        assert!(shock.blind(3100));
        assert!(
            poison
                .advance(3200, &gas, t, &collision, &mut shock)
                .is_some()
        );
        assert_eq!(shock.snapshot(3200).unwrap().index, 7);
        poison.advance(
            3300,
            &gas,
            Recipient { alive: false, ..t },
            &collision,
            &mut shock,
        );
        assert_eq!(shock.snapshot(3300).unwrap().duration_msec, 0);
        assert!(
            poison
                .advance(
                    3400,
                    &gas,
                    Recipient {
                        protected: true,
                        ..t
                    },
                    &collision,
                    &mut shock
                )
                .is_none()
        );
        assert!(
            poison
                .advance(3500, &gas, t, &collision, &mut shock)
                .is_some()
        );
        assert_eq!(shock.snapshot(3500).unwrap().index, 7);
    }
    #[test]
    fn gas_and_smoke_share_all_native_slots_and_failed_allocation_never_creates_invisible_damage() {
        let collision = world(false);
        let mut gas = Gas::default();
        let mut persistent = crate::game_smoke::Smoke::default();
        for _ in 0..31 {
            assert!(persistent.spawn(&Explosion {
                time: 1000,
                origin: [0.0, 0.0, 2.0],
                normal: [0.0, 0.0, 1.0],
                definition: crate::game_grenade::captured("smoke_grenade_mp")
            }));
        }
        deploy(&mut gas, &mut persistent, &collision, 1000);
        assert_eq!(persistent.snapshots(1000).len(), 32);
        gas.clear();
        assert!(!gas.deploy(
            &Explosion {
                time: 1000,
                origin: [0.0, 0.0, 2.0],
                normal: [0.0, 0.0, 1.0],
                definition: crate::game_grenade::captured("tabun_gas_mp")
            },
            0,
            1,
            &collision,
            &mut persistent
        ));
        assert!(gas.exposure(1000, target(), &collision).is_none());
    }
}
