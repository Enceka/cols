//! Explicit cargo metadata for native resource initialization without gameplay.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    schema_version: u32,
    client_sha256: String,
    capture_sha256: String,
    map: String,
    mode: String,
    ncs_crc: u32,
    common_crc: u32,
    constant_count: u32,
    asset_count: u32,
}

impl PreparedGameState {
    pub fn load_story_resources(path: &Path) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= 65536,
            "story metadata exceeds budget"
        );
        let m: Metadata = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            m.schema_version == 1
                && m.client_sha256 == codol_protocol::CLIENT_SHA256
                && m.map == codol_protocol::room::story::FIRST_MAP
                && m.mode == "coop"
                && m.capture_sha256.len() == 64
                && m.capture_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                && m.ncs_crc != 0
                && m.common_crc != 0
                && (1..=512).contains(&m.asset_count)
                && (1..=65536).contains(&m.constant_count),
            "invalid captured story metadata"
        );
        let mut state: Self = serde_json::from_value(serde_json::json!({
            "schema_version":m.schema_version,"client_sha256":m.client_sha256,
            "capture_sha256":m.capture_sha256,"map":m.map,"mode":m.mode,"zone":"mp",
            "playlist":0,"ncs_crc":m.ncs_crc,"common_crc":m.common_crc,
            "constant_count":m.constant_count,"asset_count":m.asset_count,
            "observed_crc_verified":false
        }))?;
        state.story_resource_only = true;
        state.validate()?;
        state.resource_initialization()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn story_metadata_cannot_import_gameplay_fields_or_use_the_tdm_loader() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("story.json");
        let mut value = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "capture_sha256":"a".repeat(64),"map":codol_protocol::room::story::FIRST_MAP,"mode":"coop",
            "ncs_crc":123,"common_crc":456,"constant_count":100,"asset_count":10});
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let state = PreparedGameState::load_story_resources(&path).unwrap();
        assert!(
            !state.has_spawn()
                && !state.has_tdm()
                && state.snapshot_limit() == 0
                && !state.snapshot_probe()
        );
        assert!(state.body(0).is_ok());
        assert!(PreparedGameState::load(&path).is_err());
        value["spawn_origin"] = serde_json::json!([0, 0, 0]);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(PreparedGameState::load_story_resources(&path).is_err());
        value.as_object_mut().unwrap().remove("spawn_origin");
        value["map"] = "mp_afghan".into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(PreparedGameState::load_story_resources(&path).is_err());
    }
}
