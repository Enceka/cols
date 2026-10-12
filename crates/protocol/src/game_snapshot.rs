//! Bounded bootstrap snapshot probe for 3.3.3.3, NOT a world simulator.
//! Parser: 0x46d170; playerState: 0x5a4230; table: 0xc55f70 (235 fields).
use crate::{
    game_events, game_shellshock::ShellShockSnapshot, game_state::Writer, game_wire::SERVER_EOF,
    lobby::LobbyError,
};

#[derive(Debug, Clone, Copy)]
pub struct Ammo {
    pub clip_key: u16,
    pub reserve_key: u16,
    pub clip: u16,
    pub reserve: u16,
}
#[derive(Debug, Clone, Copy)]
pub struct Equipment {
    pub offhands: [Option<OffhandWeapon>; 2],
    pub offhand_weapon: u16,
    pub grenade_time: u16,
    /// PS+0x258, multiplied after the weapon/stance movement scales.
    pub move_multiplier: f32,
    /// PS+0x4c0..0x4c8: 96 LSB-first native perk-ID bits, not slot IDs.
    pub perks: [u32; 3],
    pub abilities: crate::game_abilities::Abilities,
    pub weapon: u16,
    /// Owned inventory slot for a held alternate mode; the parent owns it.
    pub inventory_weapon: Option<u16>,
    /// PS+0x344: paired presentation weapon for a models-only alternate.
    pub alternate_weapon: Option<u16>,
    /// PS+0x284: native shots in the current trigger press, clamped at4.
    pub fire_count: u8,
    /// PS+0x280 bit0: a new trigger edge buffered during a burst/recovery.
    pub queued_burst: bool,
    /// Ranger:bit0 last secondary trigger, bit1 simultaneous barrels; PS+0x348 bits8/9.
    pub barrel_flags: u8,
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
        [
            self.weapon,
            self.alternate_weapon.unwrap_or(0),
            self.offhand_weapon,
        ]
    }
}
#[derive(Debug, Clone, Copy)]
pub struct EquippedWeapon {
    pub weapon: u16,
    pub ammo: Option<Ammo>,
    pub needs_rechamber: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct OffhandWeapon {
    pub weapon: u16,
    pub clip_key: u16,
    pub count: u16,
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
    pub pm_time: u16,
    pub sprint: SprintState,
    pub melee_charge: Option<MeleeCharge>,
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
/// Native PS+0x23c..0x24c; the two bools are hint-2 fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SprintState {
    pub button_up_required: bool,
    pub delay: bool,
    pub last_start: u32,
    pub last_end: u32,
    pub start_max_length: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeleeCharge {
    /// Native angle bits;0x5a17d0 sign-extends before converting to degrees.
    pub yaw: u16,
    pub distance: u8,
    pub remaining_msec: u16,
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
    /// Native HUD operator154 reads PS+0x10 bit10 (field54).
    pub emp_jammed: bool,
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
            emp_jammed: false,
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
            perk_display: false,
            local_abilities: None,
            destroyed_glass: &[],
            bags: &[],
            bag_baselines: &[],
            smokes: &[],
            smoke_baselines: &[],
            missiles: &[],
            missile_baselines: &[],
            local_abilities_baseline: None,
            delta_distance: 0,
            corpse_baselines: &[],
            impact_baselines: &[],
            obituary_baselines: &[],
            motion,
            shellshock: None,
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
            characters: &[],
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
    pub destroyed_glass: &'a [u16],
    /// Native perk_sidebar_hud operator239 reads PS+0x4bc (hint2 field122).
    pub perk_display: bool,
    pub bags: &'a [crate::game_scavenger::BagSnapshot],
    pub bag_baselines: &'a [crate::game_scavenger::BagSnapshot],
    pub smokes: &'a [crate::game_smoke::SmokeSnapshot],
    pub smoke_baselines: &'a [crate::game_smoke::SmokeSnapshot],
    pub missiles: &'a [crate::game_missile::MissileSnapshot],
    pub missile_baselines: &'a [crate::game_missile::MissileSnapshot],
    pub local_abilities: Option<crate::game_abilities::Abilities>,
    pub local_abilities_baseline: Option<crate::game_abilities::Abilities>,
    pub delta_distance: u8,
    pub motion: Option<MotionSnapshot>,
    /// Current shellshock state; requires authoritative motion. Every frame
    /// selects the zero PS base. None encodes index/begin/duration zero, which
    /// may select the native CG override. Reset a named effect with its last
    /// index and begin/duration zero. Replay callers add delay to nonzero
    /// begin_time in the receiving client's CG time domain.
    pub shellshock: Option<ShellShockSnapshot>,
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
    /// Match-frozen native compositions, indexed by client identity. Podium
    /// copies without an entry retain their explicit third-person models.
    pub characters: &'a [Option<crate::game_character::Character>],
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
    let shellshock = world.shellshock;
    let vitals = world.vitals;
    if shellshock.is_some_and(|s| s.index > 15 || motion.is_none()) {
        return Err(LobbyError("invalid authoritative shellshock state"));
    }
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
    if let Some(equipment) = equipment {
        equipment.abilities.validate()?;
        if equipment
            .offhands
            .iter()
            .flatten()
            .any(|g| !(1..900).contains(&g.weapon) || g.clip_key != g.weapon || g.count > 511)
            || (equipment.offhand_weapon != 0
                && !equipment
                    .offhands
                    .iter()
                    .flatten()
                    .any(|g| g.weapon == equipment.offhand_weapon))
            || equipment.grenade_time > 15000
            || (motion.is_none()
                && (equipment.offhands.iter().any(Option::is_some) || equipment.perks != [0; 3]))
        {
            return Err(LobbyError(
                "offhand and passive skill equipment requires authoritative movement",
            ));
        }
        if !equipment.move_multiplier.is_finite()
            || !(0.0..=10.0).contains(&equipment.move_multiplier)
        {
            return Err(LobbyError("invalid passive skill state"));
        }
    }
    let inventory_weapon = equipment.and_then(|e| e.inventory_weapon).or(weapon);
    let alternate_weapon = equipment.and_then(|e| e.alternate_weapon);
    if alternate_weapon.is_some_and(|w| w == 0 || w >= 900)
        || equipment.is_some_and(|e| {
            e.fire_count > 4
                || e.barrel_flags > 3
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
            || ((m.flags & 0x10000 != 0) != m.melee_charge.is_some())
            || m.melee_charge
                .is_some_and(|c| c.distance == 0 || c.remaining_msec > 15000)
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
            || m.sprint.last_start > command_time
            || m.sprint.last_end > command_time
            || m.pm_time > i16::MAX as u16
    }) {
        return Err(LobbyError("invalid authoritative movement"));
    }
    let ammunition: Vec<Ammo> = ammo
        .into_iter()
        .chain(secondary.and_then(|other| other.ammo))
        .chain(
            equipment
                .into_iter()
                .flat_map(|e| e.offhands.into_iter().flatten())
                .map(|g| Ammo {
                    clip_key: g.clip_key,
                    reserve_key: g.weapon,
                    clip: g.count,
                    reserve: 0,
                }),
        )
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
    let mut fields = Vec::new();
    if let Some(origin) = origin {
        // Target field table 0xc55f70. Hint-2 fields use the later tail pass.
        // Equipment and angles must also be reflected in the input delta base.
        fields.extend([
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
        ]);
        if let Some(shellshock) = shellshock {
            // Duration104 belongs exclusively to the later hint-2 pass.
            fields.extend([
                (103, shellshock.begin_time, 97),
                (105, u32::from(shellshock.index), 4),
            ]);
        }
        if let Some(motion) = motion {
            if let Some(charge) = motion.melee_charge {
                fields.extend([
                    (148, u32::from(charge.remaining_msec), 32),
                    (149, u32::from(charge.yaw), 101),
                    (150, u32::from(charge.distance), 8),
                ]);
            }
            fields.extend([
                (12, u32::from(motion.bob_cycle), 8),
                (52, u32::from(motion.pm_time), 16),
                (13, motion.velocity[1].to_bits(), 0),
                (14, motion.velocity[0].to_bits(), 0),
                (16, motion.velocity[2].to_bits(), 0),
                (35, motion.sprint.last_start, 97),
                (36, motion.sprint.last_end, 97),
                (43, u32::from(motion.sprint.start_max_length), 16),
                (
                    58,
                    equipment.map_or(1.0, |e| e.move_multiplier).to_bits(),
                    100,
                ),
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
            let e = equipment.unwrap();
            if e.offhand_weapon != 0 {
                fields.push((53, u32::from(e.offhand_weapon), 10));
            }
            if e.grenade_time != 0 {
                fields.push((79, u32::from(e.grenade_time), 16));
            }
            for (slot, grenade) in e.offhands.into_iter().enumerate() {
                if let Some(g) = grenade {
                    // The saved tactical item belongs to the default Q button's
                    // selected slot (+0x338 / field111 / button0x10000).
                    // +0x334 is the separate middle offhand button0x8000.
                    fields.push((if slot == 0 { 109 } else { 111 }, u32::from(g.weapon), 32));
                }
            }
            for (slot, id) in equipment.unwrap().perks.into_iter().enumerate() {
                if id != 0 {
                    fields.push((123 + slot as u32, id, 32));
                }
            }
            let flags = u32::from(e.barrel_flags) << 8
                | if firing.is_some_and(|f| (17..=20).contains(&f.state)) {
                    2
                } else {
                    0
                };
            if flags != 0 {
                fields.push((77, flags, 22));
            }
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
                    } | if vitals.is_some_and(|v| v.emp_jammed && v.health > 0 && !v.intermission) {
                        0x400
                    } else {
                        0
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
    }
    fields.sort_by_key(|f| f.0);
    let last_changed = fields.last().map_or(0, |f| f.0);
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
    out.bits(0, 1); // no prior playerState, including full/delta/replay frames
    out.bits(1, 1); // transmit origin/velocity rather than local prediction
    out.bits(last_changed + 1, 8);
    out.bits(1, 1); // field gap +1 from -1
    out.bits(1, 1); // -97 codec, absolute LE32 (0x5a20c0)
    out.bytes.extend_from_slice(&command_time.to_le_bytes());
    let mut previous = 0;
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
        } else if width == 97 {
            // -97 reads an absolute LE32; it is not an XOR32 scalar.
            out.bits(1, 1);
            out.bytes.extend_from_slice(&value.to_le_bytes());
        } else if width == 100 {
            // Generic float codec 0 differs from raw-float codec -88:
            // nonzero + full-precision gates, then the raw XOR word.
            out.bits(1, 1);
            out.bits(1, 1);
            out.bytes.extend_from_slice(&value.to_le_bytes());
        } else if width == 101 {
            // -100 at0x5a25c0: skip both short-delta branches and read
            // an absolute network angle via0x5a17d0.
            out.bits(0, 2);
            out.bytes.extend_from_slice(&(value as u16).to_le_bytes());
        } else if matches!(index, 43 | 52 | 105) && value == 0 {
            out.bits(0, 1); // scalar zero gate, including an explicit clear
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
    if let Some(motion) = motion {
        write_hint2_tail(
            &mut out,
            last_changed,
            motion,
            shellshock,
            world.perk_display,
        );
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
        for slot in 1..15 {
            let other = if slot == 1 {
                secondary
            } else if slot <= 3 {
                equipment.unwrap().offhands[slot - 2].map(|g| EquippedWeapon {
                    weapon: g.weapon,
                    ammo: None,
                    needs_rechamber: false,
                })
            } else {
                None
            };
            out.bits(u32::from(other.is_some()), 1);
            if let Some(other) = other {
                out.bits(1, 1);
                out.bits(u32::from(other.weapon), 10);
                out.bits(u32::from(other.needs_rechamber) << 3, 5);
            }
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
    {
        for corpse in world
            .corpses
            .iter()
            .filter(|c| !establish_baseline || c.phase == crate::game_corpse::CorpsePhase::Posed)
        {
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
                (!establish_baseline)
                    .then(|| {
                        world
                            .corpse_baselines
                            .iter()
                            .find(|b| b.present && b.entity_number == corpse.entity_number)
                            .copied()
                    })
                    .flatten(),
                establish_baseline,
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
        crate::game_missile::write(
            &mut out,
            world.missiles,
            if establish_baseline {
                &[]
            } else {
                world.missile_baselines
            },
        )?;
    } else {
        crate::game_impact::write_impacts_with_baselines(
            &mut out,
            world.impacts,
            world.impact_baselines,
            establish_baseline,
        )?;
        crate::game_missile::write(&mut out, world.missiles, world.missile_baselines)?;
    }
    crate::game_smoke::write(
        &mut out,
        world.smokes,
        if establish_baseline {
            &[]
        } else {
            world.smoke_baselines
        },
    )?;
    crate::game_scavenger::write(
        &mut out,
        world.bags,
        if establish_baseline {
            &[]
        } else {
            world.bag_baselines
        },
    )?;
    if world.delta_distance == 0 {
        crate::game_obituary::write_obituaries(&mut out, world.obituaries, establish_baseline)?;
    } else {
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
            world.characters,
            crate::game_abilities::ClientAbilities {
                current: world.local_abilities,
                baseline: world.local_abilities_baseline,
            },
        )?;
    } else if let Some(enemy) = world.enemy {
        if world.held_weapon_tag.is_some() {
            crate::game_enemy::write_player_clients_with_card(
                &mut out,
                &[enemy],
                establish_baseline,
                None,
                true,
                world.characters,
                Default::default(),
            )?;
        } else {
            crate::game_enemy::write_enemy_client(&mut out, enemy, establish_baseline)?;
        }
    } else {
        out.bits(0, 1); // clients (0x46c840)
    }
    crate::game_glass::write(&mut out, world.destroyed_glass)?;
    out.bytes.push(SERVER_EOF);
    Ok(out.bytes)
}

fn write_hint2_tail(
    out: &mut Writer,
    last_changed: u32,
    motion: MotionSnapshot,
    shellshock: Option<ShellShockSnapshot>,
    perk_display: bool,
) {
    // 0x5a4541..0x5a458d: hint-2 fields strictly below the last ordinary
    // field, in table order. No gaps and no enclosing gate. Width0 denotes
    // the single preserve gate at82 (-101) or the direct bool at122 (-57).
    for (index, value, width) in [
        (82, 0, 0),
        (89, u32::from(motion.view_height_lerp_down), 1),
        (93, u32::from(motion.sprint.delay), 1),
        (98, 0, 1),
        (101, u32::from(motion.sprint.button_up_required), 1),
        (
            104,
            shellshock.map_or(0, |s| u32::from(s.duration_msec)),
            16,
        ),
        (106, 0, 1),
        (122, u32::from(perk_display), 0),
    ] {
        if index >= last_changed {
            break;
        }
        out.bits(u32::from(value != 0), 1);
        if width != 0 && value != 0 {
            let remainder = width % 8;
            out.bits(value, remainder);
            for shift in (remainder..width).step_by(8) {
                out.bytes.push((value >> shift) as u8);
            }
        }
    }
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
                -100 => {
                    assert_eq!(r.bits(2).unwrap(), 0);
                    (f32::from(r.u16().unwrap() as i16) * (360.0 / 65536.0)).to_bits()
                }
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

    fn shellshock_from_fields(fields: &BTreeMap<usize, u32>) -> ShellShockSnapshot {
        ShellShockSnapshot {
            index: fields.get(&105).copied().unwrap_or(0).try_into().unwrap(),
            begin_time: fields.get(&103).copied().unwrap_or(0),
            duration_msec: fields.get(&104).copied().unwrap_or(0).try_into().unwrap(),
        }
    }

    fn authoritative_motion() -> MotionSnapshot {
        MotionSnapshot {
            ground_entity: 2046,
            view_height: 60.0,
            view_height_target: 60,
            view_height_lerp_target: 40,
            view_height_lerp_time: 800,
            speed: 190,
            gravity: 800,
            ..Default::default()
        }
    }
    #[test]
    fn landing_timer_and_packed_pain_event_survive_full_delta_and_clear() {
        for baseline in [true, false] {
            for timer in [2000, 15, 0] {
                let mut motion = authoritative_motion();
                motion.pm_time = timer;
                motion.flags = if timer != 0 { 0x80 } else { 0 };
                let body = player_frame_with_world(
                    3000,
                    2950,
                    Some([0.0; 3]),
                    baseline,
                    None,
                    [0; 3],
                    WorldSnapshot {
                        motion: Some(motion),
                        delta_distance: 1,
                        ..Default::default()
                    },
                )
                .unwrap();
                let mut r = Reader::new(&body);
                r.u8().unwrap();
                r.u32().unwrap();
                r.u8().unwrap();
                r.u8().unwrap();
                r.bits(2).unwrap();
                let (fields, _) = read_fields(&mut r);
                assert_eq!(fields[&52], u32::from(timer));
                assert_eq!(
                    fields.get(&20).copied().unwrap_or(0) & 0x80,
                    if timer != 0 { 0x80 } else { 0 }
                );
            }
        }
    }

    #[test]
    fn sprint_meter_decodes_all_five_fields_in_full_delta_replay_and_reset_frames() {
        let active = SprintState {
            button_up_required: true,
            delay: true,
            last_start: 1200,
            last_end: 5200,
            start_max_length: 4000,
        };
        for baseline in [false, true] {
            for replay in [
                None,
                Some(ReplayView {
                    client: 7,
                    delay_msec: 4000,
                    end_time: 11000,
                    skip_text: None,
                }),
            ] {
                for sprint in [active, SprintState::default()] {
                    let mut motion = authoritative_motion();
                    motion.sprint = sprint;
                    let body = player_frame_with_world(
                        10000,
                        9950,
                        Some([0.0; 3]),
                        baseline,
                        Some(Equipment {
                            weapon: 352,
                            inventory_weapon: None,
                            alternate_weapon: None,
                            fire_count: 0,
                            queued_burst: false,
                            barrel_flags: 0,
                            spread_scale: 0.0,
                            needs_rechamber: false,
                            ammo: None,
                            viewmodel: None,
                            firing: None,
                            aim: None,
                            secondary: None,
                            events: None,
                            offhands: [None, None],
                            offhand_weapon: 0,
                            grenade_time: 0,
                            move_multiplier: 1.0,
                            perks: [0; 3],
                            abilities: Default::default(),
                        }),
                        [0; 3],
                        WorldSnapshot {
                            motion: Some(motion),
                            replay,
                            vitals: Some(PlayerVitals::default()),
                            delta_distance: 1,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let mut r = Reader::new(&body);
                    assert_eq!(r.u8().unwrap(), 0);
                    assert_eq!(r.u32().unwrap(), 10000);
                    r.u8().unwrap();
                    r.u8().unwrap();
                    assert_eq!(r.bits(1).unwrap(), 0);
                    assert_eq!(r.bits(1).unwrap(), 1);
                    let (fields, _) = read_fields(&mut r);
                    assert_eq!(fields[&35], sprint.last_start);
                    assert_eq!(fields[&36], sprint.last_end);
                    assert_eq!(fields[&43], u32::from(sprint.start_max_length));
                    assert_eq!(fields[&93], u32::from(sprint.delay));
                    assert_eq!(fields[&101], u32::from(sprint.button_up_required));
                    // Stats must still start at the correct pool/byte boundary.
                    assert_eq!(r.bits(1).unwrap(), 1);
                    assert_eq!(r.bits(4).unwrap(), 13);
                    assert_eq!(r.u16().unwrap(), 100);
                }
            }
        }
        let mut motion = authoritative_motion();
        motion.sprint.last_start = 9951;
        assert!(
            player_frame_with_world(
                10000,
                9950,
                Some([0.0; 3]),
                true,
                None,
                [0; 3],
                WorldSnapshot {
                    motion: Some(motion),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn shellshock_duration_and_existing_hint2_fields_decode_in_full_and_delta_frames() {
        for establish_baseline in [true, false] {
            for delta_distance in [0, 1] {
                for lerp_down in [false, true] {
                    for duration_msec in [0, 1, 255, 256, 0x1234, u16::MAX] {
                        let shellshock = ShellShockSnapshot {
                            index: 5,
                            begin_time: 70_000,
                            duration_msec,
                        };
                        let body = player_frame_with_world(
                            100_000,
                            99_950,
                            Some([1.0, 2.0, 3.0]),
                            establish_baseline,
                            None,
                            [0; 3],
                            WorldSnapshot {
                                motion: Some(MotionSnapshot {
                                    view_height_lerp_down: lerp_down,
                                    ..authoritative_motion()
                                }),
                                shellshock: Some(shellshock),
                                delta_distance,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                        let mut r = Reader::new(&body);
                        assert_eq!(r.u8().unwrap(), 0);
                        assert_eq!(r.u32().unwrap(), 100_000);
                        assert_eq!(
                            r.u8().unwrap(),
                            if establish_baseline {
                                0
                            } else {
                                delta_distance
                            }
                        );
                        assert_eq!(r.u8().unwrap(), if establish_baseline { 8 } else { 0 });
                        assert_eq!(r.bits(1).unwrap(), 0); // zero PS base, even on ACK deltas
                        assert_eq!(r.bits(1).unwrap(), 1);
                        // The final ordinary camera field is227, so count228
                        // includes both new ordinary fields and duration104.
                        assert_eq!((body[7] >> 2) | ((body[8] & 3) << 6), 228);
                        let (fields, parameters) = read_fields(&mut r);
                        assert_eq!(shellshock_from_fields(&fields), shellshock);
                        assert!(parameters.is_empty());
                        assert_eq!(fields[&0], 99_950);
                        assert_eq!(fields[&89], u32::from(lerp_down));
                        assert_eq!(fields[&227], 2047);
                        for index in [82, 93, 98, 101, 106, 122] {
                            assert_eq!(fields[&index], 0, "hint-2 field {index}");
                        }
                        // The stats and section terminators must remain aligned
                        // after duration's gate and separate LE16 byte payload.
                        assert_eq!(r.bits(1).unwrap(), 1);
                        assert_eq!(r.bits(4).unwrap(), 1);
                        assert_eq!(r.u16().unwrap(), 100);
                        assert_eq!(r.bits(15).unwrap(), 0);
                        assert_eq!(r.bits(2).unwrap(), 2);
                        assert_eq!(r.bits(11).unwrap(), 2047);
                        assert_eq!(r.bits(2).unwrap(), 0);
                        assert_eq!(r.u8().unwrap(), SERVER_EOF);
                        r.finish().unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn shellshock_named_resets_and_zero_ps_states_decode_in_every_frame() {
        let mut previous = ShellShockSnapshot::default();
        for (frame, shellshock) in [
            Some(ShellShockSnapshot {
                index: 1,
                begin_time: 70_000,
                duration_msec: u16::MAX,
            }),
            Some(ShellShockSnapshot {
                index: 1,
                begin_time: 0,
                duration_msec: 0,
            }),
            Some(ShellShockSnapshot {
                index: 15,
                begin_time: 90_000,
                duration_msec: 256,
            }),
            Some(ShellShockSnapshot {
                index: 15,
                begin_time: 0,
                duration_msec: 0,
            }),
            None,
            // Index0 selects the native local override, not a named preset.
            Some(ShellShockSnapshot {
                index: 0,
                begin_time: 99_000,
                duration_msec: 511,
            }),
            Some(ShellShockSnapshot::default()),
        ]
        .into_iter()
        .enumerate()
        {
            for establish_baseline in [true, false] {
                let body = player_frame_with_world(
                    100_000 + frame as u32 * 50,
                    99_950 + frame as u32 * 50,
                    Some([0.0; 3]),
                    establish_baseline,
                    None,
                    [0; 3],
                    WorldSnapshot {
                        motion: Some(authoritative_motion()),
                        shellshock,
                        delta_distance: 1,
                        ..Default::default()
                    },
                )
                .unwrap();
                let mut r = Reader::new(&body);
                r.u8().unwrap();
                r.u32().unwrap();
                r.u8().unwrap();
                r.u8().unwrap();
                let use_previous = r.bits(1).unwrap() != 0;
                assert!(!use_previous);
                assert_eq!(r.bits(1).unwrap(), 1);
                let (fields, _) = read_fields(&mut r);
                let mut decoded = if use_previous {
                    previous
                } else {
                    ShellShockSnapshot::default()
                };
                if let Some(&index) = fields.get(&105) {
                    decoded.index = index.try_into().unwrap();
                }
                if let Some(&begin_time) = fields.get(&103) {
                    decoded.begin_time = begin_time;
                }
                decoded.duration_msec = fields[&104].try_into().unwrap();
                assert_eq!(decoded, shellshock.unwrap_or_default());
                assert_eq!(fields.contains_key(&103), shellshock.is_some());
                assert_eq!(fields.contains_key(&105), shellshock.is_some());
                previous = decoded;
            }
        }
    }

    #[test]
    fn shellshock_hint2_tail_stops_strictly_before_the_last_ordinary_field() {
        let layout: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/protocol/player-state-fields-3.3.3.3.json"
        ))
        .unwrap();
        for last_changed in [81, 83, 88, 92, 99, 102, 103, 105, 107, 121, 123, 227] {
            for duration_msec in [0, 256, u16::MAX] {
                let mut out = Writer::default();
                out.bits(last_changed + 1, 8);
                gap(&mut out, last_changed + 1);
                if last_changed == 103 {
                    out.bits(1, 1); // ordinary -97 absolute time
                    out.bytes.extend_from_slice(&0u32.to_le_bytes());
                } else {
                    out.bits(0, 1); // ordinary scalar zero gate
                }
                write_hint2_tail(
                    &mut out,
                    last_changed,
                    MotionSnapshot {
                        view_height_lerp_down: true,
                        ..authoritative_motion()
                    },
                    Some(ShellShockSnapshot {
                        index: 5,
                        begin_time: 70_000,
                        duration_msec,
                    }),
                    false,
                );
                out.bits(1, 1); // stats gate immediately follows the hint-2 tail
                out.bits(1, 4);
                out.bytes.extend_from_slice(&100u16.to_le_bytes());
                let mut r = Reader::new(&out.bytes);
                let (fields, _) = read_fields(&mut r);
                for (index, entry) in layout["fields"].as_array().unwrap().iter().enumerate() {
                    if entry[2] == 2 {
                        assert_eq!(
                            fields.contains_key(&index),
                            index < last_changed as usize,
                            "hint-2 field {index}, last ordinary field {last_changed}"
                        );
                    }
                }
                if last_changed > 89 {
                    assert_eq!(fields[&89], 1);
                }
                if last_changed > 104 {
                    assert_eq!(fields[&104], u32::from(duration_msec));
                }
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.bits(4).unwrap(), 1);
                assert_eq!(r.u16().unwrap(), 100);
                r.finish().unwrap();
            }
        }
    }

    #[test]
    fn shellshock_requires_authoritative_motion_and_a_four_bit_index() {
        let encode = |origin, motion, shellshock| {
            player_frame_with_world(
                1000,
                950,
                origin,
                true,
                None,
                [0; 3],
                WorldSnapshot {
                    motion,
                    shellshock: Some(shellshock),
                    ..Default::default()
                },
            )
        };
        for index in [0, 1, 5, 15, 16, u8::MAX] {
            let shellshock = ShellShockSnapshot {
                index,
                begin_time: 900,
                duration_msec: 1000,
            };
            assert!(encode(None, None, shellshock).is_err());
            assert!(encode(Some([0.0; 3]), None, shellshock).is_err());
            assert!(encode(None, Some(authoritative_motion()), shellshock).is_err());
            assert_eq!(
                encode(Some([0.0; 3]), Some(authoritative_motion()), shellshock).is_ok(),
                index <= 15
            );
        }
        assert!(encode(Some([0.0; 3]), None, ShellShockSnapshot::default()).is_err());
    }

    #[test]
    fn perk_sidebar_gate_is_a_direct_bool_in_the_native_hint2_tail() {
        for alignment in 0..8 {
            for display in [false, true] {
                let mut out = Writer::default();
                out.bits(0x55, alignment);
                write_hint2_tail(&mut out, 217, authoritative_motion(), None, display);
                out.bytes.push(0xb6);
                let mut r = Reader::new(&out.bytes);
                r.bits(alignment).unwrap();
                let table: serde_json::Value = serde_json::from_str(include_str!(
                    "../../../fixtures/protocol/player-state-fields-3.3.3.3.json"
                ))
                .unwrap();
                for (index, field) in table["fields"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .take(217)
                {
                    if field[2] != 2 {
                        continue;
                    }
                    if index == 122 {
                        assert_eq!(field, &serde_json::json!([0x4bc, -57, 2]));
                        assert_eq!(r.bits(1).unwrap(), u32::from(display));
                    } else {
                        assert_eq!(r.bits(1).unwrap(), 0);
                    }
                }
                assert_eq!(r.u8().unwrap(), 0xb6);
            }
        }
    }
    #[test]
    fn shellshock_replay_uses_caller_delay_and_preserves_named_reset() {
        let equipment = Equipment {
            perks: [0x80000204, 0x04000000, 0x80000000],
            abilities: Default::default(),
            offhands: [None, None],
            offhand_weapon: 0,
            grenade_time: 0,
            move_multiplier: 1.0,
            weapon: 352,
            inventory_weapon: None,
            alternate_weapon: None,
            fire_count: 0,
            queued_burst: false,
            barrel_flags: 0,
            spread_scale: 0.0,
            needs_rechamber: false,
            ammo: None,
            viewmodel: Some(54),
            firing: None,
            aim: None,
            secondary: None,
            events: None,
        };
        let archived = ShellShockSnapshot {
            index: 5,
            begin_time: 45_250,
            duration_msec: 4321,
        };
        let replay_delay_msec = 4000;
        let rebased = ShellShockSnapshot {
            begin_time: archived.begin_time + replay_delay_msec,
            ..archived
        };
        let named_reset = ShellShockSnapshot {
            begin_time: 0,
            duration_msec: 0,
            ..archived
        };
        for establish_baseline in [true, false] {
            for shellshock in [
                Some(rebased),
                Some(named_reset),
                Some(ShellShockSnapshot::default()),
                None,
            ] {
                let body = player_frame_with_world(
                    50_000,
                    49_950,
                    Some([1.0, 2.0, 3.0]),
                    establish_baseline,
                    Some(equipment),
                    [0; 3],
                    WorldSnapshot {
                        motion: Some(authoritative_motion()),
                        vitals: Some(PlayerVitals::default()),
                        replay: Some(ReplayView {
                            client: 7,
                            delay_msec: replay_delay_msec,
                            end_time: 55_000,
                            skip_text: None,
                        }),
                        shellshock,
                        delta_distance: 1,
                        ..Default::default()
                    },
                )
                .unwrap();
                let mut r = Reader::new(&body);
                r.u8().unwrap();
                assert_eq!(r.u32().unwrap(), 50_000);
                r.u8().unwrap();
                r.u8().unwrap();
                assert_eq!(r.bits(2).unwrap(), 2);
                let (fields, _) = read_fields(&mut r);
                assert_eq!([fields[&123], fields[&124], fields[&125]], equipment.perks);
                assert_eq!(
                    shellshock_from_fields(&fields),
                    shellshock.unwrap_or_default()
                );
                assert_eq!(fields[&59], 7);
                assert_eq!(fields[&142], replay_delay_msec);
                assert_eq!(fields[&54], 0x800);
                assert_eq!(r.bits(1).unwrap(), 1);
                assert_eq!(r.bits(4).unwrap(), 13);
                assert_eq!(r.u16().unwrap(), 100);
                assert_eq!(r.u16().unwrap(), 100);
                assert_eq!(r.u8().unwrap(), 1);
            }
        }
    }

    #[test]
    fn saved_tactical_smoke_is_published_in_the_default_q_selected_slot() {
        let equipment = Equipment {
            weapon: 352,
            inventory_weapon: None,
            alternate_weapon: None,
            fire_count: 0,
            queued_burst: false,
            barrel_flags: 0,
            spread_scale: 0.0,
            needs_rechamber: false,
            ammo: None,
            viewmodel: None,
            firing: None,
            aim: None,
            secondary: None,
            events: None,
            perks: [0; 3],
            abilities: Default::default(),
            move_multiplier: 1.0,
            offhands: [
                Some(OffhandWeapon {
                    weapon: 95,
                    clip_key: 95,
                    count: 1,
                }),
                Some(OffhandWeapon {
                    weapon: 236,
                    clip_key: 236,
                    count: 1,
                }),
            ],
            offhand_weapon: 236,
            grenade_time: 1000,
        };
        let body = player_frame_with_motion(
            1000,
            925,
            Some([0.0; 3]),
            true,
            Some(equipment),
            [0; 3],
            Some(MotionSnapshot {
                ground_entity: 2046,
                speed: 190,
                gravity: 800,
                view_height: 60.0,
                view_height_target: 60,
                ..Default::default()
            }),
        )
        .unwrap();
        let mut r = Reader::new(&body);
        assert_eq!(r.u8().unwrap(), 0);
        assert_eq!(r.u32().unwrap(), 1000);
        r.u8().unwrap();
        r.u8().unwrap();
        assert_eq!(r.bits(1).unwrap(), 0);
        assert_eq!(r.bits(1).unwrap(), 1);
        let (fields, _) = read_fields(&mut r);
        assert_eq!(fields[&109], 95);
        assert_eq!(fields[&111], 236);
        assert_eq!(fields.get(&110).copied().unwrap_or(0), 0);
        assert_eq!(fields[&53], 236);
    }
    #[test]
    fn smoke_removal_precedes_new_obituary_for_pinned_and_acknowledged_frames() {
        let old = crate::game_smoke::SmokeSnapshot {
            entity_number: 1708,
            origin: [0.0; 3],
            weapon: 236,
            start_time: 1000,
            end_time: 61001,
            present: true,
            generation: 1,
        };
        let kill = crate::game_obituary::ObituarySnapshot {
            falling: false,
            entity_number: 1800,
            weapon: 352,
            headshot: false,
            melee: false,
            victim: 1,
            attacker: 2,
        };
        for delta_distance in [0, 1] {
            let body = player_frame_with_world(
                62000,
                61900,
                None,
                false,
                None,
                [0; 3],
                WorldSnapshot {
                    delta_distance,
                    smoke_baselines: &[old],
                    obituaries: &[kill],
                    ..Default::default()
                },
            )
            .unwrap();
            let mut r = Reader::new(&body);
            assert_eq!(r.u8().unwrap(), 0);
            assert_eq!(r.u32().unwrap(), 62000);
            assert_eq!(r.u8().unwrap(), delta_distance);
            r.u8().unwrap();
            assert_eq!(r.bits(2).unwrap(), 2);
            read_fields(&mut r);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(4).unwrap(), 1);
            assert_eq!(r.u16().unwrap(), 100);
            assert_eq!(r.bits(15).unwrap(), 0); // empty arrays, HUD, animation and actors
            // The native sorted merge must see this deletion BEFORE it can
            // inherit lower baseline IDs while consuming the new kill event.
            assert_eq!(r.bits(2).unwrap(), 2);
            assert_eq!(r.bits(11).unwrap(), 1708);
            assert_eq!(r.bits(1).unwrap(), 1);
            assert_eq!(r.bits(2).unwrap(), 2);
            assert_eq!(r.bits(11).unwrap(), 1800);
            assert_eq!(r.bits(3).unwrap(), 6);
        }
    }

    #[test]
    fn killcam_fields_select_the_archived_player_and_clear_on_return() {
        let equipment = Equipment {
            perks: Default::default(),
            abilities: Default::default(),
            offhands: [None, None],
            offhand_weapon: 0,
            grenade_time: 0,
            move_multiplier: 1.0,
            weapon: 352,
            inventory_weapon: None,
            alternate_weapon: None,
            fire_count: 0,
            queued_burst: false,
            barrel_flags: 0,
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
            melee_charge: None,
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
            for jammed in [false, true] {
                let body = player_frame_with_world(
                    9000,
                    8950,
                    Some([1.0, 2.0, 3.0]),
                    true,
                    Some(equipment),
                    [0; 3],
                    WorldSnapshot {
                        bags: &[],
                        bag_baselines: &[],
                        smokes: &[],
                        smoke_baselines: &[],
                        missiles: &[],
                        missile_baselines: &[],
                        motion: Some(motion),
                        vitals: Some(PlayerVitals {
                            emp_jammed: jammed,
                            ..Default::default()
                        }),
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
                assert_eq!(
                    fields[&54],
                    (if replay.is_some() { 0x800 } else { 0x1000 })
                        | if jammed { 0x400 } else { 0 }
                );
                let table: serde_json::Value = serde_json::from_str(include_str!(
                    "../../../fixtures/protocol/player-state-fields-3.3.3.3.json"
                ))
                .unwrap();
                assert_eq!(table["fields"][54], serde_json::json!([16, 28, 0]));
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
            perks: Default::default(),
            abilities: Default::default(),
            offhands: [None, None],
            offhand_weapon: 0,
            grenade_time: 0,
            move_multiplier: 1.0,
            inventory_weapon: None,
            alternate_weapon: Some(180),
            fire_count: 1,
            queued_burst: false,
            barrel_flags: 0,
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
                pm_time: 0,
                sprint: SprintState::default(),
                melee_charge: None,
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
                    barrel_flags: if lerp_down { 3 } else { 0 },
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
            assert_eq!(
                fields.get(&77).copied().unwrap_or(0),
                if lerp_down { 0x300 } else { 0 }
            );
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
    fn melee_charge_fields_match_native_angle_time_and_distance_codecs() {
        for charge in [
            Some(MeleeCharge {
                yaw: 0,
                distance: 50,
                remaining_msec: 0,
            }),
            Some(MeleeCharge {
                yaw: 49152,
                distance: 180,
                remaining_msec: 80,
            }),
            None,
        ] {
            let motion = MotionSnapshot {
                pm_time: 0,
                sprint: SprintState::default(),
                melee_charge: charge,
                velocity: [0.0; 3],
                flags: if charge.is_some() { 0x10000 } else { 0 },
                ground_entity: 2046,
                view_height: 60.0,
                view_height_target: 60,
                speed: 190,
                gravity: 800,
                bob_cycle: 0,
                view_height_lerp_time: 0,
                view_height_lerp_target: 60,
                view_height_lerp_down: false,
                jump_time: 0,
                jump_origin_z: 0.0,
            };
            let body = player_frame_with_motion(
                1000,
                1000,
                Some([0.0; 3]),
                true,
                None,
                [0; 3],
                Some(motion),
            )
            .unwrap();
            let mut r = Reader::new(&body);
            r.u8().unwrap();
            r.u32().unwrap();
            r.u8().unwrap();
            r.u8().unwrap();
            r.bits(2).unwrap();
            let (fields, _) = read_fields(&mut r);
            assert_eq!(
                fields.get(&20).copied().unwrap_or(0) & 0x10000,
                motion.flags
            );
            for index in [148, 149, 150] {
                assert_eq!(fields.contains_key(&index), charge.is_some());
            }
            if let Some(c) = charge {
                assert_eq!(fields[&148], u32::from(c.remaining_msec));
                assert_eq!(
                    fields[&149],
                    (f32::from(c.yaw as i16) * (360.0 / 65536.0)).to_bits()
                );
                assert_eq!(fields[&150], u32::from(c.distance));
            }
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
            perks: Default::default(),
            abilities: Default::default(),
            offhands: [None, None],
            offhand_weapon: 0,
            grenade_time: 0,
            move_multiplier: 1.0,
            inventory_weapon: None,
            alternate_weapon: Some(180),
            fire_count: 1,
            queued_burst: false,
            barrel_flags: 0,
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
            perks: Default::default(),
            abilities: Default::default(),
            offhands: [None, None],
            offhand_weapon: 0,
            grenade_time: 0,
            move_multiplier: 1.0,
            inventory_weapon: None,
            alternate_weapon: None,
            fire_count: 0,
            queued_burst: false,
            barrel_flags: 0,
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
        assert!(
            equipped_frame(
                1000,
                Some([0.0; 3]),
                true,
                Some(Equipment {
                    ammo: None,
                    barrel_flags: 4,
                    ..bad
                })
            )
            .is_err()
        );
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
            (103, 0x5b0, -97, 0),
            (104, 0x5b4, 16, 2),
            (105, 0x5ac, 4, 0),
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
