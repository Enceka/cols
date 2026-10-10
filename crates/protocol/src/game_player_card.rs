//! Client-state card fields consumed by 0x525320..0x5253c7, then 0x6889c0.
use crate::{lobby::LobbyError, profile};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Card {
    /// Native zero-based level; the UI adds one.
    pub rank: u8,
    pub prestige: u8,
    pub icon: u16,
    pub title: u16,
    pub background: u16,
    pub nameplate: u16,
}
impl Card {
    pub fn validate(self) -> Result<(), LobbyError> {
        if self.rank >= 70
            || [self.icon, self.title, self.background, self.nameplate]
                .iter()
                .any(|v| *v >= 1024)
        {
            return Err(LobbyError("invalid native player card"));
        }
        Ok(())
    }
    pub fn from_profile(public: &[u8], projections: &[usize]) -> Result<Self, LobbyError> {
        let snapshot = profile::public_snapshot(public, projections)?;
        Self::from_items(std::array::from_fn(|i| {
            u32::from_le_bytes(snapshot[3846 + i * 4..3850 + i * 4].try_into().unwrap())
        }))
    }
    fn from_items(items: [u32; 4]) -> Result<Self, LobbyError> {
        static CATALOG: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
        let catalog = CATALOG.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../../fixtures/protocol/player-card-3.3.3.3.json"
            ))
            .expect("checked player card catalogue")
        });
        let index = |kind: &str, id: u32| -> Result<u16, LobbyError> {
            if id == 0 {
                return Ok(0);
            }
            catalog["catalog"][kind]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row[0].as_u64() == Some(u64::from(id)))
                .map(|row| row[1].as_u64().unwrap() as u16)
                .ok_or(LobbyError(
                    "saved player card item is absent from native callsign table",
                ))
        };
        if items[2] != 0 {
            return Err(LobbyError("unsupported native nameplate catalogue"));
        }
        let card = Self {
            background: index("background", items[0])?,
            icon: index("icon", items[1])?,
            title: index("title", items[3])?,
            ..Self::default()
        };
        card.validate()?;
        Ok(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callsign_item_ids_are_mapped_to_native_rows_not_truncated() {
        let f: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/player-card-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(f["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(
            Card::from_items([30060, 30059, 0, 30061]).unwrap(),
            Card::default()
        );
        let mut items = [0; 4];
        for (kind, slot) in [("background", 0), ("icon", 1), ("title", 3)] {
            let row = f["catalog"][kind]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r[1].as_u64().unwrap() > 0)
                .unwrap();
            items[slot] = row[0].as_u64().unwrap() as u32;
            let card = Card::from_items(items).unwrap();
            let actual = match slot {
                0 => card.background,
                1 => card.icon,
                _ => card.title,
            };
            assert_eq!(u64::from(actual), row[1].as_u64().unwrap());
        }
        assert!(Card::from_items([u32::MAX, 0, 0, 0]).is_err());
        assert!(
            Card {
                rank: 70,
                ..Card::default()
            }
            .validate()
            .is_err()
        );
    }
}
