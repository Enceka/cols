//! Target public profile serializer/deserializer VA 0x6d7500/0x6d7740.
//! Preserve the client's complete snapshot; no old-build profile seeds.
use crate::lobby::channel::LobbyError;
use crate::lobby::task::Reader;
use crate::lobby::task::Writer;
use flate2::{Decompress, FlushDecompress, Status};
use std::io::Write;

pub const SNAPSHOT_BYTES: usize = 32768;

/// Validate the observed public 8/3 layout including the zlib stream and
/// CRC32 of bytes 4..32768. The first dword duplicates the wire CRC.
fn snapshot_and_tail(arguments: &[u8]) -> Result<(Vec<u8>, Reader<'_>), LobbyError> {
    let mut r = Reader::new(arguments);
    let compressed = r.u32()?;
    if compressed > 1 || r.u32()? != 1 {
        return Err(LobbyError("unsupported profile flags/version"));
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
    Ok((plain, r))
}

pub fn validate_public(arguments: &[u8]) -> Result<(), LobbyError> {
    let (_, mut r) = snapshot_and_tail(arguments)?;
    // This target's two observed public uploads carry 125 Int32 indexed fields.
    // Other projections/private profile layouts are explicitly unsupported.
    for _ in 0..125 {
        r.i32()?;
    }
    r.finish()
}

/// Decode an intact saved public profile using projection offsets recovered
/// from this build's StructuredDataDef. Do not use the obfuscated live buffer.
pub fn public_snapshot(arguments: &[u8], projections: &[usize]) -> Result<Vec<u8>, LobbyError> {
    if projections.len() != 125 {
        return Err(LobbyError("public projection count"));
    }
    let (plain, mut tail) = snapshot_and_tail(arguments)?;
    for &offset in projections {
        if !(12..=SNAPSHOT_BYTES - 4).contains(&offset)
            || plain[offset..offset + 4] != tail.i32()?.to_le_bytes()
        {
            return Err(LobbyError("public projection disagrees with snapshot"));
        }
    }
    tail.finish()?;
    Ok(plain)
}

/// mp/playersecuredata.def v19: 36 projected leaves, in target walk order.
/// Addresses come from the runtime offset table; tag 13 is Float32.
const PRIVATE_PROJECTIONS: &[(u8, usize)] = &[
    (7, 12),
    (7, 676),
    (7, 680),
    (7, 684),
    (7, 4820),
    (7, 4824),
    (7, 4828),
    (7, 4832),
    (7, 4836),
    (7, 4840),
    (7, 4844),
    (7, 4848),
    (7, 4852),
    (7, 4856),
    (7, 4860),
    (13, 4864),
    (7, 4868),
    (7, 4872),
    (7, 4883),
    (7, 4887),
    (7, 4891),
    (7, 4895),
    (7, 4899),
    (7, 4903),
    (7, 4907),
    (7, 4911),
    (7, 4915),
    (7, 4919),
    (7, 4923),
    (7, 4927),
    (7, 4931),
    (7, 4935),
    (7, 4939),
    (7, 4951),
    (7, 4955),
    (13, 4959),
];

pub fn validate_private(arguments: &[u8]) -> Result<(), LobbyError> {
    let (plain, mut r) = snapshot_and_tail(arguments)?;
    for &(tag, offset) in PRIVATE_PROJECTIONS {
        let bits = if tag == 13 {
            r.f32_bits()?
        } else {
            r.i32()? as u32
        };
        if plain[offset..offset + 4] != bits.to_le_bytes() {
            return Err(LobbyError(
                "private profile projection disagrees with snapshot",
            ));
        }
    }
    r.finish()
}

pub fn private_record(user_id: u64, arguments: &[u8]) -> Result<Vec<u8>, LobbyError> {
    validate_private(arguments)?;
    let mut out = Writer::default();
    out.u64(user_id);
    out.0.extend_from_slice(&arguments[..arguments.len() - 1]);
    Ok(out.0)
}

/// Presentation values from the saved, owner-checked private profile.
/// rankVersion is a table version, never the prestige displayed on a card.
pub fn player_rank(arguments: &[u8], user_id: u64) -> Result<(u8, u8), LobbyError> {
    validate_private(arguments)?;
    let (plain, _) = snapshot_and_tail(arguments)?;
    if plain[4943..4947] != ((user_id >> 32) as u32).to_le_bytes()
        || plain[4947..4951] != (user_id as u32).to_le_bytes()
    {
        return Err(LobbyError("private profile owner mismatch"));
    }
    let rank = u32::from_le_bytes(plain[4887..4891].try_into().unwrap());
    let prestige = u32::from_le_bytes(plain[4883..4887].try_into().unwrap());
    if !(1..=70).contains(&rank) || prestige > 255 {
        return Err(LobbyError("private profile rank out of range"));
    }
    Ok(((rank - 1) as u8, prestige as u8))
}

/// Stored rank is one-based; experience is XP within that rank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerProgress {
    pub rank: u8,
    pub experience: u32,
    pub rank_version: u32,
}

