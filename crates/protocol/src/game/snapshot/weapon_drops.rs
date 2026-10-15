//! Native ITEM3 ground guns and playerState cursor hints for 3.3.3.3.
use crate::game::gamestate::Writer;
use crate::lobby::channel::LobbyError;

pub const FIRST_ENTITY: u16 = 1024;
pub const END_ENTITY: u16 = 1088;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponDropSnapshot {
    pub entity_number: u16,
    pub weapon: u16,
    pub origin: [f32; 3],
    pub angles: [u16; 3],
    pub owner: u16,
    pub generation: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeaponHint {
    pub weapon: u16,
    pub entity: u16,
}
impl WeaponHint {
    pub(crate) fn valid(self) -> bool {
        (1..900).contains(&self.weapon) && (FIRST_ENTITY..END_ENTITY).contains(&self.entity)
    }
}

pub(crate) fn write(
    out: &mut Writer,
    states: &[WeaponDropSnapshot],
    bases: &[WeaponDropSnapshot],
) -> Result<(), LobbyError> {
    for list in [states, bases] {
        if list.iter().any(|s| {
            !(FIRST_ENTITY..END_ENTITY).contains(&s.entity_number)
                || !(1..900).contains(&s.weapon)
                || !(s.owner < 12 || s.owner == 2047)
                || s.origin
                    .iter()
                    .any(|v| !v.is_finite() || !(-32768.0..=32767.0).contains(v))
        }) || list
            .windows(2)
            .any(|w| w[0].entity_number >= w[1].entity_number)
        {
            return Err(LobbyError("invalid ground weapon entities"));
        }
    }
    let numbers: std::collections::BTreeSet<_> = states
        .iter()
        .chain(bases)
        .map(|s| s.entity_number)
        .collect();
    for number in numbers {
        let current = states.iter().find(|s| s.entity_number == number);
        let base = bases.iter().find(|s| s.entity_number == number);
        if current == base {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(u32::from(number), 11);
        let Some(s) = current else {
            out.bits(1, 1);
            continue;
        };
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(1, 1);
        out.bits(30, 6);
        let mut previous = -1;
        for index in [0, 4, 5, 6, 9, 16, 17, 18, 19, 23, 28, 30] {
            gap(out, (index - previous) as u32);
            previous = index;
            match index {
                0 => out.bytes.push(3),
                4..=6 => {
                    out.bits(1, 1);
                    out.bits(
                        u32::from(s.origin[6 - index as usize].round_ties_even() as i16 as u16),
                        16,
                    );
                }
                9 => scalar(out, 1, u32::from(base.is_some()), 8), // INTERPOLATE
                16..=18 => {
                    // ITEM's -78 codec reads an absolute raw float at 0x5a313d.
                    let degrees = f32::from(s.angles[(index - 16) as usize]) * (360.0 / 65536.0);
                    out.bytes.extend_from_slice(&degrees.to_le_bytes());
                }
                19 => scalar(
                    out,
                    u32::from(s.weapon),
                    base.map_or(0, |b| u32::from(b.weapon)),
                    11,
                ),
                23 => {
                    out.bits(1, 1);
                    out.bits((s.generation & 1) * 2, 30);
                }
                28 => {
                    out.bits(1, 1);
                    out.bits(1, 1);
                    out.bytes.extend_from_slice(
                        &(1.0f32.to_bits() ^ if base.is_some() { 1.0f32.to_bits() } else { 0 })
                            .to_le_bytes(),
                    );
                }
                30 => scalar(
                    out,
                    u32::from(s.owner),
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
    let delta = value ^ base;
    let rem = width % 8;
    out.bits(delta, rem);
    for shift in (rem..width).step_by(8) {
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
    fn state(s: WeaponDropSnapshot) -> BTreeMap<u32, u32> {
        [
            (4, 3),
            (8, (s.generation & 1) * 2),
            (0xc, 1),
            (
                0x18,
                (s.origin[0].round_ties_even() as i16 as f32).to_bits(),
            ),
            (
                0x1c,
                (s.origin[1].round_ties_even() as i16 as f32).to_bits(),
            ),
            (
                0x20,
                (s.origin[2].round_ties_even() as i16 as f32).to_bits(),
            ),
            (0x48, (f32::from(s.angles[0]) * (360.0 / 65536.0)).to_bits()),
            (0x4c, (f32::from(s.angles[1]) * (360.0 / 65536.0)).to_bits()),
            (0x50, (f32::from(s.angles[2]) * (360.0 / 65536.0)).to_bits()),
            (0x98, u32::from(s.weapon)),
            (0x54, 1f32.to_bits()),
            (0x80, u32::from(s.owner)),
        ]
        .into()
    }
    fn round_trip(align: usize, current: &[WeaponDropSnapshot], base: &[WeaponDropSnapshot]) {
        let table: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/scavenger-3.3.3.3.json"
        )))
        .unwrap();
        let mut out = Writer::default();
        out.bits(0x55, align);
        write(&mut out, current, base).unwrap();
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(2047, 11);
        out.bytes.push(0xa9);
        let mut r = Reader::new(&out.bytes);
        assert_eq!(r.bits(align).unwrap(), 0x55 & ((1 << align) - 1));
        let mut world: BTreeMap<_, _> = base.iter().map(|s| (s.entity_number, state(*s))).collect();
        loop {
            assert_eq!(r.bits(2).unwrap(), 2);
            let number = r.bits(11).unwrap() as u16;
            if number == 2047 {
                break;
            }
            if r.bits(1).unwrap() == 1 {
                world.remove(&number);
                continue;
            }
            assert_eq!(r.bits(2).unwrap(), 3);
            let last = r.bits(6).unwrap() as i32;
            let mut index = -1;
            let target = world.entry(number).or_default();
            while index < last {
                let mut distance = 1;
                if r.bits(1).unwrap() == 0 {
                    loop {
                        let step = r.bits(3).unwrap();
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
                let old = target.get(&offset).copied().unwrap_or(0);
                let value = match field[1].as_i64().unwrap() {
                    -77 => u32::from(r.u8().unwrap()),
                    -98 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        r.bits(30).unwrap()
                    }
                    -83..=-81 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        (r.bits(16).unwrap() as u16 as i16 as f32).to_bits()
                    }
                    -78 => {
                        // Native 0x5a27f0 dispatches -78 directly to ReadLong;
                        // no bit gates or baseline XOR precede the float.
                        r.u32().unwrap()
                    }
                    0 => {
                        assert_eq!(r.bits(2).unwrap(), 3);
                        r.u32().unwrap() ^ old
                    }
                    n if n > 0 => {
                        if r.bits(1).unwrap() == 0 {
                            0
                        } else {
                            let n = n as usize;
                            let rem = n % 8;
                            let mut delta = r.bits(rem).unwrap();
                            for bit in (rem..n).step_by(8) {
                                delta |= u32::from(r.u8().unwrap()) << bit;
                            }
                            delta ^ old
                        }
                    }
                    n => panic!("unexpected ITEM codec {n}"),
                };
                target.insert(offset, value);
            }
        }
        assert_eq!(
            world,
            current
                .iter()
                .map(|s| (s.entity_number, state(*s)))
                .collect()
        );
        assert_eq!(r.u8().unwrap(), 0xa9);
    }
    #[test]
    fn native_ground_weapon_fields_round_trip_motion_exchange_reuse_owner_clear_and_delete() {
        let first = WeaponDropSnapshot {
            entity_number: FIRST_ENTITY,
            weapon: 352,
            origin: [10.5, -2.5, 32.0],
            angles: [32768, 40960, 16384],
            owner: 1,
            generation: 1,
        };
        let other = WeaponDropSnapshot {
            entity_number: FIRST_ENTITY + 1,
            ..first
        };
        let exchanged = WeaponDropSnapshot {
            weapon: 354,
            origin: [21.0, -5.0, 1.0],
            angles: [1024, 16384, 65535],
            owner: 0,
            generation: 2,
            ..first
        };
        let released = WeaponDropSnapshot {
            owner: 2047,
            ..exchanged
        };
        for align in 0..8 {
            round_trip(align, &[first, other], &[]);
            round_trip(align, &[first, other], &[first, other]);
            round_trip(align, &[exchanged], &[first, other]);
            round_trip(align, &[released], &[exchanged]);
            round_trip(align, &[], &[released]);
        }
    }
}
