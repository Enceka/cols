//! Target type19 actor fields (0xc53cc8). Animal tree ownership is separate
//! from script models and vehicles; animations use the native command list.
use super::{events, props::PropSnapshot};
use crate::game::gamestate::Writer;

pub(super) fn fields(out: &mut Writer, now: PropSnapshot, base: Option<PropSnapshot>) {
    let actor = now.actor.expect("validated actor pose");
    out.bits(28, 6);
    let mut previous = -1;
    for index in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13, 16, 17, 19, 20, 27, 28] {
        gap(out, (index - previous) as u32);
        previous = index;
        match index {
            0 => out.bytes.push(19),
            1 | 19 | 20 => {
                let axis = match index {
                    1 => 1,
                    19 => 0,
                    _ => 2,
                };
                out.bits(0, 2);
                out.bytes.extend_from_slice(&now.angles[axis].to_le_bytes());
            }
            2..=4 => {
                out.bits(1, 1);
                out.bits(
                    u32::from(now.origin[(index - 2) as usize].round_ties_even() as i16 as u16),
                    16,
                );
            }
            5 => events::write_sequence(out, now.events.sequence),
            6..=9 => events::write_event(out, now.events.slots[(index - 6) as usize]),
            // Target0x50d179 reads the base model from ES+0x5a, sharing
            // this field's upper word with slot and species selector.
            11 => scalar(
                out,
                u32::from(actor.slot)
                    | (u32::from(actor.selector) << 8)
                    | (u32::from(now.model) << 16),
                base.and_then(|p| p.actor.map(|a| (p, a)))
                    .map_or(0, |(p, a)| {
                        u32::from(a.slot)
                            | (u32::from(a.selector) << 8)
                            | (u32::from(p.model) << 16)
                    }),
                32,
            ),
            13 => {
                out.bits(1, 1);
                out.bits(0, 30);
            }
            16 => scalar(
                out,
                u32::from(now.weapon),
                base.map_or(0, |p| u32::from(p.weapon)),
                32,
            ),
            17 => scalar(out, 1, u32::from(base.is_some()), 8),
            27 => scalar(
                out,
                u32::from(now.model),
                base.map_or(0, |p| u32::from(p.model)),
                10,
            ),
            28 => {
                out.bits(3, 2);
                out.bytes.extend_from_slice(
                    &(1f32.to_bits() ^ base.map_or(0, |_| 1f32.to_bits())).to_le_bytes(),
                );
            }
            _ => unreachable!(),
        }
    }
}
/// Type19 also needs the actor client section. Its renderer refuses an empty
/// CG actor-info slot even when the entity model and animation are present.
/// Target0x46cc60 reads descriptor0xc54298 against default0xc57218.
pub(super) fn actors(out: &mut Writer, props: &[PropSnapshot], bases: &[PropSnapshot]) {
    let current = props
        .iter()
        .filter(|p| p.present && p.actor.is_some())
        .map(|p| (p.actor.unwrap().slot, *p))
        .collect::<std::collections::BTreeMap<_, _>>();
    let previous = bases
        .iter()
        .filter(|p| p.present && p.actor.is_some())
        .map(|p| (p.actor.unwrap().slot, *p))
        .collect::<std::collections::BTreeMap<_, _>>();
    let slots = current
        .keys()
        .chain(previous.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for slot in slots {
        out.bits(1, 1);
        out.bits(0, 1); // absolute actor slot, independent of entity numbering
        out.bits(u32::from(slot), 5);
        let Some(p) = current.get(&slot) else {
            out.bits(1, 1); // explicitly remove an acknowledged old actor
            continue;
        };
        out.bits(0, 1); // retained
        out.bits(1, 1); // fields changed
        out.bits(23, 5);
        let base = previous.get(&slot);
        let mut last = -1;
        for index in std::iter::once(8).chain(12..=23) {
            gap(out, (index - last) as u32);
            last = index;
            let (value, old, width) = match index {
                8 => (
                    u32::from(p.model),
                    base.map_or(0x4f2, |b| u32::from(b.model)),
                    10,
                ),
                // Same single-body flags as the target's entity-owned actor.
                12 => (1, if base.is_some() { 1 } else { 3 }, 32),
                13..=19 | 23 => (
                    0,
                    0,
                    if matches!(index, 15 | 17 | 19 | 23) {
                        6
                    } else {
                        10
                    },
                ),
                20 => (
                    u32::from(p.entity_number),
                    base.map_or(345, |b| u32::from(b.entity_number)),
                    11,
                ),
                21 => (
                    u32::from(p.actor.unwrap().team),
                    base.map_or(2, |b| u32::from(b.actor.unwrap().team)),
                    3,
                ),
                // No held human weapon or attachment tag on the animal body.
                22 => (0, 0, 11),
                _ => unreachable!(),
            };
            scalar(out, value, old, width);
        }
    }
    out.bits(0, 1);
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
    use crate::game::{channel::wire::Reader, snapshot::props::ActorPose};
    use std::collections::BTreeMap;
    #[test]
    fn target_descriptor_decodes_full_and_delta_actor_model_slot_species_and_pose() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/animal-fields-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(fixture["client_sha256"], crate::CLIENT_SHA256);
        let mut base = None;
        let mut decoded = BTreeMap::<u64, u32>::new();
        for model in [105, 106, 105] {
            let p = PropSnapshot {
                entity_number: 656,
                model,
                origin: [123.0, -400.0, 0.0],
                angles: [1000, 2000, 3000],
                present: true,
                weapon: 305,
                events: Default::default(),
                turret: false,
                turret_pose: None,
                vehicle: None,
                actor: Some(ActorPose {
                    team: 1,
                    slot: 0,
                    selector: 1,
                    animation: 49,
                    animation_parent: 32,
                    animation_ancestors: [31, 32, 0, 0],
                }),
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
                let f = &fixture["fields"][index as usize];
                let offset = f[0].as_u64().unwrap();
                let codec = f[1].as_i64().unwrap();
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
                    -98 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        r.bits(30).unwrap()
                    }
                    0 => {
                        assert_eq!(r.bits(2).unwrap(), 3);
                        r.u32().unwrap() ^ decoded.get(&offset).copied().unwrap_or(0)
                    }
                    n if n > 0 => {
                        if r.bits(1).unwrap() == 0 {
                            0
                        } else {
                            let width = n as usize;
                            let rem = width % 8;
                            let mut v = r.bits(rem).unwrap();
                            for shift in (rem..width).step_by(8) {
                                v |= u32::from(r.u8().unwrap()) << shift;
                            }
                            v ^ decoded.get(&offset).copied().unwrap_or(0)
                        }
                    }
                    _ => panic!("unexpected target actor codec {codec}"),
                };
                decoded.insert(offset, value);
            }
            assert_eq!(decoded[&4], 19);
            assert_eq!(decoded[&0x58], 256 | (u32::from(model) << 16));
            assert_eq!(decoded[&0x98], u32::from(model));
            assert_eq!(decoded[&0xd0], 305);
            assert_eq!(decoded[&0x18], 123);
            assert_eq!(decoded[&0x1c], u32::from(-400i16 as u16));
            assert_eq!(decoded[&0x40], 2000);
            assert_eq!(decoded[&0x54], 1f32.to_bits());
            base = Some(p);
        }
    }
    #[test]
    fn actor_clients_decode_from_target_defaults_and_clear_removed_slots() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/animal-fields-3.3.3.3.json"
        )))
        .unwrap();
        let descriptor = &fixture["actor_client"];
        let defaults = descriptor["default_words"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, v)| ((i * 4) as u64, v.as_u64().unwrap() as u32))
            .collect::<BTreeMap<_, _>>();
        let animal = |slot, team, model| PropSnapshot {
            entity_number: 656 + u16::from(slot),
            model,
            origin: [0.0; 3],
            angles: [0; 3],
            present: true,
            weapon: 305,
            events: Default::default(),
            turret: false,
            turret_pose: None,
            vehicle: None,
            actor: Some(ActorPose {
                slot,
                team,
                selector: 1,
                animation: 39,
                animation_parent: 26,
                animation_ancestors: [26, 0, 0, 0],
            }),
        };
        let mut previous = Vec::new();
        let mut decoded = BTreeMap::<u32, BTreeMap<u64, u32>>::new();
        for current in [
            vec![animal(0, 1, 105), animal(3, 2, 105)],
            vec![animal(0, 2, 106), animal(3, 2, 105)],
            vec![animal(3, 2, 105)],
            Vec::new(),
            vec![animal(0, 1, 105)],
        ] {
            let mut out = Writer::default();
            actors(&mut out, &current, &previous);
            out.bytes.push(0xa7);
            let mut r = Reader::new(&out.bytes);
            while r.bits(1).unwrap() != 0 {
                assert_eq!(r.bits(1).unwrap(), 0);
                let slot = r.bits(5).unwrap();
                if r.bits(1).unwrap() != 0 {
                    decoded.remove(&slot);
                    continue;
                }
                assert_eq!(r.bits(1).unwrap(), 1);
                let last = r.bits(5).unwrap() as i32;
                let mut index = -1;
                let old = decoded.get(&slot).unwrap_or(&defaults).clone();
                let mut fields = old.clone();
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
                    let f = &descriptor["fields"][index as usize];
                    let offset = f[0].as_u64().unwrap();
                    let width = f[1].as_u64().unwrap() as usize;
                    let value = if r.bits(1).unwrap() == 0 {
                        0
                    } else {
                        let rem = width % 8;
                        let mut bits = r.bits(rem).unwrap();
                        for shift in (rem..width).step_by(8) {
                            bits |= u32::from(r.u8().unwrap()) << shift;
                        }
                        // Target0x5a2544 masks the default to this field width.
                        let mask = u32::MAX >> (32 - width);
                        bits ^ (old[&offset] & mask)
                    };
                    fields.insert(offset, value);
                }
                decoded.insert(slot, fields);
            }
            assert_eq!(r.u8().unwrap(), 0xa7);
            r.finish().unwrap();
            assert_eq!(decoded.len(), current.len());
            for p in &current {
                let a = p.actor.unwrap();
                let fields = &decoded[&u32::from(a.slot)];
                assert_eq!(fields[&4], u32::from(p.entity_number));
                assert_eq!(fields[&8], u32::from(a.team));
                assert_eq!(fields[&12], u32::from(p.model));
                assert_eq!(fields[&16], 0);
                assert_eq!(fields[&52], 1);
                for offset in [20, 24, 28, 32, 36, 40, 44, 48] {
                    assert_eq!(fields[&offset], 0);
                }
            }
            previous = current;
        }
    }
}
