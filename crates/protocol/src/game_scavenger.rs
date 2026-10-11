//! Native ITEM3: modelIndex encodes a weapon plus 900 * world-model slot.
//! Descriptor0xc543b8, model consumer0x501320, touch consumers0x4514f0/0x4128c0.
use crate::{game_state::Writer, lobby::LobbyError};

pub const FIRST_ENTITY: u16 = 1740;
pub const END_ENTITY: u16 = 1800;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BagSnapshot {
    pub entity_number: u16,
    pub origin: [f32; 3],
    pub weapon: u16,
    pub generation: u32,
}
impl BagSnapshot {
    fn valid(self) -> bool {
        (FIRST_ENTITY..END_ENTITY).contains(&self.entity_number)
            && (1..900).contains(&self.weapon)
            && self
                .origin
                .iter()
                .all(|v| v.is_finite() && (-32768.0..=32767.0).contains(v))
    }
}

pub(crate) fn write(
    out: &mut Writer,
    bags: &[BagSnapshot],
    bases: &[BagSnapshot],
) -> Result<(), LobbyError> {
    for states in [bags, bases] {
        if states.iter().any(|s| !s.valid())
            || states
                .windows(2)
                .any(|w| w[0].entity_number >= w[1].entity_number)
        {
            return Err(LobbyError("invalid or unordered scavenger entities"));
        }
    }
    let numbers: std::collections::BTreeSet<_> =
        bags.iter().chain(bases).map(|s| s.entity_number).collect();
    for number in numbers {
        let current = bags.iter().find(|s| s.entity_number == number);
        let base = bases.iter().find(|s| s.entity_number == number);
        if current == base {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(u32::from(number), 11);
        let Some(current) = current else {
            out.bits(1, 1);
            continue;
        };
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(1, 1); // absolute position bases; retain scalar bases
        out.bits(30, 6);
        let mut previous = -1;
        for index in [0, 4, 5, 6, 9, 19, 23, 28, 30] {
            gap(out, (index - previous) as u32);
            previous = index;
            match index {
                0 => out.bytes.push(3),
                4..=6 => {
                    out.bits(1, 1);
                    out.bits(
                        u32::from(
                            current.origin[6 - index as usize].round_ties_even() as i16 as u16
                        ),
                        16,
                    );
                }
                9 => scalar(out, 0, 0, 8), // STATIONARY
                19 => scalar(
                    out,
                    u32::from(current.weapon),
                    base.map_or(0, |s| u32::from(s.weapon)),
                    11,
                ),
                23 => {
                    out.bits(1, 1);
                    out.bits((current.generation & 1) * 2, 30);
                }
                28 => {
                    out.bits(1, 1);
                    out.bits(1, 1); // full-precision float; XOR scalar base
                    out.bytes.extend_from_slice(
                        &(1.0f32.to_bits() ^ if base.is_some() { 1.0f32.to_bits() } else { 0 })
                            .to_le_bytes(),
                    );
                }
                30 => scalar(out, 2047, if base.is_some() { 2047 } else { 0 }, 11),
                _ => unreachable!(),
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
    use crate::game_wire::Reader;
    use std::collections::BTreeMap;
    type World = BTreeMap<u16, BTreeMap<u32, u32>>;
    fn fixture() -> serde_json::Value {
        let f: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/scavenger-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(f["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(f["entity_type"], 3);
        assert_eq!(f["table_va"], 0xc543b8);
        assert_eq!(f["fields"].as_array().unwrap().len(), 61);
        f
    }
    fn expected(bags: &[BagSnapshot]) -> World {
        bags.iter()
            .map(|b| {
                (
                    b.entity_number,
                    [
                        (4, 3),
                        (8, (b.generation & 1) * 2),
                        (0xc, 0),
                        (
                            0x18,
                            f32::from(b.origin[0].round_ties_even() as i16).to_bits(),
                        ),
                        (
                            0x1c,
                            f32::from(b.origin[1].round_ties_even() as i16).to_bits(),
                        ),
                        (
                            0x20,
                            f32::from(b.origin[2].round_ties_even() as i16).to_bits(),
                        ),
                        (0x98, u32::from(b.weapon)),
                        (0x54, 1.0f32.to_bits()),
                        (0x80, 2047),
                    ]
                    .into(),
                )
            })
            .collect()
    }
    fn round_trip(alignment: usize, bags: &[BagSnapshot], bases: &[BagSnapshot]) {
        let mut out = Writer::default();
        out.bits(0x55, alignment);
        write(&mut out, bags, bases).unwrap();
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(2047, 11);
        out.bytes.push(0xa9);
        let mut r = Reader::new(&out.bytes);
        assert_eq!(r.bits(alignment).unwrap(), 0x55 & ((1 << alignment) - 1));
        let mut world = expected(bases);
        let f = fixture();
        loop {
            assert_eq!(r.bits(2).unwrap(), 2);
            let number = r.bits(11).unwrap() as u16;
            if number == 2047 {
                break;
            }
            if r.bits(1).unwrap() == 1 {
                assert!(world.remove(&number).is_some());
                continue;
            }
            assert_eq!(r.bits(2).unwrap(), 3);
            let last = r.bits(6).unwrap() as i32;
            let mut index = -1;
            let state = world.entry(number).or_default();
            while index < last {
                let mut distance = 1;
                if r.bits(1).unwrap() == 0 {
                    loop {
                        let n = r.bits(3).unwrap();
                        distance += n;
                        if n != 7 {
                            break;
                        }
                    }
                }
                index += distance as i32;
                assert!((0..=last).contains(&index));
                let field = &f["fields"][index as usize];
                let offset = field[0].as_u64().unwrap() as u32;
                let base = state.get(&offset).copied().unwrap_or(0);
                let codec = field[1].as_i64().unwrap();
                let value = match codec {
                    -77 => u32::from(r.u8().unwrap()),
                    -98 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        r.bits(30).unwrap()
                    }
                    -83..=-81 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        f32::from(r.bits(16).unwrap() as u16 as i16).to_bits()
                    }
                    0 => {
                        assert_eq!(r.bits(2).unwrap(), 3);
                        r.u32().unwrap() ^ base
                    }
                    n if n > 0 => {
                        if r.bits(1).unwrap() == 0 {
                            0
                        } else {
                            let n = n as usize;
                            let rem = n % 8;
                            let mut delta = r.bits(rem).unwrap();
                            for shift in (rem..n).step_by(8) {
                                delta |= u32::from(r.u8().unwrap()) << shift;
                            }
                            delta ^ base
                        }
                    }
                    _ => panic!("unexpected ITEM codec {codec}"),
                };
                state.insert(offset, value);
            }
        }
        assert_eq!(world, expected(bags));
        assert_eq!(r.u8().unwrap(), 0xa9);
    }
    fn bag() -> BagSnapshot {
        BagSnapshot {
            entity_number: FIRST_ENTITY,
            origin: [10.5, -2.5, 9.095],
            weapon: 229,
            generation: 1,
        }
    }
    #[test]
    fn native_item_layout_round_trips_creation_reuse_and_deletion_against_selected_bases() {
        let first = bag();
        let reused = BagSnapshot {
            generation: 2,
            origin: [-120.0, 24.0, -32.0],
            ..first
        };
        let second = BagSnapshot {
            entity_number: FIRST_ENTITY + 1,
            ..first
        };
        for alignment in 0..8 {
            round_trip(alignment, &[first, second], &[]);
            round_trip(alignment, &[first, second], &[first, second]);
            round_trip(alignment, &[reused], &[first, second]);
            round_trip(alignment, &[], &[first]);
            round_trip(alignment, &[reused], &[]);
        }
    }
    #[test]
    fn invalid_bag_records_cannot_partially_write_a_packet() {
        for invalid in [
            BagSnapshot {
                entity_number: 1800,
                ..bag()
            },
            BagSnapshot {
                weapon: 900,
                ..bag()
            },
            BagSnapshot {
                origin: [f32::NAN; 3],
                ..bag()
            },
        ] {
            let mut out = Writer::default();
            assert!(write(&mut out, &[bag()], &[invalid]).is_err());
            assert!(out.bytes.is_empty());
        }
        assert!(write(&mut Writer::default(), &[bag(), bag()], &[]).is_err());
    }
}
