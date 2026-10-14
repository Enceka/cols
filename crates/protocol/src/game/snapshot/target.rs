//! Static training props in the measured 3.3.3.3 GENERAL entity layout.
//! Parser 0x5a3670; table 0xc525a8; model consumer 0x501550.
use crate::game::gamestate::Writer;
use crate::lobby::channel::LobbyError;

#[derive(Debug, Clone, Copy)]
pub struct TargetSnapshot {
    pub entity_number: u16,
    pub model: u16,
    pub origin: [f32; 3],
    pub scale: f32,
    pub alive: bool,
}

/// Write one target before the entity-list sentinel. Ordinary snapshots use
/// the pinned full baseline: live targets inherit it, while dead targets must
/// be explicitly removed on every frame. The target's model and position are
/// fixed for that baseline's lifetime; establishing a new one requires a live
/// target so later inheritance also represents respawn.
pub(crate) fn write_target(
    out: &mut Writer,
    target: TargetSnapshot,
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    if !(1..2047).contains(&target.entity_number)
        || !(1..1024).contains(&target.model)
        || target
            .origin
            .iter()
            .any(|n| !n.is_finite() || !(-32768.0..=32767.0).contains(n))
        || !target.scale.is_finite()
        || !(0.25..=4.0).contains(&target.scale)
    {
        return Err(LobbyError("invalid training target entity"));
    }
    if establish_baseline && !target.alive {
        return Err(LobbyError("training target baseline must be alive"));
    }
    if target.alive && !establish_baseline {
        return Ok(());
    }

    // 0x46c2f0 absolute entity number, independent of earlier entity records.
    out.bits(0, 1);
    out.bits(1, 1);
    out.bits(u32::from(target.entity_number), 11);
    out.bits(u32::from(!target.alive), 1);
    if !target.alive {
        return Ok(());
    }

    out.bits(1, 1); // changed
    out.bits(1, 1); // force zero position/angle delta base
    out.bits(13, 6); // last changed field index, not a field count
    field_gap(out, 1);
    out.bytes.push(0); // field0: eType GENERAL, -77 raw byte

    let mut previous = 0;
    for (index, coordinate) in [
        (2, target.origin[0]),
        (3, target.origin[1]),
        (4, target.origin[2]),
    ] {
        field_gap(out, index - previous);
        previous = index;
        out.bits(1, 1); // -83/-82/-81 full signed-16 coordinate path
        out.bits(u32::from(coordinate.round_ties_even() as i16 as u16), 16);
    }

    field_gap(out, 9 - previous);
    // field9 +0x54: generic float0 has both a nonzero and precision gate.
    out.bits(1, 1);
    out.bits(1, 1);
    out.bytes.extend_from_slice(&target.scale.to_le_bytes());

    field_gap(out, 13 - 9);
    // field13 +0x98: nonzero gate, remainder pool bits, then a whole byte.
    out.bits(1, 1);
    out.bits(u32::from(target.model), 2);
    out.bytes.push((target.model >> 2) as u8);
    Ok(())
}

