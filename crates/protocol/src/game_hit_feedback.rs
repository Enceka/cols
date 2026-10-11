//! Native MATERIAL HUD feedback in the current 3.3.3.3 playerState arrays.
//!
//! 0x5a4230 reads the two arrays through 0x5a4010/table 0xc56ac0. Type 4
//! routes to 0x515680, which resolves the actual material name through the
//! material configstring domain. This is a server-owned HUD element; the
//! received-damage directional indicator 131 is a separate path.
use crate::{game_state::Writer, lobby::LobbyError};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HitKind {
    #[default]
    Hit,
    Kill,
    Scavenger,
}
impl HitKind {
    pub fn duration_msec(self) -> u32 {
        match self {
            Self::Hit => 1000,
            Self::Kill => 600,
            Self::Scavenger => 2500,
        }
    }
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::Hit | Self::Scavenger => [255; 3],
            Self::Kill => [255, 41, 20],
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HitFeedbackSnapshot {
    pub material: u16,
    /// Authoritative hit time, in the same clock used by the snapshot.
    pub event_time: u32,
    pub visible: bool,
    pub kind: HitKind,
}

#[derive(Clone, Copy, Debug)]
pub struct ReplayHudSnapshot {
    pub end_time: u32,
    /// Map-local NCS localized text slot; present only for skippable replays.
    pub skip_text: Option<u16>,
}

/// Native world waypoint (type13). Position is frozen at death, not attached
/// to a recyclable corpse entity. Coordinates use the published CS16 zero centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathWaypointSnapshot {
    pub material: u16,
    pub origin: [f32; 3],
}
impl DeathWaypointSnapshot {
    pub fn valid(self) -> bool {
        (1..1024).contains(&self.material)
            && self
                .origin
                .iter()
                .all(|v| v.is_finite() && (-32768.0..=32767.0).contains(v))
    }
}

/// Write the complete HUD extension gate and both arrays. The playerState
/// caller must select its zero baseline, as the ordinary snapshot writer does.
/// Non-archived death waypoints use PS+0xa78; the PS+0x1ed0 array owns the
/// short-lived hit material. Empty arrays explicitly clear expired elements.
pub(crate) fn write_hud(
    out: &mut Writer,
    feedback: Option<HitFeedbackSnapshot>,
    deaths: &[DeathWaypointSnapshot],
    replay: Option<ReplayHudSnapshot>,
) -> Result<(), LobbyError> {
    let feedback = feedback.unwrap_or_default();
    if feedback.visible && !(1..1024).contains(&feedback.material) {
        return Err(LobbyError("invalid hit feedback material"));
    }
    if deaths.len() > 5 || deaths.iter().any(|d| !d.valid()) {
        return Err(LobbyError("invalid friendly death waypoints"));
    }
    let skip_text = replay.and_then(|r| r.skip_text);
    if skip_text.is_some_and(|slot| !(1..1023).contains(&slot)) {
        return Err(LobbyError("invalid replay skip text slot"));
    }
    out.bits(1, 1); // HUD extension is present.
    // Wire order is archival then current; memory order is the reverse.
    // 0x5a4230 writes PS+0x1ed0 first, then PS+0xa78 (0x5a51f6 region).
    out.bits(u32::from(feedback.visible), 5);
    if feedback.visible {
        write_hit(out, feedback);
    }
    out.bits(
        deaths.len() as u32 + u32::from(replay.is_some()) + u32::from(skip_text.is_some()),
        5,
    );
    for death in deaths {
        write_death(out, *death);
    }
    if let Some(replay) = replay {
        write_replay_timer(out, replay.end_time);
    }
    if let Some(text) = skip_text {
        write_replay_skip(out, text);
    }
    Ok(())
}

