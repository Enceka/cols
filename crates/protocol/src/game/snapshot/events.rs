//! Predictable player events for client 3.3.3.3 (EXE SHA-256 e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b).
//!
//! Names: VA 0xce4390. Parameter widths: VA 0xc56d70, 162 entries.
//! PS has counter +0xf4 and four interleaved {id, parameter} pairs at +0xf8.
//! Field 1 uses codec -75; fields 21..24 use codec -94. These layouts and
//! numbers differ from the Alpha C1 reference.
use crate::game::gamestate::Writer;
use crate::lobby::channel::LobbyError;

pub const SOUND_ALIAS: u8 = 3;
pub const MISSILE_REMOTE_BOOST: u8 = 130;
pub const ITEM_PICKUP: u8 = 16;
pub const NO_AMMO: u8 = 18;
pub const RESET_ADS: u8 = 23;
pub const RELOAD: u8 = 24;
pub const RELOAD_FROM_EMPTY: u8 = 25;
pub const RELOAD_START: u8 = 26;
pub const RELOAD_END: u8 = 27;
pub const RELOAD_START_NOTIFY: u8 = 28;
pub const RELOAD_ADD_AMMO: u8 = 29;
pub const RAISE_WEAPON: u8 = 30;
pub const FIRST_RAISE_WEAPON: u8 = 31;
pub const PUT_AWAY_WEAPON: u8 = 32;
pub const ALT_WEAPON_SWITCH: u8 = 33;
pub const WEAPON_SWITCH_STARTED: u8 = 34;
pub const FIRE_WEAPON: u8 = 36;
pub const FIRE_WEAPON_LAST_SHOT: u8 = 37;
pub const RECHAMBER_WEAPON: u8 = 40;
pub const EJECT_BRASS: u8 = 41;
pub const MELEE_SWIPE: u8 = 53;
pub const FIRE_MELEE: u8 = 54;
pub const MELEE_INIT: u8 = 55;
pub const MELEE_END: u8 = 62;
pub const FOOTSTEP_SPRINT: u8 = 141;
pub const FOOTSTEP_RUN: u8 = 142;
pub const FOOTSTEP_WALK: u8 = 143;
pub const FOOTSTEP_PRONE: u8 = 144;
pub const JUMP: u8 = 145;
pub const LANDING_HARD: u8 = 146;
pub const LANDING_PAIN: u8 = 147;
pub const DTP_LAUNCH: u8 = 150;
pub const DTP_LAND: u8 = 151;

