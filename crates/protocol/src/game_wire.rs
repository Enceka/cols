//! CODOL 3.3.3.3 native netchan, inside authenticated DTLS DATA.
//! C2S builder: 0x4602a0/0x46bff0; S2C reader: 0x467520/0x46bf80.
use crate::lobby::LobbyError;
use std::collections::BTreeMap;

pub const SEQUENCE_MASK: u32 = 0x1fff_ffff;
pub const FRAGMENT_BIT: u32 = 0x8000_0000;
pub const ACK_BIT: u32 = 0x2000_0000;
pub const FRAGMENT_SIZE: usize = 0x482;
pub const RELIABLE_WINDOW: u32 = 256;
pub const RELIABLE_COMMAND_BYTES: usize = 1024;
pub const SERVER_EOF: u8 = 6;

/// Keep exact sequence identities, including acknowledged slots: incoming
/// packets mix those command bytes into XOR until the ring overwrites them.
#[derive(Debug, Default, Clone)]
pub struct ReliableCommands {
    sequence: u32,
    sent: u32,
    acknowledged: u32,
    entries: BTreeMap<u32, Vec<u8>>,
}
impl ReliableCommands {
    pub fn sequence(&self) -> u32 {
        self.sequence
    }
    pub fn acknowledged(&self) -> u32 {
        self.acknowledged
    }
    pub fn push(&mut self, command: &[u8]) -> Result<u32, LobbyError> {
        if command.len() >= RELIABLE_COMMAND_BYTES || command.contains(&0) {
            return Err(LobbyError("invalid reliable command size/NUL"));
        }
        if self.sequence - self.acknowledged >= RELIABLE_WINDOW - 1 {
            return Err(LobbyError("reliable command window full"));
        }
        let next = self
            .sequence
            .checked_add(1)
            .filter(|n| *n <= i32::MAX as u32)
            .ok_or(LobbyError("reliable sequence exhausted"))?;
        self.entries.insert(next, command.to_vec());
        self.sequence = next;
        self.entries.retain(|seq, _| next - *seq < RELIABLE_WINDOW);
        Ok(next)
    }
    pub fn mark_sent(&mut self, sequence: u32) -> Result<(), LobbyError> {
        if sequence < self.sent || sequence > self.sequence {
            return Err(LobbyError("invalid reliable sent range"));
        }
        self.sent = sequence;
        Ok(())
    }
    pub fn acknowledge(&mut self, value: u32) -> Result<(), LobbyError> {
        if value < self.acknowledged || value > self.sent {
            return Err(LobbyError("reliable ACK outside sent range"));
        }
        self.command_for(value)?;
        self.acknowledged = value;
        Ok(())
    }
    pub fn command_for(&self, sequence: u32) -> Result<&[u8], LobbyError> {
        if sequence == 0 && self.sequence < RELIABLE_WINDOW {
            return Ok(&[]);
        }
        self.entries
            .get(&sequence)
            .map(Vec::as_slice)
            .ok_or(LobbyError("reliable command slot unavailable"))
    }
}

