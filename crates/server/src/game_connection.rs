//! Application connection bootstrap after authenticated game DATA.
use anyhow::{Result, ensure};
use codol_protocol::game_wire::{self, ClientHeader, ReliableCommands, SERVER_EOF};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Connection {
    challenge: Option<u32>,
    display_name: Option<String>,
    owner_id: u64,
    scoreboard_sent: bool,
    hud_rules_sent: bool,
    model_rules_sent: bool,
    model_rules_sequence: u32,
    connected: bool,
    stats: codol_protocol::game_stats::Upload,
    native_sent: u32,
    native_received: u32,
    native_ack: u32,
    server_commands: ReliableCommands,
    game_state: Option<Arc<crate::game_state::PreparedGameState>>,
    player: Option<crate::game_player::Player>,
    motion: Option<crate::game_motion::Motion>,
    combat: Option<crate::game_combat::CombatWorld>,
    impacts: crate::game_impact::ImpactEvents,
    marks_command_sent: bool,
    hit_feedback: crate::game_feedback::Feedback,
    death_waypoints: crate::game_death_waypoints::DeathWaypoints,
    replay: crate::game_replay::Replay,
    last_kill: Option<crate::game_match::Kill>,
    final_replay_considered: bool,
    top_players: crate::game_top_players::Presentation,
    enemy_baseline: Option<codol_protocol::game_enemy::EnemySnapshot>,
    local_baseline: Option<codol_protocol::game_enemy::EnemySnapshot>,
    corpses: std::collections::BTreeMap<u16, (u32, codol_protocol::game_corpse::CorpseSnapshot)>,
    death_sequence: [u32; 2],
    /// Headshot kills credited to client0 (player) and client1 (bot).
    headshots: [u32; 2],
    obituaries: VecDeque<(u32, codol_protocol::game_obituary::ObituarySnapshot)>,
    next_obituary: u16,
    enemy_events: codol_protocol::game_events::EventQueue,
    bot: Option<crate::game_bot::BotController>,
    tdm: Option<crate::game_match::TdmMatch>,
    next_corpse: u16,
    bot_frame: Option<crate::game_bot::BotFrame>,
    bot_fire_time: Option<u32>,
    bot_fire_toggle: bool,
    vitals: codol_protocol::game_snapshot::PlayerVitals,
    player_birth: u32,
    player_death: Option<u32>,
    health_recovery: crate::game_health::HealthRecovery,
    feedback: VecDeque<String>,
    pending_loadout: Option<(u32, Arc<crate::game_state::PreparedGameState>)>,
    /// Acknowledged backpack change, equipped when the player next spawns.
    next_spawn_loadout: Option<Arc<crate::game_state::PreparedGameState>>,
    requested_backpack: Option<u8>,
    active_backpack: Option<u8>,
    known_weapons: std::collections::BTreeMap<String, u16>,
    next_weapon: u16,
    shared_registry: Option<Arc<std::sync::Mutex<crate::game_weapon_registry::WeaponRegistry>>>,
    gamestate_sent: bool,
    gamestate_acknowledged: bool,
    gamestate_update: Option<u8>,
    client_commands: ReliableCommands,
    snapshot_count: u32,
    snapshot_next: Option<Instant>,
    snapshot_epoch: Option<Instant>,
    last_snapshot_time: u32,
    pending_inputs: std::collections::BTreeMap<u32, codol_protocol::game_usercmd::UserCommand>,
    predicted_position: Option<(u32, [f32; 3])>,
    snapshot_first_sequence: Option<u32>,
    input_count: u32,
    last_input: Option<codol_protocol::game_usercmd::UserCommand>,
    snapshot_input_bases: VecDeque<(u32, codol_protocol::game_usercmd::UserCommand)>,
    world_history: VecDeque<WorldHistory>,
    match_summary_sequence: Option<u32>,
    match_ui_sequence: Option<u32>,
    match_summary_opened: Option<u32>,
    match_leave_sequence: Option<u32>,
}

#[derive(Clone)]
struct WorldHistory {
    replay_epoch: u32,
    top_players_scene: bool,
    held_weapons: bool,
    sequence: u32,
    local: Option<codol_protocol::game_enemy::EnemySnapshot>,
    players: Vec<codol_protocol::game_enemy::EnemySnapshot>,
    impacts: Vec<codol_protocol::game_impact::ImpactSnapshot>,
    corpses: Vec<codol_protocol::game_corpse::CorpseSnapshot>,
    obituaries: Vec<codol_protocol::game_obituary::ObituarySnapshot>,
}

