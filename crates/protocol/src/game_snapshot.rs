//! Bounded bootstrap snapshot probe for 3.3.3.3, NOT a world simulator.
//! Parser: 0x46d170; playerState: 0x5a4230; table: 0xc55f70 (235 fields).
use crate::{game_state::Writer, game_wire::SERVER_EOF, lobby::LobbyError};

/// A full baseline with command time and health, no entities/clients/weapons.
/// All 14 target array sections are present; this is not a playable spawn.
pub fn bootstrap(server_time: u32) -> Result<Vec<u8>, LobbyError> {
    bootstrap_at_origin(server_time, None)
}

pub fn bootstrap_at_origin(
    server_time: u32,
    origin: Option<[f32; 3]>,
) -> Result<Vec<u8>, LobbyError> {
    frame(server_time, origin, true)
}

/// After a baseline ACK, delta-distance zero with flags0 selects the cached
/// full baseline for section histories; playerState still selects no base.
pub fn frame(
    server_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
) -> Result<Vec<u8>, LobbyError> {
    if server_time == 0 || server_time > i32::MAX as u32 {
        return Err(LobbyError("snapshot time out of range"));
    }
    if origin.is_some_and(|v| v.iter().any(|n| !n.is_finite() || n.abs() > 1_000_000.0)) {
        return Err(LobbyError("bootstrap origin out of range"));
    }
    let mut out = Writer::default();
    out.bytes.push(0); // svc_snapshot, byte pool starts at zero
    out.bytes.extend_from_slice(&server_time.to_le_bytes());
    out.bytes
        .extend_from_slice(&[0, if establish_baseline { 8 } else { 0 }]);
    out.bits(0, 1); // no prior playerState
    out.bits(1, 1); // transmit origin/velocity rather than local prediction
    out.bits(if origin.is_some() { 69 } else { 1 }, 8);
    out.bits(1, 1); // field gap +1 from -1
    out.bits(1, 1); // -97 codec, absolute LE32 (0x5a20c0)
    out.bytes.extend_from_slice(&server_time.to_le_bytes());
    if let Some(origin) = origin {
        // Target field table 0xc55f70. No hint-2 entries occur below index69.
        // Keep viewangles/weapons/buttons zero so the input base stays valid.
        let mut previous = 0;
        for (index, value, width) in [
            (8, origin[0].to_bits(), 0),
            (10, origin[2].to_bits(), 0),
            (11, origin[1].to_bits(), 0),
            (34, 60f32.to_bits(), 0),
            (38, 60, 8),
            (47, 2047, 11),
            (61, 2047, 16),
            (68, 2047, 11),
        ] {
            gap(&mut out, index - previous);
            previous = index;
            if width == 0 {
                // target -88: full raw float XOR with zero baseline
                out.bytes.extend_from_slice(&value.to_le_bytes());
            } else {
                // 0x5a27f0 nonzero gate -> 0x5a2510 -> 0x5a2470:
                // remainder bits first, then whole bytes (not all pool bits).
                out.bits(1, 1);
                let remainder = width % 8;
                out.bits(value, remainder);
                for shift in (remainder..width).step_by(8) {
                    out.bytes.push((value >> shift) as u8);
                }
            }
        }
    }
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

fn gap(out: &mut Writer, distance: u32) {
    out.bits(u32::from(distance == 1), 1);
    if distance == 1 {
        return;
    }
    let mut rest = distance - 1;
    while rest >= 15 {
        out.bits(15, 4);
        rest -= 15;
    }
    out.bits(rest, 4);
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
    #[test]
    fn spawn_coordinates_reject_nan_and_infinity() {
        assert!(bootstrap_at_origin(1000, Some([f32::NAN, 0.0, 0.0])).is_err());
        assert!(bootstrap_at_origin(1000, Some([0.0, f32::INFINITY, 0.0])).is_err());
        assert!(bootstrap_at_origin(1000, Some([0.0, 0.0, 1_000_001.0])).is_err());
    }
    #[test]
    fn selected_spawn_fields_match_independently_extracted_target_table() {
        let layout: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/player-state-fields-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(layout["client_sha256"], crate::CLIENT_SHA256);
        let fields = layout["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 235);
        for (index, offset, codec, hint) in [
            (0, 0, -97, 0),
            (8, 0x1c, -88, 3),
            (10, 0x24, -88, 3),
            (11, 0x20, -88, 3),
            (34, 0x15c, -88, 0),
            (38, 0x158, -8, 0),
            (47, 0xac, 11, 0),
            (61, 0x1b4, 16, 0),
            (68, 0x1f0, 11, 0),
        ] {
            assert_eq!(fields[index], serde_json::json!([offset, codec, hint]));
        }
        assert!(fields[..69].iter().all(|f| f[2] != 2));
        let mut baseline = frame(1000, Some([-472.0, -800.0, -113.4]), true).unwrap();
        // Accepted by the real target in client-1791345984530; frozen body
        // after removing native XOR. Includes mixed pool bits/byte fields.
        let hex = "00e8030000000816ede8030000440000ecc3cdcce2c2000048c47f00007042263c7cff9dff07fdff00640000f87f0006";
        let accepted: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(baseline, accepted);
        assert_eq!(baseline[6], 8);
        baseline[6] = 0;
        assert_eq!(
            baseline,
            frame(1000, Some([-472.0, -800.0, -113.4]), false).unwrap()
        );
    }
}
