use super::*;

impl Connection {
    pub(super) fn advance_radios(&mut self, now: u32) -> Result<()> {
        let Some(resources) = self
            .world
            .game_state
            .as_ref()
            .and_then(|s| s.radar_resources())
            .cloned()
        else {
            return Ok(());
        };
        let Some(tdm) = &mut self.world.tdm else {
            return Ok(());
        };
        if let Some(player) = &mut self.world.player
            && self.world.armor.validate(
                now,
                tdm.streaks.life(0),
                tdm.finished(),
                player,
                &mut self.world.vitals,
            )
        {
            self.world.health_recovery = Default::default();
            self.server_commands
                .push(format!("a {}", player.equipment().weapon).as_bytes())?;
        }
        tdm.validate_remote(
            now,
            self.world.vitals.health > 0,
            self.world.player.as_ref().map(|p| p.equipment().weapon),
        );
        for change in tdm.remote.drain_vision() {
            if change.life == tdm.streaks.life(0)
                && self.world.vitals.health > 0
                && !tdm.finished()
                && tdm.remote.operator_life() == Some(change.life)
            {
                self.server_commands.push(&change.command())?;
            }
        }
        for change in tdm.remote.drain_weapon_changes() {
            if change.life == tdm.streaks.life(0)
                && self.world.vitals.health > 0
                && !tdm.finished()
                && tdm.remote.operator_life() == Some(change.life)
            {
                self.server_commands
                    .push(format!("a {}", change.weapon).as_bytes())?;
                if let Some(angles) = change.view_angles {
                    self.spawn_view_anchor = Some((angles, self.native_sent + 1));
                }
            }
        }
        for returned in tdm.remote.drain_returns() {
            if returned.life == tdm.streaks.life(0)
                && self.world.vitals.health > 0
                && !tdm.finished()
                && let Some(player) = &mut self.world.player
            {
                player.freeze_at(now);
                if player.equipment().weapon == returned.weapon {
                    player.remove_reward(returned.weapon);
                    self.server_commands
                        .push(format!("a {}", player.equipment().weapon).as_bytes())?;
                }
                self.spawn_view_anchor = Some((returned.angles, self.native_sent + 1));
            }
        }
        tdm.validate_strike_selection(
            now,
            self.world.vitals.health > 0,
            self.world.player.as_ref().map(|p| p.equipment().weapon),
        );
        if self.world.vitals.health == 0 || tdm.finished() {
            tdm.cancel_airdrop();
        } else if let Some(player) = &self.world.player
            && !resources.radios.iter().any(|r| {
                crate::game::rewards::mechanisms::Mechanism::marker(r.native_id)
                    && r.weapon == player.equipment().weapon
            })
        {
            tdm.cancel_airdrop();
        }
        if let Some(player) = &mut self.world.player {
            player
                .set_support_arrow(!tdm.finished() && tdm.streaks.sr71_active(tdm.teams()[0], now));
        }
        if self.world.vitals.health > 0
            && !tdm.finished()
            && !self.world.armor.active()
            && let Some(player) = &mut self.world.player
        {
            if self.radio_return.is_none() && !tdm.remote.busy() {
                let mut desired = [None; 3];
                for (slot, id) in tdm.streaks.owned_radios(0) {
                    desired[slot] = resources.radio(id).map(|r| r.weapon);
                }
                if player.sync_reward_slots(desired) {
                    self.server_commands
                        .push(format!("a {}", player.equipment().weapon).as_bytes())?;
                }
            }
            for (slot, id) in tdm.streaks.owned_radios(0) {
                if let Some(r) = resources.radio(id) {
                    if let Some(marker) =
                        crate::game::weapons::grenade::Definition::marker(&resources, id)
                    {
                        player.grant_marker_reward(slot + 1, r, marker);
                    } else {
                        player.grant_reward(slot + 1, r.weapon, r.switch, r.fire_interval_msec);
                    }
                }
            }
        }
        for event in tdm.streaks.drain_radios() {
            match event {
                crate::game::rewards::killstreaks::RadioEvent::Earned { actor, id, time } => {
                    tracing::info!(actor, id, time, "radio reward earned")
                }
                crate::game::rewards::killstreaks::RadioEvent::Called {
                    actor,
                    id,
                    team,
                    time,
                } => {
                    if id == 14
                        && let Some(&fx) = resources.effects.get("explosions/emp_flash_mp")
                    {
                        self.impacts
                            .fx(now, fx, self.world.motion.as_ref().unwrap().origin);
                    }
                    tracing::info!(
                        actor,
                        id,
                        team,
                        time,
                        duration_msec = if matches!(id, 14 | 16) {
                            60_000
                        } else {
                            crate::game::rewards::killstreaks::RADAR_MSEC
                        },
                        "radio support called"
                    );
                }
            }
        }
        if self.radio_return.is_some_and(|(time, _)| now >= time) {
            let (_, id) = self.radio_return.take().unwrap();
            if let Some(player) = &mut self.world.player {
                player.remove_reward(id);
                self.server_commands
                    .push(format!("a {}", player.equipment().weapon).as_bytes())?;
            }
        }
        Ok(())
    }
    pub(super) fn advance_nuke(&mut self, now: u32) -> Result<()> {
        let Some(resources) = self
            .world
            .game_state
            .as_ref()
            .and_then(|s| s.nuke_resources())
            .cloned()
        else {
            return Ok(());
        };
        let Some(tdm) = &mut self.world.tdm else {
            return Ok(());
        };
        if self.world.vitals.health > 0
            && tdm.streaks.owned(0)
            && !tdm.finished()
            && !self.world.armor.active()
            && let Some(player) = &mut self.world.player
        {
            player.grant_nuke(&resources);
        }
        tdm.restore_nuke_rewards(&resources);
        tdm.streaks.advance(now);
        let events = tdm.streaks.drain();
        for event in events {
            use crate::game::rewards::killstreaks::Event;
            match event {
                Event::Earned { actor, time } => {
                    tracing::info!(actor, time, weapon = resources.weapon, "nuke reward earned");
                    if actor == 0 {
                        self.nuke_banner = Some((now + 2500, resources.earned_text));
                        let team = self.world.tdm.as_ref().unwrap().teams()[0];
                        if let Some(sound) = self
                            .world
                            .game_state
                            .as_ref()
                            .unwrap()
                            .voice_resources()
                            .and_then(|r| r.nuke_announcement(team, false, true))
                        {
                            self.server_commands.push(sound.play_command().as_bytes())?;
                        }
                    }
                }
                Event::Called { actor, time } => {
                    self.nuke_banner = None;
                    let name = if actor == 0 {
                        self.display_name.as_deref().unwrap_or("Player").to_owned()
                    } else {
                        codol_protocol::room::messages::bot_name(actor)
                    };
                    self.server_commands
                        .push(&crate::game::rewards::killstreaks::called_message(&name))?;
                    self.server_commands
                        .push(format!("p {}", resources.music_sound).as_bytes())?;
                    let team = self.world.tdm.as_ref().unwrap().teams()[usize::from(actor)];
                    let enemy = self.world.tdm.as_ref().unwrap().teams()[0] != team;
                    if let Some(sound) = self
                        .world
                        .game_state
                        .as_ref()
                        .unwrap()
                        .voice_resources()
                        .and_then(|r| r.nuke_announcement(team, enemy, false))
                    {
                        self.server_commands.push(sound.play_command().as_bytes())?;
                    }
                    tracing::info!(actor, time, countdown_msec = 10000, "nuke called");
                }
                Event::Explosion { actor, time } => {
                    let motion = self.world.motion.as_ref().unwrap();
                    let yaw = f32::from(self.view_angles()[1]) * std::f32::consts::TAU / 65536.0;
                    let point = [
                        motion.origin[0] + yaw.cos() * 5000.0,
                        motion.origin[1] + yaw.sin() * 5000.0,
                        motion.origin[2] + 60.0,
                    ];
                    ensure!(
                        self.impacts.fx(now, resources.flash_fx, point).is_some(),
                        "nuke flash placement invalid"
                    );
                    ensure!(
                        self.impacts.fx(now, resources.blast_fx, point).is_some(),
                        "nuke blast placement invalid"
                    );
                    tracing::info!(actor, time, flash = resources.flash_fx, blast = resources.blast_fx, origin = ?point, "nuke explosion FX emitted");
                }
                Event::Vision { actor, time } => {
                    if resources.vision.is_some() {
                        self.server_commands.push(b"N \"mpnuke\" 3000")?;
                        tracing::info!(actor, time, "nuke vision started");
                    }
                }
                Event::End { actor, time } => {
                    if resources.vision.is_some() {
                        self.server_commands.push(b"N \"mpnuke_aftermath\" 5000")?;
                    }
                    let state = self.world.game_state.as_ref().unwrap();
                    let mut local = state
                        .local_player_snapshot(self.world.vitals.health > 0)
                        .unwrap();
                    local.origin = self.world.motion.as_ref().unwrap().origin;
                    local.angles = self.view_angles();
                    local.weapon = self.world.player.as_ref().unwrap().equipment().weapon;
                    self.world.tdm.as_mut().unwrap().finish_nuke(
                        time,
                        actor,
                        resources.damage_weapon,
                        local,
                    );
                    self.world.vitals.health = 0;
                    self.world.player_death = Some(time);
                    self.world.health_recovery = Default::default();
                    self.nuke_banner = None;
                    self.replay.cancel("nuclear end of match");
                    self.final_replay_considered = true;
                    self.last_kill = None;
                    self.impacts.fx(now, resources.aftermath_fx, local.origin);
                    self.drain_match_kills(now);
                    tracing::info!(
                        actor,
                        time,
                        winner = self.world.tdm.as_ref().unwrap().winning_team(),
                        "nuke match end"
                    );
                }
            }
        }
        if let Some(end) = self.world.tdm.as_ref().unwrap().streaks.countdown_end() {
            let second = end.saturating_sub(now).div_ceil(1000);
            if self.nuke_tick_second != Some(second) {
                self.nuke_tick_second = Some(second);
                self.server_commands
                    .push(format!("p {}", resources.tick_sound).as_bytes())?;
            }
        }
        if self.nuke_return_at.is_some_and(|time| now >= time) {
            self.nuke_return_at = None;
            if let Some(player) = &mut self.world.player {
                player.remove_nuke(resources.weapon);
                // PS weapon correction does not reset CG's selected weapon;
                // native command a -> 0x562260 returns its HUD/ammo selection.
                self.server_commands
                    .push(format!("a {}", player.equipment().weapon).as_bytes())?;
            }
        }
        Ok(())
    }
}
