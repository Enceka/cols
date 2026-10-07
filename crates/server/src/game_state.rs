//! Opt-in, resource-derived bootstrap for the controlled training playlist.
use anyhow::{Result, ensure};
use codol_protocol::game_state::Gamestate;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
#[path = "game_loadout.rs"]
mod loadout;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapWeapon {
    name: String,
    index: u16,
    #[serde(default)]
    ammo: Option<BootstrapAmmo>,
    #[serde(default)]
    base_index: Option<u16>,
    #[serde(default)]
    composed_name: Option<String>,
    #[serde(default)]
    viewmodel_index: Option<u16>,
    #[serde(default)]
    attachments: Vec<u16>,
    #[serde(default)]
    attachment_source_sha256: Option<String>,
    #[serde(default)]
    camo: Option<u8>, // omitted only in legacy diagnostic preparations (camo3)
    #[serde(default)]
    switch: Option<crate::game_player::SwitchTiming>,
    #[serde(default)]
    move_scale: Option<f32>,
    #[serde(default)]
    fire_type: Option<u32>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapAmmo {
    clip_key: u16,
    reserve_key: u16,
    clip: u16,
    reserve: u16,
    source_sha256: String,
    #[serde(default)]
    fire_interval_msec: Option<u16>,
    #[serde(default)]
    reload: Option<crate::game_player::ReloadTiming>,
    #[serde(default)]
    ads: Option<crate::game_player::AdsTiming>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedGameState {
    schema_version: u32,
    client_sha256: String,
    capture_sha256: String,
    map: String,
    mode: String,
    zone: String,
    playlist: u32,
    ncs_crc: u32,
    common_crc: u32,
    constant_count: u32,
    asset_count: u32,
    observed_crc_verified: bool,
    #[serde(default)]
    spawn_origin: Option<[f32; 3]>,
    #[serde(default)]
    map_entities_sha256: Option<String>,
    #[serde(default)]
    bootstrap_weapon: Option<BootstrapWeapon>,
    #[serde(default)]
    player_loadout: Option<loadout::Loadout>,
    #[serde(default)]
    secondary_weapon: Option<BootstrapWeapon>,
    #[serde(default)]
    arsenal_file: Option<std::path::PathBuf>,
    #[serde(default)]
    collision_file: Option<std::path::PathBuf>,
    #[serde(skip)]
    arsenal: Option<Arc<crate::game_arsenal::Arsenal>>,
    #[serde(skip)]
    pub(crate) collision: Option<Arc<crate::game_motion::Collision>>,
    #[serde(skip)]
    pub(crate) profile_hash: Option<[u8; 32]>,
    #[serde(skip)]
    snapshot_probe: bool,
    #[serde(skip)]
    snapshot_frames: u32,
}
impl PreparedGameState {
    pub fn with_snapshot_frames(mut self, frames: u32) -> Result<Self> {
        ensure!(
            (1..=18000).contains(&frames),
            "snapshot frame budget out of range"
        );
        self.snapshot_probe = true;
        self.snapshot_frames = frames;
        Ok(self)
    }
    pub(crate) fn snapshot_limit(&self) -> u32 {
        self.snapshot_frames
    }
    pub(crate) fn snapshot_probe(&self) -> bool {
        self.snapshot_probe
    }
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= 65536,
            "prepared game state size"
        );
        let mut state: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        let directory = path.parent().unwrap_or(Path::new("."));
        if let Some(file) = &state.arsenal_file {
            state.arsenal = Some(Arc::new(crate::game_arsenal::Arsenal::load(
                directory.join(file),
                state.ncs_crc,
            )?));
        }
        if let Some(file) = &state.collision_file {
            let collision = crate::game_motion::Collision::load(&directory.join(file))?;
            ensure!(
                collision.ncs_crc == state.ncs_crc
                    && collision.map == format!("maps/mp/{}.d3dbsp", state.map),
                "collision identity mismatch"
            );
            state.collision = Some(Arc::new(collision));
        }
        state.validate()?;
        Ok(state)
    }
    fn validate(&self) -> Result<()> {
        let state = self;
        for weapon in state.bootstrap_weapon.iter().chain(&state.secondary_weapon) {
            ensure!(
                state.spawn_origin.is_some()
                    && !weapon.name.is_empty()
                    && weapon.name.len() <= 64
                    && (1..900).contains(&weapon.index),
                "invalid bootstrap weapon or missing spawn"
            );
            ensure!(
                weapon.switch.is_none_or(|t| t.valid())
                    && weapon
                        .move_scale
                        .is_none_or(|s| s.is_finite() && s > 0.0 && s <= 4.0),
                "invalid weapon switching/movement parameters"
            );
            ensure!(
                weapon.fire_type.is_none_or(|v| matches!(v, 0 | 1 | 5)),
                "unsupported burst firing state machine"
            );
            ensure!(
                weapon.viewmodel_index.is_none_or(|i| i > 0 && i < 1024),
                "invalid viewmodel index"
            );
            ensure!(
                weapon.base_index.is_some() == weapon.composed_name.is_some(),
                "incomplete composed weapon identity"
            );
            ensure!(
                weapon.camo.is_none() || weapon.base_index.is_some(),
                "camo requires composed weapon"
            );
            if let Some(base) = weapon.base_index {
                ensure!(
                    base > 0
                        && base < weapon.index
                        && weapon.composed_name.as_deref()
                            == Some(&codol_protocol::game_state::composed_weapon(
                                base,
                                weapon.camo.unwrap_or(3),
                                &weapon.attachments
                            )?),
                    "unsupported bootstrap weapon composition"
                );
            }
            ensure!(
                weapon.attachments.is_empty() == weapon.attachment_source_sha256.is_none()
                    && (weapon.attachments.is_empty() || weapon.base_index.is_some())
                    && weapon.attachment_source_sha256.as_ref().is_none_or(
                        |hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                    ),
                "missing attachment evidence or composition"
            );
            if let Some(ammo) = &weapon.ammo {
                ensure!(
                    ammo.clip_key <= 1022
                        && ammo.reserve_key <= 1023
                        && ammo.clip <= 511
                        && ammo.reserve <= 2047
                        && ammo
                            .fire_interval_msec
                            .is_none_or(|n| (50..=2000).contains(&n))
                        && ammo
                            .reload
                            .is_none_or(|r| r.valid() && ammo.fire_interval_msec.is_some())
                        && ammo.source_sha256.len() == 64
                        && ammo.ads.is_none_or(|a| a.valid())
                        && ammo.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                    "invalid bootstrap ammo seed"
                );
            }
        }
        ensure!(
            state.spawn_origin.is_some() == state.map_entities_sha256.is_some(),
            "spawn evidence missing"
        );
        if let Some(origin) = state.spawn_origin {
            ensure!(
                origin
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 1_000_000.0),
                "invalid spawn origin"
            );
            let hash = state.map_entities_sha256.as_deref().unwrap_or("");
            ensure!(
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid map entity source hash"
            );
        }
        ensure!(
            state.schema_version == 1 && state.client_sha256 == codol_protocol::CLIENT_SHA256,
            "prepared game state identity mismatch"
        );
        ensure!(
            state.capture_sha256.len() == 64
                && state.capture_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "prepared game state source hash"
        );
        ensure!(
            state.map == "mp_afghan"
                && state.mode == "war"
                && state.zone == "mp"
                && state.playlist == 0,
            "only the controlled training playlist is supported"
        );
        ensure!(
            (1..=512).contains(&state.asset_count) && (1..=65536).contains(&state.constant_count),
            "prepared game state resource counts"
        );
        if let Some(loadout) = &state.player_loadout {
            loadout.validate(state)?;
        }
        tracing::info!(
            ncs_crc = state.ncs_crc,
            common_crc = state.common_crc,
            observed_crc_verified = state.observed_crc_verified,
            "loaded prepared training state"
        );
        Ok(())
    }

    pub(crate) fn uses_player_loadout(&self) -> bool {
        self.player_loadout.is_some()
    }
    pub(crate) fn dynamic_loadout(&self) -> bool {
        self.arsenal.is_some()
    }
    pub(crate) fn next_weapon_index(&self) -> u16 {
        self.bootstrap_weapon
            .iter()
            .chain(&self.secondary_weapon)
            .map(|w| w.index + 1)
            .max()
            .unwrap_or(1)
    }

    pub(crate) fn for_profile(&self, public: &[u8], owned: &[u32]) -> Result<Self> {
        let mut state = self.for_profile_at(
            public,
            owned,
            self.bootstrap_weapon
                .as_ref()
                .map_or(self.next_weapon_index(), |w| w.index),
        )?;
        let mut next = self.bootstrap_weapon.as_ref().map_or(1, |w| w.index);
        state.register_weapons(&mut std::collections::BTreeMap::new(), &mut next)?;
        Ok(state)
    }
    pub(crate) fn for_profile_at(
        &self,
        public: &[u8],
        owned: &[u32],
        next_index: u16,
    ) -> Result<Self> {
        let loadout = self
            .player_loadout
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing loadout catalogue"))?;
        let mut state = self.clone();
        if let Some(arsenal) = &self.arsenal {
            let selections = loadout.selections(public)?;
            let hands = self
                .bootstrap_weapon
                .as_ref()
                .and_then(|w| w.viewmodel_index);
            state.bootstrap_weapon = Some(Self::resolved_weapon(
                arsenal.resolve(selections[0].0, &selections[0].1, owned, next_index)?,
                &arsenal.source_sha256,
                hands,
            )?);
            state.secondary_weapon = if selections[1].0 == 0 {
                None
            } else {
                Some(Self::resolved_weapon(
                    arsenal.resolve(selections[1].0, &selections[1].1, owned, next_index + 1)?,
                    &arsenal.source_sha256,
                    hands,
                )?)
            };
            tracing::info!(primary=?state.bootstrap_weapon.as_ref().and_then(|w|w.composed_name.as_deref()),secondary=?state.secondary_weapon.as_ref().and_then(|w|w.composed_name.as_deref()),"composed current player backpack from resource attributes");
        } else {
            state.bootstrap_weapon = Some(loadout.resolve(self, public, owned)?);
            state.player_loadout = None;
        }
        state.profile_hash = Some(Sha256::digest(public).into());
        state.validate()?;
        Ok(state)
    }
    fn resolved_weapon(
        w: crate::game_arsenal::ResolvedWeapon,
        source: &str,
        hands: Option<u16>,
    ) -> Result<BootstrapWeapon> {
        let word = |at: usize| u32::from_le_bytes(w.definition[at..at + 4].try_into().unwrap());
        let time = |at: usize| -> Result<u16> {
            let n = word(at);
            ensure!(n <= 15000, "weapon duration exceeds supported range");
            Ok(n as u16)
        };
        let normal = time(0x71c)?;
        let empty = time(0x728)?;
        let add = time(0x730)?;
        let empty_add = time(0x738)?;
        ensure!(
            word(0xd08) > 0 && word(0xd08) <= 511 && word(0x6b8) <= 1023,
            "weapon ammo limits"
        );
        ensure!(
            word(0x6f0) == 0,
            "delayed-fire weapon needs its firing state machine"
        );
        let ammo = BootstrapAmmo {
            clip_key: w.index,
            reserve_key: word(0x6b8) as u16,
            clip: word(0xd08) as u16,
            reserve: (word(0xd08) * 3).min(2047) as u16,
            source_sha256: source.into(),
            fire_interval_msec: Some(time(0xd10)?),
            reload: Some(crate::game_player::ReloadTiming {
                normal_msec: normal,
                empty_msec: empty,
                add_msec: if add == 0 { normal } else { add.min(normal) },
                empty_add_msec: if empty_add == 0 {
                    empty
                } else {
                    empty_add.min(empty)
                },
            }),
            ads: if w.definition[0xd76] == 0 {
                None
            } else {
                Some(crate::game_player::AdsTiming {
                    in_rate: f32::from_bits(word(0xbac)),
                    out_rate: f32::from_bits(word(0xbb0)),
                    reload_allowed: w.definition[0xd78] != 0,
                    reload_interruptible: w.definition[0xd86] != 0,
                    reload_end_msec: time(0xb38)?,
                })
            },
        };
        let drop = time(0x74c)?;
        let raise = time(0x750)?;
        let switch = crate::game_player::SwitchTiming {
            drop_msec: drop,
            raise_msec: raise,
            empty_drop_msec: match time(0x768)? {
                0 => drop,
                v => v,
            },
            empty_raise_msec: match time(0x764)? {
                0 => raise,
                v => v,
            },
        };
        let scale = f32::from_bits(word(0x7c8));
        let fire_type = word(0x48c);
        Ok(BootstrapWeapon {
            name: w.name,
            index: w.index,
            ammo: Some(ammo),
            base_index: Some(w.base_index),
            composed_name: Some(w.composed_name),
            viewmodel_index: hands,
            attachment_source_sha256: if w.attachments.is_empty() {
                None
            } else {
                Some(source.into())
            },
            attachments: w.attachments,
            camo: Some(0),
            switch: Some(switch),
            move_scale: Some(scale),
            fire_type: Some(fire_type),
        })
    }
    pub(crate) fn weapon_configstrings(&self) -> Vec<(u16, String)> {
        self.bootstrap_weapon
            .iter()
            .chain(&self.secondary_weapon)
            .filter_map(|w| {
                w.composed_name
                    .as_ref()
                    .map(|n| (5443 + w.index, n.clone()))
            })
            .collect()
    }
    pub(crate) fn register_weapons(
        &mut self,
        known: &mut std::collections::BTreeMap<String, u16>,
        next: &mut u16,
    ) -> Result<Vec<(u16, String)>> {
        let mut added = Vec::new();
        for weapon in self
            .bootstrap_weapon
            .iter_mut()
            .chain(&mut self.secondary_weapon)
        {
            let name = weapon
                .composed_name
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing composed identity"))?;
            let index = if let Some(&index) = known.get(name) {
                index
            } else {
                ensure!(*next < 900, "weapon registry exhausted");
                let index = *next;
                *next += 1;
                known.insert(name.clone(), index);
                added.push((5443 + index, name.clone()));
                index
            };
            weapon.index = index;
            if let Some(ammo) = &mut weapon.ammo {
                ammo.clip_key = index;
            }
        }
        Ok(added)
    }

    pub(crate) fn matches(&self, build: i32, start: &str) -> bool {
        // This is a comparison with the captured control request, never a
        // console interpreter. Do not apply this map CRC to arbitrary rooms.
        build == -688906341
            && start.split(';').map(str::trim).eq([
                "mmstart sv_resetServer",
                "2486 1",
                "3265 12",
                "2489 1",
                "playlist 0",
                "start_map_rotate_once 0",
            ])
    }

    pub(crate) fn body(&self, command_sequence: u32) -> Result<Vec<u8>> {
        // CS0 keys mapname/g_gametype confirmed at 0x465aa2/0x465abf.
        // 0x46d830 parses CS3 into C2S serverId. CS16 is map-center text.
        let mut values = vec![
            (
                0,
                format!("\\mapname\\{}\\g_gametype\\{}", self.map, self.mode),
            ),
            (1, String::new()),
            // 0x445b74 compares CS2 with literal "cod" before weapon init.
            (2, "cod".to_owned()),
            (3, "1".to_owned()),
            (16, "0 0 0".to_owned()),
            // 0x579f30 sscanf(CS6415, "%i %i %g %g", ...). Empty text leaves
            // uninitialized slow-motion values and nearly stops client time.
            (6415, "0 0 1 1".to_owned()),
        ];
        for (index, name) in self.weapon_configstrings() {
            // NCS inherited weapon bank starts at5443; unused indices are
            // untyped and accept a regular configstring. Parser0x42e3b0.
            values.push((index, name));
            values.sort_by_key(|v| v.0);
        }
        if self.bootstrap_weapon.is_some() {
            // 0x4435fa / 0x5597d0 iterate 1..<CS6800 and register each
            // CG weapon presentation. Without this the secondary's HUD name
            // stays null until viewmodel registration, racing HUD drawing.
            values.push((6800, self.next_weapon_index().to_string()));
            values.sort_by_key(|v| v.0);
        }
        Ok(Gamestate {
            command_sequence,
            map: &self.map,
            zone: &self.zone,
            ncs_crc: self.ncs_crc,
            configstrings: &values,
            client_number: 0,
            checksum_feed: 0,
        }
        .encode()?)
    }
    pub(crate) fn snapshot(
        &self,
        server_time: u32,
        command_time: u32,
        establish_baseline: bool,
        angles: [u16; 3],
        equipment: Option<codol_protocol::game_snapshot::Equipment>,
        motion: Option<&crate::game_motion::Motion>,
    ) -> Result<Vec<u8>> {
        Ok(codol_protocol::game_snapshot::player_frame_with_motion(
            server_time,
            command_time,
            motion.map(|m| m.origin).or(self.spawn_origin),
            establish_baseline,
            equipment,
            angles,
            motion.map(|m| codol_protocol::game_snapshot::MotionSnapshot {
                velocity: m.velocity,
                flags: m.flags,
                ground_entity: m.ground_entity,
                view_height: m.view_height,
                view_height_target: m.view_height_target,
                speed: m.speed,
                gravity: m.gravity,
                bob_cycle: m.bob_cycle,
                view_height_lerp_time: m.view_height_lerp_time,
                view_height_lerp_target: m.view_height_lerp_target,
                view_height_lerp_down: m.view_height_lerp_down,
                jump_time: m.jump_time,
                jump_origin_z: m.jump_origin_z,
            }),
        )?)
    }
    pub(crate) fn player(&self) -> Option<crate::game_player::Player> {
        self.primary_player()
            .map(|p| match self.secondary_player() {
                Some(s) => p.with_secondary(s),
                None => p,
            })
    }
    pub(crate) fn primary_player(&self) -> Option<crate::game_player::Player> {
        self.bootstrap_weapon.as_ref().map(Self::make_player)
    }
    pub(crate) fn secondary_player(&self) -> Option<crate::game_player::Player> {
        self.secondary_weapon.as_ref().map(Self::make_player)
    }
    fn make_player(w: &BootstrapWeapon) -> crate::game_player::Player {
        let mut player = crate::game_player::Player::new(
            codol_protocol::game_snapshot::Equipment {
                secondary: None,
                events: None,
                weapon: w.index,
                viewmodel: w.viewmodel_index,
                firing: None,
                aim: None,
                ammo: w
                    .ammo
                    .as_ref()
                    .map(|a| codol_protocol::game_snapshot::Ammo {
                        clip_key: a.clip_key,
                        reserve_key: a.reserve_key,
                        clip: a.clip,
                        reserve: a.reserve,
                    }),
            },
            w.ammo.as_ref().and_then(|a| a.fire_interval_msec),
            w.ammo.as_ref().and_then(|a| a.reload),
            w.ammo.as_ref().and_then(|a| a.ads),
        );
        if let Some(timing) = w.switch {
            player.set_switch_timing(timing);
        }
        if let Some(scale) = w.move_scale {
            player.set_move_scale(scale);
        }
        if let Some(kind) = w.fire_type {
            player.set_fire_type(kind).expect("validated fire type");
        }
        player
    }
    pub(crate) fn motion(&self) -> Option<crate::game_motion::Motion> {
        self.collision
            .as_ref()
            .and(self.spawn_origin)
            .map(|origin| {
                let mut m = crate::game_motion::Motion::new(origin);
                m.command_time = 1000;
                m
            })
    }
    pub(crate) fn input_base(&self) -> codol_protocol::game_usercmd::UserCommand {
        let mut base = codol_protocol::game_usercmd::UserCommand::default();
        if let Some(weapon) = &self.bootstrap_weapon {
            base.weapons[0] = weapon.index;
            base.melee_entity = 2047; // target 0x5a18e0, POF_PLAYER (0x1000)
        }
        base
    }
    pub(crate) fn has_spawn(&self) -> bool {
        self.spawn_origin.is_some()
    }
}

