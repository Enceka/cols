//! Persistent 3.3.3.3 GENERAL smoke entities, table 0xc525a8.
//! 0x4450a0 rebuilds FX from snapshots on initialization and replay transitions:
//! flags&0x40, start <= cgTime <= end, weaponDef+0x918, original start time.
//! This record does not replace the ordinary grenade explosion event or force
//! an already running FX to stop when the GENERAL entity expires.
use crate::game::gamestate::Writer;
use crate::lobby::channel::LobbyError;

pub const FIRST_ENTITY: u16 = 1708;
pub const END_ENTITY: u16 = 1740;
/// 0x40 marks smoke for reconstruction; 0x10 hides its model. The smoke
/// consumer does not test 0x80000. Keep this flag word with the native reset bit.
pub const SMOKE_FLAGS: u32 = 0x80050;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmokeSnapshot {
    pub entity_number: u16,
    pub origin: [f32; 3],
    pub weapon: u16,
    /// Absolute time on the snapshot's clock, including any caller time shift.
    pub start_time: u32,
    /// Inclusive reconstruction cutoff on the same clock as start_time.
    pub end_time: u32,
    pub present: bool,
    /// Server slot generation; its low bit toggles flags bit0x2 so consecutive
    /// uses of a still-present slot reset its native state. trType stays 1.
    pub generation: u32,
}

impl SmokeSnapshot {
    fn validate(self) -> Result<(), LobbyError> {
        if !(FIRST_ENTITY..END_ENTITY).contains(&self.entity_number)
            || !(1..900).contains(&self.weapon)
            || self
                .origin
                .iter()
                .any(|v| !v.is_finite() || !(-32768.0..=32767.0).contains(v))
            || self.start_time > self.end_time
            || self.end_time > i32::MAX as u32
        {
            return Err(LobbyError("invalid persistent smoke snapshot"));
        }
        Ok(())
    }

    fn flags(self) -> u32 {
        SMOKE_FLAGS | ((self.generation & 1) * 2)
    }
}

/// Append sorted smoke entity records before the caller's entity sentinel.
/// Full frames use an empty baseline slice and include every present smoke.
/// Delta frames use the exact selected snapshot baseline, including any smoke
/// in a pinned full frame: omission or present=false emits explicit removal.
/// A deleted baseline state supplies zero scalar bases on subsequent reuse.
/// The caller owns expiry, slot allocation, and shifting BOTH absolute times.
pub(crate) fn write(
    out: &mut Writer,
    smokes: &[SmokeSnapshot],
    baselines: &[SmokeSnapshot],
) -> Result<(), LobbyError> {
    for states in [smokes, baselines] {
        for (i, state) in states.iter().enumerate() {
            state.validate()?;
            if i > 0 && states[i - 1].entity_number >= state.entity_number {
                return Err(LobbyError("smoke entities must be sorted and unique"));
            }
        }
    }
    let numbers: std::collections::BTreeSet<_> = smokes
        .iter()
        .chain(baselines)
        .map(|s| s.entity_number)
        .collect();
    for number in numbers {
        let current = smokes
            .iter()
            .find(|s| s.entity_number == number && s.present);
        let base = baselines
            .iter()
            .find(|s| s.entity_number == number && s.present);
        if current == base {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1); // absolute entity index, independent of other modules
        out.bits(u32::from(number), 11);
        let Some(current) = current else {
            out.bits(1, 1); // removed from the selected baseline
            continue;
        };
        out.bits(0, 1); // retained
        out.bits(1, 1); // changed
        out.bits(1, 1); // zero position/angle bases, scalar bases still retained
        out.bits(28, 6); // last changed GENERAL field index
        let mut previous = -1;
        for index in [0, 1, 2, 3, 4, 7, 11, 12, 28] {
            gap(out, (index - previous) as u32);
            previous = index;
            match index {
                0 => out.bytes.push(0), // -77 raw byte: GENERAL
                1 => {
                    out.bits(1, 1); // -98 ABSOLUTE, no generic nonzero gate/XOR
                    out.bits(current.flags(), 30);
                }
                2..=4 => {
                    out.bits(1, 1); // -83/-82/-81 absolute signed16 coordinate
                    out.bits(
                        u32::from(
                            current.origin[index as usize - 2].round_ties_even() as i16 as u16
                        ),
                        16,
                    );
                }
                7 => scalar(
                    out,
                    u32::from(current.weapon),
                    base.map_or(0, |b| u32::from(b.weapon)),
                    32,
                ),
                11 => scalar(
                    out,
                    current.start_time,
                    base.map_or(0, |b| b.start_time),
                    32,
                ),
                12 => {
                    out.bits(1, 1); // -97 absolute branch, no XOR/nonzero gate
                    out.bytes.extend_from_slice(&current.end_time.to_le_bytes());
                }
                28 => scalar(out, 1, u32::from(base.is_some()), 8), // trType INTERPOLATE
                _ => unreachable!("fixed GENERAL smoke fields"),
            }
        }
    }
    Ok(())
}

