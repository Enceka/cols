use super::*;

impl Connection {
    pub(super) fn advance_grenades(&mut self, now: u32) -> Result<()> {
        if self.world.tdm.as_ref().is_none_or(|m| m.finished()) {
            self.publish_airdrop_hint(None)?;
            self.world.scavenger.clear();
            self.world.grenades.clear();
            self.world.smoke.clear();
            self.world.tactical.clear();
            self.world.gas.clear();
            self.world.poison.clear();
            self.world.emp.clear();
            return Ok(());
        }
        let state = self.world.game_state.as_ref().unwrap().clone();
        let collision = state.collision.as_ref().unwrap();
        self.world.smoke.expire(now);
        self.world.gas.expire(now);
        let Some(motion) = self.world.motion.as_ref() else {
            return Ok(());
        };
        let origin = motion.origin;
        if let Some(player) = &mut self.world.player
            && let Some(bag) = self.world.scavenger.collect(
                now,
                origin,
                self.world.vitals.health > 0 && self.replay.kind().is_none(),
                player,
            )
        {
            tracing::info!(time = now, entity = bag.entity_number, origin = ?origin,
                primary_ammo = ?player.equipment().ammo, secondary_ammo = ?player.equipment().secondary,
                "scavenger bag collected");
            self.hit_feedback.record(
                now,
                codol_protocol::game::snapshot::hit_feedback::HitKind::Scavenger,
            );
        }
        let body_height = motion.view_height + 10.0;
        let eye = [origin[0], origin[1], origin[2] + motion.view_height];
        if let Some(player) = &mut self.world.player {
            for request in player.take_throws() {
                let cue = (!request.in_hand && !request.dropped)
                    .then(|| {
                        crate::game::presentation::voice::throw_suffix(&request.definition.name)
                    })
                    .flatten();
                if self
                    .world
                    .grenades
                    .launch(request, eye, motion.velocity, collision)
                    && let Some(cue) = cue
                {
                    self.voice_throws.push_back(cue);
                }
            }
        }
        let angles = self.view_angles();
        let pitch = f32::from(angles[0] as i16) * std::f32::consts::TAU / 65536.0;
        let yaw = f32::from(angles[1]) * std::f32::consts::TAU / 65536.0;
        let tdm = self.world.tdm.as_mut().unwrap();
        let user = crate::game::rewards::airdrops::PickupUser {
            actor: 0,
            life: tdm.streaks.life(0),
            feet: origin,
            eye,
            forward: [
                pitch.cos() * yaw.cos(),
                pitch.cos() * yaw.sin(),
                -pitch.sin(),
            ],
            active: self.world.vitals.health > 0 && self.replay.kind().is_none(),
            holding: self.last_input.as_ref().is_some_and(|cmd| {
                now.saturating_sub(cmd.time) < 200
                    && cmd.buttons & codol_protocol::game::channel::usercmd::BUTTON_USE != 0
            }),
            steal_msec: 8000,
        };
        tdm.pickup_airdrop(now, user, self.world.player.as_mut().unwrap(), collision);
        let hint = tdm.airdrops.hint(0).and_then(|h| match h.contents {
            crate::game::rewards::airdrops::Contents::Reward(id) => Some(id),
            crate::game::rewards::airdrops::Contents::Ammo => None,
        });
        self.publish_airdrop_hint(hint)?;
        let mut local = state
            .local_player_snapshot(self.world.vitals.health > 0)
            .unwrap();
        local.origin = origin;
        local.angles = self.view_angles();
        local.weapon = self.world.player.as_ref().unwrap().equipment().weapon;
        for explosion in self.world.grenades.advance(now, collision) {
            if explosion.definition.marker_reward.is_some() {
                self.world
                    .tdm
                    .as_mut()
                    .unwrap()
                    .airdrops
                    .marker_landed(&explosion, collision);
                continue;
            }
            if explosion.definition.gas {
                let deployed = self.world.gas.deploy(
                    &explosion,
                    0,
                    local.team,
                    collision,
                    &mut self.world.smoke,
                );
                // The thrown gas weapon owns the sound; its null FX pointer
                // needs the separately captured center weapon for the cloud.
                self.impacts.smoke(
                    now,
                    explosion.time,
                    explosion.definition.index,
                    explosion.origin,
                    explosion.normal,
                );
                if deployed {
                    self.impacts.smoke(
                        now,
                        explosion.time,
                        crate::game::status::gas::effect().weapon,
                        explosion.origin,
                        explosion.normal,
                    );
                }
                tracing::info!(now, start=explosion.time, deployed, origin=?explosion.origin,
                    "gas grenade deployed");
                continue;
            }
            if explosion.definition.smoke.is_some() {
                self.world.smoke.spawn(&explosion);
                self.impacts.smoke(
                    now,
                    explosion.time,
                    explosion.definition.index,
                    explosion.origin,
                    explosion.normal,
                );
                tracing::info!(now, start=explosion.time, weapon=explosion.definition.index,
                    origin=?explosion.origin, "smoke grenade deployed");
                continue;
            }
            if explosion.definition.emp {
                self.impacts.custom(
                    now,
                    explosion.time,
                    explosion.definition.index,
                    explosion.origin,
                    explosion.normal,
                );
                let affected = self
                    .world
                    .tdm
                    .as_mut()
                    .unwrap()
                    .player_emp(now, &explosion, local, collision);
                let protected = self.world.player.as_ref().unwrap().emp_protection();
                let applied = local.alive
                    && self.world.emp.apply(
                        now,
                        &explosion,
                        crate::game::status::emp::Target {
                            origin: local.origin,
                            height: body_height,
                            protection: protected,
                        },
                        collision,
                        &mut self.world.tactical,
                    );
                tracing::info!(
                    now,
                    affected,
                    applied,
                    protection = protected,
                    "EMP grenade status"
                );
            }
            if let Some(kind) = explosion.definition.tactical {
                self.impacts.tactical(
                    now,
                    explosion.time,
                    explosion.definition.index,
                    explosion.origin,
                    explosion.normal,
                    kind,
                );
                let affected = self
                    .world
                    .tdm
                    .as_mut()
                    .unwrap()
                    .player_tactical(now, &explosion, local, collision);
                let protection = self
                    .world
                    .player
                    .as_ref()
                    .unwrap()
                    .tactical_protection(kind);
                if local.alive
                    && let Some(hit) = crate::game::status::tactical::hit(
                        &explosion,
                        crate::game::status::tactical::Recipient {
                            feet: local.origin,
                            eye,
                            angles: local.angles,
                        },
                        protection,
                        collision,
                    )
                {
                    self.world.tactical.apply(now, hit);
                    tracing::info!(now,kind=?kind,duration=hit.duration_msec,protection,"player tactical status applied");
                }
                tracing::info!(
                    now,
                    weapon = explosion.definition.index,
                    affected,
                    "tactical grenade detonated"
                );
            } else if !explosion.definition.emp {
                self.impacts.explosion(
                    now,
                    explosion.definition.index,
                    explosion.origin,
                    explosion.normal,
                );
            }
            let (feedback, damage) = self
                .world
                .tdm
                .as_mut()
                .unwrap()
                .player_explosion(now, &explosion, local, collision);
            if let Some(feedback) = feedback {
                self.hit_feedback.record(now, feedback);
            }
            let damage = self.world.player.as_ref().unwrap().explosive_damage(damage);
            tracing::info!(now, weapon=explosion.definition.index, origin=?explosion.origin, damage, feedback=?feedback, "grenade exploded");
            if damage != 0 && self.world.vitals.health > 0 {
                self.world.vitals.health = self.world.vitals.health.saturating_sub(damage);
                self.world.vitals.damage_event = self.world.vitals.damage_event.wrapping_add(1);
                self.world.vitals.damage_count = damage.min(127) as u8;
                self.world.health_recovery.damaged_to(
                    now,
                    self.world.vitals.health,
                    self.world.vitals.max_health,
                );
                if self.world.vitals.health == 0 {
                    self.world.player_death = Some(now);
                    self.hit_feedback.clear();
                    local.alive = false;
                    self.world.tdm.as_mut().unwrap().player_killed(
                        now,
                        local,
                        crate::game::modes::tdm::PlayerHit {
                            reward_life: None,
                            damage,
                            attacker: 0,
                            weapon: explosion.definition.index,
                            headshot: false,
                        },
                    );
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
        }
        let feedback =
            self.world
                .tdm
                .as_mut()
                .unwrap()
                .advance_gas(now, &self.world.gas, collision);
        if let Some(feedback) = feedback {
            self.hit_feedback.record(now, feedback);
        }
        let target = crate::game::status::gas::Recipient {
            client: 0,
            team: local.team,
            origin: local.origin,
            height: body_height,
            alive: local.alive
                && !self.world.tdm.as_ref().unwrap().finished()
                && self.replay.kind().is_none(),
            protected: self.world.player.as_ref().unwrap().gas_protected(),
        };
        if let Some(hit) = self.world.poison.advance(
            now,
            &self.world.gas,
            target,
            collision,
            &mut self.world.tactical,
        ) {
            self.world.vitals.health = self.world.vitals.health.saturating_sub(hit.damage);
            self.world.vitals.damage_event = self.world.vitals.damage_event.wrapping_add(1);
            self.world.vitals.damage_count = hit.damage.min(127) as u8;
            self.world.health_recovery.damaged_to(
                now,
                self.world.vitals.health,
                self.world.vitals.max_health,
            );
            tracing::info!(
                now,
                damage = hit.damage,
                health = self.world.vitals.health,
                attacker = hit.attacker,
                "player gas damage"
            );
            if self.world.vitals.health == 0 {
                self.world.player_death = Some(now);
                self.hit_feedback.clear();
                self.world.tactical.clear();
                self.world.poison.clear();
                self.world.tdm.as_mut().unwrap().player_killed(
                    now,
                    local,
                    crate::game::modes::tdm::PlayerHit {
                        reward_life: None,
                        damage: hit.damage,
                        attacker: hit.attacker,
                        weapon: hit.weapon,
                        headshot: false,
                    },
                );
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
        self.drain_match_kills(now);
        Ok(())
    }
    pub(super) fn advance_props(&mut self, now: u32) -> Result<()> {
        if self.world.tdm.as_ref().is_none_or(|m| m.finished()) {
            return Ok(());
        }
        let state = self.world.game_state.as_ref().unwrap().clone();
        let collision = state.collision.as_ref().unwrap();
        self.world
            .tdm
            .as_mut()
            .unwrap()
            .props
            .advance(now, collision);
        // Each object can explode only once; the bounded queue also covers
        // a chain reaching every supported object in the same frame.
        for _ in 0..=128 {
            let tdm = self.world.tdm.as_mut().unwrap();
            let mut events = tdm.props.drain();
            events.extend(tdm.deployables.drain());
            events.extend(tdm.airspace.drain());
            events.extend(tdm.aircraft.drain());
            events.extend(tdm.sam_missiles.drain());
            events.extend(tdm.strikes.drain());
            events.extend(tdm.remote.drain());
            events.extend(tdm.drones.drain());
            if events.is_empty() {
                break;
            }
            for event in events {
                match event {
                    crate::game::world::props::Event::Rocket {
                        weapon,
                        origin,
                        normal,
                        surface,
                    } => {
                        self.impacts.rocket(now, weapon, origin, normal, surface);
                    }
                    crate::game::world::props::Event::Fx { selector, origin } => {
                        self.impacts.fx(now, selector, origin);
                    }
                    crate::game::world::props::Event::Sound {
                        slot,
                        owner,
                        origin,
                    } => {
                        self.voices.prop_sound(now, owner, origin, slot)?;
                    }
                    crate::game::world::props::Event::Blast(blast) => {
                        let mut local = state
                            .local_player_snapshot(self.world.vitals.health > 0)
                            .unwrap();
                        local.origin = self.world.motion.as_ref().unwrap().origin;
                        local.angles = self.view_angles();
                        local.weapon = self.world.player.as_ref().unwrap().equipment().weapon;
                        let (feedback, hit) = self
                            .world
                            .tdm
                            .as_mut()
                            .unwrap()
                            .prop_blast(now, blast, local, collision);
                        if blast.owner == 0
                            && let Some(feedback) = feedback
                        {
                            self.hit_feedback.record(now, feedback);
                        }
                        if let Some(hit) = hit {
                            let damage = self
                                .world
                                .player
                                .as_ref()
                                .map_or(hit.damage, |p| p.explosive_damage(hit.damage));
                            self.world.vitals.health =
                                self.world.vitals.health.saturating_sub(damage);
                            self.world.vitals.damage_event =
                                self.world.vitals.damage_event.wrapping_add(1);
                            self.world.vitals.damage_count = damage.min(127) as u8;
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
                    }
                }
            }
        }
        self.drain_match_kills(now);
        Ok(())
    }
}

impl Connection {
    fn publish_airdrop_hint(&mut self, id: Option<u8>) -> Result<()> {
        if self.airdrop_hint != id {
            // Captured killstreak_hint resolves ui_ksHint by table C1 and C17.
            // The native 's' command publishes this dvar after initialization.
            let name = id
                .and_then(crate::game::rewards::killstreaks::reward_name)
                .unwrap_or("");
            self.server_commands
                .push(format!("s ui_ksHint \"{name}\"").as_bytes())?;
            self.airdrop_hint = id;
        }
        Ok(())
    }
}