pub fn player_progress(arguments: &[u8], user_id: u64) -> Result<PlayerProgress, LobbyError> {
    let (rank, _) = player_rank(arguments, user_id)?;
    let (plain, _) = snapshot_and_tail(arguments)?;
    let experience = i32::from_le_bytes(plain[4844..4848].try_into().unwrap());
    let rank_version = u32::from_le_bytes(plain[4891..4895].try_into().unwrap());
    if experience < 0 {
        return Err(LobbyError("negative private profile experience"));
    }
    Ok(PlayerProgress {
        rank: rank + 1,
        experience: experience as u32,
        rank_version,
    })
}

pub fn player_rank_unlock(arguments: &[u8], user_id: u64) -> Result<i16, LobbyError> {
    player_rank(arguments, user_id)?;
    let (plain, _) = snapshot_and_tail(arguments)?;
    Ok(i16::from_le_bytes(plain[8297..8299].try_into().unwrap()))
}

/// Index of the first `unlockFlagsNew` entry in the private snapshot.
const UNLOCK_BASE: usize = 8295;
const UNLOCK_COUNT: u16 = 256;

/// Read one story unlock flag from the private snapshot.
pub fn story_unlock(arguments: &[u8], user_id: u64, flag: u16) -> Result<i16, LobbyError> {
    player_rank(arguments, user_id)?;
    if flag >= UNLOCK_COUNT {
        return Err(LobbyError("private profile unlock flag out of range"));
    }
    let (plain, _) = snapshot_and_tail(arguments)?;
    let at = UNLOCK_BASE + usize::from(flag) * 2;
    Ok(i16::from_le_bytes(plain[at..at + 2].try_into().unwrap()))
}

/// Raise one story unlock flag. Unrelated progress survives, and the CRC is
/// rebuilt only when the flag actually changes.
pub fn with_story_unlock(arguments: &[u8], user_id: u64, flag: u16) -> Result<Vec<u8>, LobbyError> {
    if story_unlock(arguments, user_id, flag)? >= 1 {
        return Ok(arguments.to_vec());
    }
    let (mut plain, _) = snapshot_and_tail(arguments)?;
    let at = UNLOCK_BASE + usize::from(flag) * 2;
    plain[at..at + 2].copy_from_slice(&1i16.to_le_bytes());
    encode_private(plain)
}

/// Prepare an owner-checked v4 level update in memory. The caller must derive
/// rank/within-rank XP from the verified curve and persist it authoritatively.
/// Prestige and unrelated progress survive; CRC and all projections are rebuilt.
pub fn with_player_progress(
    arguments: &[u8],
    user_id: u64,
    rank: u8,
    experience: u32,
) -> Result<Vec<u8>, LobbyError> {
    let old = player_progress(arguments, user_id)?;
    if old.rank_version != 4 && !(old.rank_version == 0 && old.rank == 1 && old.experience == 0) {
        return Err(LobbyError("unsupported private profile rank-table version"));
    }
    if !(1..=70).contains(&rank) || experience > i32::MAX as u32 {
        return Err(LobbyError("private profile progression out of range"));
    }
    let (mut plain, _) = snapshot_and_tail(arguments)?;
    plain[4844..4848].copy_from_slice(&experience.to_le_bytes());
    plain[4887..4891].copy_from_slice(&u32::from(rank).to_le_bytes());
    plain[4891..4895].copy_from_slice(&4u32.to_le_bytes());
    let unlocked =
        i16::from_le_bytes(plain[8297..8299].try_into().unwrap()).max(i16::from(rank - 1));
    plain[8297..8299].copy_from_slice(&unlocked.to_le_bytes());
    encode_private(plain)
}

