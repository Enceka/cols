//! 3.3.3.3 matchmaking descriptor observed after a successful room-name check.
//! Keep unknown attributes opaque; identity/channel references are verified.
use crate::{
    lobby::LobbyError,
    task::{Reader, Writer},
};
use std::net::SocketAddrV4;

#[derive(Clone)]
pub struct Description {
    pub room_name: String,
    pub owner_name: String,
    pub channel_id: u64,
    pub maximum: u32,
    pub build: i32,
    map_crc: u32,
    mode_crc: u32,
    playlist: i32,
    pub security_key: [u8; 16],
    /// Contains the client's session security material; never log or publish it.
    pub encoded: Vec<u8>,
}
impl Description {
    pub fn parse(arguments: &[u8], owner: u64) -> Result<Self, LobbyError> {
        if arguments.len() > 1024 {
            return Err(LobbyError("room descriptor exceeds budget"));
        }
        let mut r = Reader::new(arguments);
        if r.blob()?.len() != 37 {
            return Err(LobbyError("unexpected room address size"));
        }
        let _flags = r.u32()?;
        let maximum = r.u32()?;
        let room_name = r.string(65)?.to_owned();
        if r.u64()? != owner {
            return Err(LobbyError("room descriptor owner mismatch"));
        }
        let owner_name = r.string(65)?.to_owned();
        if !(1..=32).contains(&maximum)
            || room_name.is_empty()
            || owner_name.is_empty()
            || room_name
                .chars()
                .chain(owner_name.chars())
                .any(char::is_control)
        {
            return Err(LobbyError("invalid room capacity/name"));
        }
        let build = r.i32()?;
        for _ in 0..2 {
            r.i32()?;
        }
        r.u32()?;
        for _ in 0..3 {
            r.i32()?;
        }
        let security_key = r
            .blob()?
            .try_into()
            .map_err(|_| LobbyError("unexpected room security key size"))?;
        let mut attributes = [0i32; 11];
        for attribute in &mut attributes {
            *attribute = r.i32()?;
        }
        let channel_id = r.u64()?;
        r.i32()?;
        r.i32()?;
        r.finish()?;
        Ok(Self {
            room_name,
            owner_name,
            channel_id,
            maximum,
            build,
            map_crc: attributes[4] as u32,
            mode_crc: attributes[10] as u32,
            playlist: attributes[5],
            security_key,
            encoded: arguments.to_vec(),
        })
    }

    /// C1 custom-room captures bind tail[4]/[10] to CRC32(map/mode).
    pub fn matches_custom_map(&self, map: &str, mode: &str) -> bool {
        self.playlist == -1
            && self.map_crc == crc32fast::hash(map.as_bytes())
            && self.mode_crc == crc32fast::hash(mode.as_bytes())
    }

    /// Lookup result (0xa55700→0x701db0) adds a session Blob and player count
    /// to the create form. The receiver is an actual, separately bound UDP peer.
    pub fn allocation_record(
        &self,
        endpoint: SocketAddrV4,
        id: &[u8; 8],
        security_key: &[u8; 16],
    ) -> Result<Vec<u8>, LobbyError> {
        let mut source = Reader::new(&self.encoded);
        source.blob()?;
        let game_type = source.u32()?;
        let maximum = source.u32()?;
        source.string(65)?;
        source.u64()?;
        source.string(65)?;
        let mut out = Writer::default();
        out.blob(&common_address(endpoint))?;
        out.blob(id)?;
        out.u32(game_type);
        out.u32(maximum);
        out.u32(0);
        out.string("Local Game Endpoint");
        out.u64(1_000_001);
        out.string("Local Game Endpoint");
        for _ in 0..3 {
            out.i32(source.i32()?);
        }
        out.u32(source.u32()?);
        for _ in 0..3 {
            out.i32(source.i32()?);
        }
        source.blob()?;
        out.blob(security_key)?;
        let tail = source.remaining();
        out.0.extend_from_slice(&tail[..tail.len() - 1]);
        Ok(out.0)
    }
}

