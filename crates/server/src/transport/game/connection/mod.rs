//! Application connection bootstrap after authenticated game DATA.
use anyhow::{Result, ensure};
use codol_protocol::game::channel::wire as game_wire;
use codol_protocol::game::channel::wire::ClientHeader;
use codol_protocol::game::channel::wire::ReliableCommands;
use codol_protocol::game::channel::wire::SERVER_EOF;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
struct KillCause {
    headshot: bool,
    melee: bool,
    falling: bool,
}

#[derive(Default)]
pub(crate) struct Connection {
    shared: Option<shared::Binding>,
    world: crate::game::session::local_world::LocalWorld,
    resource_collection: bool,
    challenge: Option<u32>,
    display_name: Option<String>,
    configured_name: Option<String>,
    owner_id: u64,
    scoreboard_sent: bool,
    hud_rules_sent: bool,
    hud_rules_sequence: u32,
    profile_ready_sent: bool,
    match_progress: Option<crate::game::progression::MatchProgress>,
    progression_committed: bool,
    progression_retry_at: Option<Instant>,
    progression_ack: Option<(u32, String, u64)>,
    story_completion: Option<(String, u32, u32)>,
    story_committed: bool,
    story_retry_at: Option<Instant>,
    model_rules_sent: bool,
    model_rules_sequence: u32,
    connected: bool,
    stats: codol_protocol::game::snapshot::stats::Upload,
    native_sent: u32,
    native_received: u32,
    native_ack: u32,
    latency: latency::Latency,
    server_commands: ReliableCommands,
    view_offset: [u16; 3],
    native_fragments: VecDeque<Vec<u8>>,
    spawn_view_anchor: Option<([u16; 3], u32)>,
    impacts: crate::game::presentation::impact::ImpactEvents,
    marks_command_sent: bool,
    hit_feedback: crate::game::presentation::feedback::Feedback,
    voices: crate::game::presentation::voice::Speech,
    voice_deaths: VecDeque<(u32, u8, u8)>,
    voice_throws: VecDeque<&'static str>,
    voice_damage_event: u8,
    death_waypoints: crate::game::presentation::death_waypoints::DeathWaypoints,
    replay: crate::game::presentation::replay::Replay,
    last_kill: Option<crate::game::modes::tdm::Kill>,
    final_replay_considered: bool,
    top_players: crate::game::presentation::top_players::Presentation,
    enemy_baseline: Option<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    local_baseline: Option<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    corpses: std::collections::BTreeMap<
        u16,
        (u32, codol_protocol::game::snapshot::corpse::CorpseSnapshot),
    >,
    death_sequence: [u32; 2],
    /// Headshot kills credited to client0 (player) and client1 (bot).
    headshots: [u32; 2],
    obituaries: VecDeque<(
        u32,
        codol_protocol::game::snapshot::obituary::ObituarySnapshot,
    )>,
    next_obituary: u16,
    enemy_events: codol_protocol::game::snapshot::events::EventQueue,
    next_corpse: u16,
    feedback: VecDeque<String>,
    pending_loadout: Option<(u32, Arc<crate::game::session::prepared::PreparedGameState>)>,
    /// Acknowledged backpack change, equipped when the player next spawns.
    next_spawn_loadout: Option<Arc<crate::game::session::prepared::PreparedGameState>>,
    requested_backpack: Option<u8>,
    active_backpack: Option<u8>,
    known_weapons: std::collections::BTreeMap<String, u16>,
    next_weapon: u16,
    shared_registry:
        Option<Arc<std::sync::Mutex<crate::game::session::weapon_registry::WeaponRegistry>>>,
    gamestate_sent: bool,
    gamestate_acknowledged: bool,
    gamestate_update: Option<u8>,
    client_commands: ReliableCommands,
    snapshot_count: u32,
    snapshot_next: Option<Instant>,
    snapshot_epoch: Option<Instant>,
    last_snapshot_time: u32,
    pending_inputs:
        std::collections::BTreeMap<u32, codol_protocol::game::channel::usercmd::UserCommand>,
    predicted_position: Option<(u32, [f32; 3])>,
    snapshot_first_sequence: Option<u32>,
    input_count: u32,
    last_input: Option<codol_protocol::game::channel::usercmd::UserCommand>,
    snapshot_input_bases: VecDeque<(u32, codol_protocol::game::channel::usercmd::UserCommand)>,
    world_history: VecDeque<WorldHistory>,
    match_summary_sequence: Option<u32>,
    match_ui_sequence: Option<u32>,
    match_summary_opened: Option<u32>,
    match_leave_sequence: Option<u32>,
    nuke_banner: Option<(u32, u16)>,
    nuke_return_at: Option<u32>,
    radio_return: Option<(u32, u16)>,
    airdrop_hint: Option<u8>,
    nuke_tick_second: Option<u32>,
}

