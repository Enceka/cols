use super::*;

impl Connection {
    /// An explicitly bounded stream at 20 Hz. The probe has no physics/input
    /// execution and is disabled unless explicitly selected by the operator.
    pub fn poll_snapshot(&mut self, now: Instant) -> Result<Option<Vec<u8>>> {
        if let Some(fragment) = self.native_fragments.pop_front() {
            return Ok(Some(fragment));
        }
        if self.gamestate_update != Some(6)
            || self
                .match_leave_sequence
                .is_some_and(|sequence| self.server_commands.acknowledged() >= sequence)
            || self.snapshot_count
                >= self
                    .world
                    .game_state
                    .as_ref()
                    .map_or(0, |s| s.snapshot_limit())
            || !self
                .world
                .game_state
                .as_ref()
                .is_some_and(|s| s.snapshot_probe())
            || self.snapshot_next.is_some_and(|next| now < next)
        {
            return Ok(None);
        }
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
            && let Some(tdm) = &mut self.world.tdm
        {
            tdm.advance_clock(server_time);
        }
        self.apply_acknowledged_loadout();
        self.consume_inputs(server_time);
        self.advance_radios(server_time)?;
        self.advance_nuke(server_time)?;
        if !self.hud_rules_sent
            && self.world.player.is_some()
            && self
                .snapshot_first_sequence
                .is_some_and(|first| self.native_ack >= first)
        {
            let limit = self
                .world
                .tdm
                .as_ref()
                .map_or(7500, |m| m.scoreboard(self.owner_id).score_limit);
            self.server_commands
                .push(format!("s ui_scorelimit \"{limit}\"").as_bytes())?;
            // Keep gamestate's3059=1: native0x609bf0 shows the static skill
            // icons for zero gain timestamps. Nonzero timestamps animate a
            // separate new-skill notification before the sidebar settles.
            if self.world.tdm.is_some() {
                self.server_commands
                    .push(b"s scr_gameended \"0\" ui_match_complete \"0\" ui_eog_show \"0\" ui_hide_playercards \"0\"")?;
            }
            self.hud_rules_sent = true;
            self.hud_rules_sequence = self.server_commands.sequence();
        }
        if self.world.combat.is_some()
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
        if let Some(combat) = &mut self.world.combat
            && let Some(message) = combat.advance(server_time)
        {
            self.feedback.push_back(message);
            if let Some(bot) = &mut self.world.bot {
                let spawn = self
                    .world
                    .game_state
                    .as_ref()
                    .unwrap()
                    .enemy_snapshot(true)
                    .unwrap();
                let origin = crate::game::ai::controller::grounded_spawn(
                    spawn.origin,
                    self.world
                        .game_state
                        .as_ref()
                        .unwrap()
                        .collision
                        .as_ref()
                        .unwrap(),
                )?;
                bot.respawn(server_time, origin, spawn.angles)?;
                self.enemy_events = Default::default();
                self.world.bot_fire_time = None;
            }
        }
        if self.world.tdm.is_some() {
            self.advance_match(server_time)?;
            self.advance_grenades(server_time)?;
            self.advance_props(server_time)?;
        } else {
            self.advance_bot(server_time)?;
        }
        if self.world.vitals.health == 0 || self.world.tdm.as_ref().is_some_and(|m| m.finished()) {
            self.world.tactical.clear();
            self.world.poison.clear();
            self.world.emp.clear();
            if let Some(motion) = &mut self.world.motion {
                motion.cancel_sprint();
                motion.flags &= !0x8000;
            }
        }
        if self.world.tdm.as_ref().is_some_and(|m| m.finished()) && !self.final_replay_considered {
            self.final_replay_considered = true;
            if let Some(kill) = self.last_kill {
                self.replay.start(
                    crate::game::presentation::replay::Kind::Final,
                    kill.time,
                    server_time,
                    kill.attacker,
                );
            }
        }
        self.publish_match_scoreboard()?;
        self.advance_match_summary(server_time)?;
        self.advance_voices(server_time)?;
        if self.world.tdm.is_none()
            && !self.scoreboard_sent
            && self
                .world
                .game_state
                .as_ref()
                .is_some_and(|state| state.enemy_snapshot(true).is_some())
            && self
                .snapshot_first_sequence
                .is_some_and(|first| self.native_ack >= first)
        {
            let rows = vec![
                codol_protocol::game::snapshot::scoreboard::ScoreboardRow {
                    client_number: 0,
                    user_id: self.owner_id,
                    kills: self.death_sequence[1],
                    deaths: self.death_sequence[0],
                    headshots: self.headshots[0],
                    score: (self.death_sequence[1] * 100) as i32,
                    ..Default::default()
                },
                codol_protocol::game::snapshot::scoreboard::ScoreboardRow {
                    client_number: 1,
                    user_id: 1,
                    kills: self.death_sequence[0],
                    deaths: self.death_sequence[1],
                    headshots: self.headshots[1],
                    score: (self.death_sequence[0] * 100) as i32,
                    ..Default::default()
                },
            ];
            let board = codol_protocol::game::snapshot::scoreboard::ScoreboardSnapshot {
                rows,
                team_scores: [
                    0,
                    (self.death_sequence[0] * 100) as i32,
                    (self.death_sequence[1] * 100) as i32,
                    0,
                ],
                score_limit: 7500,
            };
            for payload in codol_protocol::game::snapshot::scoreboard::binary_commands(&board)? {
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
            .world
            .game_state
            .as_ref()
            .and_then(|s| s.feedback_resources())
        {
            for _ in 0..sounds {
                self.server_commands
                    .push(resources.sound_command().as_bytes())?;
            }
        }
        let mut equipment = self
            .world
            .player
            .as_ref()
            .map(|p| self.hud_equipment(p.equipment()));
        let perk_display = self.perk_display(equipment);
        let state = self.world.game_state.as_ref().expect("enabled state");
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
        let mut impacts = self.impacts.snapshot(server_time);
        let sounds = self.voices.snapshots();
        let mut missiles = self.world.grenades.snapshots();
        if let Some(tdm) = &self.world.tdm {
            missiles.extend(tdm.sam_missiles.snapshots());
            missiles.sort_by_key(|m| m.entity_number);
        }
        let mut smokes = self.world.smoke.snapshots(server_time);
        let mut bags = self.world.scavenger.snapshots();
        let mut shellshock = self.world.tactical.snapshot(server_time);
        if self.world.tdm.is_some() {
            impacts.sort_by_key(|i| {
                if i.event != codol_protocol::game::snapshot::impact::ImpactEvent::Bullet {
                    0
                } else {
                    1
                }
            });
            impacts.truncate(8);
            impacts.sort_by_key(|i| i.entity_number);
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
                    codol_protocol::game::snapshot::corpse::CorpsePhase::Playing
                } else {
                    codol_protocol::game::snapshot::corpse::CorpsePhase::Frozen
                };
                current.present = age < 6000;
                current
            })
            .collect();
        if self.local_baseline.is_none() {
            self.local_baseline = state.local_player_snapshot(true);
        }
        let mut local_player = state.local_player_snapshot(self.world.vitals.health > 0);
        if let (Some(local), Some(motion)) = (&mut local_player, self.world.motion.as_ref()) {
            local.origin = motion.origin;
            local.angles = angles;
            if let Some(player) = self.world.player.as_ref() {
                local.weapon = player.equipment().weapon;
            }
        }
        if self.enemy_baseline.is_none() {
            self.enemy_baseline = state.enemy_snapshot(true);
        }
        let mut enemy = state.enemy_snapshot(
            self.world
                .combat
                .as_ref()
                .is_some_and(|world| world.health() > 0),
        );
        if let (Some(current), Some(frame)) = (enemy, self.world.bot_frame.as_ref()) {
            enemy = Some(
                state.animate_enemy(
                    current,
                    frame,
                    self.world
                        .bot_fire_time
                        .is_some_and(|time| server_time.wrapping_sub(time) < 150),
                    self.world.bot_fire_toggle,
                ),
            );
        }
        if let Some(current) = &mut enemy {
            current.events = self.enemy_events;
        }
        let models_ready = self.server_commands.acknowledged() >= self.model_rules_sequence;
        if self.world.tdm.is_some() && !models_ready {
            // The selected local outfit now needs the same registration/ACK
            // gate as remote actors. PS input can start without its DObj.
            local_player = None;
        }
        let mut match_players = if models_ready {
            self.world
                .tdm
                .as_ref()
                .map_or(Vec::new(), |m| m.snapshots())
        } else {
            Vec::new()
        };
        if self.world.tdm.is_some() {
            enemy = None;
        }
        let mut snapshot_motion = self.snapshot_motion();
        let mut props = self
            .world
            .tdm
            .as_ref()
            .map_or(Vec::new(), |m| m.world_models());
        let mut view_offset = self.view_offset;
        let mut streaks = self.world.tdm.as_ref().map_or(Default::default(), |m| {
            if m.finished() {
                Default::default()
            } else {
                m.streaks
                    .snapshot(0, state.nuke_resources().map_or(0, |r| r.weapon))
            }
        });
        if self.world.tdm.as_ref().is_some_and(|m| !m.finished()) {
            state.streak_previews().apply(&mut streaks);
        }
        let mut radar_flags = state.radar_resources().and_then(|_| {
            self.world.tdm.as_ref().map(|m| {
                if m.finished() {
                    0
                } else {
                    let flags = m.streaks.radar_flags(m.teams()[0], server_time);
                    if self
                        .world
                        .player
                        .as_ref()
                        .is_some_and(|p| p.counter_uav_protected())
                    {
                        flags & !2
                    } else {
                        flags
                    }
                }
            })
        });
        if let (Some(m), Some(resources)) = (&self.world.tdm, state.radar_resources())
            && !m.finished()
        {
            m.streaks.radio_snapshot(0, resources, &mut streaks);
        }
        let mut reward_inventory = self
            .world
            .player
            .as_ref()
            .map_or([None; 4], |p| p.reward_inventory());
        self.world.vitals.emp_jammed = self.world.emp.active(server_time)
            || self.world.tdm.as_ref().is_some_and(|m| {
                m.streaks.emp_active(m.teams()[0], server_time)
                    && self
                        .world
                        .player
                        .as_ref()
                        .is_some_and(|p| p.emp_protection() > 0.0)
            });
        let mut snapshot_vitals =
            (self.world.bot.is_some() || self.world.tdm.is_some()).then_some(self.world.vitals);
        let mut hit_feedback = state
            .feedback_resources()
            .map(|r| self.hit_feedback.snapshot(server_time, r));
        if let Some(tdm) = self.world.tdm.as_ref().filter(|_| models_ready) {
            let mut views = tdm.replay_views(server_time);
            if let (Some(motion), Some(equipment)) = (self.world.motion.as_ref(), equipment) {
                views.push(crate::game::presentation::replay::View {
                    streaks,
                    radar_flags,
                    reward_inventory,
                    client: 0,
                    shellshock,
                    motion: motion.clone(),
                    angles,
                    equipment,
                    vitals: self.world.vitals,
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
                    view.motion.cancel_melee_charge();
                    view.motion.velocity = [0.0; 3];
                }
            }
            self.replay
                .record(crate::game::presentation::replay::Frame {
                    props: props.clone(),
                    bags: bags.clone(),
                    smokes: smokes.clone(),
                    missiles: missiles.clone(),
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
            view_offset = [0; 3];
            angles = scene.angles;
            command_time = server_time;
            let mut camera = scene.camera.clone();
            camera.command_time = server_time;
            snapshot_motion = Some(camera);
            snapshot_vitals = Some(codol_protocol::game::snapshot::PlayerVitals {
                intermission: true,
                ..Default::default()
            });
            local_player = scene.actors.first().copied();
            match_players = scene.actors[1..].to_vec();
            corpses = scene.poses.clone();
            impacts.clear();
            obituaries.clear();
            bags.clear();
            hit_feedback = None;
        }
        let playback = self.replay.sample(server_time);
        let replay_epoch = playback.as_ref().map_or(self.replay.epoch, |p| p.epoch);
        let replay_wire = playback
            .as_ref()
            .map(|p| codol_protocol::game::snapshot::ReplayView {
                client: p.view.client,
                delay_msec: p.delay,
                end_time: p.end_time,
                skip_text: state.replay_resources().and_then(|r| r.skip_text(p.kind)),
            });
        if let Some(playback) = playback {
            if playback.entering {
                if playback.kind == crate::game::presentation::replay::Kind::Final {
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
            shellshock = playback.view.shellshock;
            angles = playback.view.angles;
            props = playback.frame.props.clone();
            view_offset = [0; 3];
            command_time = playback.view.motion.command_time;
            equipment = Some(playback.view.equipment);
            streaks = playback.view.streaks;
            radar_flags = playback.view.radar_flags;
            reward_inventory = playback.view.reward_inventory;
            snapshot_motion = Some(playback.view.motion);
            snapshot_vitals = Some(playback.view.vitals);
            local_player = playback.frame.local;
            match_players = playback.frame.players;
            corpses = playback.frame.corpses;
            impacts = playback.frame.impacts;
            missiles = playback.frame.missiles;
            smokes = playback.frame.smokes;
            bags = playback.frame.bags;
            obituaries = playback.frame.obituaries;
            hit_feedback = state.feedback_resources().map(|resources| {
                let mut feedback = crate::game::presentation::feedback::Feedback::default();
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
        let match_baselines = self.world.tdm.as_ref().map_or(&[][..], |m| m.baselines());
        // parseClients is circular; a multiplayer delta cannot retain the
        // original full frame's appearance-record indices indefinitely.
        let previous = if self.world.tdm.is_some() {
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
        let full = if self.world.tdm.is_some() {
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
                let team = self.world.tdm.as_ref().map_or(0, |m| m.teams()[0]);
                self.death_waypoints.snapshot(
                    server_time,
                    team,
                    &live_players,
                    material,
                    self.world.vitals.health > 0
                        && !self.world.vitals.intermission
                        && replay_wire.is_none(),
                    self.world.tdm.as_ref().is_none_or(|m| m.finished()),
                )
            });
        let snapshot = state.snapshot(
            server_time,
            command_time,
            full,
            angles,
            equipment,
            crate::game::session::prepared::SnapshotWorld {
                props: &props,
                prop_baselines: previous.as_ref().map_or(&[], |h| h.props.as_slice()),
                view_offset,
                streaks,
                reward_inventory,
                radar_flags,
                nuke_hud: self.world.tdm.as_ref().and_then(|m| {
                    let end = m.streaks.countdown_end();
                    let text = self
                        .nuke_banner
                        .filter(|(until, _)| server_time < *until)
                        .map(|(_, text)| text);
                    (end.is_some() || text.is_some()).then_some(
                        codol_protocol::game::snapshot::hit_feedback::NukeHudSnapshot {
                            end_time: end,
                            text,
                        },
                    )
                }),
                preparation_hud: self
                    .world
                    .tdm
                    .as_ref()
                    .and_then(|m| m.preparation_hud(server_time)),
                perk_display: perk_display && replay_wire.is_none() && !top_players_scene,
                bags: &bags,
                bag_baselines: previous.as_ref().map_or(&[], |h| h.bags.as_slice()),
                shellshock,
                smokes: &smokes,
                smoke_baselines: previous.as_ref().map_or(&[], |h| h.smokes.as_slice()),
                missiles: &missiles,
                missile_baselines: previous.as_ref().map_or(&[], |h| h.missiles.as_slice()),
                local_abilities: self.world.player.as_ref().map(|p| p.equipment().abilities),
                local_abilities_baseline: previous.as_ref().and_then(|h| h.local_abilities),
                delta_distance,
                corpse_baselines: previous.as_ref().map_or(&[], |h| h.corpses.as_slice()),
                impact_baselines: previous.as_ref().map_or(&[], |h| h.impacts.as_slice()),
                obituary_baselines: previous.as_ref().map_or(&[], |h| h.obituaries.as_slice()),
                motion: snapshot_motion.as_ref(),
                target: if state.enemy_snapshot(true).is_none() {
                    self.world.combat.as_ref().map(|w| w.snapshot())
                } else {
                    None
                },
                impacts: &impacts,
                sounds: &sounds,
                sound_baselines: previous.as_ref().map_or(&[], |h| h.sounds.as_slice()),
                enemy: if self.world.tdm.is_some() {
                    None
                } else {
                    enemy
                },
                enemy_baseline: self.enemy_baseline,
                players: &match_players,
                player_baselines: previous_players,
                vitals: snapshot_vitals,
                local_player,
                local_player_baseline: if self.world.tdm.is_some() {
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
        let (mut body, mut sent) = self
            .server_commands
            .encode_frames_bounded(budget.saturating_sub(snapshot.len()))?;
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
        // The same native XOR/inflate decoder handles gamestates and live
        // snapshots. Crowded deaths/replay exits can exceed the raw budget;
        // retain the complete world and compress only when necessary.
        let mut packets = game_wire::compact_server_packets(
            self.native_sent + 1,
            ack,
            self.challenge
                .ok_or_else(|| anyhow::anyhow!("snapshot before challenge"))?,
            self.client_commands.command_for(ack)?,
            &body,
        )?;
        let packet = packets.remove(0);
        self.native_fragments.extend(packets);
        if snapshot_included && snapshot.len() > budget {
            tracing::info!(
                raw_bytes = snapshot.len(),
                wire_bytes = packet.len(),
                "compressed gameplay snapshot"
            );
        }
        self.native_sent += 1;
        self.server_commands.mark_sent(sent)?;
        if !snapshot_included {
            self.snapshot_next = Some(now + Duration::from_millis(50));
            return Ok(Some(packet));
        }
        if self.world.tdm.is_some() {
            self.world_history.push_back(WorldHistory {
                props,
                sounds: if full { Vec::new() } else { sounds },
                bags,
                smokes,
                missiles,
                local_abilities: self.world.player.as_ref().map(|p| p.equipment().abilities),
                replay_epoch,
                top_players_scene,
                held_weapons: !full,
                sequence: self.native_sent,
                local: local_player,
                players: match_players,
                impacts: if full { Vec::new() } else { impacts },
                // Standing presentation entities are encoded even in a full
                // frame. Retain those exact scalar bases for the next delta.
                corpses: if full {
                    corpses
                        .into_iter()
                        .filter(|c| {
                            c.phase == codol_protocol::game::snapshot::corpse::CorpsePhase::Posed
                        })
                        .collect()
                } else {
                    corpses
                },
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
        // 0x5a192c..55 subtracts PS.delta_angles before forming the raw
        // usercmd baseline. Movement and combat use corrected world angles.
        base.angles = [
            angles[0].wrapping_sub(view_offset[0]),
            angles[1].wrapping_sub(view_offset[1]),
            0,
        ];
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
    pub(super) fn remember_input_base(
        &mut self,
        sequence: u32,
        base: codol_protocol::game::channel::usercmd::UserCommand,
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
    pub(super) fn snapshot_motion(&self) -> Option<crate::game::player::movement::Motion> {
        self.world.motion.as_ref().map(|motion| {
            let mut camera = motion.clone();
            if self.world.vitals.intermission {
                // CG view offset 0x51d3b0 returns immediately for PM_INTERMISSION6.
                // Its origin must already be the eye, rather than the actor's feet.
                // Keep the actor and collision origin unchanged.
                camera.origin[2] += camera.view_height;
                camera.cancel_melee_charge();
                camera.velocity = [0.0; 3];
                camera.view_height = 0.0;
                camera.view_height_target = 0;
                camera.view_height_lerp_target = 0;
                camera.view_height_lerp_time = 0;
                camera.view_height_lerp_down = false;
            }
            // Target 3.3.3.3 PmoveSingle0x41dba0 tests PS.pm_flags0x800
            // at0x41dbe3 and clears movement/buttons at0x41dcdd. Input is gated
            // separately, including before the first world ACK.
            if self.world.tdm.as_ref().is_some_and(|m| m.preparing()) {
                camera.flags |= 0x800;
                camera.velocity = [0.0; 3];
            }
            camera
        })
    }
    pub(super) fn input_base_for(
        &self,
        ack: u32,
    ) -> Result<codol_protocol::game::channel::usercmd::UserCommand> {
        self.snapshot_input_bases
            .iter()
            .rev()
            .find(|(sequence, _)| *sequence <= ack)
            .map(|(_, base)| base.clone())
            .ok_or_else(|| anyhow::anyhow!("no retained input base for snapshot ACK"))
    }
}