fn field_gap(out: &mut Writer, distance: u32) {
    // 0x5a3250(parameter3): a chunk of7 continues the gap.
    out.bits(u32::from(distance == 1), 1);
    if distance == 1 {
        return;
    }
    let mut remaining = distance - 1;
    while remaining >= 7 {
        out.bits(7, 3);
        remaining -= 7;
    }
    out.bits(remaining, 3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::channel::wire::Reader;
    use std::collections::BTreeMap;

    #[derive(Debug, PartialEq)]
    enum EntityRecord {
        Removed(u16),
        General(u16, BTreeMap<u32, u32>),
    }

    fn layout() -> serde_json::Value {
        let layout: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/entity-state-fields-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(layout["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(layout["entity_type"], 0);
        assert_eq!(layout["table_va"], 0xc525a8);
        assert_eq!(layout["fields"].as_array().unwrap().len(), 61);
        layout
    }

    // Decode by current executable field offsets and codecs, separately from
    // the writer. Raw bytes continue alongside partially consumed pool bytes.
    fn read_entity(r: &mut Reader<'_>) -> EntityRecord {
        assert_eq!(r.bits(1).unwrap(), 0);
        assert_eq!(r.bits(1).unwrap(), 1);
        let number = r.bits(11).unwrap() as u16;
        if r.bits(1).unwrap() != 0 {
            return EntityRecord::Removed(number);
        }
        assert_eq!(r.bits(1).unwrap(), 1);
        assert_eq!(r.bits(1).unwrap(), 1); // absolute position base
        let last = r.bits(6).unwrap() as i32;
        let table = layout();
        let mut index = -1;
        let mut fields = BTreeMap::new();
        while index < last {
            let mut distance = 1;
            if r.bits(1).unwrap() == 0 {
                loop {
                    let part = r.bits(3).unwrap();
                    distance += part;
                    if part != 7 {
                        break;
                    }
                }
            }
            index += distance as i32;
            assert!(index <= last && index >= 0);
            let entry = &table["fields"][index as usize];
            let offset = entry[0].as_u64().unwrap() as u32;
            let codec = entry[1].as_i64().unwrap();
            let value = match codec {
                -77 => u32::from(r.u8().unwrap()),
                -83..=-81 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    r.bits(16).unwrap()
                }
                0 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    assert_eq!(r.bits(1).unwrap(), 1);
                    r.u32().unwrap()
                }
                width if width > 0 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    let width = width as usize;
                    let remainder = width % 8;
                    let mut value = r.bits(remainder).unwrap();
                    for shift in (remainder..width).step_by(8) {
                        value |= u32::from(r.u8().unwrap()) << shift;
                    }
                    value
                }
                _ => panic!("unexpected target entity codec {codec}"),
            };
            assert!(fields.insert(offset, value).is_none());
        }
        EntityRecord::General(number, fields)
    }

    fn target() -> TargetSnapshot {
        TargetSnapshot {
            entity_number: 19,
            model: 156,
            origin: [-471.5, -800.5, -113.4],
            scale: 1.0,
            alive: true,
        }
    }

    #[test]
    fn baseline_decodes_with_current_table_at_every_pool_alignment() {
        for prefix_width in 0..8 {
            let mut out = Writer::default();
            out.bits(0b101_0110, prefix_width);
            out.bytes.push(0xa6);
            write_target(&mut out, target(), true).unwrap();
            out.bits(0, 1);
            out.bits(1, 1);
            out.bits(2047, 11);
            out.bytes.push(6);

            let mut r = Reader::new(&out.bytes);
            assert_eq!(
                r.bits(prefix_width).unwrap(),
                0b101_0110 & ((1 << prefix_width) - 1)
            );
            assert_eq!(r.u8().unwrap(), 0xa6);
            let EntityRecord::General(number, fields) = read_entity(&mut r) else {
                panic!("baseline target removed");
            };
            assert_eq!(number, 19);
            assert_eq!(fields.len(), 6);
            assert_eq!(fields[&4], 0);
            // GENERAL fields2/3 are X/Y, while PLAYER fields1/2 are Y/X.
            for (offset, coordinate) in [(0x18, -472i16), (0x1c, -800), (0x20, -113)] {
                assert_eq!(fields[&offset] as u16 as i16, coordinate);
            }
            assert_eq!(fields[&0x54], 1.0f32.to_bits());
            assert_eq!(fields[&0x98], 156);
            assert_eq!(r.bits(2).unwrap(), 2);
            assert_eq!(r.bits(11).unwrap(), 2047);
            assert_eq!(r.u8().unwrap(), 6);
            r.finish().unwrap();
        }
    }

    #[test]
    fn every_dead_frame_removes_target_and_live_respawn_inherits_baseline() {
        let initial = target();
        for alive in [true, false, false, true] {
            let mut out = Writer::default();
            out.bits(5, 3);
            let before = out.bytes.clone();
            write_target(&mut out, TargetSnapshot { alive, ..initial }, false).unwrap();
            if alive {
                assert_eq!(out.bytes, before);
            }
            out.bits(0, 1);
            out.bits(1, 1);
            out.bits(2047, 11);
            let mut r = Reader::new(&out.bytes);
            assert_eq!(r.bits(3).unwrap(), 5);
            if !alive {
                assert_eq!(read_entity(&mut r), EntityRecord::Removed(19));
            }
            assert_eq!(r.bits(2).unwrap(), 2);
            assert_eq!(r.bits(11).unwrap(), 2047);
            r.finish().unwrap();
        }
    }

    #[test]
    fn extreme_valid_entity_model_and_coordinates_decode() {
        let target = TargetSnapshot {
            entity_number: 2046,
            model: 1023,
            origin: [-32768.0, 32767.0, 0.0],
            scale: 1.0,
            alive: true,
        };
        let mut out = Writer::default();
        write_target(&mut out, target, true).unwrap();
        let mut r = Reader::new(&out.bytes);
        let EntityRecord::General(number, fields) = read_entity(&mut r) else {
            panic!("valid baseline removed");
        };
        assert_eq!(number, 2046);
        assert_eq!(fields[&0x98], 1023);
        assert_eq!(fields[&0x18] as u16 as i16, -32768);
        assert_eq!(fields[&0x1c] as u16 as i16, 32767);
        r.finish().unwrap();
    }

    #[test]
    fn enlarged_target_scale_is_sent_to_the_renderer() {
        let mut out = Writer::default();
        write_target(
            &mut out,
            TargetSnapshot {
                scale: 2.0,
                ..target()
            },
            true,
        )
        .unwrap();
        let EntityRecord::General(_, fields) = read_entity(&mut Reader::new(&out.bytes)) else {
            panic!("target removed");
        };
        assert_eq!(fields[&0x54], 2.0f32.to_bits());
        for scale in [0.0, f32::NAN, 4.1] {
            assert!(
                write_target(
                    &mut Writer::default(),
                    TargetSnapshot { scale, ..target() },
                    true
                )
                .is_err()
            );
        }
    }

    #[test]
    fn invalid_targets_do_not_partially_mutate_snapshot() {
        let initial = target();
        let invalid = [
            TargetSnapshot {
                entity_number: 0,
                ..initial
            },
            TargetSnapshot {
                entity_number: 2047,
                ..initial
            },
            TargetSnapshot {
                model: 0,
                ..initial
            },
            TargetSnapshot {
                model: 1024,
                ..initial
            },
            TargetSnapshot {
                origin: [f32::NAN, 0.0, 0.0],
                ..initial
            },
            TargetSnapshot {
                origin: [0.0, f32::INFINITY, 0.0],
                ..initial
            },
            TargetSnapshot {
                origin: [32768.0, 0.0, 0.0],
                ..initial
            },
            TargetSnapshot {
                origin: [-32769.0, 0.0, 0.0],
                ..initial
            },
            TargetSnapshot {
                alive: false,
                ..initial
            },
        ];
        for target in invalid {
            let mut out = Writer::default();
            out.bits(5, 3);
            out.bytes.push(0xa6);
            let before = out.bytes.clone();
            assert!(write_target(&mut out, target, true).is_err());
            assert_eq!(out.bytes, before);
        }
    }
}