impl Connection {
    pub fn with_game_state(state: Arc<crate::game_state::PreparedGameState>) -> Self {
        Self {
            player: state.player(),
            motion: state.motion(),
            combat: state.combat(),
            bot: state.bot(),
            tdm: state.tdm_match().expect("validated TDM state"),
            next_corpse: 19,
            vitals: Default::default(),
            player_birth: 1000,
            known_weapons: state
                .weapon_configstrings()
                .into_iter()
                .map(|(index, name)| (name, index - 5443))
                .collect(),
            next_weapon: state.next_weapon_index(),
            game_state: Some(state),
            ..Self::default()
        }
    }
    pub(crate) fn refresh_profile(
        &mut self,
        template: &crate::game_state::PreparedGameState,
        public: &[u8],
        owned: &[u32],
        rank: (u8, u8),
    ) -> Result<()> {
        if !template.dynamic_loadout()
            || !self.gamestate_acknowledged
            || self.pending_loadout.is_some()
        {
            return Ok(());
        }
        let Some(current) = &self.game_state else {
            return Ok(());
        };
        let hash: [u8; 32] = Sha256::digest(public).into();
        if current.profile_hash == Some(hash)
            && current
                .player_card
                .is_none_or(|c| (c.rank, c.prestige) == rank)
            && self.requested_backpack.is_none()
        {
            return Ok(());
        }
        // Attribute resolution uses a temporary valid slot; the persistent
        // registry then assigns the client's canonical name/index identity.
        let slot = self.requested_backpack.or(self.active_backpack);
        let selected = slot
            .map(|slot| template.for_backpack(public, slot))
            .transpose()?;
        let mut source = selected.as_ref().unwrap_or(template).clone();
        source.preserve_match_appearance(current);
        let mut state = match source.for_profile_at(public, owned, template.first_weapon_index()) {
            Ok(state) => state,
            Err(error) if self.requested_backpack.is_some() => {
                self.requested_backpack = None;
                tracing::warn!(%error,"selected backpack is unavailable");
                self.feedback
                    .push_back("Selected backpack has no usable weapon".into());
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        state.set_card_rank(rank);
        state.preserve_match_appearance(current);
        self.active_backpack = slot;
        self.requested_backpack = None;
        tracing::info!(slot,global_class=?state.selected_backpack_class(),"selected in-game backpack");
        if current.selected_weapon_names() == state.selected_weapon_names() {
            if current.player_card != state.player_card {
                self.world_history.clear();
            }
            Arc::make_mut(self.game_state.as_mut().unwrap()).update_profile_selection(&state);
            if self.next_spawn_loadout.take().is_some() {
                self.feedback
                    .push_back("Current backpack kept for next spawn".into());
            }
            return Ok(());
        }
        let mut commands = self.server_commands.clone();
        let mut known = self.known_weapons.clone();
        let mut next = self.next_weapon;
        let shared = self.shared_registry.clone();
        let mut guard = shared
            .as_ref()
            .map(|r| {
                r.lock()
                    .map_err(|_| anyhow::anyhow!("weapon registry poisoned"))
            })
            .transpose()?;
        let mut registry = guard.as_ref().map_or_else(
            || crate::game_weapon_registry::WeaponRegistry {
                names: known.clone(),
                next,
            },
            |r| (**r).clone(),
        );
        state.register_history(&mut registry)?;
        known = registry.names.clone();
        next = registry.next;
        let additions = state
            .weapon_configstrings()
            .into_iter()
            .filter(|(id, name)| self.known_weapons.get(name) != Some(&(id - 5443)))
            .collect::<Vec<_>>();
        let new_resources = !additions.is_empty();
        for (index, name) in additions {
            commands.push(format!("d {index} {name}").as_bytes())?;
        }
        if new_resources {
            commands.push(format!("d 6800 {next}").as_bytes())?;
        }
        self.pending_loadout = Some((commands.sequence(), Arc::new(state)));
        self.server_commands = commands;
        self.known_weapons = known;
        self.next_weapon = next;
        if let Some(guard) = &mut guard {
            **guard = registry;
        }
        tracing::info!("queued saved backpack update for client resource acknowledgement");
        Ok(())
    }
    /// Client resources for a backpack are acknowledged; equip it now, or at
    /// the next training spawn while the current life is still in progress.
    fn apply_acknowledged_loadout(&mut self) {
        if self
            .pending_loadout
            .as_ref()
            .is_none_or(|(seq, _)| self.server_commands.acknowledged() < *seq)
        {
            return;
        }
        let (_, state) = self.pending_loadout.take().unwrap();
        if (self.bot.is_some() || self.tdm.is_some()) && self.player.is_some() {
            // A training life keeps its weapons; respawn equips the choice.
            self.next_spawn_loadout = Some(state);
            self.feedback
                .push_back("Backpack will change on next spawn".into());
            tracing::info!("deferred acknowledged player backpack update to respawn");
        } else {
            if let Some(primary) = state.primary_player() {
                if let Some(player) = &mut self.player {
                    player.replace_loadout(primary, state.secondary_player());
                } else {
                    self.player = state.player();
                }
            }
            self.game_state = Some(state);
            tracing::info!("applied acknowledged player backpack update");
        }
    }
    fn equip_spawn_loadout(&mut self) {
        if let Some(next) = self.next_spawn_loadout.take() {
            self.game_state = Some(next);
            tracing::info!("applied deferred player backpack at respawn");
        }
    }
    /// Attach the captured enemy body pose to this connection's target.
    pub(crate) fn set_hit_regions(&mut self, regions: &Arc<crate::game_hit_location::HitRegions>) {
        if let Some(combat) = &mut self.combat
            && !combat.has_hit_regions()
        {
            combat.set_hit_regions(regions.clone());
            tracing::info!(
                source = regions.source_sha256(),
                "enemy hit regions enabled"
            );
        }
    }
    pub(crate) fn with_weapon_registry(
        mut self,
        registry: Arc<std::sync::Mutex<crate::game_weapon_registry::WeaponRegistry>>,
    ) -> Self {
        self.shared_registry = Some(registry);
        self
    }

    pub fn stage(&self) -> &'static str {
        if self.gamestate_acknowledged {
            "gamestate_acknowledged"
        } else if self.gamestate_update == Some(6) {
            "gamestate_initialized"
        } else if self.gamestate_update.is_some() {
            "gamestate_parser_entered"
        } else if self.gamestate_sent {
            "gamestate_sent"
        } else if self.server_commands.acknowledged() == 1 {
            "reliable_probe_acknowledged"
        } else if self.stats.profile().is_some() {
            "stats_received"
        } else if self.connected {
            "connected"
        } else if self.challenge.is_some() {
            "challenged"
        } else {
            "unconnected"
        }
    }

    pub fn gamestate_progress(&self) -> Option<u8> {
        self.gamestate_update
    }

    /// An explicitly bounded stream at 20 Hz. The probe has no physics/input
    /// execution and is disabled unless explicitly selected by the operator.
    pub fn poll_snapshot(&mut self, now: Instant) -> Result<Option<Vec<u8>>> {
        if self.gamestate_update != Some(6)
            || self
                .match_leave_sequence
                .is_some_and(|sequence| self.server_commands.acknowledged() >= sequence)
            || self.snapshot_count >= self.game_state.as_ref().map_or(0, |s| s.snapshot_limit())
            || !self.game_state.as_ref().is_some_and(|s| s.snapshot_probe())
            || self.snapshot_next.is_some_and(|next| now < next)
        {
            return Ok(None);
        }
        self.apply_acknowledged_loadout();
        // Wall-clock time must keep advancing even if capture I/O misses a
        // tick. Counting frames made the server clock fall behind the client.
        let epoch = *self.snapshot_epoch.get_or_insert(now);
        let server_time = 1000
            + now
                .duration_since(epoch)
                .as_millis()
                .min(u128::from(i32::MAX as u32 - 1000)) as u32;
        self.last_snapshot_time = server_time;
        self.replay.advance(server_time);
        if self.server_commands.acknowledged() >= self.model_rules_sequence
            && self
                .snapshot_first_sequence
                .is_some_and(|first| self.native_ack >= first)
            && let Some(tdm) = &mut self.tdm
        {
            tdm.advance_clock(server_time);
        }
        self.consume_inputs(server_time);
        if !self.hud_rules_sent
            && self.player.is_some()
            && self
                .snapshot_first_sequence
                .is_some_and(|first| self.native_ack >= first)
        {
            let limit = self
                .tdm
                .as_ref()
                .map_or(7500, |m| m.scoreboard(self.owner_id).score_limit);
            self.server_commands
                .push(format!("s ui_scorelimit \"{limit}\"").as_bytes())?;
            if self.tdm.is_some() {
                self.server_commands
                    .push(b"s scr_gameended \"0\" ui_match_complete \"0\" ui_eog_show \"0\" ui_hide_playercards \"0\"")?;
            }
            self.hud_rules_sent = true;
        }
        if self.combat.is_some()
            && !self.marks_command_sent
            && self
                .snapshot_first_sequence
                .is_some_and(|first| self.native_ack >= first)
        {
            // Current0x57d560 's' consumes controlled dvar/value pairs via
            // 0x57b690. Client graphics initialization can override +set;
            // apply once after map initialization for this opt-in training.
            self.server_commands.push(b"s fx_marks \"1\"")?;
            self.marks_command_sent = true;
        }
        if !self.model_rules_sent {
            // Initial model registrations are already in the acknowledged
            // gamestate. Sending d here starts streaming only after spawning.
            self.model_rules_sent = true;
        }
        if let Some(combat) = &mut self.combat
            && let Some(message) = combat.advance(server_time)
        {
            self.feedback.push_back(message);
            if let Some(bot) = &mut self.bot {
                let spawn = self
                    .game_state
                    .as_ref()
                    .unwrap()
                    .enemy_snapshot(true)
                    .unwrap();
                let origin = crate::game_bot::grounded_spawn(
                    spawn.origin,
                    self.game_state
                        .as_ref()
                        .unwrap()
                        .collision
                        .as_ref()
                        .unwrap(),
                )?;
                bot.respawn(server_time, origin, spawn.angles)?;
                self.enemy_events = Default::default();
                self.bot_fire_time = None;
            }
        }
        if self.tdm.is_some() {
            self.advance_match(server_time)?;
        } else {
            self.advance_bot(server_time)?;
        }
        if self.tdm.as_ref().is_some_and(|m| m.finished) && !self.final_replay_considered {
            self.final_replay_considered = true;
            if let Some(kill) = self.last_kill {
                self.replay.start(
                    crate::game_replay::Kind::Final,
                    kill.time,
                    server_time,
                    kill.attacker,
                );
            }
        }
        if let Some(tdm) = &mut self.tdm
            && (tdm.dirty || !self.scoreboard_sent)
        {
            for payload in
                codol_protocol::game_scoreboard::binary_commands(&tdm.scoreboard(self.owner_id))?
            {
                self.server_commands.push_binary(&payload)?;
            }
            tdm.dirty = false;
            self.scoreboard_sent = true;
            if tdm.finished && !tdm.result_sent {
                for payload in codol_protocol::game_scoreboard::summary_identity_commands(
                    self.display_name.as_deref().unwrap_or("Player"),
                    tdm.teams(),
                )? {
                    self.server_commands.push_binary(&payload)?;
                }
                self.server_commands.push_binary(
                    &codol_protocol::game_scoreboard::summary_command(
                        tdm.winning_team(),
                        tdm.duration_seconds(),
                        self.game_state.as_ref().unwrap().summary_map_id()?,
                        1,
                    )?,
                )?;
                self.match_summary_sequence = Some(self.server_commands.sequence());
                tdm.result_sent = true;
            }
        }
        self.advance_match_summary(server_time)?;
        if !self.scoreboard_sent
            && self
                .game_state
                .as_ref()
                .is_some_and(|state| state.enemy_snapshot(true).is_some())
            && self
                .snapshot_first_sequence
                .is_some_and(|first| self.native_ack >= first)
        {
            let rows = vec![
                codol_protocol::game_scoreboard::ScoreboardRow {
                    client_number: 0,
                    user_id: self.owner_id,
                    kills: self.death_sequence[1],
                    deaths: self.death_sequence[0],
                    headshots: self.headshots[0],
                    score: (self.death_sequence[1] * 100) as i32,
                    ..Default::default()
                },
                codol_protocol::game_scoreboard::ScoreboardRow {
                    client_number: 1,
                    user_id: 1,
                    kills: self.death_sequence[0],
                    deaths: self.death_sequence[1],
                    headshots: self.headshots[1],
                    score: (self.death_sequence[0] * 100) as i32,
                    ..Default::default()
                },
            ];
            let board = codol_protocol::game_scoreboard::ScoreboardSnapshot {
                rows,
                team_scores: [
                    0,
                    (self.death_sequence[0] * 100) as i32,
                    (self.death_sequence[1] * 100) as i32,
                    0,
                ],
                score_limit: 7500,
            };
            for payload in codol_protocol::game_scoreboard::binary_commands(&board)? {
                self.server_commands.push_binary(&payload)?;
            }
            self.scoreboard_sent = true;
        }
        while let Some(message) = self.feedback.pop_front() {
            // Target 0x57d560's c command displays one controlled text argument.
            let command = format!("c \"{message}\"");
            self.server_commands.push(command.as_bytes())?;
        }
        let sounds = self.hit_feedback.take_sounds();
        if let Some(resources) = self
            .game_state
            .as_ref()
            .and_then(|s| s.feedback_resources())
        {
            for _ in 0..sounds {
                self.server_commands
                    .push(resources.sound_command().as_bytes())?;
            }
        }
        let state = self.game_state.as_ref().expect("enabled state");
        let mut angles = if state.has_spawn() {
            self.view_angles()
        } else {
            [0; 3]
        };

        // View and the supported M4 actions are applied by this bootstrap. Advancing commandTime
        // with the snapshot clock suppresses local prediction (0x4522b0).
        let mut command_time = self
            .last_input
            .as_ref()
            .map_or(1000, |cmd| cmd.time)
            .min(server_time);
        let mut equipment = self.player.as_ref().map(|p| p.equipment());
        let mut impacts = self.impacts.snapshot(server_time);
        if self.tdm.is_some() {
            impacts.truncate(8);
        }
        self.obituaries
            .retain(|(time, _)| server_time.wrapping_sub(*time) < 200);
        let mut obituaries: Vec<_> = self.obituaries.iter().map(|(_, event)| *event).collect();
        obituaries.sort_unstable_by_key(|event| event.entity_number);
        let mut corpses: Vec<_> = self
            .corpses
            .values()
            .map(|(time, corpse)| {
                let mut current = *corpse;
                let age = server_time.wrapping_sub(*time);
                current.phase = if age < 1200 {
                    codol_protocol::game_corpse::CorpsePhase::Playing
                } else {
                    codol_protocol::game_corpse::CorpsePhase::Frozen
                };
                current.present = age < 6000;
                current
            })
            .collect();
        if self.local_baseline.is_none() {
            self.local_baseline = state.local_player_snapshot(true);
        }
        let mut local_player = state.local_player_snapshot(self.vitals.health > 0);
        if let (Some(local), Some(motion)) = (&mut local_player, self.motion.as_ref()) {
            local.origin = motion.origin;
            local.angles = angles;
            if let Some(player) = self.player.as_ref() {
                local.weapon = player.equipment().weapon;
            }
        }
        if self.enemy_baseline.is_none() {
            self.enemy_baseline = state.enemy_snapshot(true);
        }
        let mut enemy =
            state.enemy_snapshot(self.combat.as_ref().is_some_and(|world| world.health() > 0));
        if let (Some(current), Some(frame)) = (enemy, self.bot_frame.as_ref()) {
            enemy = Some(
                state.animate_enemy(
                    current,
                    frame,
                    self.bot_fire_time
                        .is_some_and(|time| server_time.wrapping_sub(time) < 150),
                    self.bot_fire_toggle,
                ),
            );
        }
        if let Some(current) = &mut enemy {
            current.events = self.enemy_events;
        }
        let models_ready = self.server_commands.acknowledged() >= self.model_rules_sequence;
        if self.tdm.is_some() && !models_ready {
            // The selected local outfit now needs the same registration/ACK
            // gate as remote actors. PS input can start without its DObj.
            local_player = None;
        }
        let mut match_players = if models_ready {
            self.tdm.as_ref().map_or(Vec::new(), |m| m.snapshots())
        } else {
            Vec::new()
        };
        if self.tdm.is_some() {
            enemy = None;
        }
        let mut snapshot_motion = self.snapshot_motion();
        let mut snapshot_vitals = (self.bot.is_some() || self.tdm.is_some()).then_some(self.vitals);
        let mut hit_feedback = state
            .feedback_resources()
            .map(|r| self.hit_feedback.snapshot(server_time, r));
        if let Some(tdm) = self.tdm.as_ref().filter(|_| models_ready) {
            let mut views = tdm.replay_views();
            if let (Some(motion), Some(equipment)) = (self.motion.as_ref(), equipment) {
                views.push(crate::game_replay::View {
                    client: 0,
                    motion: motion.clone(),
                    angles,
                    equipment,
                    vitals: self.vitals,
                    hit: self.hit_feedback.event(),
                });
            }
            views.retain(|v| {
                v.vitals.health > 0
                    || corpses
                        .iter()
                        .any(|c| c.present && c.donor_client == v.client)
            });
            for view in &mut views {
                if view.vitals.health == 0 {
                    if let Some(corpse) = corpses
                        .iter()
                        .filter(|c| c.donor_client == view.client && c.present)
                        .max_by_key(|c| c.death_sequence)
                    {
                        view.vitals.corpse_slot = (corpse.entity_number - 19) as u8;
                    }
                    view.motion.velocity = [0.0; 3];
                }
            }
            self.replay.record(crate::game_replay::Frame {
                time: server_time,
                views,
                local: local_player,
                players: match_players.clone(),
                corpses: corpses.clone(),
                impacts: impacts.clone(),
                obituaries: obituaries.clone(),
            });
        }
        let live_players = match_players.clone();
        let top_players_scene = self.top_players.scene.is_some();
        if let Some(scene) = &self.top_players.scene {
            angles = scene.angles;
            command_time = server_time;
            let mut camera = scene.camera.clone();
            camera.command_time = server_time;
            snapshot_motion = Some(camera);
            snapshot_vitals = Some(codol_protocol::game_snapshot::PlayerVitals {
                intermission: true,
                ..Default::default()
            });
            local_player = scene.actors.first().copied();
            match_players = scene.actors[1..].to_vec();
            corpses.clear();
            impacts.clear();
            obituaries.clear();
            hit_feedback = None;
        }
        let playback = self.replay.sample(server_time);
        let replay_epoch = playback.as_ref().map_or(self.replay.epoch, |p| p.epoch);
        let replay_wire = playback
            .as_ref()
            .map(|p| codol_protocol::game_snapshot::ReplayView {
                client: p.view.client,
                delay_msec: p.delay,
                end_time: p.end_time,
                skip_text: state.replay_resources().and_then(|r| r.skip_text(p.kind)),
            });
        if let Some(playback) = playback {
            if playback.entering {
                if playback.kind == crate::game_replay::Kind::Final {
                    // Native HUD selects the final-kill title from this flag.
                    // The summary menu remains gated until the podium ends;
                    // the next round already resets scr_gameended to zero.
                    self.server_commands.push(b"s scr_gameended \"1\"")?;
                }
                // 3.3.3.3 dispatcher0x57dc12: L <client> <card slot>.
                // 0x6889c0 copies clientInfo identity/decorations; the native
                // killcam HUD0x5d3f70 reads slot7 rather than followed PS.
                self.server_commands
                    .push(format!("L {} 7", playback.view.client).as_bytes())?;
            }
            angles = playback.view.angles;
            command_time = playback.view.motion.command_time;
            equipment = Some(playback.view.equipment);
            snapshot_motion = Some(playback.view.motion);
            snapshot_vitals = Some(playback.view.vitals);
            local_player = playback.frame.local;
            match_players = playback.frame.players;
            corpses = playback.frame.corpses;
            impacts = playback.frame.impacts;
            obituaries = playback.frame.obituaries;
            hit_feedback = state.feedback_resources().map(|resources| {
                let mut feedback = crate::game_feedback::Feedback::default();
                if let Some((time, kind)) = playback.view.hit {
                    feedback.record(time, kind);
                }
                feedback.snapshot(server_time, resources)
            });
            if let Some((time, _)) = playback
                .view
                .hit
                .filter(|(time, _)| server_time.wrapping_sub(*time) < 100)
            {
                let source_time = time.wrapping_sub(playback.delay);
                if self.replay.last_sound != Some(source_time) {
                    self.replay.last_sound = Some(source_time);
                    if let Some(resources) = state.feedback_resources() {
                        self.server_commands
                            .push(resources.sound_command().as_bytes())?;
                    }
                }
            }
        }
        let match_baselines = self.tdm.as_ref().map_or(&[][..], |m| m.baselines());
        // parseClients is circular; a multiplayer delta cannot retain the
        // original full frame's appearance-record indices indefinitely.
        let previous = if self.tdm.is_some() {
            self.world_history
                .iter()
                .rev()
                .find(|h| {
                    h.sequence <= self.native_ack
                        && h.replay_epoch == replay_epoch
                        && h.top_players_scene == top_players_scene
                        && self.native_sent + 1 - h.sequence < 24
                        && h.players.len() == match_players.len()
                })
                .cloned()
        } else {
            None
        };
        let full = if self.tdm.is_some() {
            previous.is_none()
        } else {
            self.snapshot_first_sequence
                .is_none_or(|first| self.native_ack < first)
        };
        let delta_distance = previous
            .as_ref()
            .map_or(0, |h| (self.native_sent + 1 - h.sequence) as u8);
        let previous_players = previous
            .as_ref()
            .map_or(match_baselines, |h| h.players.as_slice());
        let death_waypoints = state
            .feedback_resources()
            .and_then(|r| r.death_material)
            .map(|material| {
                let team = self.tdm.as_ref().map_or(0, |m| m.teams()[0]);
                self.death_waypoints.snapshot(
                    server_time,
                    team,
                    &live_players,
                    material,
                    self.vitals.health > 0 && !self.vitals.intermission && replay_wire.is_none(),
                    self.tdm.as_ref().is_none_or(|m| m.finished),
                )
            });
        let snapshot = state.snapshot(
            server_time,
            command_time,
            full,
            angles,
            equipment,
            crate::game_state::SnapshotWorld {
                delta_distance,
                corpse_baselines: previous.as_ref().map_or(&[], |h| h.corpses.as_slice()),
                impact_baselines: previous.as_ref().map_or(&[], |h| h.impacts.as_slice()),
                obituary_baselines: previous.as_ref().map_or(&[], |h| h.obituaries.as_slice()),
                motion: snapshot_motion.as_ref(),
                target: if state.enemy_snapshot(true).is_none() {
                    self.combat.as_ref().map(|w| w.snapshot())
                } else {
                    None
                },
                impacts: &impacts,
                enemy: if self.tdm.is_some() { None } else { enemy },
                enemy_baseline: self.enemy_baseline,
                players: &match_players,
                player_baselines: previous_players,
                vitals: snapshot_vitals,
                local_player,
                local_player_baseline: if self.tdm.is_some() {
                    previous.as_ref().and_then(|h| h.local).filter(|b| b.alive)
                } else {
                    self.local_baseline
                },
                corpses: &corpses,
                obituaries: &obituaries,
                hit_feedback,
                death_waypoints: death_waypoints.as_deref(),
                replay: replay_wire,
                held_weapons_in_baseline: previous.as_ref().is_some_and(|h| h.held_weapons),
            },
        )?;
        let budget = game_wire::FRAGMENT_SIZE - 5;
        ensure!(
            snapshot.len() <= budget,
            "player snapshot exceeds native message budget"
        );
        let (mut body, mut sent) = self
            .server_commands
            .encode_frames_bounded(budget - snapshot.len())?;
        // First send the local player's PS so the client has a real input
        // baseline. Bot appearances wait for their model commands' ACK.
        let snapshot_included = !(body.is_empty()
            && self.server_commands.sequence() > self.server_commands.acknowledged()
            && self.snapshot_first_sequence.is_some());
        if snapshot_included {
            body.extend_from_slice(&snapshot);
        } else {
            (body, sent) = self.server_commands.encode_frames_bounded(budget - 1)?;
            ensure!(
                !body.is_empty(),
                "reliable record exceeds native message budget"
            );
            body.push(SERVER_EOF);
        }
        let ack = self.client_commands.sequence();
        let packet = game_wire::server_packet(
            self.native_sent + 1,
            ack,
            self.challenge
                .ok_or_else(|| anyhow::anyhow!("snapshot before challenge"))?,
            self.client_commands.command_for(ack)?,
            &body,
        )?;
        self.native_sent += 1;
        self.server_commands.mark_sent(sent)?;
        if !snapshot_included {
            self.snapshot_next = Some(now + Duration::from_millis(50));
            return Ok(Some(packet));
        }
        if self.tdm.is_some() {
            self.world_history.push_back(WorldHistory {
                replay_epoch,
                top_players_scene,
                held_weapons: !full,
                sequence: self.native_sent,
                local: local_player,
                players: match_players,
                impacts: if full { Vec::new() } else { impacts },
                corpses: if full { Vec::new() } else { corpses },
                obituaries: if full { Vec::new() } else { obituaries },
            });
            while self.world_history.len() > 32 {
                self.world_history.pop_front();
            }
        }
        let mut base = state.input_base();
        if let Some(equipment) = equipment {
            base.weapons = equipment.input_weapons();
        }
        // 0x5a18e0 copies pitch/yaw only; the default command's roll stays0.
        base.angles = [angles[0], angles[1], 0];
        // 0x5a18e0 sets BUTTON_ADS whenever the acknowledged PS fraction is
        // nonzero, including the exit transition after the physical release.
        if equipment
            .and_then(|e| e.aim)
            .is_some_and(|a| a.fraction != 0.0)
        {
            base.buttons |= 0x800;
        }
        self.remember_input_base(self.native_sent, base);
        self.snapshot_first_sequence.get_or_insert(self.native_sent);
        self.snapshot_count += 1;
        let next = self.snapshot_next.map_or(now, |n| n) + Duration::from_millis(50);
        self.snapshot_next = Some(if next > now {
            next
        } else {
            now + Duration::from_millis(50)
        });
        Ok(Some(packet))
    }
    pub fn input_progress(&self) -> (u32, Option<u32>) {
        (self.input_count, self.last_input.as_ref().map(|c| c.time))
    }
    fn remember_input_base(
        &mut self,
        sequence: u32,
        base: codol_protocol::game_usercmd::UserCommand,
    ) {
        self.snapshot_input_bases.push_back((sequence, base));
        while self.snapshot_input_bases.len() > 256 {
            let retain_acked = self
                .snapshot_input_bases
                .front()
                .is_some_and(|(s, _)| *s <= self.native_ack)
                && self
                    .snapshot_input_bases
                    .get(1)
                    .is_some_and(|(s, _)| *s > self.native_ack);
            // A delayed client still encodes commands against its last ACK.
            // Retain that exact base while bounding the newer history.
            if retain_acked {
                self.snapshot_input_bases.remove(1);
            } else {
                self.snapshot_input_bases.pop_front();
            }
        }
    }
    pub fn prediction_progress(&self) -> Option<(u32, [f32; 3], u32)> {
        self.predicted_position.map(|(time, origin)| {
            (
                time,
                origin,
                self.motion.as_ref().map_or(0, |m| m.command_time),
            )
        })
    }
    pub fn motion_progress(&self) -> Option<([f32; 3], [f32; 3], u32)> {
        self.motion
            .as_ref()
            .map(|m| (m.origin, m.velocity, m.command_time))
    }
    fn snapshot_motion(&self) -> Option<crate::game_motion::Motion> {
        self.motion.as_ref().map(|motion| {
            let mut camera = motion.clone();
            if self.vitals.intermission {
                // CG view offset 0x51d3b0 returns immediately for PM_INTERMISSION6.
                // Its origin must already be the eye, rather than the actor's feet.
                // Keep the actor and collision origin unchanged.
                camera.origin[2] += camera.view_height;
                camera.velocity = [0.0; 3];
                camera.view_height = 0.0;
                camera.view_height_target = 0;
                camera.view_height_lerp_target = 0;
                camera.view_height_lerp_time = 0;
                camera.view_height_lerp_down = false;
            }
            camera
        })
    }
    pub fn view_angles(&self) -> [u16; 3] {
        self.last_input.as_ref().map_or([0; 3], |c| c.angles)
    }
    fn input_base_for(&self, ack: u32) -> Result<codol_protocol::game_usercmd::UserCommand> {
        self.snapshot_input_bases
            .iter()
            .rev()
            .find(|(sequence, _)| *sequence <= ack)
            .map(|(_, base)| base.clone())
            .ok_or_else(|| anyhow::anyhow!("no retained input base for snapshot ACK"))
    }

    pub fn decode_native(&self, payload: &[u8]) -> Result<(ClientHeader, Vec<u8>)> {
        let challenge = self
            .challenge
            .ok_or_else(|| anyhow::anyhow!("native message before challenge"))?;
        let header = ClientHeader::parse(payload)?;
        let command = self.server_commands.command_for(header.reliable_ack)?;
        Ok(game_wire::open_client(payload, challenge, command)?)
    }

    fn native(&mut self, payload: &[u8]) -> Result<Option<Vec<u8>>> {
        // Observed 2-byte association maintenance packet has no native header.
        if payload.len() < 4 {
            return Ok(None);
        }
        ensure!(
            self.connected && self.stats.profile().is_some(),
            "native message before stats completion"
        );
        let (header, clear) = self.decode_native(payload)?;
        ensure!(
            header.server_id == 0 || (self.gamestate_sent && header.server_id == 1),
            "unexpected server id"
        );
        ensure!(
            header.sequence > self.native_received,
            "native sequence replay"
        );
        ensure!(
            header.message_ack >= self.native_ack && header.message_ack <= self.native_sent,
            "native ACK outside sent range"
        );
        let key = self
            .snapshot_first_sequence
            .filter(|first| header.message_ack >= *first)
            .map(|_| {
                codol_protocol::game_usercmd::key(
                    header.message_ack,
                    0,
                    self.server_commands
                        .command_for(header.reliable_ack)
                        .expect("decoded slot"),
                )
            });
        let base = if key.is_some() {
            self.input_base_for(header.message_ack)?
        } else {
            Default::default()
        };
        let received = game_wire::client_message_with_base(&clear, key, &base)?;
        ensure!(
            self.gamestate_sent || received.commands.is_empty(),
            "client commands before gamestate"
        );
        let mut client_commands = self.client_commands.clone();
        for (sequence, command) in received.commands {
            if sequence <= client_commands.sequence() {
                ensure!(
                    client_commands.command_for(sequence)? == command,
                    "changed client command retry"
                );
            } else {
                ensure!(
                    sequence == client_commands.sequence() + 1,
                    "client command sequence gap"
                );
                if let Ok(text) = std::str::from_utf8(&command) {
                    if text == "s" {
                        self.scoreboard_sent = false;
                    }
                    if text.starts_with("mr ") || text == "s" || text.starts_with("class ") {
                        tracing::info!(command=%text,"game menu request");
                    }
                }
                match crate::game_class_selection::from_client_command(&command, 1) {
                    Ok(Some(slot)) => {
                        self.requested_backpack = Some(slot);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error,"declined unsupported backpack menu request");
                    }
                }
                // Only explicitly implemented semantic requests are applied.
                client_commands.push(&command)?;
                client_commands.mark_sent(sequence)?;
                client_commands.acknowledge(sequence)?;
            }
        }
        let mut commands = self.server_commands.clone();
        commands.acknowledge(header.reliable_ack)?;
        let challenge = self.challenge.expect("decoded challenge");
        let client_ack = client_commands.sequence();
        let client_command = client_commands.command_for(client_ack)?;
        let mut sending_gamestate = false;
        let reply = if self.native_sent == 0 {
            Some(game_wire::server_packet(
                1,
                client_ack,
                challenge,
                client_command,
                &[SERVER_EOF],
            )?)
        } else if self.native_sent == 1 && header.message_ack == 1 {
            // Empty server command exercises the receiver and command ACK with
            // no configstring mutation. Client command ACK remains zero.
            let sequence = commands.push(b"")?;
            let mut body = vec![1];
            body.extend_from_slice(&sequence.to_le_bytes());
            body.extend_from_slice(&[0, SERVER_EOF]);
            let reply = game_wire::server_packet(2, client_ack, challenge, client_command, &body)?;
            commands.mark_sent(sequence)?;
            Some(reply)
        } else if self.native_sent == 2
            && header.message_ack == 2
            && header.reliable_ack == 1
            && let Some(state) = &self.game_state
        {
            let body = state.body_named(commands.sequence(), self.display_name.as_deref())?;
            let reply =
                game_wire::compact_server_packet(3, client_ack, challenge, client_command, &body)?;
            sending_gamestate = true;
            Some(reply)
        } else if client_ack > self.client_commands.sequence() {
            Some(game_wire::server_packet(
                self.native_sent + 1,
                client_ack,
                challenge,
                client_command,
                &[SERVER_EOF],
            )?)
        } else {
            None
        };
        self.server_commands = commands;
        self.client_commands = client_commands;
        if let Some(movement) = received.movement {
            self.predicted_position = Some((
                movement.predicted_time,
                movement.predicted_origin_bits.map(f32::from_bits),
            ));
            for input in movement.commands {
                self.queue_input(input);
            }
            self.consume_inputs(self.last_snapshot_time.max(1000));
        }
        self.gamestate_sent |= sending_gamestate;
        self.gamestate_acknowledged |=
            self.gamestate_sent && header.message_ack >= 3 && header.server_id == 1;
        self.native_received = header.sequence;
        self.native_ack = header.message_ack;
        if reply.is_some() {
            self.native_sent += 1;
        }
        Ok(reply)
    }

