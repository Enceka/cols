//! PLAYER entities and their client appearance in the measured 3.3.3.3 layout.
//! Entity table 0xc52b68; client table 0xc55b68; DObj consumer 0x40b700.
use crate::{game_events, game_state::Writer, lobby::LobbyError};

/// One training opponent. The current world reserves entity/client1 for it.
/// Models and tags are registered NCS identifiers; body animation indices
/// belong to the loaded human tree, with optional restart toggle0x400.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemySnapshot {
    pub entity_number: u16,
    pub client_index: u8,
    pub team: u8,
    pub model: u16,
    pub attachment_models: [u16; 10],
    pub attachment_tags: [u16; 10],
    pub origin: [f32; 3],
    pub angles: [u16; 3],
    pub legs_animation: u16,
    pub torso_animation: u16,
    /// Registered weapon published to both ES+0x60 for the BG pose cache and
    /// ES+0xd0 low16 for the client gun presentation and firing effects.
    pub weapon: u16,
    /// Gameplay-owned sequence/ring. The writer never produces events or
    /// advances this counter when the same snapshot is retransmitted.
    pub events: game_events::EventQueue,
    pub alive: bool,
}

impl EnemySnapshot {
    fn validate(self) -> Result<(), LobbyError> {
        self.events.validate()?;
        let animation = |value: u16| value < 2048 && value & 0x3ff != 0;
        if self.entity_number > 18
            || u16::from(self.client_index) != self.entity_number
            || !matches!(self.team, 1 | 2)
            || !(1..1024).contains(&self.model)
            || self
                .origin
                .iter()
                .any(|n| !n.is_finite() || !(-32768.0..=32767.0).contains(n))
            || !animation(self.legs_animation)
            || (self.torso_animation != 0 && !animation(self.torso_animation))
            || !(1..900).contains(&self.weapon)
        {
            return Err(LobbyError("invalid training enemy state"));
        }
        let mut ended = false;
        for (model, tag) in self.attachment_models.into_iter().zip(self.attachment_tags) {
            if model >= 1024 || tag >= 64 || (model == 0 && tag != 0) || (ended && model != 0) {
                return Err(LobbyError("invalid training enemy attachment list"));
            }
            ended |= model == 0;
        }
        Ok(())
    }
}

