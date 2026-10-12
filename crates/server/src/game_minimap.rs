//! Native 3.3.3.3 compass: CS2037 north yaw, CS2038 material and map corners.
//! Consumers 0x43d770 / 0x43d500; coordinates come from this map's entities.
use anyhow::{Result, ensure};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Minimap {
    pub material: String,
    pub north_yaw: f32,
    pub upper_left: [f32; 2],
    pub lower_right: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_proof: Option<MaterialProof>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterialProof {
    pub client_sha256: String,
    pub capture_sha256: String,
    pub map: String,
    pub material: String,
    pub ncs_crc: u32,
    pub render_default: bool,
    pub texture_dims: Vec<[u16; 2]>,
}

impl Minimap {
    pub fn validate(&self, map: &str, ncs_crc: u32) -> Result<()> {
        // Loaded 3.3.3.3 Crash material references the _dlc image. The
        // generated _tropical material uses the default image instead.
        let material = match map {
            "mp_afghan" => "compass_map_mp_afghan",
            "mp_killhouse_sh" => "compass_map_mp_killhouse_sh",
            "mp_crash_tropical" => "compass_map_mp_crash_dlc",
            _ => {
                ensure!(
                    crate::game_map_registry::native_id(map).is_some(),
                    "unknown minimap map"
                );
                let proof = self
                    .material_proof
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("minimap needs captured material proof"))?;
                ensure!(
                    proof.client_sha256 == codol_protocol::CLIENT_SHA256
                        && proof.capture_sha256.len() == 64
                        && proof.capture_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                        && proof.map == format!("maps/mp/{map}.d3dbsp")
                        && proof.material == self.material
                        && proof.ncs_crc == ncs_crc
                        && !proof.render_default
                        && !proof.texture_dims.is_empty()
                        && proof.texture_dims.len() <= 32
                        && proof
                            .texture_dims
                            .iter()
                            .any(|d| d.iter().all(|v| (64..=8192).contains(v)))
                        && self.material.starts_with("compass_map_")
                        && self.material.len() < 128
                        && self
                            .material
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                    "minimap material proof identity or image invalid"
                );
                self.material.as_str()
            }
        };
        ensure!(self.material == material, "minimap material/map mismatch");
        self.validate_geometry()
    }
    fn validate_geometry(&self) -> Result<()> {
        ensure!(
            self.north_yaw.is_finite() && (0.0..360.0).contains(&self.north_yaw),
            "invalid minimap north yaw"
        );
        ensure!(
            self.upper_left
                .iter()
                .chain(&self.lower_right)
                .all(|v| v.is_finite() && v.abs() <= 32768.0),
            "invalid minimap corners"
        );
        let (sin, cos) = self.north_yaw.to_radians().sin_cos();
        let x = self.lower_right[0] - self.upper_left[0];
        let y = self.lower_right[1] - self.upper_left[1];
        ensure!(
            x * sin - y * cos > 1.0 && -x * cos - y * sin > 1.0,
            "minimap corners must span positive width and height"
        );
        Ok(())
    }
    pub fn configstrings(&self) -> [(u16, String); 2] {
        [
            (2037, self.north_yaw.to_string()),
            (
                2038,
                format!(
                    "{} {} {} {} {}",
                    self.material,
                    self.upper_left[0],
                    self.upper_left[1],
                    self.lower_right[0],
                    self.lower_right[1]
                ),
            ),
        ]
    }
}

/// A bounded native material-registration request used only during resource
/// initialization. It is not a material proof or a gameplay preparation.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompassProbe {
    pub schema_version: u32,
    pub client_sha256: String,
    pub map: String,
    pub ncs_crc: u32,
    pub entities_sha256: String,
    pub minimap: Minimap,
}
impl CompassProbe {
    pub fn load(path: &std::path::Path, map: &str) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= 65536,
            "compass probe exceeds limit"
        );
        let probe: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(
            probe.schema_version == 1
                && probe.client_sha256 == codol_protocol::CLIENT_SHA256
                && probe.map == map
                && matches!(
                    map,
                    "mp_highrise" | "mp_subbase_sh" | "mp_dr_box" | "mp_dr_cistern" | "za_coliseum"
                )
                && probe.entities_sha256.len() == 64
                && probe.entities_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                && probe.minimap.material == format!("compass_map_{map}")
                && probe.minimap.material_proof.is_none(),
            "compass probe requires this map's captured identity and native candidate name"
        );
        probe.minimap.validate_geometry()?;
        Ok(probe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compass_candidate_does_not_supply_gameplay_material_proof() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut value = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"mp_highrise","ncs_crc":123,"entities_sha256":"a".repeat(64),
            "minimap":{"material":"compass_map_mp_highrise","north_yaw":0,"upper_left":[100,100],"lower_right":[-100,-100]}});
        std::fs::write(file.path(), serde_json::to_vec(&value).unwrap()).unwrap();
        let probe = CompassProbe::load(file.path(), "mp_highrise").unwrap();
        assert!(probe.minimap.validate("mp_highrise", 123).is_err());
        assert!(CompassProbe::load(file.path(), "mp_subbase_sh").is_err());
        value["minimap"]["material"] = "compass_map_mp_afghan".into();
        std::fs::write(file.path(), serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(CompassProbe::load(file.path(), "mp_highrise").is_err());
    }

    #[test]
    fn additional_map_materials_require_bound_capture_identity_and_real_images() {
        let map = Minimap {
            material: "compass_map_mp_bog_sh".into(),
            north_yaw: 0.0,
            upper_left: [100.0, 100.0],
            lower_right: [-100.0, -100.0],
            material_proof: Some(MaterialProof {
                client_sha256: codol_protocol::CLIENT_SHA256.into(),
                capture_sha256: "a".repeat(64),
                map: "maps/mp/mp_bog_sh.d3dbsp".into(),
                material: "compass_map_mp_bog_sh".into(),
                ncs_crc: 123,
                render_default: false,
                texture_dims: vec![[512, 512]],
            }),
        };
        map.validate("mp_bog_sh", 123).unwrap();
        assert!(map.validate("mp_bog_sh", 124).is_err());
        assert!(map.validate("mp_nuked", 123).is_err());
        let mut altered = map.clone();
        altered.material_proof = None;
        assert!(altered.validate("mp_bog_sh", 123).is_err());
        let mut altered = map.clone();
        altered.material = "compass_map_mp_afghan".into();
        assert!(altered.validate("mp_bog_sh", 123).is_err());
        let mut altered = map.clone();
        altered.material_proof.as_mut().unwrap().render_default = true;
        assert!(altered.validate("mp_bog_sh", 123).is_err());
        let mut altered = map;
        altered.material_proof.as_mut().unwrap().texture_dims = vec![[16, 16]];
        assert!(altered.validate("mp_bog_sh", 123).is_err());
    }

    #[test]
    fn crash_requires_the_loaded_material_and_publishes_its_own_bounds() {
        let mut minimap = Minimap {
            material: "compass_map_mp_crash_dlc".into(),
            north_yaw: 0.0,
            upper_left: [1976.0, 2528.0],
            lower_right: [-1200.0, -2166.0],
            material_proof: None,
        };
        minimap.validate("mp_crash_tropical", 123).unwrap();
        assert_eq!(
            minimap.configstrings()[1],
            (
                2038,
                "compass_map_mp_crash_dlc 1976 2528 -1200 -2166".into()
            )
        );
        assert!(minimap.validate("mp_afghan", 123).is_err());
        minimap.material = "compass_map_mp_crash_tropical".into();
        assert!(minimap.validate("mp_crash_tropical", 123).is_err());
    }
}
