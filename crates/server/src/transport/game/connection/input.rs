use super::*;

impl Connection {
    pub(super) fn queue_input(
        &mut self,
        input: codol_protocol::game::channel::usercmd::UserCommand,
    ) {
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
    pub(super) fn consume_inputs(&mut self, through: u32) {
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
            if was_replaying && let Some(player) = &self.world.player {
                input.weapons = player.equipment().input_weapons();
            }
            if (self.world.bot.is_some() || self.world.tdm.is_some())
                && (self.world.vitals.health == 0
                    || input.time < self.world.player_birth
                    || self.world.tdm.as_ref().is_some_and(|m| m.finished()))
            {
                input.buttons = 0;
                input.direction = 0;
            }
            let raw_angles = input.angles;
            if let Some((angles, sequence)) = self.spawn_view_anchor {
                if input.time >= self.world.player_birth && self.native_ack >= sequence {
                    self.view_offset =
                        std::array::from_fn(|i| angles[i].wrapping_sub(raw_angles[i]));
                    self.spawn_view_anchor = None;
                } else {
                    input.angles = angles;
                }
            }
            input.angles = if input.time < self.world.player_birth {
                self.view_angles()
            } else if let Some((angles, _)) = self.spawn_view_anchor {
                angles
            } else {
                std::array::from_fn(|i| input.angles[i].wrapping_add(self.view_offset[i]))
            };
            if self
                .world
                .tdm
                .as_ref()
                .is_some_and(|m| !m.live_at(input.time))
            {
                // ACK the executing command while preserving the frozen body,
                // view and weapon state. Backpack menu requests are reliable
                // commands and remain available through the separate parser.
                input.buttons = 0;
                input.direction = 0;
                input.angles = self.view_angles();
                if let Some(player) = &self.world.player {
                    input.weapons = player.equipment().input_weapons();
                }
                if let Some(player) = &mut self.world.player {
                    player.freeze_at(input.time);
                }
                if let Some(motion) = &mut self.world.motion {
                    motion.cancel_melee_charge();
                    motion.cancel_dive();
                    motion.cancel_sprint();
                    motion.velocity = [0.0; 3];
                    motion.command_time = input.time;
                }
                self.input_count += 1;
                self.last_input = Some(input);
                continue;
            }
            if let (Some(tdm), Some(state)) = (&mut self.world.tdm, &self.world.game_state) {
                tdm.validate_remote(
                    input.time,
                    self.world.vitals.health > 0,
                    self.world.player.as_ref().map(|p| p.equipment().weapon),
                );
                let body_angles = tdm.remote.body_angles();
                if let Some(collision) = state.collision.as_ref()
                    && tdm.remote.input(&input, collision)
                {
                    if let Some(player) = &mut self.world.player {
                        player.freeze_at(input.time);
                    }
                    if let Some(motion) = &mut self.world.motion {
                        motion.freeze_remote(input.time);
                    }
                    if let Some(angles) = body_angles {
                        input.angles = angles;
                    }
                    self.input_count += 1;
                    self.last_input = Some(input);
                    continue;
                }
            }
            if self.world.armor.active()
                && let Some(player) = &self.world.player
            {
                input.weapons = player.equipment().input_weapons();
            }
            if let Some(motion) = &mut self.world.motion
                && let Some(collision) = self
                    .world
                    .game_state
                    .as_ref()
                    .and_then(|s| s.collision.as_ref())
            {
                if self.world.tdm.as_ref().is_some_and(|m| m.finished()) {
                    motion.cancel_melee_charge();
                    motion.cancel_dive();
                    motion.velocity = [0.0; 3];
                    motion.command_time = input.time;
                    self.last_input = Some(input);
                    continue;
                }
                let robot = self.world.player.as_ref().is_some_and(|p| p.asym_robot());
                motion.flags = (motion.flags & !0x8000)
                    | if self.world.tactical.stunned(input.time) {
                        0x8000
                    } else {
                        0
                    };
                let ads = self
                    .world
                    .player
                    .as_ref()
                    .and_then(|p| p.equipment().aim)
                    .is_some_and(|a| a.movement_slow);
                let scale = self.world.player.as_ref().map_or(1.0, |p| {
                    if ads {
                        p.ads_move_scale()
                    } else {
                        p.move_scale()
                    }
                }) * self.world.tactical.move_scale(input.time, robot);
                let old_bob = motion.bob_cycle;
                let was_ground = motion.ground_entity != 2047;
                let was_approaching = motion.melee_charge().is_some();
                if let Some(player) = &self.world.player {
                    motion.set_sprint_rules(player.sprint_rules());
                    motion.set_dive_rules(player.dive_rules());
                }
                motion.apply(&input, scale, ads, collision);
                if was_approaching {
                    tracing::debug!(command_time=input.time,origin=?motion.origin,velocity=?motion.velocity,
                        active=motion.melee_charge().is_some(),"melee approach motion");
                }
                if let Some(player) = &mut self.world.player {
                    for event in motion.take_movement_events() {
                        player.movement_event(event.id, event.parameter);
                    }
                    let surface = (motion.surface_flags >> 20) & 31;
                    if was_ground
                        && motion.ground_entity == 2047
                        && motion.velocity[2] > 0.0
                        && input.buttons & 0x400 != 0
                    {
                        player
                            .movement_event(codol_protocol::game::snapshot::events::JUMP, surface);
                    }
                    if (old_bob ^ motion.bob_cycle) & 128 != 0 && motion.ground_entity != 2047 {
                        let event = match motion.stance {
                            crate::game::player::movement::Stance::Prone => {
                                codol_protocol::game::snapshot::events::FOOTSTEP_PRONE
                            }
                            _ if motion.flags & 0x4000 != 0 => {
                                codol_protocol::game::snapshot::events::FOOTSTEP_SPRINT
                            }
                            _ if ads => codol_protocol::game::snapshot::events::FOOTSTEP_WALK,
                            _ => codol_protocol::game::snapshot::events::FOOTSTEP_RUN,
                        };
                        player.movement_event(event, surface);
                    }
                }
            }
            let landings = self
                .world
                .motion
                .as_mut()
                .map_or_else(Vec::new, |m| m.take_landings());
            for landing in landings {
                self.apply_landing(landing);
            }
            if self.world.vitals.health == 0 {
                input.buttons = 0;
                input.direction = 0;
            }
            // Native0x41e525/0x429320 advances movement before the weapon
            // state machine. Sprint entry/exit and melee use this command's hull.
            let melee_approach = self.world.tdm.as_ref().and_then(|tdm| {
                let motion = self.world.motion.as_ref()?;
                let state = self.world.game_state.as_ref()?;
                let mut local = state.local_player_snapshot(self.world.vitals.health > 0)?;
                local.origin = motion.origin;
                tdm.player_melee_approach(&input, motion, local, state.collision.as_ref()?)
            });
            if let Some(player) = &mut self.world.player {
                if let Some(motion) = &self.world.motion {
                    player.spread_environment(motion);
                    player.set_sprinting(motion.flags & 0x4000 != 0);
                }
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
                        current = player.equipment().weapon,
                        "unowned weapon switch requested"
                    );
                }
                let previous_equipment = player.equipment();
                let frames = self
                    .world
                    .motion
                    .as_mut()
                    .map_or_else(Vec::new, |m| m.take_dive_frames());
                player.apply_with_dive_frames(&input, melee_approach, &frames);
                if let Some(motion) = &mut self.world.motion
                    && let Some((approach, duration)) = player.take_melee_approach()
                {
                    motion.start_melee_charge(approach, duration);
                    tracing::info!(distance=approach.distance,yaw=approach.yaw,duration,
                        origin=?motion.origin,"native melee approach started");
                }
                let shots = player.take_shots();
                let melee_strikes = player.take_melee_strikes();
                let throws = player.take_throws();
                let fired = player.equipment().weapon == previous_equipment.weapon
                    && previous_equipment
                        .ammo
                        .zip(player.equipment().ammo)
                        .is_some_and(|(old, new)| new.clip < old.clip);
                if (fired || !shots.is_empty() || !melee_strikes.is_empty() || !throws.is_empty())
                    && let Some(tdm) = &mut self.world.tdm
                {
                    tdm.mark_combat(0);
                }
                if let (Some(motion), Some(collision)) = (
                    &self.world.motion,
                    self.world
                        .game_state
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
                    for request in throws {
                        if let Some(id) = request.definition.marker_reward {
                            if self
                                .world
                                .tdm
                                .as_mut()
                                .is_none_or(|m| !m.launch_airdrop_marker(request.time, id))
                            {
                                continue;
                            }
                            self.radio_return = Some((
                                request.time + u32::from(request.definition.throw_msec),
                                request.definition.index,
                            ));
                        }
                        let cue = (!request.in_hand && !request.dropped)
                            .then(|| {
                                crate::game::presentation::voice::throw_suffix(
                                    &request.definition.name,
                                )
                            })
                            .flatten();
                        if self
                            .world
                            .grenades
                            .launch(request, eye, motion.velocity, &collision)
                            && let Some(cue) = cue
                        {
                            self.voice_throws.push_back(cue);
                        }
                    }
                    for strike in melee_strikes {
                        if let Some(tdm) = &mut self.world.tdm {
                            let state = self.world.game_state.as_ref().unwrap();
                            let mut local = state
                                .local_player_snapshot(self.world.vitals.health > 0)
                                .unwrap();
                            local.origin = origin;
                            local.weapon = strike.weapon;
                            local.angles = strike.angles;
                            let outcome = tdm.player_melee(strike, eye, local, &collision);
                            if let Some(kind) = outcome.committed_feedback {
                                self.hit_feedback.record(through, kind);
                            }
                            tracing::info!(time=through,weapon=strike.weapon,damage=?outcome.damage,
                                feedback=?outcome.committed_feedback,"player melee resolved");
                        }
                    }
                    for shot in shots {
                        if let Some(tdm) = &mut self.world.tdm {
                            let state = self.world.game_state.as_ref().unwrap();
                            let mut local = state
                                .local_player_snapshot(self.world.vitals.health > 0)
                                .unwrap();
                            local.origin = origin;
                            local.weapon = shot.weapon;
                            local.angles = shot.angles;
                            let outcome = if let Some(source) = self.world.armor.source() {
                                tdm.player_fire_from(
                                    shot,
                                    eye,
                                    local,
                                    &collision,
                                    None,
                                    crate::game::modes::tdm::CreditSource::reward(
                                        source.owner,
                                        source.life,
                                    ),
                                )
                            } else {
                                tdm.player_fire(shot, eye, local, &collision, None)
                            };
                            if let Some(kind) = outcome.committed_feedback {
                                self.hit_feedback.record(through, kind);
                                tracing::info!(time = through, ?kind, "player damage feedback");
                            }
                            for impact in outcome.impacts {
                                self.impacts.push(through, shot.weapon, impact);
                            }
                            continue;
                        }
                        let outcome = self.world.combat.as_mut().map_or_else(
                            || crate::game::combat::ShotOutcome {
                                impacts: crate::game::combat::world_impacts(
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
                                .world
                                .combat
                                .as_ref()
                                .is_some_and(|world| world.health() == 0)
                            {
                                codol_protocol::game::snapshot::hit_feedback::HitKind::Kill
                            } else {
                                codol_protocol::game::snapshot::hit_feedback::HitKind::Hit
                            };
                            self.hit_feedback.record(through, kind);
                            tracing::info!(time = through, ?kind, "player damage feedback");
                            if self
                                .world
                                .combat
                                .as_ref()
                                .is_some_and(|world| world.health() == 0)
                                && let Some(bot) = &mut self.world.bot
                            {
                                bot.kill(through);
                                if let Some(state) = self.world.game_state.as_ref()
                                    && let Some(mut enemy) = state.enemy_snapshot(true)
                                {
                                    if let Some(frame) = self.world.bot_frame.as_ref() {
                                        enemy.origin = frame.motion.origin;
                                        enemy.angles = frame.angles;
                                    }
                                    self.death_sequence[1] =
                                        self.death_sequence[1].wrapping_add(1).max(1);
                                    self.corpses.insert(
                                        19,
                                        (through, state.corpse(enemy, self.death_sequence[1])),
                                    );
                                    self.queue_obituary(
                                        through,
                                        shot.weapon,
                                        1,
                                        0,
                                        KillCause {
                                            headshot,
                                            ..Default::default()
                                        },
                                    );
                                }
                            }
                            tracing::info!(
                                shot_time = shot.time,
                                health = self.world.combat.as_ref().map(|combat| combat.health()),
                                location,
                                "training target hit"
                            );
                            self.feedback.push_back(message);
                        }
                    }
                }
            }
            self.input_count += 1;
            if let Some(tdm) = &mut self.world.tdm {
                tdm.validate_strike_selection(
                    input.time,
                    self.world.vitals.health > 0,
                    self.world.player.as_ref().map(|p| p.equipment().weapon),
                );
            }
            if let (Some(tdm), Some(player), Some(resources)) = (
                &self.world.tdm,
                &mut self.world.player,
                self.world
                    .game_state
                    .as_ref()
                    .and_then(|s| s.radar_resources()),
            ) && tdm.streaks.emp_active(tdm.teams()[0], through)
                && resources
                    .radios
                    .iter()
                    .any(|r| r.weapon == player.equipment().weapon)
            {
                player.return_reward();
                let _ = self
                    .server_commands
                    .push(format!("a {}", player.equipment().weapon).as_bytes());
            }
            if let (Some(tdm), Some(player), Some(resources)) = (
                &mut self.world.tdm,
                &mut self.world.player,
                self.world
                    .game_state
                    .as_ref()
                    .and_then(|s| s.radar_resources()),
            ) && self.world.vitals.health > 0
                && !tdm.finished()
                && !self.replay.pending()
                && self.radio_return.is_none()
                && let Some(radio) = resources
                    .radios
                    .iter()
                    // Nuclear stock belongs to the independent extra slot;
                    // its registered activator must reach try_use below.
                    .find(|r| r.native_id != 15 && r.weapon == player.equipment().weapon)
            {
                if crate::game::rewards::mechanisms::Mechanism::marker(radio.native_id) {
                    if !tdm.prepare_airdrop(through, radio.native_id, resources) {
                        player.return_reward();
                    }
                } else if crate::game::rewards::mechanisms::Mechanism::for_reward(radio.native_id)
                    == Some(crate::game::rewards::mechanisms::Mechanism::Remote)
                {
                    let state = self.world.game_state.as_ref().unwrap();
                    let motion = self.world.motion.as_ref().unwrap();
                    let mut command = input.clone();
                    command.weapons[0] = player.equipment().weapon;
                    let eligible =
                        motion.ground_entity != 2047 && player.equipment().offhand_weapon == 0;
                    if !eligible
                        || tdm.remote_action(
                            through,
                            radio.native_id,
                            &command,
                            motion.origin,
                            resources,
                            state.collision.as_ref().unwrap(),
                        ) == Some(false)
                    {
                        player.return_reward();
                        let _ = self
                            .server_commands
                            .push(format!("a {}", player.equipment().weapon).as_bytes());
                    }
                } else if crate::game::rewards::mechanisms::Mechanism::for_reward(radio.native_id)
                    == Some(crate::game::rewards::mechanisms::Mechanism::LocationStrike)
                {
                    let state = self.world.game_state.as_ref().unwrap();
                    if let Some(map) = state.minimap() {
                        let mut command = input.clone();
                        command.weapons[0] = player.equipment().weapon;
                        match tdm.select_strike(
                            radio.native_id,
                            &command,
                            resources,
                            map,
                            state.collision.as_ref().unwrap(),
                        ) {
                            None => {}
                            Some(true) => self.radio_return = Some((through + 50, radio.weapon)),
                            Some(false) => {
                                player.return_reward();
                                let _ = self
                                    .server_commands
                                    .push(format!("a {}", player.equipment().weapon).as_bytes());
                            }
                        }
                    } else {
                        player.return_reward();
                    }
                } else if matches!(radio.native_id, 23 | 24) {
                    let plan = crate::game::rewards::armor::Plan::new(
                        resources,
                        radio.native_id,
                        tdm.teams()[0],
                        player.equipment(),
                    );
                    if !self.world.armor.active()
                        && let Some(plan) = plan
                        && tdm
                            .streaks
                            .try_radio(0, tdm.teams()[0], radio.native_id, through)
                    {
                        player.remove_reward(radio.weapon);
                        self.world.armor.start(
                            plan,
                            crate::game::rewards::strikes::Source {
                                owner: 0,
                                team: tdm.teams()[0],
                                life: tdm.streaks.life(0),
                            },
                            through,
                            player,
                            &mut self.world.vitals,
                        );
                        self.world.health_recovery = Default::default();
                        tdm.mark_combat(0);
                        let _ = self
                            .server_commands
                            .push(format!("a {}", player.equipment().weapon).as_bytes());
                    } else {
                        player.return_reward();
                    }
                } else if radio.native_id == 26 {
                    let state = self.world.game_state.as_ref().unwrap();
                    let motion = self.world.motion.as_ref().unwrap();
                    if tdm.call_drone(
                        through,
                        motion.origin,
                        input.angles,
                        resources,
                        state.collision.as_ref().unwrap(),
                    ) {
                        self.radio_return =
                            Some((through + u32::from(radio.switch.raise_msec), radio.weapon));
                    } else {
                        player.return_reward();
                        let _ = self
                            .server_commands
                            .push(format!("a {}", player.equipment().weapon).as_bytes());
                    }
                } else if crate::game::rewards::mechanisms::Mechanism::device(radio.native_id) {
                    if player.equipment().firing.is_none_or(|f| f.state == 0) {
                        let state = self.world.game_state.as_ref().unwrap();
                        let motion = self.world.motion.as_ref().unwrap();
                        let mut local = state.local_player_snapshot(true).unwrap();
                        local.origin = motion.origin;
                        local.angles = input.angles;
                        if tdm.deploy_device(
                            through,
                            local,
                            motion.view_height + 10.0,
                            radio.native_id,
                            resources,
                            state.collision.as_ref().unwrap(),
                        ) {
                            self.radio_return = Some((through + 50, radio.weapon));
                        } else {
                            player.return_reward();
                            let _ = self
                                .server_commands
                                .push(format!("a {}", player.equipment().weapon).as_bytes());
                            tracing::info!(
                                time = through,
                                "sentry placement rejected; stock retained"
                            );
                        }
                    }
                } else {
                    let state = self.world.game_state.as_ref().unwrap();
                    let origin = self.world.motion.as_ref().unwrap().origin;
                    if tdm.call_radio(
                        through,
                        0,
                        radio.native_id,
                        origin,
                        resources,
                        state.collision.as_ref().unwrap(),
                    ) {
                        self.radio_return =
                            Some((through + u32::from(radio.switch.raise_msec), radio.weapon));
                    } else {
                        player.return_reward();
                        let _ = self
                            .server_commands
                            .push(format!("a {}", player.equipment().weapon).as_bytes());
                    }
                }
            }
            if let (Some(tdm), Some(player), Some(resources)) = (
                &mut self.world.tdm,
                &self.world.player,
                self.world
                    .game_state
                    .as_ref()
                    .and_then(|s| s.nuke_resources()),
            ) && self.world.vitals.health > 0
                && !tdm.finished()
                && !self.replay.pending()
                && player.equipment().weapon == resources.weapon
                && tdm.streaks.try_use(0, through)
            {
                self.nuke_return_at = Some(through + u32::from(resources.switch.raise_msec));
            }
            self.last_input = Some(input);
        }
    }
    pub(super) fn apply_landing(&mut self, landing: crate::game::player::landing::Landing) {
        if self.world.vitals.health == 0 || self.world.tdm.as_ref().is_some_and(|m| m.finished()) {
            return;
        }
        let Some(player) = &mut self.world.player else {
            return;
        };
        if let Some(event) = landing.event {
            player.movement_event(event.id, event.parameter);
        }
        let damage = player.fall_damage(landing.damage);
        if damage == 0 {
            return;
        }
        let now = landing.time;
        self.world.vitals.health = self.world.vitals.health.saturating_sub(damage);
        self.world.vitals.damage_event = self.world.vitals.damage_event.wrapping_add(1);
        self.world.vitals.damage_count = damage.min(127) as u8;
        self.world.health_recovery.damaged_to(
            now,
            self.world.vitals.health,
            self.world.vitals.max_health,
        );
        tracing::info!(
            now,
            raw_damage = landing.damage,
            damage,
            health = self.world.vitals.health,
            "player landing damage"
        );
        if self.world.vitals.health != 0 {
            return;
        }
        self.world.player_death = Some(now);
        self.hit_feedback.clear();
        self.world.tactical.clear();
        let angles = self.view_angles();
        let state = self.world.game_state.as_ref().unwrap();
        let mut local = state.local_player_snapshot(false).unwrap();
        local.origin = self.world.motion.as_ref().unwrap().origin;
        local.angles = angles;
        local.weapon = self.world.player.as_ref().unwrap().equipment().weapon;
        self.world
            .player
            .as_mut()
            .unwrap()
            .drop_offhand_on_death(now, angles);
        if let Some(tdm) = &mut self.world.tdm {
            tdm.actor_fell(now, local);
            self.drain_match_kills(now);
        } else {
            self.death_sequence[0] = self.death_sequence[0].wrapping_add(1).max(1);
            self.corpses
                .insert(20, (now, state.corpse(local, self.death_sequence[0])));
            self.queue_obituary(
                now,
                local.weapon,
                0,
                0,
                KillCause {
                    falling: true,
                    ..Default::default()
                },
            );
        }
        if let Some(motion) = &mut self.world.motion {
            motion.cancel_melee_charge();
            motion.cancel_dive();
            motion.cancel_sprint();
            motion.velocity = [0.0; 3];
        }
    }
}