fn scalar(out: &mut Writer, value: u32, base: u32, width: usize) {
    out.bits(u32::from(value != 0), 1);
    if value == 0 {
        return;
    }
    let delta = value ^ base;
    let remainder = width % 8;
    out.bits(delta, remainder);
    for shift in (remainder..width).step_by(8) {
        out.bytes.push((delta >> shift) as u8);
    }
}

fn gap(out: &mut Writer, distance: u32) {
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

    type Entity = BTreeMap<u32, u32>;
    type World = BTreeMap<u16, Entity>;

    fn fixture() -> serde_json::Value {
        let table: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/entity-state-fields-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(table["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(table["table_va"], 0xc525a8);
        assert_eq!(table["entity_type"], 0);
        assert_eq!(table["fields"].as_array().unwrap().len(), 61);
        table
    }

    // Decode from the independently extracted field table, never from the
    // encoder's index list or gap/scalar helpers. Preserve omitted base fields.
    fn decode(reader: &mut Reader<'_>, base: &World) -> (World, Vec<Vec<usize>>) {
        let table = fixture();
        let mut world = base.clone();
        let mut changed = Vec::new();
        loop {
            assert_eq!(reader.bits(2).unwrap(), 2);
            let number = reader.bits(11).unwrap() as u16;
            if number == 2047 {
                break;
            }
            if reader.bits(1).unwrap() == 1 {
                assert!(world.remove(&number).is_some());
                changed.push(Vec::new());
                continue;
            }
            assert_eq!(reader.bits(1).unwrap(), 1);
            assert_eq!(reader.bits(1).unwrap(), 1); // zero position base only
            let last = reader.bits(6).unwrap() as i32;
            let mut index = -1;
            let entity = world.entry(number).or_default();
            let mut indices = Vec::new();
            while index < last {
                let mut distance = 1;
                if reader.bits(1).unwrap() == 0 {
                    loop {
                        let chunk = reader.bits(3).unwrap();
                        distance += chunk;
                        if chunk != 7 {
                            break;
                        }
                    }
                }
                index += distance as i32;
                assert!((0..=last).contains(&index));
                let entry = &table["fields"][index as usize];
                let offset = entry[0].as_u64().unwrap() as u32;
                let codec = entry[1].as_i64().unwrap();
                let previous = entity.get(&offset).copied().unwrap_or(0);
                let value = match codec {
                    -77 => u32::from(reader.u8().unwrap()),
                    -98 => {
                        assert_eq!(reader.bits(1).unwrap(), 1);
                        reader.bits(30).unwrap()
                    }
                    -97 => {
                        assert_eq!(reader.bits(1).unwrap(), 1);
                        reader.u32().unwrap()
                    }
                    -83..=-81 => {
                        assert_eq!(reader.bits(1).unwrap(), 1);
                        f32::from(reader.bits(16).unwrap() as u16 as i16).to_bits()
                    }
                    width if width > 0 => {
                        if reader.bits(1).unwrap() == 0 {
                            0
                        } else {
                            let width = width as usize;
                            let remainder = width % 8;
                            let mut delta = reader.bits(remainder).unwrap();
                            for shift in (remainder..width).step_by(8) {
                                delta |= u32::from(reader.u8().unwrap()) << shift;
                            }
                            delta ^ previous
                        }
                    }
                    _ => panic!("unexpected native GENERAL codec {codec}"),
                };
                entity.insert(offset, value);
                indices.push(index as usize);
            }
            changed.push(indices);
        }
        (world, changed)
    }

    fn smoke() -> SmokeSnapshot {
        SmokeSnapshot {
            entity_number: 1708,
            origin: [10.5, -2.5, -0.5],
            weapon: 95,
            start_time: 0x12345678,
            end_time: 0x1234a578,
            present: true,
            generation: 255,
        }
    }

    fn expected(states: &[SmokeSnapshot]) -> World {
        states
            .iter()
            .filter(|s| s.present)
            .map(|s| {
                (
                    s.entity_number,
                    [
                        (4, 0),
                        (8, 0x80050 | ((s.generation & 1) << 1)),
                        (
                            0x18,
                            f32::from(s.origin[0].round_ties_even() as i16).to_bits(),
                        ),
                        (
                            0x1c,
                            f32::from(s.origin[1].round_ties_even() as i16).to_bits(),
                        ),
                        (
                            0x20,
                            f32::from(s.origin[2].round_ties_even() as i16).to_bits(),
                        ),
                        (0xd0, u32::from(s.weapon)),
                        (0x58, s.start_time),
                        (0x7c, s.end_time),
                        (0xc, 1),
                    ]
                    .into(),
                )
            })
            .collect()
    }

    fn round_trip(
        alignment: usize,
        current: &[SmokeSnapshot],
        baselines: &[SmokeSnapshot],
    ) -> Vec<Vec<usize>> {
        let mut out = Writer::default();
        out.bits(0x55, alignment);
        out.bytes.push(0xa6);
        write(&mut out, current, baselines).unwrap();
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(2047, 11);
        out.bytes.push(0xee);
        out.bits(5, 3);
        let mut reader = Reader::new(&out.bytes);
        assert_eq!(
            reader.bits(alignment).unwrap(),
            0x55 & ((1 << alignment) - 1)
        );
        assert_eq!(reader.u8().unwrap(), 0xa6);
        let (decoded, changed) = decode(&mut reader, &expected(baselines));
        assert_eq!(decoded, expected(current));
        assert_eq!(reader.u8().unwrap(), 0xee);
        assert_eq!(reader.bits(3).unwrap(), 5);
        changed
    }

    #[test]
    fn full_smoke_baselines_match_native_general_fields_at_every_alignment() {
        let table = fixture();
        for (index, offset, codec, hint) in [
            (0, 4, -77, 0),
            (1, 8, -98, 0),
            (2, 0x18, -83, 6),
            (3, 0x1c, -82, 7),
            (4, 0x20, -81, 8),
            (7, 0xd0, 32, 0),
            (11, 0x58, 32, 0),
            (12, 0x7c, -97, 0),
            (28, 0xc, 8, 0),
        ] {
            assert_eq!(
                table["fields"][index],
                serde_json::json!([offset, codec, hint])
            );
        }
        let current = [
            smoke(),
            SmokeSnapshot {
                entity_number: 1739,
                origin: [-32768.0, 32767.0, 0.0],
                weapon: 899,
                start_time: 0,
                end_time: i32::MAX as u32,
                generation: 0,
                ..smoke()
            },
        ];
        for alignment in 0..8 {
            let changed = round_trip(alignment, &current, &[]);
            assert_eq!(changed, vec![vec![0, 1, 2, 3, 4, 7, 11, 12, 28]; 2]);
            assert!(round_trip(alignment, &current, &current).is_empty());
        }
    }

    #[test]
    fn shifted_times_reuse_deletion_and_pinned_baselines_decode_at_every_alignment() {
        let first = smoke();
        let shifted = SmokeSnapshot {
            start_time: first.start_time - 5000,
            end_time: first.end_time - 5000,
            ..first
        };
        let rebased = SmokeSnapshot {
            start_time: first.start_time + 5000,
            end_time: first.end_time + 5000,
            ..first
        };
        let reused = SmokeSnapshot {
            origin: [-312.5, -800.5, 42.5],
            weapon: 232,
            start_time: 600,
            end_time: 14000,
            generation: 256,
            ..first
        };
        let zero = SmokeSnapshot {
            start_time: 0,
            end_time: 0,
            ..reused
        };
        let deleted = SmokeSnapshot {
            present: false,
            ..first
        };
        for alignment in 0..8 {
            round_trip(alignment, &[shifted], &[first]);
            round_trip(alignment, &[rebased], &[first]);
            round_trip(alignment, &[reused], &[shifted]);
            round_trip(alignment, &[zero], &[reused]);
            round_trip(alignment, &[reused], &[first]); // selected pinned full
            assert_eq!(
                round_trip(alignment, &[], &[first]),
                vec![Vec::<usize>::new()]
            );
            assert_eq!(
                round_trip(alignment, &[], &[first]),
                vec![Vec::<usize>::new()]
            );
            round_trip(alignment, &[deleted], &[first]);
            round_trip(alignment, &[reused], &[]); // deleted from selected base
            round_trip(alignment, &[reused], &[deleted]); // no stale scalar XOR
            assert!(round_trip(alignment, &[], &[deleted]).is_empty());
            assert!(round_trip(alignment, &[deleted], &[]).is_empty());
        }
    }

    #[test]
    fn invalid_current_and_baseline_states_leave_the_writer_unchanged() {
        let valid = smoke();
        for invalid in [
            SmokeSnapshot {
                entity_number: 1707,
                ..valid
            },
            SmokeSnapshot {
                entity_number: 1740,
                ..valid
            },
            SmokeSnapshot { weapon: 0, ..valid },
            SmokeSnapshot {
                weapon: 900,
                ..valid
            },
            SmokeSnapshot {
                origin: [f32::NAN, 0.0, 0.0],
                ..valid
            },
            SmokeSnapshot {
                origin: [0.0, f32::INFINITY, 0.0],
                ..valid
            },
            SmokeSnapshot {
                origin: [0.0, 0.0, 32767.5],
                ..valid
            },
            SmokeSnapshot {
                start_time: valid.end_time + 1,
                ..valid
            },
            SmokeSnapshot {
                end_time: i32::MAX as u32 + 1,
                ..valid
            },
        ] {
            for (current, base) in [
                (&[invalid][..], &[valid][..]),
                (&[valid][..], &[invalid][..]),
            ] {
                let mut out = Writer::default();
                out.bits(5, 3);
                let before = out.bytes.clone();
                assert!(write(&mut out, current, base).is_err());
                assert_eq!(out.bytes, before);
            }
        }
        let next = SmokeSnapshot {
            entity_number: 1709,
            ..valid
        };
        for states in [[valid, valid], [next, valid]] {
            let mut out = Writer::default();
            assert!(write(&mut out, &states, &[]).is_err());
            assert!(write(&mut out, &[], &states).is_err());
            assert!(out.bytes.is_empty());
        }
    }
}
