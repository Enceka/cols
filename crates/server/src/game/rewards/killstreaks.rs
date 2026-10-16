//! Authority-owned life progress, radio support and the fixed-cost nuclear reward.
//! The 25-kill extra reward and 10s/1.5s timing follow the local Alpha/LAN
//! compatibility rules; native resources/layouts are verified independently.
use anyhow::{Result, ensure};
use codol_protocol::game::snapshot::killstreaks::StreakSnapshot;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Catalog {
    schema_version: u8,
    client_sha256: String,
    class_offsets: Vec<[usize; 4]>,
    rewards: Vec<Reward>,
    marketplace: Vec<Item>,
}
#[derive(Deserialize)]
struct Reward {
    native_id: u8,
    name: String,
    cost_c4: Option<u8>,
}
#[derive(Deserialize)]
struct Item {
    item_id: u32,
    native_id: Option<u8>,
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let catalog: Catalog = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/killstreaks-3.3.3.3.json"
    )))
    .expect("captured killstreak identities and profile layout");
    assert_eq!(catalog.schema_version, 1);
    assert_eq!(catalog.client_sha256, codol_protocol::CLIENT_SHA256);
    assert_eq!(catalog.class_offsets.len(), 60);
    catalog
});

/// The three saved ordinary rewards occupy native HUD slots 1..3. Their
/// previews do not advertise an action until an effect handler grants it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Previews {
    ids: [u8; 3],
    targets: [u8; 3],
}
impl Previews {
    fn highest_target(self) -> u8 {
        self.ids
            .into_iter()
            .zip(self.targets)
            .filter(|(id, _)| *id != 0)
            .map(|(_, target)| target)
            .max()
            .unwrap_or(0)
    }
    pub fn from_profile(snapshot: &[u8], class: u8, owned: &[u32], shift: f32) -> Result<Self> {
        let offsets = CATALOG
            .class_offsets
            .get(usize::from(class))
            .ok_or_else(|| anyhow::anyhow!("invalid killstreak class"))?;
        let mut selected = Vec::new();
        for offset in &offsets[..3] {
            let raw = snapshot
                .get(*offset..*offset + 4)
                .ok_or_else(|| anyhow::anyhow!("killstreak profile bounds"))?;
            let raw = i32::from_le_bytes(raw.try_into().unwrap());
            ensure!(raw >= 0, "negative killstreak item reference");
            let item = (raw as u32).saturating_sub(1);
            if item == 0 {
                continue;
            }
            ensure!(owned.contains(&item), "unowned killstreak item {item}");
            let native = CATALOG
                .marketplace
                .iter()
                .find(|m| m.item_id == item)
                .and_then(|m| m.native_id);
            let reward = native.and_then(|id| CATALOG.rewards.iter().find(|r| r.native_id == id));
            let Some(reward) = reward.filter(|r| r.cost_c4.is_some_and(|c| c > 0)) else {
                tracing::warn!(item, "saved killstreak has no verified native preview");
                continue;
            };
            ensure!(
                !selected.iter().any(|&(id, _)| id == reward.native_id),
                "duplicate saved killstreak"
            );
            ensure!(
                shift.is_finite() && (-1.0..=0.0).contains(&shift),
                "unsupported killstreak cost shift"
            );
            let cost = (f32::from(reward.cost_c4.unwrap()) + shift).max(1.0) as u8;
            selected.push((reward.native_id, cost));
        }
        selected.sort_by_key(|&(id, cost)| (cost, id));
        let mut result = Self::default();
        for (i, (id, cost)) in selected.into_iter().enumerate() {
            result.ids[i] = id;
            result.targets[i] = cost;
        }
        Ok(result)
    }
    pub fn apply(self, snapshot: &mut StreakSnapshot) {
        snapshot.ids[1..4].copy_from_slice(&self.ids);
        snapshot.targets[1..4].copy_from_slice(&self.targets);
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Radio {
    pub native_id: u8,
    pub weapon: u16,
    pub switch: crate::game::weapons::held::SwitchTiming,
    pub fire_interval_msec: u16,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RadarResources {
    pub schema_version: u8,
    pub client_sha256: String,
    pub sources_sha256: [String; 2],
    pub map: String,
    pub ncs_crc: u32,
    pub radios: Vec<Radio>,
    #[serde(default)]
    pub models: BTreeMap<String, u16>,
    #[serde(default)]
    pub tags: BTreeMap<String, u8>,
    #[serde(default)]
    pub vehicles: BTreeMap<String, u8>,
    #[serde(default)]
    pub animations: BTreeMap<String, AnimationBinding>,
    #[serde(default)]
    pub loop_sounds: BTreeMap<String, u8>,
    #[serde(default)]
    pub model_bounds: BTreeMap<String, ModelBounds>,
    #[serde(default)]
    pub camera_mounts: BTreeMap<String, CameraMount>,
    #[serde(default)]
    pub effects: BTreeMap<String, u16>,
    #[serde(default)]
    pub selectors: BTreeMap<String, u8>,
    #[serde(default)]
    pub weapons: BTreeMap<String, WeaponFacts>,
    #[serde(default)]
    pub air_route: Option<AirRoute>,
    #[serde(default)]
    pub air_paths: Option<crate::game::rewards::paths::FlightNodes>,
    #[serde(default)]
    pub remote_spawns: Option<RemoteSpawns>,
    #[serde(default)]
    pub remote_vision: Option<RemoteVisionResources>,
    #[serde(default)]
    pub remote_exit: Option<RemoteExitResources>,
    #[serde(default)]
    pub animals: Option<super::dogs::Resources>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteExitResources {
    pub white: u16,
    pub grain: u16,
    pub material_bank_count: u16,
}
impl RemoteExitResources {
    pub(crate) fn valid(self) -> bool {
        self.white == 1000 && self.grain == 1001 && self.material_bank_count < 1000
    }
    pub(crate) fn snapshot(
        self,
    ) -> codol_protocol::game::snapshot::hit_feedback::RemoteExitSnapshot {
        codol_protocol::game::snapshot::hit_feedback::RemoteExitSnapshot {
            visible: true,
            white: self.white,
            grain: self.grain,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteVisionResources {
    pub flight: String,
    #[serde(default)]
    pub arena_absent: bool,
    pub rawfiles_sha256: BTreeMap<String, String>,
}
impl RemoteVisionResources {
    pub(crate) fn inverted_name(&self) -> Option<&'static str> {
        (self.flight_name().is_some()
            && self
                .rawfiles_sha256
                .contains_key("vision/missilecam.vision"))
        .then_some("missilecam")
    }
    pub(crate) fn flight_name(&self) -> Option<&'static str> {
        let expected = match self.flight.as_str() {
            "thermal_mp" => "thermal_mp",
            "thermal_snowlevel_mp" => "thermal_snowlevel_mp",
            _ => return None,
        };
        let mut required = vec![
            "vision/black_bw.vision".to_owned(),
            format!("vision/{expected}.vision"),
        ];
        if !self.arena_absent {
            required.push("mp/basemaps.arena".to_owned());
        }
        if self
            .rawfiles_sha256
            .contains_key("vision/missilecam.vision")
        {
            required.push("vision/missilecam.vision".to_owned());
        }
        (self.flight == expected
            && (!self.arena_absent || expected == "thermal_mp")
            && self.rawfiles_sha256.len() == required.len()
            && required.iter().all(|name| {
                self.rawfiles_sha256.get(name).is_some_and(|hash| {
                    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                })
            }))
        .then_some(expected)
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteSpawns {
    pub sources_sha256: [String; 1],
    pub points: Vec<RemoteSpawn>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteSpawn {
    pub origin: [f32; 3],
    pub target: [f32; 3],
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnimationBinding {
    pub tree: u8,
    pub node: u16,
    #[serde(default)]
    pub parent: u16,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AirRoute {
    pub center: [f32; 3],
    pub sources_sha256: [String; 1],
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CameraMount {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
}
impl CameraMount {
    pub(crate) fn valid(self) -> bool {
        self.rotation.iter().all(|n| n.is_finite())
            && (0.5..=2.0).contains(&self.rotation.iter().map(|n| n * n).sum::<f32>())
            && self
                .translation
                .iter()
                .all(|n| n.is_finite() && n.abs() <= 30000.0)
    }
}
impl ModelBounds {
    pub fn valid(self) -> bool {
        (0..3).all(|i| {
            self.min[i].is_finite()
                && self.max[i].is_finite()
                && self.min[i] < self.max[i]
                && self.min[i].abs() <= 4096.0
                && self.max[i].abs() <= 4096.0
        })
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarkerFacts {
    pub prepare_msec: u16,
    pub throw_msec: u16,
    pub throw_delay_msec: u16,
    pub fuse_msec: u16,
    pub cook: bool,
    pub speed_up: u16,
    pub speed_forward: u16,
    pub stickiness: u32,
    pub parallel: [f32; 31],
    pub perpendicular: [f32; 31],
}
impl MarkerFacts {
    pub fn valid(&self) -> bool {
        !self.cook
            && self.prepare_msec > 0
            && self.prepare_msec <= 15000
            && self.throw_msec > 0
            && self.throw_msec <= 15000
            && self.throw_delay_msec <= self.throw_msec
            && self.fuse_msec > 0
            && self.fuse_msec <= 60000
            && self.speed_up <= 50000
            && self.speed_forward <= 50000
            && self
                .parallel
                .iter()
                .chain(&self.perpendicular)
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WeaponFacts {
    pub weapon: u16,
    pub weapon_type: u32,
    pub weapon_class: u32,
    pub clip_key: Option<u16>,
    pub ammo_key: Option<u16>,
    pub switch: crate::game::weapons::held::SwitchTiming,
    pub fire_interval_msec: u16,
    #[serde(default)]
    pub fire_delay_msec: u16,
    #[serde(default)]
    pub clip_size: u16,
    #[serde(default)]
    pub reload_msec: u16,
    pub radius: u32,
    pub inner_damage: u32,
    pub outer_damage: u32,
    #[serde(default)]
    pub projectile_speed: u32,
    #[serde(default)]
    pub projectile: Option<ProjectileFacts>,
    #[serde(default)]
    pub marker: Option<MarkerFacts>,
    #[serde(default)]
    pub damage_stages: [u32; 6],
    #[serde(default)]
    pub damage_ranges: [f32; 6],
    #[serde(default)]
    pub operator: Option<super::armor::OperatorFacts>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectileFacts {
    pub lifetime_msec: u32,
    pub explosion_kind: u8,
}
pub(crate) fn reward_name(id: u8) -> Option<&'static str> {
    CATALOG
        .rewards
        .iter()
        .find(|r| r.native_id == id)
        .map(|r| r.name.as_str())
}
impl RadarResources {
    pub fn load(path: &Path, map: &str, crc: u32) -> Result<Self> {
        ensure!(
            path.metadata()?.len() <= 256 * 1024,
            "radar resources exceed budget"
        );
        let result: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            result.schema_version == 1
                && result.client_sha256 == codol_protocol::CLIENT_SHA256
                && result.map == map
                && result.ncs_crc == crc
                && result
                    .sources_sha256
                    .iter()
                    .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                && (2..=28).contains(&result.radios.len())
                && [1, 3].iter().all(|id| result.radio(*id).is_some())
                && result
                    .radios
                    .iter()
                    .map(|r| r.native_id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == result.radios.len()
                && result
                    .radios
                    .iter()
                    .map(|r| r.weapon)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == result.radios.len()
                && result
                    .radios
                    .iter()
                    .all(|r| CATALOG.rewards.iter().any(|c| c.native_id == r.native_id))
                && result
                    .loop_sounds
                    .iter()
                    .all(|(name, slot)| name == "weap_cobra_20mm_fire_npc" && *slot == 240)
                && result
                    .remote_spawns
                    .as_ref()
                    .is_none_or(|spawns| spawns.points.len() <= 64
                        && spawns
                            .sources_sha256
                            .iter()
                            .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                        && spawns.points.iter().all(|p| p
                            .origin
                            .iter()
                            .chain(&p.target)
                            .all(|v| v.is_finite() && v.abs() <= 30000.0)
                            && p.origin
                                .iter()
                                .zip(p.target)
                                .map(|(a, b)| (a - b).powi(2))
                                .sum::<f32>()
                                > 1.0))
                && result.remote_exit.is_none_or(RemoteExitResources::valid)
                && result
                    .remote_vision
                    .as_ref()
                    .is_none_or(|v| v.flight_name().is_some())
                && result.tags.values().all(|v| (1..64).contains(v))
                && result.tags.values().collect::<BTreeSet<_>>().len() == result.tags.len()
                && result.models.values().all(|v| (1..1024).contains(v))
                && result.vehicles.values().all(|v| (1..32).contains(v))
                && result.vehicles.values().collect::<BTreeSet<_>>().len() == result.vehicles.len()
                && result.animations.values().all(|a| (1..16).contains(&a.tree)
                    && (1..2048).contains(&a.node)
                    && a.parent < 2048
                    && a.parent != a.node)
                && result.air_route.as_ref().is_none_or(|r| r
                    .center
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 30000.0)
                    && r.sources_sha256
                        .iter()
                        .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())))
                && result
                    .model_bounds
                    .iter()
                    .all(|(name, bounds)| result.models.contains_key(name) && bounds.valid())
                && result
                    .camera_mounts
                    .iter()
                    .all(|(name, mount)| result.models.contains_key(name) && mount.valid())
                && result.air_paths.as_ref().is_none_or(|paths| paths.valid())
                && result.animals.as_ref().is_none_or(|a| a.valid())
                && result.effects.values().all(|v| (1..400).contains(v))
                && result.selectors.values().all(|v| (1..=3).contains(v))
                && result.selectors.values().collect::<BTreeSet<_>>().len()
                    == result.selectors.len()
                && result.weapons.values().all(|w| (1..900).contains(&w.weapon)
                    && w.operator
                        .as_ref()
                        .is_none_or(|o| o.valid() && w.weapon_type == 0)
                    && w.weapon_type <= 6
                    && w.projectile.is_none_or(|p| w.weapon_type == 2
                        && (1..=3_600_000).contains(&p.lifetime_msec)
                        && p.explosion_kind <= 7)
                    && w.weapon_class <= 32
                    && w.reload_msec <= 15000
                    && w.fire_delay_msec <= 15000
                    && w.clip_key.is_none_or(|v| v < 1024)
                    && w.ammo_key.is_none_or(|v| v < 1024)
                    && [
                        w.switch.drop_msec,
                        w.switch.raise_msec,
                        w.switch.empty_drop_msec,
                        w.switch.empty_raise_msec,
                        w.fire_interval_msec
                    ]
                    .iter()
                    .all(|v| *v <= 15000)
                    && w.radius <= 1_000_000
                    && w.projectile_speed <= 50000
                    && w.marker.as_ref().is_none_or(|m| m.valid()
                        && w.weapon_type == 1
                        && w.weapon_class == 6
                        && w.radius == 0
                        && w.projectile_speed > 0)
                    && w.damage_stages.iter().all(|v| *v <= 1_000_000)
                    && w.damage_ranges
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=1_000_000.0).contains(v))
                    && w.inner_damage <= 1_000_000
                    && w.outer_damage <= 1_000_000)
                && result.radios.iter().all(|r| (1..900).contains(&r.weapon)
                    && [
                        r.switch.drop_msec,
                        r.switch.raise_msec,
                        r.switch.empty_drop_msec,
                        r.switch.empty_raise_msec
                    ]
                    .iter()
                    .all(|v| *v <= 15000)
                    && r.switch.raise_msec > 0
                    && r.fire_interval_msec <= 15000),
            "radar map/build/resources mismatch"
        );
        for radio in &result.radios {
            if matches!(radio.native_id, 2 | 4 | 9 | 22)
                && result
                    .weapons
                    .values()
                    .any(|w| w.weapon == radio.weapon && w.marker.is_some())
            {
                ensure!(
                    crate::game::weapons::grenade::Definition::marker(&result, radio.native_id)
                        .is_some(),
                    "invalid registered airdrop marker"
                );
            }
        }
        Ok(result)
    }
    pub fn radio(&self, id: u8) -> Option<&Radio> {
        self.radios.iter().find(|r| r.native_id == id)
    }
}

/// Local Alpha/LAN compatibility duration; independent of captured weapon timing.
pub(crate) const RADAR_MSEC: u32 = 30_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RadioEvent {
    Earned {
        actor: u8,
        id: u8,
        time: u32,
    },
    Called {
        actor: u8,
        id: u8,
        team: u8,
        time: u32,
    },
}
#[derive(Clone, Copy)]
struct Support {
    actor: u8,
    start: u32,
    team: u8,
    id: u8,
    end: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SupportFlight {
    pub actor: u8,
    pub id: u8,
    pub team: u8,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptanceSupport {
    pub after_msec: u32,
    pub native_id: u8,
    #[serde(default)]
    pub gunner_delay_msec: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NukeResources {
    pub schema_version: u8,
    pub client_sha256: String,
    pub sources_sha256: Vec<String>,
    pub map: String,
    pub ncs_crc: u32,
    pub weapon: u16,
    pub damage_weapon: u16,
    pub switch: crate::game::weapons::held::SwitchTiming,
    pub fire_interval_msec: u16,
    pub flash_fx: u16,
    pub blast_fx: u16,
    pub aftermath_fx: u16,
    pub tick_sound: u16,
    pub music_sound: u16,
    pub earned_text: u16,
    pub called_text: u16,
    /// Content hashes of both native naked-vision rawfiles.
    #[serde(default)]
    pub vision: Option<NukeVision>,
    #[serde(default)]
    pub acceptance_seed: u8,
    #[serde(default)]
    pub acceptance_stationary: bool,
    #[serde(default)]
    pub acceptance_attack_aircraft: bool,
    #[serde(default)]
    pub acceptance_remote_missile: bool,
    #[serde(default)]
    pub acceptance_remote_vehicles: bool,
    #[serde(default)]
    pub acceptance_gunners: bool,
    #[serde(default)]
    pub acceptance_missile_drone: bool,
    #[serde(default)]
    pub acceptance_dogs: bool,
    #[serde(default)]
    pub acceptance_armor: bool,
    #[serde(default)]
    pub acceptance_kingpin_stock: bool,
    #[serde(default)]
    pub acceptance_fire_after_msec: Option<u32>,
    #[serde(default)]
    pub acceptance_enemy_support: Vec<AcceptanceSupport>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NukeVision {
    pub rawfiles_sha256: BTreeMap<String, String>,
}
impl NukeVision {
    fn valid(&self) -> bool {
        self.rawfiles_sha256.len() == 2
            && ["vision/mpnuke.vision", "vision/mpnuke_aftermath.vision"]
                .into_iter()
                .all(|name| {
                    self.rawfiles_sha256
                        .get(name)
                        .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                })
    }
}
impl NukeResources {
    pub fn load(path: &Path, map: &str, crc: u32) -> Result<Self> {
        ensure!(
            path.metadata()?.len() <= 16384,
            "nuke resources exceed budget"
        );
        let result: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            result
                .acceptance_fire_after_msec
                .is_none_or(|t| result.acceptance_stationary && (1000..=600_000).contains(&t))
                && result.acceptance_enemy_support.len() <= 8
                && result
                    .acceptance_enemy_support
                    .iter()
                    .all(|s| result.acceptance_stationary
                        && (1000..=600_000).contains(&s.after_msec)
                        && matches!(s.native_id, 1 | 3 | 14 | 20)
                        && s.gunner_delay_msec.is_none_or(
                            |delay| s.native_id == 14 && (1000..=15000).contains(&delay)
                        ))
                && result
                    .acceptance_enemy_support
                    .windows(2)
                    .all(|w| w[0].after_msec < w[1].after_msec)
                && result.acceptance_seed <= 24
                && (!result.acceptance_attack_aircraft || result.acceptance_stationary)
                && (!result.acceptance_remote_missile || result.acceptance_stationary)
                && (!result.acceptance_remote_vehicles || result.acceptance_stationary)
                && (!result.acceptance_gunners || result.acceptance_stationary)
                && (!result.acceptance_missile_drone || result.acceptance_stationary)
                && (!result.acceptance_dogs || result.acceptance_stationary)
                && (!result.acceptance_armor || result.acceptance_stationary)
                && (!result.acceptance_kingpin_stock
                    || result.acceptance_armor && result.acceptance_stationary)
                && ((result.acceptance_seed == 0 && !result.acceptance_stationary)
                    || path
                        .parent()
                        .is_some_and(|p| p.join(".codol-nuke-acceptance").is_file())),
            "nuke diagnostics require a marked directory and seed below the award threshold"
        );
        ensure!(
            result.schema_version == 1
                && result.vision.as_ref().is_none_or(NukeVision::valid)
                && result.client_sha256 == codol_protocol::CLIENT_SHA256
                && result.map == map
                && result.ncs_crc == crc
                && result.sources_sha256.len() == 2
                && result
                    .sources_sha256
                    .iter()
                    .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                && (1..900).contains(&result.weapon)
                && (1..900).contains(&result.damage_weapon)
                && result.weapon != result.damage_weapon
                && result.switch.valid()
                && (1..=15000).contains(&result.fire_interval_msec)
                && [result.flash_fx, result.blast_fx, result.aftermath_fx]
                    .iter()
                    .all(|s| (1..=399).contains(s))
                && [result.tick_sound, result.music_sound]
                    .iter()
                    .all(|s| (512..=16000).contains(s))
                && [result.earned_text, result.called_text]
                    .iter()
                    .all(|s| (1..1023).contains(s)),
            "nuke map/build/resources mismatch"
        );
        Ok(result)
    }
}
#[cfg(test)]
pub(crate) fn test_resources() -> NukeResources {
    NukeResources {
        schema_version: 1,
        client_sha256: codol_protocol::CLIENT_SHA256.into(),
        sources_sha256: vec!["0".repeat(64); 2],
        map: "mp_afghan".into(),
        ncs_crc: 0x12345678,
        weapon: 323,
        damage_weapon: 334,
        switch: crate::game::weapons::held::SwitchTiming {
            drop_msec: 1000,
            raise_msec: 833,
            empty_drop_msec: 1000,
            empty_raise_msec: 833,
        },
        fire_interval_msec: 800,
        flash_fx: 77,
        blast_fx: 76,
        aftermath_fx: 61,
        tick_sound: 1986,
        music_sound: 1976,
        earned_text: 448,
        called_text: 556,
        vision: None,
        acceptance_seed: 0,
        acceptance_stationary: false,
        acceptance_attack_aircraft: false,
        acceptance_remote_missile: false,
        acceptance_remote_vehicles: false,
        acceptance_gunners: false,
        acceptance_missile_drone: false,
        acceptance_dogs: false,
        acceptance_armor: false,
        acceptance_kingpin_stock: false,
        acceptance_fire_after_msec: None,
        acceptance_enemy_support: Vec::new(),
    }
}

pub(crate) const NUKE_KILLS: u16 = 25;
pub(crate) const COUNTDOWN_MSEC: u32 = 10_000;
pub(crate) const AFTER_FLASH_MSEC: u32 = 1500;

/// Native e -> 0x5141f0 -> 0x5fbee0 expands a localized segment and its
/// literal argument. A bare HUD text slot cannot supply the caller's name.
pub(crate) fn called_message(name: &str) -> Vec<u8> {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '"' => '\'',
            '\\' => '/',
            _ => c,
        })
        .take(64)
        .collect();
    let name = if name.is_empty() { "Player" } else { &name };
    format!("e \"\u{14}MP_TACTICAL_NUKE_CALLED\u{15}\u{16}{name}\"").into_bytes()
}

struct Actor {
    count: u16,
    record_count: u16,
    best: u16,
    death: u32,
    alive: bool,
    granted: bool,
    owned: bool,
    previews: Previews,
    inventory: crate::game::rewards::inventory::Inventory,
    radios_granted: [bool; 3],
}
impl Actor {
    fn hud_progress(&self) -> u8 {
        // Ordinary rewards repeat over the highest selected threshold. The
        // completion kill still shows the full bar; the next kill starts at1.
        // Full eligible life kills remain independent for the record and nuke.
        let span = u16::from(self.previews.highest_target());
        if span == 0 {
            self.count.min(255) as u8
        } else if self.count == 0 {
            0
        } else {
            ((self.count - 1) % span + 1) as u8
        }
    }
    fn display(&self) -> Previews {
        let mut p = self.previews;
        let owned = self.inventory.owned_ids();
        let visible = &owned[..owned.len().min(3)];
        for &id in visible {
            if p.ids.contains(&id) {
                continue;
            }
            let slot = (0..3)
                .rev()
                .find(|&s| !visible.contains(&p.ids[s]))
                .expect("an undisplayed reward has a replaceable slot");
            p.ids[slot] = id;
            p.targets[slot] = CATALOG
                .rewards
                .iter()
                .find(|r| r.native_id == id)
                .and_then(|r| r.cost_c4)
                .unwrap_or(0);
        }
        p
    }
}
impl Default for Actor {
    fn default() -> Self {
        Self {
            count: 0,
            record_count: 0,
            best: 0,
            death: 0,
            alive: true,
            granted: false,
            owned: false,
            previews: Previews::default(),
            inventory: Default::default(),
            radios_granted: [false; 3],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Earned { actor: u8, time: u32 },
    Called { actor: u8, time: u32 },
    Explosion { actor: u8, time: u32 },
    Vision { actor: u8, time: u32 },
    End { actor: u8, time: u32 },
}
#[derive(Clone, Copy)]
struct Incoming {
    actor: u8,
    start: u32,
    exploded: bool,
    vision_started: bool,
    ended: bool,
}
#[derive(Default)]
pub(crate) struct Streaks {
    actors: [Actor; 12],
    incoming: Option<Incoming>,
    events: VecDeque<Event>,
    enabled: bool,
    acceptance_seed: u8,
    radios_enabled: bool,
    available: u64,
    radio_events: VecDeque<RadioEvent>,
    support: Vec<Support>,
}
impl Streaks {
    pub fn configure_radios(&mut self, actor: u8, previews: Previews) {
        self.radios_enabled = true;
        if self.available == 0 {
            self.available = (1 << 1) | (1 << 3);
        }
        let state = &mut self.actors[usize::from(actor)];
        if state.previews != previews {
            state.radios_granted = [false; 3];
        }
        state.previews = previews;
    }
    /// Explicit isolated diagnostic stock, never reported as earned kills.
    pub(crate) fn acceptance_supply_support(&mut self, actor: u8, id: u8) {
        assert!(actor > 0 && actor < 12 && matches!(id, 1 | 3 | 14 | 20));
        self.configure_radios(
            actor,
            Previews {
                ids: [id, 0, 0],
                targets: [0; 3],
            },
        );
        let state = &mut self.actors[usize::from(actor)];
        assert!(state.inventory.grant(actor, id, state.death));
        tracing::info!(
            actor,
            id,
            "diagnostic enemy support stock supplied; not natural kills"
        );
    }
    pub fn enable_supported(&mut self, resources: &RadarResources) {
        self.available = resources
            .radios
            .iter()
            .filter(|r| {
                crate::game::rewards::mechanisms::Mechanism::radio_available(r.native_id, resources)
            })
            .fold(0, |m, r| m | (1u64 << r.native_id));
    }
    /// Caller must use the resource loader's marked stationary diagnostic.
    pub(crate) fn acceptance_enable_attack_aircraft(&mut self) {
        self.available |= (1 << 7) | (1 << 8) | (1 << 10);
        tracing::warn!("diagnostic attack aircraft controllers enabled; native acceptance pending");
    }
    /// Caller must use the resource loader's marked stationary diagnostic.
    pub(crate) fn acceptance_enable_predator(&mut self) {
        self.available |= 1 << 5;
    }
    pub(crate) fn acceptance_enable_remote_vehicles(&mut self, r: &RadarResources) {
        if super::remote_car::Car::resources_present(r) && r.tags.contains_key("tag_player") {
            self.available |= 1 << 18;
        }
    }
    pub(crate) fn acceptance_enable_gunners(&mut self, r: &RadarResources) {
        for id in [12, 13] {
            if super::gunners::Gunners::resources_present(r, id) {
                self.available |= 1 << id;
            }
        }
    }
    pub(crate) fn acceptance_enable_armor(&mut self, r: &RadarResources) {
        if r.weapons
            .get("mg4juggernaut_mp")
            .is_some_and(|w| w.operator.as_ref().is_some_and(|o| o.valid()))
            && [
                "mp_fullbody_juggernaut_explosive_so",
                "viewhands_juggernaut_ally",
            ]
            .into_iter()
            .all(|n| r.models.contains_key(n))
        {
            self.available |= 1 << 23;
        }
        if r.weapons
            .get("minigungunship_mp")
            .is_some_and(|w| w.clip_key.is_some() && w.operator.as_ref().is_some_and(|o| o.valid()))
            && [
                "c_kp_juggernaut_tf_00",
                "c_kp_juggernaut_sc_00",
                "viewhands_player_kp_juggernaut_tf",
                "viewhands_player_kp_juggernaut_sc",
            ]
            .into_iter()
            .all(|n| r.models.contains_key(n))
        {
            self.available |= 1 << 24;
        }
    }
    pub(crate) fn acceptance_enable_dogs(&mut self, r: &RadarResources) {
        if super::dogs::Dogs::resources_present(r) {
            self.available |= 1 << 20;
        }
    }
    pub(crate) fn acceptance_enable_drone(&mut self, r: &RadarResources) {
        if super::drones::Drones::resources_present(r) {
            self.available |= 1 << 26;
        }
    }
    pub fn emp_active(&self, team: u8, now: u32) -> bool {
        self.support
            .iter()
            .any(|s| s.id == 14 && s.team != team && s.end > now)
    }
    pub fn sr71_active(&self, team: u8, now: u32) -> bool {
        !self.emp_active(team, now)
            && self
                .support
                .iter()
                .any(|s| s.id == 19 && s.team == team && s.end > now)
    }
    pub fn radio_snapshot(
        &self,
        actor: u8,
        resources: &RadarResources,
        snapshot: &mut StreakSnapshot,
    ) {
        let state = &self.actors[usize::from(actor)];
        let display = state.display();
        display.apply(snapshot);
        for slot in 0..3 {
            if state.alive
                && self.available & (1u64 << display.ids[slot]) != 0
                && state.inventory.contains(display.ids[slot])
                && let Some(radio) = resources.radio(display.ids[slot])
            {
                snapshot.action_types[slot + 1] = 1;
                snapshot.action_weapons[slot + 1] = radio.weapon;
            }
        }
    }
    pub fn owned_radios(&self, actor: u8) -> impl Iterator<Item = (usize, u8)> + '_ {
        let state = &self.actors[usize::from(actor)];
        state
            .display()
            .ids
            .into_iter()
            .enumerate()
            .filter(|(_, id)| self.available & (1u64 << id) != 0 && state.inventory.contains(*id))
    }
    pub fn life(&self, actor: u8) -> u32 {
        self.actors[usize::from(actor)].death
    }
    pub fn pickup_reward(&mut self, actor: u8, id: u8, time: u32) -> bool {
        if actor >= 12
            || !CATALOG
                .rewards
                .iter()
                .any(|r| r.native_id == id && r.cost_c4.is_some_and(|cost| cost > 0))
        {
            return false;
        }
        let state = &mut self.actors[usize::from(actor)];
        if !state.alive || !state.inventory.grant(actor, id, state.death) {
            return false;
        }
        self.radio_events
            .push_back(RadioEvent::Earned { actor, id, time });
        true
    }
    pub fn can_radio(&self, actor: u8, team: u8, id: u8, now: u32) -> bool {
        if !self.radios_enabled
            || actor >= 12
            || id >= 64
            || self.available & (1u64 << id) == 0
            || !matches!(team, 1 | 2)
            || self.emp_active(team, now)
        {
            return false;
        }
        let state = &self.actors[usize::from(actor)];
        state.alive && state.inventory.contains(id)
    }
    pub fn reserve_radio(
        &mut self,
        actor: u8,
        team: u8,
        id: u8,
        now: u32,
    ) -> Option<crate::game::rewards::inventory::Ticket> {
        if !self.can_radio(actor, team, id, now) {
            return None;
        }
        let state = &mut self.actors[usize::from(actor)];
        state.inventory.reserve(actor, id, state.death)
    }
    pub fn cancel_radio(
        &mut self,
        actor: u8,
        ticket: crate::game::rewards::inventory::Ticket,
    ) -> bool {
        self.actors
            .get_mut(usize::from(actor))
            .is_some_and(|s| s.inventory.cancel(ticket))
    }
    pub fn try_radio(&mut self, actor: u8, team: u8, id: u8, now: u32) -> bool {
        let Some(ticket) = self.reserve_radio(actor, team, id, now) else {
            return false;
        };
        if self.commit_radio(actor, team, ticket, now) {
            return true;
        }
        self.cancel_radio(actor, ticket);
        false
    }
    pub fn commit_radio(
        &mut self,
        actor: u8,
        team: u8,
        ticket: crate::game::rewards::inventory::Ticket,
        now: u32,
    ) -> bool {
        let id = ticket.native_id();
        if !self.can_radio(actor, team, id, now) {
            return false;
        }
        let state = &mut self.actors[usize::from(actor)];
        if !state
            .inventory
            .commit(ticket, actor, state.death, state.alive)
        {
            return false;
        }
        self.support.retain(|s| s.end > now);
        if id == 14 {
            self.support
                .retain(|s| s.team == team || !matches!(s.id, 1 | 19));
        }
        if matches!(id, 1 | 3 | 14 | 19) {
            self.support.push(Support {
                actor,
                start: now,
                team,
                id,
                end: now.saturating_add(if id == 14 { 60_000 } else { RADAR_MSEC }),
            });
        }
        self.radio_events.push_back(RadioEvent::Called {
            actor,
            id,
            team,
            time: now,
        });
        true
    }
    pub fn support_flights(&self, now: u32) -> Vec<SupportFlight> {
        self.support
            .iter()
            .filter(|s| s.end > now && matches!(s.id, 1 | 3))
            .map(|s| SupportFlight {
                actor: s.actor,
                id: s.id,
                team: s.team,
                start: s.start,
                end: s.end,
            })
            .collect()
    }
    pub fn stop_flight(&mut self, source: SupportFlight) {
        let before = self.support.len();
        self.support.retain(|s| {
            !(s.actor == source.actor
                && s.id == source.id
                && s.team == source.team
                && s.start == source.start
                && s.end == source.end)
        });
        if self.support.len() < before {
            tracing::info!(
                actor = source.actor,
                id = source.id,
                team = source.team,
                start = source.start,
                end = source.end,
                remaining = self.support.len(),
                "aircraft support removed"
            );
        }
    }
    pub fn radar_flags(&self, team: u8, now: u32) -> u8 {
        let uavs = self
            .support
            .iter()
            .filter(|s| s.team == team && s.id == 1 && s.end > now)
            .count();
        let blocked = self
            .support
            .iter()
            .any(|s| s.team != team && matches!(s.id, 3 | 14) && s.end > now);
        // Native PS+0x218: enable bit0, blocked bit1, scan mode bits2..3.
        u8::from(uavs > 0 || self.sr71_active(team, now))
            | if self.sr71_active(team, now) {
                12
            } else if uavs > 1 {
                8
            } else if uavs == 1 {
                4
            } else {
                0
            }
            | (u8::from(blocked) << 1)
    }
    pub fn drain_radios(&mut self) -> Vec<RadioEvent> {
        self.radio_events.drain(..).collect()
    }
    pub fn acceptance_seed(&mut self, count: u8) {
        assert!(count < 25);
        if count != 0 {
            self.acceptance_seed = count;
            self.actors[0].count = u16::from(count);
            self.actors[0].record_count = u16::from(count);
            tracing::warn!(
                count,
                "diagnostic nuclear streak starting progress; not natural kills"
            );
        }
    }
    pub fn enable(&mut self) {
        self.enabled = true;
    }
    pub fn spawn(&mut self, actor: u8) {
        self.actors[usize::from(actor)].alive = true;
        if actor == 0 && self.acceptance_seed != 0 {
            self.actors[0].count = u16::from(self.acceptance_seed);
            self.actors[0].record_count = u16::from(self.acceptance_seed);
            tracing::warn!(
                count = self.acceptance_seed,
                "diagnostic nuclear starting progress on new life; not natural kills"
            );
        }
    }
    pub fn credit(&mut self, time: u32, victim: u8, death: u32, attacker: Option<u8>) {
        self.credit_kill(time, victim, death, attacker, true);
    }
    pub fn credit_reward(&mut self, time: u32, victim: u8, death: u32, attacker: Option<u8>) {
        self.credit_kill(time, victim, death, attacker, false);
    }
    fn credit_kill(
        &mut self,
        time: u32,
        victim: u8,
        death: u32,
        attacker: Option<u8>,
        earns_rewards: bool,
    ) {
        let victim_state = &mut self.actors[usize::from(victim)];
        if death <= victim_state.death {
            return;
        }
        victim_state.death = death;
        victim_state.count = 0;
        victim_state.record_count = 0;
        victim_state.alive = false;
        victim_state.granted = false;
        victim_state.radios_granted = [false; 3];
        victim_state.inventory.invalidate_use();
        let Some(attacker) = attacker.filter(|a| *a != victim) else {
            return;
        };
        let actor = &mut self.actors[usize::from(attacker)];
        if !actor.alive {
            return;
        }
        actor.record_count = actor.record_count.saturating_add(1);
        actor.best = actor.best.max(actor.record_count);
        if !earns_rewards {
            return;
        }
        actor.count = actor.count.saturating_add(1);
        let progress = actor.hud_progress();
        if self.radios_enabled && progress == 1 {
            actor.radios_granted = [false; 3];
            actor.inventory.reset_natural_round(actor.death);
        }
        if self.radios_enabled {
            for slot in 0..3 {
                let id = actor.previews.ids[slot];
                if id < 64
                    && self.available & (1u64 << id) != 0
                    && progress >= actor.previews.targets[slot]
                    && !actor.radios_granted[slot]
                {
                    actor.radios_granted[slot] = true;
                    if actor.inventory.grant_natural(attacker, id, actor.death) {
                        self.radio_events.push_back(RadioEvent::Earned {
                            actor: attacker,
                            id,
                            time,
                        });
                    }
                }
            }
        }
        if self.enabled && actor.count >= NUKE_KILLS && !actor.granted {
            actor.granted = true;
            if !actor.owned {
                actor.owned = true;
                self.events.push_back(Event::Earned {
                    actor: attacker,
                    time,
                });
            }
        }
    }
    pub fn owned(&self, actor: u8) -> bool {
        self.actors[usize::from(actor)].owned
    }
    pub fn best(&self, actor: u8) -> u16 {
        self.actors[usize::from(actor)].best
    }
    pub fn snapshot(&self, actor: u8, weapon: u16) -> StreakSnapshot {
        let actor = &self.actors[usize::from(actor)];
        let mut result = StreakSnapshot {
            progress: actor.hud_progress(),
            ..Default::default()
        };
        if actor.owned && actor.alive && weapon != 0 {
            // HUD items 90..95 use ID0 and actionslot 1 for extra rewards.
            // Slot 5's material is the saved acestreak, not the ID4 table icon.
            result.ids[0] = 15;
            result.targets[0] = 25;
            result.action_types[0] = 1;
            result.action_weapons[0] = weapon;
        }
        result
    }
    pub fn try_use(&mut self, actor: u8, time: u32) -> bool {
        let state = &mut self.actors[usize::from(actor)];
        if !self.enabled || !state.alive || !state.owned || self.incoming.is_some() {
            return false;
        }
        state.owned = false;
        self.incoming = Some(Incoming {
            actor,
            start: time,
            exploded: false,
            vision_started: false,
            ended: false,
        });
        self.events.push_back(Event::Called { actor, time });
        true
    }
    pub fn advance(&mut self, time: u32) {
        let Some(incoming) = &mut self.incoming else {
            return;
        };
        let elapsed = time.saturating_sub(incoming.start);
        if elapsed >= COUNTDOWN_MSEC && !incoming.exploded {
            incoming.exploded = true;
            self.events.push_back(Event::Explosion {
                actor: incoming.actor,
                time: incoming.start + COUNTDOWN_MSEC,
            });
        }
        if elapsed >= COUNTDOWN_MSEC + 250 && !incoming.vision_started {
            incoming.vision_started = true;
            self.events.push_back(Event::Vision {
                actor: incoming.actor,
                time: incoming.start + COUNTDOWN_MSEC + 250,
            });
        }
        if elapsed >= COUNTDOWN_MSEC + AFTER_FLASH_MSEC && !incoming.ended {
            incoming.ended = true;
            self.events.push_back(Event::End {
                actor: incoming.actor,
                time: incoming.start + COUNTDOWN_MSEC + AFTER_FLASH_MSEC,
            });
        }
    }
    pub fn countdown_end(&self) -> Option<u32> {
        self.incoming
            .filter(|i| !i.exploded)
            .map(|i| i.start + COUNTDOWN_MSEC)
    }
    pub fn incoming(&self) -> bool {
        self.incoming.is_some()
    }
    pub fn drain(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
    pub fn clear_owned(&mut self) {
        for actor in &mut self.actors {
            actor.owned = false;
            actor.inventory.clear();
        }
        self.support.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn radar_resources() -> RadarResources {
        let timing = test_resources();
        RadarResources {
            schema_version: 1,
            client_sha256: codol_protocol::CLIENT_SHA256.into(),
            sources_sha256: ["0".repeat(64), "1".repeat(64)],
            map: "mp_afghan".into(),
            ncs_crc: 123,
            models: Default::default(),
            tags: Default::default(),
            vehicles: Default::default(),
            animations: Default::default(),
            loop_sounds: Default::default(),
            model_bounds: Default::default(),
            camera_mounts: Default::default(),
            effects: Default::default(),
            selectors: Default::default(),
            weapons: Default::default(),
            air_route: None,
            air_paths: None,
            remote_spawns: None,
            remote_vision: None,
            remote_exit: None,
            animals: None,
            radios: vec![
                Radio {
                    native_id: 1,
                    weapon: 301,
                    switch: timing.switch,
                    fire_interval_msec: 800,
                },
                Radio {
                    native_id: 3,
                    weapon: 302,
                    switch: timing.switch,
                    fire_interval_msec: 800,
                },
            ],
        }
    }
    #[test]
    fn ordinary_rewards_use_real_death_thresholds_and_independent_hud_and_inventory() {
        let mut s = Streaks::default();
        s.enable();
        s.configure_radios(
            0,
            Previews {
                ids: [1, 3, 8],
                targets: [3, 4, 7],
            },
        );
        let r = radar_resources();
        for n in 1..=2 {
            kill(&mut s, n);
        }
        assert!(s.owned_radios(0).next().is_none());
        kill(&mut s, 3);
        s.credit(3001, 1, 3, Some(0));
        assert_eq!(s.owned_radios(0).collect::<Vec<_>>(), [(0, 1)]);
        kill(&mut s, 4);
        let mut hud = s.snapshot(0, 323);
        s.radio_snapshot(0, &r, &mut hud);
        assert_eq!(hud.ids, [0, 1, 3, 8, 0]);
        assert_eq!(hud.action_weapons, [0, 301, 302, 0, 0]);
        assert!(s.try_radio(0, 1, 1, 4100));
        assert!(!s.try_radio(0, 1, 1, 4101));
        for n in 5..=25 {
            kill(&mut s, n);
        }
        assert!(s.owned(0));
        assert_eq!(s.owned_radios(0).collect::<Vec<_>>(), [(0, 1), (1, 3)]);
        let mut hud = s.snapshot(0, 323);
        s.radio_snapshot(0, &r, &mut hud);
        assert_eq!(hud.action_weapons, [323, 301, 302, 0, 0]);
        assert!(s.try_radio(0, 1, 3, 26000));
        assert!(s.try_use(0, 26000));
        assert!(!s.try_radio(0, 1, 8, 26000));
        assert_eq!(
            s.drain_radios()
                .iter()
                .filter(|e| matches!(e, RadioEvent::Earned { .. }))
                .count(),
            3
        );
    }
    #[test]
    fn three_four_five_rewards_share_one_life_count_and_all_are_owned_at_five() {
        for use_first_reward in [false, true] {
            let mut s = Streaks::default();
            let mut r = radar_resources();
            let mut third = r.radios[0].clone();
            third.native_id = 19;
            third.weapon = 336;
            r.radios.push(third);
            s.enable_supported(&r);
            s.configure_radios(
                0,
                Previews {
                    ids: [1, 3, 19],
                    targets: [3, 4, 5],
                },
            );
            for n in 1..=5 {
                kill(&mut s, n);
                let mut hud = s.snapshot(0, 0);
                s.radio_snapshot(0, &r, &mut hud);
                assert_eq!(hud.progress, n as u8);
                assert_eq!(hud.targets, [0, 3, 4, 5, 0]);
                let mut expected = [0, u8::from(n >= 3), u8::from(n >= 4), u8::from(n >= 5), 0];
                if use_first_reward && n > 3 {
                    expected[1] = 0;
                }
                assert_eq!(
                    hud.action_types, expected,
                    "kill {n}, use_first={use_first_reward}"
                );
                if use_first_reward && n == 3 {
                    assert!(s.try_radio(0, 1, 1, 3100));
                    assert_eq!(s.snapshot(0, 0).progress, 3);
                }
            }
            let earned = s
                .drain_radios()
                .into_iter()
                .filter_map(|e| match e {
                    RadioEvent::Earned { id, time, .. } => Some((id, time)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(earned, [(1, 3000), (3, 4000), (19, 5000)]);
            assert_eq!(s.actors[0].count, 5);
            assert!(s.actors[0].inventory.contains(3));
            assert!(s.actors[0].inventory.contains(19));
            assert_eq!(s.actors[0].inventory.contains(1), !use_first_reward);
        }
    }
    fn configure_three_four_five(s: &mut Streaks) {
        let mut r = radar_resources();
        let mut third = r.radios[0].clone();
        third.native_id = 19;
        third.weapon = 336;
        r.radios.push(third);
        s.enable_supported(&r);
        s.configure_radios(
            0,
            Previews {
                ids: [1, 3, 19],
                targets: [3, 4, 5],
            },
        );
    }
    #[test]
    fn three_four_five_repeats_at_eight_nine_ten_without_death() {
        let mut s = Streaks::default();
        configure_three_four_five(&mut s);
        let mut earned = Vec::new();
        for n in 1..=15 {
            kill(&mut s, n);
            let progress = ((n - 1) % 5 + 1) as u8;
            assert_eq!(s.snapshot(0, 0).progress, progress);
            let id = match progress {
                3 => Some(1),
                4 => Some(3),
                5 => Some(19),
                _ => None,
            };
            if let Some(id) = id {
                assert!(s.try_radio(0, 1, id, n * 1000 + 1));
                assert!(!s.try_radio(0, 1, id, n * 1000 + 2));
            }
            earned.extend(s.drain_radios().into_iter().filter_map(|e| match e {
                RadioEvent::Earned { id, time, .. } => Some((id, time)),
                _ => None,
            }));
        }
        assert_eq!(
            earned,
            [
                (1, 3000),
                (3, 4000),
                (19, 5000),
                (1, 8000),
                (3, 9000),
                (19, 10000),
                (1, 13000),
                (3, 14000),
                (19, 15000)
            ]
        );
        assert_eq!(s.actors[0].count, 15);
        assert_eq!(s.life(0), 0);
    }
    #[test]
    fn unspent_skills_do_not_stack_across_rounds_or_backfill_after_use() {
        let mut s = Streaks::default();
        configure_three_four_five(&mut s);
        for n in 1..=8 {
            kill(&mut s, n);
        }
        assert_eq!(
            s.drain_radios()
                .iter()
                .filter(|e| matches!(e, RadioEvent::Earned { .. }))
                .count(),
            3
        );
        // Round2's A threshold was already reached while A was still owned.
        assert!(s.try_radio(0, 1, 1, 8100));
        s.configure_radios(
            0,
            Previews {
                ids: [3, 1, 19],
                targets: [4, 3, 5],
            },
        );
        kill(&mut s, 9);
        kill(&mut s, 10);
        assert!(!s.try_radio(0, 1, 1, 10100));
        for n in 11..=13 {
            kill(&mut s, n);
        }
        assert!(s.try_radio(0, 1, 1, 13100));
        assert!(!s.try_radio(0, 1, 1, 13101));
        assert!(s.try_radio(0, 1, 3, 13200));
        assert!(!s.try_radio(0, 1, 3, 13201));
    }
    #[test]
    fn reward_kills_only_change_the_record_not_reward_progress_or_nuke() {
        let mut s = Streaks::default();
        s.enable();
        configure_three_four_five(&mut s);
        kill(&mut s, 1);
        kill(&mut s, 2);
        for death in 3..=30 {
            s.credit_reward(death * 1000, 1, death, Some(0));
        }
        assert_eq!(s.snapshot(0, 323).progress, 2);
        assert_eq!(s.actors[0].count, 2);
        assert_eq!(s.best(0), 30);
        assert!(!s.owned(0));
        assert!(s.owned_radios(0).next().is_none());
        kill(&mut s, 31);
        assert_eq!(s.snapshot(0, 323).progress, 3);
        assert!(s.actors[0].inventory.contains(1));
        for death in 32..=53 {
            kill(&mut s, death);
        }
        assert_eq!(s.actors[0].count, 25);
        assert!(s.owned(0));
        assert_eq!(s.best(0), 53);
    }
    #[test]
    fn highest_selected_cost_cycles_the_hud_without_resetting_life_count_or_nuke() {
        let mut s = Streaks::default();
        s.enable();
        s.configure_radios(
            0,
            Previews {
                ids: [1, 3, 19],
                targets: [3, 4, 5],
            },
        );
        // An unavailable handler still belongs to the selected progression
        // span; readiness must not shorten it to the last available reward.
        for n in 1..=25 {
            kill(&mut s, n);
        }
        assert_eq!(s.snapshot(0, 323).progress, 5);
        assert_eq!(s.actors[0].count, 25);
        assert_eq!(s.best(0), 25);
        assert!(s.owned(0));
        assert!(s.try_use(0, 26000));
        assert_eq!(s.snapshot(0, 323).progress, 5);
        s.credit(27000, 0, 1, None);
        assert_eq!(s.snapshot(0, 323).progress, 0);
        assert_eq!(s.best(0), 25);
        s.spawn(0);
        kill(&mut s, 26);
        assert_eq!(s.snapshot(0, 323).progress, 1);
    }
    #[test]
    fn team_support_stacks_expires_survives_death_and_counters_only_opponents() {
        let mut s = Streaks::default();
        for actor in 0..3 {
            s.configure_radios(
                actor,
                Previews {
                    ids: [1, 3, 0],
                    targets: [3, 4, 0],
                },
            );
            let state = &mut s.actors[usize::from(actor)];
            assert!(state.inventory.grant(actor, 1, state.death));
            assert!(state.inventory.grant(actor, 3, state.death));
        }
        assert!(s.try_radio(0, 1, 1, 1000));
        assert_eq!(s.radar_flags(1, 1000), 5);
        assert_eq!(s.radar_flags(2, 1000), 0);
        assert!(s.try_radio(1, 1, 1, 2000));
        assert_eq!(s.radar_flags(1, 2000), 9);
        assert!(s.try_radio(2, 2, 3, 3000));
        assert_eq!(s.radar_flags(1, 3000), 11);
        assert_eq!(s.radar_flags(2, 3000), 0);
        s.credit(4000, 2, 1, None);
        assert_eq!(s.radar_flags(1, 4000), 11);
        assert_eq!(s.radar_flags(1, 31000), 7);
        assert_eq!(s.radar_flags(1, 32000), 2);
        assert_eq!(s.radar_flags(1, 33000), 0);
        s.clear_owned();
        assert_eq!(s.radar_flags(1, 35000), 0);
    }
    #[test]
    fn destroyed_aircraft_removes_its_call_without_clearing_other_owned_support() {
        let mut s = Streaks::default();
        for actor in [0, 1, 6] {
            s.configure_radios(
                actor,
                Previews {
                    ids: [1, 3, 0],
                    targets: [3, 4, 0],
                },
            );
            let state = &mut s.actors[usize::from(actor)];
            assert!(state.inventory.grant(actor, 1, state.death));
            assert!(state.inventory.grant(actor, 3, state.death));
        }
        assert!(s.try_radio(0, 1, 1, 1000));
        assert!(s.try_radio(1, 1, 1, 2000));
        assert!(s.try_radio(6, 2, 3, 3000));
        let calls = s.support_flights(3000);
        assert_eq!(calls.len(), 3);
        assert_eq!(s.radar_flags(1, 3000), 11);
        let first = *calls.iter().find(|c| c.actor == 0 && c.id == 1).unwrap();
        s.stop_flight(first);
        assert_eq!(s.radar_flags(1, 3000), 7);
        s.stop_flight(first);
        assert_eq!(s.support_flights(3000).len(), 2);
        let counter = *calls.iter().find(|c| c.id == 3).unwrap();
        s.stop_flight(counter);
        assert_eq!(s.radar_flags(1, 3000), 5);
        assert_eq!(s.owned_radios(0).collect::<Vec<_>>(), [(1, 3)]);
        assert_eq!(s.radar_flags(1, 32000), 0);
    }
    #[test]
    fn death_preserves_earned_radios_but_resets_threshold_and_new_life_awards() {
        let mut s = Streaks::default();
        let previews = Previews {
            ids: [1, 3, 0],
            targets: [2, 3, 0],
        };
        s.configure_radios(0, previews);
        for n in 1..=3 {
            kill(&mut s, n);
        }
        s.credit(4000, 0, 1, None);
        assert!(!s.try_radio(0, 1, 1, 4000));
        let mut hud = s.snapshot(0, 0);
        s.radio_snapshot(0, &radar_resources(), &mut hud);
        assert_eq!(hud.action_types, [0; 5]);
        s.spawn(0);
        s.configure_radios(0, previews);
        assert!(s.try_radio(0, 1, 1, 5000));
        kill(&mut s, 4);
        assert!(!s.try_radio(0, 1, 1, 6000));
        kill(&mut s, 5);
        assert!(s.try_radio(0, 1, 1, 7000));
        s.credit(8000, 0, 2, None);
        s.spawn(0);
        s.configure_radios(
            0,
            Previews {
                ids: [3, 8, 0],
                targets: [4, 7, 0],
            },
        );
        assert_eq!(s.owned_radios(0).collect::<Vec<_>>(), [(0, 3)]);
    }
    #[test]
    fn repeated_rounds_and_class_changes_keep_at_most_one_stock_per_skill() {
        let mut s = Streaks::default();
        let preview = Previews {
            ids: [1, 0, 0],
            targets: [2, 0, 0],
        };
        s.configure_radios(0, preview);
        kill(&mut s, 1);
        kill(&mut s, 2);
        s.configure_radios(
            0,
            Previews {
                ids: [3, 0, 0],
                targets: [4, 0, 0],
            },
        );
        s.configure_radios(0, preview);
        kill(&mut s, 3);
        assert!(s.try_radio(0, 1, 1, 3500));
        assert!(!s.try_radio(0, 1, 1, 3501));
        s.credit(4000, 0, 1, None);
        s.spawn(0);
        kill(&mut s, 4);
        kill(&mut s, 5);
        s.credit(6000, 0, 2, None);
        s.spawn(0);
        kill(&mut s, 6);
        kill(&mut s, 7);
        assert!(s.try_radio(0, 1, 1, 8000));
        assert!(!s.try_radio(0, 1, 1, 8001));
        assert!(!s.try_radio(0, 1, 1, 8002));
        assert_eq!(s.support_flights(8002).len(), 2);
    }
    #[test]
    fn deferred_radio_permission_is_revoked_by_death_without_losing_the_stock() {
        let mut s = Streaks::default();
        s.configure_radios(
            0,
            Previews {
                ids: [1, 3, 0],
                targets: [2, 3, 0],
            },
        );
        kill(&mut s, 1);
        kill(&mut s, 2);
        let pending = s.reserve_radio(0, 1, 1, 2500).unwrap();
        kill(&mut s, 3);
        assert!(s.commit_radio(0, 1, pending, 3500));
        assert!(!s.commit_radio(0, 1, pending, 3501));
        let pending = s.reserve_radio(0, 1, 3, 3600).unwrap();
        s.credit(4000, 0, 1, None);
        s.spawn(0);
        assert!(!s.commit_radio(0, 1, pending, 4500));
        assert!(s.try_radio(0, 1, 3, 4501));
        assert!(!s.cancel_radio(0, pending));
    }
    #[test]
    fn pickup_overflow_queues_stock_and_does_not_revoke_a_hidden_pending_instance() {
        let mut s = Streaks::default();
        s.configure_radios(
            0,
            Previews {
                ids: [1, 3, 19],
                targets: [3, 4, 8],
            },
        );
        s.available = [1, 3, 16, 19]
            .into_iter()
            .fold(0, |mask, id| mask | (1 << id));
        for id in [1, 3, 19] {
            assert!(s.pickup_reward(0, id, 1000));
        }
        let pending = s.reserve_radio(0, 1, 1, 1000).unwrap();
        assert!(s.pickup_reward(0, 16, 1100));
        let visible = s.owned_radios(0).map(|(_, id)| id).collect::<BTreeSet<_>>();
        assert_eq!(visible, BTreeSet::from([3, 16, 19]));
        assert!(s.commit_radio(0, 1, pending, 1200));
        assert!(!s.commit_radio(0, 1, pending, 1201));
        // Preserve a real unfinished pool reward as stock, without publishing
        // an activator or borrowing another reward's effect handler.
        assert!(s.pickup_reward(0, 5, 1300));
        let mut r = radar_resources();
        let mut activator = r.radios[0].clone();
        activator.native_id = 5;
        activator.weapon = 325;
        r.radios.push(activator);
        let mut hud = s.snapshot(0, 0);
        s.radio_snapshot(0, &r, &mut hud);
        let slot = hud.ids.iter().position(|&id| id == 5).unwrap();
        assert_eq!(hud.action_types[slot], 0);
        assert!(!s.owned_radios(0).any(|(_, id)| id == 5));
        assert!(s.actors[0].inventory.contains(5));
    }
    #[test]
    fn radar_bindings_reject_wrong_map_crc_duplicates_and_bad_timing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("radar.json");
        let mut json = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "sources_sha256":["0".repeat(64),"1".repeat(64)],"map":"mp_afghan","ncs_crc":123,
            "radios":[{"native_id":1,"weapon":301,"switch":{"drop_msec":1000,"raise_msec":833,"empty_drop_msec":1000,"empty_raise_msec":833},"fire_interval_msec":800},
                      {"native_id":3,"weapon":302,"switch":{"drop_msec":1000,"raise_msec":833,"empty_drop_msec":1000,"empty_raise_msec":833},"fire_interval_msec":800}]});
        let write =
            |j: &serde_json::Value| std::fs::write(&path, serde_json::to_vec(j).unwrap()).unwrap();
        write(&json);
        assert!(RadarResources::load(&path, "mp_afghan", 123).is_ok());
        assert!(RadarResources::load(&path, "mp_rust", 123).is_err());
        assert!(RadarResources::load(&path, "mp_afghan", 124).is_err());
        json["remote_exit"] =
            serde_json::json!({"white":1000,"grain":1001,"material_bank_count":27});
        write(&json);
        assert!(RadarResources::load(&path, "mp_afghan", 123).is_ok());
        for (field, value) in [("white", 1), ("grain", 1000), ("material_bank_count", 1000)] {
            let mut bad = json.clone();
            bad["remote_exit"][field] = value.into();
            write(&bad);
            assert!(RadarResources::load(&path, "mp_afghan", 123).is_err());
        }
        json.as_object_mut().unwrap().remove("remote_exit");

        json["radios"][1]["weapon"] = 301.into();
        write(&json);
        assert!(RadarResources::load(&path, "mp_afghan", 123).is_err());
        json["radios"][1]["weapon"] = 302.into();
        json["radios"][0]["switch"]["raise_msec"] = 0.into();
        write(&json);
        assert!(RadarResources::load(&path, "mp_afghan", 123).is_err());
    }
    #[test]
    fn sr71_and_emp_have_independent_team_lifetimes_and_block_without_consuming_owned_rewards() {
        let mut s = Streaks::default();
        let mut r = radar_resources();
        for (id, weapon) in [(14, 315), (19, 336)] {
            r.radios.push(Radio {
                native_id: id,
                weapon,
                switch: test_resources().switch,
                fire_interval_msec: 800,
            });
        }
        s.enable_supported(&r);
        s.configure_radios(
            0,
            Previews {
                ids: [1, 19, 14],
                targets: [3, 8, 15],
            },
        );
        s.configure_radios(
            2,
            Previews {
                ids: [1, 19, 14],
                targets: [3, 8, 15],
            },
        );
        for actor in [0, 2] {
            let state = &mut s.actors[usize::from(actor)];
            for id in [1, 19, 14] {
                assert!(state.inventory.grant(actor, id, state.death));
            }
        }
        assert!(s.try_radio(0, 1, 19, 1000));
        assert_eq!(s.radar_flags(1, 1000), 13);
        assert!(s.sr71_active(1, 1000));
        assert!(s.try_radio(2, 2, 14, 2000));
        assert!(s.emp_active(1, 2000));
        assert_eq!(s.radar_flags(1, 2000), 2);
        assert!(!s.sr71_active(1, 2000));
        assert!(!s.try_radio(0, 1, 1, 3000));
        assert_eq!(s.owned_radios(0).collect::<Vec<_>>(), [(0, 1), (2, 14)]);
        s.credit(5000, 2, 1, None);
        assert!(s.emp_active(1, 61999));
        assert!(!s.emp_active(1, 62000));
        assert_eq!(s.radar_flags(1, 62000), 0);
        assert!(s.try_radio(0, 1, 1, 62000));
        assert_eq!(s.radar_flags(1, 62000), 5);
    }
    #[test]
    fn nuclear_called_text_has_a_literal_caller_argument_and_cannot_split_commands() {
        let command = called_message("A\"B\\C\n\u{15}");
        assert_eq!(command, b"e \"\x14MP_TACTICAL_NUKE_CALLED\x15\x16A'B/C\"");
        assert_eq!(
            called_message(""),
            b"e \"\x14MP_TACTICAL_NUKE_CALLED\x15\x16Player\""
        );
    }
    #[test]
    fn seeded_and_stationary_resources_require_the_diagnostic_marker() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nuke.json");
        let mut resource = serde_json::json!({
            "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "sources_sha256":["0".repeat(64),"1".repeat(64)],"map":"mp_afghan","ncs_crc":123,
            "weapon":323,"damage_weapon":334,"switch":{"drop_msec":1000,"raise_msec":833,
                "empty_drop_msec":1000,"empty_raise_msec":833},"fire_interval_msec":800,
            "flash_fx":77,"blast_fx":76,"aftermath_fx":61,"tick_sound":1986,"music_sound":1976,
            "earned_text":448,"called_text":556
        });
        let write = |value: &serde_json::Value| {
            std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap()
        };
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_ok());
        assert!(NukeResources::load(&path, "mp_afghan", 124).is_err());
        assert!(NukeResources::load(&path, "mp_nuked", 123).is_err());
        for flag in [
            "acceptance_remote_vehicles",
            "acceptance_missile_drone",
            "acceptance_gunners",
            "acceptance_dogs",
            "acceptance_armor",
            "acceptance_kingpin_stock",
        ] {
            resource[flag] = true.into();
            write(&resource);
            assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
            resource.as_object_mut().unwrap().remove(flag);
        }
        resource["acceptance_attack_aircraft"] = true.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
        resource["acceptance_stationary"] = true.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
        std::fs::write(directory.path().join(".codol-nuke-acceptance"), b"").unwrap();
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_ok());
        resource["acceptance_remote_missile"] = true.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_ok());
        resource["acceptance_stationary"] = false.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
        resource["acceptance_stationary"] = true.into();
        resource["acceptance_remote_missile"] = false.into();
        resource["acceptance_attack_aircraft"] = false.into();
        resource["acceptance_enemy_support"] =
            serde_json::json!([{"after_msec":2000,"native_id":1}]);
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_ok());
        resource["acceptance_enemy_support"][0]["native_id"] = 14.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_ok());
        resource["acceptance_enemy_support"][0]["native_id"] = 23.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
        resource["acceptance_enemy_support"][0]["native_id"] = 1.into();
        resource["acceptance_fire_after_msec"] = 4000.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_ok());
        resource["acceptance_stationary"] = false.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
        resource
            .as_object_mut()
            .unwrap()
            .remove("acceptance_fire_after_msec");
        resource
            .as_object_mut()
            .unwrap()
            .remove("acceptance_enemy_support");
        resource["acceptance_seed"] = 24.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_ok());
        std::fs::remove_file(directory.path().join(".codol-nuke-acceptance")).unwrap();
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
        std::fs::write(directory.path().join(".codol-nuke-acceptance"), b"").unwrap();
        resource["acceptance_seed"] = 25.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
    }
    fn kill(streaks: &mut Streaks, n: u32) {
        streaks.spawn(1);
        streaks.credit(n * 1000, 1, n, Some(0));
    }
    #[test]
    fn saved_previews_use_native_ids_sorted_costs_and_keep_extra_slots_independent() {
        let mut profile = vec![0; codol_protocol::lobby::profile::SNAPSHOT_BYTES];
        // Class 59 has independently captured offsets, unlike the early stride.
        for (offset, item) in CATALOG.class_offsets[59][..3]
            .iter()
            .zip([60084i32, 60077, 60081])
        {
            profile[*offset..*offset + 4].copy_from_slice(&item.to_le_bytes());
        }
        let previews = Previews::from_profile(&profile, 59, &[60076, 60080, 60083], -1.0).unwrap();
        let mut s = Streaks::default();
        s.enable();
        for n in 1..=25 {
            kill(&mut s, n);
        }
        let mut snapshot = s.snapshot(0, 323);
        previews.apply(&mut snapshot);
        assert_eq!(snapshot.ids, [15, 1, 5, 8, 0]);
        assert_eq!(snapshot.targets, [25, 2, 4, 6, 0]);
        assert_eq!(snapshot.action_types, [1, 0, 0, 0, 0]);
        assert_eq!(snapshot.action_weapons, [323, 0, 0, 0, 0]);
        assert!(Previews::from_profile(&profile, 59, &[60076], 0.0).is_err());
        assert!(Previews::from_profile(&profile[..100], 59, &[], 0.0).is_err());
        assert!(Previews::from_profile(&profile, 60, &[], 0.0).is_err());
        s.credit(26000, 0, 1, None);
        let mut dead = s.snapshot(0, 323);
        previews.apply(&mut dead);
        assert_eq!(dead.ids, [0, 1, 5, 8, 0]);
        assert_eq!(dead.progress, 0);
        assert_eq!(dead.action_types, [0; 5]);
        s.spawn(0);
        assert_eq!(s.snapshot(0, 323).ids[0], 15);
        assert!(s.try_use(0, 27000));
        assert_eq!(s.snapshot(0, 323).ids, [0; 5]);
    }
    #[test]
    fn unmatched_pro_items_are_not_aliased_and_empty_profile_clears_previews() {
        let mut profile = vec![0; codol_protocol::lobby::profile::SNAPSHOT_BYTES];
        let offset = CATALOG.class_offsets[0][0];
        profile[offset..offset + 4].copy_from_slice(&60098i32.to_le_bytes());
        assert_eq!(
            Previews::from_profile(&profile, 0, &[60097], 0.0).unwrap(),
            Previews::default()
        );
        profile[offset..offset + 4].copy_from_slice(&1i32.to_le_bytes());
        assert_eq!(
            Previews::from_profile(&profile, 0, &[], 0.0).unwrap(),
            Previews::default()
        );
    }
    #[test]
    fn twenty_five_real_deaths_award_once_and_retries_do_not_advance() {
        let mut s = Streaks::default();
        s.enable();
        for i in 1..=24 {
            kill(&mut s, i);
        }
        assert!(!s.owned(0));
        s.credit(24001, 1, 24, Some(0));
        assert_eq!(s.snapshot(0, 322).progress, 24);
        kill(&mut s, 25);
        assert!(s.owned(0));
        for i in 26..=50 {
            kill(&mut s, i);
        }
        assert_eq!(
            s.drain(),
            [Event::Earned {
                actor: 0,
                time: 25000
            }]
        );
        assert_eq!(s.best(0), 50);
    }
    #[test]
    fn suicide_world_death_reset_progress_and_dead_attackers_cannot_earn() {
        let mut s = Streaks::default();
        s.enable();
        kill(&mut s, 1);
        s.credit(2000, 0, 1, Some(0));
        assert_eq!(s.snapshot(0, 322).progress, 0);
        kill(&mut s, 2);
        assert_eq!(s.snapshot(0, 322).progress, 0);
        s.spawn(0);
        kill(&mut s, 3);
        s.credit(4000, 0, 2, None);
        assert_eq!(s.snapshot(0, 322).progress, 0);
    }
    #[test]
    fn owned_reward_survives_death_and_singleton_countdown_survives_caller_death() {
        let mut s = Streaks::default();
        s.enable();
        for i in 1..=25 {
            kill(&mut s, i);
        }
        s.credit(26000, 0, 1, None);
        assert!(s.owned(0));
        assert!(!s.try_use(0, 27000));
        assert_eq!(s.snapshot(0, 322).action_types, [0; 5]);
        s.spawn(0);
        assert!(s.try_use(0, 30000));
        assert!(!s.try_use(0, 30001));
        s.credit(31000, 0, 2, None);
        s.drain();
        s.advance(39999);
        assert!(s.drain().is_empty());
        s.advance(40000);
        assert_eq!(
            s.drain(),
            [Event::Explosion {
                actor: 0,
                time: 40000
            }]
        );
        s.advance(40249);
        assert!(s.drain().is_empty());
        s.advance(40250);
        assert_eq!(
            s.drain(),
            [Event::Vision {
                actor: 0,
                time: 40250
            }]
        );
        s.advance(41500);
        assert_eq!(
            s.drain(),
            [Event::End {
                actor: 0,
                time: 41500
            }]
        );
        s.advance(50000);
        assert!(s.drain().is_empty());
    }
}
