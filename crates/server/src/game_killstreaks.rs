//! Authority-owned life progress and the extra fixed-cost nuclear reward.
//! The 25-kill extra reward and 10s/1.5s timing follow the local Alpha/LAN
//! compatibility rules; native resources/layouts are verified independently.
use anyhow::{Result, ensure};
use codol_protocol::game_killstreaks::StreakSnapshot;
use serde::Deserialize;
use std::collections::VecDeque;
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
}
impl NukeResources {
    pub fn load(path: &Path, map: &str, crc: u32) -> Result<Self> {
        ensure!(
            path.metadata()?.len() <= 16384,
            "nuke resources exceed budget"
        );
        let result: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            result.acceptance_seed <= 24
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
    }
}

pub(crate) const NUKE_KILLS: u16 = 25;
pub(crate) const COUNTDOWN_MSEC: u32 = 10_000;
pub(crate) const AFTER_FLASH_MSEC: u32 = 1500;

#[derive(Clone, Copy)]
struct Actor {
    count: u16,
    best: u16,
    death: u32,
    alive: bool,
    granted: bool,
    owned: bool,
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
}
impl Streaks {
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
        let Some(attacker) = attacker.filter(|a| *a != victim) else {
            return;
        };
        let actor = &mut self.actors[usize::from(attacker)];
        if !actor.alive {
            return;
        }
        actor.count = actor.count.saturating_add(1);
        actor.best = actor.best.max(actor.count);
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        resource["acceptance_stationary"] = false.into();
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