#[derive(Clone, Default)]
struct WorldHistory {
    weapon_drops: Vec<codol_protocol::game::snapshot::weapon_drops::WeaponDropSnapshot>,
    props: Vec<codol_protocol::game::snapshot::props::PropSnapshot>,
    sounds: Vec<codol_protocol::game::snapshot::sound::SoundSnapshot>,
    bags: Vec<codol_protocol::game::snapshot::scavenger::BagSnapshot>,
    smokes: Vec<codol_protocol::game::snapshot::smoke::SmokeSnapshot>,
    missiles: Vec<codol_protocol::game::snapshot::missile::MissileSnapshot>,
    local_abilities: Option<codol_protocol::game::snapshot::abilities::Abilities>,
    replay_epoch: u32,
    top_players_scene: bool,
    held_weapons: bool,
    sequence: u32,
    local: Option<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    players: Vec<codol_protocol::game::snapshot::enemy::EnemySnapshot>,
    impacts: Vec<codol_protocol::game::snapshot::impact::ImpactSnapshot>,
    corpses: Vec<codol_protocol::game::snapshot::corpse::CorpseSnapshot>,
    obituaries: Vec<codol_protocol::game::snapshot::obituary::ObituarySnapshot>,
}

impl Connection {
    pub fn with_game_state(state: Arc<crate::game::session::prepared::PreparedGameState>) -> Self {
        Self {
            world: crate::game::session::local_world::LocalWorld::from_prepared(state.clone()),
            active_backpack: state
                .selected_backpack_class()
                .map(|i| i % crate::game::player::class_selection::BACKPACKS_PER_CHARACTER),
            view_offset: state.spawn_angles(),
            next_corpse: 19,
            known_weapons: state
                .weapon_configstrings()
                .into_iter()
                .map(|(index, name)| (name, index - 5443))
                .collect(),
            next_weapon: state.next_weapon_index(),
            ..Self::default()
        }
    }
    /// Attach the captured enemy body pose to this connection's target.
    pub(crate) fn set_hit_regions(
        &mut self,
        regions: &Arc<crate::game::combat::hit_location::HitRegions>,
    ) {
        if let Some(combat) = &mut self.world.combat
            && !combat.has_hit_regions()
        {
            combat.set_hit_regions(regions.clone());
            tracing::info!(
                source = regions.source_sha256(),
                "enemy hit regions enabled"
            );
        }
    }

