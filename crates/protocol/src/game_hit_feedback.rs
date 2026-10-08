//! Native MATERIAL HUD feedback in the current 3.3.3.3 playerState arrays.
//!
//! 0x5a4230 reads the two arrays through 0x5a4010/table 0xc56ac0. Type 4
//! routes to 0x515680, which resolves the actual material name through the
//! material configstring domain. This is a server-owned HUD element; the
//! received-damage directional indicator 131 is a separate path.
use crate::{game_state::Writer, lobby::LobbyError};

pub const DURATION_MSEC: u32 = 150;

#[derive(Clone, Copy, Debug, Default)]
pub struct HitFeedbackSnapshot {
    pub material: u16,
    /// Authoritative hit time, in the same clock used by the snapshot.
    pub event_time: u32,
    pub visible: bool,
}

/// Write the complete HUD extension gate and both arrays. The playerState
/// caller must select its zero baseline, as the ordinary snapshot writer does.
/// The first (PS+0x1ed0) array is empty; the active PS+0xa78 array owns one
/// short-lived, native material element. A hidden frame explicitly clears both.
pub(crate) fn write_hit_feedback(
    out: &mut Writer,
    feedback: HitFeedbackSnapshot,
) -> Result<(), LobbyError> {
    if feedback.visible && !(1..1024).contains(&feedback.material) {
        return Err(LobbyError("invalid hit feedback material"));
    }
    out.bits(1, 1); // HUD extension is present.
    out.bits(0, 5); // First array at PS+0x1ed0 is empty.
    out.bits(u32::from(feedback.visible), 5); // Active array at PS+0xa78.
    if !feedback.visible {
        return Ok(());
    }

    out.bits(15, 6); // Last changed field 14, expressed as field count.
    let mut previous = None;
    for index in [0u32, 1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14] {
        let distance = previous.map_or(index + 1, |before| index - before);
        field_gap(out, distance);
        previous = Some(index);
        match index {
            0 => color(out, 0), // White target colour with zero alpha.
            1 | 2 => {
                // -91/-92: compact signed delta from the zero HUD baseline.
                // Offsets and 24x48 dimensions follow the existing resource's
                // damage-feedback script usage; placement is runtime checked.
                out.bits(0, 1);
                out.bits(52, 7); // -12 +64.
            }
            3 => unsigned(out, 4, 4), // HE_TYPE_MATERIAL.
            4 => unsigned(out, u32::from(feedback.material), 10),
            5 => unsigned(out, 48, 10),   // Height.
            6 => unsigned(out, 24, 10),   // Width.
            7 => unsigned(out, 2047, 11), // No target entity attachment.
            10 => {
                // -97: direct 32-bit time path, avoiding a relative-time base.
                out.bits(1, 1);
                out.bytes
                    .extend_from_slice(&feedback.event_time.to_le_bytes());
            }
            11 => unsigned(out, 0x22, 8), // Screen centre/middle, local top-left.
            12 => color(out, 31),         // Opaque white starting colour.
            13 => unsigned(out, DURATION_MSEC, 16),
            14 => unsigned(out, 1, 18), // Foreground HUD pass.
            _ => unreachable!(),
        }
    }
    Ok(())
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

fn color(out: &mut Writer, alpha: u32) {
    // -85: explicit RGB branch (three direct bytes), then 5 pooled alpha bits.
    // Alpha 31 decodes to 255; other values decode to alpha*8.
    out.bits(0, 1); // Do not toggle the baseline alpha.
    out.bits(0, 1); // Explicit RGB.
    out.bits(1, 1); // Direct RGB bytes, not a palette index.
    out.bytes.extend_from_slice(&[255; 3]);
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
                    -92 | -91 => {
                        assert_eq!(r.bits(1).unwrap(), 0); // Relative7-bit path.
                        ((r.bits(7).unwrap() as i32 - 64) as f32).to_bits()
                    }
                    -97 => {
                        assert_eq!(r.bits(1).unwrap(), 1); // Absolute time.
                        r.u32().unwrap()
                    }
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
    fn native_material_hud_decodes_at_every_pool_alignment() {
        for alignment in 0..8 {
            let feedback = HitFeedbackSnapshot {
                material: 125,
                event_time: 12_345,
                visible: true,
            };
            let mut out = Writer::default();
            out.bits(0x65, alignment);
            out.bytes.push(0xb6);
            write_hit_feedback(&mut out, feedback).unwrap();
            out.bits(0x2d, 6);
            out.bytes.push(0xa7);

            let mut r = Reader::new(&out.bytes);
            assert_eq!(r.bits(alignment).unwrap(), 0x65 & ((1 << alignment) - 1));
            assert_eq!(r.u8().unwrap(), 0xb6);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert!(read_array(&mut r).is_empty());
            let current = read_array(&mut r);
            assert_eq!(current.len(), 1);
            let hud = &current[0];
            assert_eq!(hud[&0], 4);
            assert_eq!(hud[&4], (-12.0f32).to_bits());
            assert_eq!(hud[&8], (-12.0f32).to_bits());
            assert_eq!(hud[&0x10], 2047);
            assert_eq!(hud[&0x2c], 0x22);
            assert_eq!(hud[&0x30], 0x00ff_ffff);
            assert_eq!(hud[&0x34], 0xffff_ffff);
            assert_eq!(hud[&0x38], 12_345);
            assert_eq!(hud[&0x3c], 150);
            assert_eq!(hud[&0x44], 24);
            assert_eq!(hud[&0x48], 48);
            assert_eq!(hud[&0x4c], 125);
            assert_eq!(hud[&0xa4], 1);
            assert!(!hud.contains_key(&0x28)); // Zero baseline top-left origin.
            assert_eq!(r.bits(6).unwrap(), 0x2d);
            assert_eq!(r.u8().unwrap(), 0xa7);
        }
    }

    #[test]
    fn hidden_feedback_clears_both_native_arrays_without_a_material() {
        let mut out = Writer::default();
        write_hit_feedback(&mut out, HitFeedbackSnapshot::default()).unwrap();
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
                write_hit_feedback(
                    &mut out,
                    HitFeedbackSnapshot {
                        material,
                        event_time: 12_345,
                        visible: true
                    },
                )
                .is_err()
            );
            assert_eq!(out.bytes, before);
        }
    }

    #[test]
    fn absolute_time_keeps_the_full_native_u32_clock() {
        let mut out = Writer::default();
        write_hit_feedback(
            &mut out,
            HitFeedbackSnapshot {
                material: 1023,
                event_time: u32::MAX - 20,
                visible: true,
            },
        )
        .unwrap();
        let mut r = Reader::new(&out.bytes);
        assert_eq!(r.bits(1).unwrap(), 1);
        assert!(read_array(&mut r).is_empty());
        let hud = read_array(&mut r);
        assert_eq!(hud[0][&0x38], u32::MAX - 20);
        assert_eq!(hud[0][&0x4c], 1023);
    }
}
