//! 3.3.3.3 world bullet impacts: ET_EVENTS22 + EV_BULLET_HIT74.
//! Uses descriptor22 (0xc52888), eventParm width8 (0xc56d70), and
//! world hitEntity2046. Consumer 0x513970 latches each event entity once.
use crate::{game_state::Writer, lobby::LobbyError};

/// Match the native DirToByte maximum-dot search over the measured table.
pub fn normal_byte(normal: [f32; 3]) -> Result<u8, LobbyError> {
    static DIRECTIONS: std::sync::OnceLock<Vec<[f32; 3]>> = std::sync::OnceLock::new();
    if normal.iter().any(|v| !v.is_finite()) || normal.iter().map(|v| v * v).sum::<f32>() < 1e-12 {
        return Err(LobbyError("invalid impact normal"));
    }
    let directions = DIRECTIONS.get_or_init(|| {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/bullet-impact-3.3.3.3.json"
        ))
        .expect("verified impact fixture");
        serde_json::from_value(fixture["directions"].clone()).expect("verified direction table")
    });
    let mut best_dot = f32::NEG_INFINITY;
    let mut best = 0;
    for (index, direction) in directions.iter().enumerate() {
        let dot = (0..3).map(|i| normal[i] * direction[i]).sum::<f32>();
        if dot > best_dot {
            best_dot = dot;
            best = index;
        }
    }
    Ok(best as u8)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactSnapshot {
    pub event: ImpactEvent,
    pub entity_number: u16,
    pub origin: [f32; 3],
    pub normal_byte: u8,
    pub surface: u8,
    pub weapon: u16,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImpactEvent {
    #[default]
    Bullet,
    GrenadeExplosion,
}

impl ImpactSnapshot {
    fn validate(self) -> Result<(), LobbyError> {
        if !(20..2046).contains(&self.entity_number)
            || self
                .origin
                .iter()
                .any(|v| !v.is_finite() || !(-32768.0..=32767.0).contains(v))
            || self.normal_byte >= 162
            || self.surface > 30
            || !(1..900).contains(&self.weapon)
        {
            return Err(LobbyError("invalid world bullet impact"));
        }
        Ok(())
    }
}

/// Impacts never enter the pinned full baseline. Omitting expired events from
/// later frames therefore removes them, allowing eventual entity-ID reuse.
pub(crate) fn write_impacts(
    out: &mut Writer,
    impacts: &[ImpactSnapshot],
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    write_impacts_with_baselines(out, impacts, &[], establish_baseline)
}
pub(crate) fn write_impacts_with_baselines(
    out: &mut Writer,
    impacts: &[ImpactSnapshot],
    baselines: &[ImpactSnapshot],
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    let mut previous_number = 19;
    for impact in impacts {
        impact.validate()?;
        if impact.entity_number <= previous_number {
            return Err(LobbyError("impact entities must be sorted and unique"));
        }
        previous_number = impact.entity_number;
    }
    if establish_baseline {
        return Ok(());
    }
    let numbers: std::collections::BTreeSet<_> = impacts
        .iter()
        .chain(baselines)
        .map(|s| s.entity_number)
        .collect();
    for number in numbers {
        let Some(impact) = impacts.iter().find(|s| s.entity_number == number) else {
            out.bits(0, 1);
            out.bits(1, 1);
            out.bits(u32::from(number), 11);
            out.bits(1, 1);
            continue;
        };
        let base = baselines.iter().find(|b| b.entity_number == number);
        if base == Some(impact) {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(u32::from(impact.entity_number), 11);
        out.bits(0, 1); // retained
        out.bits(1, 1); // changed
        out.bits(1, 1); // zero position base
        out.bits(22, 6); // last changed field
        gap(out, 1);
        out.bytes.push(match impact.event {
            ImpactEvent::Bullet => 96,
            ImpactEvent::GrenadeExplosion => 105,
        });
        let mut previous = 0;
        for (index, value) in [
            (2, impact.origin[0]),
            (3, impact.origin[1]),
            (4, impact.origin[2]),
        ] {
            gap(out, index - previous);
            previous = index;
            out.bits(1, 1);
            out.bits(u32::from(value.round_ties_even() as i16 as u16), 16);
        }
        gap(out, 5 - previous);
        out.bits(u32::from(impact.normal_byte), 8); // -93: no generic gate
        previous = 5;
        if impact.surface != 0 || base.is_some_and(|b| b.surface != 0) {
            gap(out, 6 - previous);
            out.bits(u32::from(impact.surface != 0), 1);
            if impact.surface != 0 {
                out.bytes
                    .push(impact.surface ^ base.map_or(0, |b| b.surface));
            }
            previous = 6;
        }
        // shooter(field9)=local0, impact flags(field10)=0 inherit zero.
        gap(out, 16 - previous);
        out.bits(1, 1);
        out.bytes.extend_from_slice(
            &(u32::from(impact.weapon) ^ base.map_or(0, |b| u32::from(b.weapon))).to_le_bytes(),
        );
        gap(out, 22 - 16);
        out.bits(1, 1); // -96 compact world2046; 2047 would suppress FX
    }
    Ok(())
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

    fn fixture() -> serde_json::Value {
        let value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/bullet-impact-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(value["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(value["table_va"], 0xc52888);
        assert_eq!(value["fields"].as_array().unwrap().len(), 61);
        value
    }

    // Independent decoder follows the current field table. In particular -93
    // uses pool bits while generic positive widths split pool and raw bytes.
    fn decode(r: &mut Reader<'_>) -> (u16, BTreeMap<u32, u32>) {
        assert_eq!(r.bits(2).unwrap(), 2);
        let number = r.bits(11).unwrap() as u16;
        assert_eq!(r.bits(3).unwrap(), 6);
        let last = r.bits(6).unwrap() as i32;
        let mut index = -1;
        let mut fields = BTreeMap::new();
        let table = fixture();
        while index < last {
            let mut distance = 1;
            if r.bits(1).unwrap() == 0 {
                loop {
                    let chunk = r.bits(3).unwrap();
                    distance += chunk;
                    if chunk != 7 {
                        break;
                    }
                }
            }
            index += distance as i32;
            assert!((0..=last).contains(&index));
            let field = &table["fields"][index as usize];
            let offset = field[0].as_u64().unwrap() as u32;
            let codec = field[1].as_i64().unwrap();
            let value = match codec {
                -77 => u32::from(r.u8().unwrap()),
                -83..=-81 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    r.bits(16).unwrap()
                }
                -93 => r.bits(8).unwrap(),
                -96 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    2046
                }
                n if n > 0 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    let width = n as usize;
                    let rem = width % 8;
                    let mut value = r.bits(rem).unwrap();
                    for shift in (rem..width).step_by(8) {
                        value |= u32::from(r.u8().unwrap()) << shift;
                    }
                    value
                }
                _ => panic!("unexpected event codec {codec}"),
            };
            assert!(fields.insert(offset, value).is_none());
        }
        (number, fields)
    }

    fn impact() -> ImpactSnapshot {
        ImpactSnapshot {
            event: Default::default(),
            entity_number: 20,
            origin: [-312.5, -800.5, -115.0],
            normal_byte: 0,
            surface: 0,
            weapon: 352,
        }
    }

    #[test]
    fn event_decodes_with_measured_fields_and_all_pool_alignments() {
        for prefix in 0..8 {
            for surface in [0, 3, 30] {
                let mut out = Writer::default();
                out.bits(0x56, prefix);
                out.bytes.push(0xa6);
                let impacts = [
                    ImpactSnapshot {
                        surface,
                        ..impact()
                    },
                    ImpactSnapshot {
                        entity_number: 2045,
                        normal_byte: 161,
                        weapon: 899,
                        ..impact()
                    },
                ];
                write_impacts(&mut out, &impacts, false).unwrap();
                out.bits(0, 1);
                out.bits(1, 1);
                out.bits(2047, 11);
                out.bytes.push(6);
                let mut reader = Reader::new(&out.bytes);
                assert_eq!(reader.bits(prefix).unwrap(), 0x56 & ((1 << prefix) - 1));
                assert_eq!(reader.u8().unwrap(), 0xa6);
                for expected in impacts {
                    let (number, fields) = decode(&mut reader);
                    assert_eq!(number, expected.entity_number);
                    assert_eq!(fields[&4], 96);
                    assert_eq!(fields[&0x18] as u16 as i16, -312);
                    assert_eq!(fields[&0x1c] as u16 as i16, -800);
                    assert_eq!(fields[&0x20] as u16 as i16, -115);
                    assert_eq!(fields[&0xa8], u32::from(expected.normal_byte));
                    assert_eq!(
                        fields.get(&0x90).copied().unwrap_or(0),
                        u32::from(expected.surface)
                    );
                    assert_eq!(fields[&0xd0], u32::from(expected.weapon));
                    assert_eq!(fields[&0x88], 2046);
                    assert!(!fields.contains_key(&0xdc));
                }
                assert_eq!(reader.bits(2).unwrap(), 2);
                assert_eq!(reader.bits(11).unwrap(), 2047);
                assert_eq!(reader.u8().unwrap(), 6);
                reader.finish().unwrap();
            }
        }
    }

    #[test]
    fn baselines_and_expired_frames_contain_no_impact_entities() {
        let mut out = Writer::default();
        out.bits(5, 3);
        let before = out.bytes.clone();
        write_impacts(&mut out, &[impact()], true).unwrap();
        assert_eq!(out.bytes, before);
        write_impacts(&mut out, &[], false).unwrap();
        assert_eq!(out.bytes, before);
        for invalid in [
            ImpactSnapshot {
                entity_number: 19,
                ..impact()
            },
            ImpactSnapshot {
                normal_byte: 162,
                ..impact()
            },
            ImpactSnapshot {
                surface: 31,
                ..impact()
            },
            ImpactSnapshot {
                weapon: 0,
                ..impact()
            },
            ImpactSnapshot {
                origin: [f32::NAN; 3],
                ..impact()
            },
        ] {
            assert!(write_impacts(&mut out, &[invalid], false).is_err());
        }
        assert!(write_impacts(&mut out, &[impact(), impact()], false).is_err());
    }

    #[test]
    fn captured_direction_table_preserves_native_byte_indices() {
        let table = fixture();
        let directions: Vec<[f32; 3]> =
            serde_json::from_value(table["directions"].clone()).unwrap();
        assert_eq!(directions.len(), 162);
        for (index, vector) in directions.into_iter().enumerate() {
            assert_eq!(normal_byte(vector).unwrap(), index as u8);
        }
        assert_eq!(normal_byte([0.0, 0.0, 1.0]).unwrap(), 5);
        assert_eq!(normal_byte([-1.0, 0.0, 0.0]).unwrap(), 143);
        assert!(normal_byte([0.0; 3]).is_err());
        assert!(normal_byte([f32::NAN; 3]).is_err());
    }
}
