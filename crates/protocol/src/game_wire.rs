//! CoDOL inner netchan primitives. This is separate from the authenticated DATA
//! envelope. The 3.3.3.3 target uses the native stream for gamestate/reliable
//! traffic; it does not use the AlphaBuild binary reliable marker.

use crate::lobby::LobbyError;

pub const FRAGMENT_SIZE: usize = 1162;
pub const SEQUENCE_MASK: u32 = 0x1fff_ffff;
pub const FRAGMENT_BIT: u32 = 0x8000_0000;
pub const ACK_BIT: u32 = 0x2000_0000;
pub const RELIABLE_WINDOW: u32 = 256;
pub const RELIABLE_COMMAND_BYTES: usize = 1024;

#[derive(Debug, Default)]
pub struct ReliableCommands {
    sequence: u32,
    acknowledge: u32,
    entries: std::collections::HashMap<u32, Vec<u8>>,
}

impl ReliableCommands {
    pub fn sequence(&self) -> u32 {
        self.sequence
    }
    pub fn acknowledge_sequence(&self) -> u32 {
        self.acknowledge
    }
    pub fn pending(&self) -> u32 {
        self.sequence.saturating_sub(self.acknowledge)
    }

    /// Queue one C1 reliable command. The ring is bounded exactly like the
    /// target's 256-entry / 1024-byte command storage.
    pub fn push(&mut self, command: &[u8]) -> Result<u32, LobbyError> {
        if command.is_empty() || command.len() >= RELIABLE_COMMAND_BYTES {
            return Err(LobbyError("reliable command size limit"));
        }
        if self.pending() >= RELIABLE_WINDOW - 1 {
            return Err(LobbyError("reliable command window full"));
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(LobbyError("reliable sequence exhausted"))?;
        self.entries.insert(self.sequence, command.to_vec());
        Ok(self.sequence)
    }

    /// Apply the client command acknowledgement carried by the next native
    /// packet. Acknowledgements cannot move backwards or past the sent range.
    pub fn acknowledge(&mut self, value: u32) -> Result<(), LobbyError> {
        if value < self.acknowledge || value > self.sequence {
            return Err(LobbyError("reliable acknowledgement outside window"));
        }
        self.acknowledge = value;
        self.entries.retain(|seq, _| *seq > value);
        Ok(())
    }

    /// Command used by native XOR for the acknowledged slot. Empty is the
    /// initial command and remains valid before the first server command.
    pub fn command_for(&self, value: u32) -> &[u8] {
        self.entries.get(&value).map_or(&[], Vec::as_slice)
    }
}

/// Target 0x46d830 sets DAT_0796f2d8 by looking up configstring index 3
/// (ECX=3 at 0x46d837), then parsing its decimal text with base 10. The
/// native XOR consumes only the low byte of this value.
pub fn native_feed_byte(server_id_configstring: &str) -> Result<u8, LobbyError> {
    let value = server_id_configstring
        .trim()
        .parse::<u32>()
        .map_err(|_| LobbyError("invalid native server-id feed"))?;
    Ok(value as u8)
}

#[derive(Clone, Debug)]
pub struct NativeXor {
    key: u8,
    command: Vec<u8>,
    cursor: usize,
    offset: usize,
}

impl NativeXor {
    fn new(key: u8, command: &[u8]) -> Self {
        Self {
            key,
            command: command.to_vec(),
            cursor: 0,
            offset: 0,
        }
    }