/// Apply once to a complete message before fragmentation/after reassembly.
/// The command is a C string; bytes after its first NUL never participate.
pub fn xor_stream(data: &mut [u8], mut key: u8, command: &[u8]) {
    let end = command
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(command.len());
    let command = &command[..end];
    for (i, byte) in data.iter_mut().enumerate() {
        if !command.is_empty() {
            key ^= command[i % command.len()].wrapping_shl((i & 1) as u32);
        }
        *byte ^= key;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientHeader {
    pub sequence: u32,
    pub server_id: u8,
    pub message_ack: u32,
    pub reliable_ack: u32,
}
impl ClientHeader {
    pub fn parse(packet: &[u8]) -> Result<Self, LobbyError> {
        if packet.len() < 14 || packet.len() > 2048 {
            return Err(LobbyError("client native packet size"));
        }
        let read = |at| u32::from_le_bytes(packet[at..at + 4].try_into().expect("bounded header"));
        let sequence = read(0);
        if sequence == 0 || sequence & !SEQUENCE_MASK != 0 {
            return Err(LobbyError(
                "unsupported client native sequence/fragment/ACK",
            ));
        }
        let header = Self {
            sequence,
            server_id: packet[4],
            message_ack: read(5),
            reliable_ack: read(9),
        };
        if header.message_ack > SEQUENCE_MASK || header.reliable_ack > i32::MAX as u32 {
            return Err(LobbyError("invalid client native ACK"));
        }
        Ok(header)
    }
}

/// C2S: seq32, serverId8, messageAck32, reliableAck32 are clear (13 bytes).
pub fn open_client(
    packet: &[u8],
    challenge: u32,
    server_command: &[u8],
) -> Result<(ClientHeader, Vec<u8>), LobbyError> {
    let header = ClientHeader::parse(packet)?;
    let mut body = packet[13..].to_vec();
    xor_stream(
        &mut body,
        challenge as u8 ^ header.server_id ^ header.message_ack as u8,
        server_command,
    );
    Ok((header, body))
}

/// S2C: BOTH seq32 and clientCommandAck32 remain clear (8 bytes).
/// The high bit of ACK selects zlib; this writer emits uncompressed messages.
pub fn server_packet(
    sequence: u32,
    client_ack: u32,
    challenge: u32,
    client_command: &[u8],
    body: &[u8],
) -> Result<Vec<u8>, LobbyError> {
    if sequence == 0
        || sequence > SEQUENCE_MASK
        || client_ack > i32::MAX as u32
        || body.len() + 4 >= FRAGMENT_SIZE
    {
        return Err(LobbyError(
            "unsupported server native packet size/sequence/ACK",
        ));
    }
    let mut packet = sequence.to_le_bytes().to_vec();
    packet.extend_from_slice(&client_ack.to_le_bytes());
    packet.extend_from_slice(body);
    xor_stream(
        &mut packet[8..],
        sequence as u8 ^ challenge as u8,
        client_command,
    );
    Ok(packet)
}

/// Target bit reader 0x40c370/0x5a13a0: byte fields advance the byte cursor
/// without discarding unused bits in the previously reserved pool byte.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    bit: usize,
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            bit: 0,
        }
    }
    pub fn bits(&mut self, count: usize) -> Result<u32, LobbyError> {
        if count > 32 {
            return Err(LobbyError("native bit count"));
        }
        let mut value = 0;
        for i in 0..count {
            if self.bit.is_multiple_of(8) {
                if self.at >= self.bytes.len() {
                    return Err(LobbyError("native bit stream truncated"));
                }
                self.bit = self.at * 8;
                self.at += 1;
            }
            value |= u32::from((self.bytes[self.bit / 8] >> (self.bit % 8)) & 1) << i;
            self.bit += 1;
        }
        Ok(value)
    }
    pub fn u32(&mut self) -> Result<u32, LobbyError> {
        let bytes = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or(LobbyError("native u32 truncated"))?;
        self.at += 4;
        Ok(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
    }
    pub fn string(&mut self) -> Result<&'a [u8], LobbyError> {
        let left = &self.bytes[self.at..];
        let len = left
            .iter()
            .position(|b| *b == 0)
            .filter(|n| *n < RELIABLE_COMMAND_BYTES)
            .ok_or(LobbyError("native command terminator/limit"))?;
        self.at += len + 1;
        Ok(&left[..len])
    }
    pub fn finish(&self) -> Result<(), LobbyError> {
        if self.at != self.bytes.len() {
            return Err(LobbyError("native trailing bytes"));
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClientCommands {
    pub commands: Vec<(u32, Vec<u8>)>,
}
pub fn client_commands(body: &[u8]) -> Result<ClientCommands, LobbyError> {
    let mut reader = Reader::new(body);
    let mut commands = Vec::new();
    loop {
        match reader.bits(3)? {
            2 => {
                if commands.len() >= 255 {
                    return Err(LobbyError("native command count limit"));
                }
                let sequence = reader.u32()?;
                if sequence == 0 || sequence > i32::MAX as u32 {
                    return Err(LobbyError("invalid client command sequence"));
                }
                commands.push((sequence, reader.string()?.to_vec()));
            }
            3 => {
                reader.finish()?;
                return Ok(ClientCommands { commands });
            }
            _ => {
                return Err(LobbyError(
                    "unsupported client native opcode (movement pending)",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_headers_and_xor_vectors() {
        // Hand-calculated: C2S key=0x34^0x09^0x02=0x3f; EOF=3.
        let client = [1, 0, 0, 0, 9, 2, 0, 0, 0, 0, 0, 0, 0, 0x3c];
        let (header, body) = open_client(&client, 0x34, b"").unwrap();
        assert_eq!(header.server_id, 9);
        assert_eq!(header.message_ack, 2);
        assert_eq!(body, [3]);
        // S2C key=0x34^7=0x33, command "ab": masks 0x52,0x96,0xf7.
        let packet = server_packet(7, 2, 0x34, b"ab", &[1, 2, 6]).unwrap();
        assert_eq!(packet, [7, 0, 0, 0, 2, 0, 0, 0, 0x53, 0x94, 0xf1]);
        let mut body = [6];
        xor_stream(&mut body, 0x33, b"\0ignored");
        assert_eq!(body, [0x35]);
        for n in 0..14 {
            assert!(ClientHeader::parse(&client[..n]).is_err());
        }
        for flags in [FRAGMENT_BIT, ACK_BIT, 0x4000_0000] {
            let mut invalid = client;
            invalid[..4].copy_from_slice(&(flags | 1).to_le_bytes());
            assert!(ClientHeader::parse(&invalid).is_err());
        }
    }

    #[test]
    fn captured_target_keepalive_header_is_c2s_not_server_body() {
        // Captured repeatedly after heartbeat in client-1791335109328:
        // seq=1, serverId=0, messageAck=0, reliableAck=0, one body byte.
        let packet = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x76];
        let (header, body) = open_client(&packet, 0, b"").unwrap();
        assert_eq!(
            header,
            ClientHeader {
                sequence: 1,
                server_id: 0,
                message_ack: 0,
                reliable_ack: 0
            }
        );
        assert_eq!(body, [0x76]);
    }

    #[test]
    fn captured_keepalive_decrypts_to_client_eof_with_challenge_key() {
        // client-1791312016136: challenge low byte 0x75, body 0x76.
        // With empty acknowledged command and zero header fields, 0x76 ^ 0x75 = clc_EOF(3).
        let packet = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x76];
        let (_, body) = open_client(&packet, 0x75, b"").unwrap();
        assert_eq!(body, [3]);
    }
    #[test]
    fn command_pool_reuses_bits_across_byte_fields() {
        // Commands 2/2/EOF3: the EOF consumes two old pool bits and one new.
        let body = [0xd2, 1, 0, 0, 0, b'a', 0, 2, 0, 0, 0, b'b', 0, 0];
        let decoded = client_commands(&body).unwrap();
        assert_eq!(
            decoded.commands,
            vec![(1, b"a".to_vec()), (2, b"b".to_vec())]
        );
        for n in 0..body.len() {
            assert!(client_commands(&body[..n]).is_err());
        }
        assert!(client_commands(&[0]).is_err());
        assert!(client_commands(&[3, 0]).is_err());
    }
    #[test]
    fn acknowledged_commands_survive_for_xor_and_unsent_ack_is_rejected() {
        let mut ring = ReliableCommands::default();
        assert_eq!(ring.command_for(0).unwrap(), b"");
        for _ in 0..255 {
            ring.push(b"command").unwrap();
        }
        assert!(ring.push(b"overflow").is_err());
        assert!(ring.acknowledge(128).is_err());
        ring.mark_sent(255).unwrap();
        ring.acknowledge(128).unwrap();
        assert_eq!(ring.command_for(128).unwrap(), b"command");
        assert!(ring.acknowledge(127).is_err());
        assert!(ring.acknowledge(256).is_err());
        ring.acknowledge(255).unwrap();
        for _ in 0..129 {
            ring.push(b"new").unwrap();
        }
        assert!(ring.command_for(128).is_err());
        assert!(ring.command_for(0).is_err());
        assert!(ring.push(b"x\0y").is_err());
    }
}
