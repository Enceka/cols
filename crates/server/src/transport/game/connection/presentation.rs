use super::*;

impl Connection {
    pub(super) fn advance_voices(&mut self, now: u32) -> Result<()> {
        let Some(state) = self.world.game_state.clone() else {
            return Ok(());
        };
        let Some(resources) = state.voice_resources() else {
            self.voice_deaths.clear();
            self.voice_throws.clear();
            return Ok(());
        };
        let lives: std::collections::BTreeMap<_, _> = self
            .world
            .tdm
            .as_ref()
            .map_or_else(Vec::new, |m| m.scoreboard(self.owner_id).rows)
            .into_iter()
            .map(|r| (r.client_number, r.deaths))
            .collect();
        let mut actors: Vec<_> = self
            .world
            .tdm
            .as_ref()
            .map_or_else(Vec::new, |m| m.snapshots())
            .into_iter()
            .map(|s| {
                let mut actor = state.voice_actor(s);
                actor.life = lives.get(&actor.client).copied().unwrap_or(0);
                actor
            })
            .collect();
        if let Some(mut local) = state.local_player_snapshot(self.world.vitals.health > 0) {
            if let Some(motion) = &self.world.motion {
                local.origin = motion.origin;
            }
            if let Some(events) = self
                .world
                .player
                .as_ref()
                .and_then(|p| p.equipment().events)
            {
                local.events = events;
            }
            let mut actor = state.voice_actor(local);
            actor.life = self.world.player_birth;
            actors.push(actor);
        }
        let ready = !self.resource_collection
            && self.hud_rules_sent
            && self.server_commands.acknowledged() >= self.hud_rules_sequence
            && self
                .snapshot_first_sequence
                .is_some_and(|first| self.native_ack >= first);
        let gameplay =
            !self.replay.pending() && !self.world.tdm.as_ref().is_some_and(|m| m.finished());
        // Bind committed kills to the cached prior life before observing
        // respawns in this frame; delayed speech must not switch owners.
        for &(time, victim, attacker) in &self.voice_deaths {
            if ready && gameplay && victim != attacker {
                self.voices.killed(time, attacker);
            }
        }
        self.voices.advance(
            now,
            resources,
            &actors,
            ready,
            gameplay,
            &mut self.server_commands,
        )?;
        let local = actors.iter().find(|a| a.client == 0);
        for (time, victim, _attacker) in self.voice_deaths.drain(..) {
            if ready
                && (gameplay || victim == 0)
                && now.wrapping_sub(time) < 200
                && let Some(actor) = actors.iter().find(|a| a.client == victim)
            {
                self.voices
                    .role(now, resources, *actor, "mp_death", local, false);
            }
        }
        for suffix in self.voice_throws.drain(..) {
            if ready
                && gameplay
                && let Some(local) = local
            {
                self.voices
                    .role(now, resources, *local, suffix, Some(local), true);
            }
        }
        if self.world.vitals.damage_event != self.voice_damage_event {
            self.voice_damage_event = self.world.vitals.damage_event;
            if ready
                && gameplay
                && let Some(local) = local.filter(|a| a.alive)
            {
                self.voices
                    .role(now, resources, *local, "mp_pain", Some(local), false);
            }
        }
        Ok(())
    }
    pub(super) fn hud_equipment(
        &self,
        mut equipment: codol_protocol::game::snapshot::Equipment,
    ) -> codol_protocol::game::snapshot::Equipment {
        if !self.hud_rules_sent || self.server_commands.acknowledged() < self.hud_rules_sequence {
            equipment.perks = [0; 3];
        }
        equipment
    }
    pub(super) fn perk_display(
        &self,
        equipment: Option<codol_protocol::game::snapshot::Equipment>,
    ) -> bool {
        if self.world.vitals.health == 0
            || self.world.tdm.as_ref().is_some_and(|m| m.finished())
            || equipment.is_none_or(|e| e.perks == [0; 3])
        {
            return false;
        }
        true
    }
    pub(super) fn queue_obituary(
        &mut self,
        now: u32,
        weapon: u16,
        victim: u8,
        attacker: u8,
        cause: KillCause,
    ) {
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
            codol_protocol::game::snapshot::obituary::ObituarySnapshot {
                falling: cause.falling,
                entity_number,
                weapon,
                headshot: cause.headshot,
                melee: cause.melee,
                victim,
                attacker,
            },
        ));
        self.voice_deaths.push_back((now, victim, attacker));
        if cause.headshot && attacker < 2 {
            self.headshots[usize::from(attacker)] += 1;
        }
        self.scoreboard_sent = false;
        tracing::info!(
            victim,
            attacker,
            weapon,
            headshot = cause.headshot,
            "training kill obituary queued"
        );
    }
    pub(super) fn drain_match_kills(&mut self, now: u32) {
        let kills = self.world.tdm.as_mut().unwrap().take_kills();
        for kill in kills {
            let nuclear = self
                .world
                .game_state
                .as_ref()
                .and_then(|s| s.nuke_resources())
                .is_some_and(|r| r.damage_weapon == kill.weapon);
            if !nuclear
                && let Some(bag) = self.world.scavenger.drop_for_kill(
                    kill,
                    self.world
                        .game_state
                        .as_ref()
                        .unwrap()
                        .collision
                        .as_ref()
                        .unwrap(),
                )
            {
                tracing::info!(time = kill.time, entity = bag.entity_number, victim = kill.victim.client_index, origin = ?bag.origin, "scavenger bag dropped");
            }
            if !kill.falling && !nuclear {
                self.last_kill = Some(kill);
            }
            if kill.victim.client_index == 0
                && !nuclear
                && kill.attacker != 0
                && !self.world.tdm.as_ref().unwrap().finished()
            {
                self.replay.start(
                    crate::game::presentation::replay::Kind::Death,
                    kill.time,
                    now,
                    kill.attacker,
                );
            }
            if kill.attacker != 0
                || self
                    .world
                    .player
                    .as_ref()
                    .is_none_or(|p| !p.hides_victim_death_icon())
            {
                self.death_waypoints.record(
                    now,
                    self.world.tdm.as_ref().unwrap().teams()[0],
                    kill.victim,
                );
            }
            let mut corpse = self
                .world
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
                KillCause {
                    headshot: kill.headshot,
                    melee: kill.melee,
                    falling: kill.falling,
                },
            );
        }
    }
    pub(super) fn advance_match_summary(&mut self, now: u32) -> Result<()> {
        if self.match_summary_sequence.is_none() {
            return Ok(());
        }
        if self.replay.pending() {
            return Ok(());
        }
        if !self.resource_collection
            && let Some(state) = &self.world.game_state
            && let Some(resources) = state.voice_resources()
            && let Some(tdm) = &self.world.tdm
        {
            self.voices.finish(
                resources,
                tdm.teams()[0],
                tdm.winning_team(),
                &mut self.server_commands,
            )?;
        }
        self.world.vitals.intermission = true;
        if let Some(motion) = &mut self.world.motion {
            motion.cancel_melee_charge();
            motion.velocity = [0.0; 3];
        }
        if let Some(menu) = self.world.game_state.as_ref().and_then(|s| s.top_players()) {
            if self.top_players.scene.is_none() {
                let state = self.world.game_state.as_ref().unwrap();
                let mut actors = self.world.tdm.as_ref().unwrap().snapshots();
                let mut local = state.local_player_snapshot(true).unwrap();
                local.weapon = self.world.player.as_ref().unwrap().equipment().weapon;
                actors.push(local);
                let mut scene = state.top_players_scene(
                    &self
                        .world
                        .tdm
                        .as_ref()
                        .unwrap()
                        .scoreboard(self.owner_id)
                        .rows,
                    actors,
                )?;
                for pose in &mut scene.poses {
                    if let Some((_, previous)) = self.corpses.get(&pose.entity_number) {
                        // Native ResetEntity compares the teleport low bit;
                        // reusing a corpse slot must initialize a fresh tree.
                        pose.death_sequence = previous.death_sequence.wrapping_add(1).max(1);
                    }
                }
                self.top_players.scene = Some(scene);
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
                    &self
                        .world
                        .tdm
                        .as_ref()
                        .unwrap()
                        .scoreboard(self.owner_id)
                        .rows,
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
                winner = self.world.tdm.as_ref().unwrap().winning_team(),
                result = self.world.tdm.as_ref().unwrap().result(),
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
}
