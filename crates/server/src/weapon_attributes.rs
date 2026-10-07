//! Numeric portion of the target 3.3.3.3 WeaponDef attachment composer.
//!
//! Recovered independently from executable SHA256
//! e3823fd6afc80fb25fa84747e6e9634707b9a0b189357a5466921cda354af37b:
//! 0x42ba70 collects records, 0x42ba40 orders numeric records, 0x42b380
//! dispatches value kinds, 0x42b270/0x42b300 evaluate, and 0x42af20 derives
//! ADS rates. See research-output/ghidra/loadout-attributes*.c.
//!
//! This does not resolve process-local string/asset pointers. Those fields
//! remain as in `base`; conflicts with numeric fields are errors. The result
//! is an image for reading gameplay scalars, not a client-loadable asset.
use anyhow::{Result, bail, ensure};

pub(crate) const WEAPON_DEF_SIZE: usize = 0xe28;
const ADS_IN_MSEC: usize = 0xad0;
const ADS_OUT_MSEC: usize = 0xad4;
const ADS_IN_RATE: usize = 0xbac;
const ADS_OUT_RATE: usize = 0xbb0;

#[derive(Clone, Copy, Debug)]
struct Record {
    kind: u8,
    operation: u8,
    offset: usize,
    operand: u32,
}

impl Record {
    fn parse(bytes: &[u8; 12]) -> Self {
        Self {
            kind: bytes[0],
            operation: bytes[1],
            offset: u16::from_le_bytes([bytes[4], bytes[5]]) as usize,
            operand: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        }
    }

    fn numeric_operation(self) -> bool {
        (6..=9).contains(&self.operation)
    }

    fn scalar(self) -> bool {
        (4..=9).contains(&self.kind) || (self.kind >= 16 && !matches!(self.kind, 39 | 42 | 43))
    }

    fn width(self) -> usize {
        if self.kind == 6 { 1 } else { 4 }
    }

    fn ignored_write(self) -> (usize, usize) {
        match self.kind {
            // Special parsers use fixed tag/notify arrays, ignoring offset.
            39 => (0x38, 0x40),
            42 => (0x398, 0x80),
            43 => (0x418, 0x60),
            _ => (self.offset, 4),
        }
    }
}

fn overlaps(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < b.0 + b.1 && b.0 < a.0 + a.1
}

// Fields consumed by the currently recovered ammo/fire/reload/ADS paths.
// Protect these even if no attachment happens to write a numeric override.
const GAMEPLAY_FIELDS: &[(usize, usize)] = &[
    (0x47c, 4),
    (0x488, 4),
    (0x48c, 4),
    (0x6b0, 1),
    (0x6b8, 4),
    (0x6c0, 4),
    (0x6c4, 4),
    (0x6d0, 4),
    (0x6d4, 4),
    (0x6f0, 4),
    (0x700, 4),
    (0x704, 4),
    (0x708, 4),
    (0x70c, 4),
    (0x71c, 4),
    (0x720, 4),
    (0x728, 4),
    (0x72c, 4),
    (0x730, 4),
    (0x734, 4),
    (0x738, 4),
    (0x73c, 4),
    (0x740, 4),
    (0x744, 4),
    (0x7ac, 4),
    (0x8c0, 4),
    (0x934, 1),
    (0x938, 4),
    (0xa7c, 4),
    (ADS_IN_MSEC, 4),
    (ADS_OUT_MSEC, 4),
    (0xad8, 4),
    (0xb38, 4),
    (ADS_IN_RATE, 4),
    (ADS_OUT_RATE, 4),
    (0xd08, 4),
    (0xd10, 4),
    (0xd24, 4),
    (0xd68, 1),
    (0xd69, 1),
    (0xd74, 1),
    (0xd75, 1),
    (0xd76, 1),
    (0xd77, 1),
    (0xd78, 1),
    (0xd7b, 1),
    (0xd7d, 1),
    (0xd83, 1),
    (0xd85, 1),
    (0xd86, 1),
    (0xd9f, 1),
    (0xdac, 1),
];

fn read_word(image: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(image[offset..offset + 4].try_into().unwrap())
}

