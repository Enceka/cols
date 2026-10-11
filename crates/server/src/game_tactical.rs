//! Native shock resources with reference-script server duration rules.
//! Geometry uses the server's current body/eye and world traces; full original
//! production scripts and bone-level exposure are not recovered.
use codol_protocol::game_shellshock::ShellShockSnapshot;
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    Flash,
    Concussion,
}
#[derive(Deserialize)]
struct Binding {
    kind: Kind,
    name: String,
    index: u8,
    configstring: u16,
    screen_type: u8,
    movement: bool,
    look_control: bool,
    max_pitch_speed: f32,
    max_yaw_speed: f32,
    rawfile_sha256: String,
    parameter_sha256: String,
}
#[derive(Deserialize)]
struct Rules {
    source: String,
    flash_max_msec: u32,
    flash_min_msec: u32,
    flash_angle_floor: f32,
    flash_angle_full_threshold: f32,
    concussion_min_msec: u32,
    concussion_distance_msec: u32,
}
#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    client_sha256: String,
    capture_sha256: String,
    bindings: Vec<Binding>,
    server_rules: Rules,
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let c: Catalog = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/tactical-status-3.3.3.3.json"
    ))
    .expect("captured native shock resources");
    assert_eq!(c.schema_version, 1);
    assert_eq!(c.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(c.capture_sha256.len(), 64);
    assert_eq!(c.bindings.len(), 2);
    assert!(c.server_rules.source.contains("reference script port"));
    for binding in &c.bindings {
        assert!((1..16).contains(&binding.index));
        assert_eq!(binding.configstring, 4275 + u16::from(binding.index));
        assert_eq!(binding.rawfile_sha256.len(), 64);
        assert_eq!(binding.parameter_sha256.len(), 64);
        assert_eq!(
            binding.name,
            if binding.kind == Kind::Flash {
                "flashbang_mp"
            } else {
                "concussion_grenade_mp"
            }
        );
        assert_eq!(
            binding.screen_type,
            if binding.kind == Kind::Flash { 1 } else { 0 }
        );
        assert_eq!(binding.movement, binding.kind == Kind::Concussion);
    }
    c
});
fn binding(kind: Kind) -> &'static Binding {
    CATALOG.bindings.iter().find(|b| b.kind == kind).unwrap()
}
#[cfg(test)]
pub(crate) fn configstrings() -> Vec<(u16, String)> {
    CATALOG
        .bindings
        .iter()
        .map(|b| (b.configstring, b.name.clone()))
        .collect()
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub kind: Kind,
    pub duration_msec: u16,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct State {
    last_index: Option<u8>,
    active: Option<(Kind, ShellShockSnapshot)>,
}
impl State {
    pub fn apply(&mut self, now: u32, hit: Hit) -> bool {
        if hit.duration_msec == 0 {
            return false;
        }
        // Same-time notifications combine using the strongest duration, as
        // reference applyFlash does. Later different effects use the single
        // native shader slot rather than inventing a combined shock preset.
        if self
            .active
            .is_some_and(|(_, s)| s.begin_time == now && s.duration_msec >= hit.duration_msec)
        {
            return false;
        }
        let index = binding(hit.kind).index;
        self.last_index = Some(index);
        self.active = Some((
            hit.kind,
            ShellShockSnapshot {
                index,
                begin_time: now,
                duration_msec: hit.duration_msec,
            },
        ));
        true
    }
    pub fn clear(&mut self) {
        self.active = None;
    }
    pub fn snapshot(&self, now: u32) -> Option<ShellShockSnapshot> {
        if let Some((_, s)) = self
            .active
            .filter(|(_, s)| now >= s.begin_time && now - s.begin_time < u32::from(s.duration_msec))
        {
            return Some(s);
        }
        // index0 reads the client's local override. Preserve the bound Named
        // index with begin/duration0 to force the native reset branch.
        self.last_index.map(|index| ShellShockSnapshot {
            index,
            ..Default::default()
        })
    }
    fn active_kind(&self, now: u32) -> Option<Kind> {
        self.active
            .filter(|(_, s)| now >= s.begin_time && now - s.begin_time < u32::from(s.duration_msec))
            .map(|(k, _)| k)
    }
    pub fn blind(&self, now: u32) -> bool {
        self.active_kind(now) == Some(Kind::Flash)
    }
    pub fn stunned(&self, now: u32) -> bool {
        self.active_kind(now).is_some_and(|k| binding(k).movement)
    }
    pub fn move_scale(&self, now: u32, robot: bool) -> f32 {
        if self.stunned(now) {
            if robot { 0.75 } else { 0.4 }
        } else {
            1.0
        }
    }
    pub fn turn_limit(&self, now: u32) -> Option<[f32; 2]> {
        self.active_kind(now).and_then(|k| {
            let b = binding(k);
            b.look_control
                .then_some([b.max_pitch_speed, b.max_yaw_speed])
        })
    }
}
pub(crate) struct Recipient {
    pub feet: [f32; 3],
    pub eye: [f32; 3],
    pub angles: [u16; 3],
}
pub(crate) fn hit(
    explosion: &crate::game_grenade::Explosion,
    target: Recipient,
    protection: f32,
    collision: &crate::game_motion::Collision,
) -> Option<Hit> {
    let kind = explosion.definition.tactical?;
    if !protection.is_finite() || protection <= 0.0 {
        return None;
    }
    let radius = f32::from(explosion.definition.radius);
    let distance = (0..3)
        .map(|i| (target.feet[i] - explosion.origin[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    if radius <= 0.0 || distance >= radius {
        return None;
    }
    let trace = collision.raycast(explosion.origin, target.eye);
    if trace.start_solid || trace.all_solid || trace.fraction < 1.0 {
        return None;
    }
    let falloff = 1.0 - distance / radius;
    let rules = &CATALOG.server_rules;
    let duration = match kind {
        Kind::Flash => {
            let yaw = f32::from(target.angles[1]) * std::f32::consts::TAU / 65536.0;
            let pitch = f32::from(target.angles[0] as i16) * std::f32::consts::TAU / 65536.0;
            let forward = [
                pitch.cos() * yaw.cos(),
                pitch.cos() * yaw.sin(),
                -pitch.sin(),
            ];
            let offset: [f32; 3] = std::array::from_fn(|i| explosion.origin[i] - target.eye[i]);
            let length = offset.iter().map(|x| x * x).sum::<f32>().sqrt();
            // Current server angular exposure estimate; upstream native
            // flashbang amount_angle generation is not recovered.
            let angle = if length > 0.0 {
                (1.0 + (0..3).map(|i| forward[i] * offset[i] / length).sum::<f32>()) * 0.5
            } else {
                1.0
            };
            let angle = if angle > rules.flash_angle_full_threshold {
                1.0
            } else {
                angle.max(rules.flash_angle_floor)
            };
            let duration = rules.flash_max_msec as f32 * falloff * angle;
            if duration < rules.flash_min_msec as f32 {
                return None;
            }
            duration
        }
        Kind::Concussion => {
            rules.concussion_min_msec as f32 + rules.concussion_distance_msec as f32 * falloff
        }
    };
    let duration = (duration * protection).round().clamp(0.0, 65535.0) as u16;
    (duration != 0).then_some(Hit {
        kind,
        duration_msec: duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world(wall: bool) -> crate::game_motion::Collision {
        let mut brushes = vec![
            serde_json::json!({"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}),
        ];
        if wall {
            brushes.push(serde_json::json!({"center":[64,0,100],"half":[2,1000,100],"contents":1,"planes":[]}));
        }
        let data = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,"map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":brushes,"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&data).unwrap()).unwrap();
        crate::game_motion::Collision::load(file.path()).unwrap()
    }
    fn blast(kind: Kind) -> crate::game_grenade::Explosion {
        crate::game_grenade::Explosion {
            time: 1000,
            origin: [128.0, 0.0, 60.0],
            normal: [0.0, 0.0, 1.0],
            definition: crate::game_grenade::captured(if kind == Kind::Flash {
                "flash_grenade_mp"
            } else {
                "concussion_grenade_mp"
            }),
        }
    }
    fn target(yaw: u16) -> Recipient {
        Recipient {
            feet: [0.0; 3],
            eye: [0.0, 0.0, 60.0],
            angles: [0, yaw, 0],
        }
    }
    #[test]
    fn reference_duration_port_obeys_facing_world_occlusion_and_recipient_mask() {
        let clear = world(false);
        let wall = world(true);
        let flash = blast(Kind::Flash);
        let front = hit(&flash, target(0), 1.0, &clear).unwrap();
        let back = hit(&flash, target(32768), 1.0, &clear).unwrap();
        assert!(front.duration_msec >= 4000);
        assert!((i32::from(front.duration_msec) - 2 * i32::from(back.duration_msec)).abs() <= 1);
        let perks = crate::game_perks::Perks::selected([0, 0, 60137], &[60137]).unwrap();
        let protected = hit(
            &flash,
            target(0),
            perks.value("modFlashProtectionScale", 1.0),
            &clear,
        )
        .unwrap();
        assert!(
            (i32::from(front.duration_msec) - 10 * i32::from(protected.duration_msec)).abs() <= 5
        );
        assert!(hit(&flash, target(0), 1.0, &wall).is_none());
        assert!(hit(&flash, target(0), 0.0, &clear).is_none());
        let mut distant = target(0);
        distant.feet = [-1000.0, 0.0, 0.0];
        distant.eye = [-1000.0, 0.0, 60.0];
        assert!(hit(&flash, distant, 1.0, &clear).is_none());
        let concussion = blast(Kind::Concussion);
        let normal = hit(&concussion, target(0), 1.0, &clear).unwrap();
        let turned = hit(&concussion, target(32768), 1.0, &clear).unwrap();
        assert_eq!(normal.duration_msec, turned.duration_msec);
        let mask = hit(
            &concussion,
            target(0),
            perks.value("modStunProtectionScale", 1.0),
            &clear,
        )
        .unwrap();
        assert!((i32::from(normal.duration_msec) - 10 * i32::from(mask.duration_msec)).abs() <= 5);
    }
    #[test]
    fn expiry_death_and_same_tick_maximum_keep_a_named_reset_and_restore_controls() {
        let mut state = State::default();
        assert_eq!(state.snapshot(1000), None);
        assert!(state.apply(
            1000,
            Hit {
                kind: Kind::Concussion,
                duration_msec: 5000
            }
        ));
        assert!(!state.apply(
            1000,
            Hit {
                kind: Kind::Concussion,
                duration_msec: 500
            }
        ));
        assert_eq!(state.move_scale(1000, false), 0.4);
        assert_eq!(state.move_scale(1000, true), 0.75);
        assert_eq!(state.turn_limit(1000), Some([75.0, 75.0]));
        assert!(!state.blind(1000));
        assert!(state.stunned(5999));
        assert!(!state.stunned(6000));
        assert_eq!(
            state.snapshot(6000),
            Some(ShellShockSnapshot {
                index: 1,
                begin_time: 0,
                duration_msec: 0
            })
        );
        assert_eq!(state.move_scale(6000, false), 1.0);
        state.apply(
            7000,
            Hit {
                kind: Kind::Flash,
                duration_msec: 600,
            },
        );
        assert!(state.blind(7000));
        assert!(!state.stunned(7000));
        assert_eq!(state.move_scale(7000, false), 1.0);
        state.clear();
        assert_eq!(
            state.snapshot(7001),
            Some(ShellShockSnapshot {
                index: 5,
                begin_time: 0,
                duration_msec: 0
            })
        );
        assert!(!state.blind(7001));
        assert_eq!(
            configstrings(),
            vec![
                (4280, "flashbang_mp".to_owned()),
                (4276, "concussion_grenade_mp".to_owned())
            ]
        );
    }
}