    fn queue_input(&mut self, input: codol_protocol::game_usercmd::UserCommand) {
        if input.time < 1000
            || input.time > self.last_snapshot_time.max(1000).saturating_add(250)
            || self
                .last_input
                .as_ref()
                .is_some_and(|old| input.time <= old.time)
            || self.pending_inputs.len() >= 256
        {
            return;
        }
        self.pending_inputs.entry(input.time).or_insert(input);
    }
    fn queue_obituary(&mut self, now: u32, weapon: u16, victim: u8, attacker: u8, headshot: bool) {
        let entity_number = if self.next_obituary < 1800 {
            1800
        } else {
            self.next_obituary
        };
        self.next_obituary = if entity_number == 2045 {
            1800
        } else {
            entity_number + 1
        };
        self.obituaries.push_back((
            now,
            codol_protocol::game_obituary::ObituarySnapshot {
                entity_number,
                weapon,
                headshot,
                victim,
                attacker,
            },
        ));
        if headshot && attacker < 2 {
            self.headshots[usize::from(attacker)] += 1;
        }
        self.scoreboard_sent = false;
        tracing::info!(
            victim,
            attacker,
            weapon,
            headshot,
            "training kill obituary queued"
        );
    }

    fn advance_bot(&mut self, now: u32) -> Result<()> {
        if self.bot.is_none()
            || self
                .snapshot_first_sequence
                .is_none_or(|first| self.native_ack < first)
        {
            return Ok(());
        }
        let respawning = self
            .player_death
            .is_some_and(|death| now.wrapping_sub(death) >= 3000);
        if respawning {
            self.equip_spawn_loadout();
        }
        let state = self.game_state.as_ref().unwrap();
        if respawning {
            self.player_death = None;
            self.hit_feedback.clear();
            self.player_birth = now;
            self.health_recovery = Default::default();
            let spawn_count = self.vitals.spawn_count.wrapping_add(1);
            self.vitals = codol_protocol::game_snapshot::PlayerVitals {
                spawn_count,
                ..Default::default()
            };
            self.motion = state.motion();
            if let Some(motion) = &mut self.motion {
                motion.command_time = now;
            }
            if let (Some(player), Some(primary)) = (&mut self.player, state.primary_player()) {
                player.replace_loadout(primary, state.secondary_player());
            }
            self.feedback.push_back("Training player respawned".into());
            tracing::info!(spawn_count,origin=?self.motion.as_ref().map(|m| m.origin),"training player respawned");
        }
        let recovered = self.health_recovery.recover(now, self.vitals.health);
        if recovered != self.vitals.health {
            self.vitals.health = recovered;
            if recovered == 100 {
                self.vitals.damage_count = 0;
            }
            tracing::info!(health = recovered, "training player recovered health");
        }
        let (Some(weapon), Some(motion), Some(collision)) = (
            state.bot_weapon(),
            self.motion.as_ref(),
            state.collision.as_ref(),
        ) else {
            return Ok(());
        };
        let bot = self.bot.as_mut().unwrap();
        if self
            .combat
            .as_ref()
            .is_some_and(|world| world.health() == 0)
        {
            bot.kill(now);
        }
        let frame = bot.step(
            now,
            Some(crate::game_bot::BotTarget {
                origin: motion.origin,
                eye: [
                    motion.origin[0],
                    motion.origin[1],
                    motion.origin[2] + motion.view_height,
                ],
                // A short training spawn grace gives the player time to reorient.
                alive: self.vitals.health > 0 && now.wrapping_sub(self.player_birth) >= 2500,
            }),
            weapon,
            collision,
        );
        if self
            .bot_frame
            .as_ref()
            .is_none_or(|previous| previous.phase != frame.phase)
        {
            tracing::info!(phase=?frame.phase,origin=?frame.motion.origin,"training bot behavior changed");
        }
        if let Some(combat) = &mut self.combat {
            combat.move_target(frame.motion.origin);
            combat.turn_target(f32::from(frame.angles[1]) * 360.0 / 65536.0);
        }
        if let Some(shot) = frame.shot
            && bot.shot_is_current(&shot)
        {
            tracing::info!(
                shot_time = shot.time,
                weapon = shot.weapon,
                "training bot fired"
            );
            // Native eventSeq0 is a reset; pad a crossing burst so meaningful
            // fire/brass is published with a nonzero final sequence.
            if self.enemy_events.sequence == 254 {
                self.enemy_events.push(0, 0)?;
            }
            self.enemy_events
                .push(codol_protocol::game_events::FIRE_WEAPON, 0)?;
            self.enemy_events
                .push(codol_protocol::game_events::EJECT_BRASS, 0)?;
            self.bot_fire_time = Some(now);
            self.bot_fire_toggle = !self.bot_fire_toggle;
            if self.vitals.health > 0
                && let Some(damage) = crate::game_combat::hits_player(
                    shot.eye,
                    shot.angles,
                    shot.ballistics,
                    motion.origin,
                    motion.stance.height(),
                    collision,
                )
            {
                self.vitals.health = self.vitals.health.saturating_sub(damage);
                self.health_recovery.damaged(now, self.vitals.health);
                self.vitals.damage_event = self.vitals.damage_event.wrapping_add(1);
                self.vitals.damage_count = damage.min(127) as u8;
                tracing::info!(
                    shot_time = now,
                    health = self.vitals.health,
                    "training bot hit player"
                );
                if self.vitals.health == 0 {
                    self.player_death = Some(now);
                    self.hit_feedback.clear();
                    if let Some(mut local) = state.local_player_snapshot(true) {
                        local.origin = motion.origin;
                        local.angles = self
                            .last_input
                            .as_ref()
                            .map_or([0; 3], |input| input.angles);
                        if let Some(player) = self.player.as_ref() {
                            local.weapon = player.equipment().weapon;
                        }
                        self.death_sequence[0] = self.death_sequence[0].wrapping_add(1).max(1);
                        self.corpses
                            .insert(20, (now, state.corpse(local, self.death_sequence[0])));
                        self.queue_obituary(now, shot.weapon, 0, 1, false);
                    }
                }
            }
        }
        self.bot_frame = Some(frame);
        Ok(())
    }