#[cfg(test)]
pub(crate) fn synthetic() -> PreparedGameState {
    PreparedGameState {
        schema_version: 1,
        client_sha256: codol_protocol::CLIENT_SHA256.into(),
        capture_sha256: "0".repeat(64),
        map: "mp_afghan".into(),
        mode: "war".into(),
        zone: "mp".into(),
        playlist: 0,
        ncs_crc: 0x12345678,
        common_crc: 1,
        constant_count: 2,
        asset_count: 1,
        observed_crc_verified: false,
        spawn_origin: None,
        map_entities_sha256: None,
        bootstrap_weapon: None,
        player_loadout: None,
        secondary_weapon: None,
        arsenal_file: None,
        collision_file: None,
        arsenal: None,
        collision: None,
        profile_hash: None,
        snapshot_probe: false,
        snapshot_frames: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_training_state_cannot_match_another_playlist_or_console_suffix() {
        let state = synthetic();
        let start =
            "mmstart sv_resetServer; 2486 1; 3265 12; 2489 1; playlist 0; start_map_rotate_once 0";
        assert!(state.matches(-688906341, start));
        assert!(!state.matches(0, start));
        assert!(!state.matches(-688906341, &start.replace("playlist 0", "playlist 1")));
        assert!(!state.matches(-688906341, &format!("{start}; map other")));
    }
    #[test]
    fn bootstrap_supplies_a_complete_normal_speed_slowmotion_config() {
        let body = synthetic().body(1).unwrap();
        let mut r = codol_protocol::game_wire::Reader::new(&body);
        assert_eq!(r.u8().unwrap(), 3);
        r.u32().unwrap();
        r.string().unwrap();
        r.string().unwrap();
        r.u32().unwrap();
        assert_eq!(r.u8().unwrap(), 4);
        let count = r.u16().unwrap();
        let mut index = -1i32;
        let mut slowmo = None;
        for _ in 0..count {
            index = if r.bits(1).unwrap() == 1 {
                index + 1
            } else {
                r.bits(13).unwrap() as i32
            };
            let value = r.string().unwrap();
            if index == 6415 {
                slowmo = Some(std::str::from_utf8(value).unwrap().to_owned());
            }
        }
        let fields = slowmo
            .unwrap()
            .split_whitespace()
            .map(|v| v.parse::<f32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(fields, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(r.bits(10).unwrap(), 0);
        assert_eq!(r.bits(10).unwrap(), 0);
        assert_eq!(r.u8().unwrap(), 6);
        r.u32().unwrap();
        r.u32().unwrap();
        assert_eq!(r.u8().unwrap(), 6);
        r.finish().unwrap();
    }
}
