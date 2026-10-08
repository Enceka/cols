//! Resolve explicit map/mode room choices against captured preparation files.
use crate::game_state::PreparedGameState;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    entries: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    map: String,
    mode: String,
    file: PathBuf,
}
#[derive(Clone, Default)]
pub struct GameMapCatalog {
    states: BTreeMap<(String, String), Arc<PreparedGameState>>,
}
impl GameMapCatalog {
    pub fn load(path: &Path) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= 65536,
            "game map catalog size"
        );
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            manifest.schema_version == 1 && (1..=32).contains(&manifest.entries.len()),
            "unsupported or empty game map catalog"
        );
        let directory = path.parent().unwrap_or(Path::new("."));
        let mut states = BTreeMap::new();
        for entry in manifest.entries {
            ensure!(
                identifier(&entry.map)
                    && entry.map.starts_with("mp_")
                    && identifier(&entry.mode)
                    && !entry.file.as_os_str().is_empty(),
                "invalid game map catalog entry"
            );
            let key = (entry.map.clone(), entry.mode.clone());
            ensure!(
                !states.contains_key(&key),
                "duplicate game map/mode preparation"
            );
            let state = PreparedGameState::load(&directory.join(&entry.file))
                .with_context(|| format!("load prepared map {} / {}", entry.map, entry.mode))?;
            ensure!(
                state.map() == entry.map && state.mode() == entry.mode,
                "prepared file map/mode does not match catalog entry"
            );
            states.insert(key, Arc::new(state));
        }
        Ok(Self { states })
    }
    pub fn with_snapshot_frames(mut self, frames: u32) -> Result<Self> {
        for state in self.states.values_mut() {
            *state = Arc::new((**state).clone().with_snapshot_frames(frames)?);
        }
        Ok(self)
    }
    pub(crate) fn select(&self, map: &str, mode: &str) -> Result<Arc<PreparedGameState>> {
        self.states
            .get(&(map.to_owned(), mode.to_owned()))
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("no captured preparation for requested map {map} / {mode}")
            })
    }
    pub(crate) fn snapshot_limit(&self) -> u32 {
        self.states
            .values()
            .map(|s| s.snapshot_limit())
            .max()
            .unwrap_or(0)
    }
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
#[cfg(test)]
pub(crate) fn write_synthetic(path: &Path, map: &str) {
    std::fs::write(path, serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"capture_sha256":"0".repeat(64),
        "map":map,"mode":"war","zone":"mp","playlist":0,"ncs_crc":0x12345678u32,
        "common_crc":1,"constant_count":2,"asset_count":1,"observed_crc_verified":false
    })).unwrap()).unwrap();
}
#[cfg(test)]
mod tests {
    use super::*;
    fn catalog(path: &Path, entries: serde_json::Value) {
        std::fs::write(
            path,
            serde_json::to_vec(&serde_json::json!({"schema_version":1,"entries":entries})).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn relative_files_bind_both_maps_and_missing_maps_remain_explicit() {
        let temp = tempfile::tempdir().unwrap();
        write_synthetic(&temp.path().join("afghan.json"), "mp_afghan");
        write_synthetic(&temp.path().join("killhouse.json"), "mp_killhouse_sh");
        let path = temp.path().join("maps.json");
        catalog(
            &path,
            serde_json::json!([
                {"map":"mp_afghan","mode":"war","file":"afghan.json"},
                {"map":"mp_killhouse_sh","mode":"war","file":"killhouse.json"}
            ]),
        );
        let maps = GameMapCatalog::load(&path)
            .unwrap()
            .with_snapshot_frames(10)
            .unwrap();
        assert_eq!(maps.select("mp_afghan", "war").unwrap().map(), "mp_afghan");
        assert_eq!(
            maps.select("mp_killhouse_sh", "war").unwrap().map(),
            "mp_killhouse_sh"
        );
        assert_eq!(maps.snapshot_limit(), 10);
        assert!(maps.select("mp_killhouse", "war").is_err());
        assert!(maps.select("mp_afghan", "sd").is_err());
    }
    #[test]
    fn duplicate_missing_and_wrong_identity_files_do_not_produce_a_catalog() {
        let temp = tempfile::tempdir().unwrap();
        write_synthetic(&temp.path().join("afghan.json"), "mp_afghan");
        let path = temp.path().join("maps.json");
        for entries in [
            serde_json::json!([
                {"map":"mp_afghan","mode":"war","file":"afghan.json"},
                {"map":"mp_afghan","mode":"war","file":"afghan.json"}
            ]),
            serde_json::json!([{"map":"mp_killhouse_sh","mode":"war","file":"afghan.json"}]),
            serde_json::json!([{"map":"mp_afghan","mode":"sd","file":"afghan.json"}]),
            serde_json::json!([{"map":"mp_afghan","mode":"war","file":"missing.json"}]),
            serde_json::json!([]),
        ] {
            catalog(&path, entries);
            assert!(GameMapCatalog::load(&path).is_err());
        }
        std::fs::write(&path, br#"{"schema_version":2,"entries":[]}"#).unwrap();
        assert!(GameMapCatalog::load(&path).is_err());
        std::fs::write(
            &path,
            br#"{"schema_version":1,"entries":[],"fallback":"mp_afghan"}"#,
        )
        .unwrap();
        assert!(GameMapCatalog::load(&path).is_err());
    }
}