    fn drain_match_kills(&mut self, now: u32) {
        let kills = self.tdm.as_mut().unwrap().take_kills();
        for kill in kills {
            self.last_kill = Some(kill);
            if kill.victim.client_index == 0
                && kill.attacker != 0
                && !self.tdm.as_ref().unwrap().finished
            {
                self.replay.start(
                    crate::game_replay::Kind::Death,
                    kill.time,
                    now,
                    kill.attacker,
                );
            }
            self.death_waypoints
                .record(now, self.tdm.as_ref().unwrap().teams()[0], kill.victim);
            let mut corpse = self
                .game_state
                .as_ref()
                .unwrap()
                .corpse(kill.victim, kill.sequence);
            corpse.entity_number = if kill.victim.client_index == 0 {
                20
            } else {
                let slot = self.next_corpse;
                self.next_corpse = if slot == 26 {
                    19
                } else if slot == 19 {
                    21
                } else {
                    slot + 1
                };
                slot
            };
            self.corpses.insert(corpse.entity_number, (now, corpse));
            self.queue_obituary(
                now,
                kill.weapon,
                kill.victim.client_index,
                kill.attacker,
                kill.headshot,
            );
        }
    }

    fn advance_match_summary(&mut self, now: u32) -> Result<()> {
        if self.match_summary_sequence.is_none() {
            return Ok(());
        }
        if self.replay.pending() {
            return Ok(());
        }
        self.vitals.intermission = true;
        if let Some(motion) = &mut self.motion {
            motion.velocity = [0.0; 3];
        }
        if let Some(menu) = self.game_state.as_ref().and_then(|s| s.top_players()) {
            if self.top_players.scene.is_none() {
                let state = self.game_state.as_ref().unwrap();
                let mut actors = self.tdm.as_ref().unwrap().snapshots();
                let mut local = state.local_player_snapshot(true).unwrap();
                local.weapon = self.player.as_ref().unwrap().equipment().weapon;
                actors.push(local);
                self.top_players.scene = Some(state.top_players_scene(
                    &self.tdm.as_ref().unwrap().scoreboard(self.owner_id).rows,
                    actors,
                )?);
                return Ok(());
            }
            // Wait until the client has applied a live snapshot in the new
            // epoch; opening the menu while it still displays killcam races
            // the native replay HUD. Final scoreboard data must also be ACKed.
            if self.server_commands.acknowledged() < self.match_summary_sequence.unwrap()
                || !self.world_history.iter().any(|h| {
                    h.sequence <= self.native_ack
                        && h.replay_epoch == self.replay.epoch
                        && h.top_players_scene
                })
                || !self.top_players.advance(
                    now,
                    menu,
                    &self.tdm.as_ref().unwrap().scoreboard(self.owner_id).rows,
                    &mut self.server_commands,
                )?
            {
                return Ok(());
            }
        }
        if self.match_ui_sequence.is_none()
            && self.server_commands.acknowledged() >= self.match_summary_sequence.unwrap()
        {
            self.server_commands
                .push(b"s scr_gameended \"1\" ui_match_complete \"1\" ui_eog_show \"1\"")?;
            self.match_ui_sequence = Some(self.server_commands.sequence());
            tracing::info!(
                winner = self.tdm.as_ref().unwrap().winning_team(),
                result = self.tdm.as_ref().unwrap().result(),
                "native match summary ready"
            );
        }
        if self.match_summary_opened.is_none()
            && self
                .match_ui_sequence
                .is_some_and(|sequence| self.server_commands.acknowledged() >= sequence)
        {
            self.match_summary_opened = Some(now);
        }
        if self
            .match_summary_opened
            .is_some_and(|start| now.saturating_sub(start) >= 12_000)
            && self.match_leave_sequence.is_none()
        {
            self.match_leave_sequence = Some(self.server_commands.push(b"t EXE_MATCHENDED")?);
            tracing::info!("returning finished match to lobby");
        }
        Ok(())
    }

