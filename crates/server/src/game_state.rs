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
}
impl PreparedGameState {
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= 4096,
            "prepared game state size"
        );
        let state: Self = serde_json::from_slice(&std::fs::read(path)?)?;
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
}
