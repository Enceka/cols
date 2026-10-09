//! Native 3.3.3.3 kill feed: ET_EVENTS22 + EV_OBITUARY132 = eType154.
//! Descriptor22 (0xc52888) supplies the event fields; 0x50d270 consumes
//! eventParm as a registered weapon and ES+0x80/+0x84 as victim/attacker.
//! eventParm>=900 selects a means-of-death icon instead; MOD9 (909) is the
//! native `killiconheadshot` string, replacing the weapon icon in the feed.
use crate::{game_state::Writer, lobby::LobbyError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObituarySnapshot {
    /// One temporary entity, outside the player/corpse slots0..20. The caller
    /// allocates IDs separately from impacts and sorts the complete stream.
    pub entity_number: u16,
    pub weapon: u16,
    /// Show the native headshot icon instead of the registered weapon.
    pub headshot: bool,
    /// Current training world has local client0 and enemy client1.
    pub victim: u8,
    pub attacker: u8,
}

const MEANS_OF_DEATH_BASE: u32 = 900;
const MOD_HEAD_SHOT: u32 = 9;

impl ObituarySnapshot {
    fn event_parameter(self) -> u32 {
        if self.headshot {
            MEANS_OF_DEATH_BASE + MOD_HEAD_SHOT
        } else {
            u32::from(self.weapon)
        }
    }
    fn validate(self) -> Result<(), LobbyError> {
        if !(27..2046).contains(&self.entity_number)
            || !(1..900).contains(&self.weapon)
            || self.victim > 18
            || self.attacker > 18
        {
            return Err(LobbyError("invalid training obituary"));
        }
        Ok(())
    }
}

/// Write only event records, leaving the entity sentinel to the caller.
/// Obituaries are excluded from pinned baselines. Keeping an ID in ordinary
/// frames lets 0x513970 latch it once; omission after its lifetime clears it
/// before eventual ID reuse. Event IDs, expiry and rotation belong to gameplay.
pub(crate) fn write_obituaries(
    out: &mut Writer,
    obituaries: &[ObituarySnapshot],
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    write_obituaries_with_baselines(out, obituaries, &[], establish_baseline)
}
pub(crate) fn write_obituaries_with_baselines(
    out: &mut Writer,
    obituaries: &[ObituarySnapshot],
    baselines: &[ObituarySnapshot],
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    let mut previous_number = 26;
    for obituary in obituaries {
        obituary.validate()?;
        if obituary.entity_number <= previous_number {
            return Err(LobbyError("obituary entities must be sorted and unique"));
        }
        previous_number = obituary.entity_number;
    }
    if establish_baseline {
        return Ok(());
    }
    let numbers: std::collections::BTreeSet<_> = obituaries
        .iter()
        .chain(baselines)
        .map(|s| s.entity_number)
        .collect();
    for number in numbers {
        let Some(obituary) = obituaries.iter().find(|s| s.entity_number == number) else {
            out.bits(0, 1);
            out.bits(1, 1);
            out.bits(u32::from(number), 11);
            out.bits(1, 1);
            continue;
        };
        let base = baselines.iter().find(|b| b.entity_number == number);
        if base == Some(obituary) {
            continue;
        }
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(u32::from(obituary.entity_number), 11);
        out.bits(0, 1); // retained
        out.bits(1, 1); // changed
        out.bits(1, 1); // zero position/angle bases
        let last = if obituary.attacker != 0 || base.is_some_and(|b| b.attacker != 0) {
            13
        } else if obituary.victim != 0 || base.is_some_and(|b| b.victim != 0) {
            9
        } else {
            5
        };
        out.bits(last, 6);
        gap(out, 1);
        out.bytes.push(154); // field0: raw eType byte, codec-77
        gap(out, 5);
        // field5 ES+a8, codec-93: EV132 table width10, directly in bit pool.
        // No generic nonzero gate, and no raw byte split for this codec.
        out.bits(obituary.event_parameter(), 10);
        let mut previous = 5;
        if obituary.victim != 0 || base.is_some_and(|b| b.victim != 0) {
            gap(out, 9 - previous);
            client_index_delta(out, obituary.victim, base.map_or(0, |b| b.victim));
            previous = 9;
        }
        if obituary.attacker != 0 || base.is_some_and(|b| b.attacker != 0) {
            gap(out, 13 - previous);
            client_index_delta(out, obituary.attacker, base.map_or(0, |b| b.attacker));
        }
        // Omitted zero client indices and XYZ inherit the all-zero event
        // baseline. The dispatcher gates on event ID, not event position;
        // the obituary consumer does not use the event's origin.
    }
    Ok(())
}

fn client_index_delta(out: &mut Writer, value: u8, base: u8) {
    out.bits(u32::from(value != 0), 1);
    if value != 0 {
        let delta = value ^ base;
        out.bits(u32::from(delta) & 7, 3);
        out.bytes.push(delta >> 3);
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
    use crate::game_wire::{Reader, SERVER_EOF};
    use std::collections::BTreeMap;
    #[test]
    fn rolling_obituary_deltas_clear_changed_clients_and_remove_expired_events_once() {
        let old = ObituarySnapshot {
            entity_number: 1800,
            weapon: 352,
            headshot: false,
            victim: 11,
            attacker: 6,
        };
        let current = ObituarySnapshot {
            victim: 0,
            attacker: 7,
            ..old
        };
        let mut out = Writer::default();
        write_obituaries_with_baselines(&mut out, &[current], &[old], false).unwrap();
        let (_, fields) = decode(&mut Reader::new(&out.bytes));
        assert_eq!(fields[&0x80], 0);
        assert_eq!(fields[&0x84] ^ u32::from(old.attacker), 7);
        let mut identical = Writer::default();
        write_obituaries_with_baselines(&mut identical, &[current], &[current], false).unwrap();
        assert!(identical.bytes.is_empty());
        let mut removal = Writer::default();
        write_obituaries_with_baselines(&mut removal, &[], &[current], false).unwrap();
        let mut r = Reader::new(&removal.bytes);
        assert_eq!(r.bits(2).unwrap(), 2);
        assert_eq!(r.bits(11).unwrap(), 1800);
        assert_eq!(r.bits(1).unwrap(), 1);
    }

    fn fixture() -> serde_json::Value {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/obituary-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(fixture["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(fixture["table_va"], 0xc52888);
        assert_eq!(fixture["event_id"], 132);
        assert_eq!(fixture["entity_type"], 154);
        assert_eq!(fixture["event_parameter_bits"], 10);
        assert_eq!(fixture["fields"].as_array().unwrap().len(), 61);
        fixture
    }

    // Decoder is driven by the independently extracted current-client table.
    // Positive-width11 fields mix pool bits and raw bytes; eventParm-93 does
    // not use that generic numeric layout, even when its width exceeds8.
    fn decode(reader: &mut Reader<'_>) -> (u16, BTreeMap<u32, u32>) {
        assert_eq!(reader.bits(2).unwrap(), 2);
        let number = reader.bits(11).unwrap() as u16;
        assert_eq!(reader.bits(3).unwrap(), 6);
        let last = reader.bits(6).unwrap() as i32;
        let fixture = fixture();
        let mut fields = BTreeMap::new();
        let mut index = -1;
        while index < last {
            let mut distance = 1;
            if reader.bits(1).unwrap() == 0 {
                loop {
                    let part = reader.bits(3).unwrap();
                    distance += part;
                    if part != 7 {
                        break;
                    }
                }
            }
            index += distance as i32;
            assert!((0..=last).contains(&index));
            let descriptor = &fixture["fields"][index as usize];
            let offset = descriptor[0].as_u64().unwrap() as u32;
            let codec = descriptor[1].as_i64().unwrap();
            let value = match codec {
                -77 => u32::from(reader.u8().unwrap()),
                -93 => {
                    assert_eq!(fields.get(&4), Some(&154));
                    reader
                        .bits(fixture["event_parameter_bits"].as_u64().unwrap() as usize)
                        .unwrap()
                }
                n if n > 0 => {
                    if reader.bits(1).unwrap() == 0 {
                        fields.insert(offset, 0);
                        continue;
                    }
                    let width = n as usize;
                    let low = width % 8;
                    let mut value = reader.bits(low).unwrap();
                    for shift in (low..width).step_by(8) {
                        value |= u32::from(reader.u8().unwrap()) << shift;
                    }
                    value
                }
                _ => panic!("unexpected obituary codec {codec}"),
            };
            assert!(fields.insert(offset, value).is_none());
        }
        (number, fields)
    }

    fn obituary() -> ObituarySnapshot {
        ObituarySnapshot {
            entity_number: 1800,
            weapon: 352,
            headshot: false,
            victim: 1,
            attacker: 0,
        }
    }

    fn finish(out: &mut Writer) {
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(2047, 11);
        out.bytes.push(SERVER_EOF);
    }

    fn read_finish(reader: &mut Reader<'_>) {
        assert_eq!(reader.bits(2).unwrap(), 2);
        assert_eq!(reader.bits(11).unwrap(), 2047);
        assert_eq!(reader.u8().unwrap(), SERVER_EOF);
        reader.finish().unwrap();
    }

    #[test]
    fn kills_and_suicides_decode_with_current_fields_at_every_pool_alignment() {
        for prefix in 0..8 {
            for (weapon, headshot) in [(1, false), (352, false), (899, false), (352, true)] {
                let mut out = Writer::default();
                out.bits(0x56, prefix);
                out.bytes.push(0xa6);
                let records = [
                    ObituarySnapshot {
                        entity_number: 27,
                        weapon,
                        headshot,
                        victim: 0,
                        attacker: 0,
                    },
                    ObituarySnapshot {
                        entity_number: 1800,
                        weapon,
                        headshot,
                        victim: 1,
                        attacker: 0,
                    },
                    ObituarySnapshot {
                        entity_number: 1801,
                        weapon,
                        headshot,
                        victim: 0,
                        attacker: 1,
                    },
                    ObituarySnapshot {
                        entity_number: 2045,
                        weapon,
                        headshot,
                        victim: 1,
                        attacker: 1,
                    },
                ];
                write_obituaries(&mut out, &records, false).unwrap();
                finish(&mut out);
                let mut reader = Reader::new(&out.bytes);
                assert_eq!(reader.bits(prefix).unwrap(), 0x56 & ((1 << prefix) - 1));
                assert_eq!(reader.u8().unwrap(), 0xa6);
                for expected in records {
                    let (number, fields) = decode(&mut reader);
                    assert_eq!(number, expected.entity_number);
                    assert_eq!(fields[&4], 154);
                    assert_eq!(
                        fields[&0xa8],
                        if expected.headshot {
                            909
                        } else {
                            u32::from(expected.weapon)
                        }
                    );
                    assert_eq!(
                        fields.get(&0x80).copied().unwrap_or(0),
                        u32::from(expected.victim)
                    );
                    assert_eq!(
                        fields.get(&0x84).copied().unwrap_or(0),
                        u32::from(expected.attacker)
                    );
                    assert_eq!(
                        fields.len(),
                        2 + expected.victim as usize + expected.attacker as usize
                    );
                    assert!([0x18, 0x1c, 0x20].iter().all(|p| !fields.contains_key(p)));
                }
                read_finish(&mut reader);
            }
        }
    }

    #[test]
    fn repeated_frames_preserve_event_identity_and_baselines_exclude_it() {
        let records = [obituary()];
        let mut first = Writer::default();
        let mut repeat = Writer::default();
        write_obituaries(&mut first, &records, false).unwrap();
        write_obituaries(&mut repeat, &records, false).unwrap();
        assert_eq!(first.bytes, repeat.bytes);
        for prefix in 0..8 {
            let mut out = Writer::default();
            out.bits(0x56, prefix);
            out.bytes.push(0xa6);
            let before = out.bytes.clone();
            write_obituaries(&mut out, &records, true).unwrap();
            assert_eq!(out.bytes, before);
            write_obituaries(&mut out, &[], false).unwrap();
            assert_eq!(out.bytes, before);
            finish(&mut out);
            let mut reader = Reader::new(&out.bytes);
            assert_eq!(reader.bits(prefix).unwrap(), 0x56 & ((1 << prefix) - 1));
            assert_eq!(reader.u8().unwrap(), 0xa6);
            read_finish(&mut reader);
        }
    }

    #[test]
    fn invalid_or_unordered_events_fail_before_mutating_the_stream() {
        let valid = obituary();
        for invalid in [
            ObituarySnapshot {
                entity_number: 20,
                ..valid
            },
            ObituarySnapshot {
                entity_number: 2046,
                ..valid
            },
            ObituarySnapshot { weapon: 0, ..valid },
            ObituarySnapshot {
                weapon: 900,
                ..valid
            },
            ObituarySnapshot {
                victim: 19,
                ..valid
            },
            ObituarySnapshot {
                attacker: 19,
                ..valid
            },
        ] {
            for baseline in [false, true] {
                let mut out = Writer::default();
                out.bits(5, 3);
                let before = out.bytes.clone();
                assert!(write_obituaries(&mut out, &[valid, invalid], baseline).is_err());
                assert_eq!(out.bytes, before);
                finish(&mut out);
                let mut reader = Reader::new(&out.bytes);
                assert_eq!(reader.bits(3).unwrap(), 5);
                read_finish(&mut reader);
            }
        }
        for records in [
            [valid, valid],
            [
                valid,
                ObituarySnapshot {
                    entity_number: 27,
                    ..valid
                },
            ],
        ] {
            let mut out = Writer::default();
            assert!(write_obituaries(&mut out, &records, false).is_err());
            assert!(out.bytes.is_empty());
        }
    }
}