    fn advance_match(&mut self, now: u32) -> Result<()> {
        let tick_started = Instant::now();
        if self.server_commands.acknowledged() < self.model_rules_sequence {
            return Ok(());
        }
        if self
            .snapshot_first_sequence
            .is_none_or(|first| self.native_ack < first)
        {
            return Ok(());
        }
        if self
            .player_death
            .is_some_and(|death| now.wrapping_sub(death) >= 3000)
            && !self.tdm.as_ref().unwrap().finished
            && !self.replay.pending()
        {
            self.equip_spawn_loadout();
            let state = self.game_state.as_ref().unwrap();
            let origin = self
                .tdm
                .as_ref()
                .unwrap()
                .player_spawn(state.collision.as_ref().unwrap())?;
            self.motion = state.motion();
            if let Some(motion) = &mut self.motion {
                motion.origin = origin;
                motion.command_time = now;
            }
            self.vitals = codol_protocol::game_snapshot::PlayerVitals {
                spawn_count: self.vitals.spawn_count.wrapping_add(1),
                ..Default::default()
            };
            self.player_death = None;
            self.hit_feedback.clear();
            self.player_birth = now;
            self.health_recovery = Default::default();
            if let Some(primary) = state.primary_player() {
                self.player
                    .as_mut()
                    .unwrap()
                    .replace_loadout(primary, state.secondary_player());
            }
        }
        let state = self.game_state.as_ref().unwrap().clone();
        let Some(motion) = &self.motion else {
            return Ok(());
        };
        let mut local = state.local_player_snapshot(self.vitals.health > 0).unwrap();
        local.origin = motion.origin;
        local.weapon = self.player.as_ref().unwrap().equipment().weapon;
        local.angles = self.view_angles();
        let active = self.vitals.health > 0 && now.wrapping_sub(self.player_birth) >= 1000;
        local.alive = active;
        let collision = state.collision.as_ref().unwrap();
        let tick = self.tdm.as_mut().unwrap().step(
            now,
            (
                local,
                motion.view_height + 10.0,
                if active { self.vitals.health } else { 0 },
            ),
            state.match_bot_weapon().unwrap(),
            collision,
            |snapshot, frame, firing, toggle| state.animate_enemy(snapshot, frame, firing, toggle),
        )?;
        for hit in tick.hits {
            if self.vitals.health == 0 {
                break;
            }
            self.vitals.health = self.vitals.health.saturating_sub(hit.damage);
            self.vitals.damage_event = self.vitals.damage_event.wrapping_add(1);
            self.vitals.damage_count = hit.damage.min(127) as u8;
            self.health_recovery.damaged(now, self.vitals.health);
            if self.vitals.health == 0 {
                self.player_death = Some(now);
                self.hit_feedback.clear();
                self.tdm.as_mut().unwrap().player_killed(now, local, hit);
                if let Some(motion) = &mut self.motion {
                    motion.velocity = [0.0; 3];
                }
            }
        }
        self.vitals.health = self.health_recovery.recover(now, self.vitals.health);
        if self.vitals.health == 100 {
            self.vitals.damage_count = 0;
        }
        for (weapon, impact) in tick.impacts {
            self.impacts.push(now, weapon, impact);
        }
        self.drain_match_kills(now);
        let elapsed = tick_started.elapsed();
        if elapsed > Duration::from_millis(25) {
            tracing::warn!(
                msec = elapsed.as_millis(),
                "TDM simulation exceeded frame budget"
            );
        }
        Ok(())
    }

