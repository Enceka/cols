//! CoDOL inner netchan primitives. This is separate from the authenticated DATA
//! envelope. The 3.3.3.3 target uses the native stream for gamestate/reliable
//! traffic; it does not use the AlphaBuild binary reliable marker.

use crate::lobby::LobbyError;

pub const FRAGMENT_SIZE: usize = 1162;

pub fn packet(sequence: u32, message: &[u8]) -> Result<Vec<u8>, LobbyError> {
    if message.len() >= FRAGMENT_SIZE {
        return Err(LobbyError("inner game fragmentation not implemented"));
    }
    let mut out = sequence.to_le_bytes().to_vec();
    out.extend_from_slice(message);
    Ok(out)
}

struct Bits {
    bytes: Vec<u8>,
    bit: usize,
}
impl Bits {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            bit: 0,
        }
    }
    fn bits(&mut self, value: u32, count: usize) {
        for i in 0..count {
            if self.bit.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> i) & 1 != 0 {
                self.bytes[self.bit / 8] |= 1 << (self.bit % 8);
            }
            self.bit += 1;
        }
    }
    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
fn string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(value.as_bytes());
    out.push(0);
}

/// Minimal target-shaped gamestate. It deliberately publishes only the map,
/// mode, server ID and system info needed to exercise the client parser. A
/// complete asset/configstring bank must replace this before claiming a match.
pub fn minimal_gamestate(
    map: &str,
    mode: &str,
    server_id: u32,
    client: u32,
) -> Result<Vec<u8>, LobbyError> {
    if map.is_empty() || mode.is_empty() || map.len() > 255 || mode.len() > 63 {
        return Err(LobbyError("invalid minimal gamestate identity"));
    }
    let mut out = vec![2];
    out.extend_from_slice(&0u32.to_le_bytes());
    string(&mut out, map);
    string(&mut out, "mp");
    out.extend_from_slice(&0u32.to_le_bytes());
    let values = [
        (
            0u32,
            format!("\\mapname\\{map}\\gametype\\{mode}\\sv_maxclients\\12"),
        ),
        (
            1u32,
            format!("\\sv_serverid\\{server_id}\\sv_fps\\20\\g_gametype\\{mode}"),
        ),
        (3u32, server_id.to_string()),
    ];
    out.push(3);
    out.extend_from_slice(&(values.len() as u16).to_le_bytes());
    let mut bits = Bits::new();
    for (index, value) in &values {
        bits.bits(0, 1);
        bits.bits(*index, 13);
        string(&mut bits.bytes, value);
    }
    out.extend_from_slice(&bits.finish());
    out.push(5);
    out.extend_from_slice(&client.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(5);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_gamestate_has_expected_header_config_indices_and_tail() {
        let message = minimal_gamestate("mp_afghan", "war", 1, 0).unwrap();
        assert_eq!(message[0], 2);
        assert_eq!(&message[5..15], b"mp_afghan\0");
        assert!(
            message
                .windows(b"\\mapname\\mp_afghan".len())
                .any(|w| w == b"\\mapname\\mp_afghan")
        );
        assert_eq!(*message.last().unwrap(), 5);
        let wire = packet(1, &message).unwrap();
        assert_eq!(&wire[..4], &[1, 0, 0, 0]);
        assert!(packet(1, &[0; FRAGMENT_SIZE]).is_err());
    }
}