/// Create a clean local private profile at the selected displayed level.
pub fn new_private(user_id: u64, rank: u8) -> Result<Vec<u8>, LobbyError> {
    if !(1..=70).contains(&rank) {
        return Err(LobbyError("private profile rank out of range"));
    }
    let mut blank = vec![0; SNAPSHOT_BYTES];
    let crc = crc32fast::hash(&blank[4..]);
    blank[..4].copy_from_slice(&crc.to_le_bytes());
    let seed = prepare_private(&blank, user_id, true)?;
    let (mut plain, _) = snapshot_and_tail(&seed)?;
    plain[4887..4891].copy_from_slice(&u32::from(rank).to_le_bytes());
    plain[4891..4895].copy_from_slice(&4u32.to_le_bytes());
    plain[8297..8299].copy_from_slice(&i16::from(rank - 1).to_le_bytes());
    encode_private(plain)
}

/// Prepare a local level-one private profile from an intact client-default
/// snapshot. Preserve all unrelated bytes, including the observed zero header.
pub fn prepare_private(
    default: &[u8],
    user_id: u64,
    training_complete: bool,
) -> Result<Vec<u8>, LobbyError> {
    if default.len() != SNAPSHOT_BYTES
        || default[..4] != crc32fast::hash(&default[4..]).to_le_bytes()
    {
        return Err(LobbyError("default private snapshot size/CRC invalid"));
    }
    if default[4..].iter().any(|b| *b != 0) {
        return Err(LobbyError(
            "expected a fresh private default; refusing to reset existing progress",
        ));
    }
    let mut plain = default.to_vec();
    plain[4887..4891].copy_from_slice(&1i32.to_le_bytes()); // rank
    plain[4943..4947].copy_from_slice(&((user_id >> 32) as u32).to_le_bytes());
    plain[4947..4951].copy_from_slice(&(user_id as u32).to_le_bytes());
    if training_complete {
        // mp/unlockFlags.csv: uf_mp_cqbtraining = 141. Reader VA 0x5b3430
        // checks unlockFlagsNew[141] >= 1 via VA 0x4a4ca0.
        plain[8577..8579].copy_from_slice(&1i16.to_le_bytes());
    }
    encode_private(plain)
}

/// Local testing only: target rankTable v4 ends at displayed level 70.
/// VA 0x590720 returns rank-1; VA 0x590550 adds experience to the rank's
/// cumulative threshold, so experience is WITHIN the rank, not lifetime XP.
pub fn maximize_test_rank(arguments: &[u8], user_id: u64) -> Result<Vec<u8>, LobbyError> {
    validate_private(arguments)?;
    let (mut plain, _) = snapshot_and_tail(arguments)?;
    if plain[4943..4947] != ((user_id >> 32) as u32).to_le_bytes()
        || plain[4947..4951] != (user_id as u32).to_le_bytes()
    {
        return Err(LobbyError("private profile owner mismatch"));
    }
    plain[4844..4848].copy_from_slice(&0i32.to_le_bytes());
    plain[4887..4891].copy_from_slice(&70i32.to_le_bytes());
    plain[4891..4895].copy_from_slice(&4i32.to_le_bytes());
    // uiNotifications un_room_list/un_mp_room_creation require
    // uf_prestige_rank >= 4, not the displayed rank (VA 0x5c63b0).
    // unlockFlags.csv maps it to unlockFlagsNew[1], an Int16 at 8297.
    // Raise the ordinary level component to 69 without losing a higher value.
    let unlocked = i16::from_le_bytes([plain[8297], plain[8298]]).max(69);
    plain[8297..8299].copy_from_slice(&unlocked.to_le_bytes());
    encode_private(plain)
}