    fn apply(&mut self, data: &mut [u8]) {
        for byte in data {
            if !self.command.is_empty() {
                if self.cursor == self.command.len() || self.command[self.cursor] == 0 {
                    self.cursor = 0;
                }
                self.key ^= self.command[self.cursor].wrapping_shl((self.offset & 1) as u32);
                self.cursor += 1;
            }
            *byte ^= self.key;
            self.offset += 1;
        }
    }
}

/// Target 0x46bf80, used for bytes received from the server.
pub fn incoming_xor(sequence_byte: u8, challenge_byte: u8, command: &[u8], data: &mut [u8]) {
    NativeXor::new(sequence_byte ^ challenge_byte, command).apply(data);
}

/// Target 0x46bf00, used for bytes sent to the client. `feed_byte` is the
/// separate runtime value at DAT_0796f2d8; it is not part of the DTLS key.
pub fn outgoing_xor(
    sequence_byte: u8,
    challenge_byte: u8,
    feed_byte: u8,
    command: &[u8],
    data: &mut [u8],
) {
    NativeXor::new(sequence_byte ^ challenge_byte ^ feed_byte, command).apply(data);
}

/// Server-side native netchan encode from AlphaBuild's `SV_Netchan_Encode`:
/// the four-byte outgoing sequence stays clear and the body uses challenge ^
/// outgoingSequence, mixed with the last client command. This is distinct from
/// the target client's 0x46bf00 path above, which has the extra configstring
/// feed byte.
pub fn server_outgoing_xor(
    sequence: u32,
    challenge_byte: u8,
    command: &[u8],
    packet: &mut [u8],
) -> Result<(), LobbyError> {
    if packet.len() < 4 || u32::from_le_bytes(packet[..4].try_into().unwrap()) != sequence {
        return Err(LobbyError("server inner packet sequence mismatch"));
    }
    NativeXor::new(challenge_byte ^ sequence as u8, command).apply(&mut packet[4..]);
    Ok(())
}

pub fn server_packet(
    sequence: u32,
    challenge_byte: u8,
    command: &[u8],
    body: &[u8],
) -> Result<Vec<u8>, LobbyError> {
    if body.len() + 4 >= FRAGMENT_SIZE {
        return Err(LobbyError("server inner fragmentation not implemented"));
    }
    let mut packet = sequence.to_le_bytes().to_vec();
    packet.extend_from_slice(body);
    server_outgoing_xor(sequence, challenge_byte, command, &mut packet)?;
    Ok(packet)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Snapshot,
    ReliableServerCommand,
    ReliableClientCommand,
    Gamestate,
    Configstring,
    End,
    Unknown(u8),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Incoming {
    pub raw_sequence: u32,
    pub sequence: u32,
    pub dropped: u32,
    pub kind: Option<MessageKind>,
    pub body: Vec<u8>,
}

/// Decode the native stream header after the DTLS DATA envelope has been
/// opened. This intentionally does not decrypt or acknowledge packets: the
/// target's rolling XOR state is association-specific and must be supplied by
/// the authenticated game session before a caller treats `body` as trusted.
pub fn incoming(wire: &[u8], previous: Option<u32>) -> Result<Incoming, LobbyError> {
    if wire.len() < 4 {
        return Err(LobbyError("inner game packet header truncated"));
    }
    let raw = u32::from_le_bytes(wire[..4].try_into().unwrap());
    if raw & ACK_BIT != 0 {
        return Err(LobbyError("inner game acknowledgement not a message"));
    }
    let sequence = raw & SEQUENCE_MASK;
    if let Some(last) = previous {
        if sequence <= last {
            return Err(LobbyError("inner game sequence replay"));
        }
        if sequence - last > 0x1000_0000 {
            return Err(LobbyError("inner game sequence jump"));
        }
    }
    let body = wire[4..].to_vec();
    let kind = body.first().copied().map(|v| match v {
        0 => MessageKind::Snapshot,
        1 => MessageKind::ReliableServerCommand,
        2 => MessageKind::ReliableClientCommand,
        3 => MessageKind::Gamestate,
        5 => MessageKind::End,
        6 => MessageKind::Configstring,
        n => MessageKind::Unknown(n),
    });
    Ok(Incoming {
        raw_sequence: raw,
        sequence,
        dropped: previous.map_or(0, |last| sequence - last - 1),
        kind,
        body,
    })
}

/// CoDOL's native stream XOR (target VA 0x46bf80). The key and rotating
/// command bytes are association state, not the DTLS key. Callers must obtain
/// them from a verified connection before encoding a gamestate.
pub fn xor_stream(data: &mut [u8], mut key: u8, command: &[u8]) {
    let mut cursor = 0usize;
    for (index, byte) in data.iter_mut().enumerate() {
        if !command.is_empty() {
            if cursor == command.len() || command[cursor] == 0 {
                cursor = 0;
            }
            key ^= command[cursor].wrapping_shl((index & 1) as u32);
            cursor += 1;
        }
        *byte ^= key;
    }
}

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
    // Client dispatch at 0x46e6a0 routes opcode 3 to 0x46dea0 gamestate;
    // opcode 2 is the reliable client-command path.
    let mut out = vec![3];
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
        assert_eq!(message[0], 3);
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

    #[test]
    fn xor_stream_is_reversible_and_restarts_command_at_nul() {
        let original = b"gamestate wire".to_vec();
        let mut encoded = original.clone();
        xor_stream(&mut encoded, 0x37, b"\\map\\test\0ignored");
        xor_stream(&mut encoded, 0x37, b"\\map\\test\0ignored");
        assert_eq!(encoded, original);
    }

    #[test]
    fn native_xor_direction_and_chunking_are_stateful() {
        let plain = b"gamestate-message".to_vec();
        let mut encoded = plain.clone();
        outgoing_xor(0x48, 0x34, 0xa7, b"\\map\\mp_afghan\0ignored", &mut encoded);
        let mut decoded = encoded.clone();
        let command = b"\\map\\mp_afghan\0ignored";
        let mut state = NativeXor::new(0x48 ^ 0x34 ^ 0xa7, command);
        state.apply(&mut decoded[..7]);
        state.apply(&mut decoded[7..]);
        assert_eq!(decoded, plain);
        let mut wrong_direction = encoded.clone();
        incoming_xor(0x48, 0x34, command, &mut wrong_direction);
        assert_ne!(wrong_direction, plain);
    }

    #[test]
    fn server_outgoing_keeps_sequence_clear_and_uses_challenge_sequence_key() {
        let body = b"\x03gamestate";
        let packet = server_packet(7, 0x34, b"last client command\0", body).unwrap();
        assert_eq!(&packet[..4], &7u32.to_le_bytes());
        let mut decoded = packet[4..].to_vec();
        NativeXor::new(0x34 ^ 7, b"last client command\0").apply(&mut decoded);
        assert_eq!(decoded, body);
    }

    #[test]
    fn native_feed_comes_from_decimal_configstring_three_and_uses_low_byte() {
        assert_eq!(native_feed_byte("1").unwrap(), 1);
        assert_eq!(native_feed_byte("257").unwrap(), 1);
        assert!(native_feed_byte("-1").is_err());
        assert!(native_feed_byte("serverid").is_err());
    }

    #[test]
    fn incoming_validates_native_sequence_and_dispatch_opcode_without_guessing_body() {
        let first = incoming(&[1, 0, 0, 0, 3, 9, 8], None).unwrap();
        assert_eq!(first.sequence, 1);
        assert_eq!(first.dropped, 0);
        assert_eq!(first.kind, Some(MessageKind::Gamestate));
        let next = incoming(&[3, 0, 0, 0, 6, 1], Some(first.sequence)).unwrap();
        assert_eq!(next.dropped, 1);
        assert_eq!(next.kind, Some(MessageKind::Configstring));
        assert!(incoming(&[2, 0, 0, 0, 0], Some(3)).is_err());
        assert!(incoming(&[0, 0, 0, 0, 0], None).is_ok());
        assert!(incoming(&ACK_BIT.to_le_bytes(), None).is_err());
        assert!(incoming(&(FRAGMENT_BIT | 5).to_le_bytes(), None).is_ok());
    }

    #[test]
    fn reliable_window_matches_target_ack_and_ring_limits() {
        let mut ring = ReliableCommands::default();
        assert_eq!(ring.command_for(0), b"");
        for i in 0..255 {
            let text = format!("d {} value-{i}", i + 1);
            assert_eq!(ring.push(text.as_bytes()).unwrap(), i + 1);
        }
        assert_eq!(ring.pending(), 255);
        assert!(ring.push(b"overflow").is_err());
        assert!(ring.acknowledge(256).is_err());
        ring.acknowledge(128).unwrap();
        assert_eq!(ring.pending(), 127);
        assert!(ring.command_for(128).is_empty());
        assert_eq!(ring.command_for(255), b"d 255 value-254");
        assert!(ring.acknowledge(127).is_err());
        assert!(ring.push(&vec![b'x'; RELIABLE_COMMAND_BYTES]).is_err());
    }
}
