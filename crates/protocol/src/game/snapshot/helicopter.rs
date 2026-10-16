//! Target type13 vehicle descriptor, renderer0x684440, pose0x683cb0.
//! Type12 is a turret: a helicopter model alone does not connect its rotors.
use super::{events, props::PropSnapshot};
use crate::game::gamestate::Writer;

pub(super) fn fields(out: &mut Writer, now: PropSnapshot, base: Option<PropSnapshot>) {
    let vehicle = now.vehicle.expect("validated vehicle pose");
    let old = base.and_then(|p| p.vehicle);
    out.bits(38, 6);
    let mut previous = -1;
    for field in [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 13, 14, 15, 16, 18, 21, 22, 24, 27, 36, 37, 38,
    ] {
        gap(out, (field - previous) as u32);
        previous = field;
        match field {
            0 => out.bytes.push(13),
            // -61/-60/-59 carry XOR float bits (0x5a27f0). Unlike the
            // integer-origin codecs, their payload still depends on the
            // acknowledged entity baseline's decoded position.
            1 | 5 | 7 => {
                let axis = match field {
                    1 => 0,
                    5 => 1,
                    _ => 2,
                };
                out.bytes.extend_from_slice(
                    &(now.origin[axis].to_bits() ^ base.map_or(0, |p| p.origin[axis].to_bits()))
                        .to_le_bytes(),
                );
            }
            2 | 3 | 6 => {
                let axis = match field {
                    2 => 0,
                    3 => 2,
                    _ => 1,
                };
                out.bits(0, 2);
                out.bytes.extend_from_slice(&now.angles[axis].to_le_bytes());
            }
            4 | 8 => {
                let axis = usize::from(field == 8);
                float(out, vehicle.gun[axis], old.map_or(0.0, |v| v.gun[axis]));
            }
            9 => events::write_sequence(out, now.events.sequence),
            13..=16 => events::write_event(out, now.events.slots[(field - 13) as usize]),
            18 => scalar(
                out,
                u32::from(vehicle.loop_sound),
                old.map_or(0, |v| u32::from(v.loop_sound)),
                8,
            ),
            21 => scalar(
                out,
                u32::from(now.model),
                base.map_or(0, |p| u32::from(p.model)),
                32,
            ),
            22 => scalar(
                out,
                u32::from(now.weapon),
                base.map_or(0, |p| u32::from(p.weapon)),
                32,
            ),
            24 => scalar(out, packed(vehicle), old.map_or(0, packed), 9),
            27 => out.bits(4, 7), // engine running; native sound consumer0x683354
            36 => float(out, 1.0, base.map_or(0.0, |_| 1.0)),
            37 => scalar(
                out,
                u32::from(vehicle.tree),
                old.map_or(0, |v| u32::from(v.tree)),
                4,
            ),
            38 => scalar(out, 2047, base.map_or(0, |_| 2047), 11),
            _ => unreachable!(),
        }
    }
}
fn packed(v: super::props::VehiclePose) -> u32 {
    u32::from(v.slot) | (u32::from(v.definition) << 4)
}
fn float(out: &mut Writer, value: f32, base: f32) {
    if value == 0.0 {
        out.bits(0, 2);
    } else {
        out.bits(3, 2);
        out.bytes
            .extend_from_slice(&(value.to_bits() ^ base.to_bits()).to_le_bytes());
    }
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
    let mut n = distance - 1;
    while n >= 7 {
        out.bits(7, 3);
        n -= 7;
    }
    out.bits(n, 3);
}