fn encode_private(mut plain: Vec<u8>) -> Result<Vec<u8>, LobbyError> {
    let crc = crc32fast::hash(&plain[4..]);
    plain[..4].copy_from_slice(&crc.to_le_bytes());
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder
        .write_all(&plain)
        .map_err(|_| LobbyError("profile compression failed"))?;
    let blob = encoder
        .finish()
        .map_err(|_| LobbyError("profile compression failed"))?;
    let mut out = Writer::default();
    out.u32(1);
    out.u32(1);
    out.u32(crc);
    out.blob(&blob)?;
    for &(tag, offset) in PRIVATE_PROJECTIONS {
        let bits = u32::from_le_bytes(
            plain[offset..offset + 4]
                .try_into()
                .map_err(|_| LobbyError("projection bounds"))?,
        );
        if tag == 13 {
            out.f32_bits(bits);
        } else {
            out.i32(bits as i32);
        }
    }
    out.0.push(0);
    validate_private(&out.0)?;
    Ok(out.0)
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
    #[test]
    fn fresh_private_profiles_cover_all_levels_without_inheriting_progress() {
        for level in 1..=70 {
            let bytes = new_private(123, level).unwrap();
            assert_eq!(player_rank(&bytes, 123).unwrap(), (level - 1, 0));
            let (plain, _) = snapshot_and_tail(&bytes).unwrap();
            assert_eq!(&plain[4844..4848], &0u32.to_le_bytes());
            assert_eq!(&plain[4891..4895], &4u32.to_le_bytes());
            assert_eq!(&plain[8297..8299], &i16::from(level - 1).to_le_bytes());
            assert!(player_rank(&bytes, 124).is_err());
        }
        for level in [0, 71, 255] {
            assert!(new_private(123, level).is_err());
        }
    }
    fn fixture() -> Vec<u8> {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/public-profile-synthetic.bin"
        ))
        .to_vec()
    }
    #[test]
    fn progression_update_preserves_prestige_and_every_unrelated_byte() {
        let mut blank = vec![0; SNAPSHOT_BYTES];
        blank[..4].copy_from_slice(&crc32fast::hash(&vec![0; SNAPSHOT_BYTES - 4]).to_le_bytes());
        let seed = prepare_private(&blank, 123, true).unwrap();
        let (mut before, _) = snapshot_and_tail(&seed).unwrap();
        before[4883..4887].copy_from_slice(&2u32.to_le_bytes());
        before[8297..8299].copy_from_slice(&200i16.to_le_bytes());
        before[12000] = 42;
        let old = encode_private(before.clone()).unwrap();
        let updated = with_player_progress(&old, 123, 3, 1).unwrap();
        assert_eq!(player_rank(&updated, 123).unwrap(), (2, 2));
        assert_eq!(
            player_progress(&updated, 123).unwrap(),
            PlayerProgress {
                rank: 3,
                experience: 1,
                rank_version: 4,
            }
        );
        let (mut after, _) = snapshot_and_tail(&updated).unwrap();
        assert_eq!(&after[8297..8299], &200i16.to_le_bytes());
        for range in [0..4, 4844..4848, 4887..4895] {
            after[range.clone()].copy_from_slice(&before[range]);
        }
        assert_eq!(after, before);
        assert_eq!(with_player_progress(&updated, 123, 3, 1).unwrap(), updated);
        assert!(with_player_progress(&old, 124, 3, 1).is_err());
        for (rank, xp) in [(0, 0), (71, 0), (1, u32::MAX)] {
            assert!(with_player_progress(&old, 123, rank, xp).is_err());
        }
        before[4891..4895].copy_from_slice(&2u32.to_le_bytes());
        assert!(with_player_progress(&encode_private(before.clone()).unwrap(), 123, 3, 1).is_err());
        before[4844..4848].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(player_progress(&encode_private(before).unwrap(), 123).is_err());
        let ordinary = with_player_progress(&seed, 123, 2, 1).unwrap();
        assert_eq!(
            &snapshot_and_tail(&ordinary).unwrap().0[8297..8299],
            &1i16.to_le_bytes()
        );
    }
    #[test]
    fn test_rank_preserves_every_unrelated_byte_and_checks_owner() {
        let mut blank = vec![0; SNAPSHOT_BYTES];
        let crc = crc32fast::hash(&blank[4..]);
        blank[..4].copy_from_slice(&crc.to_le_bytes());
        let uid = 0x89abcdef01234567;
        let old = prepare_private(&blank, uid, true).unwrap();
        let (mut before, _) = snapshot_and_tail(&old).unwrap();
        before[12345] = 42; // Unrelated progress must survive a boost.
        let old = encode_private(before.clone()).unwrap();
        let updated = maximize_test_rank(&old, uid).unwrap();
        assert_eq!(player_rank(&old, uid).unwrap(), (0, 0));
        assert_eq!(player_rank(&updated, uid).unwrap(), (69, 0));
        assert!(player_rank(&updated, uid + 1).is_err());
        let (mut after, _) = snapshot_and_tail(&updated).unwrap();
        assert_eq!(&after[4844..4848], &0i32.to_le_bytes());
        assert_eq!(&after[4887..4891], &70i32.to_le_bytes());
        assert_eq!(&after[4891..4895], &4i32.to_le_bytes());
        assert_eq!(&after[8297..8299], &69i16.to_le_bytes());
        for range in [0..4, 4844..4848, 4887..4895, 8297..8299] {
            after[range.clone()].copy_from_slice(&before[range]);
        }
        assert_eq!(after, before);
        assert_eq!(maximize_test_rank(&updated, uid).unwrap(), updated);
        assert!(maximize_test_rank(&old, uid + 1).is_err());
        before[8297..8299].copy_from_slice(&140i16.to_le_bytes());
        let higher = encode_private(before).unwrap();
        let boosted = maximize_test_rank(&higher, uid).unwrap();
        assert_eq!(
            &snapshot_and_tail(&boosted).unwrap().0[8297..8299],
            &140i16.to_le_bytes()
        );
    }
    #[test]
    fn private_projection_matches_independent_target_layout_vector() {
        let data = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/private-profile-synthetic.bin"
        ));
        validate_private(data).unwrap();
        let record = private_record(u64::MAX, data).unwrap();
        assert_eq!(&record[9..], &data[..data.len() - 1]);
        let mut bad = data.to_vec();
        let last = bad.len() - 2;
        bad[last] ^= 1;
        assert!(validate_private(&bad).is_err());
        for size in 0..data.len() {
            assert!(validate_private(&data[..size]).is_err());
        }
    }
    #[test]
    fn private_seed_changes_only_verified_fields_and_refuses_existing_progress() {
        let mut blank = vec![0; SNAPSHOT_BYTES];
        let crc = crc32fast::hash(&blank[4..]);
        blank[..4].copy_from_slice(&crc.to_le_bytes());
        let user = 0x89abcdef01234567;
        let normal = prepare_private(&blank, user, false).unwrap();
        let (plain, _) = snapshot_and_tail(&normal).unwrap();
        assert_eq!(&plain[4887..4891], &1i32.to_le_bytes());
        assert_eq!(&plain[4943..4947], &0x89abcdefu32.to_le_bytes());
        assert_eq!(&plain[4947..4951], &0x01234567u32.to_le_bytes());
        assert_eq!(&plain[8577..8579], &[0, 0]);
        let marked = prepare_private(&blank, user, true).unwrap();
        let (mut marked_plain, _) = snapshot_and_tail(&marked).unwrap();
        assert_eq!(&marked_plain[8577..8579], &[1, 0]);
        assert!(prepare_private(&marked_plain, user, true).is_err());
        marked_plain[8577..8579].fill(0);
        marked_plain[..4].copy_from_slice(&plain[..4]);
        assert_eq!(marked_plain, plain);
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
    #[test]
    fn story_unlock_flags_are_independent_and_idempotent() {
        let profile = new_private(123, 1).unwrap();
        assert_eq!(story_unlock(&profile, 123, 150).unwrap(), 0);
        let unlocked = with_story_unlock(&profile, 123, 150).unwrap();
        assert_eq!(story_unlock(&unlocked, 123, 150).unwrap(), 1);
        assert_eq!(story_unlock(&unlocked, 123, 151).unwrap(), 0);
        assert_eq!(player_rank_unlock(&unlocked, 123).unwrap(), 0);
        assert_eq!(player_rank(&unlocked, 123).unwrap(), (0, 0));
        assert_eq!(with_story_unlock(&unlocked, 123, 150).unwrap(), unlocked);
        assert!(with_story_unlock(&profile, 999, 150).is_err());
        assert!(with_story_unlock(&profile, 123, 256).is_err());
    }
}
