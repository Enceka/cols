//! Bounded bootstrap snapshot probe for 3.3.3.3, NOT a world simulator.
//! Parser: 0x46d170; playerState: 0x5a4230; table: 0xc55f70 (235 fields).
use crate::{game_state::Writer, game_wire::SERVER_EOF, lobby::LobbyError};

/// A full baseline with command time and health, no entities/clients/weapons.
/// All 14 target array sections are present; this is not a playable spawn.
pub fn bootstrap(server_time: u32) -> Result<Vec<u8>, LobbyError> {
    if server_time == 0 || server_time > i32::MAX as u32 {
        return Err(LobbyError("snapshot time out of range"));
    }
    let mut out = Writer::default();
    out.bytes.push(0); // svc_snapshot, byte pool starts at zero
    out.bytes.extend_from_slice(&server_time.to_le_bytes());
    out.bytes.extend_from_slice(&[0, 8]); // no delta; establish full baseline
    out.bits(0, 1); // no prior playerState
    out.bits(1, 1); // transmit origin/velocity rather than local prediction
    out.bits(1, 8); // last changed field + 1: commandTime at index 0
    out.bits(1, 1); // field gap +1 from -1
    out.bits(1, 1); // -97 codec, absolute LE32 (0x5a20c0)
    out.bytes.extend_from_slice(&server_time.to_le_bytes());
    // 0x5a463d: stats presence, 4-bit mask, first stat is a short.
    out.bits(1, 1);
    out.bits(1, 4);
    out.bytes.extend_from_slice(&100u16.to_le_bytes());
    // Clip, equipped weapons, reserve; four extended bundles; objectives,
    // HUD; four final indexed lists. 3.3.3.3 has fourteen gates in total.
    out.bits(0, 13);
    out.bits(0, 1); // animation command section (0x5a3b50)
    out.bits(0, 1); // actors (0x46cc60)
    // Entities (0x46c2f0): explicit 11-bit sentinel, not a presence gate.
    out.bits(0, 1);
    out.bits(1, 1);
    out.bits(2047, 11);
    out.bits(0, 1); // clients (0x46c840)
    out.bits(0, 1); // glass (0x43e070)
    out.bytes.push(SERVER_EOF);
    Ok(out.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_full_frame_layout_has_fourteen_arrays_and_entity_sentinel() {
        // Independent bit accounting: header7; pool 0x06,0x3c; time4;
        // pool0 (one remaining stats-mask bit); health2; pools0,0xfe,0x1f.
        let packet = bootstrap(1000).unwrap();
        assert_eq!(
            packet,
            [
                0, 0xe8, 3, 0, 0, 0, 8, 6, 0x3c, 0xe8, 3, 0, 0, 0, 100, 0, 0, 0xfe, 0x1f, 6
            ]
        );
        assert!(bootstrap(0).is_err());
        assert!(bootstrap(u32::MAX).is_err());
    }
}