fn write_word(image: &mut [u8], offset: usize, word: u32) {
    image[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
}

fn compare(a: &Record, b: &Record) -> std::cmp::Ordering {
    (a.offset, a.operation).cmp(&(b.offset, b.operation))
}

/// Target CRT qsort, including the unstable ordering of equal keys.
/// This is an independent index-based translation of 0xaf7330/0xaf72a0.
fn target_sort(records: &mut [Record]) {
    if records.len() < 2 {
        return;
    }
    let mut pending = vec![(0isize, records.len() as isize - 1)];
    while let Some((mut low, mut high)) = pending.pop() {
        while low < high {
            if high - low < 8 {
                while low < high {
                    let mut maximum = low;
                    for index in low + 1..=high {
                        if compare(&records[index as usize], &records[maximum as usize]).is_gt() {
                            maximum = index;
                        }
                    }
                    records.swap(maximum as usize, high as usize);
                    high -= 1;
                }
                break;
            }
            let mut pivot = low + (high - low + 1) / 2;
            for (a, b) in [(low, pivot), (low, high), (pivot, high)] {
                if compare(&records[a as usize], &records[b as usize]).is_gt() {
                    records.swap(a as usize, b as usize);
                }
            }
            let (mut left, mut right) = (low, high);
            loop {
                if left < pivot {
                    loop {
                        left += 1;
                        if left >= pivot
                            || compare(&records[left as usize], &records[pivot as usize]).is_gt()
                        {
                            break;
                        }
                    }
                }
                if left >= pivot {
                    loop {
                        left += 1;
                        if left > high
                            || compare(&records[left as usize], &records[pivot as usize]).is_gt()
                        {
                            break;
                        }
                    }
                }
                loop {
                    right -= 1;
                    if right <= pivot
                        || !compare(&records[right as usize], &records[pivot as usize]).is_gt()
                    {
                        break;
                    }
                }
                if left > right {
                    break;
                }
                records.swap(left as usize, right as usize);
                if pivot == right {
                    pivot = left;
                }
            }
            right += 1;
            if pivot < right {
                loop {
                    right -= 1;
                    if right <= pivot
                        || !compare(&records[right as usize], &records[pivot as usize]).is_eq()
                    {
                        break;
                    }
                }
            }
            if pivot >= right {
                loop {
                    right -= 1;
                    if right <= low
                        || !compare(&records[right as usize], &records[pivot as usize]).is_eq()
                    {
                        break;
                    }
                }
            }
            // The target processes the smaller partition before the larger.
            if right - low >= high - left {
                if low < right {
                    pending.push((low, right));
                }
                low = left;
            } else {
                if left < high {
                    pending.push((left, high));
                }
                high = right;
            }
        }
    }
}

fn apply_scalar(base: &[u8], image: &mut [u8], record: Record) -> Result<()> {
    let offset = record.offset;
    if record.kind >= 16 {
        write_word(image, offset, record.operand);
        return Ok(());
    }
    if record.kind == 5 {
        write_word(image, offset, (record.operand & 255) * 0x01010101);
        return Ok(());
    }
    if record.kind == 6 {
        image[offset] = u8::from(record.operand != 0);
        return Ok(());
    }
    let integer = matches!(record.kind, 4 | 9);
    let decode = |word: u32| {
        if integer {
            (word as i32) as f32
        } else {
            f32::from_bits(word)
        }
    };
    let original = decode(read_word(base, offset));
    let current = decode(read_word(image, offset));
    let mut operand = f32::from_bits(record.operand);
    ensure!(
        operand.is_finite(),
        "nonfinite attachment operand at {offset:#x}"
    );
    if !matches!(record.operation, 8 | 9) {
        let multiplier = match record.kind {
            8 => 17.600000381469727f64,
            9 => 1000.0,
            _ => 1.0,
        };
        operand = (f64::from(operand) * multiplier) as f32;
    }
    let result = match record.operation {
        6 => operand,
        7 => current + operand,
        // x87 has no intervening float32 store between multiply and add.
        // Fused rounding covers finite asset-scale inputs; extreme x87
        // precision-control/double-rounding cases are not runtime certified.
        8 => original.mul_add(operand, current),
        9 => current * operand,
        _ => unreachable!("validated numeric operation"),
    };
    ensure!(
        result.is_finite(),
        "nonfinite composed value at {offset:#x}"
    );
    let word = if integer {
        // Rust casts saturate; target CVTTSD2SI does not. Reject invalid
        // values rather than changing the target's result silently.
        ensure!(
            (-2147483648.0f64..2147483648.0).contains(&f64::from(result)),
            "attachment integer overflow at {offset:#x}"
        );
        (result.trunc() as i32) as u32
    } else {
        result.to_bits()
    };
    write_word(image, offset, word);
    Ok(())
}

/// Compose all supported numeric properties in original attachment/record
/// order. `base` is the unmodified 0xe28-byte target WeaponDef; record values
/// are captured bits, not preconverted numbers or offsets into `base`.
///
/// String/asset writes are left untouched, checked for overlap with supported
/// scalar writes and known gameplay fields. Unresolved string precedence
/// cannot choose between conflicting raw scalar sets: that ambiguity is an
/// error. No process address in either input is dereferenced.
pub(crate) fn compose(base: &[u8], records: &[[u8; 12]]) -> Result<Vec<u8>> {
    ensure!(
        base.len() == WEAPON_DEF_SIZE,
        "target WeaponDef must be 0xe28 bytes"
    );
    let parsed: Vec<_> = records.iter().map(Record::parse).collect();
    let mut numeric = Vec::new();
    let mut raw = Vec::new();
    let mut ignored = Vec::new();
    let mut nonnumeric_count = 0;
    for &record in &parsed {
        ensure!(
            record.operation <= 9,
            "unknown attachment operation {}",
            record.operation
        );
        if !record.numeric_operation() {
            nonnumeric_count += 1;
        }
        if !record.scalar() {
            ensure!(
                !record.numeric_operation(),
                "numeric operation on unresolved attachment kind {}",
                record.kind
            );
            let range = record.ignored_write();
            ensure!(
                range.0 + range.1 <= base.len(),
                "attachment reference field out of bounds"
            );
            ignored.push(record);
            continue;
        }
        ensure!(
            record.offset + record.width() <= base.len(),
            "attachment scalar field out of bounds"
        );
        if record.numeric_operation() {
            numeric.push(record);
        } else {
            ensure!(
                record.kind >= 16 && matches!(record.operation, 0 | 3),
                "unresolved string operation {} on scalar kind {} at {:#x}",
                record.operation,
                record.kind,
                record.offset
            );
            raw.push(record);
        }
    }
    ensure!(
        numeric.len() <= 256 && nonnumeric_count <= 256,
        "target attachment record capacity exceeded"
    );
    for &reference in &ignored {
        let range = reference.ignored_write();
        ensure!(
            !GAMEPLAY_FIELDS.iter().any(|&field| overlaps(range, field)),
            "unresolved attachment reference overlaps gameplay scalar at {:#x}",
            range.0
        );
        ensure!(
            !numeric
                .iter()
                .chain(&raw)
                .any(|r| overlaps(range, (r.offset, r.width()))),
            "unresolved attachment reference overlaps scalar override at {:#x}",
            range.0
        );
        ensure!(
            !raw.iter().any(|r| r.offset == reference.offset),
            "unresolved attachment group can suppress scalar override at {:#x}",
            reference.offset
        );
    }
    // Nonnumeric qsort compares strings for operations 1/2/4/5. Missing
    // those strings can alter equal-key ordering. Disallow only an outcome
    // that would require that ordering, rather than inventing a winner.
    let unresolved_comparison = ignored.iter().any(|r| matches!(r.operation, 1 | 2 | 4 | 5));
    if unresolved_comparison {
        for (index, a) in raw.iter().enumerate() {
            if raw[index + 1..]
                .iter()
                .any(|b| compare(a, b).is_eq() && a.operand != b.operand)
            {
                bail!(
                    "conflicting raw scalar sets require unresolved string ordering at {:#x}",
                    a.offset
                );
            }
        }
    }
    // Include ignored records: their positions participate in CRT sorting.
    let mut nonnumeric: Vec<_> = parsed
        .iter()
        .copied()
        .filter(|r| !r.numeric_operation())
        .collect();
    target_sort(&mut nonnumeric);
    let mut image = base.to_vec();
    let mut cursor = 0;
    while cursor < nonnumeric.len() {
        let first = nonnumeric[cursor];
        if first.scalar() {
            // Supported raw records have op0/op3. 42b940 applies the first
            // and skips the remaining set records for that same offset.
            apply_scalar(base, &mut image, first)?;
        }
        cursor += 1;
        while cursor < nonnumeric.len() && nonnumeric[cursor].offset == first.offset {
            cursor += 1;
        }
    }
    target_sort(&mut numeric);
    for record in numeric {
        apply_scalar(base, &mut image, record)?;
    }
    // 42af20 only derives these two ADS rates. Fire interval +0xd10 is an
    // ordinary scalar; this function does not invent another derivation.
    for (time_offset, rate_offset, default_bits) in [
        (ADS_IN_MSEC, ADS_IN_RATE, 0x3b5a740e),
        (ADS_OUT_MSEC, ADS_OUT_RATE, 0x3b03126f),
    ] {
        let msec = read_word(&image, time_offset) as i32;
        let bits = if msec > 0 {
            (1.0f64 / f64::from(msec)) as f32
        } else {
            f32::from_bits(default_bits)
        }
        .to_bits();
        write_word(&mut image, rate_offset, bits);
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Vec<u8> {
        let mut b = vec![0; WEAPON_DEF_SIZE];
        for (offset, value) in [
            (0xd08, 30),
            (0xd10, 80),
            (0x71c, 2000),
            (0x728, 2800),
            (ADS_IN_MSEC, 250),
            (ADS_OUT_MSEC, 300),
        ] {
            write_word(&mut b, offset, value);
        }
        b
    }

    fn raw(kind: u8, operation: u8, offset: usize, operand: u32) -> [u8; 12] {
        let mut r = [0; 12];
        r[0] = kind;
        r[1] = operation;
        r[4..6].copy_from_slice(&(offset as u16).to_le_bytes());
        r[8..].copy_from_slice(&operand.to_le_bytes());
        r
    }

    fn number(kind: u8, operation: u8, offset: usize, operand: f32) -> [u8; 12] {
        raw(kind, operation, offset, operand.to_bits())
    }

    #[test]
    fn combines_real_field_units_and_operation_precedence_without_mutating_base() {
        let b = base();
        let composed = compose(
            &b,
            &[
                number(9, 9, 0x71c, 0.5),
                number(9, 8, 0x71c, 0.25),
                number(9, 7, 0x71c, 0.1),
                number(9, 6, 0x71c, 3.0),
                number(4, 7, 0xd08, 15.0),
                number(9, 9, 0xd10, 0.75),
                number(9, 6, ADS_IN_MSEC, 0.2),
            ],
        )
        .unwrap();
        assert_eq!(read_word(&composed, 0x71c), 1800); // ((3000+100)+2000*.25)*.5
        assert_eq!(read_word(&composed, 0xd08), 45);
        assert_eq!(read_word(&composed, 0xd10), 60);
        assert_eq!(
            read_word(&composed, ADS_IN_RATE),
            (1.0f32 / 200.0).to_bits()
        );
        assert_eq!(read_word(&b, 0x71c), 2000);
    }

    #[test]
    fn scalar_operand_is_float_bits_and_each_integer_step_truncates() {
        let mut b = base();
        write_word(&mut b, 0xd08, 10);
        let composed = compose(&b, &[number(4, 9, 0xd08, 1.5), number(4, 9, 0xd08, 0.19)]).unwrap();
        // CRT shortsort swaps the equal records: trunc(trunc(10*.19)*1.5).
        // Input-order evaluation would incorrectly produce 2.
        assert_eq!(read_word(&composed, 0xd08), 1);
        let negative = compose(&b, &[number(4, 6, 0xd08, -1.9)]).unwrap();
        assert_eq!(read_word(&negative, 0xd08) as i32, -1);
    }

    #[test]
    fn float_distance_boolean_and_raw_enum_kinds_are_distinct() {
        let b = base();
        let composed = compose(
            &b,
            &[
                number(8, 6, 0x900, 2.0),
                number(8, 9, 0x900, 0.5),
                raw(5, 6, 0x904, 0x12345602),
                raw(6, 6, 0xd76, 0x100),
                raw(16, 0, 0x47c, 2),
            ],
        )
        .unwrap();
        assert_eq!(read_word(&composed, 0x900), 17.6f32.to_bits());
        assert_eq!(read_word(&composed, 0x904), 0x02020202);
        assert_eq!(composed[0xd76], 1);
        assert_eq!(read_word(&composed, 0x47c), 2);
    }

    #[test]
    fn derives_ads_after_overrides_and_uses_target_nonpositive_defaults() {
        let b = base();
        let composed = compose(
            &b,
            &[
                number(7, 6, ADS_IN_RATE, 1.0),
                number(4, 6, ADS_IN_MSEC, 0.0),
                number(4, 6, ADS_OUT_MSEC, -1.0),
            ],
        )
        .unwrap();
        assert_eq!(read_word(&composed, ADS_IN_RATE), 0x3b5a740e);
        assert_eq!(read_word(&composed, ADS_OUT_RATE), 0x3b03126f);
        assert_eq!(read_word(&composed, 0xd10), 80);
    }

    #[test]
    fn base_relative_float_add_rounds_only_after_multiply_and_add() {
        let mut b = base();
        let original = f32::from_bits(0x3f800001); // 1 + 2^-23
        write_word(&mut b, 0x900, original.to_bits());
        let composed = compose(
            &b,
            &[
                number(7, 6, 0x900, -f32::from_bits(0x3f800002)),
                number(7, 8, 0x900, original),
            ],
        )
        .unwrap();
        assert_eq!(
            f32::from_bits(read_word(&composed, 0x900)),
            2.0f32.powi(-46)
        );
    }

    #[test]
    fn ignores_unresolved_art_but_rejects_scalar_aliasing_and_unsupported_ops() {
        let b = base();
        let composed = compose(
            &b,
            &[raw(1, 3, 0x80, 0xfffffff0), raw(39, 0, 0, 0xfffffff0)],
        )
        .unwrap();
        assert_eq!(&composed[0x38..0x84], &b[0x38..0x84]);
        for records in [
            vec![raw(1, 3, 0xd10, 123)],
            vec![raw(1, 3, 0x900, 123), number(7, 6, 0x902, 1.0)],
            vec![raw(42, 0, 0, 123), number(7, 6, 0x400, 1.0)],
            vec![number(4, 3, 0xd08, 30.0)],
            vec![raw(1, 6, 0x80, 123)],
            vec![raw(4, 10, 0xd08, 123)],
            vec![number(4, 6, WEAPON_DEF_SIZE - 2, 1.0)],
            vec![number(4, 6, 0xd08, f32::NAN)],
            vec![number(4, 6, 0xd08, 2147483648.0)],
        ] {
            assert!(compose(&b, &records).is_err(), "{records:?}");
        }
        assert!(compose(&b[..b.len() - 1], &[]).is_err());
    }

    #[test]
    fn target_sort_handles_both_partitions_and_preserves_the_record_multiset() {
        let mut seed = 1u32;
        for count in 0..=256 {
            let mut records: Vec<_> = (0..count)
                .map(|id| {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    Record {
                        kind: 4,
                        operation: 6 + ((seed >> 8) % 4) as u8,
                        offset: (seed % 17) as usize * 4,
                        operand: id as u32,
                    }
                })
                .collect();
            target_sort(&mut records);
            assert!(records.windows(2).all(|r| !compare(&r[0], &r[1]).is_gt()));
            let mut ids: Vec<_> = records.iter().map(|r| r.operand).collect();
            ids.sort_unstable();
            assert_eq!(ids, (0..count as u32).collect::<Vec<_>>());
        }
    }
}
