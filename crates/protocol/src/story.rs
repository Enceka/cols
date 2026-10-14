//! Diagnostic story allocation in the measured 3.3.3.3 result layout.
//! Consumer0x701db0 reads the key at +0x1cc; this is not a gameplay descriptor.
use crate::{lobby::LobbyError, room::common_address, task::Writer};
use std::net::SocketAddrV4;

pub const CLIENT_BUILD: i32 = -688906341;
pub const FIRST_MAP: &str = "coop_training_mp_cargoship";

/// Server-issued security material is returned through the authenticated
/// lobby. Map choice is checked against the later native start, not query1.
pub fn allocation_record(
    endpoint: SocketAddrV4,
    channel: u64,
    build: i32,
    security_id: &[u8; 8],
    security_key: &[u8; 16],
) -> Result<Vec<u8>, LobbyError> {
    if !endpoint.ip().is_loopback()
        || endpoint.port() == 0
        || matches!(channel, 0 | u64::MAX)
        || build != CLIENT_BUILD
        || *security_id == [0; 8]
        || *security_key == [0; 16]
    {
        return Err(LobbyError("invalid diagnostic story allocation"));
    }
    let mut out = Writer::default();
    out.blob(&common_address(endpoint))?;
    out.blob(security_id)?;
    out.u32(0); // Diagnostic endpoint flags, not a claimed public room.
    out.u32(1); // Native story map player limit.
    out.u32(0);
    out.string("Local Story Capture");
    out.u64(1_000_001);
    out.string("Local Story Capture");
    out.i32(build);
    for _ in 0..2 {
        out.i32(0);
    }
    out.u32(0);
    for _ in 0..3 {
        out.i32(0);
    }
    out.blob(security_key)?;
    for index in 0..11 {
        // An explicit start has no publisher playlist. Unknown endpoint
        // attributes remain zero; no map is inferred from a build-only query.
        out.i32(if index == 5 { -1 } else { 0 });
    }
    out.u64(channel);
    out.i32(0);
    out.i32(0);
    Ok(out.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Reader;

    #[test]
    fn story_result_carries_server_key_after_native_fields_and_binds_the_channel() {
        let address = "127.0.0.1:39004".parse().unwrap();
        let wire = allocation_record(address, 42, CLIENT_BUILD, &[7; 8], &[9; 16]).unwrap();
        let mut r = Reader::new(&wire);
        assert_eq!(r.blob().unwrap(), common_address(address));
        assert_eq!(r.blob().unwrap(), [7; 8]);
        assert_eq!(
            [r.u32().unwrap(), r.u32().unwrap(), r.u32().unwrap()],
            [0, 1, 0]
        );
        assert!(r.string(24).unwrap().len() < 24);
        assert_eq!(r.u64().unwrap(), 1_000_001);
        r.string(64).unwrap();
        assert_eq!(r.i32().unwrap(), CLIENT_BUILD);
        for _ in 0..2 {
            assert_eq!(r.i32().unwrap(), 0);
        }
        assert_eq!(r.u32().unwrap(), 0);
        for _ in 0..3 {
            assert_eq!(r.i32().unwrap(), 0);
        }
        assert_eq!(r.blob().unwrap(), [9; 16]);
        for i in 0..11 {
            assert_eq!(r.i32().unwrap(), if i == 5 { -1 } else { 0 });
        }
        assert_eq!(r.u64().unwrap(), 42);
        assert_eq!(r.i32().unwrap(), 0);
        assert_eq!(r.i32().unwrap(), 0);
        assert!(r.remaining().is_empty());
    }

    #[test]
    fn story_result_rejects_invalid_identity_and_empty_security_material() {
        for (address, channel, build, id, key) in [
            ("192.0.2.1:39004", 42, CLIENT_BUILD, [7; 8], [9; 16]),
            ("127.0.0.1:0", 42, CLIENT_BUILD, [7; 8], [9; 16]),
            ("127.0.0.1:39004", 0, CLIENT_BUILD, [7; 8], [9; 16]),
            ("127.0.0.1:39004", 42, 0, [7; 8], [9; 16]),
            ("127.0.0.1:39004", 42, CLIENT_BUILD, [0; 8], [9; 16]),
            ("127.0.0.1:39004", 42, CLIENT_BUILD, [7; 8], [0; 16]),
        ] {
            assert!(
                allocation_record(address.parse().unwrap(), channel, build, &id, &key).is_err()
            );
        }
    }
}
