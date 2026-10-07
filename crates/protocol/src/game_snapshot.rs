//! Bounded bootstrap snapshot probe for 3.3.3.3, NOT a world simulator.
//! Parser: 0x46d170; playerState: 0x5a4230; table: 0xc55f70 (235 fields).
use crate::{game_state::Writer, game_wire::SERVER_EOF, lobby::LobbyError};

#[derive(Debug, Clone, Copy)]
pub struct Ammo {
    pub clip_key: u16,
    pub reserve_key: u16,
    pub clip: u16,
    pub reserve: u16,
}
#[derive(Debug, Clone, Copy)]
pub struct Equipment {
    pub weapon: u16,
    pub ammo: Option<Ammo>,
    pub viewmodel: Option<u16>,
}

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
    equipped_frame(server_time, origin, establish_baseline, None)
}

pub fn equipped_frame(
    server_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
    equipment: Option<Equipment>,
) -> Result<Vec<u8>, LobbyError> {
    aimed_frame(server_time, origin, establish_baseline, equipment, [0; 3])
}

pub fn aimed_frame(
    server_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
    equipment: Option<Equipment>,
    angles: [u16; 3],
) -> Result<Vec<u8>, LobbyError> {
    let weapon = equipment.map(|e| e.weapon);
    let ammo = equipment.and_then(|e| e.ammo);
    let viewmodel = equipment.and_then(|e| e.viewmodel);
    if viewmodel.is_some_and(|i| i == 0 || i >= 1024) {
        return Err(LobbyError("invalid viewmodel index"));
    }
    if ammo.is_some_and(|a| {
        a.clip_key > 1022 || a.reserve_key > 1023 || a.clip > 511 || a.reserve > 2047
    }) {
        return Err(LobbyError("invalid bootstrap ammo"));
    }
    if weapon.is_some_and(|w| w == 0 || w >= 900) || (weapon.is_some() && origin.is_none()) {
        return Err(LobbyError("invalid bootstrap equipment"));
    }
    if weapon.is_some()
        && origin.is_some_and(|v| v.iter().any(|x| !(-32768.0..=32767.0).contains(x)))
    {
        return Err(LobbyError(
            "entity origin exceeds current zero map-center range",
        ));
    }
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
        // Equipment and angles must also be reflected in the input delta base.
        let mut previous = 0;
        let mut fields = vec![
            (8, origin[0].to_bits(), 0),
            (10, origin[2].to_bits(), 0),
            (11, origin[1].to_bits(), 0),
            (34, 60f32.to_bits(), 0),
            (38, 60, 8),
            (47, 2047, 11),
            (61, 2047, 16),
            (68, 2047, 11),
        ];
        if let Some(weapon) = weapon {
            // 0x422e5c/0x42956f select idle0 with the0x400 restart bit.
            fields.extend([
                (9, 0x400, 11),
                (39, u32::from(weapon), 10),
                (54, 0x1000, 28),
            ]);
        }
        if let Some(model) = viewmodel {
            fields.push((56, u32::from(model), 10));
        }
        // Marker87 is the target's -87 short-angle codec, with no nonzero bit.
        for (index, value) in [(19, angles[0]), (18, angles[1]), (29, angles[2])] {
            if value != 0 {
                fields.push((index, u32::from(value), 87));
            }
        }
        fields.sort_by_key(|f| f.0);
        for (index, value, width) in fields {
            gap(&mut out, index - previous);
            previous = index;
            if width == 0 {
                // target -88: full raw float XOR with zero baseline
                out.bytes.extend_from_slice(&value.to_le_bytes());
            } else if width == 87 {
                out.bytes.extend_from_slice(&(value as u16).to_le_bytes());
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
    out.bits(if weapon.is_some() { 5 } else { 1 }, 4);
    out.bytes.extend_from_slice(&100u16.to_le_bytes());
    if let Some(weapon) = weapon {
        out.bytes.extend_from_slice(&100u16.to_le_bytes()); // stats[2], max health
        if let Some(ammo) = ammo {
            out.bits(1, 1);
            out.bits(0, 4);
            out.bits(u32::from(ammo.clip_key), 10);
            out.bits(1, 1);
            out.bits(u32::from(ammo.clip), 9);
            out.bits(1, 1);
            out.bits(0, 9); // explicitly clear offhand magazine
        }
        out.bits(0, 1); // clip list terminator
        out.bits(1, 1); // equipped weapon section
        out.bits(1, 1); // slot0 changed
        out.bits(1, 1); // weapon ID present
        out.bits(u32::from(weapon), 10);
        out.bits(0, 5); // five per-slot flags
        out.bits(0, 14); // slots1..14 unchanged
        if let Some(ammo) = ammo {
            out.bits(1, 1);
            out.bits(0, 4);
            out.bits(u32::from(ammo.reserve_key), 10);
            out.bits(u32::from(ammo.reserve), 11);
        }
        out.bits(0, 1); // reserve terminator
        out.bits(0, 10); // four extension groups + objectives/HUD + four lists
    } else {
        // Clip, equipped weapons, reserve; four extended bundles; objectives,
        // HUD; four final indexed lists. 3.3.3.3 has fourteen gates in total.
        out.bits(0, 13);
    }
    out.bits(0, 1); // animation command section (0x5a3b50)
    out.bits(0, 1); // actors (0x46cc60)
    if establish_baseline && weapon.is_some() {
        // Target type1 table 0xc52b68. Entity0 has the local player type and
        // map-derived origin. Ordinary frames inherit it from the cached base.
        out.bits(1, 1); // entity index -1 -> 0
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(0, 1); // retained, changed, origin flag
        out.bits(5, 6); // last changed index (not field count)
        out.bits(1, 1);
        out.bytes.push(1); // first field eType, codec -77 raw byte
        let origin = origin.expect("validated equipped origin");
        for (distance, coordinate) in [(1, origin[1]), (1, origin[0]), (3, origin[2])] {
            out.bits(u32::from(distance == 1), 1);
            if distance != 1 {
                out.bits(distance - 1, 3);
            }
            out.bits(1, 1); // full 16-bit position codec; baseline/map center zero
            out.bits(u32::from(coordinate.round_ties_even() as i16 as u16), 16);
        }
    }
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
    fn equipped_snapshot_matches_target_field_array_and_entity_sections() {
        use crate::game_wire::Reader;
        let layout: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/player-state-fields-3.3.3.3.json"
        ))
        .unwrap();
        let equipment = Equipment {
            weapon: 163,
            viewmodel: Some(53),
            ammo: Some(Ammo {
                clip_key: 163,
                reserve_key: 163,
                clip: 30,
                reserve: 90,
            }),
        };
        for baseline in [true, false] {
            let body = aimed_frame(
                1000,
                Some([-472.0, -800.0, -113.4]),
                baseline,
                Some(equipment),
                [65535, 16384, 0],
            )
            .unwrap();
            let mut r = Reader::new(&body);
            assert_eq!(r.u8().unwrap(), 0);
            assert_eq!(r.u32().unwrap(), 1000);
            assert_eq!(r.u8().unwrap(), 0);
            assert_eq!(r.u8().unwrap(), if baseline { 8 } else { 0 });
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.bits(1).unwrap(), 1);
            let last = r.bits(8).unwrap() as i32 - 1;
            let mut index = -1i32;
            let mut fields = std::collections::BTreeMap::new();
            while index < last {
                let mut distance = 1;
                if r.bits(1).unwrap() == 0 {
                    loop {
                        let n = r.bits(4).unwrap();
                        distance += n;
                        if n < 15 {
                            break;
                        }
                    }
                }
                index += distance as i32;
                let codec = layout["fields"][index as usize][1].as_i64().unwrap();
                let value = match codec {
                    -97 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        r.u32().unwrap()
                    }
                    -88 => r.u32().unwrap(),
                    -87 => u32::from(r.u16().unwrap()),
                    _ => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        let width = codec.unsigned_abs() as usize;
                        let low = width % 8;
                        let mut value = r.bits(low).unwrap();
                        for bit in (low..width).step_by(8) {
                            value |= u32::from(r.u8().unwrap()) << bit;
                        }
                        value
                    }
                };
                fields.insert(index, value);
            }
            assert_eq!(fields[&39], 163);
            assert_eq!(fields[&56], 53);
            assert_eq!(fields[&9], 0x400);
            assert_eq!(fields[&19], 65535);
            assert_eq!(fields[&18], 16384);
            assert_eq!(fields[&54], 0x1000);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(4).unwrap(), 5);
            assert_eq!(r.u16().unwrap(), 100);
            assert_eq!(r.u16().unwrap(), 100);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(4).unwrap(), 0);
            assert_eq!(r.bits(10).unwrap(), 163);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(9).unwrap(), 30);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(9).unwrap(), 0);
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.bits(3).unwrap(), 7);
            assert_eq!(r.bits(10).unwrap(), 163);
            assert_eq!(r.bits(19).unwrap(), 0); // slot flags and fourteen absent slots
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(4).unwrap(), 0);
            assert_eq!(r.bits(10).unwrap(), 163);
            assert_eq!(r.bits(11).unwrap(), 90);
            assert_eq!(r.bits(13).unwrap(), 0); // reserve EOF, ten arrays, anim, actors
            if baseline {
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.bits(3).unwrap(), 2);
                assert_eq!(r.bits(6).unwrap(), 5);
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.u8().unwrap(), 1);
                for (delta, expected) in [(1, -800i16), (1, -472), (3, -113)] {
                    assert_eq!(r.bits(1).unwrap(), u32::from(delta == 1));
                    if delta != 1 {
                        assert_eq!(r.bits(3).unwrap(), delta - 1);
                    }
                    assert_eq!(r.bits(1).unwrap(), 1);
                    assert_eq!(r.bits(16).unwrap() as u16 as i16, expected);
                }
            }
            assert_eq!(r.bits(2).unwrap(), 2);
            assert_eq!(r.bits(11).unwrap(), 2047);
            assert_eq!(r.bits(2).unwrap(), 0);
            assert_eq!(r.u8().unwrap(), 6);
            r.finish().unwrap();
        }
        let bad = Equipment {
            weapon: 163,
            viewmodel: None,
            ammo: Some(Ammo {
                clip_key: 163,
                reserve_key: 163,
                clip: 512,
                reserve: 90,
            }),
        };
        assert!(equipped_frame(1000, Some([0.0; 3]), true, Some(bad)).is_err());
    }
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
