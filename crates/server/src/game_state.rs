//! Opt-in, resource-derived bootstrap for the controlled training playlist.
use anyhow::{Result, ensure};
use codol_protocol::game_state::Gamestate;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
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
            std::fs::metadata(path)?.len() <= 4096,
            "prepared game state size"
        );
        let state: Self = serde_json::from_slice(&std::fs::read(path)?)?;
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
        tracing::info!(
            ncs_crc = state.ncs_crc,
            common_crc = state.common_crc,
            observed_crc_verified = state.observed_crc_verified,
            "loaded prepared training state"
        );
        Ok(state)
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
        let values = [
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
    pub(crate) fn snapshot(&self, server_time: u32, establish_baseline: bool) -> Result<Vec<u8>> {
        Ok(codol_protocol::game_snapshot::frame(
            server_time,
            self.spawn_origin,
            establish_baseline,
        )?)
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
