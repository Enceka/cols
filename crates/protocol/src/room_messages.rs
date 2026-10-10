//! Current C1 room bit streams, separate from tagged Demonware arguments.
//! RoomInfo 0x6dc3e0/0x6dc840; JoinRequest 0x6de470/0x6de590;
//! member fields 0x6dbd20/0x6dbfc0. No old-build struct offsets are used.
use crate::lobby::LobbyError;

pub const BOT_CLIENTS: u8 = 11;
pub fn bot_id(client: u8) -> u64 {
    0x7fff_434f_444f_0000 + u64::from(client)
}
pub fn bot_name(client: u8) -> String {
    format!("Bot {client}")
}

#[derive(Debug, PartialEq, Eq)]
pub struct RoomInfo {
    pub mode: String,
    pub map: String,
    pub playlist: u8,
    pub minimum: u8,
    pub maximum: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
    pub id: u64,
    /// Room enum: 1 = allies, 2 = axis; game snapshots use the reverse order.
    pub team: u8,
    pub ready: bool,
}
#[derive(Default, Debug)]
pub struct Updates {
    pub info: Option<RoomInfo>,
    pub members: Vec<Member>,
    pub full_roster: bool,
    pub join_result: Option<u8>,
    pub ready: Option<bool>,
}
struct Bits<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Bits<'a> {
    fn read(&mut self, n: usize) -> Result<u64, LobbyError> {
        if n > 64 || self.position + n > self.bytes.len() * 8 {
            return Err(LobbyError("truncated room bits"));
        }
        let mut out = 0;
        for i in 0..n {
            out |=
                u64::from((self.bytes[(self.position + i) / 8] >> ((self.position + i) % 8)) & 1)
                    << i;
        }
        self.position += n;
        Ok(out)
    }
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, LobbyError> {
        if n > 1024 {
            return Err(LobbyError("room submessage size"));
        }
        (0..n).map(|_| self.read(8).map(|b| b as u8)).collect()
    }
    fn string(&mut self) -> Result<String, LobbyError> {
        let mut bytes = Vec::new();
        for _ in 0..65 {
            let b = self.read(8)? as u8;
            if b == 0 {
                return String::from_utf8(bytes)
                    .map_err(|_| LobbyError("room identifier encoding"));
            }
            if !(32..=126).contains(&b) {
                return Err(LobbyError("invalid room identifier"));
            }
            bytes.push(b);
        }
        Err(LobbyError("room identifier too long"))
    }
    fn member(&mut self) -> Result<Member, LobbyError> {
        let result = Member {
            id: self.read(64)?,
            team: self.read(8)? as u8,
            ready: self.read(1)? != 0,
        };
        // 0x6dbfc0: the rest of the native member metadata, not nicknames.
        for bits in [16, 16, 8, 8, 64, 32, 1, 8, 8, 64, 8, 32, 32] {
            self.read(bits)?;
        }
        Ok(result)
    }
}
#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    position: usize,
}
impl Output {
    fn bits(&mut self, value: u64, count: usize) {
        for i in 0..count {
            if self.position / 8 == self.bytes.len() {
                self.bytes.push(0);
            }
            self.bytes[self.position / 8] |= (((value >> i) & 1) as u8) << (self.position % 8);
            self.position += 1;
        }
    }
    fn string(&mut self, value: &str) {
        for b in value.bytes().chain(std::iter::once(0)) {
            self.bits(u64::from(b), 8);
        }
    }
    fn frame(mut self, kind: u8) -> Self {
        self.bits(0, 1);
        self.bits(u64::from(kind), 8);
        self
    }
    fn batch(self) -> Vec<u8> {
        let mut result = (self.bytes.len() as u32).to_le_bytes().to_vec();
        result.extend(self.bytes);
        result
    }
}

