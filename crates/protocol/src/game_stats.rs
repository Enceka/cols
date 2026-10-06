//! Target game stats upload: 0x6d80d0/0x6d81a0, independently captured on 3.3.3.3.
//! After `stats\0`: chunk u8, compressed-total LE16 (0 means raw 32768), bytes.
use crate::{lobby::LobbyError, profile::SNAPSHOT_BYTES};
use flate2::{Decompress, FlushDecompress, Status};

pub const CHUNK_BYTES: usize = 0x4d6;
const MAX_CHUNKS: usize = 27;

struct Pending {
    compressed_total: u16,
    chunks: Vec<Option<Vec<u8>>>,
}

#[derive(Default)]
pub struct Upload {
    pending: Option<Pending>,
    profile: Option<Vec<u8>>,
}

impl Upload {
    /// Returns actual missing chunks. Zero is possible only after full length
    /// and zlib validation. Upload bytes are not an authoritative profile update.
    pub fn accept(&mut self, packet: &[u8]) -> Result<u32, LobbyError> {
        if packet.len() < 3 {
            return Err(LobbyError("truncated stats chunk"));
        }
        let index = usize::from(packet[0]);
        let compressed_total = u16::from_le_bytes([packet[1], packet[2]]);
        let total = if compressed_total == 0 {
            SNAPSHOT_BYTES
        } else {
            usize::from(compressed_total)
        };
        let count = total.div_ceil(CHUNK_BYTES);
        if count > MAX_CHUNKS
            || index >= count
            || packet.len() - 3 != CHUNK_BYTES.min(total - index * CHUNK_BYTES)
        {
            return Err(LobbyError("invalid stats chunk index/length"));
        }
        if let Some(pending) = &self.pending {
            if pending.compressed_total != compressed_total {
                return Err(LobbyError("stats upload format changed"));
            }
            if let Some(previous) = &pending.chunks[index]
                && previous != &packet[3..]
            {
                return Err(LobbyError("conflicting stats chunk retry"));
            }
        }
        let pending = self.pending.get_or_insert_with(|| Pending {
            compressed_total,
            chunks: vec![None; count],
        });
        let missing = pending
            .chunks
            .iter()
            .enumerate()
            .fold(0, |mask, (i, chunk)| {
                if i != index && chunk.is_none() {
                    mask | (1 << i)
                } else {
                    mask
                }
            });
        if missing == 0 && self.profile.is_none() {
            let mut encoded = Vec::with_capacity(total);
            for (i, chunk) in pending.chunks.iter().enumerate() {
                encoded.extend_from_slice(if i == index {
                    &packet[3..]
                } else {
                    chunk.as_deref().ok_or(LobbyError("missing stats chunk"))?
                });
            }
            let plain = if compressed_total == 0 {
                encoded
            } else {
                let mut decoder = Decompress::new(true);
                let mut out = vec![0; SNAPSHOT_BYTES + 1];
                let status = decoder
                    .decompress(&encoded, &mut out, FlushDecompress::Finish)
                    .map_err(|_| LobbyError("invalid stats zlib stream"))?;
                if status != Status::StreamEnd
                    || decoder.total_in() != total as u64
                    || decoder.total_out() != SNAPSHOT_BYTES as u64
                {
                    return Err(LobbyError(
                        "stats decompression length/termination mismatch",
                    ));
                }
                out.truncate(SNAPSHOT_BYTES);
                out
            };
            self.profile = Some(plain);
        }
        pending.chunks[index] = Some(packet[3..].to_vec());
        Ok(missing)
    }

    pub fn profile(&self) -> Option<&[u8]> {
        self.profile.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn compressed(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }
    fn packet(data: &[u8], total: u16, index: usize) -> Vec<u8> {
        let mut packet = vec![index as u8];
        packet.extend_from_slice(&total.to_le_bytes());
        packet.extend_from_slice(
            &data[index * CHUNK_BYTES..data.len().min((index + 1) * CHUNK_BYTES)],
        );
        packet
    }
    #[test]
    fn reordered_raw_upload_acknowledges_only_received_chunks() {
        let raw = (0..SNAPSHOT_BYTES).map(|i| i as u8).collect::<Vec<_>>();
        let mut upload = Upload::default();
        assert_eq!(upload.accept(&packet(&raw, 0, 26)).unwrap(), (1 << 26) - 1);
        for i in (0..26).rev() {
            let part = packet(&raw, 0, i);
            let missing = upload.accept(&part).unwrap();
            assert_eq!(missing, (1 << i) - 1);
            assert_eq!(upload.accept(&part).unwrap(), missing);
        }
        assert_eq!(upload.profile().unwrap(), raw);
        let mut conflicting = packet(&raw, 0, 0);
        conflicting[4] ^= 1;
        assert!(upload.accept(&conflicting).is_err());
        assert_eq!(upload.profile().unwrap(), raw);
    }
    #[test]
    fn compressed_upload_rejects_bad_streams_expansion_and_trailing_bytes() {
        let raw = vec![19; SNAPSHOT_BYTES];
        let encoded = compressed(&raw);
        let mut upload = Upload::default();
        let valid = packet(&encoded, encoded.len() as u16, 0);
        for damaged in [
            valid[..valid.len() - 1].to_vec(),
            {
                let mut b = valid.clone();
                b[0] = 1;
                b
            },
            {
                let mut b = valid.clone();
                b[4] ^= 1;
                b
            },
        ] {
            assert!(upload.accept(&damaged).is_err());
            assert!(upload.profile().is_none());
        }
        assert_eq!(upload.accept(&valid).unwrap(), 0);
        assert_eq!(upload.profile().unwrap(), raw);
        for bytes in [
            compressed(&vec![0; SNAPSHOT_BYTES + 1]),
            compressed(&[0; 100]),
            {
                let mut b = encoded.clone();
                b.push(0);
                b
            },
        ] {
            let mut bad = Upload::default();
            assert!(bad.accept(&packet(&bytes, bytes.len() as u16, 0)).is_err());
            assert!(bad.profile().is_none());
        }
    }
}
