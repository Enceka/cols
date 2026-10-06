//! 3.3.3.3 matchmaking descriptor observed after a successful room-name check.
//! Keep unknown attributes opaque; identity/channel references are verified.
use crate::{lobby::LobbyError, task::Reader};

#[derive(Clone)]
pub struct Description {
    pub room_name: String,
    pub owner_name: String,
    pub channel_id: u64,
    pub maximum: u32,
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
        for _ in 0..3 {
            r.i32()?;
        }
        r.u32()?;
        for _ in 0..3 {
            r.i32()?;
        }
        if r.blob()?.len() != 16 {
            return Err(LobbyError("unexpected room security key size"));
        }
        for _ in 0..11 {
            r.i32()?;
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
            encoded: arguments.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