/// Write a PLAYER record before the entity sentinel. Every live ordinary
/// frame supplies position, angles, animation and weapons. Numeric payloads
/// use the pinned full baseline, while originforce1 makes position and angle
/// codecs absolute. The caller retains that baseline until a new one is ACKed.
pub(crate) fn write_enemy_entity(
    out: &mut Writer,
    enemy: EnemySnapshot,
    baseline: Option<EnemySnapshot>,
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    enemy.validate()?;
    if establish_baseline {
        if baseline.is_some() {
            return Err(LobbyError(
                "full player baseline cannot reference a prior entity",
            ));
        }
    } else if let Some(base) = baseline {
        base.validate()?;
        if !base.alive
            || enemy.model != base.model
            || enemy.attachment_models != base.attachment_models
            || enemy.attachment_tags != base.attachment_tags
            || enemy.team != base.team
        {
            return Err(LobbyError("enemy appearance must match its live baseline"));
        }
    }

    out.bits(0, 1);
    out.bits(1, 1);
    out.bits(u32::from(enemy.entity_number), 11);
    out.bits(u32::from(!enemy.alive), 1);
    if !enemy.alive {
        return Ok(());
    }
    out.bits(1, 1); // changed
    out.bits(1, 1); // absolute position/angle codec bases
    out.bits(39, 6); // last changed field index

    let mut previous = -1;
    for index in [
        0, 1, 2, 3, 5, 6, 7, 8, 9, 11, 12, 13, 14, 17, 26, 27, 34, 36, 37, 39,
    ] {
        entity_gap(out, (index - previous) as u32);
        previous = index;
        match index {
            0 => out.bytes.push(1), // eType PLAYER, rawbyte -77
            1 | 2 | 5 => {
                let axis = match index {
                    1 => 1, // ES+1c Y
                    2 => 0, // ES+18 X
                    _ => 2, // ES+20 Z
                };
                out.bits(1, 1); // full signed16 position
                out.bits(
                    u32::from(enemy.origin[axis].round_ties_even() as i16 as u16),
                    16,
                );
            }
            3 | 7 | 26 => {
                let axis = match index {
                    3 => 1, // ES+40 yaw
                    7 => 0, // ES+3c pitch
                    _ => 2, // ES+44 roll
                };
                // -79 -> 0x5a25c0: 0,0 selects absolute short angle, whose
                // raw LE16 bytes coexist with the unfinished bit pool.
                out.bits(0, 2);
                out.bytes
                    .extend_from_slice(&enemy.angles[axis].to_le_bytes());
            }
            // These codecs are shared with PS but occupy different fields.
            // The validated pinned entity baseline's ring is entirely zero.
            6 => game_events::write_sequence(out, enemy.events.sequence),
            11..=14 => game_events::write_event(out, enemy.events.slots[index as usize - 11]),
            8 => scalar(
                out,
                u32::from(enemy.legs_animation),
                baseline.map_or(0, |b| u32::from(b.legs_animation)),
                11,
            ),
            9 => scalar(
                out,
                u32::from(enemy.torso_animation),
                baseline.map_or(0, |b| u32::from(b.torso_animation)),
                11,
            ),
            // ES+0xd0 is a packed word. The firing consumer0x55dde0 and gun
            // presentation0x55bf90 read its low16 registered weapon index;
            // merely publishing the separate BG cached weapon at+0x60 is
            // insufficient. The unused upper16 stays zero in this world.
            17 => scalar(
                out,
                u32::from(enemy.weapon),
                baseline.map_or(0, |b| u32::from(b.weapon)),
                32,
            ),
            // ES+0x0c position trType1 selects the client's interpolation
            // between current/next snapshot positions (0x506a80 -> 0x506000).
            // It is an ordinary numeric field, so originforce1 does not reset
            // this field's baseline. Live full baselines always contain1.
            27 => scalar(out, 1, u32::from(baseline.is_some()), 8),
            34 => scalar(
                out,
                u32::from(enemy.client_index),
                baseline.map_or(0, |b| u32::from(b.client_index)),
                6,
            ),
            36 => scalar(
                out,
                u32::from(enemy.weapon),
                baseline.map_or(0, |b| u32::from(b.weapon)),
                16,
            ),
            37 => scalar(out, 0, 0, 16), // no second held weapon
            39 => {
                out.bits(1, 1); // nonzero unit model scale
                out.bits(1, 1); // full precision XOR
                let base = if baseline.is_some() {
                    1.0f32.to_bits()
                } else {
                    0
                };
                out.bytes
                    .extend_from_slice(&(1.0f32.to_bits() ^ base).to_le_bytes());
            }
            _ => unreachable!("fixed PLAYER fields"),
        }
    }
    Ok(())
}

/// Write the complete clients section, including its single terminator.
/// Appearance is static within the pinned baseline. Ordinary frames emit
/// presence0 and inherit it, including while the entity is dead.
pub(crate) fn write_enemy_client(
    out: &mut Writer,
    enemy: EnemySnapshot,
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    write_player_clients(out, &[enemy], establish_baseline)
}

pub(crate) fn write_player_clients(
    out: &mut Writer,
    players: &[EnemySnapshot],
    establish_baseline: bool,
) -> Result<(), LobbyError> {
    write_player_clients_with_card(out, players, establish_baseline, None, false, &[])
}

pub(crate) fn write_player_clients_with_card(
    out: &mut Writer,
    players: &[EnemySnapshot],
    establish_baseline: bool,
    local_card: Option<crate::game_player_card::Card>,
    held_weapons: bool,
    characters: &[Option<crate::game_character::Character>],
) -> Result<(), LobbyError> {
    if let Some(card) = local_card {
        card.validate()?;
    }
    if characters.len() > 19 {
        return Err(LobbyError("too many native character identities"));
    }
    for character in characters.iter().flatten() {
        character.fields()?;
    }
    for (i, player) in players.iter().enumerate() {
        player.validate()?;
        if i > 0 && players[i - 1].client_index >= player.client_index {
            return Err(LobbyError("player clients must be sorted and unique"));
        }
        if establish_baseline && player.client_index > 18 {
            return Err(LobbyError(
                "player client baseline requires alive state and empty events",
            ));
        }
    }
    if establish_baseline {
        for player in players {
            write_client_record(
                out,
                *player,
                local_card.filter(|_| player.client_index == 0),
                held_weapons,
                characters
                    .get(usize::from(player.client_index))
                    .copied()
                    .flatten(),
            )?;
        }
    }
    out.bits(0, 1);
    Ok(())
}