    fn consume_inputs(&mut self, through: u32) {
        while self
            .pending_inputs
            .first_key_value()
            .is_some_and(|(&time, _)| time <= through)
        {
            let (_, mut input) = self.pending_inputs.pop_first().unwrap();
            if self
                .last_input
                .as_ref()
                .is_some_and(|last| input.time <= last.time)
            {
                continue;
            }
            let was_replaying = self.replay.kind().is_some();
            self.replay.input(input.time, input.buttons);
            if was_replaying && let Some(player) = &self.player {
                input.weapons = player.equipment().input_weapons();
            }
            if (self.bot.is_some() || self.tdm.is_some())
                && (self.vitals.health == 0
                    || input.time < self.player_birth
                    || self.tdm.as_ref().is_some_and(|m| m.finished))
            {
                input.buttons = 0;
                input.direction = 0;
            }
            if let Some(player) = &mut self.player {
                if let Some(motion) = &self.motion {
                    player.spread_environment(motion);
                }
                let equipment = player.equipment();
                let requested = input.weapons[0];
                if requested != 0
                    && !player.owns_weapon(requested)
                    && self
                        .last_input
                        .as_ref()
                        .is_none_or(|l| l.weapons[0] != requested)
                {
                    tracing::info!(
                        requested,
                        current = equipment.weapon,
                        "unowned weapon switch requested"
                    );
                }
                player.apply(&input);
            }
            if let Some(motion) = &mut self.motion
                && let Some(collision) = self.game_state.as_ref().and_then(|s| s.collision.as_ref())
            {
                if self.tdm.as_ref().is_some_and(|m| m.finished) {
                    motion.velocity = [0.0; 3];
                    motion.command_time = input.time;
                    self.last_input = Some(input);
                    continue;
                }
                let scale = self.player.as_ref().map_or(1.0, |p| p.move_scale());
                let ads = self
                    .player
                    .as_ref()
                    .and_then(|p| p.equipment().aim)
                    .is_some_and(|a| a.movement_slow);
                let old_bob = motion.bob_cycle;
                let was_ground = motion.ground_entity != 2047;
                motion.apply(&input, scale, ads, collision);
                if let Some(player) = &mut self.player {
                    let surface = (motion.surface_flags >> 20) & 31;
                    if was_ground
                        && motion.ground_entity == 2047
                        && motion.velocity[2] > 0.0
                        && input.buttons & 0x400 != 0
                    {
                        player.movement_event(codol_protocol::game_events::JUMP, surface);
                    }
                    if (old_bob ^ motion.bob_cycle) & 128 != 0 && motion.ground_entity != 2047 {
                        let event = match motion.stance {
                            crate::game_motion::Stance::Prone => {
                                codol_protocol::game_events::FOOTSTEP_PRONE
                            }
                            _ if input.buttons & 2 != 0 => {
                                codol_protocol::game_events::FOOTSTEP_SPRINT
                            }
                            _ if ads => codol_protocol::game_events::FOOTSTEP_WALK,
                            _ => codol_protocol::game_events::FOOTSTEP_RUN,
                        };
                        player.movement_event(event, surface);
                    }
                }
            }
            if let Some(player) = &mut self.player {
                let shots = player.take_shots();
                if let (Some(motion), Some(collision)) = (
                    &self.motion,
                    self.game_state
                        .as_ref()
                        .and_then(|s| s.collision.as_ref())
                        .cloned(),
                ) {
                    let eye = [
                        motion.origin[0],
                        motion.origin[1],
                        motion.origin[2] + motion.view_height,
                    ];
                    let origin = motion.origin;
                    for shot in shots {
                        if let Some(tdm) = &mut self.tdm {
                            let state = self.game_state.as_ref().unwrap();
                            let mut local =
                                state.local_player_snapshot(self.vitals.health > 0).unwrap();
                            local.origin = origin;
                            local.weapon = shot.weapon;
                            local.angles = shot.angles;
                            let outcome = tdm.player_fire(shot, eye, local, &collision, None);
                            if let Some(kind) = outcome.committed_feedback {
                                self.hit_feedback.record(through, kind);
                                tracing::info!(time = through, ?kind, "player damage feedback");
                            }
                            for impact in outcome.impacts {
                                self.impacts.push(through, shot.weapon, impact);
                            }
                            continue;
                        }
                        let outcome = self.combat.as_mut().map_or_else(
                            || crate::game_combat::ShotOutcome {
                                impacts: crate::game_combat::world_impacts(
                                    eye,
                                    shot.angles,
                                    shot.time,
                                    shot.ballistics,
                                    &collision,
                                ),
                                ..Default::default()
                            },
                            |combat| {
                                combat.fire(
                                    shot.time,
                                    eye,
                                    shot.angles,
                                    shot.ballistics,
                                    &collision,
                                )
                            },
                        );
                        for hit in &outcome.impacts {
                            if let Some(event) = self.impacts.push(through, shot.weapon, *hit) {
                                tracing::info!(shot_time = shot.time, entity = event.entity_number, origin = ?event.origin, normal = event.normal_byte, surface = event.surface, weapon = event.weapon, "world bullet impact");
                            }
                        }
                        let headshot = outcome.headshot();
                        let location = outcome.location.map(|l| l.name());
                        if let Some(message) = outcome.feedback {
                            let kind = if self
                                .combat
                                .as_ref()
                                .is_some_and(|world| world.health() == 0)
                            {
                                codol_protocol::game_hit_feedback::HitKind::Kill
                            } else {
                                codol_protocol::game_hit_feedback::HitKind::Hit
                            };
                            self.hit_feedback.record(through, kind);
                            tracing::info!(time = through, ?kind, "player damage feedback");
                            if self
                                .combat
                                .as_ref()
                                .is_some_and(|world| world.health() == 0)
                                && let Some(bot) = &mut self.bot
                            {
                                bot.kill(through);
                                if let Some(state) = self.game_state.as_ref()
                                    && let Some(mut enemy) = state.enemy_snapshot(true)
                                {
                                    if let Some(frame) = self.bot_frame.as_ref() {
                                        enemy.origin = frame.motion.origin;
                                        enemy.angles = frame.angles;
                                    }
                                    self.death_sequence[1] =
                                        self.death_sequence[1].wrapping_add(1).max(1);
                                    self.corpses.insert(
                                        19,
                                        (through, state.corpse(enemy, self.death_sequence[1])),
                                    );
                                    self.queue_obituary(through, shot.weapon, 1, 0, headshot);
                                }
                            }
                            tracing::info!(
                                shot_time = shot.time,
                                health = self.combat.as_ref().map(|combat| combat.health()),
                                location,
                                "training target hit"
                            );
                            self.feedback.push_back(message);
                        }
                    }
                }
            }
            self.input_count += 1;
            self.last_input = Some(input);
        }
    }
    pub fn handle(&mut self, payload: &[u8], owner: u64, build: i32) -> Result<Option<Vec<u8>>> {
        let Some(command) = payload.strip_prefix(&[255; 4]) else {
            return self.native(payload);
        };
        if let Some(update) = command.strip_prefix(b"gamestateupdate ") {
            ensure!(self.gamestate_sent, "gamestate update before send");
            let value = update.strip_suffix(&[0]).unwrap_or(update);
            ensure!(
                matches!(value, b"1" | b"2" | b"3" | b"4" | b"5" | b"6"),
                "invalid gamestate update"
            );
            self.gamestate_update = Some(self.gamestate_update.unwrap_or(0).max(value[0] - b'0'));
            return Ok(None);
        }
        if let Some(chunk) = command.strip_prefix(b"stats\0") {
            ensure!(self.connected, "stats before connect");
            let missing = self.stats.accept(chunk)?;
            let mut reply = vec![255; 4];
            reply.extend_from_slice(format!("statResponse {missing}\0").as_bytes());
            return Ok(Some(reply));
        }
        if command.strip_suffix(&[0]).unwrap_or(command) == b"heartbeat" {
            ensure!(
                self.stats.profile().is_some(),
                "heartbeat before stats completion"
            );
            // Target 0x466c30 accepts this only in state 6 and advances when
            // the local map-loading worker has completed (0x4640a0).
            return Ok(Some(b"\xff\xff\xff\xffheartbeatresponse\0".to_vec()));
        }
        if let Some(connect) = command.strip_prefix(b"connect ") {
            let challenge = self
                .challenge
                .ok_or_else(|| anyhow::anyhow!("connect before challenge"))?;
            self.display_name = Some(validate_connect(connect, owner, build, challenge)?);
            self.owner_id = owner;
            self.connected = true;
            // 0x466b24..0x466c22 initializes netchan, enters state 5 and starts
            // uploading stats. This target does not consume an info-string here.
            return Ok(Some(b"\xff\xff\xff\xffconnectResponse\0".to_vec()));
        }
        if !command.starts_with(b"getchallenge ") {
            return Ok(None);
        }
        ensure!(command.len() <= 128, "game challenge request too long");
        let command = command.strip_suffix(&[0]).unwrap_or(command);
        let command = std::str::from_utf8(command)?;
        let fields = command.split(' ').collect::<Vec<_>>();
        ensure!(
            fields.len() == 3 && fields[1] == "0",
            "unsupported challenge request"
        );
        let uid = fields[2]
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(|| anyhow::anyhow!("missing challenge identity"))?;
        ensure!(
            uid.len() == 16 && uid.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid challenge identity"
        );
        ensure!(
            u64::from_str_radix(uid, 16)? == owner,
            "challenge identity differs from admission"
        );
        let challenge = if let Some(value) = self.challenge {
            value
        } else {
            let mut random = [0; 4];
            openssl::rand::rand_bytes(&mut random)?;
            let value = (u32::from_le_bytes(random) & i32::MAX as u32).max(1);
            self.challenge = Some(value);
            value
        };
        // Target 0x466a55 reads argv(1) as an integer into connection +0x138,
        // then changes state from challenging (3) to connecting (4).
        let mut reply = vec![255; 4];
        reply.extend_from_slice(format!("challengeResponse {challenge}\0").as_bytes());
        Ok(Some(reply))
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
mod tests {
    use super::*;
    #[test]
    fn a_stalled_ack_keeps_its_input_base_while_new_snapshot_history_stays_bounded() {
        let mut c = Connection {
            native_ack: 4,
            ..Default::default()
        };
        let base = codol_protocol::game_usercmd::UserCommand {
            weapons: [352, 0, 0],
            ..Default::default()
        };
        c.remember_input_base(4, base.clone());
        for sequence in 5..1000 {
            c.remember_input_base(sequence, Default::default());
        }
        assert_eq!(c.snapshot_input_bases.len(), 256);
        assert_eq!(c.input_base_for(4).unwrap(), base);
        c.native_ack = 999;
        for sequence in 1000..2000 {
            c.remember_input_base(sequence, Default::default());
        }
        assert_eq!(c.snapshot_input_bases.len(), 256);
        assert!(c.input_base_for(999).is_ok());
        assert!(c.input_base_for(4).is_err());
    }
    #[test]
    fn training_backpack_changes_wait_for_the_next_spawn() {
        let current = Arc::new(crate::game_state::synthetic());
        let next = Arc::new(crate::game_state::synthetic());
        let mut c = Connection::with_game_state(current.clone());
        c.bot = Some(crate::game_bot::BotController::new([0.0; 3], [0; 3], 190.0).unwrap());
        c.player = Some(crate::game_player::Player::new(
            codol_protocol::game_snapshot::Equipment {
                inventory_weapon: None,
                alternate_weapon: None,
                fire_count: 0,
                spread_scale: 0.0,
                needs_rechamber: false,
                weapon: 352,
                secondary: None,
                events: None,
                aim: None,
                viewmodel: None,
                firing: None,
                ammo: None,
            },
            None,
            None,
            None,
        ));
        c.pending_loadout = Some((1, next.clone()));
        c.apply_acknowledged_loadout(); // Resources not yet acknowledged.
        assert!(c.pending_loadout.is_some() && c.next_spawn_loadout.is_none());
        c.pending_loadout = Some((0, next.clone()));
        c.apply_acknowledged_loadout();
        assert!(c.pending_loadout.is_none());
        assert!(Arc::ptr_eq(c.game_state.as_ref().unwrap(), &current));
        assert_eq!(c.player.as_ref().unwrap().equipment().weapon, 352);
        assert_eq!(
            c.feedback.back().map(String::as_str),
            Some("Backpack will change on next spawn")
        );
        c.equip_spawn_loadout();
        assert!(Arc::ptr_eq(c.game_state.as_ref().unwrap(), &next));
        assert!(c.next_spawn_loadout.is_none());
        // Without a training life there is no spawn to wait for.
        let mut plain = Connection::with_game_state(current);
        plain.pending_loadout = Some((0, next.clone()));
        plain.apply_acknowledged_loadout();
        assert!(Arc::ptr_eq(plain.game_state.as_ref().unwrap(), &next));
    }
    #[test]
    fn headshot_kills_are_counted_for_their_attacker() {
        let mut c = Connection::default();
        c.queue_obituary(1000, 352, 1, 0, true);
        c.queue_obituary(1100, 352, 1, 0, false);
        c.queue_obituary(1200, 352, 0, 1, true);
        assert_eq!(c.headshots, [1, 1]);
        assert!(c.obituaries[0].1.headshot && !c.obituaries[1].1.headshot);
    }
    #[test]
    fn intermission_camera_uses_eye_height_without_moving_the_actor_or_accumulating_height() {
        let mut c = Connection {
            motion: Some(crate::game_motion::Motion::new([3076.0, -838.0, 4.0])),
            ..Default::default()
        };
        assert_eq!(c.snapshot_motion().unwrap().origin, [3076.0, -838.0, 4.0]);
        c.vitals.intermission = true;
        for _ in 0..20 {
            let view = c.snapshot_motion().unwrap();
            assert_eq!(view.origin, [3076.0, -838.0, 64.0]);
            assert_eq!(view.view_height, 0.0);
            assert_eq!(c.motion.as_ref().unwrap().origin, [3076.0, -838.0, 4.0]);
        }
        c.motion.as_mut().unwrap().view_height = 20.0;
        assert_eq!(c.snapshot_motion().unwrap().origin[2], 24.0);
    }
    #[test]
    fn native_encoded_nickname_is_decoded_without_accepting_controls() {
        assert_eq!(
            decode_connect_name("c3RlYW11c2VyQ2xpZW50"),
            "steamuserClient"
        );
        assert_eq!(decode_connect_name("Player"), "Player");
        assert_eq!(decode_connect_name("cXVpdAo="), "cXVpdAo=");
    }
    #[test]
    fn predicted_inputs_are_queued_until_the_clock_reaches_them_without_losing_transitions() {
        let mut c = Connection {
            last_snapshot_time: 1000,
            ..Default::default()
        };
        for (time, direction) in [(1010, 1), (1030, 0), (1020, 1), (4000, 1)] {
            c.queue_input(codol_protocol::game_usercmd::UserCommand {
                time,
                direction,
                ..Default::default()
            });
        }
        c.consume_inputs(1000);
        assert_eq!(c.input_count, 0);
        c.consume_inputs(1025);
        assert_eq!(c.input_count, 2);
        assert_eq!(c.last_input.as_ref().unwrap().direction, 1);
        c.consume_inputs(1030);
        assert_eq!(c.input_count, 3);
        assert_eq!(c.last_input.as_ref().unwrap().direction, 0);
        c.consume_inputs(5000);
        assert_eq!(c.input_count, 3);
    }
    #[test]
    fn missed_snapshot_ticks_do_not_slow_the_server_clock_or_acknowledge_unapplied_input() {
        let now = Instant::now();
        let mut c = Connection::with_game_state(Arc::new(
            crate::game_state::synthetic()
                .with_snapshot_frames(60)
                .unwrap(),
        ));
        c.challenge = Some(0x75);
        c.native_sent = 3;
        c.gamestate_update = Some(6);
        c.poll_snapshot(now).unwrap().unwrap();
        let mut packet = c
            .poll_snapshot(now + Duration::from_millis(275))
            .unwrap()
            .unwrap();
        game_wire::xor_stream(&mut packet[8..], 0x75 ^ 5, b"");
        assert_eq!(&packet[9..13], &1275u32.to_le_bytes());
        assert_eq!(&packet[17..21], &1000u32.to_le_bytes());
    }
    #[test]
    fn input_baseline_follows_acknowledged_snapshot_not_latest_view() {
        let mut c = Connection::default();
        let first = codol_protocol::game_usercmd::UserCommand {
            angles: [100, 200, 0],
            weapons: [352, 0, 0],
            ..Default::default()
        };
        let second = codol_protocol::game_usercmd::UserCommand {
            angles: [300, 400, 0],
            buttons: 0x800,
            ..first.clone()
        };
        c.snapshot_input_bases.push_back((4, first.clone()));
        c.snapshot_input_bases.push_back((6, second.clone()));
        assert!(c.input_base_for(3).is_err());
        assert_eq!(c.input_base_for(4).unwrap(), first);
        assert_eq!(c.input_base_for(5).unwrap(), first); // reliable-only message
        assert_eq!(c.input_base_for(6).unwrap(), second);
    }
    #[test]
    fn model_commands_remain_bounded_while_the_local_player_can_initialize() {
        let now = Instant::now();
        let mut c = Connection::with_game_state(Arc::new(
            crate::game_state::synthetic()
                .with_snapshot_frames(60)
                .unwrap(),
        ));
        c.challenge = Some(0x75);
        c.native_sent = 3;
        c.gamestate_update = Some(6);
        c.model_rules_sent = true;
        for id in 200..260 {
            c.server_commands
                .push(format!("d {} character_part_{id}_tf_m", 2051 + id).as_bytes())
                .unwrap();
        }
        c.model_rules_sequence = c.server_commands.sequence();
        let packet = c.poll_snapshot(now).unwrap().unwrap();
        assert!(packet.len() < game_wire::FRAGMENT_SIZE + 4);
        assert_eq!(c.snapshot_count, 1);
        assert_eq!(c.snapshot_first_sequence, Some(4));
        assert!(c.input_base_for(4).is_ok());
        assert!(c.server_commands.acknowledged() < c.model_rules_sequence);
    }
    #[test]
    fn snapshot_probe_requires_opt_in_initialization_and_has_rate_and_count_limits() {
        let now = Instant::now();
        let mut disabled = Connection::with_game_state(Arc::new(crate::game_state::synthetic()));
        disabled.gamestate_update = Some(6);
        assert!(disabled.poll_snapshot(now).unwrap().is_none());
        let mut c = Connection::with_game_state(Arc::new(
            crate::game_state::synthetic()
                .with_snapshot_frames(60)
                .unwrap(),
        ));
        c.challenge = Some(0x75);
        c.native_sent = 3;
        assert!(c.poll_snapshot(now).unwrap().is_none());
        c.gamestate_update = Some(5);
        assert!(c.poll_snapshot(now).unwrap().is_none());
        c.gamestate_update = Some(6);
        for i in 0..60u32 {
            if i == 1 {
                c.native_ack = 4;
            }
            let at = now + Duration::from_millis(u64::from(i) * 50);
            let mut packet = c.poll_snapshot(at).unwrap().unwrap();
            assert_eq!(&packet[..4], &(i + 4).to_le_bytes());
            game_wire::xor_stream(&mut packet[8..], 0x75 ^ (i + 4) as u8, b"");
            assert_eq!(packet[14], if i == 0 { 8 } else { 0 });
            assert_eq!(&packet[9..13], &(1000 + i * 50).to_le_bytes());
            // The frozen captured layout places PS.commandTime at body+9.
            // Sending more frames without input must not acknowledge input
            // that the client has not generated yet.
            assert_eq!(&packet[17..21], &1000u32.to_le_bytes());
            assert!(
                c.poll_snapshot(at + Duration::from_millis(49))
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            c.poll_snapshot(now + Duration::from_secs(60))
                .unwrap()
                .is_none()
        );
        assert_eq!(c.native_sent, 63);
        assert_eq!(c.native_received, 0); // sending must never invent client ACKs
    }
    #[test]
    fn acknowledged_match_exit_stops_snapshots_but_unacknowledged_exit_is_retried() {
        let now = Instant::now();
        let mut c = Connection::with_game_state(Arc::new(
            crate::game_state::synthetic()
                .with_snapshot_frames(60)
                .unwrap(),
        ));
        c.challenge = Some(0x75);
        c.native_sent = 3;
        c.gamestate_update = Some(6);
        let leave = c.server_commands.push(b"t EXE_MATCHENDED").unwrap();
        c.match_leave_sequence = Some(leave);
        assert!(c.poll_snapshot(now).unwrap().is_some());
        assert!(
            c.poll_snapshot(now + Duration::from_millis(50))
                .unwrap()
                .is_some()
        );
        c.server_commands.acknowledge(leave).unwrap();
        let count = c.snapshot_count;
        assert!(
            c.poll_snapshot(now + Duration::from_millis(100))
                .unwrap()
                .is_none()
        );
        assert_eq!(c.snapshot_count, count);
    }
    #[test]
    fn normal_play_keeps_the_frame_stream_past_the_diagnostic_frame_limit() {
        let mut c = Connection::with_game_state(Arc::new(
            crate::game_state::synthetic().with_continuous_play(),
        ));
        c.challenge = Some(0x75);
        c.native_sent = 3;
        c.gamestate_update = Some(6);
        c.snapshot_count = 18000;
        assert!(c.poll_snapshot(Instant::now()).unwrap().is_some());
        assert_eq!(c.snapshot_count, 18001);
    }
    #[test]
    fn stats_and_heartbeat_require_prior_stages_and_all_fragments() {
        let mut connection = Connection::default();
        let heartbeat = b"\xff\xff\xff\xffheartbeat\0";
        assert!(connection.handle(heartbeat, 42, 7).is_err());
        let raw = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
        for (index, data) in raw
            .chunks(codol_protocol::game_stats::CHUNK_BYTES)
            .enumerate()
        {
            let mut packet = b"\xff\xff\xff\xffstats\0".to_vec();
            packet.extend_from_slice(&[index as u8, 0, 0]);
            packet.extend_from_slice(data);
            if index == 0 {
                assert!(connection.handle(&packet, 42, 7).is_err());
                connection.connected = true;
            }
            let reply = connection.handle(&packet, 42, 7).unwrap().unwrap();
            if index < 26 {
                assert_ne!(reply, b"\xff\xff\xff\xffstatResponse 0\0");
                assert!(connection.handle(heartbeat, 42, 7).is_err());
            } else {
                assert_eq!(reply, b"\xff\xff\xff\xffstatResponse 0\0");
            }
        }
        assert_eq!(connection.stage(), "stats_received");
        assert_eq!(
            connection.handle(heartbeat, 42, 7).unwrap().unwrap(),
            b"\xff\xff\xff\xffheartbeatresponse\0"
        );
    }
    #[test]
    fn challenge_requires_admitted_identity_and_retries_reuse_nonce() {
        let mut connection = Connection::default();
        let request = b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\"\0";
        assert!(connection.handle(request, 1, 7).is_err());
        assert!(connection.challenge.is_none());
        let response = connection
            .handle(request, u64::MAX - 1, 7)
            .unwrap()
            .unwrap();
        assert!(response.starts_with(b"\xff\xff\xff\xffchallengeResponse "));
        assert_eq!(
            connection
                .handle(request, u64::MAX - 1, 7)
                .unwrap()
                .unwrap(),
            response
        );
        assert!(
            connection
                .handle(
                    b"\xff\xff\xff\xffgetchallenge 0 \"fffffffffffffffe\" trailing",
                    u64::MAX - 1,
                    7
                )
                .is_err()
        );
        assert!(
            connection
                .handle(&[0, 0], u64::MAX - 1, 7)
                .unwrap()
                .is_none()
        );
        assert!(
            connection
                .handle(b"\xff\xff\xff\xffarbitrary", u64::MAX - 1, 7)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn connect_binds_both_identities_build_and_nonce_without_duplicate_fields() {
        let mut connection = Connection::default();
        let valid = b"\xff\xff\xff\xffconnect fffffffffffffffe \"\\name\\Lab player\\xuid\\fffffffffffffffe\\challenge\\12345\\2490\\-688906341\"";
        assert!(connection.handle(valid, u64::MAX - 1, -688906341).is_err());
        connection.challenge = Some(12345);
        for (old, new) in [
            ("fffffffffffffffe", "ffffffffffffffff"),
            ("12345", "12346"),
            ("-688906341", "0"),
            ("\\name\\", "\\xuid\\"),
        ] {
            let text = std::str::from_utf8(&valid[4..]).unwrap().replace(old, new);
            let mut damaged = vec![255; 4];
            damaged.extend_from_slice(text.as_bytes());
            assert!(
                connection
                    .handle(&damaged, u64::MAX - 1, -688906341)
                    .is_err()
            );
            assert!(!connection.connected);
        }
        for _ in 0..2 {
            assert_eq!(
                connection
                    .handle(valid, u64::MAX - 1, -688906341)
                    .unwrap()
                    .unwrap(),
                b"\xff\xff\xff\xffconnectResponse\0"
            );
        }
        assert_eq!(connection.stage(), "connected");
        let mut extra = valid.to_vec();
        extra.extend_from_slice(b" garbage");
        assert!(connection.handle(&extra, u64::MAX - 1, -688906341).is_err());
    }

    fn eof(sequence: u32, message_ack: u32, reliable_ack: u32) -> Vec<u8> {
        let mut wire = sequence.to_le_bytes().to_vec();
        wire.push(0);
        wire.extend_from_slice(&message_ack.to_le_bytes());
        wire.extend_from_slice(&reliable_ack.to_le_bytes());
        wire.push(3 ^ 0x75 ^ message_ack as u8);
        wire
    }

    #[test]
    fn probes_keep_client_ack_zero_terminate_stream_and_require_valid_progress() {
        let mut connection = Connection {
            challenge: Some(0x75),
            connected: true,
            ..Default::default()
        };
        assert!(connection.handle(&eof(1, 0, 0), 1, 7).is_err());
        let profile = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
        for (i, chunk) in profile
            .chunks(codol_protocol::game_stats::CHUNK_BYTES)
            .enumerate()
        {
            let mut part = vec![i as u8, 0, 0];
            part.extend_from_slice(chunk);
            connection.stats.accept(&part).unwrap();
        }
        assert!(connection.handle(&eof(1, 1, 0), 1, 7).is_err());
        let first = connection.handle(&eof(1, 0, 0), 1, 7).unwrap().unwrap();
        assert_eq!(&first[..8], &[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(first[8], 6 ^ 0x75 ^ 1);
        assert!(connection.handle(&eof(1, 0, 0), 1, 7).is_err());
        assert!(connection.handle(&eof(2, 1, 1), 1, 7).is_err());
        assert_eq!(connection.native_received, 1); // rejection must not consume sequence
        let mut second = connection.handle(&eof(2, 1, 0), 1, 7).unwrap().unwrap();
        assert_eq!(&second[..8], &[2, 0, 0, 0, 0, 0, 0, 0]);
        game_wire::xor_stream(&mut second[8..], 0x75 ^ 2, b"");
        assert_eq!(&second[8..], &[1, 1, 0, 0, 0, 0, 6]);
        assert!(connection.handle(&eof(3, 3, 1), 1, 7).is_err());
        assert!(connection.handle(&eof(3, 2, 1), 1, 7).unwrap().is_none());
        assert_eq!(connection.stage(), "reliable_probe_acknowledged");
        assert!(connection.handle(&eof(4, 1, 0), 1, 7).is_err());
    }

    #[test]
    fn gamestate_requires_probe_ack_and_client_commands_commit_atomically() {
        let mut connection = Connection::with_game_state(Arc::new(crate::game_state::synthetic()));
        connection.challenge = Some(0x75);
        connection.connected = true;
        let profile = vec![0; codol_protocol::profile::SNAPSHOT_BYTES];
        for (i, chunk) in profile
            .chunks(codol_protocol::game_stats::CHUNK_BYTES)
            .enumerate()
        {
            let mut part = vec![i as u8, 0, 0];
            part.extend_from_slice(chunk);
            connection.stats.accept(&part).unwrap();
        }
        assert!(
            connection
                .handle(b"\xff\xff\xff\xffgamestateupdate 1\0", 1, 7)
                .is_err()
        );
        connection.handle(&eof(1, 0, 0), 1, 7).unwrap();
        connection.handle(&eof(2, 1, 0), 1, 7).unwrap();
        assert!(connection.handle(&eof(3, 2, 0), 1, 7).unwrap().is_none());
        let mut packet = connection.handle(&eof(4, 2, 1), 1, 7).unwrap().unwrap();
        assert_eq!(packet[..8], [3, 0, 0, 0, 0, 0, 0, 0]);
        game_wire::xor_stream(&mut packet[8..], 0x75 ^ 3, b"");
        assert_eq!(packet[8], 3);
        assert_eq!(packet.last(), Some(&6));
        assert_eq!(connection.stage(), "gamestate_sent");
        connection
            .handle(b"\xff\xff\xff\xffgamestateupdate 1\0", 1, 7)
            .unwrap();
        assert_eq!(connection.stage(), "gamestate_parser_entered");
        connection
            .handle(b"\xff\xff\xff\xffgamestateupdate 6\0", 1, 7)
            .unwrap();
        connection
            .handle(b"\xff\xff\xff\xffgamestateupdate 3\0", 1, 7)
            .unwrap();
        assert_eq!(connection.stage(), "gamestate_initialized");
        assert_eq!(connection.gamestate_progress(), Some(6));
        assert!(
            connection
                .handle(b"\xff\xff\xff\xffgamestateupdate 7\0", 1, 7)
                .is_err()
        );
        // One reliable command, then EOF in the same pool byte (2 | 3<<3).
        let mut wire = eof(5, 3, 1);
        wire[4] = 1;
        wire.truncate(13);
        let mut body = vec![0x1a, 1, 0, 0, 0, b'a', 0];
        game_wire::xor_stream(&mut body, 0x75 ^ 3 ^ 1, b"");
        wire.extend_from_slice(&body);
        let mut broken = wire.clone();
        broken.pop();
        assert!(connection.handle(&broken, 1, 7).is_err());
        assert_eq!(connection.client_commands.sequence(), 0);
        let mut response = connection.handle(&wire, 1, 7).unwrap().unwrap();
        assert_eq!(response[..8], [4, 0, 0, 0, 1, 0, 0, 0]);
        game_wire::xor_stream(&mut response[8..], 0x75 ^ 4, b"a");
        assert_eq!(response[8..], [6]);
        assert_eq!(connection.stage(), "gamestate_acknowledged");
        wire[..4].copy_from_slice(&6u32.to_le_bytes());
        assert!(connection.handle(&wire, 1, 7).unwrap().is_none());
        assert_eq!(connection.client_commands.sequence(), 1);
    }
}
