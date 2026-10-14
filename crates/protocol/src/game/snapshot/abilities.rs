//! 3.3.3.3 clientState abilities: five MSB-first masks and thirty values.
//! 0x525356 copies CS+0x88..0x98 to CI+0xa8..0xb8; 0x4e1a10
//! copies CS+0x9c..0x110. The local prediction consumes PS+0x4cc/+0x4e0.
use crate::lobby::channel::LobbyError;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Abilities {
    pub masks: [u32; 5],
    pub values: [f32; 30],
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ClientAbilities {
    pub current: Option<Abilities>,
    pub baseline: Option<Abilities>,
}
impl Abilities {
    pub fn validate(self) -> Result<(), LobbyError> {
        if self.values.iter().any(|v| !v.is_finite()) {
            return Err(LobbyError("nonfinite native ability value"));
        }
        Ok(())
    }
    pub(crate) fn fields(self) -> Vec<(i32, u32, usize)> {
        let mut fields = Vec::with_capacity(35);
        for (slot, value) in self.masks.into_iter().enumerate() {
            fields.push((2 + slot as i32, value, 32));
        }
        for (slot, value) in self.values.into_iter().enumerate() {
            let index = if slot < 9 { 7 + slot } else { 55 + slot - 9 };
            fields.push((index as i32, value.to_bits(), 0));
        }
        fields
    }
}