/// Inspect authenticated room records addressed to this recipient or broadcast.
/// Unknown records remain opaque; all nesting and byte lengths stay bounded.
pub fn updates(batch: &[u8], recipient: u64) -> Result<Updates, LobbyError> {
    updates_for(batch, &[recipient])
}
/// The local host multiplexes bot-targeted replies inside ordinary broadcasts.
/// Only the authenticated owner and this room's simulated peers are consumers.
pub fn bot_room_updates(batch: &[u8], owner: u64) -> Result<Updates, LobbyError> {
    let mut recipients = vec![owner];
    recipients.extend((1..=BOT_CLIENTS).map(bot_id));
    updates_for(batch, &recipients)
}
fn updates_for(batch: &[u8], recipients: &[u64]) -> Result<Updates, LobbyError> {
    fn message(
        bytes: &[u8],
        recipients: &[u64],
        depth: usize,
        result: &mut Updates,
    ) -> Result<(), LobbyError> {
        if depth > 4 || bytes.is_empty() || bytes.len() > 1024 {
            return Err(LobbyError("room nesting limit"));
        }
        let mut r = Bits { bytes, position: 0 };
        if r.read(1)? != 0 {
            return Err(LobbyError("unsupported typed room stream"));
        }
        match r.read(8)? {
            1 => {
                let count = r.read(32)?;
                if count > 64 {
                    return Err(LobbyError("room manifold count"));
                }
                for _ in 0..count {
                    let targeted = r.read(1)? != 0;
                    let recipient = if targeted { Some(r.read(64)?) } else { None };
                    let size = r.read(32)? as usize;
                    let body = r.bytes(size)?;
                    if recipient.is_none_or(|r| recipients.contains(&r)) {
                        message(&body, recipients, depth + 1, result)?;
                    }
                }
            }
            5 => result.join_result = Some(r.read(8)? as u8),
            10 => result.members.push(r.member()?),
            14 => {
                let count = r.read(32)?;
                if count > 18 {
                    return Err(LobbyError("room member count"));
                }
                result.members.clear();
                for _ in 0..count {
                    result.members.push(r.member()?);
                }
                result.full_roster = true;
            }
            15 => {
                let info = RoomInfo {
                    mode: r.string()?,
                    map: r.string()?,
                    playlist: r.read(8)? as u8,
                    minimum: r.read(8)? as u8,
                    maximum: r.read(8)? as u8,
                };
                // Native room batches can contain several arrow changes.
                // Their final owner record is the resulting room choice.
                result.info = Some(info);
            }
            16 => result.ready = Some(r.read(1)? != 0),
            _ => {}
        }
        Ok(())
    }
    if batch.len() > 16384 {
        return Err(LobbyError("room batch size"));
    }
    let mut left = batch;
    let mut result = Updates::default();
    while !left.is_empty() {
        let header = left.get(..4).ok_or(LobbyError("room length missing"))?;
        let size = u32::from_le_bytes(header.try_into().unwrap()) as usize;
        let record = left
            .get(4..4 + size)
            .ok_or(LobbyError("room record truncated"))?;
        message(record, recipients, 0, &mut result)?;
        left = &left[4 + size..];
    }
    Ok(result)
}
pub fn room_info(batch: &[u8], owner: u64) -> Result<Option<RoomInfo>, LobbyError> {
    Ok(updates(batch, owner)?.info)
}
pub fn roster_request() -> Vec<u8> {
    Output::default().frame(13).batch()
}

/// Local simulated players join the host through the native request, keeping
/// the host's own map options authoritative. No JoinResponse is forged.
pub fn bot_join(client: u8, publisher_version: u32) -> Result<Vec<u8>, LobbyError> {
    if !(1..=BOT_CLIENTS).contains(&client) || publisher_version == 0 {
        return Err(LobbyError("invalid room bot"));
    }
    let mut w = Output::default().frame(6);
    w.string(""); // Password: opt-in local bot rooms are public.
    w.bits(1, 32); // Native JoinRequest constructor's request kind.
    w.bits(1000, 16); // Native default skill.
    w.bits(u64::from(publisher_version), 32);
    w.bits(0, 32);
    w.bits(0, 64); // JoinRequest leaves identity for the host to assign from sender.
    w.bits(0, 8); // Native room team zero asks the host to assign/balance.
    w.bits(1, 1); // Ready.
    w.bits(1000, 16);
    w.bits(0, 16); // Skill and rank.
    w.bits(0, 8);
    w.bits(0, 8); // Prestige attributes.
    w.bits(0, 64);
    w.bits(0, 32); // Clan and inventory metadata.
    w.bits(0, 1);
    w.bits(255, 8); // No party/group; this is not the member's team field.
    w.bits(0, 8);
    for _ in 0..4 {
        w.bits(0, 16);
    }
    w.bits(0, 8);
    w.bits(0, 32);
    w.bits(0, 32);
    w.string(""); // Member +0x48 is the clan tag, not the display name.
    Ok(w.batch())
}
pub fn bot_ready() -> Vec<u8> {
    let mut w = Output::default().frame(16);
    w.bits(1, 1);
    w.batch()
}

