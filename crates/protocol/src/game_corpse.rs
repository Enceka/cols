//! Measured 3.3.3.3 CORPSE entities: table 0xc52e48, renderer 0x518740.
//! ResetEntity 0x51ad90 copies the donor's appearance and, with 0x80000, its
//! human animation state into the corpse's separately initialized tree.
use crate::{game_state::Writer, lobby::LobbyError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpsePhase {
    Playing,
    /// Special captured death nodes (metadata flag 0x40) freeze at time 1 when
    /// the copy-pose flag clears and their animation toggle changes.
    Frozen,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorpseSnapshot {
    pub entity_number: u16,
    pub donor_client: u8,
    pub origin: [f32; 3],
    pub angles: [u16; 3],
    pub legs_animation: u16,
    pub torso_animation: u16,
    pub weapon: u16,
    pub present: bool,
    pub phase: CorpsePhase,
    /// Advances once per actual death. Its low bit toggles the native teleport
    /// reset so reusing a still-present slot copies a new donor pose.
    pub death_sequence: u32,
}

impl CorpseSnapshot {
    fn validate(self) -> Result<(), LobbyError> {
        let animation = |value: u16| value < 2048 && value & 0x3ff != 0;
        if !matches!((self.entity_number, self.donor_client), (19, 1) | (20, 0))
            || self
                .origin
                .iter()
                .any(|n| !n.is_finite() || !(-32768.0..=32767.0).contains(n))
            || !animation(self.legs_animation)
            || !animation(self.torso_animation)
            || !(1..900).contains(&self.weapon)
            || (self.present && self.death_sequence == 0)
        {
            return Err(LobbyError("invalid captured corpse state"));
        }
        Ok(())
    }

    fn flags(self) -> u32 {
        ((self.death_sequence & 1) * 2)
            | if self.phase == CorpsePhase::Playing {
                0x80000
            } else {
                0
            }
    }

    fn animation(self, value: u16) -> u32 {
        u32::from(value & 0x3ff)
            | if self.phase == CorpsePhase::Playing {
                0x400
            } else {
                0
            }
    }
}

/// Write one complete entity record, before the caller's entity sentinel.
/// Initial world baselines normally have no corpse, so ordinary frames use
/// None as their zero entity base. A caller that deliberately includes a corpse
/// in a later full baseline must retain it and supply that explicit base.
/// Present=false emits removal on every ordinary frame, including a pinned
/// baseline containing the corpse. The caller owns corpse lifetime and must
/// ensure the donor's real clientInfo is valid before the first appearance.
pub(crate) fn write_corpse(
    out: &mut Writer,
    corpse: CorpseSnapshot,
    baseline: Option<CorpseSnapshot>,
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    corpse.validate()?;
    if establish_baseline && baseline.is_some() {
        return Err(LobbyError(
            "full corpse baseline cannot have a prior entity base",
        ));
    }
    if let Some(base) = baseline {
        base.validate()?;
        if !base.present
            || base.entity_number != corpse.entity_number
            || base.donor_client != corpse.donor_client
        {
            return Err(LobbyError("corpse pinned base identity mismatch"));
        }
    }
    if establish_baseline && !corpse.present {
        return Ok(());
    }

    out.bits(0, 1);
    out.bits(1, 1);
    out.bits(u32::from(corpse.entity_number), 11);
    out.bits(u32::from(!corpse.present), 1);
    if !corpse.present {
        return Ok(());
    }
    out.bits(1, 1); // changed
    out.bits(1, 1); // absolute position/angle bases
    out.bits(46, 6); // last changed corpse field
    let mut previous = -1;
    for index in [0, 1, 2, 3, 4, 6, 7, 8, 9, 12, 13, 14, 38, 39, 41, 46] {
        gap(out, (index - previous) as u32);
        previous = index;
        match index {
            0 => out.bytes.push(2),
            1 => {
                // -98 absolute 30-bit branch, not a generic 32-bit XOR integer.
                out.bits(1, 1);
                out.bits(corpse.flags(), 30);
            }
            2 | 3 | 6 => {
                let axis = match index {
                    2 => 0,
                    3 => 1,
                    _ => 2,
                };
                out.bits(1, 1);
                out.bits(
                    u32::from(corpse.origin[axis].round_ties_even() as i16 as u16),
                    16,
                );
            }
            4 => scalar(out, 0, 0, 8), // stationary root; animation drives the body pose
            7 | 12 | 13 => {
                let axis = match index {
                    7 => 1,
                    12 => 0,
                    _ => 2,
                };
                out.bits(0, 2); // -79 absolute short angle
                out.bytes
                    .extend_from_slice(&corpse.angles[axis].to_le_bytes());
            }
            8 => scalar(
                out,
                u32::from(corpse.donor_client),
                baseline.map_or(0, |b| u32::from(b.donor_client)),
                6,
            ),
            9 => scalar(
                out,
                corpse.animation(corpse.legs_animation),
                baseline.map_or(0, |b| b.animation(b.legs_animation)),
                11,
            ),
            14 => scalar(
                out,
                corpse.animation(corpse.torso_animation),
                baseline.map_or(0, |b| b.animation(b.torso_animation)),
                11,
            ),
            38 => scalar(
                out,
                u32::from(corpse.weapon),
                baseline.map_or(0, |b| u32::from(b.weapon)),
                16,
            ),
            39 => scalar(out, 0, 0, 16),
            41 => scalar(
                out,
                u32::from(corpse.weapon),
                baseline.map_or(0, |b| u32::from(b.weapon)),
                32,
            ),
            46 => {
                out.bits(1, 1);
                out.bits(1, 1);
                let base = if baseline.is_some() {
                    1.0f32.to_bits()
                } else {
                    0
                };
                out.bytes
                    .extend_from_slice(&(1.0f32.to_bits() ^ base).to_le_bytes());
            }
            _ => unreachable!("fixed corpse fields"),
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
    let mut left = distance - 1;
    while left >= 7 {
        out.bits(7, 3);
        left -= 7;
    }
    out.bits(left, 3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_wire::Reader;
    use std::collections::BTreeMap;

    fn fixture() -> serde_json::Value {
        let table: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/corpse-entity-fields-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(table["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(table["table_va"], 0xc52e48);
        assert_eq!(table["entity_type"], 2);
        assert_eq!(table["fields"].as_array().unwrap().len(), 61);
        assert_eq!(table["captured_death_animation"]["index"], 47);
        assert_eq!(table["captured_death_animation"]["flags"], 64);
        table
    }

    fn decode(r: &mut Reader<'_>, base: &BTreeMap<u32, u32>) -> (u16, Option<BTreeMap<u32, u32>>) {
        assert_eq!(r.bits(2).unwrap(), 2);
        let number = r.bits(11).unwrap() as u16;
        if r.bits(1).unwrap() != 0 {
            return (number, None);
        }
        assert_eq!(r.bits(2).unwrap(), 3);
        let last = r.bits(6).unwrap() as i32;
        let table = fixture();
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
            assert!((0..=last).contains(&index));
            let field = &table["fields"][index as usize];
            let offset = field[0].as_u64().unwrap() as u32;
            let codec = field[1].as_i64().unwrap();
            let b = base.get(&offset).copied().unwrap_or(0);
            let value = match codec {
                -77 => u32::from(r.u8().unwrap()),
                -98 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    r.bits(30).unwrap()
                }
                -83..=-81 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    r.bits(16).unwrap()
                }
                -79 => {
                    assert_eq!(r.bits(2).unwrap(), 0);
                    u32::from(r.u16().unwrap())
                }
                0 => {
                    assert_eq!(r.bits(2).unwrap(), 3);
                    r.u32().unwrap() ^ b
                }
                n if n > 0 => {
                    if r.bits(1).unwrap() == 0 {
                        0
                    } else {
                        let width = n as usize;
                        let remainder = width % 8;
                        let mut value = r.bits(remainder).unwrap();
                        for shift in (remainder..width).step_by(8) {
                            value |= u32::from(r.u8().unwrap()) << shift;
                        }
                        value ^ b
                    }
                }
                _ => panic!("unexpected corpse codec {codec}"),
            };
            assert!(fields.insert(offset, value).is_none());
        }
        (number, Some(fields))
    }

    fn corpse() -> CorpseSnapshot {
        CorpseSnapshot {
            entity_number: 19,
            donor_client: 1,
            origin: [-328.5, -800.5, -119.4],
            angles: [2345, 65535, 12345],
            legs_animation: 47,
            torso_animation: 47,
            weapon: 352,
            present: true,
            phase: CorpsePhase::Playing,
            death_sequence: 1,
        }
    }

    #[test]
    fn two_corpse_slots_decode_copy_pose_death_animation_and_unit_scale_at_every_alignment() {
        for (number, donor) in [(19, 1), (20, 0)] {
            for align in 0..8 {
                let state = CorpseSnapshot {
                    entity_number: number,
                    donor_client: donor,
                    ..corpse()
                };
                let mut out = Writer::default();
                out.bits(0x55, align);
                out.bytes.push(0xa6);
                write_corpse(&mut out, state, None, false).unwrap();
                out.bits(2047, 11);
                out.bytes.push(6);
                let mut r = Reader::new(&out.bytes);
                assert_eq!(r.bits(align).unwrap(), 0x55 & ((1 << align) - 1));
                assert_eq!(r.u8().unwrap(), 0xa6);
                let (number, fields) = decode(&mut r, &BTreeMap::new());
                let fields = fields.unwrap();
                assert_eq!(number, state.entity_number);
                assert_eq!(fields.len(), 16);
                assert_eq!(fields[&4], 2);
                assert_eq!(fields[&8], 0x80002);
                assert_eq!(fields[&0x18] as u16 as i16, -328);
                assert_eq!(fields[&0x1c] as u16 as i16, -800);
                assert_eq!(fields[&0x20] as u16 as i16, -119);
                assert_eq!(fields[&0xc], 0);
                assert_eq!(fields[&0x9c], u32::from(donor));
                assert_eq!(fields[&0xd4], 0x42f);
                assert_eq!(fields[&0xd8], 0x42f);
                assert_eq!(fields[&0x3c], 2345);
                assert_eq!(fields[&0x40], 65535);
                assert_eq!(fields[&0x44], 12345);
                assert_eq!(fields[&0x60], 352);
                assert_eq!(fields[&0x64], 0);
                assert_eq!(fields[&0xd0], 352);
                assert_eq!(fields[&0x54], 1.0f32.to_bits());
                assert_eq!(r.bits(11).unwrap(), 2047);
                assert_eq!(r.u8().unwrap(), 6);
                r.finish().unwrap();
            }
        }
    }

    #[test]
    fn explicit_baseline_freeze_and_new_death_reset_preserve_native_state_semantics() {
        let initial = corpse();
        let mut out = Writer::default();
        write_corpse(&mut out, initial, None, true).unwrap();
        let (_, base) = decode(&mut Reader::new(&out.bytes), &BTreeMap::new());
        let base = base.unwrap();
        for current in [
            initial,
            CorpseSnapshot {
                phase: CorpsePhase::Frozen,
                ..initial
            },
            CorpseSnapshot {
                death_sequence: 2,
                weapon: 353,
                ..initial
            },
        ] {
            let mut out = Writer::default();
            write_corpse(&mut out, current, Some(initial), false).unwrap();
            let mut r = Reader::new(&out.bytes);
            let (_, fields) = decode(&mut r, &base);
            let fields = fields.unwrap();
            r.finish().unwrap();
            let (flags, animation) = match (current.phase, current.death_sequence) {
                (CorpsePhase::Frozen, _) => (2, 47),
                (_, 2) => (0x80000, 0x42f),
                _ => (0x80002, 0x42f),
            };
            assert_eq!(fields[&8], flags);
            assert_eq!(fields[&0xd4], animation);
            assert_eq!(fields[&0xd8], animation);
            assert_eq!(fields[&0x60], u32::from(current.weapon));
            assert_eq!(fields[&0xd0], u32::from(current.weapon));
            assert_eq!(fields[&0x54], 1.0f32.to_bits());
        }
    }

    #[test]
    fn absent_corpses_do_not_enter_initial_baseline_and_remove_on_every_ordinary_frame() {
        let absent = CorpseSnapshot {
            present: false,
            ..corpse()
        };
        let mut out = Writer::default();
        out.bits(5, 3);
        let before = out.bytes.clone();
        write_corpse(&mut out, absent, None, true).unwrap();
        assert_eq!(out.bytes, before);
        for base in [None, Some(corpse())] {
            for _ in 0..2 {
                let mut out = Writer::default();
                write_corpse(&mut out, absent, base, false).unwrap();
                out.bits(2047, 11);
                let mut r = Reader::new(&out.bytes);
                assert_eq!(decode(&mut r, &BTreeMap::new()), (19, None));
                assert_eq!(r.bits(11).unwrap(), 2047);
                r.finish().unwrap();
            }
        }
    }

    #[test]
    fn invalid_donor_pose_and_baseline_fail_without_partial_writes() {
        let c = corpse();
        for invalid in [
            CorpseSnapshot {
                entity_number: 18,
                ..c
            },
            CorpseSnapshot {
                donor_client: 0,
                ..c
            },
            CorpseSnapshot {
                entity_number: 20,
                ..c
            },
            CorpseSnapshot {
                legs_animation: 0,
                ..c
            },
            CorpseSnapshot {
                torso_animation: 1024,
                ..c
            },
            CorpseSnapshot { weapon: 0, ..c },
            CorpseSnapshot {
                death_sequence: 0,
                ..c
            },
            CorpseSnapshot {
                origin: [f32::NAN, 0.0, 0.0],
                ..c
            },
            CorpseSnapshot {
                origin: [32768.0, 0.0, 0.0],
                ..c
            },
        ] {
            let mut out = Writer::default();
            out.bits(5, 3);
            out.bytes.push(0xa6);
            let before = out.bytes.clone();
            assert!(write_corpse(&mut out, invalid, None, false).is_err());
            assert_eq!(out.bytes, before);
        }
        let mut out = Writer::default();
        assert!(write_corpse(&mut out, c, Some(c), true).is_err());
        assert!(
            write_corpse(
                &mut out,
                c,
                Some(CorpseSnapshot {
                    present: false,
                    ..c
                }),
                false
            )
            .is_err()
        );
        let other = CorpseSnapshot {
            entity_number: 20,
            donor_client: 0,
            ..c
        };
        assert!(write_corpse(&mut out, c, Some(other), false).is_err());
        assert!(out.bytes.is_empty());
    }
}
