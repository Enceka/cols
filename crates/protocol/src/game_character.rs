//! Native custom-character clientState packing (0x40cbd0 / 0x51bc80).
use crate::lobby::LobbyError;

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Character {
    /// One-based item indices in fpa/boo/fja/han/hea/ves/heg/elp/knp/eye/bar.
    pub groups: [u16; 11],
    pub bits: [u8; 11],
    pub gender: u8,
    /// Native faction selector: TF0, SC1 (different from gameplay team IDs).
    pub faction: u8,
    /// Native skin order: wht/yel/blk/cau.
    pub skin: u8,
}

// 0x40ca90 filters the measured client field table and preserves its order.
const PACKING: [(i32, usize); 21] = [
    (20, 10),
    (21, 10),
    (22, 10),
    (27, 10),
    (31, 10),
    (32, 10),
    (33, 10),
    (34, 6),
    (35, 6),
    (36, 6),
    (43, 6),
    (44, 6),
    (45, 6),
    (46, 10),
    (47, 10),
    (48, 10),
    (49, 10),
    (50, 6),
    (51, 6),
    (52, 6),
    (53, 6),
];

impl Character {
    pub fn fields(self) -> Result<Vec<(i32, u32, usize)>, LobbyError> {
        if self.gender > 1
            || self.faction > 1
            || self.skin > 3
            || self.groups[..5].contains(&0)
            || self.bits.iter().any(|&b| b > 10)
            || self
                .groups
                .iter()
                .zip(self.bits)
                .any(|(&i, b)| i > 0 && (b == 0 || u32::from(i) > 1u32 << b))
        {
            return Err(LobbyError("invalid native character composition"));
        }
        let mut packed = Vec::new();
        for (&index, width) in self.groups.iter().zip(self.bits) {
            packed.push(u32::from(index != 0));
            if index != 0 {
                for bit in 0..width {
                    packed.push(u32::from((index - 1) >> bit & 1));
                }
            }
        }
        if packed.len() > 170 {
            return Err(LobbyError("native character packing exceeds capacity"));
        }
        let mut fields = vec![
            (16, self.groups.iter().filter(|&&i| i > 0).count() as u32, 4),
            (17, u32::from(self.faction), 1),
            (18, u32::from(self.gender), 1),
            (19, u32::from(self.skin), 2),
        ];
        let mut at = 0;
        for (index, width) in PACKING {
            let mut value = 0;
            for bit in 0..width {
                value |= packed.get(at).copied().unwrap_or(0) << bit;
                at += 1;
            }
            fields.push((index, value, width));
        }
        Ok(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packing_matches_measured_field_order_and_roundtrips_group_presence() {
        let c = Character {
            groups: [19, 1, 49, 79, 6, 0, 12, 0, 0, 0, 0],
            bits: [7, 4, 8, 8, 6, 7, 8, 5, 0, 8, 0],
            gender: 0,
            faction: 0,
            skin: 0,
        };
        let f = c.fields().unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/enemy-snapshot-fields-3.3.3.3.json"
        ))
        .unwrap();
        let expected: Vec<_> = fixture["client"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                let offset = r[0].as_u64().unwrap();
                (offset == 0x14 || (0x18..0x68).contains(&offset))
                    .then_some((i as i32, r[1].as_u64().unwrap() as usize))
            })
            .collect();
        assert_eq!(expected, PACKING);
        let mut bits = Vec::new();
        for (index, width) in expected {
            let value = f.iter().find(|x| x.0 == index).unwrap().1;
            for i in 0..width {
                bits.push((value >> i) & 1);
            }
        }
        let mut at = 0;
        for (&index, width) in c.groups.iter().zip(c.bits) {
            let present = bits[at];
            at += 1;
            let mut decoded = 0;
            if present != 0 {
                for b in 0..width {
                    decoded |= bits[at] << b;
                    at += 1;
                }
                decoded += 1;
            }
            assert_eq!(decoded, u32::from(index));
        }
        assert!(bits[at..].iter().all(|&b| b == 0));
        assert_eq!(f[0], (16, 6, 4));
        let mut invalid = c;
        invalid.groups[0] = 129;
        assert!(invalid.fields().is_err());
        invalid = c;
        invalid.gender = 2;
        assert!(invalid.fields().is_err());
    }
}