    pub fn input_progress(&self) -> (u32, Option<u32>) {
        (self.input_count, self.last_input.as_ref().map(|c| c.time))
    }
    pub fn prediction_progress(&self) -> Option<(u32, [f32; 3], u32)> {
        self.predicted_position.map(|(time, origin)| {
            (
                time,
                origin,
                self.motion_progress().map_or(0, |(_, _, time)| time),
            )
        })
    }
    pub fn motion_progress(&self) -> Option<([f32; 3], [f32; 3], u32)> {
        if let Some(binding) = &self.shared {
            return binding
                .world
                .lock()
                .ok()?
                .progress(binding.user)
                .ok()
                .map(|(m, _)| (m.origin, m.velocity, m.command_time));
        }
        self.world
            .motion
            .as_ref()
            .map(|m| (m.origin, m.velocity, m.command_time))
    }
    pub fn dive_progress(&self) -> Option<(u32, i32, u32)> {
        if let Some(binding) = &self.shared {
            let (m, _) = binding.world.lock().ok()?.progress(binding.user).ok()?;
            return Some((m.flags, m.jump_time as i32, m.dive_end_time));
        }
        self.world
            .motion
            .as_ref()
            .map(|m| (m.flags, m.jump_time as i32, m.dive_end_time))
    }
    pub fn view_angles(&self) -> [u16; 3] {
        if let Some(binding) = &self.shared
            && let Ok(world) = binding.world.lock()
            && let Ok((_, angles)) = world.progress(binding.user)
        {
            return angles;
        }
        self.last_input
            .as_ref()
            .map_or(self.view_offset, |c| c.angles)
    }
    fn reset_spawn_view(&mut self, angles: [u16; 3]) {
        let raw = self.last_input.as_ref().map_or([0; 3], |c| {
            std::array::from_fn(|i| c.angles[i].wrapping_sub(self.view_offset[i]))
        });
        self.view_offset = std::array::from_fn(|i| angles[i].wrapping_sub(raw[i]));
        // Native respawn resets prediction's input history. Rebase once more
        // from its first command acknowledging the new life snapshot.
        self.spawn_view_anchor = Some((angles, self.native_sent + 1));
        if let Some(input) = &mut self.last_input {
            input.angles = angles;
        }
    }
}

fn identity(bytes: &[u8]) -> Result<u64> {
    ensure!(
        bytes.len() == 16 && bytes.iter().all(u8::is_ascii_hexdigit),
        "invalid connect identity"
    );
    Ok(u64::from_str_radix(std::str::from_utf8(bytes)?, 16)?)
}

fn validate_connect(body: &[u8], owner: u64, build: i32, challenge: u32) -> Result<String> {
    ensure!(body.len() <= 1024, "connect request too long");
    let body = body.strip_suffix(&[0]).unwrap_or(body);
    ensure!(
        body.len() >= 20 && body[16] == b' ',
        "invalid connect framing"
    );
    ensure!(
        identity(&body[..16])? == owner,
        "connect identity differs from admission"
    );
    let info = body[17..]
        .strip_prefix(b"\"")
        .and_then(|s| s.strip_suffix(b"\""))
        .ok_or_else(|| anyhow::anyhow!("connect userinfo must be quoted"))?;
    ensure!(
        !info.iter().any(|b| *b < 32 || *b == 127 || *b == b'"'),
        "invalid connect userinfo character"
    );
    let info = info
        .strip_prefix(b"\\")
        .ok_or_else(|| anyhow::anyhow!("invalid userinfo prefix"))?;
    let mut fields = std::collections::BTreeMap::<&[u8], &[u8]>::new();
    let mut parts = info.split(|b| *b == b'\\');
    while let Some(key) = parts.next() {
        let value = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("unpaired userinfo field"))?;
        ensure!(
            !key.is_empty()
                && key.len() <= 64
                && key.is_ascii()
                && value.len() <= 512
                && fields.len() < 64,
            "userinfo field limit"
        );
        ensure!(
            fields.insert(key, value).is_none(),
            "duplicate userinfo field"
        );
    }
    let get = |key: &[u8]| -> Result<&[u8]> {
        fields
            .get(key)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("missing required connect field"))
    };
    ensure!(
        identity(get(b"xuid")?)? == owner,
        "userinfo identity differs from admission"
    );
    ensure!(
        std::str::from_utf8(get(b"challenge")?)?.parse::<u32>()? == challenge,
        "connect challenge mismatch"
    );
    ensure!(
        std::str::from_utf8(get(b"2490")?)?.parse::<i32>()? == build,
        "connect build mismatch"
    );
    let name = fields
        .get(b"name".as_slice())
        .and_then(|value| std::str::from_utf8(value).ok())
        .filter(|name| !name.is_empty() && name.len() < 64)
        .unwrap_or("Player");
    Ok(decode_connect_name(name))
}

fn decode_connect_name(name: &str) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    STANDARD
        .decode(name)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|decoded| {
            !decoded.is_empty() && decoded.len() < 64 && !decoded.chars().any(char::is_control)
        })
        .unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests;

mod loadout;

mod replication;

mod input;

mod channel;

mod presentation;
mod progression;

mod story;

mod matchplay;

mod rewards;

mod latency;
mod shared;
mod world;

mod weapon_drops;
