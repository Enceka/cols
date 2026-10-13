//! 3.3.3.3 sound selectors: CG local sound at VA0x57b500/0x444910.
//! Dynamic selectors use CS3107+slot; selectors >=512 use zero-based NCS
//! category5. The reliable command accepts 1..=16000, including selector512.
use crate::{game_events::Event, lobby::LobbyError};

pub const DYNAMIC_SOUND_BASE: u16 = 3107;
pub const CONSTANT_SOUND_OFFSET: u16 = 512;
pub const MAX_SOUND_SELECTOR: u16 = 16000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundSelector(u16);

impl SoundSelector {
    pub fn dynamic(slot: u16) -> Result<Self, LobbyError> {
        if !(1..CONSTANT_SOUND_OFFSET).contains(&slot) {
            return Err(LobbyError("dynamic sound slot outside 1..511"));
        }
        Ok(Self(slot))
    }

    pub fn constant(zero_based_index: u16) -> Result<Self, LobbyError> {
        if zero_based_index > MAX_SOUND_SELECTOR - CONSTANT_SOUND_OFFSET {
            return Err(LobbyError("constant sound exceeds native command range"));
        }
        Ok(Self(CONSTANT_SOUND_OFFSET + zero_based_index))
    }

    pub fn value(self) -> u16 {
        self.0
    }

    pub fn dynamic_configstring(self) -> Option<u16> {
        (self.0 < CONSTANT_SOUND_OFFSET).then_some(DYNAMIC_SOUND_BASE + self.0)
    }

    pub fn play_command(self) -> String {
        format!("p {}", self.0)
    }

    pub fn stop_command(self) -> String {
        format!("h {}", self.0)
    }

    /// Caller owns recipient filtering, source entity and event sequence.
    /// Do not also send a local command for the same spatial sound.
    pub fn spatial_event(self) -> Event {
        Event {
            id: crate::game_events::SOUND_ALIAS,
            parameter: u32::from(self.0),
        }
    }
}

/// Native character faction selectors, independent of room/gameplay teams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceFaction {
    TaskForce,
    ShadowCompany,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceGender {
    Male,
    Female,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoicePrefix {
    faction: VoiceFaction,
    gender: VoiceGender,
    variant: u8,
}

impl VoicePrefix {
    pub fn new(
        faction: VoiceFaction,
        gender: VoiceGender,
        variant: u8,
    ) -> Result<Self, LobbyError> {
        if !matches!(variant, 1 | 2) {
            return Err(LobbyError(
                "voice variant absent from verified native families",
            ));
        }
        Ok(Self {
            faction,
            gender,
            variant,
        })
    }

    pub fn prefix(self) -> String {
        let faction = match self.faction {
            VoiceFaction::TaskForce => "UK",
            VoiceFaction::ShadowCompany => "AB",
        };
        let gender = match self.gender {
            VoiceGender::Male => 'M',
            VoiceGender::Female => 'F',
        };
        // VA0x45567f tests prefix[3] against uppercase 'F'.
        format!("{faction}_{gender}{}_", self.variant)
    }

    pub fn command(self) -> String {
        format!("s cg_vo_prefix \"{}\"", self.prefix())
    }

    /// A name still needs to be resolved against this map's captured bank.
    pub fn alias(self, suffix: &str) -> Result<String, LobbyError> {
        if suffix.is_empty()
            || suffix.len() > 96
            || !suffix
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        {
            return Err(LobbyError("invalid voice alias suffix"));
        }
        Ok(format!("{}{suffix}", self.prefix().to_ascii_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_and_constant_banks_meet_at_512_without_shifting_index_zero() {
        assert!(SoundSelector::dynamic(0).is_err());
        assert!(SoundSelector::dynamic(512).is_err());
        let dynamic = SoundSelector::dynamic(511).unwrap();
        assert_eq!(dynamic.dynamic_configstring(), Some(3618));
        let first = SoundSelector::constant(0).unwrap();
        assert_eq!(first.value(), 512);
        assert_eq!(first.play_command(), "p 512");
        assert_eq!(first.stop_command(), "h 512");
        assert_eq!(first.dynamic_configstring(), None);
        let last = SoundSelector::constant(15488).unwrap();
        assert_eq!(last.value(), 16000);
        last.spatial_event().validate().unwrap();
        assert!(SoundSelector::constant(15489).is_err());
    }

    #[test]
    fn gender_case_matches_native_breathing_test_and_alias_lookup() {
        let female = VoicePrefix::new(VoiceFaction::TaskForce, VoiceGender::Female, 2).unwrap();
        assert_eq!(female.command(), "s cg_vo_prefix \"UK_F2_\"");
        assert_eq!(female.prefix().as_bytes()[3], b'F');
        assert_eq!(
            female.alias("inform_reloading_generic").unwrap(),
            "uk_f2_inform_reloading_generic"
        );
        let male = VoicePrefix::new(VoiceFaction::ShadowCompany, VoiceGender::Male, 1).unwrap();
        assert_eq!(male.prefix(), "AB_M1_");
        for suffix in ["", "foo\" p 512", "foo\nbar", "MP_PAIN"] {
            assert!(male.alias(suffix).is_err());
        }
        assert!(VoicePrefix::new(VoiceFaction::TaskForce, VoiceGender::Male, 0).is_err());
        assert!(VoicePrefix::new(VoiceFaction::TaskForce, VoiceGender::Male, 3).is_err());
    }
}