fn write_client_record(
    out: &mut Writer,
    enemy: EnemySnapshot,
    card: Option<crate::game_player_card::Card>,
    held_weapons: bool,
    character: Option<crate::game_character::Character>,
) -> Result<(), LobbyError> {
    enemy.validate()?;

    out.bits(1, 1); // list entry
    out.bits(0, 1); // absolute client number
    out.bits(u32::from(enemy.client_index), 5);
    out.bits(0, 1); // retained
    out.bits(1, 1); // generic field section changed
    out.bits(53, 7); // last changed client field index
    let mut fields = vec![
        (23, u32::from(enemy.team), 3),
        (27, u32::from(enemy.model), 10),
    ];
    if held_weapons {
        // clientState+0x114 -> CI+0x330 (0x52533e): native main gun entity.
        fields.push((
            0,
            u32::from(crate::game_held_weapon::entity_for(enemy.entity_number)),
            11,
        ));
    }
    if let Some(card) = card {
        fields.extend([
            (30, u32::from(card.rank), 8),
            (37, u32::from(card.prestige), 8),
            (39, u32::from(card.icon), 10),
            (40, u32::from(card.title), 10),
            (41, u32::from(card.background), 10),
            (42, u32::from(card.nameplate), 10),
        ]);
    }
    for (slot, index) in [20, 21, 22, 31, 32, 33, 46, 47, 48, 49]
        .into_iter()
        .enumerate()
    {
        fields.push((index, u32::from(enemy.attachment_models[slot]), 10));
    }
    for (slot, index) in [44, 34, 35, 36, 45, 43, 50, 51, 52, 53]
        .into_iter()
        .enumerate()
    {
        fields.push((index, u32::from(enemy.attachment_tags[slot]), 6));
    }
    if let Some(character) = character {
        let packed = character.fields()?;
        fields.retain(|(i, _, _)| !packed.iter().any(|p| p.0 == *i));
        fields.extend(packed);
    }
    fields.sort_by_key(|field| field.0);
    let mut previous = -1;
    for (index, value, width) in fields {
        // Client gaps use parameter1 at0x5a3250: unary distance-1 zeros,1.
        for _ in 1..index - previous {
            out.bits(0, 1);
        }
        out.bits(1, 1);
        previous = index;
        scalar(out, value, 0, width);
    }
    Ok(())
}

fn scalar(out: &mut Writer, value: u32, base: u32, width: usize) {
    out.bits(u32::from(value != 0), 1);
    if value == 0 {
        return;
    }
    let delta = value ^ base;
    let remainder = width % 8;
    out.bits(delta, remainder);
    for shift in (remainder..width).step_by(8) {
        out.bytes.push((delta >> shift) as u8);
    }
}

