//! Measured target type12, renderer0x501ff0 and pose0x501db0.
use super::{events, props::PropSnapshot};
use crate::game::gamestate::Writer;

pub(super) fn fields(out: &mut Writer, now: PropSnapshot, base: Option<PropSnapshot>) {
    let pose = now.turret_pose.expect("validated turret pose");
    out.bits(27, 6);
    let mut previous = -1;
    for field in [
        0, 1, 2, 3, 5, 6, 7, 8, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24, 25, 27,
    ] {
        gap(out, (field - previous) as u32);
        previous = field;
        match field {
            0 => out.bytes.push(12),
            1 | 2 | 15 => {
                let axis = match field {
                    1 => 1,
                    2 => 0,
                    _ => 2,
                };
                // -100: absolute short converted to floating point degrees.
                out.bits(0, 2);
                out.bytes.extend_from_slice(&pose.gun[axis].to_le_bytes());
            }
            3 => events::write_sequence(out, now.events.sequence),
            5..=8 => events::write_event(out, now.events.slots[(field - 5) as usize]),
            14 => {
                // -98 absolute30-bit eFlags; this is not scalar XOR coding.
                out.bits(1, 1);
                out.bits(if pose.firing { 0x200 } else { 0 }, 30);
            }
            16..=18 => {
                out.bits(1, 1);
                out.bits(
                    u32::from(now.origin[(field - 16) as usize].round_ties_even() as i16 as u16),
                    16,
                );
            }
            19..=21 => {
                let axis = match field {
                    19 => 1,
                    20 => 0,
                    _ => 2,
                };
                out.bits(0, 2);
                out.bytes.extend_from_slice(&now.angles[axis].to_le_bytes());
            }
            22 => scalar(
                out,
                u32::from(now.model),
                base.map_or(0, |p| u32::from(p.model)),
                10,
            ),
            24 => scalar(out, 2047, base.map_or(0, |_| 2047), 11),
            25 => scalar(
                out,
                u32::from(now.weapon),
                base.map_or(0, |p| u32::from(p.weapon)),
                32,
            ),
            27 => {
                // Generic float0: nonzero, full precision, XOR baseline bits.
                out.bits(3, 2);
                out.bytes.extend_from_slice(
                    &(1f32.to_bits() ^ base.map_or(0, |_| 1f32.to_bits())).to_le_bytes(),
                );
            }
            _ => unreachable!(),
        }
    }
}
fn scalar(out: &mut Writer, value: u32, base: u32, width: usize) {
    out.bits(u32::from(value != 0), 1);
    if value != 0 {
        let delta = value ^ base;
        let rem = width % 8;
        out.bits(delta, rem);
        for shift in (rem..width).step_by(8) {
            out.bytes.push((delta >> shift) as u8);
        }
    }
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
    use crate::game::{channel::wire::Reader, snapshot::props::TurretPose};
    #[test]
    fn target_descriptor_decodes_independent_gun_angles_flags_events_and_delta_clear() {
        let table: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/turret-fields-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(table["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(table["entity_type"], 12);
        let mut base = None;
        let mut decoded = std::collections::BTreeMap::<u64, u32>::new();
        for step in 0..3 {
            let mut p = PropSnapshot {
                actor: None,
                entity_number: 576,
                model: 153,
                weapon: 335,
                origin: [102.5 + step as f32 * 50.0, -180.25, 1020.0],
                angles: [1000, 16384, 500],
                present: true,
                turret: false,
                vehicle: None,
                turret_pose: Some(TurretPose {
                    gun: [60000, 17000 + step * 2000, 0],
                    firing: step != 1,
                }),
                events: Default::default(),
            };
            for _ in 0..(step + 1) {
                p.events.push(events::FIRE_WEAPON, 0).unwrap();
            }
            let mut out = Writer::default();
            assert!(
                super::super::props::write(&mut out, &[p], &base.into_iter().collect::<Vec<_>>())
                    .is_ok()
            );
            let mut raw = Writer::default();
            fields(&mut raw, p, base);
            let mut r = Reader::new(&raw.bytes);
            let last = r.bits(6).unwrap() as i32;
            let mut index = -1;
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
                let offset = table["fields"][index as usize][0].as_u64().unwrap();
                let codec = table["fields"][index as usize][1].as_i64().unwrap();
                let value = match codec {
                    -77 => u32::from(r.u8().unwrap()),
                    -100 => {
                        assert_eq!(r.bits(2).unwrap(), 0);
                        (f32::from(r.u16().unwrap() as i16) * (360.0 / 65536.0)).to_bits()
                    }
                    -79 => {
                        assert_eq!(r.bits(2).unwrap(), 0);
                        u32::from(r.u16().unwrap())
                    }
                    -83..=-81 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        (r.bits(16).unwrap() as u16 as i16 as f32).to_bits()
                    }
                    -98 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        r.bits(30).unwrap()
                    }
                    -75 => {
                        assert_eq!(r.bits(1).unwrap(), 0);
                        u32::from(r.u8().unwrap())
                    }
                    -94 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        let event = r.u8().unwrap();
                        assert_eq!(r.bits(1).unwrap(), 0);
                        u32::from(event)
                    }
                    0 => {
                        assert_eq!(r.bits(2).unwrap(), 3);
                        r.u32().unwrap() ^ decoded.get(&offset).copied().unwrap_or(0)
                    }
                    n if n > 0 => {
                        if r.bits(1).unwrap() == 0 {
                            0
                        } else {
                            let rem = n as usize % 8;
                            let mut value = r.bits(rem).unwrap();
                            for shift in (rem..n as usize).step_by(8) {
                                value |= u32::from(r.u8().unwrap()) << shift;
                            }
                            value ^ decoded.get(&offset).copied().unwrap_or(0)
                        }
                    }
                    _ => panic!("unexpected codec {codec}"),
                };
                decoded.insert(offset, value);
            }
            assert_eq!(decoded[&4], 12);
            assert_eq!(decoded[&0x98], 153);
            assert_eq!(decoded[&0xd0], 335);
            assert_eq!(decoded[&0x80], 2047);
            assert_eq!(decoded[&8], if step == 1 { 0 } else { 0x200 });
            assert_eq!(
                decoded[&0x58],
                (f32::from(p.turret_pose.unwrap().gun[0] as i16) * (360.0 / 65536.0)).to_bits()
            );
            assert_eq!(
                decoded[&0x5c],
                (f32::from(p.turret_pose.unwrap().gun[1] as i16) * (360.0 / 65536.0)).to_bits()
            );
            assert_eq!(decoded[&0x1c], (-180f32).to_bits());
            assert_eq!(decoded[&0x54], 1f32.to_bits());
            assert_eq!(decoded[&0xac], u32::from(p.events.sequence));
            base = Some(p);
        }
        let mut p = base.unwrap();
        p.entity_number = 608;
        assert!(super::super::props::write(&mut Writer::default(), &[p], &[]).is_err());
        p.entity_number = 576;
        p.weapon = 0;
        assert!(super::super::props::write(&mut Writer::default(), &[p], &[]).is_err());
    }
}
