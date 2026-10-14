use super::*;

impl TdmMatch {
    /// Marked acceptance can stage ordinary enemy fire while keeping all
    /// actors at their captured starts. Shots use the normal weapon, collision,
    /// damage and replay paths; the diagnostic never writes client state.
    pub(super) fn stationary_shots(
        &mut self,
        now: u32,
        local: EnemySnapshot,
        health: u16,
    ) -> Vec<(u8, u8, crate::game::ai::controller::BotShot)> {
        if health == 0
            || !local.alive
            || self.acceptance_fire_after_msec.is_none_or(|t| {
                self.rules
                    .start
                    .is_none_or(|start| now.saturating_sub(start) < t)
            })
        {
            return Vec::new();
        }
        let Some(bot) = self.bots.iter_mut().find(|b| {
            b.health > 0
                && b.snapshot.team != local.team
                && b.weapon
                    .as_ref()
                    .is_some_and(|w| !w.counter_uav_protected())
        }) else {
            return Vec::new();
        };
        let held = bot.weapon.as_mut().unwrap();
        let eye = [
            bot.snapshot.origin[0],
            bot.snapshot.origin[1],
            bot.snapshot.origin[2] + bot.view_motion.view_height,
        ];
        let angles = crate::game::ai::controller::aim_angles(
            eye,
            [local.origin[0], local.origin[1], local.origin[2] + 40.0],
        );
        let reload = held
            .equipment()
            .ammo
            .is_some_and(|a| a.clip == 0 && a.reserve > 0);
        held.apply(&codol_protocol::game::channel::usercmd::UserCommand {
            time: now,
            angles,
            buttons: if reload {
                16
            } else if (now / 500).is_multiple_of(2) {
                1
            } else {
                0
            },
            weapons: held.equipment().input_weapons(),
            ..Default::default()
        });
        bot.snapshot.angles = angles;
        bot.snapshot.events = held.equipment().events.unwrap();
        bot.snapshot_time = now;
        bot.view_motion.command_time = now;
        held.take_shots()
            .into_iter()
            .map(|s| {
                (
                    bot.snapshot.client_index,
                    bot.snapshot.team,
                    crate::game::ai::controller::BotShot {
                        time: s.time,
                        eye,
                        angles: s.angles,
                        weapon: s.weapon,
                        ballistics: s.ballistics,
                        generation: 0,
                    },
                )
            })
            .collect()
    }
    pub(super) fn resolve_bot_shots(
        &mut self,
        now: u32,
        player: (EnemySnapshot, f32, u16),
        shots: Vec<(u8, u8, crate::game::ai::controller::BotShot)>,
        collision: &Collision,
        tick: &mut Tick,
    ) {
        let (mut local, local_height, local_health) = player;
        let mut player_health = local_health;
        for (attacker, team, shot) in shots {
            if self.rules.finished {
                break;
            }
            if self.bots[usize::from(attacker - 1)].health == 0 {
                continue;
            }
            let outcome = game_combat::fire_at_actors(
                shot.eye,
                shot.angles,
                now,
                shot.ballistics,
                (attacker, team),
                game_combat::ActorScene {
                    actors: &self.actors(local, local_height),
                    regions: None,
                    collision,
                },
            );
            let mut feedback = self.damage_bots(now, attacker, shot.weapon, &outcome.damage, false);
            for &(entity, damage) in &outcome.props_damage {
                self.damage_world_model(now, entity, damage, attacker, false, collision);
            }
            for &(victim, damage, headshot) in &outcome.damage {
                if victim == 0 && player_health > 0 {
                    player_health = player_health.saturating_sub(damage);
                    if player_health == 0 {
                        feedback =
                            Some(codol_protocol::game::snapshot::hit_feedback::HitKind::Kill);
                    } else {
                        feedback.get_or_insert(
                            codol_protocol::game::snapshot::hit_feedback::HitKind::Hit,
                        );
                    }
                    tick.hits.push(PlayerHit {
                        damage,
                        attacker,
                        weapon: shot.weapon,
                        headshot,
                    });
                    local.alive = player_health > 0;
                }
            }
            if let Some(kind) = feedback {
                self.bots[usize::from(attacker - 1)].feedback = Some((now, kind));
            }
            tick.impacts.extend(
                outcome
                    .impacts
                    .into_iter()
                    .map(|impact| (shot.weapon, impact)),
            );
        }
    }
    #[cfg(test)]
    pub fn step(
        &mut self,
        now: u32,
        player: (EnemySnapshot, f32, u16),
        weapon: BotWeapon,
        collision: &Collision,
        animate: impl Fn(
            EnemySnapshot,
            &crate::game::ai::controller::BotFrame,
            bool,
            bool,
        ) -> EnemySnapshot,
    ) -> Result<Tick> {
        self.step_with_smoke(
            now,
            player,
            weapon,
            &crate::game::weapons::smoke::Smoke::default(),
            collision,
            animate,
        )
    }
    pub fn step_with_smoke(
        &mut self,
        now: u32,
        player: (EnemySnapshot, f32, u16),
        weapon: BotWeapon,
        smoke: &crate::game::weapons::smoke::Smoke,
        collision: &Collision,
        animate: impl Fn(
            EnemySnapshot,
            &crate::game::ai::controller::BotFrame,
            bool,
            bool,
        ) -> EnemySnapshot,
    ) -> Result<Tick> {
        let started = std::time::Instant::now();
        let mut movement = std::time::Duration::ZERO;
        let (local, local_height, local_health) = player;
        let mut tick = Tick::default();
        self.advance_clock(now);
        if self.rules.finished {
            return Ok(tick);
        }
        for i in 0..self.bots.len() {
            if self.bots[i]
                .death
                .is_some_and(|death| now.wrapping_sub(death) >= 3000)
            {
                let team = usize::from(self.bots[i].snapshot.team - 1);
                let starts = &self.config.spawns[team];
                let index = (i + self.rows[i + 1].deaths as usize) % starts.len();
                let origin = if self.acceptance_stationary {
                    self.baselines[i].origin
                } else {
                    crate::game::ai::controller::grounded_spawn(starts[index], collision)?
                };
                let bot = &mut self.bots[i];
                let angles = if self.acceptance_stationary {
                    self.baselines[i].angles
                } else {
                    self.config
                        .spawn_angles
                        .as_ref()
                        .map_or(bot.snapshot.angles, |angles| angles[team][index])
                };
                bot.controller.respawn(now, origin, angles)?;
                bot.snapshot.angles = angles;
                bot.snapshot.origin = origin;
                bot.snapshot.alive = true;
                bot.snapshot.events = EventQueue::default();
                bot.health = 100;
                self.streaks.spawn(bot.snapshot.client_index);
                bot.damage_event = 0;
                bot.damage_count = 0;
                bot.feedback = None;
                bot.birth = now;
                bot.death = None;
                bot.fire_time = None;
                bot.reload_animation = Default::default();
                bot.route.clear();
                bot.route_target = None;
                bot.steering = None;
                bot.snapshot_time = now;
                bot.weapon = bot.initial_weapon.clone();
                bot.tactical.clear();
                bot.poison.clear();
                bot.emp.clear();
                bot.memory = None;
                bot.visible_target = None;
                bot.perception_time = 0;
                bot.progress_origin = origin;
                bot.progress_time = now;
                bot.recent_route.clear();
                bot.stuck_level = 0;
            }
        }
        // Isolated acceptance keeps ordinary death/respawn and damage rules,
        // while the bots stay at their configured starts and never attack.
        if self.acceptance_stationary {
            let shots = self.stationary_shots(now, local, local_health);
            self.resolve_bot_shots(now, player, shots, collision, &mut tick);
            return Ok(tick);
        }
        let mut shots = Vec::new();
        let mut falling_deaths = Vec::new();
        let actors = self.actors(local, local_height);
        let peers: Vec<_> = actors.iter().map(|a| (a.client, a.origin)).collect();
        let report = now >= self.next_diagnostics;
        if report {
            self.next_diagnostics = now + 5000;
        }
        for i in 0..self.bots.len() {
            if self.bots[i].health == 0 {
                continue;
            }
            let origin = self.bots[i].snapshot.origin;
            let team = self.bots[i].snapshot.team;
            let occupied: Vec<_> = self.bots.iter().filter_map(|b| b.route_target).collect();
            let bot = &mut self.bots[i];
            bot.controller
                .separate_from(bot.snapshot.client_index, &peers, collision);
            // Acquire only living opponents in view, with a close awareness
            // radius. Occluded actors never update last-known coordinates.
            if now.wrapping_sub(bot.perception_time) >= 150 {
                bot.perception_time = now;
                let found = (!bot.tactical.blind(now))
                    .then(|| {
                        visible_enemy(
                            origin,
                            bot.snapshot.angles[1],
                            team,
                            &actors,
                            now,
                            smoke,
                            collision,
                        )
                    })
                    .flatten();
                bot.visible_target = found.map(|(_, t)| t);
                if let Some((client, target)) = found {
                    bot.memory = Some((client, target, now));
                }
            }
            if bot.memory.is_some_and(|(client, _, time)| {
                now.wrapping_sub(time) > 10_000 || !actors.iter().any(|a| a.client == client)
            }) {
                bot.memory = None;
                bot.visible_target = None;
            }
            let target = bot
                .visible_target
                .filter(|_| !bot.tactical.blind(now))
                .filter(|t| smoke.visible(now, bot.controller.eye(), t.eye));
            let remembered = bot
                .memory
                .map(|(_, t, _)| t.origin)
                .filter(|_| bot.stuck_level < 2);
            if bot
                .recent_route
                .back()
                .is_none_or(|&p| distance(origin, p) > 48.0 * 48.0)
            {
                bot.recent_route.push_back(origin);
                if bot.recent_route.len() > 32 {
                    bot.recent_route.pop_front();
                }
            }
            if i == self.next_route_client && !bot.controller.traversing() {
                let stuck = now.wrapping_sub(bot.progress_time) >= 2500
                    && distance(origin, bot.progress_origin) < 24.0 * 24.0
                    && !bot.route.is_empty()
                    && bot.controller.has_move_intent();
                if now.wrapping_sub(bot.progress_time) >= 2500 {
                    if distance(origin, bot.progress_origin) >= 96.0 * 96.0 {
                        bot.stuck_level = 0;
                    }
                    bot.progress_origin = origin;
                    bot.progress_time = now;
                }
                if stuck {
                    bot.stuck_level = bot.stuck_level.saturating_add(1);
                    bot.route.clear();
                    bot.route_target = None;
                    bot.steering = None;
                }
                let arrived = bot
                    .route_target
                    .is_some_and(|p| distance(origin, p) < 96.0 * 96.0);
                let chasing = remembered.is_some_and(|p| {
                    bot.route_target
                        .is_none_or(|q| distance(p, q) > 192.0 * 192.0)
                });
                if bot.route_target.is_none()
                    || arrived
                    || bot.route.is_empty()
                    || (chasing && now.wrapping_sub(bot.route_time) >= 2000)
                {
                    let goal = remembered
                        .filter(|_| bot.stuck_level < 2)
                        .filter(|p| distance(origin, *p) > 96.0 * 96.0)
                        .or_else(|| {
                            self.navigation
                                .roam_goal(origin, &occupied, &mut bot.random)
                        });
                    if let Some(goal) = goal {
                        let seed = crate::game::ai::navigation::next_random(&mut bot.random);
                        let recent: Vec<_> = bot.recent_route.iter().copied().collect();
                        bot.route = self
                            .navigation
                            .route_varied(origin, goal, collision, seed, &recent)
                            .into();
                        bot.route_target = (!bot.route.is_empty()).then_some(goal);
                        bot.route_time = now;
                        bot.steering = None;
                        tracing::debug!(client=bot.snapshot.client_index, now, seed,
                            origin=?origin, goal=?goal, nodes=bot.route.len(),
                            jumps=bot.route.iter().filter(|p| p.jump_from.is_some()).count(),
                            stuck_level=bot.stuck_level, "TDM bot route planned");
                    }
                }
            }
            let move_started = std::time::Instant::now();
            if bot.steering.is_none()
                || now.wrapping_sub(bot.steering_time) >= 250
                || bot
                    .steering
                    .is_some_and(|p| distance(origin, p.origin) < 36.0 * 36.0)
            {
                bot.steering = self.navigation.steering(origin, &mut bot.route, collision);
                bot.steering_time = now;
            }
            let active_weapon = bot
                .weapon
                .as_ref()
                .and_then(|w| w.bot_weapon())
                .unwrap_or(weapon);
            let class = self.config.roster.get(i);
            let range = active_weapon
                .ballistics
                .range
                .min(if active_weapon.ballistics.pellets > 1 {
                    active_weapon.ballistics.range.min(750.0)
                } else {
                    1500.0
                })
                .max(96.0);
            let spacing = range
                * match class.map(|c| c.personality.as_str()) {
                    Some("camper") => 0.7,
                    Some("run_and_gun") => 0.25,
                    _ => 0.5,
                };
            bot.controller.set_match_range(range, spacing);
            let robot = bot.weapon.as_ref().is_some_and(|w| w.asym_robot());
            bot.controller.set_tactical_control(
                bot.tactical.move_scale(now, robot),
                bot.tactical.turn_limit(now),
            );
            // A pane on a real walking route is an aim target, not permission
            // to move through it. The held weapon still owns cadence and ammo;
            // only its committed bullet can change glass collision.
            let target = target.or_else(|| {
                if bot.tactical.blind(now) {
                    return None;
                }
                let waypoint = bot.steering?.origin;
                let eye = bot.controller.eye();
                let end = [waypoint[0], waypoint[1], waypoint[2] + 60.0];
                let hit = collision.raycast(eye, end);
                if hit.start_solid || hit.all_solid || hit.glass_piece.is_none() {
                    return None;
                }
                let point: [f32; 3] =
                    std::array::from_fn(|axis| eye[axis] + (end[axis] - eye[axis]) * hit.fraction);
                if distance(eye, point) > 192.0 * 192.0 {
                    return None;
                }
                Some(BotTarget {
                    origin: [point[0], point[1], origin[2]],
                    eye: point,
                    alive: true,
                })
            });
            let was_traversing = bot.controller.traversing();
            let previous_jumps = bot.controller.jump_count();
            let mut frame =
                bot.controller
                    .step_route(now, target, bot.steering, active_weapon, collision);
            if was_traversing
                || bot.controller.traversing()
                || previous_jumps != bot.controller.jump_count()
            {
                tracing::debug!(client=bot.snapshot.client_index, now,
                    origin=?frame.motion.origin, velocity=?frame.motion.velocity,
                    grounded=frame.motion.grounded(), traversing=bot.controller.traversing(),
                    jumps=bot.controller.jump_count(), waypoint=?bot.steering,
                    "TDM bot traversal sample");
            }
            for landing in &frame.landings {
                if let Some(event) = landing.event {
                    if let Some(held) = &mut bot.weapon {
                        held.movement_event(event.id, event.parameter);
                    } else {
                        bot.snapshot.events.push(event.id, event.parameter)?;
                    }
                }
                let damage = bot
                    .weapon
                    .as_ref()
                    .map_or(landing.damage, |w| w.fall_damage(landing.damage));
                if damage == 0 {
                    continue;
                }
                bot.health = bot.health.saturating_sub(damage);
                bot.damage_event = bot.damage_event.wrapping_add(1);
                bot.damage_count = damage.min(127) as u8;
                tracing::info!(
                    client = bot.snapshot.client_index,
                    time = landing.time,
                    raw_damage = landing.damage,
                    damage,
                    health = bot.health,
                    "Bot landing damage"
                );
                if bot.health == 0 {
                    bot.snapshot.origin = frame.motion.origin;
                    bot.snapshot.angles = frame.angles;
                    bot.snapshot.alive = false;
                    bot.snapshot.weapon = active_weapon.weapon;
                    bot.view_motion = frame.motion.clone();
                    bot.death = Some(landing.time);
                    bot.controller.kill(now);
                    bot.tactical.clear();
                    bot.poison.clear();
                    bot.emp.clear();
                    falling_deaths.push((landing.time, bot.snapshot));
                    break;
                }
            }
            if bot.health == 0 {
                movement += move_started.elapsed();
                continue;
            }
            if let Some(held) = &mut bot.weapon {
                let previous_weapon_state = held.equipment().firing.map(|f| f.state);
                let ammo = held.equipment().ammo;
                let reload = ammo.is_some_and(|a| a.clip == 0 && a.reserve > 0);
                held.spread_environment(&frame.motion);
                let command = codol_protocol::game::channel::usercmd::UserCommand {
                    time: now,
                    angles: frame.angles,
                    buttons: if reload {
                        16
                    } else if frame.shot.is_some() {
                        1 | 0x800
                    } else {
                        0
                    },
                    weapons: held.equipment().input_weapons(),
                    ..Default::default()
                };
                held.apply(&command);
                let equipment = held.equipment();
                if previous_weapon_state != equipment.firing.map(|f| f.state) {
                    tracing::debug!(client=bot.snapshot.client_index, now, weapon=equipment.weapon,
                        firing=?equipment.firing, needs_rechamber=equipment.needs_rechamber,
                        "TDM bot weapon state");
                }
                let shots = held.take_shots();
                frame.shot = shots.first().map(|s| crate::game::ai::controller::BotShot {
                    time: s.time,
                    eye: [
                        frame.motion.origin[0],
                        frame.motion.origin[1],
                        frame.motion.origin[2] + frame.motion.view_height,
                    ],
                    angles: s.angles,
                    weapon: s.weapon,
                    ballistics: s.ballistics,
                    generation: 0,
                });
                if let Some(events) = held.equipment().events {
                    bot.snapshot.events = events;
                }
            }
            // Include crowd separation in the body's animated velocity.
            let delta = now.saturating_sub(bot.snapshot_time).clamp(1, 200) as f32 * 0.001;
            for axis in 0..2 {
                frame.motion.velocity[axis] =
                    (frame.motion.origin[axis] - bot.snapshot.origin[axis]) / delta;
            }
            bot.snapshot_time = now;
            bot.view_motion = frame.motion.clone();
            movement += move_started.elapsed();
            if report {
                tracing::info!(client=bot.snapshot.client_index,team,health=bot.health,phase=?frame.phase,origin=?frame.motion.origin,target=?bot.route_target,route_nodes=bot.route.len(),"TDM bot state");
            }
            if let Some(shot) = frame.shot {
                tracing::debug!(
                    client = bot.snapshot.client_index,
                    now,
                    weapon = shot.weapon,
                    "TDM bot weapon shot"
                );
                bot.fire_time = Some(now);
                bot.fire_toggle = !bot.fire_toggle;
                if bot.weapon.is_none() {
                    bot.snapshot
                        .events
                        .push(codol_protocol::game::snapshot::events::FIRE_WEAPON, 0)?;
                    bot.snapshot
                        .events
                        .push(codol_protocol::game::snapshot::events::EJECT_BRASS, 0)?;
                }
                shots.push((bot.snapshot.client_index, team, shot));
            }
            let events = bot.snapshot.events;
            let firing_duration = class
                .and_then(|c| c.animation.fire_animation_msec)
                .map_or(150, u32::from);
            let firing = bot
                .fire_time
                .is_some_and(|t| now.wrapping_sub(t) < firing_duration);
            bot.snapshot = match class {
                Some(class) => {
                    crate::game::session::prepared::PreparedGameState::animate_with_config(
                        &class.animation,
                        bot.snapshot,
                        &frame,
                        firing,
                        bot.fire_toggle,
                    )
                }
                None => animate(bot.snapshot, &frame, firing, bot.fire_toggle),
            };
            bot.snapshot.events = events;
            bot.snapshot.weapon = active_weapon.weapon;
            if bot.reload_animation.apply(
                &mut bot.snapshot,
                class.and_then(|c| c.animation.reload_animation),
                bot.weapon.as_ref().map(|w| w.equipment()),
            ) {
                tracing::info!(
                    client = bot.snapshot.client_index,
                    weapon = bot.snapshot.weapon,
                    torso = bot.snapshot.torso_animation,
                    "TDM bot reload pose started"
                );
            }
        }
        self.next_route_client = (self.next_route_client + 1) % self.bots.len();
        for (time, victim) in falling_deaths {
            self.actor_fell(time, victim);
        }
        let routing = started.elapsed().saturating_sub(movement);
        let damage_started = std::time::Instant::now();
        self.resolve_bot_shots(now, player, shots, collision, &mut tick);
        if started.elapsed().as_millis() > 25 {
            tracing::warn!(
                route_msec = routing.as_millis(),
                movement_msec = movement.as_millis(),
                damage_msec = damage_started.elapsed().as_millis(),
                "Bot frame work profile"
            );
        }
        Ok(tick)
    }
}
