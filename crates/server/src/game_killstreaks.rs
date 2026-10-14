//! Authority-owned life progress, radio support and the fixed-cost nuclear reward.
//! The 25-kill extra reward and 10s/1.5s timing follow the local Alpha/LAN
//! compatibility rules; native resources/layouts are verified independently.
use anyhow::{Result, ensure};
use codol_protocol::game_killstreaks::StreakSnapshot;
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
    cost_c4: Option<u8>,
}
#[derive(Deserialize)]
struct Item {
    item_id: u32,
    native_id: Option<u8>,
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let catalog: Catalog = serde_json::from_str(include_str!(
        "../../../fixtures/protocol/killstreaks-3.3.3.3.json"
    ))
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
    pub switch: crate::game_player::SwitchTiming,
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
    pub model_bounds: BTreeMap<String, ModelBounds>,
    #[serde(default)]
    pub effects: BTreeMap<String, u16>,
    #[serde(default)]
    pub weapons: BTreeMap<String, WeaponFacts>,
    #[serde(default)]
    pub air_route: Option<AirRoute>,
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
pub(crate) struct WeaponFacts {
    pub weapon: u16,
    pub weapon_type: u32,
    pub weapon_class: u32,
    pub clip_key: Option<u16>,
    pub ammo_key: Option<u16>,
    pub switch: crate::game_player::SwitchTiming,
    pub fire_interval_msec: u16,
    pub radius: u32,
    pub inner_damage: u32,
    pub outer_damage: u32,
    #[serde(default)]
    pub damage_stages: [u32; 6],
    #[serde(default)]
    pub damage_ranges: [f32; 6],
}
/// Effect handlers explicitly publish their implemented IDs.
pub(crate) const SUPPORT_IDS: [u8; 5] = [1, 3, 14, 16, 19];
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
                && result.models.values().all(|v| (1..1024).contains(v))
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
                && result.effects.values().all(|v| (1..400).contains(v))
                && result.weapons.values().all(|w| (1..900).contains(&w.weapon)
                    && w.weapon_type <= 6
                    && w.weapon_class <= 32
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
pub(crate) struct NukeResources {
    pub schema_version: u8,
    pub client_sha256: String,
    pub sources_sha256: Vec<String>,
    pub map: String,
    pub ncs_crc: u32,
    pub weapon: u16,
    pub damage_weapon: u16,
    pub switch: crate::game_player::SwitchTiming,
    pub fire_interval_msec: u16,
    pub flash_fx: u16,
    pub blast_fx: u16,
    pub aftermath_fx: u16,
    pub tick_sound: u16,
    pub music_sound: u16,
    pub earned_text: u16,
    pub called_text: u16,
    #[serde(default)]
    pub acceptance_seed: u8,
    #[serde(default)]
    pub acceptance_stationary: bool,
    #[serde(default)]
    pub acceptance_fire_after_msec: Option<u32>,
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
                && result.acceptance_seed <= 24
                && ((result.acceptance_seed == 0 && !result.acceptance_stationary)
                    || path
                        .parent()
                        .is_some_and(|p| p.join(".codol-nuke-acceptance").is_file())),
            "nuke diagnostics require a marked directory and seed below the award threshold"
        );
        ensure!(
            result.schema_version == 1
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
        switch: crate::game_player::SwitchTiming {
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
        acceptance_seed: 0,
        acceptance_stationary: false,
        acceptance_fire_after_msec: None,
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

#[derive(Clone, Copy)]
struct Actor {
    count: u16,
    best: u16,
    death: u32,
    alive: bool,
    granted: bool,
    owned: bool,
    previews: Previews,
    radios_owned: [bool; 3],
    radios_granted: [bool; 3],
}
impl Default for Actor {
    fn default() -> Self {
        Self {
            count: 0,
            best: 0,
            death: 0,
            alive: true,
            granted: false,
            owned: false,
            previews: Previews::default(),
            radios_owned: [false; 3],
            radios_granted: [false; 3],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Earned { actor: u8, time: u32 },
    Called { actor: u8, time: u32 },
    Explosion { actor: u8, time: u32 },
    End { actor: u8, time: u32 },
}
#[derive(Clone, Copy)]
struct Incoming {
    actor: u8,
    start: u32,
    exploded: bool,
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
        let owned =
            std::array::from_fn(|slot| {
                state.previews.ids.iter().enumerate().any(|(old, &id)| {
                    id != 0 && id == previews.ids[slot] && state.radios_owned[old]
                })
            });
        if state.previews != previews {
            state.radios_granted = [false; 3];
        }
        state.previews = previews;
        state.radios_owned = owned;
    }
    pub fn enable_supported(&mut self, resources: &RadarResources) {
        self.available = resources
            .radios
            .iter()
            .filter(|r| {
                SUPPORT_IDS.contains(&r.native_id)
                    && (r.native_id != 16
                        || crate::game_deployables::SentryDefinition::from_resources(resources)
                            .is_some())
            })
            .fold(0, |m, r| m | (1u64 << r.native_id));
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
        let state = self.actors[usize::from(actor)];
        state.previews.apply(snapshot);
        for slot in 0..3 {
            if state.alive
                && state.radios_owned[slot]
                && let Some(radio) = resources.radio(state.previews.ids[slot])
            {
                snapshot.action_types[slot + 1] = 1;
                snapshot.action_weapons[slot + 1] = radio.weapon;
            }
        }
    }
    pub fn owned_radios(&self, actor: u8) -> impl Iterator<Item = (usize, u8)> + '_ {
        let state = &self.actors[usize::from(actor)];
        state
            .previews
            .ids
            .iter()
            .copied()
            .enumerate()
            .filter(|(slot, _)| state.radios_owned[*slot])
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
        let Some(slot) = state.previews.ids.iter().position(|v| *v == id) else {
            return false;
        };
        state.alive && state.radios_owned[slot]
    }
    pub fn try_radio(&mut self, actor: u8, team: u8, id: u8, now: u32) -> bool {
        if !self.can_radio(actor, team, id, now) {
            return false;
        }
        let state = &mut self.actors[usize::from(actor)];
        let slot = state.previews.ids.iter().position(|v| *v == id).unwrap();
        state.radios_owned[slot] = false;
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
        self.support.retain(|s| {
            !(s.actor == source.actor
                && s.id == source.id
                && s.team == source.team
                && s.start == source.start
                && s.end == source.end)
        });
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
            tracing::warn!(
                count = self.acceptance_seed,
                "diagnostic nuclear starting progress on new life; not natural kills"
            );
        }
    }
    pub fn credit(&mut self, time: u32, victim: u8, death: u32, attacker: Option<u8>) {
        let victim_state = &mut self.actors[usize::from(victim)];
        if death <= victim_state.death {
            return;
        }
        victim_state.death = death;
        victim_state.count = 0;
        victim_state.alive = false;
        victim_state.granted = false;
        victim_state.radios_granted = [false; 3];
        let Some(attacker) = attacker.filter(|a| *a != victim) else {
            return;
        };
        let actor = &mut self.actors[usize::from(attacker)];
        if !actor.alive {
            return;
        }
        actor.count = actor.count.saturating_add(1);
        actor.best = actor.best.max(actor.count);
        if self.radios_enabled {
            for slot in 0..3 {
                let id = actor.previews.ids[slot];
                if id < 64
                    && self.available & (1u64 << id) != 0
                    && actor.count >= u16::from(actor.previews.targets[slot])
                    && !actor.radios_granted[slot]
                {
                    actor.radios_granted[slot] = true;
                    if !actor.radios_owned[slot] {
                        actor.radios_owned[slot] = true;
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
        let actor = self.actors[usize::from(actor)];
        let mut result = StreakSnapshot {
            progress: actor.count.min(255) as u8,
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
            actor.radios_owned = [false; 3];
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
            model_bounds: Default::default(),
            effects: Default::default(),
            weapons: Default::default(),
            air_route: None,
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
        assert_eq!(s.owned_radios(0).collect::<Vec<_>>(), [(1, 3)]);
        let mut hud = s.snapshot(0, 323);
        s.radio_snapshot(0, &r, &mut hud);
        assert_eq!(hud.action_weapons, [323, 0, 302, 0, 0]);
        assert!(s.try_radio(0, 1, 3, 26000));
        assert!(s.try_use(0, 26000));
        assert!(!s.try_radio(0, 1, 8, 26000));
        assert_eq!(
            s.drain_radios()
                .iter()
                .filter(|e| matches!(e, RadioEvent::Earned { .. }))
                .count(),
            2
        );
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
            s.actors[usize::from(actor)].radios_owned = [true, true, false];
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
            s.actors[usize::from(actor)].radios_owned = [true, true, false];
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
        s.actors[0].radios_owned = [true, true, true];
        s.actors[2].radios_owned = [true, true, true];
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
        resource["acceptance_stationary"] = true.into();
        write(&resource);
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_err());
        std::fs::write(directory.path().join(".codol-nuke-acceptance"), b"").unwrap();
        assert!(NukeResources::load(&path, "mp_afghan", 123).is_ok());
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
        let mut profile = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
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
        let mut profile = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
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
