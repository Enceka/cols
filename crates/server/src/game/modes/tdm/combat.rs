use super::*;

impl TdmMatch {
    pub(crate) fn take_death_weapon(
        &mut self,
        client: u8,
    ) -> Option<crate::game::weapons::held::DroppedWeapon> {
        let bot = self.bots.get_mut(usize::from(client.checked_sub(1)?))?;
        if bot.health != 0 {
            return None;
        }
        bot.weapon.as_mut()?.take_death_weapon()
    }
    pub(super) fn credit(
        &mut self,
        now: u32,
        victim: EnemySnapshot,
        attacker: impl Into<CreditSource>,
        weapon: u16,
        headshot: bool,
        melee: bool,
    ) {
        let source: CreditSource = attacker.into();
        let attacker = source.actor;
        if !self.live_at(now) {
            return;
        }
        if victim.client_index != 0 {
            self.bots[usize::from(victim.client_index - 1)]
                .tactical
                .clear();
            self.bots[usize::from(victim.client_index - 1)]
                .poison
                .clear();
            self.bots[usize::from(victim.client_index - 1)].emp.clear();
        }
        self.rows[usize::from(victim.client_index)].deaths += 1;
        let death = self.rows[usize::from(victim.client_index)].deaths;
        let opposing = self.config.teams[usize::from(attacker)]
            != self.config.teams[usize::from(victim.client_index)];
        let current_life = (opposing
            && source
                .life
                .is_none_or(|life| life == self.streaks.life(attacker)))
        .then_some(attacker);
        if source.life.is_some() {
            self.streaks
                .credit_reward(now, victim.client_index, death, current_life);
        } else {
            self.streaks
                .credit(now, victim.client_index, death, current_life);
        }
        self.rows[usize::from(attacker)].killstreak = u32::from(self.streaks.best(attacker));
        let row = &mut self.rows[usize::from(attacker)];
        let suicide = attacker == victim.client_index;
        row.kills += u32::from(!suicide);
        row.score += if suicide { -100 } else { 100 };
        row.headshots += u32::from(headshot);
        let team = usize::from(self.config.teams[usize::from(attacker)]);
        // Reference war.onNormalDeath awards team points only for opposing
        // player kills. Self-grenade penalties affect the personal row only.
        self.rules.scores[team] += if suicide { 0 } else { 100 };
        if self
            .rules
            .score_reached(team, self.config.score_limit, self.streaks.incoming())
        {
            self.finish(now);
        }
        self.kills.push(Kill {
            falling: false,
            time: now,
            victim,
            attacker,
            weapon,
            headshot,
            melee,
            sequence: self.rows[usize::from(victim.client_index)].deaths,
        });
        self.dirty = true;
        tracing::info!(
            now,
            victim = victim.client_index,
            attacker,
            team,
            score = self.rules.scores[team],
            "TDM kill"
        );
    }
    pub fn player_killed(&mut self, now: u32, victim: EnemySnapshot, hit: PlayerHit) {
        self.credit(
            now,
            victim,
            CreditSource {
                actor: hit.attacker,
                life: hit.reward_life,
            },
            hit.weapon,
            hit.headshot,
            false,
        );
    }
    pub fn actor_fell(&mut self, now: u32, victim: EnemySnapshot) {
        if !self.live_at(now) {
            return;
        }
        // A world fall has no attacker. Reference handleWorldDeath does not
        // apply suicidepointloss or invoke war.onNormalDeath in that case.
        self.rows[usize::from(victim.client_index)].deaths += 1;
        self.streaks.credit(
            now,
            victim.client_index,
            self.rows[usize::from(victim.client_index)].deaths,
            None,
        );
        self.kills.push(Kill {
            falling: true,
            time: now,
            attacker: victim.client_index,
            weapon: victim.weapon,
            headshot: false,
            melee: false,
            sequence: self.rows[usize::from(victim.client_index)].deaths,
            victim,
        });
        self.dirty = true;
        tracing::info!(
            now,
            victim = victim.client_index,
            "TDM environmental fall death"
        );
    }
    pub fn player_tactical(
        &mut self,
        now: u32,
        explosion: &crate::game::weapons::grenade::Explosion,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> usize {
        if !self.live_at(now) {
            return 0;
        }
        let Some(kind) = explosion.definition.tactical else {
            return 0;
        };
        let mut affected = 0;
        for bot in &mut self.bots {
            if bot.health == 0 || bot.snapshot.team == local.team {
                continue;
            }
            let protection = bot
                .weapon
                .as_ref()
                .map_or(1.0, |w| w.tactical_protection(kind));
            if let Some(hit) = crate::game::status::tactical::hit(
                explosion,
                crate::game::status::tactical::Recipient {
                    feet: bot.snapshot.origin,
                    eye: bot.controller.eye(),
                    angles: bot.snapshot.angles,
                },
                protection,
                collision,
            ) {
                affected += usize::from(bot.tactical.apply(now, hit));
            }
        }
        affected
    }
    pub fn player_explosion(
        &mut self,
        now: u32,
        explosion: &crate::game::weapons::grenade::Explosion,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> (
        Option<codol_protocol::game::snapshot::hit_feedback::HitKind>,
        u16,
    ) {
        if !self.live_at(now) {
            return (None, 0);
        }
        self.props.radial(
            now,
            crate::game::world::props::Blast {
                reward_life: None,
                source_entity: None,
                owner: 0,
                weapon: explosion.definition.index,
                origin: explosion.origin,
                radius: f32::from(explosion.definition.radius),
                inner: explosion.definition.inner_damage,
                outer: explosion.definition.outer_damage,
            },
            collision,
        );
        self.deployables.radial(
            now,
            crate::game::world::props::Blast {
                reward_life: None,
                source_entity: None,
                owner: 0,
                weapon: explosion.definition.index,
                origin: explosion.origin,
                radius: f32::from(explosion.definition.radius),
                inner: explosion.definition.inner_damage,
                outer: explosion.definition.outer_damage,
            },
            local.team,
            collision,
        );
        let blast = crate::game::world::props::Blast {
            reward_life: None,
            source_entity: None,
            owner: 0,
            weapon: explosion.definition.index,
            origin: explosion.origin,
            radius: f32::from(explosion.definition.radius),
            inner: explosion.definition.inner_damage,
            outer: explosion.definition.outer_damage,
        };
        for source in self.airspace.radial(now, blast, local.team, collision) {
            self.streaks.stop_flight(source);
        }
        let hits = self
            .bots
            .iter()
            .filter(|bot| bot.health != 0 && bot.snapshot.team != local.team)
            .map(|bot| {
                (
                    bot.snapshot.client_index,
                    crate::game::weapons::grenade::radius_damage(
                        explosion,
                        bot.snapshot.origin,
                        80.0,
                        collision,
                    ),
                    false,
                )
            })
            .filter(|hit| hit.1 != 0)
            .collect::<Vec<_>>();
        let self_damage = if local.alive {
            crate::game::weapons::grenade::radius_damage(explosion, local.origin, 70.0, collision)
        } else {
            0
        };
        (
            self.damage_bots(now, 0, explosion.definition.index, &hits, false),
            self_damage,
        )
    }
    pub fn player_emp(
        &mut self,
        now: u32,
        explosion: &crate::game::weapons::grenade::Explosion,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> usize {
        if !self.live_at(now) {
            return 0;
        }
        let mut affected = 0;
        for bot in &mut self.bots {
            if bot.health == 0
                || bot.snapshot.team == local.team
                || now.wrapping_sub(bot.birth) < 1000
            {
                continue;
            }
            let protection = bot.weapon.as_ref().map_or(1.0, |w| w.emp_protection());
            affected += usize::from(bot.emp.apply(
                now,
                explosion,
                crate::game::status::emp::Target {
                    origin: bot.snapshot.origin,
                    height: bot.view_motion.view_height + 10.0,
                    protection,
                },
                collision,
                &mut bot.tactical,
            ));
        }
        affected
    }
    pub fn prop_blast(
        &mut self,
        now: u32,
        blast: crate::game::world::props::Blast,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> (
        Option<codol_protocol::game::snapshot::hit_feedback::HitKind>,
        Option<PlayerHit>,
    ) {
        if !self.live_at(now) {
            return (None, None);
        }
        let team = self.config.teams[usize::from(blast.owner)];
        let hits = self
            .bots
            .iter()
            .filter(|b| b.health > 0 && b.snapshot.team != team)
            .map(|b| {
                let damage = crate::game::world::props::radius_damage(
                    blast,
                    b.snapshot.origin,
                    40.0,
                    None,
                    collision,
                );
                (
                    b.snapshot.client_index,
                    b.weapon
                        .as_ref()
                        .map_or(damage, |w| w.explosive_damage(damage)),
                    false,
                )
            })
            .filter(|(_, d, _)| *d > 0)
            .collect::<Vec<_>>();
        let feedback = self.damage_bots(
            now,
            CreditSource {
                actor: blast.owner,
                life: blast.reward_life,
            },
            blast.weapon,
            &hits,
            false,
        );
        let local_hit = if local.alive && (blast.owner == 0 || local.team != team) {
            let damage = crate::game::world::props::radius_damage(
                blast,
                local.origin,
                35.0,
                None,
                collision,
            );
            (damage > 0).then_some(PlayerHit {
                reward_life: blast.reward_life,
                damage,
                attacker: blast.owner,
                weapon: blast.weapon,
                headshot: false,
            })
        } else {
            None
        };
        self.props.radial(now, blast, collision);
        self.deployables.radial(now, blast, team, collision);
        self.aircraft.radial(now, blast, team, collision);
        self.remote.radial_cars(blast, team, collision);
        for source in self.airspace.radial(now, blast, team, collision) {
            self.streaks.stop_flight(source);
        }
        (feedback, local_hit)
    }
    pub fn advance_gas(
        &mut self,
        now: u32,
        gas: &crate::game::status::gas::Gas,
        collision: &Collision,
    ) -> Option<codol_protocol::game::snapshot::hit_feedback::HitKind> {
        let mut hits = Vec::new();
        for bot in &mut self.bots {
            let target = crate::game::status::gas::Recipient {
                client: bot.snapshot.client_index,
                team: bot.snapshot.team,
                origin: bot.snapshot.origin,
                height: bot.view_motion.view_height + 10.0,
                alive: !self.rules.finished
                    && bot.health > 0
                    && now.wrapping_sub(bot.birth) >= 1000,
                protected: bot.weapon.as_ref().is_some_and(|w| w.gas_protected()),
            };
            if let Some(hit) = bot
                .poison
                .advance(now, gas, target, collision, &mut bot.tactical)
            {
                hits.push((bot.snapshot.client_index, hit));
            }
        }
        let mut feedback = None;
        for (client, hit) in hits {
            let result = self.damage_bots(
                now,
                hit.attacker,
                hit.weapon,
                &[(client, hit.damage, false)],
                false,
            );
            if result == Some(codol_protocol::game::snapshot::hit_feedback::HitKind::Kill)
                || feedback.is_none()
            {
                feedback = result;
            }
        }
        feedback
    }
    pub fn player_fire(
        &mut self,
        shot: crate::game::weapons::held::Shot,
        eye: [f32; 3],
        local: EnemySnapshot,
        collision: &Collision,
        regions: Option<&crate::game::combat::hit_location::HitRegions>,
    ) -> ActorShot {
        self.player_fire_from(shot, eye, local, collision, regions, 0.into())
    }
    pub(crate) fn player_fire_from(
        &mut self,
        shot: crate::game::weapons::held::Shot,
        eye: [f32; 3],
        local: EnemySnapshot,
        collision: &Collision,
        regions: Option<&crate::game::combat::hit_location::HitRegions>,
        source: CreditSource,
    ) -> ActorShot {
        if !self.live_at(shot.time) {
            return ActorShot::default();
        }
        let mut outcome = game_combat::fire_at_actors(
            eye,
            shot.angles,
            shot.time,
            shot.ballistics,
            (0, self.config.teams[0]),
            game_combat::ActorScene {
                actors: &self.actors(local, 70.0),
                regions,
                collision,
            },
        );
        outcome.committed_feedback =
            self.damage_bots(shot.time, source, shot.weapon, &outcome.damage, false);
        for &(entity, damage) in &outcome.props_damage {
            self.damage_world_model(shot.time, entity, damage, source, false, collision);
        }
        outcome
    }
    pub fn player_melee(
        &mut self,
        strike: crate::game::weapons::held::MeleeStrike,
        eye: [f32; 3],
        local: EnemySnapshot,
        collision: &Collision,
    ) -> ActorShot {
        if !self.live_at(strike.time) || !local.alive {
            return ActorShot::default();
        }
        let mut outcome = game_combat::melee_at_actors(
            eye,
            strike.angles,
            strike.damage,
            (0, self.config.teams[0]),
            game_combat::ActorScene {
                actors: &self.actors(local, 70.0),
                regions: None,
                collision,
            },
        );
        outcome.committed_feedback =
            self.damage_bots(strike.time, 0, strike.weapon, &outcome.damage, true);
        for &(entity, damage) in &outcome.props_damage {
            self.damage_world_model(strike.time, entity, damage, 0, false, collision);
        }
        outcome
    }
    pub fn player_melee_approach(
        &self,
        cmd: &codol_protocol::game::channel::usercmd::UserCommand,
        motion: &crate::game::player::movement::Motion,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> Option<crate::game::weapons::held::MeleeApproach> {
        if !self.live_at(cmd.time)
            || !local.alive
            || cmd.melee_distance == 0
            || cmd.buttons & 4 == 0
            || cmd.buttons & (0x10000000 | 0x200 | 0x100 | 0x400) != 0
            || motion.stance != crate::game::player::movement::Stance::Stand
            || !motion.grounded()
            || motion.melee_charge().is_some()
        {
            return None;
        }
        let eye = [
            motion.origin[0],
            motion.origin[1],
            motion.origin[2] + motion.view_height,
        ];
        let actors = self.actors(local, motion.view_height + 10.0);
        let approach = crate::game::weapons::held::MeleeApproach {
            yaw: cmd.melee_yaw,
            distance: cmd.melee_distance,
        };
        let target = game_combat::melee_approach_at_actors(
            eye,
            cmd.angles,
            approach,
            (0, self.config.teams[0]),
            game_combat::ActorScene {
                actors: &actors,
                regions: None,
                collision,
            },
        )?;
        if cmd.melee_entity != 2047 && cmd.melee_entity != u16::from(target.client) {
            return None;
        }
        if (motion.origin[2] - target.origin[2]).abs() > 18.0 {
            return None;
        }
        Some(approach)
    }
    pub fn take_kills(&mut self) -> Vec<Kill> {
        std::mem::take(&mut self.kills)
    }
}
