//! Map script-model props in the measured GENERAL entity layout.
use crate::{game_state::Writer, lobby::LobbyError};
use std::collections::BTreeSet;

pub const FIRST_ENTITY: u16 = 256;
pub const END_ENTITY: u16 = 1024;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropSnapshot {
    pub entity_number: u16,
    pub model: u16,
    pub origin: [f32; 3],
    pub angles: [u16; 3],
    pub present: bool,
}
impl PropSnapshot {
    fn valid(self) -> bool {
        (FIRST_ENTITY..END_ENTITY).contains(&self.entity_number)
            && (1..1024).contains(&self.model)
            && self
                .origin
                .iter()
                .all(|n| n.is_finite() && (-32768.0..=32767.0).contains(n))
    }
}
pub(crate) fn write(
    out: &mut Writer,
    current: &[PropSnapshot],
    baseline: &[PropSnapshot],
) -> Result<(), LobbyError> {
    for list in [current, baseline] {
        if list.len() > 128
            || list.iter().any(|p| !p.valid())
            || list
                .windows(2)
                .any(|p| p[0].entity_number >= p[1].entity_number)
        {
            return Err(LobbyError("invalid map prop snapshots"));
        }
    }
    let numbers: BTreeSet<_> = current
        .iter()
        .chain(baseline)
        .map(|p| p.entity_number)
        .collect();
    for number in numbers {
        let now = current
            .iter()
            .find(|p| p.entity_number == number)
            .filter(|p| p.present);
        let old = baseline
            .iter()
            .find(|p| p.entity_number == number)
            .filter(|p| p.present);
        if now == old {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(u32::from(number), 11);
        out.bits(u32::from(now.is_none()), 1);
        let Some(prop) = now else {
            continue;
        };
        out.bits(1, 1);
        out.bits(1, 1); // changed, absolute position/angle bases
        out.bits(32, 6); // last changed GENERAL field, including roll
        gap(out, 1);
        out.bytes.push(0); // GENERAL eType
        let mut previous = 0;
        for (field, axis) in [(2, 0), (3, 1), (4, 2)] {
            gap(out, field - previous);
            previous = field;
            out.bits(1, 1);
            out.bits(
                u32::from(prop.origin[axis].round_ties_even() as i16 as u16),
                16,
            );
        }
        if old.is_none() {
            gap(out, 9 - previous);
            out.bits(1, 1);
            out.bits(1, 1);
            out.bytes.extend_from_slice(&1f32.to_le_bytes());
            previous = 9;
        }
        gap(out, 13 - previous);
        let model = u32::from(prop.model ^ old.map_or(0, |p| p.model));
        out.bits(1, 1);
        out.bits(model, 2);
        out.bytes.push((model >> 2) as u8);
        previous = 13;
        for (field, axis) in [(18, 1), (19, 0), (32, 2)] {
            gap(out, field - previous);
            previous = field;
            out.bits(0, 2); // native -79 absolute short angle
            out.bytes
                .extend_from_slice(&prop.angles[axis].to_le_bytes());
        }
    }
    Ok(())
}
fn gap(out: &mut Writer, distance: u32) {
    out.bits(u32::from(distance == 1), 1);
    if distance == 1 {
        return;
    }
    let mut n = distance - 1;
    while n >= 7 {
        out.bits(7, 3);
        n -= 7;
    }
    out.bits(n, 3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_wire::Reader;
    fn decode(
        body: &[u8],
        baseline: Option<PropSnapshot>,
    ) -> (u16, Option<std::collections::BTreeMap<usize, u32>>) {
        let f: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/entity-state-fields-3.3.3.3.json"
        ))
        .unwrap();
        let mut r = Reader::new(body);
        assert_eq!(r.bits(2).unwrap(), 2);
        let number = r.bits(11).unwrap() as u16;
        if r.bits(1).unwrap() != 0 {
            return (number, None);
        }
        assert_eq!(r.bits(2).unwrap(), 3);
        let last = r.bits(6).unwrap() as i32;
        let mut index = -1i32;
        let mut fields = std::collections::BTreeMap::new();
        if let Some(p) = baseline {
            fields.insert(0x98, u32::from(p.model));
            fields.insert(0x54, 1f32.to_bits());
        }
        while index < last {
            let mut distance = 1;
            if r.bits(1).unwrap() == 0 {
                loop {
                    let n = r.bits(3).unwrap();
                    distance += n;
                    if n < 7 {
                        break;
                    }
                }
            }
            index += distance as i32;
            let field = &f["fields"][index as usize];
            let codec = field[1].as_i64().unwrap();
            let value = match codec {
                -77 => u32::from(r.u8().unwrap()),
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
                    r.u32().unwrap()
                }
                width if width > 0 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    let width = width as usize;
                    let low = width % 8;
                    let mut value = r.bits(low).unwrap();
                    for bit in (low..width).step_by(8) {
                        value |= u32::from(r.u8().unwrap()) << bit;
                    }
                    value
                        ^ fields
                            .get(&(field[0].as_u64().unwrap() as usize))
                            .copied()
                            .unwrap_or(0)
                }
                _ => panic!("unexpected general codec {codec}"),
            };
            fields.insert(field[0].as_u64().unwrap() as usize, value);
        }
        (number, Some(fields))
    }
    #[test]
    fn general_model_and_all_pose_axes_decode_and_changed_wreck_or_removal_do_not_inherit_an_old_model()
     {
        let first = PropSnapshot {
            entity_number: 256,
            model: 192,
            origin: [123.0, -456.0, 7.0],
            angles: [100, 32768, 49152],
            present: true,
        };
        for prop in [
            first,
            PropSnapshot {
                model: 191,
                ..first
            },
        ] {
            let mut out = Writer::default();
            write(
                &mut out,
                &[prop],
                &[first]
                    .into_iter()
                    .filter(|p| *p != prop)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            let (id, fields) = decode(&out.bytes, (prop != first).then_some(first));
            let fields = fields.unwrap();
            assert_eq!(id, 256);
            assert_eq!(fields[&0x98], u32::from(prop.model));
            for (offset, value) in [
                (0x18, 123),
                (0x1c, (-456i16) as u16 as u32),
                (0x20, 7),
                (0x3c, 100),
                (0x40, 32768),
                (0x44, 49152),
            ] {
                assert_eq!(fields[&offset], value);
            }
        }
        let mut same = Writer::default();
        write(&mut same, &[first], &[first]).unwrap();
        assert!(same.bytes.is_empty());
        let mut gone = Writer::default();
        write(
            &mut gone,
            &[PropSnapshot {
                present: false,
                ..first
            }],
            &[first],
        )
        .unwrap();
        assert_eq!(decode(&gone.bytes, Some(first)), (256, None));
        let mut invalid = Writer::default();
        assert!(
            write(
                &mut invalid,
                &[PropSnapshot {
                    entity_number: 54,
                    ..first
                }],
                &[]
            )
            .is_err()
        );
    }
}
