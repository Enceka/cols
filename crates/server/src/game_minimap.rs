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
}

impl Minimap {
    pub fn validate(&self, map: &str) -> Result<()> {
        // Loaded 3.3.3.3 Crash material references the _dlc image. The
        // generated _tropical material uses the default image instead.
        let material = match map {
            "mp_afghan" => "compass_map_mp_afghan",
            "mp_killhouse_sh" => "compass_map_mp_killhouse_sh",
            "mp_crash_tropical" => "compass_map_mp_crash_dlc",
            _ => anyhow::bail!("unsupported minimap map"),
        };
        ensure!(self.material == material, "minimap material/map mismatch");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_requires_the_loaded_material_and_publishes_its_own_bounds() {
        let mut minimap = Minimap {
            material: "compass_map_mp_crash_dlc".into(),
            north_yaw: 0.0,
            upper_left: [1976.0, 2528.0],
            lower_right: [-1200.0, -2166.0],
        };
        minimap.validate("mp_crash_tropical").unwrap();
        assert_eq!(
            minimap.configstrings()[1],
            (
                2038,
                "compass_map_mp_crash_dlc 1976 2528 -1200 -2166".into()
            )
        );
        assert!(minimap.validate("mp_afghan").is_err());
        minimap.material = "compass_map_mp_crash_tropical".into();
        assert!(minimap.validate("mp_crash_tropical").is_err());
    }
}