fn write_replay_skip(out: &mut Writer, text: u16) {
    // Text HUD: +0x84/field22 -> 0x514230 -> CS952+slot, localized text
    // expansion and native input bindings. No literal key name is painted.
    out.bits(23, 6);
    let mut previous = None;
    for index in [0u32, 1, 3, 7, 11, 14, 15, 16, 22] {
        field_gap(out, previous.map_or(index + 1, |p| index - p));
        previous = Some(index);
        match index {
            0 => color(out, [255; 3], 31),
            1 => {
                out.bits(0, 1);
                out.bits(38, 7);
            } // y=-26
            3 => unsigned(out, 1, 4),
            7 => unsigned(out, 2047, 11),
            11 => unsigned(out, 0x23, 8), // centre/bottom
            14 => unsigned(out, 5, 18),
            15 => unsigned(out, 5, 4),
            16 => out.bits(10, 6), // fontScale1.0; font0 is native default
            22 => unsigned(out, u32::from(text), 10),
            _ => unreachable!(),
        }
    }
}

fn write_replay_timer(out: &mut Writer, end: u32) {
    // 0x514420: type8 formats ceil((time-cgTime)/100) as mm:ss.t.
    // 0x514b70 maps font8 to UI11 (Agency); native scale codec is 6 bits /10.
    out.bits(35, 6);
    let mut previous = None;
    for index in [0u32, 1, 3, 7, 11, 14, 15, 16, 20, 34] {
        field_gap(out, previous.map_or(index + 1, |p| index - p));
        previous = Some(index);
        match index {
            0 => color(out, [15; 3], 31),
            1 => {
                out.bits(0, 1);
                out.bits(106, 7);
            } // y=42, top adjustable.
            3 => unsigned(out, 8, 4),
            7 => unsigned(out, 2047, 11),
            11 => unsigned(out, 0x78, 8), // safe-area horizontal centre / top adjustable
            14 => unsigned(out, 5, 18),   // foreground, hidden in menus
            15 => unsigned(out, 5, 4),    // centre/middle anchor
            16 => out.bits(16, 6),        // scale1.6 (closest native value to1.56)
            20 => unsigned(out, 8, 4),
            34 => {
                out.bits(1, 1);
                out.bytes.extend_from_slice(&end.to_le_bytes());
            }
            _ => unreachable!(),
        }
    }
}

fn write_hit(out: &mut Writer, feedback: HitFeedbackSnapshot) {
    out.bits(15, 6); // Last changed field 14, expressed as field count.
    let mut previous = None;
    for index in [0u32, 1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14] {
        let distance = previous.map_or(index + 1, |before| index - before);
        field_gap(out, distance);
        previous = Some(index);
        match index {
            0 => color(out, feedback.kind.rgb(), 0), // Target colour, zero alpha.
            1 | 2 => {
                // -91/-92: compact signed delta from the zero HUD baseline.
                // Offsets and 24x48 dimensions follow the existing resource's
                // damage-feedback script usage; placement is runtime checked.
                out.bits(0, 1);
                let coordinate = if feedback.kind == HitKind::Scavenger {
                    if index == 1 { 32 } else { -36 }
                } else {
                    -12
                };
                out.bits((coordinate + 64) as u32, 7);
            }
            3 => unsigned(out, 4, 4), // HE_TYPE_MATERIAL.
            4 => unsigned(out, u32::from(feedback.material), 10),
            5 => unsigned(
                out,
                if feedback.kind == HitKind::Scavenger {
                    32
                } else {
                    48
                },
                10,
            ),
            6 => unsigned(
                out,
                if feedback.kind == HitKind::Scavenger {
                    64
                } else {
                    24
                },
                10,
            ),
            7 => unsigned(out, 2047, 11), // No target entity attachment.
            10 => {
                // -97: direct 32-bit time path, avoiding a relative-time base.
                out.bits(1, 1);
                out.bytes
                    .extend_from_slice(&feedback.event_time.to_le_bytes());
            }
            11 => unsigned(out, 0x22, 8), // Screen centre/middle, local top-left.
            12 => color(out, feedback.kind.rgb(), 31), // Opaque starting colour.
            13 => unsigned(out, feedback.kind.duration_msec(), 16),
            14 => unsigned(out, 1, 18), // Foreground HUD pass.
            _ => unreachable!(),
        }
    }
}

