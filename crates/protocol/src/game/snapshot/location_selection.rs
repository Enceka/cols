//! Target3.3.3.3 PS+0x234, field143/8. Render0x439236 uses CS2047+shader,
//! 0x439252 extracts five radius bits; input0x45fa6b extracts direction bit7.
use crate::lobby::channel::LobbyError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocationSelection {
    pub shader: u8,
    pub radius: u8,
    pub directional: bool,
}
impl LocationSelection {
    pub fn native_byte(self) -> Result<u8, LobbyError> {
        if !(1..=3).contains(&self.shader) || self.radius > 31 {
            return Err(LobbyError("invalid location selection"));
        }
        Ok(self.shader | self.radius << 2 | u8::from(self.directional) << 7)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_layout_and_selector_bits_are_independent() {
        let table: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/player-state-fields-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(table["fields"][143], serde_json::json!([0x234, 8, 0]));
        for shader in 1..=3 {
            for radius in 0..=31 {
                for directional in [false, true] {
                    let byte = LocationSelection {
                        shader,
                        radius,
                        directional,
                    }
                    .native_byte()
                    .unwrap();
                    assert_eq!(byte & 3, shader);
                    assert_eq!((byte >> 2) & 31, radius);
                    assert_eq!(byte >> 7, u8::from(directional));
                }
            }
        }
        assert!(
            LocationSelection {
                shader: 0,
                radius: 1,
                directional: false
            }
            .native_byte()
            .is_err()
        );
    }
}
