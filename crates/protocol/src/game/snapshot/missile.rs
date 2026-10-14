//! Type4 MISSILE fields at0xc55540, using absolute interpolated positions.
use crate::game::gamestate::Writer;
use crate::lobby::channel::LobbyError;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MissileSnapshot {
    pub entity_number: u16,
    pub origin: [f32; 3],
    pub weapon: u16,
    pub present: bool,
    pub generation: u32,
    pub owner: u8,
    pub angles: [u16; 3],
}
pub(crate) fn write(
    out: &mut Writer,
    missiles: &[MissileSnapshot],
    baselines: &[MissileSnapshot],
) -> Result<(), LobbyError> {
    for states in [missiles, baselines] {
        for (i, state) in states.iter().enumerate() {
            if !((1700..1708).contains(&state.entity_number)
                || (1648..1664).contains(&state.entity_number))
                || !(1..900).contains(&state.weapon)
                || state.owner >= 12
                || state
                    .origin
                    .iter()
                    .any(|v| !v.is_finite() || !(-32768.0..=32767.0).contains(v))
                || (i > 0 && states[i - 1].entity_number >= state.entity_number)
            {
                return Err(LobbyError("invalid missile snapshot"));
            }
        }
    }
    let numbers: std::collections::BTreeSet<_> = missiles
        .iter()
        .chain(baselines)
        .map(|m| m.entity_number)
        .collect();
    for number in numbers {
        let current = missiles.iter().find(|m| m.entity_number == number);
        let base = baselines.iter().find(|m| m.entity_number == number);
        if current == base {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(u32::from(number), 11);
        let Some(current) = current.filter(|m| m.present) else {
            out.bits(1, 1);
            continue;
        };
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(1, 1);
        out.bits(36, 6);
        let mut previous = -1;
        for index in [0, 1, 2, 6, 7, 8, 16, 17, 20, 33, 36] {
            gap(out, (index - previous) as u32);
            previous = index;
            match index {
                0 => out.bytes.push(4),
                1 | 2 | 8 => {
                    let axis = if index == 1 {
                        1
                    } else if index == 2 {
                        0
                    } else {
                        2
                    };
                    out.bits(1, 1);
                    out.bits(
                        u32::from(current.origin[axis].round_ties_even() as i16 as u16),
                        16,
                    );
                }
                17 => scalar(out, 1, u32::from(base.is_some()), 8),
                6 | 7 | 16 => {
                    let axis = match index {
                        6 => 2,
                        7 => 0,
                        _ => 1,
                    };
                    out.bits(0, 2);
                    out.bytes
                        .extend_from_slice(&current.angles[axis].to_le_bytes());
                }
                // -98 absolute 30-bit branch, like the corpse reset flags.
                20 => {
                    out.bits(1, 1);
                    out.bits((current.generation & 1) * 2, 30);
                }
                33 => scalar(
                    out,
                    u32::from(current.weapon),
                    base.map_or(0, |b| u32::from(b.weapon)),
                    32,
                ),
                36 => scalar(
                    out,
                    u32::from(current.owner),
                    base.map_or(0, |b| u32::from(b.owner)),
                    11,
                ),
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
    let value = value ^ base;
    let remainder = width % 8;
    out.bits(value, remainder);
    for shift in (remainder..width).step_by(8) {
        out.bytes.push((value >> shift) as u8);
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
    fn fields(reader: &mut Reader<'_>, base: &BTreeMap<u32, u32>) -> BTreeMap<u32, u32> {
        let table: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/missile-fields-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(table["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(table["table_va"], 0xc55540);
        let last = reader.bits(6).unwrap() as i32;
        let mut index = -1;
        let mut result = BTreeMap::new();
        while index < last {
            let mut distance = 1;
            if reader.bits(1).unwrap() == 0 {
                loop {
                    let step = reader.bits(3).unwrap();
                    distance += step;
                    if step != 7 {
                        break;
                    }
                }
            }
            index += distance as i32;
            assert!(index <= last);
            let field = &table["fields"][index as usize];
            let offset = field[0].as_u64().unwrap() as u32;
            let codec = field[1].as_i64().unwrap();
            let previous = base.get(&offset).copied().unwrap_or(0);
            let value = match codec {
                -77 => u32::from(reader.u8().unwrap()),
                -83..=-81 => {
                    assert_eq!(reader.bits(1).unwrap(), 1);
                    f32::from(reader.bits(16).unwrap() as u16 as i16).to_bits()
                }
                -79 => {
                    assert_eq!(reader.bits(2).unwrap(), 0);
                    u32::from(reader.u16().unwrap())
                }
                -98 => {
                    assert_eq!(reader.bits(1).unwrap(), 1);
                    reader.bits(30).unwrap()
                }
                width if width > 0 => {
                    if reader.bits(1).unwrap() == 0 {
                        0
                    } else {
                        let width = width as usize;
                        let remainder = width % 8;
                        let mut value = reader.bits(remainder).unwrap();
                        for shift in (remainder..width).step_by(8) {
                            value |= u32::from(reader.u8().unwrap()) << shift;
                        }
                        value ^ previous
                    }
                }
                _ => panic!("unexpected native missile codec"),
            };
            result.insert(offset, value);
        }
        result
    }
    #[test]
    fn native_missile_fields_survive_every_bit_alignment_and_reused_slots() {
        let first = MissileSnapshot {
            owner: 7,
            angles: [8192, 49152, 1024],
            entity_number: 1648,
            origin: [10.0, -2.0, 4.0],
            weapon: 95,
            present: true,
            generation: 1,
        };
        let second = MissileSnapshot {
            origin: [20.0, -4.0, 8.0],
            weapon: 232,
            generation: 2,
            owner: 11,
            angles: [65535, 32768, 0],
            ..first
        };
        for alignment in 0..8 {
            let mut out = Writer::default();
            out.bits(0x55, alignment);
            write(&mut out, &[first], &[]).unwrap();
            out.bytes.push(0xee);
            let mut reader = Reader::new(&out.bytes);
            reader.bits(alignment).unwrap();
            assert_eq!(reader.bits(2).unwrap(), 2);
            assert_eq!(reader.bits(11).unwrap(), 1648);
            assert_eq!(reader.bits(3).unwrap(), 6);
            let base = fields(&mut reader, &BTreeMap::new());
            assert_eq!(base[&4], 4);
            assert_eq!(base[&0x18], 10f32.to_bits());
            assert_eq!(base[&0x1c], (-2f32).to_bits());
            assert_eq!(base[&0xc], 1);
            assert_eq!(base[&8], 2);
            assert_eq!(base[&0xd0], 95);
            assert_eq!(base[&0x80], 7);
            assert_eq!(base[&0x3c], 8192);
            assert_eq!(base[&0x40], 49152);
            assert_eq!(base[&0x44], 1024);
            assert_eq!(reader.u8().unwrap(), 0xee);
            let mut delta = Writer::default();
            write(&mut delta, &[second], &[first]).unwrap();
            delta.bytes.push(0xee);
            let mut reader = Reader::new(&delta.bytes);
            assert_eq!(reader.bits(2).unwrap(), 2);
            assert_eq!(reader.bits(11).unwrap(), 1648);
            assert_eq!(reader.bits(3).unwrap(), 6);
            let state = fields(&mut reader, &base);
            assert_eq!(state[&0xd0], 232);
            assert_eq!(state[&0x80], 11);
            assert_eq!(state[&0x3c], 65535);
            assert_eq!(state[&0x40], 32768);
            assert_eq!(state[&0x44], 0);
            assert_eq!(state[&8], 0);
            assert_eq!(state[&0x18], 20f32.to_bits());
            assert_eq!(reader.u8().unwrap(), 0xee);
            let mut removed = Writer::default();
            write(&mut removed, &[], &[second]).unwrap();
            removed.bytes.push(0xee);
            let mut reader = Reader::new(&removed.bytes);
            assert_eq!(reader.bits(2).unwrap(), 2);
            assert_eq!(reader.bits(11).unwrap(), 1648);
            assert_eq!(reader.bits(1).unwrap(), 1);
            assert_eq!(reader.u8().unwrap(), 0xee);
        }
    }
}
