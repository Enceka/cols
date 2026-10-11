//! Bounded bootstrap snapshot probe for 3.3.3.3, NOT a world simulator.
//! Parser: 0x46d170; playerState: 0x5a4230; table: 0xc55f70 (235 fields).
use crate::{game_events, game_state::Writer, game_wire::SERVER_EOF, lobby::LobbyError};

#[derive(Debug, Clone, Copy)]
pub struct Ammo {
    pub clip_key: u16,
    pub reserve_key: u16,
    pub clip: u16,
    pub reserve: u16,
}
#[derive(Debug, Clone, Copy)]
pub struct Equipment {
    pub weapon: u16,
    /// Owned inventory slot for a held alternate mode; the parent owns it.
    pub inventory_weapon: Option<u16>,
    /// PS+0x344: paired presentation weapon for a models-only alternate.
    pub alternate_weapon: Option<u16>,
    /// PS+0x284: native shots in the current trigger press, clamped at4.
    pub fire_count: u8,
    /// PS+0x280 bit0: a new trigger edge buffered during a burst/recovery.
    pub queued_burst: bool,
    pub spread_scale: f32,
    /// Inventory slot flag3, PS+0x2e3: bolt still needs rechambering.
    pub needs_rechamber: bool,
    pub ammo: Option<Ammo>,
    pub viewmodel: Option<u16>,
    pub firing: Option<Firing>,
    pub aim: Option<Aim>,
    pub secondary: Option<EquippedWeapon>,
    pub events: Option<crate::game_events::EventQueue>,
}
impl Equipment {
    /// Native0x5a18e0: primary, paired presentation, then offhand.
    pub fn input_weapons(self) -> [u16; 3] {
        [self.weapon, self.alternate_weapon.unwrap_or(0), 0]
    }
}
#[derive(Debug, Clone, Copy)]
pub struct EquippedWeapon {
    pub weapon: u16,
    pub ammo: Option<Ammo>,
    pub needs_rechamber: bool,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct Aim {
    pub fraction: f32,
    pub button: bool,
    pub movement_slow: bool,
}
/// Authoritative movement fields from the target playerState layout. Position
/// remains the frame's `origin` argument so it has a single source of truth.
#[derive(Debug, Clone, Copy, Default)]
pub struct MotionSnapshot {
    pub velocity: [f32; 3],
    pub flags: u32,
    pub ground_entity: u16,
    pub view_height: f32,
    pub view_height_target: u8,
    pub speed: u16,
    pub gravity: u16,
    pub bob_cycle: u8,
    pub view_height_lerp_time: u32,
    pub view_height_lerp_target: u8,
    pub view_height_lerp_down: bool,
    pub jump_time: u32,
    pub jump_origin_z: f32,
}
#[derive(Debug, Clone, Copy)]
pub struct Firing {
    pub animation: u16,
    pub remaining_msec: u16,
    pub delay_msec: u16,
    pub state: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerVitals {
    pub health: u16,
    pub damage_event: u8,
    pub damage_count: u8,
    pub spawn_count: u8,
    pub intermission: bool,
    pub corpse_slot: u8,
}
impl Default for PlayerVitals {
    fn default() -> Self {
        Self {
            health: 100,
            damage_event: 0,
            damage_count: 0,
            spawn_count: 1,
            intermission: false,
            corpse_slot: 1,
        }
    }
}

/// 3.3.3.3: clientNum +0x144, deltaTime +0xa68; 0x51b3f0 owns killcam entry/exit.
#[derive(Debug, Clone, Copy)]
pub struct ReplayView {
    pub client: u8,
    pub delay_msec: u32,
    pub end_time: u32,
    pub skip_text: Option<u16>,
}

/// A full baseline with command time and health, no entities/clients/weapons.
/// All 14 target array sections are present; this is not a playable spawn.
pub fn bootstrap(server_time: u32) -> Result<Vec<u8>, LobbyError> {
    bootstrap_at_origin(server_time, None)
}

pub fn bootstrap_at_origin(
    server_time: u32,
    origin: Option<[f32; 3]>,
) -> Result<Vec<u8>, LobbyError> {
    frame(server_time, origin, true)
}

/// After a baseline ACK, delta-distance zero with flags0 selects the cached
/// full baseline for section histories; playerState still selects no base.
pub fn frame(
    server_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
) -> Result<Vec<u8>, LobbyError> {
    equipped_frame(server_time, origin, establish_baseline, None)
}

pub fn equipped_frame(
    server_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
    equipment: Option<Equipment>,
) -> Result<Vec<u8>, LobbyError> {
    aimed_frame(server_time, origin, establish_baseline, equipment, [0; 3])
}

pub fn aimed_frame(
    server_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
    equipment: Option<Equipment>,
    angles: [u16; 3],
) -> Result<Vec<u8>, LobbyError> {
    player_frame(
        server_time,
        server_time,
        origin,
        establish_baseline,
        equipment,
        angles,
    )
}

/// Snapshot time and the last applied input time are distinct clocks. The
/// target's 0x4522b0 skips prediction and restores old angles when commandTime
/// is at or ahead of its newest local command.
pub fn player_frame(
    server_time: u32,
    command_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
    equipment: Option<Equipment>,
    angles: [u16; 3],
) -> Result<Vec<u8>, LobbyError> {
    player_frame_with_motion(
        server_time,
        command_time,
        origin,
        establish_baseline,
        equipment,
        angles,
        None,
    )
}

pub fn player_frame_with_motion(
    server_time: u32,
    command_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
    equipment: Option<Equipment>,
    angles: [u16; 3],
    motion: Option<MotionSnapshot>,
) -> Result<Vec<u8>, LobbyError> {
    player_frame_with_world(
        server_time,
        command_time,
        origin,
        establish_baseline,
        equipment,
        angles,
        WorldSnapshot {
            delta_distance: 0,
            corpse_baselines: &[],
            impact_baselines: &[],
            obituary_baselines: &[],
            motion,
            target: None,
            impacts: &[],
            enemy: None,
            enemy_baseline: None,
            players: &[],
            player_baselines: &[],
            hit_feedback: None,
            death_waypoints: None,
            replay: None,
            local_card: None,
            local_character: None,
            held_weapon_tag: None,
            held_weapons_in_baseline: false,
            vitals: None,
            local_player: None,
            local_player_baseline: None,
            corpses: &[],
            obituaries: &[],
        },
    )
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorldSnapshot<'a> {
    pub delta_distance: u8,
    pub motion: Option<MotionSnapshot>,
    pub target: Option<crate::game_target::TargetSnapshot>,
    pub impacts: &'a [crate::game_impact::ImpactSnapshot],
    pub enemy: Option<crate::game_enemy::EnemySnapshot>,
    pub enemy_baseline: Option<crate::game_enemy::EnemySnapshot>,
    pub players: &'a [crate::game_enemy::EnemySnapshot],
    pub player_baselines: &'a [crate::game_enemy::EnemySnapshot],
    pub hit_feedback: Option<crate::game_hit_feedback::HitFeedbackSnapshot>,
    pub death_waypoints: Option<&'a [crate::game_hit_feedback::DeathWaypointSnapshot]>,
    pub replay: Option<ReplayView>,
    pub local_card: Option<crate::game_player_card::Card>,
    pub local_character: Option<crate::game_character::Character>,
    pub held_weapon_tag: Option<u8>,
    pub held_weapons_in_baseline: bool,
    pub vitals: Option<PlayerVitals>,
    pub local_player: Option<crate::game_enemy::EnemySnapshot>,
    pub local_player_baseline: Option<crate::game_enemy::EnemySnapshot>,
    pub corpses: &'a [crate::game_corpse::CorpseSnapshot],
    pub corpse_baselines: &'a [crate::game_corpse::CorpseSnapshot],
    pub impact_baselines: &'a [crate::game_impact::ImpactSnapshot],
    pub obituaries: &'a [crate::game_obituary::ObituarySnapshot],
    pub obituary_baselines: &'a [crate::game_obituary::ObituarySnapshot],
}

pub fn player_frame_with_world(
    server_time: u32,
    command_time: u32,
    origin: Option<[f32; 3]>,
    establish_baseline: bool,
    equipment: Option<Equipment>,
    angles: [u16; 3],
    world: WorldSnapshot<'_>,
) -> Result<Vec<u8>, LobbyError> {
    let motion = world.motion;
    let vitals = world.vitals;
    if vitals.is_some_and(|v| {
        v.health > 100 || v.damage_count > 127 || v.corpse_slot >= 8 || motion.is_none()
    }) {
        return Err(LobbyError("invalid authoritative player vitals"));
    }
    if world.replay.is_some_and(|r| {
        r.client >= 12
            || !(1..=10_000).contains(&r.delay_msec)
            || r.end_time <= server_time
            || r.end_time - server_time > 10_000
            || motion.is_none()
            || equipment.is_none()
            || vitals.is_none()
    }) {
        return Err(LobbyError("invalid archived replay view"));
    }
    let weapon = equipment.map(|e| e.weapon);
    let inventory_weapon = equipment.and_then(|e| e.inventory_weapon).or(weapon);
    let alternate_weapon = equipment.and_then(|e| e.alternate_weapon);
    if alternate_weapon.is_some_and(|w| w == 0 || w >= 900)
        || equipment.is_some_and(|e| {
            e.fire_count > 4
                || !e.spread_scale.is_finite()
                || !(0.0..=255.0).contains(&e.spread_scale)
        })
    {
        return Err(LobbyError("invalid weapon prediction state"));
    }
    if inventory_weapon.is_some_and(|w| w == 0 || w >= 900) {
        return Err(LobbyError("invalid inventory weapon"));
    }
    let ammo = equipment.and_then(|e| e.ammo);
    let secondary = equipment.and_then(|e| e.secondary);
    let events = equipment.and_then(|e| e.events);
    if let Some(events) = events {
        events.validate()?;
    }
    if secondary.is_some_and(|other| {
        other.weapon == 0 || other.weapon >= 900 || Some(other.weapon) == weapon
    }) {
        return Err(LobbyError("invalid secondary weapon"));
    }
    if motion.is_some_and(|m| {
        origin.is_none()
            || m.velocity
                .iter()
                .any(|n| !n.is_finite() || n.abs() > 1_000_000.0)
            || !m.view_height.is_finite()
            || !(0.0..=127.0).contains(&m.view_height)
            || m.view_height_target > 127
            || m.view_height_lerp_target > 127
            || !m.jump_origin_z.is_finite()
            || m.jump_origin_z.abs() > 1_000_000.0
            || m.ground_entity > 2047
            || m.view_height_lerp_time > command_time
            || m.jump_time > command_time
    }) {
        return Err(LobbyError("invalid authoritative movement"));
    }
    let ammunition: Vec<Ammo> = ammo
        .into_iter()
        .chain(secondary.and_then(|other| other.ammo))
        .collect();
    let mut clips = Vec::<Ammo>::new();
    let mut reserves = Vec::<Ammo>::new();
    for record in &ammunition {
        if let Some(previous) = clips.iter().find(|a| a.clip_key == record.clip_key) {
            if previous.clip != record.clip {
                return Err(LobbyError("conflicting shared clip ammunition"));
            }
        } else {
            clips.push(*record);
        }
        if !reserves.iter().any(|a| a.reserve_key == record.reserve_key) {
            reserves.push(*record);
        }
    }
    let viewmodel = equipment.and_then(|e| e.viewmodel);
    let firing = equipment.and_then(|e| e.firing);
    let aim = equipment.and_then(|e| e.aim);
    if aim.is_some_and(|a| !a.fraction.is_finite() || !(0.0..=1.0).contains(&a.fraction)) {
        return Err(LobbyError("invalid bootstrap ADS fraction"));
    }
    if firing.is_some_and(|f| {
        f.animation > 2047 || f.remaining_msec > 32767 || f.delay_msec > 32767 || f.state > 63
    }) {
        return Err(LobbyError("invalid bootstrap firing state"));
    }
    if viewmodel.is_some_and(|i| i == 0 || i >= 1024) {
        return Err(LobbyError("invalid viewmodel index"));
    }
    if ammunition
        .iter()
        .any(|a| a.clip_key > 1022 || a.reserve_key > 1023 || a.clip > 511 || a.reserve > 2047)
    {
        return Err(LobbyError("invalid bootstrap ammo"));
    }
    if weapon.is_some_and(|w| w == 0 || w >= 900) || (weapon.is_some() && origin.is_none()) {
        return Err(LobbyError("invalid bootstrap equipment"));
    }
    if weapon.is_some()
        && origin.is_some_and(|v| v.iter().any(|x| !(-32768.0..=32767.0).contains(x)))
    {
        return Err(LobbyError(
            "entity origin exceeds current zero map-center range",
        ));
    }
    if server_time == 0 || server_time > i32::MAX as u32 {
        return Err(LobbyError("snapshot time out of range"));
    }
    if command_time > server_time {
        return Err(LobbyError("input time exceeds snapshot time"));
    }
    if origin.is_some_and(|v| v.iter().any(|n| !n.is_finite() || n.abs() > 1_000_000.0)) {
        return Err(LobbyError("bootstrap origin out of range"));
    }
    let mut out = Writer::default();
    out.bytes.push(0); // svc_snapshot, byte pool starts at zero
    out.bytes.extend_from_slice(&server_time.to_le_bytes());
    out.bytes.extend_from_slice(&[
        if establish_baseline {
            0
        } else {
            world.delta_distance
        },
        if establish_baseline { 8 } else { 0 },
    ]);
    out.bits(0, 1); // no prior playerState
    out.bits(1, 1); // transmit origin/velocity rather than local prediction
    out.bits(
        if motion.is_some() {
            228
        } else if weapon.is_some() {
            88
        } else if origin.is_some() {
            69
        } else {
            1
        },
        8,
    );
    out.bits(1, 1); // field gap +1 from -1
    out.bits(1, 1); // -97 codec, absolute LE32 (0x5a20c0)
    out.bytes.extend_from_slice(&command_time.to_le_bytes());
    if let Some(origin) = origin {
        // Target field table 0xc55f70. Hint-2 fields use the later tail pass.
        // Equipment and angles must also be reflected in the input delta base.
        let mut previous = 0;
        let mut fields = vec![
            (8, origin[0].to_bits(), 0),
            (10, origin[2].to_bits(), 0),
            (11, origin[1].to_bits(), 0),
            (34, motion.map_or(60.0, |m| m.view_height).to_bits(), 0),
            (
                38,
                u32::from(motion.map_or(60, |m| m.view_height_target)),
                8,
            ),
            (47, u32::from(motion.map_or(2047, |m| m.ground_entity)), 11),
            (61, 2047, 16),
            (68, 2047, 11),
        ];
        if let Some(motion) = motion {
            fields.extend([
                (12, u32::from(motion.bob_cycle), 8),
                (13, motion.velocity[1].to_bits(), 0),
                (14, motion.velocity[0].to_bits(), 0),
                (16, motion.velocity[2].to_bits(), 0),
                (58, 1f32.to_bits(), 100),
                (60, u32::from(motion.speed), 16),
                (63, u32::from(motion.gravity), 16),
                (81, u32::from(motion.view_height_lerp_target), 8),
                (92, motion.view_height_lerp_time, 32),
                (102, motion.jump_time, 32),
                (114, motion.jump_origin_z.to_bits(), 100),
                (217, 1f32.to_bits(), 100),
                // 0x51d2a0 applies this entity's rotation to the camera.
                // Zero selects entity0 and adds our own body yaw a second
                // time. An unlinked first-person camera uses ENTITYNUM_NONE.
                (227, 2047, 11),
                (136, 2047, 11), // No projectile/vehicle killcam target.
                (137, 2047, 11),
            ]);
        }
        if let Some(replay) = world.replay {
            fields.extend([
                (59, u32::from(replay.client), 8),
                (142, replay.delay_msec, 32),
            ]);
        }
        if let Some(events) = events {
            fields.push((1, u32::from(events.sequence), 75));
            for slot in 0..4 {
                fields.push((21 + slot, slot, 94));
            }
        }
        if let Some(vitals) = vitals {
            // Current PS table: received damage event/count and the verified
            // directionless255/255 branch; PM_DEAD is8 in this build.
            fields.extend([
                (55, 255, 8),
                (66, u32::from(vitals.damage_event), 8),
                (73, 255, 8),
                (74, u32::from(vitals.damage_count), 7),
            ]);
            if vitals.intermission {
                fields.push((83, 6, 8)); // 0x41dba0 case6 skips movement and weapons
            } else if vitals.health == 0 {
                fields.extend([(83, 8, 8), (84, u32::from(vitals.corpse_slot), 3)]);
            }
        }
        if let Some(weapon) = weapon {
            if equipment.unwrap().queued_burst {
                fields.push((121, 1, 1));
            }
            // 0x422e5c/0x42956f select idle0 with the0x400 restart bit.
            fields.extend([
                (6, equipment.unwrap().spread_scale.to_bits(), 0),
                (9, u32::from(firing.map_or(0x400, |f| f.animation)), 11),
                (39, u32::from(weapon), 10),
                (40, u32::from(alternate_weapon.unwrap_or(0)), 10),
                (87, u32::from(equipment.unwrap().fire_count), 3),
                (
                    54,
                    if world.replay.is_some() || vitals.is_some_and(|v| v.intermission) {
                        0x800
                    } else {
                        0x1000
                    },
                    28,
                ),
            ]);
            if let Some(firing) = firing {
                if firing.remaining_msec != 0 {
                    fields.push((3, u32::from(firing.remaining_msec), 16));
                }
                if firing.state != 0 {
                    fields.push((17, u32::from(firing.state), 6));
                }
                if firing.delay_msec != 0 {
                    fields.push((44, u32::from(firing.delay_msec), 16));
                }
            }
        }
        if let Some(model) = viewmodel {
            fields.push((56, u32::from(model), 10));
        }
        // Motion and ADS share PS.pm_flags. Emit their combined word once.
        let flags = motion.map_or(0, |m| m.flags)
            | aim.map_or(0, |a| {
                (u32::from(a.button) * 0x10) | (u32::from(a.movement_slow) * 0x40)
            });
        if flags != 0 {
            fields.push((20, flags, 32));
        }
        if let Some(aim) = aim
            && aim.fraction != 0.0
        {
            fields.push((41, aim.fraction.to_bits(), 0));
        }
        // Marker87 is the target's -87 short-angle codec, with no nonzero bit.
        for (index, value) in [(19, angles[0]), (18, angles[1]), (29, angles[2])] {
            if value != 0 {
                fields.push((index, u32::from(value), 87));
            }
        }
        fields.sort_by_key(|f| f.0);
        for (index, value, width) in fields {
            gap(&mut out, index - previous);
            previous = index;
            if width == 0 {
                // target -88: full raw float XOR with zero baseline
                out.bytes.extend_from_slice(&value.to_le_bytes());
            } else if width == 87 {
                out.bytes.extend_from_slice(&(value as u16).to_le_bytes());
            } else if width == 75 {
                game_events::write_sequence(&mut out, value as u8);
            } else if width == 94 {
                game_events::write_event(&mut out, events.unwrap().slots[value as usize]);
            } else if width == 100 {
                // Generic float codec 0 differs from raw-float codec -88:
                // nonzero + full-precision gates, then the raw XOR word.
                out.bits(1, 1);
                out.bits(1, 1);
                out.bytes.extend_from_slice(&value.to_le_bytes());
            } else {
                // 0x5a27f0 nonzero gate -> 0x5a2510 -> 0x5a2470:
                // remainder bits first, then whole bytes (not all pool bits).
                out.bits(1, 1);
                let remainder = width % 8;
                out.bits(value, remainder);
                for shift in (remainder..width).step_by(8) {
                    out.bytes.push((value >> shift) as u8);
                }
            }
        }
    }
    if let Some(motion) = motion {
        // 0x5a4541..0x5a458d: all hint-2 fields strictly below the last
        // ordinary field, in table order. No gaps and no enclosing gate.
        // 82(-101),89(1),93(1),98(1),101(1),104(16),106(1),122(-57).
        out.bits(0, 1); // 82: preserve the initialized protected float.
        out.bits(u32::from(motion.view_height_lerp_down), 1); // 89 nonzero gate
        if motion.view_height_lerp_down {
            out.bits(1, 1);
        }
        out.bits(0, 6); // Five zero numeric gates, then direct bool at122.
    }
    // 0x5a463d: stats presence, 4-bit mask, first stat is a short.
    out.bits(1, 1);
    out.bits(
        (if weapon.is_some() { 5 } else { 1 }) | if vitals.is_some() { 8 } else { 0 },
        4,
    );
    out.bytes
        .extend_from_slice(&vitals.map_or(100, |v| v.health).to_le_bytes());
    if weapon.is_some() {
        out.bytes.extend_from_slice(&100u16.to_le_bytes()); // stats[2], max health
        if let Some(vitals) = vitals {
            out.bytes.push(vitals.spawn_count);
        } // current stats[3], directbyte
        for (slot, ammo) in clips.iter().enumerate() {
            out.bits(1, 1);
            out.bits(slot as u32, 4);
            out.bits(u32::from(ammo.clip_key), 10);
            out.bits(1, 1);
            out.bits(u32::from(ammo.clip), 9);
            out.bits(1, 1);
            out.bits(0, 9); // explicitly clear offhand magazine
        }
        out.bits(0, 1); // clip list terminator
        out.bits(1, 1); // equipped weapon section
        out.bits(1, 1); // slot0 changed
        out.bits(1, 1); // weapon ID present
        out.bits(u32::from(inventory_weapon.unwrap()), 10);
        out.bits(u32::from(equipment.unwrap().needs_rechamber) << 3, 5);
        if let Some(other) = secondary {
            out.bits(1, 1); // slot1 changed
            out.bits(1, 1); // weapon ID present
            out.bits(u32::from(other.weapon), 10);
            out.bits(u32::from(other.needs_rechamber) << 3, 5);
            out.bits(0, 13); // slots2..14 unchanged
        } else {
            out.bits(0, 14); // slots1..14 unchanged
        }
        for (slot, ammo) in reserves.iter().enumerate() {
            out.bits(1, 1);
            out.bits(slot as u32, 4);
            out.bits(u32::from(ammo.reserve_key), 10);
            out.bits(u32::from(ammo.reserve), 11);
        }
        out.bits(0, 1); // reserve terminator
        out.bits(0, 5); // four extension groups + objectives
    } else {
        if let Some(vitals) = vitals {
            out.bytes.push(vitals.spawn_count);
        }
        // Clip, equipped weapons, reserve; four extended bundles; objectives,
        // HUD; four final indexed lists. 3.3.3.3 has fourteen gates in total.
        out.bits(0, 8);
    }
    if world.hit_feedback.is_some() || world.death_waypoints.is_some() || world.replay.is_some() {
        crate::game_hit_feedback::write_hud(
            &mut out,
            world.hit_feedback,
            world.death_waypoints.unwrap_or(&[]),
            world
                .replay
                .map(|r| crate::game_hit_feedback::ReplayHudSnapshot {
                    end_time: r.end_time,
                    skip_text: r.skip_text,
                }),
        )?;
    } else {
        out.bits(0, 1); // HUD
    }
    out.bits(0, 4); // four final indexed lists
    out.bits(0, 1); // animation command section (0x5a3b50)
    out.bits(0, 1); // actors (0x46cc60)
    if let Some(player) = world.local_player {
        crate::game_enemy::write_enemy_entity(
            &mut out,
            player,
            if establish_baseline {
                None
            } else {
                world.local_player_baseline
            },
            establish_baseline,
        )?;
    } else if establish_baseline && weapon.is_some() {
        // Target type1 table 0xc52b68. Entity0 has the local player type and
        // map-derived origin. Ordinary frames inherit it from the cached base.
        out.bits(1, 1); // entity index -1 -> 0
        out.bits(0, 1);
        out.bits(1, 1);
        out.bits(0, 1); // retained, changed, origin flag
        out.bits(5, 6); // last changed index (not field count)
        out.bits(1, 1);
        out.bytes.push(1); // first field eType, codec -77 raw byte
        let origin = origin.expect("validated equipped origin");
        for (distance, coordinate) in [(1, origin[1]), (1, origin[0]), (3, origin[2])] {
            out.bits(u32::from(distance == 1), 1);
            if distance != 1 {
                out.bits(distance - 1, 3);
            }
            out.bits(1, 1); // full 16-bit position codec; baseline/map center zero
            out.bits(u32::from(coordinate.round_ties_even() as i16 as u16), 16);
        }
    }
    if let Some(enemy) = world.enemy {
        crate::game_enemy::write_enemy_entity(
            &mut out,
            enemy,
            if establish_baseline {
                None
            } else {
                world.enemy_baseline
            },
            establish_baseline,
        )?;
    }
    for (index, player) in world.players.iter().enumerate() {
        if (index > 0 && world.players[index - 1].client_index >= player.client_index)
            || player.client_index == 0
            || world.enemy.is_some()
        {
            return Err(LobbyError("invalid player snapshot order"));
        }
        if world.delta_distance != 0 && world.player_baselines.iter().any(|b| b == player) {
            continue;
        }
        crate::game_enemy::write_enemy_entity(
            &mut out,
            *player,
            if establish_baseline {
                None
            } else {
                world
                    .player_baselines
                    .iter()
                    .find(|b| b.alive && b.client_index == player.client_index)
                    .copied()
            },
            establish_baseline,
        )?;
    }
    if let Some(target) = world.target {
        crate::game_target::write_target(&mut out, target, establish_baseline)?;
    }
    if !establish_baseline {
        for corpse in world.corpses {
            if world.delta_distance != 0
                && (world.corpse_baselines.iter().any(|b| b == corpse)
                    || (!corpse.present
                        && !world
                            .corpse_baselines
                            .iter()
                            .any(|b| b.present && b.entity_number == corpse.entity_number)))
            {
                continue;
            }
            crate::game_corpse::write_corpse(
                &mut out,
                *corpse,
                world
                    .corpse_baselines
                    .iter()
                    .find(|b| b.present && b.entity_number == corpse.entity_number)
                    .copied(),
                false,
            )?;
        }
    }
    // Keep the full character/client baseline bounded. Linked weapon models
    // follow in the next delta, like corpses; never assume they were sent just
    // because their parent was present in the acknowledged frame.
    if let Some(tag) = world.held_weapon_tag.filter(|_| !establish_baseline) {
        let reserved = crate::game_held_weapon::FIRST_ENTITY..crate::game_held_weapon::END_ENTITY;
        if world
            .impacts
            .iter()
            .chain(world.impact_baselines)
            .any(|i| reserved.contains(&i.entity_number))
            || world
                .obituaries
                .iter()
                .chain(world.obituary_baselines)
                .any(|i| reserved.contains(&i.entity_number))
        {
            return Err(LobbyError("held weapon entity overlaps an event"));
        }
        let current = crate::game_held_weapon::weapons(
            world
                .local_player
                .into_iter()
                .chain(world.enemy)
                .chain(world.players.iter().copied()),
            world.corpses,
        );
        let baseline = if !world.held_weapons_in_baseline {
            Default::default()
        } else {
            crate::game_held_weapon::weapons(
                world
                    .local_player_baseline
                    .into_iter()
                    .chain(world.enemy_baseline.filter(|_| world.enemy.is_some()))
                    .chain(world.player_baselines.iter().copied()),
                world.corpse_baselines,
            )
        };
        crate::game_held_weapon::write(&mut out, &current, &baseline, tag)?;
    }
    if world.delta_distance == 0 {
        crate::game_impact::write_impacts(&mut out, world.impacts, establish_baseline)?;
        crate::game_obituary::write_obituaries(&mut out, world.obituaries, establish_baseline)?;
    } else {
        crate::game_impact::write_impacts_with_baselines(
            &mut out,
            world.impacts,
            world.impact_baselines,
            establish_baseline,
        )?;
        crate::game_obituary::write_obituaries_with_baselines(
            &mut out,
            world.obituaries,
            world.obituary_baselines,
            establish_baseline,
        )?;
    }
    // Entities (0x46c2f0): explicit 11-bit sentinel, not a presence gate.
    out.bits(0, 1);
    out.bits(1, 1);
    out.bits(2047, 11);
    if let Some(local) = world.local_player {
        let mut players = vec![local];
        if let Some(enemy) = world.enemy {
            players.push(enemy);
        }
        players.extend_from_slice(world.players);
        crate::game_enemy::write_player_clients_with_card(
            &mut out,
            &players,
            establish_baseline,
            world.local_card,
            world.held_weapon_tag.is_some(),
            world.local_character,
        )?;
    } else if let Some(enemy) = world.enemy {
        if world.held_weapon_tag.is_some() {
            crate::game_enemy::write_player_clients_with_card(
                &mut out,
                &[enemy],
                establish_baseline,
                None,
                true,
                None,
            )?;
        } else {
            crate::game_enemy::write_enemy_client(&mut out, enemy, establish_baseline)?;
        }
    } else {
        out.bits(0, 1); // clients (0x46c840)
    }
    out.bits(0, 1); // glass (0x43e070)
    out.bytes.push(SERVER_EOF);
    Ok(out.bytes)
}

fn gap(out: &mut Writer, distance: u32) {
    out.bits(u32::from(distance == 1), 1);
    if distance == 1 {
        return;
    }
    let mut rest = distance - 1;
    while rest >= 15 {
        out.bits(15, 4);
        rest -= 15;
    }
    out.bits(rest, 4);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{game_events::EventQueue, game_wire::Reader};
    use std::collections::BTreeMap;

    // An independent reader driven by the extracted target table. The event
    // widths used here are measured entries from 0xc56d70, not writer helpers.
    fn read_fields(r: &mut Reader<'_>) -> (BTreeMap<usize, u32>, BTreeMap<usize, u32>) {
        let layout: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/player-state-fields-3.3.3.3.json"
        ))
        .unwrap();
        let integer = |r: &mut Reader<'_>, width: usize| {
            if r.bits(1).unwrap() == 0 {
                return 0;
            }
            let low = width % 8;
            let mut value = r.bits(low).unwrap();
            for bit in (low..width).step_by(8) {
                value |= u32::from(r.u8().unwrap()) << bit;
            }
            value
        };
        let last = r.bits(8).unwrap() as i32 - 1;
        let mut index = -1i32;
        let mut fields = BTreeMap::new();
        let mut parameters = BTreeMap::new();
        while index < last {
            let mut distance = 1;
            if r.bits(1).unwrap() == 0 {
                loop {
                    let part = r.bits(4).unwrap();
                    distance += part;
                    if part < 15 {
                        break;
                    }
                }
            }
            index += distance as i32;
            assert!(index <= last);
            let entry = &layout["fields"][index as usize];
            assert_ne!(
                entry[2], 2,
                "hint-2 must not appear in ordinary field stream"
            );
            let codec = entry[1].as_i64().unwrap();
            let value = match codec {
                -97 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    r.u32().unwrap()
                }
                -88 => r.u32().unwrap(),
                -87 => u32::from(r.u16().unwrap()),
                -75 => {
                    assert_eq!(r.bits(1).unwrap(), 0);
                    u32::from(r.u8().unwrap())
                }
                -94 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    let id = r.u8().unwrap();
                    let width = match id {
                        0 => 11,
                        24..=33 | 40 | 41 => 0,
                        34 | 35 => 10,
                        36 | 37 => 5,
                        _ => panic!("unexpected event in fixture test"),
                    };
                    let parameter = if width != 0 && r.bits(1).unwrap() != 0 {
                        r.bits(width).unwrap()
                    } else {
                        0
                    };
                    parameters.insert(index as usize, parameter);
                    u32::from(id)
                }
                0 => {
                    assert_eq!(r.bits(1).unwrap(), 1);
                    assert_eq!(r.bits(1).unwrap(), 1);
                    r.u32().unwrap()
                }
                _ => integer(r, codec.unsigned_abs() as usize),
            };
            fields.insert(index as usize, value);
        }
        for (index, entry) in layout["fields"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .take(last as usize)
        {
            if entry[2] != 2 {
                continue;
            }
            let codec = entry[1].as_i64().unwrap();
            let value = match codec {
                -101 => {
                    assert_eq!(r.bits(1).unwrap(), 0);
                    0
                }
                -57 => r.bits(1).unwrap(),
                _ => integer(r, codec.unsigned_abs() as usize),
            };
            fields.insert(index, value);
        }
        (fields, parameters)
    }

    #[test]
    fn killcam_fields_select_the_archived_player_and_clear_on_return() {
        let equipment = Equipment {
            weapon: 352,
            inventory_weapon: None,
            alternate_weapon: None,
            fire_count: 0,
            queued_burst: false,
            spread_scale: 0.0,
            needs_rechamber: false,
            ammo: None,
            viewmodel: Some(54),
            firing: None,
            aim: None,
            secondary: None,
            events: None,
        };
        let motion = MotionSnapshot {
            view_height: 60.0,
            view_height_target: 60,
            ground_entity: 2047,
            speed: 190,
            gravity: 800,
            ..Default::default()
        };
        for replay in [
            Some(ReplayView {
                client: 7,
                delay_msec: 4000,
                end_time: 10000,
                skip_text: Some(247),
            }),
            None,
        ] {
            let body = player_frame_with_world(
                9000,
                8950,
                Some([1.0, 2.0, 3.0]),
                true,
                Some(equipment),
                [0; 3],
                WorldSnapshot {
                    motion: Some(motion),
                    vitals: Some(PlayerVitals::default()),
                    replay,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut reader = Reader::new(&body);
            assert_eq!(reader.u8().unwrap(), 0);
            assert_eq!(reader.u32().unwrap(), 9000);
            reader.u8().unwrap();
            reader.u8().unwrap();
            assert_eq!(reader.bits(2).unwrap(), 2);
            let (fields, _) = read_fields(&mut reader);
            assert_eq!(fields[&54], if replay.is_some() { 0x800 } else { 0x1000 });
            assert_eq!(
                fields.get(&59).copied().unwrap_or(0),
                if replay.is_some() { 7 } else { 0 }
            );
            assert_eq!(
                fields.get(&142).copied().unwrap_or(0),
                if replay.is_some() { 4000 } else { 0 }
            );
            assert_eq!(fields[&136], 2047);
            assert_eq!(fields[&137], 2047);
        }
        for bad in [
            ReplayView {
                client: 12,
                delay_msec: 4000,
                end_time: 10000,
                skip_text: None,
            },
            ReplayView {
                client: 7,
                delay_msec: 0,
                end_time: 10000,
                skip_text: None,
            },
        ] {
            assert!(
                player_frame_with_world(
                    9000,
                    8950,
                    Some([0.0; 3]),
                    true,
                    Some(equipment),
                    [0; 3],
                    WorldSnapshot {
                        motion: Some(motion),
                        vitals: Some(PlayerVitals::default()),
                        replay: Some(bad),
                        ..Default::default()
                    }
                )
                .is_err()
            );
        }
    }

    #[test]
    fn movement_events_and_two_weapons_decode_with_the_target_field_and_array_layout() {
        let mut events = EventQueue::default();
        events
            .push(game_events::WEAPON_SWITCH_STARTED, 352)
            .unwrap();
        events.push(game_events::PUT_AWAY_WEAPON, 0).unwrap();
        events.push(game_events::RAISE_WEAPON, 0).unwrap();
        events.push(game_events::FIRE_WEAPON_LAST_SHOT, 0).unwrap();
        let equipment = Equipment {
            inventory_weapon: None,
            alternate_weapon: Some(180),
            fire_count: 1,
            queued_burst: false,
            spread_scale: 0.0,
            needs_rechamber: false,
            weapon: 352,
            ammo: Some(Ammo {
                clip_key: 352,
                reserve_key: 163,
                clip: 29,
                reserve: 90,
            }),
            secondary: Some(EquippedWeapon {
                needs_rechamber: false,
                weapon: 180,
                ammo: Some(Ammo {
                    clip_key: 180,
                    reserve_key: 163,
                    clip: 9,
                    reserve: 90,
                }),
            }),
            aim: Some(Aim {
                fraction: 0.25,
                button: true,
                movement_slow: true,
            }),
            viewmodel: Some(54),
            firing: None,
            events: Some(events),
        };
        for lerp_down in [false, true] {
            let motion = MotionSnapshot {
                velocity: [127.25, -52.5, 190.0],
                flags: 0x400002,
                ground_entity: 2046,
                view_height: 47.5,
                view_height_target: 40,
                speed: 190,
                gravity: 800,
                bob_cycle: 201,
                view_height_lerp_time: 800,
                view_height_lerp_target: 40,
                view_height_lerp_down: lerp_down,
                jump_time: 700,
                jump_origin_z: -123.75,
            };
            let body = player_frame_with_motion(
                1000,
                950,
                Some([25.0, 0.0, -113.4]),
                false,
                Some(Equipment {
                    queued_burst: lerp_down,
                    ..equipment
                }),
                [0, 1234, 0],
                Some(motion),
            )
            .unwrap();
            let mut r = Reader::new(&body);
            assert_eq!(r.u8().unwrap(), 0);
            assert_eq!(r.u32().unwrap(), 1000);
            assert_eq!(r.u8().unwrap(), 0);
            assert_eq!(r.u8().unwrap(), 0);
            assert_eq!(r.bits(2).unwrap(), 2); // Zero PS baseline, authoritative movement.
            let (fields, parameters) = read_fields(&mut r);
            assert_eq!(fields.get(&121).copied().unwrap_or(0), u32::from(lerp_down));
            for (index, expected) in [
                (0, 950),
                (1, 4),
                (12, 201),
                (13, (-52.5f32).to_bits()),
                (14, 127.25f32.to_bits()),
                (16, 190f32.to_bits()),
                (20, 0x400052),
                (21, 34),
                (22, 32),
                (23, 30),
                (24, 37),
                (34, 47.5f32.to_bits()),
                (38, 40),
                (47, 2046),
                (58, 1f32.to_bits()),
                (60, 190),
                (63, 800),
                (81, 40),
                (89, u32::from(lerp_down)),
                (92, 800),
                (102, 700),
                (114, (-123.75f32).to_bits()),
                (217, 1f32.to_bits()),
                (227, 2047),
            ] {
                assert_eq!(fields[&index], expected, "field {index}");
            }
            assert_eq!(
                parameters,
                BTreeMap::from([(21, 352), (22, 0), (23, 0), (24, 0)])
            );
            for index in [82, 93, 98, 101, 104, 106, 122] {
                assert_eq!(fields[&index], 0);
            }
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(4).unwrap(), 5);
            assert_eq!(r.u16().unwrap(), 100);
            assert_eq!(r.u16().unwrap(), 100);
            for (slot, key, rounds) in [(0, 352, 29), (1, 180, 9)] {
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.bits(4).unwrap(), slot);
                assert_eq!(r.bits(10).unwrap(), key);
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.bits(9).unwrap(), rounds);
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.bits(9).unwrap(), 0);
            }
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.bits(1).unwrap(), 1); // Equipped array.
            for id in [352, 180] {
                assert_eq!(r.bits(2).unwrap(), 3);
                assert_eq!(r.bits(10).unwrap(), id);
                assert_eq!(r.bits(5).unwrap(), 0);
            }
            assert_eq!(r.bits(13).unwrap(), 0);
            assert_eq!(r.bits(1).unwrap(), 1); // Only one shared reserve record.
            assert_eq!(r.bits(4).unwrap(), 0);
            assert_eq!(r.bits(10).unwrap(), 163);
            assert_eq!(r.bits(11).unwrap(), 90);
            assert_eq!(r.bits(13).unwrap(), 0); // Reserve EOF, ten arrays, anim, actor.
            assert_eq!(r.bits(2).unwrap(), 2);
            assert_eq!(r.bits(11).unwrap(), 2047);
            assert_eq!(r.bits(2).unwrap(), 0);
            assert_eq!(r.u8().unwrap(), SERVER_EOF);
            r.finish().unwrap();
        }
    }
    #[test]
    fn equipped_snapshot_matches_target_field_array_and_entity_sections() {
        use crate::game_wire::Reader;
        let layout: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/player-state-fields-3.3.3.3.json"
        ))
        .unwrap();
        let equipment = Equipment {
            inventory_weapon: None,
            alternate_weapon: Some(180),
            fire_count: 1,
            queued_burst: false,
            spread_scale: 0.0,
            needs_rechamber: true,
            secondary: None,
            events: None,
            weapon: 163,
            aim: Some(Aim {
                fraction: 0.5,
                button: true,
                movement_slow: true,
            }),
            firing: Some(Firing {
                animation: 0x402,
                remaining_msec: 79,
                delay_msec: 40,
                state: 6,
            }),
            viewmodel: Some(53),
            ammo: Some(Ammo {
                clip_key: 163,
                reserve_key: 163,
                clip: 30,
                reserve: 90,
            }),
        };
        assert_eq!(equipment.input_weapons(), [163, 180, 0]);
        for baseline in [true, false] {
            let body = player_frame(
                1000,
                925,
                Some([-472.0, -800.0, -113.4]),
                baseline,
                Some(equipment),
                [65535, 16384, 0],
            )
            .unwrap();
            let mut r = Reader::new(&body);
            assert_eq!(r.u8().unwrap(), 0);
            assert_eq!(r.u32().unwrap(), 1000);
            assert_eq!(r.u8().unwrap(), 0);
            assert_eq!(r.u8().unwrap(), if baseline { 8 } else { 0 });
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.bits(1).unwrap(), 1);
            let last = r.bits(8).unwrap() as i32 - 1;
            let mut index = -1i32;
            let mut fields = std::collections::BTreeMap::new();
            while index < last {
                let mut distance = 1;
                if r.bits(1).unwrap() == 0 {
                    loop {
                        let n = r.bits(4).unwrap();
                        distance += n;
                        if n < 15 {
                            break;
                        }
                    }
                }
                index += distance as i32;
                let codec = layout["fields"][index as usize][1].as_i64().unwrap();
                let value = match codec {
                    -97 => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        r.u32().unwrap()
                    }
                    -88 => r.u32().unwrap(),
                    -87 => u32::from(r.u16().unwrap()),
                    _ => {
                        assert_eq!(r.bits(1).unwrap(), 1);
                        let width = codec.unsigned_abs() as usize;
                        let low = width % 8;
                        let mut value = r.bits(low).unwrap();
                        for bit in (low..width).step_by(8) {
                            value |= u32::from(r.u8().unwrap()) << bit;
                        }
                        value
                    }
                };
                fields.insert(index, value);
            }
            assert_eq!(fields[&39], 163);
            assert_eq!(fields[&40], 180);
            assert_eq!(fields[&87], 1);
            assert_eq!(fields[&0], 925);
            assert_eq!(fields[&56], 53);
            assert_eq!(fields[&9], 0x402);
            assert_eq!(fields[&3], 79);
            assert_eq!(fields[&17], 6);
            assert_eq!(fields[&20], 0x50);
            assert_eq!(fields[&41], 0.5f32.to_bits());
            assert_eq!(fields[&44], 40);
            assert_eq!(fields[&19], 65535);
            assert_eq!(fields[&18], 16384);
            assert_eq!(fields[&54], 0x1000);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(4).unwrap(), 5);
            assert_eq!(r.u16().unwrap(), 100);
            assert_eq!(r.u16().unwrap(), 100);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(4).unwrap(), 0);
            assert_eq!(r.bits(10).unwrap(), 163);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(9).unwrap(), 30);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(9).unwrap(), 0);
            assert_eq!(r.bits(1).unwrap(), 0);
            assert_eq!(r.bits(3).unwrap(), 7);
            assert_eq!(r.bits(10).unwrap(), 163);
            assert_eq!(r.bits(5).unwrap(), 8); // inventory flag3 survives authoritative correction
            assert_eq!(r.bits(14).unwrap(), 0);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(4).unwrap(), 0);
            assert_eq!(r.bits(10).unwrap(), 163);
            assert_eq!(r.bits(11).unwrap(), 90);
            assert_eq!(r.bits(13).unwrap(), 0); // reserve EOF, ten arrays, anim, actors
            if baseline {
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.bits(3).unwrap(), 2);
                assert_eq!(r.bits(6).unwrap(), 5);
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.u8().unwrap(), 1);
                for (delta, expected) in [(1, -800i16), (1, -472), (3, -113)] {
                    assert_eq!(r.bits(1).unwrap(), u32::from(delta == 1));
                    if delta != 1 {
                        assert_eq!(r.bits(3).unwrap(), delta - 1);
                    }
                    assert_eq!(r.bits(1).unwrap(), 1);
                    assert_eq!(r.bits(16).unwrap() as u16 as i16, expected);
                }
            }
            assert_eq!(r.bits(2).unwrap(), 2);
            assert_eq!(r.bits(11).unwrap(), 2047);
            assert_eq!(r.bits(2).unwrap(), 0);
            assert_eq!(r.u8().unwrap(), 6);
            r.finish().unwrap();
        }
        let bad = Equipment {
            inventory_weapon: None,
            alternate_weapon: None,
            fire_count: 0,
            queued_burst: false,
            spread_scale: 0.0,
            needs_rechamber: false,
            secondary: None,
            events: None,
            weapon: 163,
            aim: None,
            firing: None,
            viewmodel: None,
            ammo: Some(Ammo {
                clip_key: 163,
                reserve_key: 163,
                clip: 512,
                reserve: 90,
            }),
        };
        assert!(equipped_frame(1000, Some([0.0; 3]), true, Some(bad)).is_err());
        assert!(player_frame(1000, 1001, None, true, None, [0; 3]).is_err());
    }
    #[test]
    fn target_full_frame_layout_has_fourteen_arrays_and_entity_sentinel() {
        // Independent bit accounting: header7; pool 0x06,0x3c; time4;
        // pool0 (one remaining stats-mask bit); health2; pools0,0xfe,0x1f.
        let packet = bootstrap(1000).unwrap();
        assert_eq!(
            packet,
            [
                0, 0xe8, 3, 0, 0, 0, 8, 6, 0x3c, 0xe8, 3, 0, 0, 0, 100, 0, 0, 0xfe, 0x1f, 6
            ]
        );
        assert!(bootstrap(0).is_err());
        assert!(bootstrap(u32::MAX).is_err());
    }
    #[test]
    fn spawn_coordinates_reject_nan_and_infinity() {
        assert!(bootstrap_at_origin(1000, Some([f32::NAN, 0.0, 0.0])).is_err());
        assert!(bootstrap_at_origin(1000, Some([0.0, f32::INFINITY, 0.0])).is_err());
        assert!(bootstrap_at_origin(1000, Some([0.0, 0.0, 1_000_001.0])).is_err());
    }
    #[test]
    fn selected_spawn_fields_match_independently_extracted_target_table() {
        let layout: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/player-state-fields-3.3.3.3.json"
        ))
        .unwrap();
        assert_eq!(layout["client_sha256"], crate::CLIENT_SHA256);
        let fields = layout["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 235);
        for (index, offset, codec, hint) in [
            (0, 0, -97, 0),
            (1, 0xf4, -75, 0),
            (8, 0x1c, -88, 3),
            (10, 0x24, -88, 3),
            (11, 0x20, -88, 3),
            (12, 0x18, 8, 3),
            (13, 0x2c, -88, 3),
            (14, 0x28, -88, 3),
            (16, 0x30, -88, 3),
            (20, 0xc, 32, 0),
            (21, 0xf8, -94, 0),
            (22, 0x100, -94, 0),
            (23, 0x108, -94, 0),
            (24, 0x110, -94, 0),
            (34, 0x15c, -88, 0),
            (38, 0x158, -8, 0),
            (47, 0xac, 11, 0),
            (58, 0x258, 0, 0),
            (60, 0x9c, 16, 0),
            (61, 0x1b4, 16, 0),
            (63, 0x90, 16, 0),
            (68, 0x1f0, 11, 0),
            (81, 0x164, -8, 0),
            (82, 0x218, -101, 2),
            (89, 0x168, 1, 2),
            (92, 0x160, 32, 0),
            (93, 0x240, 1, 2),
            (98, 0x210, 1, 2),
            (101, 0x23c, 1, 2),
            (102, 0xbc, 32, 0),
            (104, 0x5b4, 16, 2),
            (106, 0x360, 1, 2),
            (114, 0xc0, 0, 0),
            (122, 0x4bc, -57, 2),
            (217, 0x98, 0, 0),
            (227, 0x200, 11, 0),
        ] {
            assert_eq!(fields[index], serde_json::json!([offset, codec, hint]));
        }
        assert!(fields[..69].iter().all(|f| f[2] != 2));
        let mut baseline = frame(1000, Some([-472.0, -800.0, -113.4]), true).unwrap();
        // Accepted by the real target in client-1791345984530; frozen body
        // after removing native XOR. Includes mixed pool bits/byte fields.
        let hex = "00e8030000000816ede8030000440000ecc3cdcce2c2000048c47f00007042263c7cff9dff07fdff00640000f87f0006";
        let accepted: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(baseline, accepted);
        assert_eq!(baseline[6], 8);
        baseline[6] = 0;
        assert_eq!(
            baseline,
            frame(1000, Some([-472.0, -800.0, -113.4]), false).unwrap()
        );
    }
    #[test]
    fn retired_training_baseline_does_not_corrupt_respawned_multiplayer_gun() {
        let local = crate::game_enemy::EnemySnapshot {
            entity_number: 0,
            client_index: 0,
            team: 2,
            model: 4,
            attachment_models: [0; 10],
            attachment_tags: [0; 10],
            origin: [0.0; 3],
            angles: [0; 3],
            legs_animation: 395,
            torso_animation: 0,
            weapon: 352,
            events: Default::default(),
            alive: true,
        };
        let bot = crate::game_enemy::EnemySnapshot {
            entity_number: 1,
            client_index: 1,
            weapon: 355,
            ..local
        };
        let dead = crate::game_enemy::EnemySnapshot {
            alive: false,
            ..bot
        };
        let training = crate::game_enemy::EnemySnapshot { weapon: 352, ..bot };
        let render = |legacy| {
            player_frame_with_world(
                5000,
                5000,
                Some([0.0; 3]),
                false,
                None,
                [0; 3],
                WorldSnapshot {
                    delta_distance: 1,
                    local_player: Some(local),
                    local_player_baseline: Some(local),
                    players: &[bot],
                    player_baselines: &[dead],
                    enemy_baseline: legacy,
                    held_weapon_tag: Some(1),
                    held_weapons_in_baseline: true,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        // An inactive training fixture previously invented a gun in the dead
        // bot's baseline: new355 XOR old352 decoded as weapon3 and link0.
        assert_eq!(render(None), render(Some(training)));
    }
}
