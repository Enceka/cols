//! Target public profile serializer/deserializer VA 0x6d7500/0x6d7740.
//! Preserve the client's complete snapshot; no old-build profile seeds.
use crate::{
    lobby::LobbyError,
    task::{Reader, Writer},
};
use flate2::{Decompress, FlushDecompress, Status};

pub const SNAPSHOT_BYTES: usize = 32768;

/// Validate the observed public 8/3 layout including the zlib stream and
/// CRC32 of bytes 4..32768. The first dword duplicates the wire CRC.
pub fn validate_public(arguments: &[u8]) -> Result<(), LobbyError> {
    let mut r = Reader::new(arguments);
    let compressed = r.u32()?;
    if compressed > 1 || r.u32()? != 1 {
        return Err(LobbyError("unsupported public profile flags/version"));
    }
    let checksum = r.u32()?;
    let blob = r.blob()?;
    let plain = if compressed == 1 {
        let mut decoder = Decompress::new(true);
        let mut out = vec![0; SNAPSHOT_BYTES + 1];
        let status = decoder
            .decompress(blob, &mut out, FlushDecompress::Finish)
            .map_err(|_| LobbyError("invalid profile zlib stream"))?;
        if status != Status::StreamEnd
            || decoder.total_out() != SNAPSHOT_BYTES as u64
            || decoder.total_in() != blob.len() as u64
        {
            return Err(LobbyError(
                "profile decompression length/termination mismatch",
            ));
        }
        out.truncate(SNAPSHOT_BYTES);
        out
    } else {
        if blob.len() != SNAPSHOT_BYTES {
            return Err(LobbyError("invalid uncompressed profile length"));
        }
        blob.to_vec()
    };
    if plain[..4] != checksum.to_le_bytes() || crc32fast::hash(&plain[4..]) != checksum {
        return Err(LobbyError("profile CRC mismatch"));
    }
    // This target's two observed public uploads carry 125 Int32 indexed fields.
    // Other projections/private profile layouts are explicitly unsupported.
    for _ in 0..125 {
        r.i32()?;
    }
    r.finish()
}

pub fn public_record(user_id: u64, arguments: &[u8]) -> Result<Vec<u8>, LobbyError> {
    validate_public(arguments)?;
    let mut out = Writer::default();
    out.u64(user_id);
    out.0.extend_from_slice(&arguments[..arguments.len() - 1]);
    Ok(out.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        include_bytes!("../../../fixtures/protocol/public-profile-synthetic.bin").to_vec()
    }
    #[test]
    fn validates_independent_zlib_crc_fixture_and_rejects_corruption() {
        let data = fixture();
        validate_public(&data).unwrap();
        let record = public_record(u64::MAX, &data).unwrap();
        assert_eq!(&record[..9], &[10, 255, 255, 255, 255, 255, 255, 255, 255]);
        assert_eq!(&record[9..], &data[..data.len() - 1]);
        for n in 0..data.len() {
            assert!(validate_public(&data[..n]).is_err());
        }
        for offset in [1, 6, 11, 21, data.len() - 6] {
            let mut bad = data.clone();
            bad[offset] ^= 0xff;
            assert!(validate_public(&bad).is_err());
        }
        let mut trailing = data.clone();
        trailing.push(0);
        assert!(validate_public(&trailing).is_err());
    }
    #[test]
    fn limits_expansion_and_rejects_trailing_compressed_data() {
        use std::io::Write;
        for expanded in [SNAPSHOT_BYTES - 1, SNAPSHOT_BYTES + 1, 1024 * 1024] {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            z.write_all(&vec![0; expanded]).unwrap();
            let b = z.finish().unwrap();
            let mut w = Writer::default();
            w.u32(1);
            w.u32(1);
            w.u32(0);
            w.blob(&b).unwrap();
            assert!(validate_public(&w.0).is_err());
        }
        let mut bad = fixture();
        let n = u32::from_le_bytes(bad[17..21].try_into().unwrap()) as usize;
        bad.insert(21 + n, 0);
        bad[17..21].copy_from_slice(&((n + 1) as u32).to_le_bytes());
        assert!(validate_public(&bad).is_err());
    }
}