pub fn common_address(endpoint: SocketAddrV4) -> [u8; 37] {
    let mut out = [0u8; 37];
    for slot in 0..6 {
        out[slot * 6..slot * 6 + 6].copy_from_slice(&[0, 255, 0, 255, 0, 0]);
    }
    for slot in [0, 5] {
        out[slot * 6..slot * 6 + 4].copy_from_slice(&endpoint.ip().octets());
        out[slot * 6 + 4..slot * 6 + 6].copy_from_slice(&endpoint.port().to_le_bytes());
    }
    out[36] = 1;
    out
}

pub struct Search {
    pub query: u32,
    pub offset: u32,
    pub maximum: u32,
    pub criteria: Vec<i32>,
}
impl Search {
    pub fn parse(arguments: &[u8]) -> Result<Self, LobbyError> {
        let mut r = Reader::new(arguments);
        let query = r.u32()?;
        let offset = r.u32()?;
        let maximum = r.u32()?;
        if !matches!(query, 1 | 3) || offset != 0 || maximum == 0 || maximum > 50 {
            return Err(LobbyError("unsupported local session search"));
        }
        let count = if query == 1 { 1 } else { 8 };
        let mut criteria = Vec::new();
        for _ in 0..count {
            criteria.push(r.i32()?);
        }
        r.finish()?;
        Ok(Self {
            query,
            offset,
            maximum,
            criteria,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_has_address_security_id_and_three_base_counts() {
        let d = Description::parse(
            include_bytes!("../../../fixtures/protocol/room-create-synthetic.bin"),
            0x8000000000000001,
        )
        .unwrap();
        let data = d
            .allocation_record("127.0.0.1:39004".parse().unwrap(), &[7; 8], &[0x91; 16])
            .unwrap();
        let mut r = Reader::new(&data);
        let address = r.blob().unwrap();
        assert_eq!(address.len(), 37);
        assert_eq!(&address[..6], &[127, 0, 0, 1, 92, 152]);
        assert_eq!(&address[6..12], &[0, 255, 0, 255, 0, 0]);
        assert_eq!(r.blob().unwrap(), [7; 8]);
        assert_eq!(r.u32().unwrap(), 0);
        assert_eq!(r.u32().unwrap(), 12);
        assert_eq!(r.u32().unwrap(), 0);
        assert_eq!(r.string(64).unwrap(), "Local Game Endpoint");
        assert_eq!(r.u64().unwrap(), 1_000_001);
        assert_eq!(r.string(64).unwrap(), "Local Game Endpoint");
        assert_eq!(r.i32().unwrap(), -688906341);
        for _ in 0..2 {
            r.i32().unwrap();
        }
        r.u32().unwrap();
        for _ in 0..3 {
            r.i32().unwrap();
        }
        assert_eq!(r.blob().unwrap(), [0x91; 16]);
        assert_eq!(r.i32().unwrap(), 16);
    }
    #[test]
    fn session_search_rejects_extra_criteria_and_unsupported_pagination() {
        let mut w = Writer::default();
        w.u32(1);
        w.u32(0);
        w.u32(10);
        w.i32(-688906341);
        w.0.push(0);
        let search = Search::parse(&w.0).unwrap();
        assert_eq!(search.criteria, [-688906341]);
        for n in 0..w.0.len() {
            assert!(Search::parse(&w.0[..n]).is_err());
        }
        w.0[6] = 1;
        assert!(Search::parse(&w.0).is_err());
        w.0[6] = 0;
        w.0.push(0);
        assert!(Search::parse(&w.0).is_err());
    }
    #[test]
    fn descriptor_matches_independent_target_shape_and_binds_owner() {
        let data = include_bytes!("../../../fixtures/protocol/room-create-synthetic.bin");
        let d = Description::parse(data, 0x8000000000000001).unwrap();
        assert_eq!(d.channel_id, 42);
        assert_eq!(d.maximum, 12);
        assert_eq!(d.room_name, "Lab room");
        assert_eq!(d.owner_name, "Lab player");
        assert!(Description::parse(data, 2).is_err());
        for n in 0..data.len() {
            assert!(Description::parse(&data[..n], 0x8000000000000001).is_err());
        }
        let mut extra = data.to_vec();
        extra.push(0);
        assert!(Description::parse(&extra, 0x8000000000000001).is_err());
    }
}
