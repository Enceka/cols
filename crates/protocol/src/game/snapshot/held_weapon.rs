//! Native SCRIPTMOVER weapon models linked to a character's hand.
//! 0x501550 selects ES+0x68 and 0x501320 builds the composed world parts.
//! 0x5036e0 resolves ES+0xe4: parent+1 low16, dynamic tag slot in bits16..23.
use crate::game::gamestate::Writer;
use crate::game::snapshot::corpse::CorpseSnapshot;
use crate::game::snapshot::enemy::EnemySnapshot;
use crate::lobby::channel::LobbyError;
use std::collections::BTreeMap;

pub const FIRST_ENTITY: u16 = 27;
pub const END_ENTITY: u16 = 54;
pub const HAND_TAG_SLOT: u8 = 1;
pub const HAND_TAG_CONFIGSTRING: u16 = 6802 + HAND_TAG_SLOT as u16;

pub fn entity_for(parent: u16) -> u16 {
    FIRST_ENTITY + parent
}

pub(crate) fn weapons(
    players: impl Iterator<Item = EnemySnapshot>,
    corpses: &[CorpseSnapshot],
) -> BTreeMap<u16, u16> {
    players
        .filter(|p| p.alive)
        .map(|p| (p.entity_number, p.weapon))
        .chain(
            corpses
                .iter()
                .filter(|c| c.present)
                .map(|c| (c.entity_number, c.weapon)),
        )
        .collect()
}

pub(crate) fn write(
    out: &mut Writer,
    current: &BTreeMap<u16, u16>,
    baseline: &BTreeMap<u16, u16>,
    tag: u8,
) -> Result<(), LobbyError> {
    if tag == 0
        || current
            .iter()
            .chain(baseline)
            .any(|(&parent, &weapon)| parent > 26 || !(1..900).contains(&weapon))
    {
        return Err(LobbyError(
            "invalid held weapon parent, tag or registration",
        ));
    }
    let parents: std::collections::BTreeSet<_> =
        current.keys().chain(baseline.keys()).copied().collect();
    for parent in parents {
        let weapon = current.get(&parent).copied();
        let base = baseline.get(&parent).copied();
        if weapon == base {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(u32::from(entity_for(parent)), 11);
        out.bits(u32::from(weapon.is_none()), 1);
        let Some(weapon) = weapon else {
            continue;
        };
        out.bits(1, 1);
        out.bits(1, 1);
        out.bits(45, 6);
        let mut previous = -1;
        for field in [0, 18, 29, 45] {
            gap(out, (field - previous) as u32);
            previous = field;
            match field {
                0 => out.bytes.push(6),
                18 => {
                    let link = u32::from(parent + 1) | (u32::from(tag) << 16);
                    scalar(out, link, if base.is_some() { link } else { 0 }, 28);
                }
                29 => scalar(out, u32::from(weapon), u32::from(base.unwrap_or(0)), 10),
                45 => {
                    out.bits(1, 1);
                    out.bits(1, 1);
                    out.bytes.extend_from_slice(
                        &(1.0f32.to_bits() ^ if base.is_some() { 1.0f32.to_bits() } else { 0 })
                            .to_le_bytes(),
                    );
                }
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
    let r = width % 8;
    out.bits(delta, r);
    for shift in (r..width).step_by(8) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::channel::wire::Reader;
    #[test]
    fn native_table_decodes_spawn_switch_remove_and_reused_parent_for_every_pool_alignment() {
        let f: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/held-weapon-3.3.3.3.json"
        )))
        .unwrap();
        assert_eq!(f["client_sha256"], crate::CLIENT_SHA256);
        for alignment in 0..8 {
            let mut baseline = BTreeMap::new();
            let mut fields = BTreeMap::new();
            for weapon in [Some(352u16), Some(363), None, Some(355)] {
                let current = weapon.map(|w| [(7, w)].into()).unwrap_or_default();
                let mut out = Writer::default();
                out.bits(0, alignment);
                write(&mut out, &current, &baseline, 1).unwrap();
                let mut r = Reader::new(&out.bytes);
                r.bits(alignment).unwrap();
                assert_eq!(r.bits(2).unwrap(), 2);
                assert_eq!(r.bits(11).unwrap(), 34);
                assert_eq!(r.bits(1).unwrap(), u32::from(weapon.is_none()));
                if let Some(weapon) = weapon {
                    assert_eq!(r.bits(2).unwrap(), 3);
                    let last = r.bits(6).unwrap();
                    let mut index = -1;
                    while index < last as i32 {
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
                        let at = f["fields"][index as usize][0].as_u64().unwrap() as u32;
                        let codec = f["fields"][index as usize][1].as_i64().unwrap();
                        let value = if codec == -77 {
                            u32::from(r.u8().unwrap())
                        } else if r.bits(1).unwrap() == 0 {
                            0
                        } else if codec == 0 {
                            assert_eq!(r.bits(1).unwrap(), 1);
                            r.u32().unwrap() ^ fields.get(&at).copied().unwrap_or(0)
                        } else {
                            let width = codec as usize;
                            let rem = width % 8;
                            let mut delta = r.bits(rem).unwrap();
                            for shift in (rem..width).step_by(8) {
                                delta |= u32::from(r.u8().unwrap()) << shift;
                            }
                            delta ^ fields.get(&at).copied().unwrap_or(0)
                        };
                        fields.insert(at, value);
                    }
                    assert_eq!(fields[&4], 6);
                    assert_eq!(fields[&0x68], u32::from(weapon));
                    assert_eq!(fields[&0xe4], 8 | (1 << 16));
                    assert_eq!(fields[&0x54], 1.0f32.to_bits());
                } else {
                    fields.clear();
                }
                r.finish().unwrap();
                baseline = current;
            }
        }
        let mut out = Writer::default();
        assert!(write(&mut out, &[(27, 352)].into(), &BTreeMap::new(), 1).is_err());
        assert!(out.bytes.is_empty());
    }
}