// Directly extracted from the checked target image, not inferred from C1.
const PARAMETER_BITS: [u8; 162] = [
    11, 0, 6, 14, 14, 32, 32, 32, 32, 0, 14, 6, 2, 2, 2, 2, 10, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 10, 10, 5, 5, 8, 8, 0, 0, 5, 5, 0, 5, 5, 5, 5, 5, 5, 5, 5, 0, 0, 0, 2, 2, 2,
    2, 2, 2, 0, 10, 10, 10, 2, 2, 0, 11, 0, 5, 5, 11, 8, 8, 8, 0, 0, 0, 8, 0, 8, 8, 8, 8, 8, 8, 8,
    8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 9, 8, 18, 18, 18, 18, 9, 10, 9, 10, 16, 16, 16, 16, 0, 19, 17,
    18, 16, 8, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 10, 19, 10, 0, 0, 0, 0, 0, 8, 0, 0, 6, 6, 6, 6, 6,
    10, 13, 0, 0, 5, 5, 11, 11, 0, 0, 0, 0, 5, 0, 0, 0,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Event {
    pub id: u8,
    pub parameter: u32,
}

impl Event {
    pub fn validate(self) -> Result<(), LobbyError> {
        let bits = *PARAMETER_BITS
            .get(usize::from(self.id))
            .ok_or(LobbyError("unsupported player event"))?;
        if bits < 32 && self.parameter >= (1u32 << bits) {
            return Err(LobbyError("player event parameter exceeds target width"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventQueue {
    pub sequence: u8,
    pub slots: [Event; 4],
}

impl EventQueue {
    /// The sequence advances only when gameplay produces an event. Snapshot
    /// retransmission must preserve both this sequence and the four slots.
    pub fn push(&mut self, id: u8, parameter: u32) -> Result<(), LobbyError> {
        let event = Event { id, parameter };
        event.validate()?;
        self.slots[usize::from(self.sequence & 3)] = event;
        self.sequence = self.sequence.wrapping_add(1);
        Ok(())
    }

    pub fn validate(&self) -> Result<(), LobbyError> {
        self.slots.iter().try_for_each(|event| event.validate())
    }
}

/// Codec -75, absolute branch against the snapshot's zero PS baseline.
/// 0x5a29e0 uses 0x5a2510 with width 8: a raw byte, not eight pool bits.
pub(crate) fn write_sequence(out: &mut Writer, sequence: u8) {
    out.bits(0, 1);
    out.bytes.push(sequence);
}

/// Codec -94 against a zero PS baseline. Caller validates before writing.
/// 0x5a2240 reads the event ID as a raw byte; parameters use pool bits.
pub(crate) fn write_event(out: &mut Writer, event: Event) {
    out.bits(1, 1); // Explicit event ID (including empty slots).
    out.bytes.push(event.id);
    let bits = PARAMETER_BITS[usize::from(event.id)];
    if bits != 0 {
        out.bits(u32::from(event.parameter != 0), 1);
        if event.parameter != 0 {
            out.bits(event.parameter, usize::from(bits));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::channel::wire::Reader;

    #[test]
    fn measured_event_codec_keeps_raw_bytes_separate_from_pool_bits() {
        let mut out = Writer::default();
        out.bits(5, 3); // Earlier PS data has already opened a bit pool.
        write_sequence(&mut out, 254);
        write_event(
            &mut out,
            Event {
                id: RELOAD,
                parameter: 0,
            },
        );
        write_event(
            &mut out,
            Event {
                id: WEAPON_SWITCH_STARTED,
                parameter: 352,
            },
        );
        write_event(
            &mut out,
            Event {
                id: FIRE_WEAPON,
                parameter: 0,
            },
        );
        write_event(&mut out, Event::default());
        out.bytes.push(0xa7); // Following raw section must remain readable.

        let mut read = Reader::new(&out.bytes);
        assert_eq!(read.bits(3).unwrap(), 5);
        assert_eq!(read.bits(1).unwrap(), 0);
        assert_eq!(read.u8().unwrap(), 254);
        assert_eq!(read.bits(1).unwrap(), 1);
        assert_eq!(read.u8().unwrap(), 24);
        // Reload has no parameter gate at all.
        assert_eq!(read.bits(1).unwrap(), 1);
        assert_eq!(read.u8().unwrap(), 34);
        assert_eq!(read.bits(1).unwrap(), 1);
        assert_eq!(read.bits(10).unwrap(), 352);
        assert_eq!(read.bits(1).unwrap(), 1);
        assert_eq!(read.u8().unwrap(), 36);
        assert_eq!(read.bits(1).unwrap(), 0); // Five-bit parameter remains zero.
        assert_eq!(read.bits(1).unwrap(), 1);
        assert_eq!(read.u8().unwrap(), 0);
        assert_eq!(read.bits(1).unwrap(), 0); // EV_NONE still has an 11-bit parameter.
        assert_eq!(read.u8().unwrap(), 0xa7);
    }

    #[test]
    fn ring_preserves_latest_four_across_counter_wrap_and_rejects_bad_parameters() {
        let mut queue = EventQueue {
            sequence: 254,
            ..EventQueue::default()
        };
        for weapon in 350..355 {
            queue.push(WEAPON_SWITCH_STARTED, weapon).unwrap();
        }
        assert_eq!(queue.sequence, 3);
        assert_eq!(
            queue.slots.map(|event| event.parameter),
            [352, 353, 354, 351]
        );
        let before = queue;
        assert!(queue.push(RELOAD, 1).is_err());
        assert!(queue.push(FIRE_WEAPON, 32).is_err());
        assert!(queue.push(WEAPON_SWITCH_STARTED, 1024).is_err());
        assert!(queue.push(162, 0).is_err());
        assert_eq!(queue, before);
        queue.push(5, u32::MAX).unwrap(); // Measured pitch event is full-width.
    }
}