/// Snapshot animation commands, target0x46d060/0x5a3c90. Reassert an
/// idempotent goal (restart flag0), so packet loss and late views recover.
pub(super) fn animations(out: &mut Writer, props: &[PropSnapshot], time: u32) {
    let active = props
        .iter()
        .filter_map(|p| {
            if !p.present {
                return None;
            }
            p.vehicle
                .filter(|v| v.animation != 0)
                .map(|v| (p, [v.animation_parent, 0, 0, 0], v.animation))
                .or_else(|| p.actor.map(|a| (p, a.animation_ancestors, a.animation)))
        })
        .collect::<Vec<_>>();
    if active.is_empty() {
        out.bits(0, 1);
        return;
    }
    let nodes = active
        .iter()
        .flat_map(|(_, parents, node)| parents.iter().copied().chain([*node]))
        .filter(|&node| node != 0)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    out.bits(1, 1);
    out.bits(1, 1); // section, replace dictionary
    out.bytes.push(nodes.len() as u8);
    for node in &nodes {
        out.bits(u32::from(*node), 11);
    }
    let width = if nodes.len() <= 1 {
        0
    } else {
        usize::BITS as usize - (nodes.len() - 1).leading_zeros() as usize
    };
    for (prop, parents, animation) in active {
        for node in parents.into_iter().chain([animation]).filter(|&n| n != 0) {
            out.bits(0, 1); // new command, not an earlier snapshot's command
            opcode(out, 3);
            out.bits(u32::from(prop.entity_number), 11);
            opcode(out, 4);
            out.bits(1, 1);
            out.bytes.extend_from_slice(&time.to_le_bytes());
            // The target's opcode2 blends siblings (0x64bed0), but does not
            // activate the group above the leaf. Set the captured parent first.
            opcode(out, 2);
            out.bits(0, 2); // no restart, no notify
            let index = nodes.binary_search(&node).expect("animation dictionary");
            out.bits(index as u32, width);
            out.bits(0, 1);
            out.bits(1, 1); // goal1 (0x5a3ad0)
            out.bits(0, 1); // default0.2-second blend (0x5a39d0)
            out.bits(0, 1); // rate1 (0x5a3a50)
        }
    }
    out.bits(0, 1);
    opcode(out, 5); // end of list
}
fn opcode(out: &mut Writer, value: u32) {
    if value <= 4 {
        out.bits(1, 1);
        out.bits(value - 1, 2);
    } else {
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(value - 5, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::super::props::VehiclePose;
    use super::*;
    use crate::game::channel::wire::Reader;
    #[test]
    fn aircraft_slots_require_vehicle_bindings_and_reject_aliasing_or_missing_guns() {
        let mut p = PropSnapshot {
            actor: None,
            entity_number: 560,
            model: 146,
            weapon: 302,
            origin: [0.0; 3],
            angles: [0; 3],
            present: true,
            turret: false,
            turret_pose: None,
            events: Default::default(),
            vehicle: None,
        };
        let mut out = Writer::default();
        assert!(super::super::props::write(&mut out, &[p], &[]).is_err());
        assert!(out.bytes.is_empty());
        p.vehicle = Some(VehiclePose {
            definition: 3,
            slot: 0,
            tree: 3,
            animation: 11,
            animation_parent: 10,
            loop_sound: 0,
            animation_time: 1000,
            gun: [0.0; 2],
        });
        assert!(super::super::props::write(&mut out, &[p], &[]).is_ok());
        p.vehicle.as_mut().unwrap().slot = 1;
        assert!(super::super::props::write(&mut Writer::default(), &[p], &[]).is_err());
        p.vehicle.as_mut().unwrap().slot = 0;
        p.weapon = 0;
        assert!(super::super::props::write(&mut Writer::default(), &[p], &[]).is_err());
    }
    #[test]
    fn rotor_goal_uses_target_animation_dictionary_and_command_stream() {
        let prop = PropSnapshot {
            actor: None,
            entity_number: 560,
            model: 146,
            weapon: 302,
            origin: [0.0; 3],
            angles: [0; 3],
            present: true,
            turret: false,
            turret_pose: None,
            events: Default::default(),
            vehicle: Some(VehiclePose {
                definition: 3,
                slot: 0,
                tree: 3,
                animation: 11,
                animation_parent: 10,
                loop_sound: 0,
                animation_time: 1000,
                gun: [0.0; 2],
            }),
        };
        let mut out = Writer::default();
        animations(&mut out, &[prop], 2000);
        let mut r = Reader::new(&out.bytes);
        assert_eq!(r.bits(2).unwrap(), 3);
        assert_eq!(r.u8().unwrap(), 2);
        assert_eq!(r.bits(11).unwrap(), 10);
        assert_eq!(r.bits(11).unwrap(), 11);
        for node in [0, 1] {
            assert_eq!(r.bits(1).unwrap(), 0); // new command
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(2).unwrap(), 2); // set entity
            assert_eq!(r.bits(11).unwrap(), 560);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(2).unwrap(), 3); // set clock
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.u32().unwrap(), 2000);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(2).unwrap(), 1); // target goal opcode2
            assert_eq!(r.bits(2).unwrap(), 0); // no restart/notify
            assert_eq!(r.bits(1).unwrap(), node); // parent before leaf
            assert_eq!(r.bits(2).unwrap(), 2); // goal1
            assert_eq!(r.bits(2).unwrap(), 0); // blend0.2, rate1
        }
        assert_eq!(r.bits(1).unwrap(), 0);
        assert_eq!(r.bits(2).unwrap(), 2);
        assert_eq!(r.bits(2).unwrap(), 0); // end opcode5
        let mut absent = prop;
        absent.present = false;
        let mut out = Writer::default();
        animations(&mut out, &[absent], 2050);
        assert_eq!(out.bytes, vec![0]);
    }
    #[test]
    fn native_vehicle_descriptor_decodes_model_origin_gun_and_registration_deltas() {
        let table: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/helicopter-fields-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(table["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(table["entity_type"], 13);
        assert_eq!(table["model_centity_offset"], 0x1d0);
        let mut base = None;
        let mut decoded = std::collections::BTreeMap::new();
        for step in 0..3 {
            let p = PropSnapshot {
                actor: None,
                entity_number: 560,
                model: if step < 2 { 146 } else { 148 },
                weapon: if step < 2 { 302 } else { 335 },
                origin: [100.25 + step as f32 * 30.0, -400.75, 1500.5],
                angles: [0, 16384, 0],
                present: true,
                turret: false,
                turret_pose: None,
                vehicle: Some(VehiclePose {
                    definition: if step < 2 { 3 } else { 6 },
                    slot: 0,
                    tree: 3,
                    gun: [25.5 * step as f32, -45.0],
                    animation: 11,
                    animation_parent: 10,
                    loop_sound: if step == 1 { 0 } else { 240 },
                    animation_time: 1000,
                }),
                events: Default::default(),
            };
            let mut out = Writer::default();
            fields(&mut out, p, base);
            let mut r = Reader::new(&out.bytes);
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
                    -61..=-59 => r.u32().unwrap() ^ decoded.get(&offset).copied().unwrap_or(0),
                    -79 => {
                        assert_eq!(r.bits(2).unwrap(), 0);
                        u32::from(r.u16().unwrap())
                    }
                    -75 => {
                        assert_eq!(r.bits(1).unwrap(), 0);
                        u32::from(r.u8().unwrap())
                    }
                    -94 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        let id = r.u8().unwrap();
                        assert_eq!(r.bits(1).unwrap(), 0);
                        u32::from(id)
                    }
                    -84 => r.bits(7).unwrap(),
                    0 => match r.bits(2).unwrap() {
                        0 => 0,
                        3 => r.u32().unwrap() ^ decoded.get(&offset).copied().unwrap_or(0),
                        _ => panic!("unexpected float codec"),
                    },
                    n if n > 0 => {
                        if r.bits(1).unwrap() == 0 {
                            0
                        } else {
                            let bits = n as usize;
                            let rem = bits % 8;
                            let mut value = r.bits(rem).unwrap();
                            for shift in (rem..bits).step_by(8) {
                                value |= u32::from(r.u8().unwrap()) << shift;
                            }
                            value ^ decoded.get(&offset).copied().unwrap_or(0)
                        }
                    }
                    _ => panic!("unhandled codec {codec}"),
                };
                decoded.insert(offset, value);
            }
            assert_eq!(decoded[&4], 13);
            assert_eq!(decoded[&0xe0], u32::from(p.model));
            assert_eq!(decoded[&0xd0], u32::from(p.weapon));
            assert_eq!(decoded[&0x58], packed(p.vehicle.unwrap()));
            assert_eq!(decoded[&0x8c], u32::from(p.vehicle.unwrap().loop_sound));
            assert_eq!(decoded[&0x18], p.origin[0].to_bits());
            assert_eq!(decoded[&0x6c], p.vehicle.unwrap().gun[0].to_bits());
            assert_eq!(decoded[&0x70], p.vehicle.unwrap().gun[1].to_bits());
            base = Some(p);
        }
    }
}