fn write_death(out: &mut Writer, death: DeathWaypointSnapshot) {
    out.bits(15, 6);
    let mut previous = None;
    for index in [0u32, 1, 2, 3, 4, 5, 6, 7, 8, 14] {
        field_gap(out, previous.map_or(index + 1, |before| index - before));
        previous = Some(index);
        match index {
            0 => color(out, [255; 3], 31),
            1 | 2 | 8 => {
                let axis = match index {
                    1 => 1,
                    2 => 0,
                    _ => 2,
                };
                // 0x5a2320/0x5a23d0, -91/-92/-90: signed16 XOR delta,
                // zero HUD baseline and CS16=(0,0,0), as for entity origins.
                out.bits(1, 1);
                out.bits(
                    u32::from(death.origin[axis].round_ties_even() as i16 as u16),
                    16,
                );
            }
            3 => unsigned(out, 13, 4),
            4 => unsigned(out, u32::from(death.material), 10),
            5 | 6 => unsigned(out, 24, 10),
            7 => unsigned(out, 2047, 11),
            // Foreground and native explicit waypoint dimensions (0x536697).
            // Edge arrow bit8 stays clear, so offscreen icons are clipped.
            14 => unsigned(out, 0x2001, 18),
            _ => unreachable!(),
        }
    }
}

fn unsigned(out: &mut Writer, value: u32, width: usize) {
    out.bits(u32::from(value != 0), 1);
    if value == 0 {
        return;
    }
    let remainder = width % 8;
    out.bits(value, remainder);
    for shift in (remainder..width).step_by(8) {
        out.bytes.push((value >> shift) as u8);
    }
}

fn color(out: &mut Writer, rgb: [u8; 3], alpha: u32) {
    // -85: explicit RGB branch (three direct bytes), then 5 pooled alpha bits.
    // Alpha 31 decodes to 255; other values decode to alpha*8.
    out.bits(0, 1); // Do not toggle the baseline alpha.
    out.bits(0, 1); // Explicit RGB.
    out.bits(1, 1); // Direct RGB bytes, not a palette index.
    out.bytes.extend_from_slice(&rgb);
    out.bits(alpha, 5);
}

