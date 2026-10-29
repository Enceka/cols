use super::*;

impl Connection {
    pub(super) fn advance_bot(&mut self, now: u32) -> Result<()> {
        if self.world.bot.is_none()
            || self
                .snapshot_first_sequence
                .is_none_or(|first| self.native_ack < first)
        {
            return Ok(());
        }
        let respawning = self
            .world
            .player_death
            .is_some_and(|death| now.wrapping_sub(death) >= 3000);
        if respawning {
            self.equip_spawn_loadout();
            self.reset_spawn_view(self.world.game_state.as_ref().unwrap().spawn_angles());
        }
        let state = self.world.game_state.as_ref().unwrap();
        if respawning {
            self.world.player_death = None;
            if let Some(tdm) = &mut self.world.tdm {
                tdm.streaks.spawn(0);
            }
            self.hit_feedback.clear();
            self.world.player_birth = now;
            self.world.health_recovery = Default::default();
            let spawn_count = self.world.vitals.spawn_count.wrapping_add(1);
            self.world.vitals = codol_protocol::game::snapshot::PlayerVitals {
                spawn_count,
                ..Default::default()
            };
            self.world.motion = state.motion();
            if let Some(motion) = &mut self.world.motion {
                motion.command_time = now;
            }
            if let (Some(player), Some(primary)) = (&mut self.world.player, state.primary_player())
            {
                player.replace_loadout(primary, state.secondary_player());
            }
            self.feedback.push_back("Training player respawned".into());
            tracing::info!(spawn_count,origin=?self.world.motion.as_ref().map(|m| m.origin),"training player respawned");
        }
        let recovered = self.world.health_recovery.recover_to(
            now,
            self.world.vitals.health,
            self.world.vitals.max_health,
        );
        if recovered != self.world.vitals.health {
            self.world.vitals.health = recovered;
            if recovered == self.world.vitals.max_health {
                self.world.vitals.damage_count = 0;
            }
            tracing::info!(health = recovered, "training player recovered health");
        }
        let (Some(weapon), Some(motion), Some(collision)) = (
            state.bot_weapon(),
            self.world.motion.as_ref(),
            state.collision.as_ref(),
        ) else {
            return Ok(());
        };
        let bot = self.world.bot.as_mut().unwrap();
        if self
            .world
            .combat
            .as_ref()
            .is_some_and(|world| world.health() == 0)
        {
            bot.kill(now);
        }
        let frame = bot.step(
            now,
            Some(crate::game::ai::controller::BotTarget {
                origin: motion.origin,
                eye: [
                    motion.origin[0],
                    motion.origin[1],
                    motion.origin[2] + motion.view_height,
                ],
                // A short training spawn grace gives the player time to reorient.
                alive: self.world.vitals.health > 0
                    && now.wrapping_sub(self.world.player_birth) >= 2500,
            }),
            weapon,
            collision,
        );
        if self
            .world
            .bot_frame
            .as_ref()
            .is_none_or(|previous| previous.phase != frame.phase)
        {
            tracing::info!(phase=?frame.phase,origin=?frame.motion.origin,"training bot behavior changed");
        }
        if let Some(combat) = &mut self.world.combat {
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
                .push(codol_protocol::game::snapshot::events::FIRE_WEAPON, 0)?;
            self.enemy_events
                .push(codol_protocol::game::snapshot::events::EJECT_BRASS, 0)?;
            self.world.bot_fire_time = Some(now);
            self.world.bot_fire_toggle = !self.world.bot_fire_toggle;
            if self.world.vitals.health > 0
                && let Some(damage) = crate::game::combat::hits_player(
                    shot.eye,
                    shot.angles,
                    shot.ballistics,
                    motion.origin,
                    motion.stance.height(),
                    collision,
                )
            {
                self.world.vitals.health = self.world.vitals.health.saturating_sub(damage);
                self.world.health_recovery.damaged_to(
                    now,
                    self.world.vitals.health,
                    self.world.vitals.max_health,
                );
                self.world.vitals.damage_event = self.world.vitals.damage_event.wrapping_add(1);
                self.world.vitals.damage_count = damage.min(127) as u8;
                tracing::info!(
                    shot_time = now,
                    health = self.world.vitals.health,
                    "training bot hit player"
                );
                if self.world.vitals.health == 0 {
                    self.world.player_death = Some(now);
                    self.hit_feedback.clear();
                    if let Some(mut local) = state.local_player_snapshot(true) {
                        local.origin = motion.origin;
                        local.angles = self
                            .last_input
                            .as_ref()
                            .map_or([0; 3], |input| input.angles);
                        if let Some(player) = self.world.player.as_ref() {
                            local.weapon = player.equipment().weapon;
                        }
                        self.death_sequence[0] = self.death_sequence[0].wrapping_add(1).max(1);
                        self.corpses
                            .insert(20, (now, state.corpse(local, self.death_sequence[0])));
                        self.queue_obituary(now, shot.weapon, 0, 1, KillCause::default());
                    }
                }
            }
        }
        self.world.bot_frame = Some(frame);
        Ok(())
    }
    pub(super) fn advance_match(&mut self, now: u32) -> Result<()> {
        if self.world.vitals.health == 0 || self.world.tdm.as_ref().is_some_and(|m| m.finished()) {
            self.world.tactical.clear();
            self.world.poison.clear();
            self.world.emp.clear();
        }
        if let Some(motion) = &mut self.world.motion {
            motion.flags = (motion.flags & !0x8000)
                | if self.world.tactical.stunned(now) {
                    0x8000
                } else {
                    0
                };
        }
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
        if !self.world.tdm.as_ref().unwrap().live_at(now) {
            return Ok(());
        }
        if self
            .world
            .player_death
            .is_some_and(|death| now.wrapping_sub(death) >= 3000)
            && !self.world.tdm.as_ref().unwrap().finished()
            && !self.replay.pending()
        {
            self.equip_spawn_loadout();
            let collision = self
                .world
                .game_state
                .as_ref()
                .unwrap()
                .collision
                .as_ref()
                .unwrap()
                .clone();
            let spawn = self
                .world
                .tdm
                .as_mut()
                .unwrap()
                .respawn_point(now, 0, None, &collision)?;
            self.reset_spawn_view(spawn.angles);
            let state = self.world.game_state.as_ref().unwrap();
            self.world.motion = state.motion();
            if let Some(motion) = &mut self.world.motion {
                motion.origin = spawn.origin;
                motion.command_time = now;
            }
            self.world.vitals = codol_protocol::game::snapshot::PlayerVitals {
                spawn_count: self.world.vitals.spawn_count.wrapping_add(1),
                ..Default::default()
            };
            self.world.player_death = None;
            self.hit_feedback.clear();
            self.world.player_birth = now;
            self.world.health_recovery = Default::default();
            self.world.poison.clear();
            self.world.emp.clear();
            if let Some(primary) = state.primary_player() {
                self.world
                    .player
                    .as_mut()
                    .unwrap()
                    .replace_loadout(primary, state.secondary_player());
            }
        }
        let state = self.world.game_state.as_ref().unwrap().clone();
        let Some(motion) = &self.world.motion else {
            return Ok(());
        };
        let mut local = state
            .local_player_snapshot(self.world.vitals.health > 0)
            .unwrap();
        local.origin = motion.origin;
        local.weapon = self.world.player.as_ref().unwrap().equipment().weapon;
        local.angles = self.view_angles();
        let active =
            self.world.vitals.health > 0 && now.wrapping_sub(self.world.player_birth) >= 1000;
        local.alive = active;
        let collision = state.collision.as_ref().unwrap();
        let mut tick = self.world.tdm.as_mut().unwrap().step_with_smoke(
            now,
            (
                local,
                motion.view_height + 10.0,
                if active { self.world.vitals.health } else { 0 },
            ),
            state.match_bot_weapon().unwrap(),
            &self.world.smoke,
            collision,
            |snapshot, frame, firing, toggle| state.animate_enemy(snapshot, frame, firing, toggle),
        )?;
        let health = tick.hits.iter().fold(
            if active { self.world.vitals.health } else { 0 },
            |h, hit| h.saturating_sub(hit.damage),
        );
        local.alive = health > 0;
        if let (Some(diagnostics), Some(resources)) =
            (state.nuke_resources(), state.radar_resources())
        {
            self.world.tdm.as_mut().unwrap().acceptance_enemy_support(
                now,
                diagnostics,
                resources,
                collision,
            );
        }
        let devices = self.world.tdm.as_mut().unwrap().step_deployables(
            now,
            (local, motion.view_height + 10.0, health),
            self.world
                .player
                .as_ref()
                .is_some_and(|p| p.sentry_undetectable()),
            collision,
        );
        tick.hits.extend(devices.hits);
        tick.impacts.extend(devices.impacts);
        let aircraft = self.world.tdm.as_mut().unwrap().step_aircraft(
            now,
            (
                local,
                motion.view_height + 10.0,
                self.world.player_birth,
                self.world.player.as_ref().map_or([false; 2], |p| {
                    [p.helicopter_undetectable(), p.airplane_undetectable()]
                }),
            ),
            collision,
        );
        tick.hits.extend(aircraft.hits);
        tick.impacts.extend(aircraft.impacts);
        for hit in tick.hits {
            if state
                .nuke_resources()
                .is_some_and(|r| r.acceptance_seed != 0 && r.acceptance_fire_after_msec.is_none())
            {
                continue;
            }
            if self.world.vitals.health == 0 {
                break;
            }
            self.world.vitals.health = self.world.vitals.health.saturating_sub(hit.damage);
            self.world.vitals.damage_event = self.world.vitals.damage_event.wrapping_add(1);
            self.world.vitals.damage_count = hit.damage.min(127) as u8;
            self.world.health_recovery.damaged_to(
                now,
                self.world.vitals.health,
                self.world.vitals.max_health,
            );
            if self.world.vitals.health == 0 {
                self.world.player_death = Some(now);
                self.hit_feedback.clear();
                self.world
                    .tdm
                    .as_mut()
                    .unwrap()
                    .player_killed(now, local, hit);
                self.world
                    .player
                    .as_mut()
                    .unwrap()
                    .drop_offhand_on_death(now, local.angles);
                if let Some(motion) = &mut self.world.motion {
                    motion.cancel_melee_charge();
                    motion.velocity = [0.0; 3];
                }
            }
        }
        self.world.vitals.health = self.world.health_recovery.recover_to(
            now,
            self.world.vitals.health,
            self.world.vitals.max_health,
        );
        if self.world.vitals.health == self.world.vitals.max_health {
            self.world.vitals.damage_count = 0;
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

    /// Advance the cooperative mission and honour its retry checkpoints.
    pub(super) fn advance_story(&mut self, now: u32) -> Result<()> {
        let Some(origin) = self.world.motion.as_ref().map(|motion| motion.origin) else {
            return Ok(());
        };
        let alive = self.world.vitals.health > 0;
        // Story enemies are the captured destructible target: a kill is the
        // frame its health reaches zero, and the target respawns afterwards.
        let target_health = self.world.combat.as_ref().map(|combat| combat.health());
        let killed = match (self.story_target_health, target_health) {
            (Some(previous), Some(0)) => previous > 0,
            _ => false,
        };
        self.story_target_health = target_health;
        let Some(story) = self.world.story.as_mut() else {
            return Ok(());
        };
        if killed {
            story.record_kill();
            tracing::info!(stage = %story.stage_id(), "story enemy killed");
        }
        story.advance(
            now,
            crate::game::modes::story::PlayerFrame { origin, alive },
        );
        let events = story.take_events();
        let respawn = story.respawn_origin();
        let angles = story.spawn_angles();
        let attempts = story.attempts();
        let finished = story.outcome();
        let spawn = story.spawn_origin();
        let stage = story.stage_index();
        let stage_id = story.stage_id().to_string();
        let completion = story.mission().completion.native_flag.clone();
        for event in &events {
            match event {
                crate::game::modes::story::Event::StageStarted { index, id } => {
                    self.feedback
                        .push_back(format!("objective {} started: {}", index + 1, id));
                    tracing::info!(index, id = %id, stage, spawn = ?spawn, "story stage started");
                }
                crate::game::modes::story::Event::Checkpoint { index } => {
                    self.feedback.push_back(format!("checkpoint {}", index + 1));
                    tracing::info!(index, stage = %stage_id, "story checkpoint");
                }
                crate::game::modes::story::Event::Retried { attempts } => {
                    tracing::info!(attempts, stage = %stage_id, "story attempt failed");
                }
                crate::game::modes::story::Event::Completed { seconds } => {
                    self.feedback
                        .push_back(format!("mission complete in {}s", seconds));
                    tracing::info!(seconds, flag = %completion, "story mission completed");
                    let story_map = story.mission().map.clone();
                    // The ledger records tries, so a first-attempt clear is one try.
                    self.story_completion = Some((story_map, attempts + 1, *seconds));
                }
            }
        }
        if events
            .iter()
            .any(|event| matches!(event, crate::game::modes::story::Event::Retried { .. }))
        {
            self.retry_story(now, respawn, angles, attempts)?;
        }
        if let Some(outcome) = finished
            && !self.story_outcome_logged
        {
            self.story_outcome_logged = true;
            tracing::info!(?outcome, "story mission outcome");
        }
        Ok(())
    }
    fn retry_story(
        &mut self,
        now: u32,
        origin: [f32; 3],
        angles: [u16; 3],
        attempts: u32,
    ) -> Result<()> {
        self.equip_spawn_loadout();
        self.reset_spawn_view(angles);
        let state = self.world.game_state.as_ref().unwrap().clone();
        self.world.motion = state.motion();
        if let Some(motion) = &mut self.world.motion {
            motion.origin = origin;
            motion.command_time = now;
        }
        self.world.vitals = codol_protocol::game::snapshot::PlayerVitals {
            spawn_count: self.world.vitals.spawn_count.wrapping_add(1),
            ..Default::default()
        };
        self.world.player_death = None;
        self.hit_feedback.clear();
        self.world.player_birth = now;
        self.world.health_recovery = Default::default();
        self.world.poison.clear();
        self.world.emp.clear();
        if let Some(primary) = state.primary_player() {
            self.world
                .player
                .as_mut()
                .unwrap()
                .replace_loadout(primary, state.secondary_player());
        }
        tracing::info!(attempts, "story mission checkpoint restart");
        Ok(())
    }
}
