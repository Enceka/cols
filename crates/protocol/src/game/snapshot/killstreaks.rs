//! 3.3.3.3 PS progress, reward previews and five independent action slots.
use crate::lobby::channel::LobbyError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreakSnapshot {
    pub progress: u8,
    pub ids: [u8; 5],
    pub targets: [u8; 5],
    pub action_types: [u8; 5],
    pub action_weapons: [u16; 5],
}
impl StreakSnapshot {
    pub(crate) fn fields(self) -> Result<Vec<(u32, u32, u32)>, LobbyError> {
        if self.targets.iter().any(|v| *v > 31)
            || self.action_types.iter().any(|v| *v > 3)
            || self.action_weapons.iter().any(|v| *v >= 900)
            || self
                .action_types
                .iter()
                .zip(self.action_weapons)
                .any(|(t, w)| *t == 1 && w == 0)
        {
            return Err(LobbyError("invalid killstreak snapshot"));
        }
        let mut result = vec![(88, u32::from(self.progress), 8)];
        for i in 0..5 {
            result.extend([
                (126 + i as u32, u32::from(self.ids[i]), 8),
                (131 + i as u32, u32::from(self.targets[i]), 5),
                (
                    [139, 187, 100, 174, 222][i],
                    u32::from(self.action_types[i]),
                    2,
                ),
                (
                    [144, 203, 141, 173, 221][i],
                    u32::from(self.action_weapons[i]),
                    11,
                ),
            ]);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fields_match_the_independent_target_layout_and_clear_every_slot() {
        let layout: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/player-state-fields-3.3.3.3.json"
        )))
        .unwrap();
        for (field, value, width) in StreakSnapshot::default().fields().unwrap() {
            assert_eq!(value, 0);
            assert_eq!(
                layout["fields"][field as usize][1].as_u64().unwrap(),
                u64::from(width)
            );
        }
        let mut state = StreakSnapshot::default();
        state.targets[4] = 32;
        assert!(state.fields().is_err());
        state.targets[4] = 25;
        state.action_types[4] = 1;
        assert!(state.fields().is_err());
        state.action_weapons[4] = 322;
        assert!(state.fields().is_ok());
    }
}
