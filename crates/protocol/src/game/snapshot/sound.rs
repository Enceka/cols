//! 3.3.3.3 sound selectors: CG local sound at VA0x57b500/0x444910.
//! Dynamic selectors use CS3107+slot; selectors >=512 use zero-based NCS
//! category5. The reliable command accepts 1..=16000, including selector512.
use crate::game::gamestate::Writer;
use crate::game::snapshot::events::Event;
use crate::lobby::channel::LobbyError;

pub const DYNAMIC_SOUND_BASE: u16 = 3107;
pub const CONSTANT_SOUND_OFFSET: u16 = 512;
pub const MAX_SOUND_SELECTOR: u16 = 16000;
pub const FIRST_SOUND_ENTITY: u16 = 1668;
pub const END_SOUND_ENTITY: u16 = 1700;

/// Source player and temporary one-shot playback latch have separate identities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundSnapshot {
    pub entity_number: u16,
    pub source: u8,
    pub origin: [f32; 3],
    pub selector: SoundSelector,
}

pub(crate) fn write_sounds(
    out: &mut Writer,
    sounds: &[SoundSnapshot],
    baselines: &[SoundSnapshot],
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    for list in [sounds, baselines] {
        for (i, sound) in list.iter().enumerate() {
            if !(FIRST_SOUND_ENTITY..END_SOUND_ENTITY).contains(&sound.entity_number)
                || sound.source > 18
                || sound
                    .origin
                    .iter()
                    .any(|v| !v.is_finite() || !(-32768.0..=32767.0).contains(v))
                || (i > 0 && list[i - 1].entity_number >= sound.entity_number)
            {
                return Err(LobbyError("invalid spatial sound snapshot"));
            }
        }
    }
    if establish_baseline {
        return Ok(());
    }
    let numbers: std::collections::BTreeSet<_> = sounds
        .iter()
        .chain(baselines)
        .map(|s| s.entity_number)
        .collect();
    for number in numbers {
        let current = sounds.iter().find(|s| s.entity_number == number);
        let base = baselines.iter().find(|s| s.entity_number == number);
        if current == base {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(u32::from(number), 11);
        let Some(sound) = current else {
            out.bits(1, 1);
            continue;
        };
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(1, 1);
        out.bits(9, 6);
        gap(out, 1);
        out.bytes.push(25); // ET_EVENTS22 + EV_SOUND_ALIAS3
        let mut previous = 0;
        for (index, axis) in [(2, 0), (3, 1), (4, 2)] {
            gap(out, index - previous);
            previous = index;
            out.bits(1, 1);
            out.bits(
                u32::from(sound.origin[axis].round_ties_even() as i16 as u16),
                16,
            );
        }
        gap(out, 5 - previous);
        // Descriptor22 field5: codec-93 uses the event's 14-bit parameter.
        out.bits(u32::from(sound.selector.value()), 14);
        gap(out, 4); // field9: ES+0x80, positive-width11 source entity
        out.bits(u32::from(sound.source != 0), 1);
        if sound.source != 0 {
            let value = sound.source ^ base.map_or(0, |b| b.source);
            out.bits(u32::from(value) & 7, 3);
            out.bytes.push(value >> 3);
        }
    }
    Ok(())
}

fn gap(out: &mut Writer, distance: u32) {
    out.bits(u32::from(distance == 1), 1);
    if distance != 1 {
        let mut remaining = distance - 1;
        while remaining >= 7 {
            out.bits(7, 3);
            remaining -= 7;
        }
        out.bits(remaining, 3);
    }
}

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
            id: crate::game::snapshot::events::SOUND_ALIAS,
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
    fn spatial_sound_uses_descriptor22_parameter_and_source_and_removes_expired_latch() {
        use crate::game::channel::wire::Reader;
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/bullet-impact-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(fixture["fields"][5], serde_json::json!([168, -93, 0]));
        assert_eq!(fixture["fields"][9], serde_json::json!([128, 11, 0]));
        let sound = SoundSnapshot {
            entity_number: FIRST_SOUND_ENTITY,
            source: 7,
            origin: [-3.0, 12.0, 90.0],
            selector: SoundSelector::constant(1126).unwrap(),
        };
        let mut out = Writer::default();
        write_sounds(&mut out, &[sound], &[], false).unwrap();
        let mut r = Reader::new(&out.bytes);
        assert_eq!(r.bits(2).unwrap(), 2);
        assert_eq!(r.bits(11).unwrap(), u32::from(FIRST_SOUND_ENTITY));
        assert_eq!(r.bits(3).unwrap(), 6);
        let last = r.bits(6).unwrap();
        assert_eq!(last, 9);
        let mut index = -1i32;
        let mut values = std::collections::BTreeMap::new();
        while index < last as i32 {
            let mut gap = 1;
            if r.bits(1).unwrap() == 0 {
                loop {
                    let part = r.bits(3).unwrap();
                    gap += part;
                    if part != 7 {
                        break;
                    }
                }
            }
            index += gap as i32;
            let row = &fixture["fields"][index as usize];
            let offset = row[0].as_u64().unwrap();
            let value = match row[1].as_i64().unwrap() {
                -77 => u32::from(r.u8().unwrap()),
                -83..=-81 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    r.bits(16).unwrap()
                }
                -93 => r.bits(14).unwrap(),
                11 => {
                    if r.bits(1).unwrap() == 0 {
                        0
                    } else {
                        r.bits(3).unwrap() | (u32::from(r.u8().unwrap()) << 3)
                    }
                }
                _ => panic!("unexpected sound field"),
            };
            values.insert(offset, value);
        }
        assert_eq!(values[&4], 25);
        assert_eq!(values[&168], 1638);
        assert_eq!(values[&128], 7);
        assert_eq!(values[&24], u32::from((-3i16) as u16));
        let mut same = Writer::default();
        write_sounds(&mut same, &[sound], &[sound], false).unwrap();
        assert!(same.bytes.is_empty());
        let mut removal = Writer::default();
        write_sounds(&mut removal, &[], &[sound], false).unwrap();
        let mut r = Reader::new(&removal.bytes);
        assert_eq!(r.bits(2).unwrap(), 2);
        assert_eq!(r.bits(11).unwrap(), u32::from(FIRST_SOUND_ENTITY));
        assert_eq!(r.bits(1).unwrap(), 1);
        let mut full = Writer::default();
        write_sounds(&mut full, &[sound], &[], true).unwrap();
        assert!(full.bytes.is_empty());
        let mut invalid = sound;
        invalid.entity_number = 1700;
        assert!(write_sounds(&mut Writer::default(), &[invalid], &[], false).is_err());
    }

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
