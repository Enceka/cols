//! Bot equipment and faction appearance shared by game modes.
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BotLoadout {
    pub class: String,
    pub weapon_name: String,
    pub animation: crate::game::ai::enemy::EnemyConfig,
    pub attachment_models: [u16; 10],
    pub personality: String,
    #[serde(default)]
    pub outfits: Option<[BotOutfit; 2]>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BotOutfit {
    pub model: u16,
    pub attachment_models: [u16; 10],
    #[serde(default)]
    pub character: Option<codol_protocol::game::snapshot::character::Character>,
}
impl BotLoadout {
    pub(crate) fn outfit(&self, team: u8) -> BotOutfit {
        self.outfits.as_ref().map_or(
            BotOutfit {
                model: self.animation.model,
                attachment_models: self.attachment_models,
                character: None,
            },
            |a| a[usize::from(team - 1)],
        )
    }
}
