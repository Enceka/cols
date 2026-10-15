//! 3.3.3.3 private byte patches (binary K ->0x57e330 ->0x57bf70(1))
//! and cumulative weapon XP updates (< ->0x57cac0). All profile offsets below
//! exclude the saved snapshot CRC, as in the native public J patch format.
use crate::{
    game::snapshot::scoreboard::BINARY_PAYLOAD_LIMIT,
    lobby::{channel::LobbyError, profile},
};

pub fn private_progress(arguments: &[u8], user_id: u64) -> Result<Vec<u8>, LobbyError> {
    let progress = profile::player_progress(arguments, user_id)?;
    if progress.rank_version != 4 {
        return Err(LobbyError("unsupported native rank version"));
    }
    let mut output = vec![b'K'];
    for (offset, bytes) in [
        (4840u32, progress.experience.to_le_bytes()),
        (4883, u32::from(progress.rank).to_le_bytes()),
        (4887, 4u32.to_le_bytes()),
    ] {
        output.extend_from_slice(&offset.to_le_bytes());
        output.push(4);
        output.extend_from_slice(&bytes);
    }
    output.extend_from_slice(&8293u32.to_le_bytes());
    output.push(2);
    output.extend_from_slice(&profile::player_rank_unlock(arguments, user_id)?.to_le_bytes());
    if output.len() > BINARY_PAYLOAD_LIMIT {
        return Err(LobbyError("private progress patch too long"));
    }
    Ok(output)
}
pub fn weapon_xp(item: u32, xp: u32) -> Result<Vec<u8>, LobbyError> {
    if !(1..=65535).contains(&item) || xp > i32::MAX as u32 {
        return Err(LobbyError("invalid native weapon XP"));
    }
    Ok(format!("< {item} xp {xp}").into_bytes())
}
pub fn promotion(displayed_level: u8) -> Result<Vec<u8>, LobbyError> {
    if !(2..=70).contains(&displayed_level) {
        return Err(LobbyError("invalid native promotion level"));
    }
    Ok(format!("$ 0 0 {}", displayed_level - 1).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_patch_changes_only_measured_fields_in_the_native_crc_free_record() {
        let mut blank = vec![0; profile::SNAPSHOT_BYTES];
        let crc = crc32fast::hash(&blank[4..]);
        blank[..4].copy_from_slice(&crc.to_le_bytes());
        let seed = profile::prepare_private(&blank, 42, true).unwrap();
        let next = profile::with_player_progress(&seed, 42, 2, 1).unwrap();
        let patch = private_progress(&next, 42).unwrap();
        assert_eq!(patch[0], b'K');
        let mut record = vec![17; 32764];
        let original = record.clone();
        let mut position = 1;
        while position < patch.len() {
            let at = u32::from_le_bytes(patch[position..position + 4].try_into().unwrap()) as usize;
            let size = usize::from(patch[position + 4]);
            record[at..at + size].copy_from_slice(&patch[position + 5..position + 5 + size]);
            position += 5 + size;
        }
        assert_eq!(
            u32::from_le_bytes(record[4840..4844].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_le_bytes(record[4883..4887].try_into().unwrap()),
            2
        );
        assert_eq!(
            u32::from_le_bytes(record[4887..4891].try_into().unwrap()),
            4
        );
        assert_eq!(
            i16::from_le_bytes(record[8293..8295].try_into().unwrap()),
            1
        );
        for range in [4840..4844, 4883..4891, 8293..8295] {
            record[range.clone()].copy_from_slice(&original[range]);
        }
        assert_eq!(record, original);
        assert!(private_progress(&next, 43).is_err());
    }
    #[test]
    fn native_text_updates_are_absolute_and_reject_invalid_identity_or_overflow() {
        assert_eq!(weapon_xp(10001, 500).unwrap(), b"< 10001 xp 500");
        assert_eq!(promotion(2).unwrap(), b"$ 0 0 1");
        assert!(weapon_xp(0, 500).is_err());
        assert!(weapon_xp(10001, u32::MAX).is_err());
        assert!(promotion(1).is_err());
        assert!(promotion(71).is_err());
    }
}
