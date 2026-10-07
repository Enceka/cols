//! Target 3.3.3.3 gamestate: 0x46dea0 and configstrings reader 0x46db20.
use crate::{game_wire::SERVER_EOF, lobby::LobbyError};

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bit: usize,
}
impl Writer {
    fn bits(&mut self, value: u32, count: usize) {
        for i in 0..count {
            if self.bit.is_multiple_of(8) {
                self.bit = self.bytes.len() * 8;
                self.bytes.push(0);
            }
            self.bytes[self.bit / 8] |= (((value >> i) & 1) as u8) << (self.bit % 8);
            self.bit += 1;
        }
    }
    fn string(&mut self, value: &str) {
        self.bytes.extend_from_slice(value.as_bytes());
        self.bytes.push(0);
    }
}

pub struct Gamestate<'a> {
    pub command_sequence: u32,
    pub map: &'a str,
    pub zone: &'a str,
    pub ncs_crc: u32,
    pub configstrings: &'a [(u16, String)],
    pub client_number: u32,
    pub checksum_feed: u32,
}
impl Gamestate<'_> {
    pub fn encode(&self) -> Result<Vec<u8>, LobbyError> {
        let identity = |s: &str| {
            !s.is_empty()
                && s.len() < 32
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        };
        if !identity(self.map)
            || !identity(self.zone)
            || self.client_number >= 19
            || self.command_sequence > i32::MAX as u32
            || self.configstrings.len() > 7058
        {
            return Err(LobbyError("invalid gamestate header"));
        }
        let mut out = Writer::default();
        out.bytes.push(3);
        out.bytes
            .extend_from_slice(&self.command_sequence.to_le_bytes());
        out.string(self.map);
        out.string(self.zone);
        out.bytes.extend_from_slice(&self.ncs_crc.to_le_bytes());
        out.bytes.push(4);
        out.bytes
            .extend_from_slice(&(self.configstrings.len() as u16).to_le_bytes());
        let mut previous = None;
        for (index, value) in self.configstrings {
            if *index >= 7058
                || previous.is_some_and(|p| p >= *index)
                || value.len() >= 1024
                || value.contains('\0')
            {
                return Err(LobbyError("invalid gamestate configstring"));
            }
            // First index is relative to -1. Byte strings leave the remaining
            // pool bits intact; a new pool byte starts at the current cursor.
            let consecutive = previous.map_or(*index == 0, |p| p + 1 == *index);
            out.bits(u32::from(consecutive), 1);
            if !consecutive {
                out.bits(u32::from(*index), 13);
            }
            out.string(value);
            previous = Some(*index);
        }
        out.bits(0, 10); // compact integer overrides
        out.bits(0, 10); // compact weapon overrides
        out.bytes.push(SERVER_EOF);
        out.bytes
            .extend_from_slice(&self.client_number.to_le_bytes());
        out.bytes
            .extend_from_slice(&self.checksum_feed.to_le_bytes());
        out.bytes.push(SERVER_EOF);
        Ok(out.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamestate_matches_independent_target_layout_vector() {
        // First index 0 = one increment bit, then "x". Index 3 uses 0+13
        // absolute bits, filling the old pool before reserving the next byte.
        let values = [(0, "x".to_owned()), (3, "1".to_owned())];
        let message = Gamestate {
            command_sequence: 1,
            map: "m",
            zone: "mp",
            ncs_crc: 0x12345678,
            configstrings: &values,
            client_number: 0,
            checksum_feed: 0xabcdef01,
        }
        .encode()
        .unwrap();
        assert_eq!(
            message,
            [
                3, 1, 0, 0, 0, b'm', 0, b'm', b'p', 0, 0x78, 0x56, 0x34, 0x12, 4, 2, 0, 0x0d, b'x',
                0, 0, b'1', 0, 0, 0, 0, 6, 0, 0, 0, 0, 1, 0xef, 0xcd, 0xab, 6
            ]
        );
    }

    #[test]
    fn invalid_strings_and_indices_are_rejected() {
        for values in [
            vec![(3, "1".into()), (3, "2".into())],
            vec![(7058, "x".into())],
            vec![(0, "a\0b".into())],
            vec![(0, "a".repeat(1024))],
        ] {
            assert!(
                Gamestate {
                    command_sequence: 1,
                    map: "mp_afghan",
                    zone: "mp",
                    ncs_crc: 123,
                    configstrings: &values,
                    client_number: 0,
                    checksum_feed: 0
                }
                .encode()
                .is_err()
            );
        }
    }
}