fn field_gap(out: &mut Writer, distance: u32) {
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
            "../../../fixtures/protocol/hit-feedback-hud-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(value["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(value["table_va"], 0xc56ac0);
        assert_eq!(value["hud_element_bytes"], 168);
        assert_eq!(value["fields"].as_array().unwrap().len(), 42);
        value
    }

    // Decode the extension by the independently extracted current HUD table.
    // No calls to the writer's gap/colour/integer helpers are reused.
    fn read_array(r: &mut Reader<'_>) -> Vec<BTreeMap<u32, u32>> {
        let count = r.bits(5).unwrap();
        assert!(count < 32);
        let layout = fixture();
        let mut elements = Vec::new();
        for _ in 0..count {
            let last = r.bits(6).unwrap();
            assert!(last <= 42);
            let mut index = -1i32;
            let mut fields = BTreeMap::new();
            while index + 1 < last as i32 {
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
                assert!(index >= 0 && index < last as i32);
                let row = &layout["fields"][index as usize];
                let offset = row[0].as_u64().unwrap() as u32;
                let codec = row[1].as_i64().unwrap();
                let value = match codec {
                    -85 => {
                        assert_eq!(r.bits(1).unwrap(), 0); // No alpha toggle.
                        assert_eq!(r.bits(1).unwrap(), 0); // Read RGB.
                        assert_eq!(r.bits(1).unwrap(), 1); // Direct bytes.
                        let red = u32::from(r.u8().unwrap());
                        let green = u32::from(r.u8().unwrap());
                        let blue = u32::from(r.u8().unwrap());
                        let alpha = r.bits(5).unwrap();
                        let alpha = if alpha == 31 { 255 } else { alpha * 8 };
                        red | green << 8 | blue << 16 | alpha << 24
                    }
                    -92..=-90 => {
                        if r.bits(1).unwrap() == 0 {
                            ((r.bits(7).unwrap() as i32 - 64) as f32).to_bits()
                        } else {
                            // Native zero baseline/CS16 centre: XOR biased
                            // signed16 reconstruction reduces to sign extension.
                            (r.bits(16).unwrap() as u16 as i16 as f32).to_bits()
                        }
                    }
                    -97 => {
                        assert_eq!(r.bits(1).unwrap(), 1); // Absolute time.
                        r.u32().unwrap()
                    }
                    -86 => (r.bits(6).unwrap() as f32 / 10.0).to_bits(),
                    width if width > 0 => {
                        if r.bits(1).unwrap() == 0 {
                            0
                        } else {
                            let width = width as usize;
                            let remainder = width % 8;
                            let mut value = r.bits(remainder).unwrap();
                            for shift in (remainder..width).step_by(8) {
                                value |= u32::from(r.u8().unwrap()) << shift;
                            }
                            value
                        }
                    }
                    _ => panic!("unexpected HUD codec {codec}"),
                };
                assert!(fields.insert(offset, value).is_none());
            }
            elements.push(fields);
        }
        elements
    }

    #[test]
    fn replay_skip_text_uses_localized_hud_and_only_exists_with_the_prompt() {
        for alignment in 0..8 {
            for skip_text in [None, Some(1), Some(247), Some(1022)] {
                let mut out = Writer::default();
                out.bits(0x65, alignment);
                write_hud(
                    &mut out,
                    None,
                    &[],
                    Some(ReplayHudSnapshot {
                        end_time: 12345,
                        skip_text,
                    }),
                )
                .unwrap();
                out.bytes.push(0xa7);
                let mut r = Reader::new(&out.bytes);
                r.bits(alignment).unwrap();
                assert_eq!(r.bits(1).unwrap(), 1);
                assert!(read_array(&mut r).is_empty());
                let current = read_array(&mut r);
                assert_eq!(current.len(), 1 + usize::from(skip_text.is_some()));
                if let Some(slot) = skip_text {
                    let text = &current[1];
                    assert_eq!(text[&0], 1);
                    assert_eq!(text[&0x84], u32::from(slot));
                    assert_eq!(text[&8], (-26.0f32).to_bits());
                    assert_eq!(text[&0x14], 1.0f32.to_bits());
                    assert_eq!(text[&0x2c], 0x23);
                    assert_eq!(text[&0xa4], 5);
                    assert_eq!(text[&0x30], 0xffffffff);
                }
                assert_eq!(r.u8().unwrap(), 0xa7);
            }
        }
        for invalid in [0, 1023, u16::MAX] {
            let mut out = Writer::default();
            assert!(
                write_hud(
                    &mut out,
                    None,
                    &[],
                    Some(ReplayHudSnapshot {
                        end_time: 1,
                        skip_text: Some(invalid)
                    })
                )
                .is_err()
            );
            assert!(out.bytes.is_empty());
        }
    }
    #[test]
    fn native_material_hud_decodes_at_every_pool_alignment() {
        for (alignment, kind, color, duration) in (0..8).flat_map(|a| {
            [
                (a, HitKind::Hit, 0x00ff_ffff, 1000),
                (a, HitKind::Kill, 0x0014_29ff, 600),
                (a, HitKind::Scavenger, 0x00ff_ffff, 2500),
            ]
        }) {
            let feedback = HitFeedbackSnapshot {
                material: 125,
                event_time: 12_345,
                visible: true,
                kind,
            };
            let mut out = Writer::default();
            out.bits(0x65, alignment);
            out.bytes.push(0xb6);
            write_hud(&mut out, Some(feedback), &[], None).unwrap();
            out.bits(0x2d, 6);
            out.bytes.push(0xa7);

            let mut r = Reader::new(&out.bytes);
            assert_eq!(r.bits(alignment).unwrap(), 0x65 & ((1 << alignment) - 1));
            assert_eq!(r.u8().unwrap(), 0xb6);
            assert_eq!(r.bits(1).unwrap(), 1);
            let current = read_array(&mut r);
            assert!(read_array(&mut r).is_empty());
            assert_eq!(current.len(), 1);
            let hud = &current[0];
            assert_eq!(hud[&0], 4);
            assert_eq!(
                hud[&4],
                (if kind == HitKind::Scavenger {
                    -36.0f32
                } else {
                    -12.0f32
                })
                .to_bits()
            );
            assert_eq!(
                hud[&8],
                (if kind == HitKind::Scavenger {
                    32.0f32
                } else {
                    -12.0f32
                })
                .to_bits()
            );
            assert_eq!(hud[&0x10], 2047);
            assert_eq!(hud[&0x2c], 0x22);
            assert_eq!(hud[&0x30], color);
            assert_eq!(hud[&0x34], color | 0xff00_0000);
            assert_eq!(hud[&0x38], 12_345);
            assert_eq!(hud[&0x3c], duration);
            assert_eq!(hud[&0x44], if kind == HitKind::Scavenger { 64 } else { 24 });
            assert_eq!(hud[&0x48], if kind == HitKind::Scavenger { 32 } else { 48 });
            assert_eq!(hud[&0x4c], 125);
            assert_eq!(hud[&0xa4], 1);
            assert!(!hud.contains_key(&0x28)); // Zero baseline top-left origin.
            assert_eq!(r.bits(6).unwrap(), 0x2d);
            assert_eq!(r.u8().unwrap(), 0xa7);
        }
    }

    #[test]
    fn death_waypoints_and_hit_feedback_coexist_in_separate_arrays_at_every_alignment() {
        for alignment in 0..8 {
            let deaths = [
                DeathWaypointSnapshot {
                    material: 169,
                    origin: [-32768.0, 32767.0, -123.0],
                },
                DeathWaypointSnapshot {
                    material: 169,
                    origin: [1184.0, -1960.0, 123.0],
                },
            ];
            let mut out = Writer::default();
            out.bits(0x65, alignment);
            write_hud(
                &mut out,
                Some(HitFeedbackSnapshot {
                    material: 144,
                    event_time: 2345,
                    visible: true,
                    kind: HitKind::Kill,
                }),
                &deaths,
                None,
            )
            .unwrap();
            out.bytes.push(0xa7);
            let mut r = Reader::new(&out.bytes);
            assert_eq!(r.bits(alignment).unwrap(), 0x65 & ((1 << alignment) - 1));
            assert_eq!(r.bits(1).unwrap(), 1);
            let archived = read_array(&mut r);
            assert_eq!(archived.len(), 1);
            assert_eq!(archived[0][&0x4c], 144);
            assert_eq!(archived[0][&0x3c], 600);
            let hud = read_array(&mut r);
            assert_eq!(hud.len(), 2);
            for (value, death) in hud.iter().zip(deaths) {
                assert_eq!(value[&0], 13);
                for axis in 0..3 {
                    assert_eq!(value[&(4 + axis as u32 * 4)], death.origin[axis].to_bits());
                }
                assert_eq!(value[&0x30], 0xffff_ffff);
                assert_eq!(value[&0x10], 2047);
                assert_eq!(value[&0x44], 24);
                assert_eq!(value[&0x48], 24);
                assert_eq!(value[&0x4c], 169);
                assert_eq!(value[&0xa4], 0x2001); // Explicit size; no offscreen arrow.
                assert!(!value.contains_key(&0x3c)); // No stale hit fade.
            }
            assert_eq!(r.u8().unwrap(), 0xa7);
        }
    }
    #[test]
    fn replay_countdown_uses_native_tenths_time_and_numeric_font_and_clears_on_exit() {
        for alignment in 0..8 {
            let mut out = Writer::default();
            out.bits(0x65, alignment);
            write_hud(
                &mut out,
                None,
                &[],
                Some(ReplayHudSnapshot {
                    end_time: 12345,
                    skip_text: None,
                }),
            )
            .unwrap();
            out.bytes.push(0xa7);
            let mut r = Reader::new(&out.bytes);
            r.bits(alignment).unwrap();
            assert_eq!(r.bits(1).unwrap(), 1);
            assert!(read_array(&mut r).is_empty());
            let current = read_array(&mut r);
            assert_eq!(current.len(), 1);
            let timer = &current[0];
            assert_eq!(timer[&0], 8);
            assert_eq!(timer[&0x78], 12345);
            assert_eq!(timer[&0x24], 8);
            assert_eq!(timer[&0x14], 1.6f32.to_bits());
            assert_eq!(timer[&8], 42.0f32.to_bits());
            assert_eq!(timer[&0x2c], 0x78);
            assert_eq!(timer[&0x28], 5);
            assert_eq!(timer[&0x30], 0xff0f0f0f);
            assert_eq!(r.u8().unwrap(), 0xa7);
            let mut clear = Writer::default();
            write_hud(&mut clear, None, &[], None).unwrap();
            let mut r = Reader::new(&clear.bytes);
            r.bits(1).unwrap();
            assert!(read_array(&mut r).is_empty());
            assert!(read_array(&mut r).is_empty());
        }
    }
    #[test]
    fn invalid_death_waypoints_fail_before_touching_the_packet() {
        let good = DeathWaypointSnapshot {
            material: 169,
            origin: [0.0; 3],
        };
        for deaths in [
            vec![good; 6],
            vec![DeathWaypointSnapshot {
                material: 0,
                ..good
            }],
            vec![DeathWaypointSnapshot {
                material: 1024,
                ..good
            }],
            vec![DeathWaypointSnapshot {
                origin: [f32::NAN, 0.0, 0.0],
                ..good
            }],
            vec![DeathWaypointSnapshot {
                origin: [0.0, 0.0, 32768.0],
                ..good
            }],
        ] {
            let mut out = Writer::default();
            out.bytes.push(0xa7);
            assert!(write_hud(&mut out, None, &deaths, None).is_err());
            assert_eq!(out.bytes, [0xa7]);
        }
    }
    #[test]
    fn hidden_feedback_clears_both_native_arrays_without_a_material() {
        let mut out = Writer::default();
        write_hud(&mut out, Some(HitFeedbackSnapshot::default()), &[], None).unwrap();
        out.bytes.push(0xa7);
        let mut r = Reader::new(&out.bytes);
        assert_eq!(r.bits(1).unwrap(), 1);
        assert!(read_array(&mut r).is_empty());
        assert!(read_array(&mut r).is_empty());
        assert_eq!(r.u8().unwrap(), 0xa7);
    }

    #[test]
    fn a_bad_visible_material_rejects_before_changing_the_packet() {
        for material in [0, 1024, u16::MAX] {
            let mut out = Writer::default();
            out.bytes.push(0xa7);
            let before = out.bytes.clone();
            assert!(
                write_hud(
                    &mut out,
                    Some(HitFeedbackSnapshot {
                        material,
                        event_time: 12_345,
                        visible: true,
                        kind: HitKind::Hit
                    }),
                    &[],
                    None,
                )
                .is_err()
            );
            assert_eq!(out.bytes, before);
        }
    }

    #[test]
    fn absolute_time_keeps_the_full_native_u32_clock() {
        let mut out = Writer::default();
        write_hud(
            &mut out,
            Some(HitFeedbackSnapshot {
                material: 1023,
                event_time: u32::MAX - 20,
                visible: true,
                kind: HitKind::Hit,
            }),
            &[],
            None,
        )
        .unwrap();
        let mut r = Reader::new(&out.bytes);
        assert_eq!(r.bits(1).unwrap(), 1);
        let hud = read_array(&mut r);
        assert!(read_array(&mut r).is_empty());
        assert_eq!(hud[0][&0x38], u32::MAX - 20);
        assert_eq!(hud[0][&0x4c], 1023);
    }
}