/// Channel sends use a targeted native manifold, including one embedded record.
pub fn to_member(record: &[u8], _recipient: u64) -> Result<Vec<u8>, LobbyError> {
    if record.len() < 5
        || record.len() > 1024
        || u32::from_le_bytes(record[..4].try_into().unwrap()) as usize != record.len() - 4
    {
        return Err(LobbyError("invalid targeted room record"));
    }
    let mut w = Output::default().frame(1);
    w.bits(1, 32);
    // The native channel carries a broadcast manifold. The server already
    // scopes this push to the sole authenticated owner of this local room.
    w.bits(0, 1);
    w.bits((record.len() - 4) as u64, 32);
    for &b in &record[4..] {
        w.bits(u64::from(b), 8);
    }
    Ok(w.batch())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn join_result_is_one_byte_before_the_room_mode_string() {
        // C1 0x6de190 reads an 8-bit result, then RoomInfo begins with "war".
        let capture = [7, 0, 0, 0, 0x0a, 0, 0xee, 0xc2, 0xe4, 0, 0];
        assert_eq!(updates(&capture, 42).unwrap().join_result, Some(0));
    }
    #[test]
    fn host_broadcast_contains_the_roster_targeted_to_a_simulated_peer() {
        // C1 type14/0x6de780 capture: retain team/ready bits, replace the human
        // UID with 42 and clear all other member metadata before committing.
        let batch = include_bytes!("../../../fixtures/protocol/room-roster-synthetic.bin");
        assert!(updates(batch, 42).unwrap().members.is_empty());
        let result = bot_room_updates(batch, 42).unwrap();
        assert!(result.full_roster);
        assert_eq!(result.members.len(), 12);
        assert_eq!(result.members[0].id, 42);
        assert_eq!(
            result.members.iter().map(|m| m.team).collect::<Vec<_>>(),
            [2, 1, 1, 2, 2, 1, 2, 1, 2, 1, 2, 1]
        );
        for (i, m) in result.members.iter().enumerate().skip(1) {
            assert_eq!(m.id, bot_id(i as u8));
        }
        for n in 1..batch.len() {
            assert!(bot_room_updates(&batch[..n], 42).is_err());
        }
    }
    #[test]
    fn ready_matches_native_record_and_join_has_bounded_identity_and_version() {
        assert_eq!(bot_ready(), [2, 0, 0, 0, 0x20, 0x02]);
        for client in 1..=11 {
            let data = bot_join(client, 7).unwrap();
            let mut r = Bits {
                bytes: &data[4..],
                position: 0,
            };
            assert_eq!(r.read(1).unwrap(), 0);
            assert_eq!(r.read(8).unwrap(), 6);
            assert_eq!(r.string().unwrap(), "");
            assert_eq!(r.read(32).unwrap(), 1);
            assert_eq!(r.read(16).unwrap(), 1000);
            assert_eq!(r.read(32).unwrap(), 7);
            assert_eq!(r.read(32).unwrap(), 0);
            assert_eq!(r.read(64).unwrap(), 0);
            // Native member fields end at bit 499; the trailing string is
            // the clan tag. Nicknames arrive via the SDK channel sender.
            r.position = 499;
            assert_eq!(r.string().unwrap(), "");
            assert_eq!(data.len(), 4 + 64);
        }
        assert!(bot_join(0, 7).is_err());
        assert!(bot_join(12, 7).is_err());
        assert!(bot_join(1, 0).is_err());
    }
    #[test]
    fn only_room_info_for_this_owner_selects_a_map_and_truncation_fails() {
        let mut info = Output::default().frame(15);
        info.string("war");
        info.string("mp_killhouse_sh");
        info.bits(255, 8);
        info.bits(4, 8);
        info.bits(12, 8);
        let mut manifold = Output::default().frame(1);
        manifold.bits(1, 32);
        manifold.bits(1, 1);
        manifold.bits(9, 64);
        manifold.bits(info.bytes.len() as u64, 32);
        for b in info.bytes {
            manifold.bits(u64::from(b), 8);
        }
        let data = manifold.batch();
        assert_eq!(room_info(&data, 9).unwrap().unwrap().map, "mp_killhouse_sh");
        assert_eq!(room_info(&data, 10).unwrap(), None);
        for n in 1..data.len() {
            assert!(room_info(&data[..n], 9).is_err());
        }
    }
}
