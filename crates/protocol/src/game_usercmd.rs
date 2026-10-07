//! Inverse of target 0x5a1be0, with an explicit baseline from server playerState.
//! The 3.3.3.3 writer adds an 11-bit melee entity and a fifth trailing word.
use crate::{game_wire::Reader, lobby::LobbyError};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserCommand {
    pub time: u32,
    pub buttons: u32,
    pub angles: [u16; 3],
    pub direction: u8,
    pub weapons: [u16; 3],
    pub selected: [u8; 3],
    pub remote: [u8; 2],
    pub melee_yaw: u16,
    pub melee_distance: u8,
    pub melee_entity: u16,
}
fn delta(r: &mut Reader<'_>, key: u32, old: u32, width: usize) -> Result<u32, LobbyError> {
    if r.bits(1)? == 0 {
        return Ok(old);
    }
    Ok((r.bits(width)? ^ key) & ((1u32 << width) - 1))
}
fn short(r: &mut Reader<'_>, key: u32, old: u16) -> Result<u16, LobbyError> {
    Ok(if r.bits(1)? == 0 {
        old
    } else {
        r.u16()? ^ key as u16
    })
}
fn byte(r: &mut Reader<'_>, key: u32, old: u8) -> Result<u8, LobbyError> {
    Ok(if r.bits(1)? == 0 {
        old
    } else {
        r.u8()? ^ key as u8
    })
}
fn command(r: &mut Reader<'_>, key: u32, old: &UserCommand) -> Result<UserCommand, LobbyError> {
    let mut out = old.clone();
    out.time = if r.bits(1)? == 1 {
        old.time
            .checked_add(u32::from(r.u8()?))
            .ok_or(LobbyError("usercmd time overflow"))?
    } else {
        r.u32()?
    };
    let mut tag = 7;
    if r.bits(1)? ^ (key & 1) != 0 {
        if r.bits(1)? ^ (key & 1) == 0 {
            tag = 2;
            let k = key ^ out.time;
            out.buttons = (old.buttons & !1) | (r.bits(1)? ^ (k & 1));
            out.angles[0] = short(r, k, old.angles[0])?;
            out.angles[1] = short(r, k, old.angles[1])?;
            out.direction = delta(r, k, u32::from(old.direction), 4)? as u8;
        } else {
            tag = 5;
            let fire = r.bits(1)? ^ (key & 1);
            out.angles[0] = short(r, key, old.angles[0])?;
            out.angles[1] = short(r, key, old.angles[1])?;
            out.direction = delta(r, key, u32::from(old.direction), 4)? as u8;
            let k = key ^ out.time;
            out.angles[2] = short(r, k, old.angles[2])?;
            out.buttons = (delta(r, k, old.buttons >> 1, 29)? << 1) | fire;
            for i in 0..3 {
                out.weapons[i] = delta(r, k, u32::from(old.weapons[i]), 10)? as u16;
            }
            if out.buttons & 0x20000 != 0 {
                for i in 0..3 {
                    out.selected[i] = byte(r, k, old.selected[i])?;
                }
            }
            if out.buttons & 0x200000 != 0 {
                for i in 0..2 {
                    out.remote[i] = byte(r, k, old.remote[i])?;
                }
            }
            if out.buttons & 4 != 0 {
                out.melee_yaw = short(r, k, old.melee_yaw)?;
                out.melee_distance = delta(r, k, u32::from(old.melee_distance), 8)? as u8;
                out.melee_entity = delta(r, k, u32::from(old.melee_entity), 11)? as u16;
            }
        }
    }
    if r.bits(3)? != tag {
        return Err(LobbyError("usercmd branch tag mismatch"));
    }
    if out.time == 0 || out.time > i32::MAX as u32 || out.time < old.time {
        return Err(LobbyError("invalid usercmd time"));
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Movement {
    pub delta_snapshot: bool,
    pub commands: Vec<UserCommand>,
    pub predicted_origin_bits: [u32; 3],
    pub predicted_time: u32,
    pub extra_word: u32,
}
pub fn read_movement(
    r: &mut Reader<'_>,
    key: u32,
    delta_snapshot: bool,
) -> Result<Movement, LobbyError> {
    read_movement_from(r, key, delta_snapshot, &UserCommand::default())
}

pub fn read_movement_from(
    r: &mut Reader<'_>,
    key: u32,
    delta_snapshot: bool,
    initial: &UserCommand,
) -> Result<Movement, LobbyError> {
    let count = r.u8()?;
    if !(1..=32).contains(&count) {
        return Err(LobbyError("usercmd count out of range"));
    }
    // 0x5a18e0 builds a fresh base from the current playerState per packet.
    // The caller must supply the base matching the acknowledged playerState.
    let mut base = initial.clone();
    let mut commands = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        base = command(r, key, &base)?;
        commands.push(base.clone());
    }
    let mut origin = [0; 3];
    for word in &mut origin {
        *word = r.u32()?;
        if !f32::from_bits(*word).is_finite() {
            return Err(LobbyError("non-finite predicted origin"));
        }
    }
    Ok(Movement {
        delta_snapshot,
        commands,
        predicted_origin_bits: origin,
        predicted_time: r.u32()?,
        extra_word: r.u32()?,
    })
}

/// 0x62d850: 32-byte, signed-char weighted hash, arithmetic shifts.
pub fn key(ack: u32, checksum_feed: u32, command: &[u8]) -> u32 {
    let mut hash = 0i32;
    for (i, b) in command
        .iter()
        .copied()
        .take(32)
        .take_while(|b| *b != 0)
        .enumerate()
    {
        hash = hash.wrapping_add(i32::from(b as i8).wrapping_mul(119 + i as i32));
    }
    ack ^ checksum_feed ^ (((hash >> 10) ^ hash) >> 10 ^ hash) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{game_state::Writer, game_wire::client_message};
    #[test]
    fn captured_first_active_input_checks_tags_tail_and_truncation() {
        // Public protocol vector from client-1791344097175, ACK4. No identity,
        // keys, reliable text or account data; native XOR has been removed.
        let packet = [
            0x30, 1, 0xe8, 3, 0, 0, 0xc8, 0x3e, 0, 0x90, 0xd0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0xe8, 3, 0, 0, 0, 0, 0, 0, 1,
        ];
        let decoded = client_message(&packet, Some(4)).unwrap();
        let moves = decoded.movement.unwrap();
        assert_eq!(moves.commands.len(), 1);
        assert_eq!(moves.commands[0].time, 1000);
        assert_eq!(moves.predicted_time, 1000);
        assert_eq!(moves.predicted_origin_bits, [0; 3]);
        assert_eq!(moves.extra_word, 0);
        for end in 0..packet.len() {
            assert!(client_message(&packet[..end], Some(4)).is_err());
        }
        assert!(client_message(&packet, None).is_err());
        let mut bad = packet;
        bad[1] = 33;
        assert!(client_message(&bad, Some(4)).is_err());
        let mut bad = packet;
        bad[31] = 0;
        assert!(client_message(&bad, Some(4)).is_err());
    }
    #[test]
    fn full_melee_input_consumes_the_new_eleven_bit_entity() {
        let mut w = Writer::default();
        w.bits(1, 3);
        w.bytes.push(1); // nondelta, one command
        w.bits(0, 1);
        w.bytes.extend_from_slice(&1000u32.to_le_bytes());
        w.bits(1, 1);
        w.bits(1, 1); // key0, full branch
        w.bits(0, 1); // fire
        w.bits(0, 4); // two angles, direction, third angle unchanged
        w.bits(1, 1);
        w.bits(2 ^ 1000, 29); // buttons=4 (melee)
        w.bits(0, 3); // weapon IDs unchanged
        w.bits(1, 1);
        w.bytes.extend_from_slice(&(0x1234u16 ^ 1000).to_le_bytes());
        w.bits(1, 1);
        w.bits(127 ^ (1000 & 255), 8);
        w.bits(1, 1);
        w.bits(2047 ^ 1000, 11);
        w.bits(5, 3);
        w.bytes.extend_from_slice(&[0; 20]);
        w.bits(3, 3);
        let message = client_message(&w.bytes, Some(0)).unwrap();
        let movement = message.movement.unwrap();
        assert!(!movement.delta_snapshot);
        let c = &movement.commands[0];
        assert_eq!(
            (c.buttons, c.melee_yaw, c.melee_distance, c.melee_entity),
            (4, 0x1234, 127, 2047)
        );
        assert_eq!(key(4, 0, b""), 4);
        assert_eq!(key(4, 123, b"a\0ignored"), 4 ^ 123 ^ 11548);
    }
    #[test]
    fn unchanged_command_retains_equipment_and_player_melee_sentinel() {
        let mut w = Writer::default();
        w.bits(0, 3);
        w.bytes.push(1);
        w.bits(0, 1);
        w.bytes.extend_from_slice(&1000u32.to_le_bytes());
        w.bits(0, 1);
        w.bits(7, 3);
        w.bytes.extend_from_slice(&[0; 20]);
        w.bits(3, 3);
        let mut base = UserCommand::default();
        base.weapons[0] = 163;
        base.melee_entity = 2047;
        let message = crate::game_wire::client_message_with_base(&w.bytes, Some(0), &base).unwrap();
        let command = &message.movement.unwrap().commands[0];
        assert_eq!(command.time, 1000);
        assert_eq!(command.weapons, [163, 0, 0]);
        assert_eq!(command.melee_entity, 2047);
    }
}
