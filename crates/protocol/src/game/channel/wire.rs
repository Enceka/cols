//! CODOL 3.3.3.3 native netchan, inside authenticated DTLS DATA.
//! C2S builder: 0x4602a0/0x46bff0; S2C reader: 0x467520/0x46bf80.
use crate::lobby::channel::LobbyError;
use std::collections::BTreeMap;

pub const SEQUENCE_MASK: u32 = 0x1fff_ffff;
pub const FRAGMENT_BIT: u32 = 0x8000_0000;
pub const ACK_BIT: u32 = 0x2000_0000;
pub const FRAGMENT_SIZE: usize = 0x482;
pub const RELIABLE_WINDOW: u32 = 256;
pub const RELIABLE_COMMAND_BYTES: usize = 1024;
/// Binary reader 0x46e470 reserves four bytes for its native ring header.
pub const BINARY_RELIABLE_BYTES: usize = RELIABLE_COMMAND_BYTES - 4;
pub const SERVER_EOF: u8 = 6;

#[derive(Debug, Clone)]
struct ReliableCommand {
    /// Text omits its trailing NUL. Binary retains the native !/NUL/length
    /// header and payload, so all existing XOR consumers see the native key.
    ring: Vec<u8>,
    binary: bool,
}

/// Keep exact sequence identities, including acknowledged slots: incoming
/// packets mix those command bytes into XOR until the ring overwrites them.
#[derive(Debug, Default, Clone)]
pub struct ReliableCommands {
    sequence: u32,
    sent: u32,
    acknowledged: u32,
    entries: BTreeMap<u32, ReliableCommand>,
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
        self.push_entry(ReliableCommand {
            ring: command.to_vec(),
            binary: false,
        })
    }
    /// Store a binary reliable payload exactly as reader 0x46e470 does:
    /// [!, NUL, payload length LE16, payload]. This shares the text sequence
    /// and ACK window; embedded NUL is valid and never terminates the payload.
    pub fn push_binary(&mut self, payload: &[u8]) -> Result<u32, LobbyError> {
        if payload.len() > BINARY_RELIABLE_BYTES {
            return Err(LobbyError("binary reliable command exceeds native limit"));
        }
        let mut ring = vec![b'!', 0];
        ring.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        ring.extend_from_slice(payload);
        self.push_entry(ReliableCommand { ring, binary: true })
    }
    fn push_entry(&mut self, entry: ReliableCommand) -> Result<u32, LobbyError> {
        if self.sequence - self.acknowledged >= RELIABLE_WINDOW - 1 {
            return Err(LobbyError("reliable command window full"));
        }
        let next = self
            .sequence
            .checked_add(1)
            .filter(|n| *n <= i32::MAX as u32)
            .ok_or(LobbyError("reliable sequence exhausted"))?;
        self.entries.insert(next, entry);
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
            .map(|entry| entry.ring.as_slice())
            .ok_or(LobbyError("reliable command slot unavailable"))
    }
    /// Encode one retained command as a server-message record, without EOF.
    /// Text uses opcode 1 and a NUL terminator; binary uses opcode 2, a LE16
    /// payload length and the complete bytes, without the native ring header.
    pub fn encode_command(&self, sequence: u32) -> Result<Vec<u8>, LobbyError> {
        let entry = self
            .entries
            .get(&sequence)
            .ok_or(LobbyError("reliable command slot unavailable"))?;
        let mut frame = vec![if entry.binary { 2 } else { 1 }];
        frame.extend_from_slice(&sequence.to_le_bytes());
        if entry.binary {
            frame.extend_from_slice(&entry.ring[2..]);
        } else {
            frame.extend_from_slice(&entry.ring);
            frame.push(0);
        }
        Ok(frame)
    }
    /// Encode all unacknowledged commands in sequence order. The caller adds
    /// the snapshot/EOF and marks the last transmitted sequence after sending.
    pub fn encode_frames(&self) -> Result<Vec<u8>, LobbyError> {
        let mut frames = Vec::new();
        for sequence in self.acknowledged + 1..=self.sequence {
            frames.extend_from_slice(&self.encode_command(sequence)?);
        }
        Ok(frames)
    }
    /// Fit complete reliable records within one native message. A large
    /// scoreboard can need several messages; never split a binary J record.
    pub fn encode_frames_bounded(&self, budget: usize) -> Result<(Vec<u8>, u32), LobbyError> {
        let mut frames = Vec::new();
        let mut last = self.sent;
        for sequence in self.acknowledged + 1..=self.sequence {
            let frame = self.encode_command(sequence)?;
            if frames.len() + frame.len() > budget {
                break;
            }
            frames.extend_from_slice(&frame);
            last = last.max(sequence);
        }
        Ok((frames, last))
    }
    /// Append atomically, leaving an existing message unchanged on failure.
    pub fn append_frames(&self, out: &mut Vec<u8>) -> Result<(), LobbyError> {
        let frames = self.encode_frames()?;
        out.extend_from_slice(&frames);
        Ok(())
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

/// Initial resource lists can exceed one raw datagram. Target0x467520 reads
/// ACK bit31, XORs the body, then0x46e6a0/0x659040 inflates zlib into128KiB.
/// Keep small messages byte-for-byte unchanged. This single-packet interface
/// rejects messages requiring fragments; streaming callers use the plural API.
pub fn compact_server_packet(
    sequence: u32,
    client_ack: u32,
    challenge: u32,
    client_command: &[u8],
    body: &[u8],
) -> Result<Vec<u8>, LobbyError> {
    let mut packets =
        compact_server_packets(sequence, client_ack, challenge, client_command, body)?;
    if packets.len() != 1 {
        return Err(LobbyError(
            "compressed server message requires fragmentation",
        ));
    }
    Ok(packets.remove(0))
}

/// Native 0x5a5960 reassembly uses an index byte, last-index byte, stride
/// short and payload-length short. ACK32 and XOR/zlib data are reassembled
/// before 0x467520 decodes them; these are not offset/length-only fragments.
pub fn compact_server_packets(
    sequence: u32,
    client_ack: u32,
    challenge: u32,
    client_command: &[u8],
    body: &[u8],
) -> Result<Vec<Vec<u8>>, LobbyError> {
    if body.len() + 4 < FRAGMENT_SIZE {
        return Ok(vec![server_packet(
            sequence,
            client_ack,
            challenge,
            client_command,
            body,
        )?]);
    }
    if sequence == 0
        || sequence > SEQUENCE_MASK
        || client_ack > i32::MAX as u32
        || body.len() > 0x20000
    {
        return Err(LobbyError("invalid compressed server message bounds"));
    }
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(body)
        .map_err(|_| LobbyError("server message compression failed"))?;
    let compressed = encoder
        .finish()
        .map_err(|_| LobbyError("server message compression failed"))?;
    let mut packet = sequence.to_le_bytes().to_vec();
    packet.extend_from_slice(&(client_ack | 0x8000_0000).to_le_bytes());
    packet.extend_from_slice(&compressed);
    xor_stream(
        &mut packet[8..],
        sequence as u8 ^ challenge as u8,
        client_command,
    );
    if packet.len() - 4 < FRAGMENT_SIZE {
        return Ok(vec![packet]);
    }
    let payload = &packet[4..];
    let count = payload.len().div_ceil(FRAGMENT_SIZE);
    if count > 128 {
        return Err(LobbyError("native fragment bitmap budget"));
    }
    let mut packets = Vec::new();
    for (index, chunk) in payload.chunks(FRAGMENT_SIZE).enumerate() {
        let mut fragment = (sequence | FRAGMENT_BIT).to_le_bytes().to_vec();
        fragment.extend_from_slice(&[index as u8, (count - 1) as u8]);
        fragment.extend_from_slice(&(FRAGMENT_SIZE as u16).to_le_bytes());
        fragment.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        fragment.extend_from_slice(chunk);
        packets.push(fragment);
    }
    Ok(packets)
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
    pub fn u8(&mut self) -> Result<u8, LobbyError> {
        let value = *self
            .bytes
            .get(self.at)
            .ok_or(LobbyError("native byte truncated"))?;
        self.at += 1;
        Ok(value)
    }
    pub fn u16(&mut self) -> Result<u16, LobbyError> {
        Ok(u16::from_le_bytes([self.u8()?, self.u8()?]))
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
    pub movement: Option<crate::game::channel::usercmd::Movement>,
}
pub fn client_commands(body: &[u8]) -> Result<ClientCommands, LobbyError> {
    client_message(body, None)
}
pub fn client_message(
    body: &[u8],
    movement_key: Option<u32>,
) -> Result<ClientCommands, LobbyError> {
    client_message_with_base(
        body,
        movement_key,
        &crate::game::channel::usercmd::UserCommand::default(),
    )
}
pub fn client_message_with_base(
    body: &[u8],
    movement_key: Option<u32>,
    base: &crate::game::channel::usercmd::UserCommand,
) -> Result<ClientCommands, LobbyError> {
    read_client_message(body, movement_key, base, false)
}
/// Resource initialization has no playerState baseline. Read reliable controls
/// and ignore the remaining bounded movement payload. Gameplay stays strict.
pub fn client_controls_without_player_state(body: &[u8]) -> Result<ClientCommands, LobbyError> {
    if body.len() > 4096 {
        return Err(LobbyError("resource control body size"));
    }
    read_client_message(
        body,
        None,
        &crate::game::channel::usercmd::UserCommand::default(),
        true,
    )
}
fn read_client_message(
    body: &[u8],
    movement_key: Option<u32>,
    base: &crate::game::channel::usercmd::UserCommand,
    ignore_movement: bool,
) -> Result<ClientCommands, LobbyError> {
    let mut reader = Reader::new(body);
    let mut commands = Vec::new();
    let mut movement = None;
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
                return Ok(ClientCommands { commands, movement });
            }
            op @ (0 | 1) => {
                if ignore_movement {
                    return Ok(ClientCommands {
                        commands,
                        movement: None,
                    });
                }
                let key = movement_key.ok_or(LobbyError("movement before bootstrap snapshot"))?;
                movement = Some(crate::game::channel::usercmd::read_movement_from(
                    &mut reader,
                    key,
                    op == 0,
                    base,
                )?);
                if reader.bits(3)? != 3 {
                    return Err(LobbyError("movement must end with EOF"));
                }
                reader.finish()?;
                return Ok(ClientCommands { commands, movement });
            }
            _ => {
                return Err(LobbyError("unsupported client native opcode"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn target_indexed_fragment_headers_reassemble_out_of_order_without_repeating_duplicate_payloads()
     {
        use std::io::Read;
        let mut n = 0x198733u32;
        let body = (0..9000)
            .map(|_| {
                n ^= n << 13;
                n ^= n >> 17;
                n ^= n << 5;
                n as u8
            })
            .collect::<Vec<_>>();
        let fragments = compact_server_packets(31, 7, 0x75, b"s", &body).unwrap();
        assert!(fragments.len() > 2);
        let mut storage = vec![0u8; fragments.len() * FRAGMENT_SIZE];
        let mut seen = std::collections::BTreeSet::new();
        let mut total = 0;
        for fragment in fragments.iter().rev().chain(std::iter::once(&fragments[0])) {
            assert_eq!(
                u32::from_le_bytes(fragment[..4].try_into().unwrap()),
                31 | FRAGMENT_BIT
            );
            let index = usize::from(fragment[4]);
            assert_eq!(usize::from(fragment[5]), fragments.len() - 1);
            let stride = usize::from(u16::from_le_bytes(fragment[6..8].try_into().unwrap()));
            assert_eq!(stride, FRAGMENT_SIZE);
            let len = usize::from(u16::from_le_bytes(fragment[8..10].try_into().unwrap()));
            assert_eq!(len, fragment.len() - 10);
            if seen.insert(index) {
                storage[index * stride..index * stride + len].copy_from_slice(&fragment[10..]);
                total += len;
            }
        }
        assert_eq!(seen.len(), fragments.len());
        storage.truncate(total);
        assert_eq!(
            u32::from_le_bytes(storage[..4].try_into().unwrap()),
            0x80000007
        );
        let mut encoded = storage[4..].to_vec();
        xor_stream(&mut encoded, 31 ^ 0x75, b"s");
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(&encoded[..])
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, body);
        assert_eq!(
            compact_server_packets(2, 0, 0, b"", &[6]).unwrap(),
            [server_packet(2, 0, 0, b"", &[6]).unwrap()]
        );
    }
    use super::*;
    #[test]
    fn resource_controls_ignore_opaque_movement_without_relaxing_gameplay_parsing() {
        // Reliable sequence1/text"s", then an intentionally undecodable move.
        let body = [10, 1, 0, 0, 0, b's', 0, 0xff, 0xff];
        let controls = client_controls_without_player_state(&body).unwrap();
        assert_eq!(controls.commands, vec![(1, b"s".to_vec())]);
        assert!(controls.movement.is_none());
        assert!(client_commands(&body).is_err());
        assert!(client_controls_without_player_state(&[3, 0]).is_err());
        assert!(client_controls_without_player_state(&vec![0; 4097]).is_err());
    }
    #[test]
    fn large_reliable_updates_fit_complete_native_messages_and_ack_only_sent_records() {
        let mut commands = ReliableCommands::default();
        commands
            .push_binary(&[b'J'; BINARY_RELIABLE_BYTES])
            .unwrap();
        commands
            .push_binary(&[b'J'; BINARY_RELIABLE_BYTES])
            .unwrap();
        let (none, sent) = commands.encode_frames_bounded(100).unwrap();
        assert!(none.is_empty());
        assert_eq!(sent, 0);
        let (mut first, sent) = commands.encode_frames_bounded(FRAGMENT_SIZE - 6).unwrap();
        assert_eq!(sent, 1);
        first.push(SERVER_EOF);
        server_packet(1, 0, 0, b"", &first).unwrap();
        commands.mark_sent(sent).unwrap();
        assert!(commands.acknowledge(2).is_err());
        commands.acknowledge(1).unwrap();
        let (mut second, sent) = commands.encode_frames_bounded(FRAGMENT_SIZE - 6).unwrap();
        assert_eq!(sent, 2);
        second.push(SERVER_EOF);
        server_packet(2, 0, 0, b"", &second).unwrap();
        commands.mark_sent(sent).unwrap();
        commands.acknowledge(2).unwrap();
        assert!(
            commands
                .encode_frames_bounded(FRAGMENT_SIZE)
                .unwrap()
                .0
                .is_empty()
        );
    }
    #[test]
    fn compressed_gamestate_keeps_clear_ack_and_inflates_after_xor() {
        use std::io::Read;
        let mut body = b"fpa_infbasee_tf_m_00\0boo_infbasea_tf_m_00\0".repeat(80);
        body.push(SERVER_EOF);
        let packet = compact_server_packet(3, 7, 0x75, b"ready\0ignored", &body).unwrap();
        assert_eq!(&packet[..4], &3u32.to_le_bytes());
        assert_eq!(&packet[4..8], &0x80000007u32.to_le_bytes());
        assert!(packet.len() - 4 < FRAGMENT_SIZE);
        let mut encoded = packet[8..].to_vec();
        xor_stream(&mut encoded, 0x76, b"ready");
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(&encoded[..])
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, body);
        assert_eq!(
            compact_server_packet(3, 0, 0x75, b"", &[6]).unwrap(),
            server_packet(3, 0, 0x75, b"", &[6]).unwrap()
        );
        assert!(compact_server_packet(3, 0, 0, b"", &vec![0; 0x20001]).is_err());
        let mut random = Vec::new();
        let mut seed = 12345u32;
        for _ in 0..4000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            random.push(seed as u8);
        }
        assert!(compact_server_packet(3, 0, 0, b"", &random).is_err());
        assert!(compact_server_packet(3, 0x80000000, 0, b"", &body).is_err());
    }
    #[test]
    fn crowded_live_snapshot_survives_compression_with_all_corpses_and_obituaries() {
        use crate::game::snapshot::WorldSnapshot;
        use crate::game::snapshot::corpse::CorpsePhase;
        use crate::game::snapshot::corpse::CorpseSnapshot;
        use crate::game::snapshot::enemy::EnemySnapshot;
        use crate::game::snapshot::obituary::ObituarySnapshot;
        use crate::game::snapshot::player_frame_with_world;
        use std::io::Read;
        let actors: Vec<_> = (0..12)
            .map(|i| EnemySnapshot {
                entity_number: i,
                client_index: i as u8,
                team: if i < 6 { 2 } else { 1 },
                model: 4,
                attachment_models: [3, 2, 5, 6, 0, 0, 0, 0, 0, 0],
                attachment_tags: [0; 10],
                origin: [f32::from(i) * 43.0, 40.0, 0.0],
                angles: [0, 1000 * i, 0],
                legs_animation: 395,
                torso_animation: 0,
                weapon: 352 + i,
                events: Default::default(),
                alive: true,
            })
            .collect();
        let corpses: Vec<_> = (0..8)
            .map(|i| CorpseSnapshot {
                entity_number: 19 + i,
                donor_client: i as u8,
                origin: [f32::from(i) * 40.0, -70.0, 0.0],
                angles: [0; 3],
                legs_animation: 47,
                torso_animation: 47,
                weapon: 352 + i,
                present: true,
                phase: CorpsePhase::Playing,
                death_sequence: 1,
            })
            .collect();
        let obituaries: Vec<_> = (0..32)
            .map(|i| ObituarySnapshot {
                falling: false,
                entity_number: 1800 + i,
                weapon: 352,
                headshot: false,
                melee: i % 2 == 0,
                victim: (i % 12) as u8,
                attacker: ((i + 1) % 12) as u8,
            })
            .collect();
        let body = player_frame_with_world(
            12000,
            12000,
            Some([0.0; 3]),
            false,
            None,
            [0; 3],
            WorldSnapshot {
                local_player: Some(actors[0]),
                players: &actors[1..],
                corpses: &corpses,
                obituaries: &obituaries,
                held_weapon_tag: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            body.len() + 4 >= FRAGMENT_SIZE,
            "snapshot {} did not exercise compression",
            body.len()
        );
        assert!(server_packet(16, 7, 0x75, b"s", &body).is_err());
        let packet = compact_server_packet(16, 7, 0x75, b"s", &body).unwrap();
        assert_eq!(
            u32::from_le_bytes(packet[4..8].try_into().unwrap()),
            0x80000007
        );
        let mut encoded = packet[8..].to_vec();
        xor_stream(&mut encoded, 16 ^ 0x75, b"s");
        let mut decoded = Vec::new();
        flate2::read::ZlibDecoder::new(&encoded[..])
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, body);
    }
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

    #[test]
    fn text_and_binary_share_sequence_ack_and_retransmit_complete_native_records() {
        let mut ring = ReliableCommands::default();
        assert_eq!(ring.push(b"E 0 100").unwrap(), 1);
        let payload = [b'J', 63, 0, 0, 0, 4, 100, 0, 0, 0];
        assert_eq!(ring.push_binary(&payload).unwrap(), 2);
        assert_eq!(ring.push(b"s fx_marks 1").unwrap(), 3);
        assert_eq!(ring.command_for(1).unwrap(), b"E 0 100");
        assert_eq!(
            ring.command_for(2).unwrap(),
            [b'!', 0, 10, 0, b'J', 63, 0, 0, 0, 4, 100, 0, 0, 0]
        );
        let mut expected = vec![1, 1, 0, 0, 0];
        expected.extend_from_slice(b"E 0 100\0");
        expected.extend_from_slice(&[2, 2, 0, 0, 0, 10, 0]);
        expected.extend_from_slice(&payload);
        expected.extend_from_slice(&[1, 3, 0, 0, 0]);
        expected.extend_from_slice(b"s fx_marks 1\0");
        assert_eq!(ring.encode_frames().unwrap(), expected);
        assert_eq!(ring.encode_frames().unwrap(), expected);
        assert!(ring.acknowledge(2).is_err());
        ring.mark_sent(3).unwrap();
        ring.acknowledge(1).unwrap();
        let expected_pending = [
            ring.encode_command(2).unwrap(),
            ring.encode_command(3).unwrap(),
        ]
        .concat();
        assert_eq!(ring.encode_frames().unwrap(), expected_pending);
        ring.acknowledge(2).unwrap();
        assert_eq!(
            ring.encode_frames().unwrap(),
            ring.encode_command(3).unwrap()
        );
        assert_eq!(ring.command_for(2).unwrap()[4..], payload);
        ring.acknowledge(3).unwrap();
        assert!(ring.encode_frames().unwrap().is_empty());
        assert_eq!(ring.encode_command(2).unwrap()[7..], payload);
    }

    #[test]
    fn binary_native_ring_nul_limits_both_packet_xor_and_usercmd_hash_to_the_marker() {
        let mut ring = ReliableCommands::default();
        ring.push_binary(&[b'J', 0x92, 0x25, 0, 0xff]).unwrap();
        let native = ring.command_for(1).unwrap();
        let mut bytes = [0u8; 6];
        // Independently calculated with initial key 0x33 and single byte !:
        // alternate XOR 0x21/0x42; no length or payload byte participates.
        xor_stream(&mut bytes, 0x33, native);
        assert_eq!(bytes, [0x12, 0x50, 0x71, 0x33, 0x12, 0x50]);
        assert_eq!(
            crate::game::channel::usercmd::key(17, 0x1234, native),
            crate::game::channel::usercmd::key(17, 0x1234, b"!")
        );
        assert_ne!(
            crate::game::channel::usercmd::key(17, 0x1234, native),
            crate::game::channel::usercmd::key(17, 0x1234, &native[4..])
        );
        let packet = [1, 0, 0, 0, 9, 2, 0, 0, 0, 1, 0, 0, 0, 0x1d];
        // Native C2S start key=0x34^9^2=0x3f, first mask=0x3f^!=0x1e.
        assert_eq!(open_client(&packet, 0x34, native).unwrap().1, [3]);
    }

    #[test]
    fn binary_bounds_and_window_fail_without_partial_state_or_frame_writes() {
        let mut ring = ReliableCommands::default();
        ring.push_binary(&vec![0; 1020]).unwrap();
        assert_eq!(ring.command_for(1).unwrap().len(), 1024);
        assert_eq!(&ring.command_for(1).unwrap()[..4], [b'!', 0, 0xfc, 3]);
        assert_eq!(ring.encode_command(1).unwrap().len(), 1027);
        let before = ring.encode_frames().unwrap();
        assert!(ring.push_binary(&vec![0; 1021]).is_err());
        assert_eq!(ring.sequence(), 1);
        assert_eq!(ring.encode_frames().unwrap(), before);
        assert!(ring.encode_command(0).is_err());
        for _ in 1..255 {
            ring.push_binary(&[]).unwrap();
        }
        assert!(ring.push_binary(b"J").is_err());
        assert_eq!(ring.sequence(), 255);
        let mut missing = ring.clone();
        missing.entries.remove(&128);
        let mut body = vec![0xa6];
        assert!(missing.append_frames(&mut body).is_err());
        assert_eq!(body, [0xa6]);
        ring.mark_sent(255).unwrap();
        ring.acknowledge(255).unwrap();
        assert_eq!(ring.push_binary(&[b'J', 0]).unwrap(), 256);
        assert!(ring.command_for(0).is_err());
        assert_eq!(ring.command_for(1).unwrap().len(), 1024);
        ring.push(b"new").unwrap();
        assert!(ring.command_for(1).is_err());
    }
}
