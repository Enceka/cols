//! Native glass tail, parser0x43e070. Piece IDs are cbrush+2 minus one.
//! The client's state is monotonic and independent of entity delta baselines.
use crate::game::gamestate::Writer;
use crate::lobby::channel::LobbyError;

pub const MAX_PIECES: u16 = 1024;

/// Resend all destroyed pieces in every frame, including full/replay frames.
/// State2 with no hit information is a supported native shatter operation.
pub(crate) fn write(out: &mut Writer, destroyed: &[u16]) -> Result<(), LobbyError> {
    for (i, &piece) in destroyed.iter().enumerate() {
        if piece >= MAX_PIECES || (i > 0 && destroyed[i - 1] >= piece) {
            return Err(LobbyError(
                "glass pieces must be bounded, sorted and unique",
            ));
        }
    }
    out.bits(u32::from(!destroyed.is_empty()), 1);
    if destroyed.is_empty() {
        return Ok(());
    }
    out.bits(1, 1); // sparse format
    for &piece in destroyed {
        out.bits(u32::from(piece), 11);
        out.bits(2, 2); // shattered
        out.bits(0, 1); // no hit information: native0xff sentinel
    }
    out.bits(u32::from(MAX_PIECES), 11);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::channel::wire::Reader;
    use crate::game::channel::wire::SERVER_EOF;

    // Decode in the order of the recovered client parser, independently of
    // the writer. Native raw bytes and bit fields use separate cursors.
    fn receive(reader: &mut Reader<'_>, states: &mut [u8; 1024]) -> Result<(), LobbyError> {
        if reader.bits(1)? == 0 {
            return Ok(());
        }
        assert_eq!(reader.bits(1)?, 1);
        loop {
            let piece = reader.bits(11)? as usize;
            if piece == 1024 {
                return Ok(());
            }
            assert!(piece < 1024);
            let state = reader.bits(2)? as u8;
            if state == 2 {
                assert_eq!(reader.bits(1)?, 0);
            }
            states[piece] = states[piece].max(state);
        }
    }

    #[test]
    fn native_sparse_glass_tail_preserves_eof_at_every_bit_alignment() {
        for prefix in 0..8 {
            for pieces in [vec![], vec![0, 168, 1023], (0..1024).collect()] {
                let mut out = Writer::default();
                out.bits(0, prefix);
                write(&mut out, &pieces).unwrap();
                out.bytes.push(SERVER_EOF);
                let mut reader = Reader::new(&out.bytes);
                reader.bits(prefix).unwrap();
                let mut states = [0; 1024];
                receive(&mut reader, &mut states).unwrap();
                assert_eq!(reader.u8().unwrap(), SERVER_EOF);
                for (piece, &state) in states.iter().enumerate() {
                    assert_eq!(
                        state,
                        if pieces.contains(&(piece as u16)) {
                            2
                        } else {
                            0
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn repeated_full_delta_and_replay_delivery_keeps_monotonic_destruction() {
        let mut states = [0; 1024];
        for pieces in [&[4][..], &[4, 9][..], &[][..], &[4, 9][..], &[4, 9][..]] {
            let mut out = Writer::default();
            write(&mut out, pieces).unwrap();
            receive(&mut Reader::new(&out.bytes), &mut states).unwrap();
            assert_eq!(states[4], 2);
        }
        assert_eq!(states[9], 2);
        assert_eq!(states[5], 0);
    }

    #[test]
    fn malformed_glass_ids_are_rejected_before_writing() {
        for pieces in [&[1024][..], &[2047][..], &[3, 3][..], &[9, 4][..]] {
            let mut out = Writer::default();
            assert!(write(&mut out, pieces).is_err());
            assert!(out.bytes.is_empty());
        }
    }

    #[test]
    fn truncated_native_glass_tail_cannot_complete_with_an_eof() {
        let mut out = Writer::default();
        write(&mut out, &[0, 168, 1023]).unwrap();
        out.bytes.push(SERVER_EOF);
        for length in 0..out.bytes.len() {
            let mut reader = Reader::new(&out.bytes[..length]);
            let result = receive(&mut reader, &mut [0; 1024]).and_then(|()| reader.u8());
            assert!(result.is_err(), "truncated length {length}");
        }
    }
}