fn entity_gap(out: &mut Writer, distance: u32) {
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
        let table: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/enemy-snapshot-fields-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(table["client_sha256"], crate::CLIENT_SHA256);
        assert_eq!(table["player_entity"]["table_va"], 0xc52b68);
        assert_eq!(table["client"]["table_va"], 0xc55b68);
        assert_eq!(table["event_parameters"]["table_va"], 0xc56d70);
        table
    }

    fn read_fields(
        reader: &mut Reader<'_>,
        table: &serde_json::Value,
        last_width: usize,
        unary: bool,
        base: &BTreeMap<u32, u32>,
    ) -> BTreeMap<u32, u32> {
        let last = reader.bits(last_width).unwrap() as i32;
        let mut index = -1;
        let mut result = BTreeMap::new();
        while index < last {
            let mut distance = 1;
            if unary {
                while reader.bits(1).unwrap() == 0 {
                    distance += 1;
                }
            } else if reader.bits(1).unwrap() == 0 {
                loop {
                    let chunk = reader.bits(3).unwrap();
                    distance += chunk;
                    if chunk != 7 {
                        break;
                    }
                }
            }
            index += distance as i32;
            assert!((0..=last).contains(&index));
            let field = &table["fields"][index as usize];
            let offset = field[0].as_u64().unwrap() as u32;
            let codec = field[1].as_i64().unwrap();
            let baseline = base.get(&offset).copied().unwrap_or(0);
            let value = match codec {
                -77 => u32::from(reader.u8().unwrap()),
                -75 => {
                    assert_eq!(reader.bits(1).unwrap(), 0);
                    assert_eq!(baseline, 0, "event counter requires a zero pinned base");
                    u32::from(reader.u8().unwrap())
                }
                -94 => {
                    assert_eq!(reader.bits(1).unwrap(), 1);
                    let id = reader.u8().unwrap();
                    let widths = fixture();
                    let width = widths["event_parameters"]["bits"][usize::from(id)]
                        .as_u64()
                        .unwrap() as usize;
                    let parameter = if width != 0 && reader.bits(1).unwrap() != 0 {
                        reader.bits(width).unwrap()
                    } else {
                        0
                    };
                    assert!(result.insert(offset + 4, parameter).is_none());
                    u32::from(id)
                }
                -83..=-81 => {
                    assert_eq!(reader.bits(1).unwrap(), 1);
                    reader.bits(16).unwrap()
                }
                -79 => {
                    assert_eq!(reader.bits(2).unwrap(), 0);
                    u32::from(reader.u16().unwrap())
                }
                0 => {
                    assert_eq!(reader.bits(2).unwrap(), 3);
                    reader.u32().unwrap() ^ baseline
                }
                width if width > 0 => {
                    if reader.bits(1).unwrap() == 0 {
                        0
                    } else {
                        let width = width as usize;
                        let remainder = width % 8;
                        let mut delta = reader.bits(remainder).unwrap();
                        for shift in (remainder..width).step_by(8) {
                            delta |= u32::from(reader.u8().unwrap()) << shift;
                        }
                        delta ^ baseline
                    }
                }
                _ => panic!("unsupported fixture codec {codec}"),
            };
            assert!(result.insert(offset, value).is_none());
        }
        result
    }

    fn read_entity(
        reader: &mut Reader<'_>,
        base: &BTreeMap<u32, u32>,
    ) -> Option<BTreeMap<u32, u32>> {
        assert_eq!(reader.bits(2).unwrap(), 2);
        assert_eq!(reader.bits(11).unwrap(), 1);
        if reader.bits(1).unwrap() != 0 {
            return None;
        }
        assert_eq!(reader.bits(2).unwrap(), 3);
        Some(read_fields(
            reader,
            &fixture()["player_entity"],
            6,
            false,
            base,
        ))
    }

    fn enemy() -> EnemySnapshot {
        EnemySnapshot {
            entity_number: 1,
            client_index: 1,
            team: 1,
            model: 4,
            attachment_models: [3, 2, 5, 6, 0, 0, 0, 0, 0, 0],
            attachment_tags: [0; 10],
            origin: [-312.5, -800.5, -113.4],
            angles: [12345, 54321, 65535],
            legs_animation: 7,
            torso_animation: 0x412,
            weapon: 352,
            events: game_events::EventQueue::default(),
            alive: true,
        }
    }

    #[test]
    fn player_baseline_and_moving_frame_decode_against_the_pinned_base() {
        let initial = enemy();
        let moving = EnemySnapshot {
            origin: [100.5, -1550.5, 213.5],
            angles: [54321, 1001, 0],
            legs_animation: 0x41f,
            torso_animation: 44,
            weapon: 353,
            ..initial
        };
        for alignment in 0..8 {
            let mut first = Writer::default();
            first.bits(0x55, alignment);
            first.bytes.push(0xe6);
            write_enemy_entity(&mut first, initial, None, true).unwrap();
            first.bits(2047, 11);
            first.bytes.push(6);
            let mut r = Reader::new(&first.bytes);
            assert_eq!(r.bits(alignment).unwrap(), 0x55 & ((1 << alignment) - 1));
            assert_eq!(r.u8().unwrap(), 0xe6);
            let base = read_entity(&mut r, &BTreeMap::new()).unwrap();
            assert_eq!(base.len(), 24); // 20 fields, including four event parameter words
            assert_eq!(base[&4], 1);
            assert_eq!(base[&0xc], 1); // TR_INTERPOLATE
            assert_eq!(base[&0x18] as u16 as i16, -312);
            assert_eq!(base[&0x1c] as u16 as i16, -800);
            assert_eq!(base[&0x20] as u16 as i16, -113);
            assert_eq!(base[&0x3c], 12345);
            assert_eq!(base[&0x40], 54321);
            assert_eq!(base[&0x44], 65535);
            assert_eq!(base[&0xd4], 7);
            assert_eq!(base[&0xd8], 0x412);
            assert_eq!(base[&0x9c], 1);
            assert_eq!(base[&0x60], 352);
            assert_eq!(base[&0xd0], 352);
            assert_eq!(base[&0x64], 0);
            assert_eq!(base[&0x54], 1.0f32.to_bits());
            assert_eq!(r.bits(11).unwrap(), 2047);
            assert_eq!(r.u8().unwrap(), 6);
            r.finish().unwrap();

            let mut out = Writer::default();
            out.bits(0x55, alignment);
            out.bytes.push(0xe6);
            write_enemy_entity(&mut out, moving, Some(initial), false).unwrap();
            let mut r = Reader::new(&out.bytes);
            assert_eq!(r.bits(alignment).unwrap(), 0x55 & ((1 << alignment) - 1));
            assert_eq!(r.u8().unwrap(), 0xe6);
            let current = read_entity(&mut r, &base).unwrap();
            assert_eq!(current[&0xc], 1); // same value encoded as XOR0 against baseline1
            assert_eq!(current[&0x18] as u16 as i16, 100);
            assert_eq!(current[&0x1c] as u16 as i16, -1550);
            assert_eq!(current[&0x20] as u16 as i16, 214);
            assert_eq!(current[&0x3c], 54321);
            assert_eq!(current[&0x40], 1001);
            assert_eq!(current[&0x44], 0);
            assert_eq!(current[&0xd4], 0x41f);
            assert_eq!(current[&0xd8], 44);
            assert_eq!(current[&0x9c], 1);
            assert_eq!(current[&0x60], 353);
            assert_eq!(current[&0xd0], 353);
            assert_eq!(current[&0x64], 0);
            assert_eq!(current[&0x54], 1.0f32.to_bits());
            r.finish().unwrap();
        }
    }

    #[test]
    fn client_appearance_uses_current_offsets_unary_gaps_and_one_terminator() {
        let enemy = EnemySnapshot {
            attachment_models: [1023, 22, 33, 44, 55, 66, 77, 88, 99, 100],
            attachment_tags: [63, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            ..enemy()
        };
        for alignment in 0..8 {
            let mut out = Writer::default();
            out.bits(0x35, alignment);
            out.bytes.push(0xe6);
            write_enemy_client(&mut out, enemy, true).unwrap();
            out.bits(5, 3); // next section continues the same pool
            out.bytes.push(6);
            let mut r = Reader::new(&out.bytes);
            assert_eq!(r.bits(alignment).unwrap(), 0x35 & ((1 << alignment) - 1));
            assert_eq!(r.u8().unwrap(), 0xe6);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.bits(5).unwrap(), 1);
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.bits(1).unwrap(), 1);
            let fields = read_fields(&mut r, &fixture()["client"], 7, true, &BTreeMap::new());
            assert_eq!(fields.len(), 22);
            assert_eq!(fields[&4], 1);
            assert_eq!(fields[&0x14], 4);
            for slot in 0..10 {
                assert_eq!(
                    fields[&(0x18 + slot as u32 * 4)],
                    u32::from(enemy.attachment_models[slot])
                );
                assert_eq!(
                    fields[&(0x40 + slot as u32 * 4)],
                    u32::from(enemy.attachment_tags[slot])
                );
            }
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.bits(3).unwrap(), 5);
            assert_eq!(r.u8().unwrap(), 6);
            r.finish().unwrap();
        }
    }

    #[test]
    fn native_character_fields_preserve_each_identity_and_explicit_podium_models() {
        let character = crate::game_character::Character {
            groups: [19, 1, 49, 79, 6, 0, 12, 0, 0, 0, 0],
            bits: [7, 4, 8, 8, 6, 7, 8, 5, 0, 8, 0],
            gender: 1,
            faction: 1,
            skin: 3,
        };
        let remote_character = crate::game_character::Character {
            groups: [10, 1, 12, 85, 5, 0, 0, 0, 0, 0, 0],
            gender: 0,
            faction: 0,
            skin: 0,
            ..character
        };
        for alignment in 0..8 {
            let mut local = enemy();
            local.client_index = 0;
            local.entity_number = 0;
            let podium = EnemySnapshot {
                client_index: 12,
                entity_number: 12,
                ..enemy()
            };
            let mut out = Writer::default();
            out.bits(0, alignment);
            write_player_clients_with_card(
                &mut out,
                &[local, enemy(), podium],
                true,
                None,
                true,
                &[Some(character), Some(remote_character)],
            )
            .unwrap();
            out.bytes.push(0xa7);
            let mut r = Reader::new(&out.bytes);
            r.bits(alignment).unwrap();
            for client in [0, 1, 12] {
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.bits(1).unwrap(), 0);
                assert_eq!(r.bits(5).unwrap(), client);
                assert_eq!(r.bits(1).unwrap(), 0);
                assert_eq!(r.bits(1).unwrap(), 1);
                let fields = read_fields(&mut r, &fixture()["client"], 7, true, &BTreeMap::new());
                assert_eq!(fields[&0x114], 27 + client);
                if client < 12 {
                    let character = if client == 0 {
                        character
                    } else {
                        remote_character
                    };
                    assert_eq!(fields[&0x124], if client == 0 { 6 } else { 5 });
                    assert_eq!(fields[&0x128], u32::from(character.faction));
                    assert_eq!(fields[&0x12c], u32::from(character.gender));
                    assert_eq!(fields[&0x130], u32::from(character.skin));
                    let mut packed = Vec::new();
                    for row in fixture()["client"]["fields"].as_array().unwrap() {
                        let offset = row[0].as_u64().unwrap() as u32;
                        if offset != 0x14 && !(0x18..0x68).contains(&offset) {
                            continue;
                        }
                        for bit in 0..row[1].as_u64().unwrap() {
                            packed.push((fields[&offset] >> bit) & 1);
                        }
                    }
                    let mut at = 0;
                    for (&id, width) in character.groups.iter().zip(character.bits) {
                        let present = packed[at];
                        at += 1;
                        let mut decoded = 0;
                        if present != 0 {
                            for bit in 0..width {
                                decoded |= packed[at] << bit;
                                at += 1;
                            }
                            decoded += 1;
                        }
                        assert_eq!(decoded, u32::from(id));
                    }
                } else {
                    assert!(!fields.contains_key(&0x124));
                    assert_eq!(fields[&0x14], u32::from(enemy().model));
                }
            }
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.u8().unwrap(), 0xa7);
            r.finish().unwrap();
        }
    }
    #[test]
    fn local_card_decodes_through_the_measured_client_fields() {
        for alignment in 0..8 {
            let mut local = enemy();
            local.client_index = 0;
            local.entity_number = 0;
            let card = crate::game_player_card::Card {
                rank: 69,
                prestige: 2,
                icon: 138,
                title: 214,
                background: 396,
                nameplate: 0,
            };
            let mut out = Writer::default();
            out.bits(0, alignment);
            write_player_clients_with_card(&mut out, &[local], true, Some(card), false, &[])
                .unwrap();
            let mut reader = Reader::new(&out.bytes);
            reader.bits(alignment).unwrap();
            assert_eq!(reader.bits(1).unwrap(), 1);
            assert_eq!(reader.bits(1).unwrap(), 0);
            assert_eq!(reader.bits(5).unwrap(), 0);
            assert_eq!(reader.bits(1).unwrap(), 0);
            assert_eq!(reader.bits(1).unwrap(), 1);
            let fields = read_fields(&mut reader, &fixture()["client"], 7, true, &BTreeMap::new());
            for (offset, value) in [
                (0x6c, 69),
                (0x70, 2),
                (0x74, 138),
                (0x78, 214),
                (0x7c, 396),
                (0x80, 0),
            ] {
                assert_eq!(fields[&offset], value);
            }
            assert_eq!(reader.bits(1).unwrap(), 0);
            reader.finish().unwrap();
        }
    }
    #[test]
    fn death_repeats_removal_and_respawn_restores_entity_with_static_client() {
        let initial = enemy();
        for alive in [false, false, true] {
            let current = EnemySnapshot { alive, ..initial };
            let mut out = Writer::default();
            write_enemy_entity(&mut out, current, Some(initial), false).unwrap();
            out.bits(0, 1);
            out.bits(1, 1);
            out.bits(2047, 11);
            write_enemy_client(&mut out, current, false).unwrap();
            out.bytes.push(6);
            let mut r = Reader::new(&out.bytes);
            assert_eq!(read_entity(&mut r, &BTreeMap::new()).is_some(), alive);
            assert_eq!(r.bits(2).unwrap(), 2);
            assert_eq!(r.bits(11).unwrap(), 2047);
            assert_eq!(r.bits(1).unwrap(), 0); // inherit client appearance
            assert_eq!(r.u8().unwrap(), 6);
            r.finish().unwrap();
        }
    }

    // The current 0x5139b0 consumer uses the decoded counter and slot&3,
    // catches up at most four events, and treats counter0 as a reset.
    fn consume_events(fields: &BTreeMap<u32, u32>, processed: &mut i32) -> Vec<game_events::Event> {
        let sequence = fields[&0xac] as i32;
        if sequence == 0 {
            *processed = 0;
            return Vec::new();
        }
        if *processed > sequence + 64 {
            *processed -= 256;
        }
        if sequence - *processed > 4 {
            *processed = sequence - 4;
        }
        let mut events = Vec::new();
        while *processed < sequence {
            let offset = 0xb0 + ((*processed as u32) & 3) * 8;
            let id = fields[&offset] as u8;
            if id != 0 {
                events.push(game_events::Event {
                    id,
                    parameter: fields[&(offset + 4)],
                });
            }
            *processed += 1;
        }
        events
    }

    #[test]
    fn fire_and_brass_ring_decode_once_across_repeated_frames_and_counter_wrap() {
        let initial = enemy();
        let mut first = Writer::default();
        write_enemy_entity(&mut first, initial, None, true).unwrap();
        let mut r = Reader::new(&first.bytes);
        let base = read_entity(&mut r, &BTreeMap::new()).unwrap();
        assert_eq!(base[&0xac], 0);
        for slot in 0..4 {
            assert_eq!(base[&(0xb0 + slot * 8)], 0);
            assert_eq!(base[&(0xb4 + slot * 8)], 0);
        }

        let mut queue = game_events::EventQueue {
            sequence: 253,
            ..Default::default()
        };
        let mut processed = 0;
        // Explicit no-event counter0 reset separates the wrap. The writer
        // faithfully carries this gameplay-owned ring; it does not manufacture
        // events or advance the counter during repeated snapshot generation.
        for id in [36, 41, 0, 36, 41] {
            queue.push(id, 0).unwrap();
            let current = EnemySnapshot {
                events: queue,
                ..initial
            };
            for alignment in 0..8 {
                let mut out = Writer::default();
                out.bits(0x55, alignment);
                out.bytes.push(0xe6);
                write_enemy_entity(&mut out, current, Some(initial), false).unwrap();
                let mut r = Reader::new(&out.bytes);
                assert_eq!(r.bits(alignment).unwrap(), 0x55 & ((1 << alignment) - 1));
                assert_eq!(r.u8().unwrap(), 0xe6);
                let fields = read_entity(&mut r, &base).unwrap();
                assert_eq!(fields[&0xac], u32::from(queue.sequence));
                for slot in 0..4 {
                    assert_eq!(
                        fields[&(0xb0 + slot * 8)],
                        u32::from(queue.slots[slot as usize].id)
                    );
                    assert_eq!(
                        fields[&(0xb4 + slot * 8)],
                        queue.slots[slot as usize].parameter
                    );
                }
                r.finish().unwrap();
                if alignment == 0 {
                    let expected = if id == 0 {
                        Vec::new()
                    } else {
                        vec![game_events::Event { id, parameter: 0 }]
                    };
                    assert_eq!(consume_events(&fields, &mut processed), expected);
                    assert!(consume_events(&fields, &mut processed).is_empty());
                }
                let mut repeated = Writer::default();
                repeated.bits(0x55, alignment);
                repeated.bytes.push(0xe6);
                write_enemy_entity(&mut repeated, current, Some(initial), false).unwrap();
                assert_eq!(repeated.bytes, out.bytes);
                assert_eq!(current.events, queue);
            }
        }
        assert_eq!(queue.sequence, 2);
    }

    #[test]
    fn rejects_invalid_event_parameters_and_supports_live_snapshot_bases() {
        let initial = enemy();
        let mut fired = game_events::EventQueue::default();
        fired.push(36, 0).unwrap();
        let active = EnemySnapshot {
            events: fired,
            ..initial
        };
        let mut invalid_parameter = fired;
        invalid_parameter.slots[0].parameter = 32; // FIRE only has5 bits
        let mut invalid_id = fired;
        invalid_id.slots[0].id = 162;
        for invalid in [invalid_parameter, invalid_id] {
            let current = EnemySnapshot {
                events: invalid,
                ..initial
            };
            let mut out = Writer::default();
            out.bits(5, 3);
            out.bytes.push(0xe6);
            let before = out.bytes.clone();
            assert!(write_enemy_entity(&mut out, current, Some(initial), false).is_err());
            assert_eq!(out.bytes, before);
            assert!(write_enemy_client(&mut out, current, false).is_err());
            assert_eq!(out.bytes, before);
        }
        for events in [
            fired,
            game_events::EventQueue {
                sequence: 1,
                ..Default::default()
            },
            game_events::EventQueue {
                sequence: 0,
                slots: fired.slots,
            },
        ] {
            let nonzero = EnemySnapshot { events, ..initial };
            let mut out = Writer::default();
            out.bits(5, 3);
            out.bytes.push(0xe6);
            let before = out.bytes.clone();
            write_enemy_entity(&mut out, nonzero, None, true).unwrap();
            assert_ne!(out.bytes, before);
            write_enemy_client(&mut out, nonzero, true).unwrap();
            write_enemy_entity(&mut out, active, Some(nonzero), false).unwrap();
        }
    }

    #[test]
    fn rejects_invalid_identity_assets_animations_and_baselines_before_writing() {
        let initial = enemy();
        let invalid = [
            EnemySnapshot {
                entity_number: 0,
                ..initial
            },
            EnemySnapshot {
                entity_number: 2,
                ..initial
            },
            EnemySnapshot {
                client_index: 0,
                ..initial
            },
            EnemySnapshot {
                model: 0,
                ..initial
            },
            EnemySnapshot {
                model: 1024,
                ..initial
            },
            EnemySnapshot {
                legs_animation: 0,
                ..initial
            },
            EnemySnapshot {
                legs_animation: 0x400,
                ..initial
            },
            EnemySnapshot {
                torso_animation: 2048,
                ..initial
            },
            EnemySnapshot {
                weapon: 0,
                ..initial
            },
            EnemySnapshot {
                weapon: 900,
                ..initial
            },
            EnemySnapshot {
                attachment_models: [3, 0, 2, 0, 0, 0, 0, 0, 0, 0],
                ..initial
            },
            EnemySnapshot {
                attachment_tags: [64, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                ..initial
            },
            EnemySnapshot {
                attachment_tags: [0, 0, 0, 0, 1, 0, 0, 0, 0, 0],
                ..initial
            },
            EnemySnapshot {
                origin: [f32::NAN, 0.0, 0.0],
                ..initial
            },
            EnemySnapshot {
                origin: [0.0, 32768.0, 0.0],
                ..initial
            },
        ];
        for enemy in invalid {
            let mut out = Writer::default();
            out.bits(5, 3);
            let before = out.bytes.clone();
            assert!(write_enemy_entity(&mut out, enemy, None, true).is_err());
            assert_eq!(out.bytes, before);
            assert!(write_enemy_client(&mut out, enemy, true).is_err());
            assert_eq!(out.bytes, before);
        }
        let mut out = Writer::default();
        assert!(write_enemy_entity(&mut out, initial, None, false).is_ok());
        out = Writer::default();
        assert!(write_enemy_entity(&mut out, initial, Some(initial), true).is_err());
        let dead = EnemySnapshot {
            alive: false,
            ..initial
        };
        assert!(write_enemy_entity(&mut out, dead, None, true).is_ok());
        assert!(write_enemy_client(&mut out, dead, true).is_ok());
        out = Writer::default();
        assert!(write_enemy_entity(&mut out, initial, Some(dead), false).is_err());
        let changed = EnemySnapshot {
            model: 109,
            ..initial
        };
        assert!(write_enemy_entity(&mut out, changed, Some(initial), false).is_err());
        assert!(out.bytes.is_empty());
    }
}
