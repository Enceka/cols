use super::*;

impl TdmMatch {
    pub(crate) fn prepare_airdrop(
        &mut self,
        now: u32,
        id: u8,
        resources: &crate::game::rewards::killstreaks::RadarResources,
    ) -> bool {
        if self.rules.finished {
            return false;
        }
        if let Some((old, _, _)) = self.marker_reservation {
            return old == id;
        }
        let team = self.config.teams[0];
        let Some(ticket) = self.streaks.reserve_radio(0, team, id, now) else {
            return false;
        };
        let Some(entity) = self.airdrops.reserve(resources, id, 0, team) else {
            self.streaks.cancel_radio(0, ticket);
            return false;
        };
        self.marker_reservation = Some((id, entity, ticket));
        true
    }
    pub(crate) fn cancel_airdrop(&mut self) {
        if let Some((_, entity, ticket)) = self.marker_reservation.take() {
            self.airdrops.cancel(entity);
            self.streaks.cancel_radio(0, ticket);
        }
    }
    pub(crate) fn launch_airdrop_marker(&mut self, time: u32, id: u8) -> bool {
        let Some((selected, entity, ticket)) = self.marker_reservation.take() else {
            return false;
        };
        if selected != id
            || !self
                .streaks
                .commit_radio(0, self.config.teams[0], ticket, time)
        {
            self.airdrops.cancel(entity);
            self.streaks.cancel_radio(0, ticket);
            return false;
        }
        assert!(self.airdrops.launch(entity, time));
        true
    }

    pub(crate) fn pickup_airdrop(
        &mut self,
        now: u32,
        user: crate::game::rewards::airdrops::PickupUser,
        player: &mut crate::game::weapons::held::HeldWeapons,
        collision: &Collision,
    ) {
        let Some(claim) = self.airdrops.pickup(now, user, collision) else {
            return;
        };
        self.airdrops
            .claim(claim, collision, |contents| match contents {
                crate::game::rewards::airdrops::Contents::Reward(id) => {
                    self.streaks.pickup_reward(user.actor, id, now)
                }
                crate::game::rewards::airdrops::Contents::Ammo => {
                    player.refill_supply();
                    true
                }
            });
    }
    pub(crate) fn world_models(&self) -> Vec<codol_protocol::game::snapshot::props::PropSnapshot> {
        let mut models = self.props.snapshots();
        models.extend(self.deployables.snapshots());
        models.extend(self.airspace.snapshots());
        models.extend(self.aircraft.snapshots());
        models.extend(self.remote.props());
        models.extend(self.dogs.snapshots());
        models.extend(self.strikes.planes(self.last_world_time));
        models.extend(
            self.airdrops
                .snapshots(self.config.teams[0], self.last_world_time),
        );
        models.sort_by_key(|p| p.entity_number);
        models
    }
    pub(crate) fn acceptance_enemy_support(
        &mut self,
        now: u32,
        diagnostics: &crate::game::rewards::killstreaks::NukeResources,
        resources: &crate::game::rewards::killstreaks::RadarResources,
        collision: &Collision,
    ) {
        if !self.acceptance_stationary || !diagnostics.acceptance_stationary || self.rules.finished
        {
            return;
        }
        let Some(start) = self.rules.start else {
            return;
        };
        let Some(request) = diagnostics
            .acceptance_enemy_support
            .get(self.acceptance_support_index)
        else {
            return;
        };
        if now.saturating_sub(start) < request.after_msec {
            return;
        }
        if let Some(delay) = request.gunner_delay_msec
            && self
                .remote
                .gunner_started_at()
                .is_none_or(|began| now.saturating_sub(began) < delay)
        {
            return;
        }
        let Some(bot) = self
            .bots
            .iter()
            .find(|b| b.snapshot.alive && b.snapshot.team != self.config.teams[0])
        else {
            return;
        };
        let actor = bot.snapshot.client_index;
        let origin = bot.snapshot.origin;
        if !self
            .streaks
            .owned_radios(actor)
            .any(|(_, id)| id == request.native_id)
        {
            self.streaks
                .acceptance_supply_support(actor, request.native_id);
        }
        if self.call_radio(now, actor, request.native_id, origin, resources, collision) {
            self.acceptance_support_index += 1;
            tracing::info!(
                now,
                actor,
                id = request.native_id,
                "diagnostic enemy support called through ordinary support path"
            );
        }
    }
    pub(crate) fn call_radio(
        &mut self,
        now: u32,
        actor: u8,
        id: u8,
        origin: [f32; 3],
        resources: &crate::game::rewards::killstreaks::RadarResources,
        collision: &Collision,
    ) -> bool {
        if actor >= self.config.player_count {
            return false;
        }
        let team = self.config.teams[usize::from(actor)];
        if self.rules.finished || !self.streaks.can_radio(actor, team, id, now) {
            return false;
        }
        if id == 20 {
            let Some(plan) = self.dogs.plan(resources, origin, team, collision) else {
                return false;
            };
            if !self.streaks.try_radio(actor, team, id, now) {
                return false;
            }
            self.dogs.launch(
                now,
                plan,
                crate::game::rewards::strikes::Source {
                    owner: actor,
                    team,
                    life: self.streaks.life(actor),
                },
            );
            return true;
        }
        if matches!(id, 8 | 10) {
            self.aircraft.reserve_slots(self.remote.vehicle_slots());
            let Some(plan) = self.aircraft.plan(resources, id, collision) else {
                return false;
            };
            if !self.streaks.try_radio(actor, team, id, now) {
                return false;
            }
            self.aircraft.launch(
                now,
                plan,
                crate::game::rewards::aircraft::Source {
                    owner: actor,
                    team,
                    life: self.streaks.life(actor),
                    serial: now,
                },
                collision,
            );
            return true;
        }
        let plan = if matches!(id, 1 | 3)
            && crate::game::rewards::airspace::Airspace::resources_present(resources, id)
        {
            match self.airspace.plan_radar(resources, id, origin, collision) {
                Ok(plan) => Some(plan),
                Err(reason) => {
                    tracing::info!(
                        actor,
                        id,
                        ?reason,
                        "air support call rejected; stock retained"
                    );
                    return false;
                }
            }
        } else {
            None
        };
        if !self.streaks.try_radio(actor, team, id, now) {
            return false;
        }
        if let Some(plan) = plan {
            let source = self
                .streaks
                .support_flights(now)
                .into_iter()
                .find(|s| s.actor == actor && s.id == id && s.start == now)
                .unwrap();
            self.airspace.launch(plan, source, collision);
        }
        if id == 14 {
            self.aircraft.emp(now, team);
            for source in self.airspace.emp(now, team, collision) {
                self.streaks.stop_flight(source);
            }
        }
        true
    }
    pub(crate) fn deploy_device(
        &mut self,
        now: u32,
        local: EnemySnapshot,
        height: f32,
        id: u8,
        resources: &crate::game::rewards::killstreaks::RadarResources,
        collision: &Collision,
    ) -> bool {
        if self.rules.finished
            || !local.alive
            || !matches!(id, 16 | 21)
            || !self.streaks.can_radio(0, local.team, id, now)
        {
            return false;
        }
        let actors = self.actors(local, height);
        let Some(player) = actors.iter().find(|a| a.client == 0).copied() else {
            return false;
        };
        let planned = if id == 21 {
            self.deployables.plan_sam(resources, player, collision)
        } else {
            self.deployables.plan_sentry(resources, player, collision)
        };
        let placement = match planned {
            Ok(placement) => placement,
            Err(reason) => {
                tracing::info!(now,?reason,origin=?local.origin,yaw=player.yaw,"sentry placement unavailable");
                return false;
            }
        };
        if !self.deployables.place(
            placement,
            crate::game::rewards::deployables::Source {
                owner: 0,
                team: local.team,
                life: self.streaks.life(0),
            },
            now,
            &actors,
            collision,
        ) {
            return false;
        }
        assert!(self.streaks.try_radio(0, local.team, id, now));
        true
    }
    pub(crate) fn step_deployables(
        &mut self,
        now: u32,
        player: (EnemySnapshot, f32, u16),
        local_undetectable: bool,
        collision: &Collision,
    ) -> Tick {
        self.last_world_time = now;
        self.airdrops.advance(now, self.rules.finished, collision);
        self.aircraft.advance(now, self.rules.finished, collision);
        let (mut local, height, health) = player;
        self.airspace.advance(
            now,
            self.rules.finished,
            &self.streaks.support_flights(now),
            collision,
        );
        let protected = self
            .bots
            .iter()
            .filter(|b| b.weapon.as_ref().is_some_and(|w| w.sentry_undetectable()))
            .fold(u16::from(local_undetectable), |mask, b| {
                mask | (1 << b.snapshot.client_index)
            });
        let emp = (1..=2)
            .filter(|&team| self.streaks.emp_active(team, now))
            .fold(0, |mask, team| mask | (1 << team));
        let mut targets = self.airspace.targets();
        targets.extend(self.aircraft.targets());
        targets.extend(self.remote.gunner_targets());
        for hit in self
            .sam_missiles
            .advance(now, self.rules.finished, &targets, collision)
        {
            if (crate::game::rewards::aircraft::FIRST_ENTITY
                ..crate::game::rewards::aircraft::END_ENTITY)
                .contains(&hit.entity)
            {
                self.aircraft.damage(
                    now,
                    hit.entity,
                    hit.damage,
                    (hit.owner, self.config.teams[usize::from(hit.owner)]),
                    true,
                    collision,
                );
            } else {
                self.damage_world_model(now, hit.entity, hit.damage, hit.owner, false, collision);
            }
        }
        let mut targets = self.airspace.targets();
        targets.extend(self.aircraft.targets());
        targets.extend(self.remote.gunner_targets());
        if !self.rules.finished {
            let launches = self.deployables.sam_launches(
                now,
                &targets,
                emp,
                self.sam_missiles.capacity(),
                collision,
            );
            for launch in launches {
                let target = launch.target;
                assert!(self.sam_missiles.launch(now, launch));
                if self.aircraft.decoy(target) {
                    self.sam_missiles.divert(target);
                }
            }
            let launches =
                self.aircraft
                    .launches(now, &targets, emp, self.sam_missiles.capacity(), collision);
            for launch in launches {
                let target = launch.target;
                assert!(self.sam_missiles.launch(now, launch));
                if self.aircraft.decoy(target) {
                    self.sam_missiles.divert(target);
                }
            }
        }
        let shots = self.deployables.step(
            now,
            &self.actors(local, height),
            protected,
            emp,
            self.rules.finished,
            collision,
        );
        let mut tick = Tick::default();
        let mut local_health = health;
        for shot in shots {
            if self.rules.finished {
                break;
            }
            let outcome = crate::game::rewards::deployables::resolve_shot(
                &shot,
                now,
                &self.actors(local, height),
                collision,
            );
            let feedback = self.damage_bots(
                now,
                CreditSource::reward(shot.owner, shot.life),
                shot.weapon,
                &outcome.damage,
                false,
            );
            if shot.owner > 0
                && let Some(kind) = feedback
            {
                self.bots[usize::from(shot.owner - 1)].feedback = Some((now, kind));
            }
            for &(entity, damage) in &outcome.props_damage {
                self.damage_world_model(
                    now,
                    entity,
                    damage,
                    CreditSource::reward(shot.owner, shot.life),
                    false,
                    collision,
                );
            }
            for &(victim, damage, headshot) in &outcome.damage {
                if victim == 0 && local_health > 0 {
                    local_health = local_health.saturating_sub(damage);
                    local.alive = local_health > 0;
                    tick.hits.push(PlayerHit {
                        reward_life: Some(shot.life),
                        damage,
                        attacker: shot.owner,
                        weapon: shot.weapon,
                        headshot,
                    });
                }
            }
            tick.impacts.extend(
                outcome
                    .impacts
                    .into_iter()
                    .map(|impact| (shot.weapon, impact)),
            );
        }
        let strikes = self.advance_strikes(now, local, height, local_health, emp, collision);
        tick.hits.extend(strikes.hits);
        tick.impacts.extend(strikes.impacts);
        tick
    }
    pub fn finish_nuke(&mut self, now: u32, owner: u8, weapon: u16, local: EnemySnapshot) {
        if self.rules.finished {
            return;
        }
        let team = self.config.teams[usize::from(owner)];
        let victims: Vec<_> = std::iter::once(local)
            .filter(|s| s.alive)
            .chain(
                self.bots
                    .iter()
                    .filter(|b| b.health > 0)
                    .map(|b| b.snapshot),
            )
            .collect();
        for victim in victims {
            let index = usize::from(victim.client_index);
            self.rows[index].deaths += 1;
            self.streaks
                .credit(now, victim.client_index, self.rows[index].deaths, None);
            if victim.team != team {
                self.rows[usize::from(owner)].kills += 1;
                self.rows[usize::from(owner)].score += 100;
                self.rules.scores[usize::from(team)] += 100;
            }
            if index != 0 {
                let bot = &mut self.bots[index - 1];
                bot.health = 0;
                bot.death = Some(now);
                bot.snapshot.alive = false;
            }
            self.kills.push(Kill {
                falling: false,
                time: now,
                victim,
                attacker: owner,
                weapon,
                headshot: false,
                melee: false,
                sequence: self.rows[index].deaths,
            });
        }
        self.rules.forced_winner = Some(team);
        self.streaks.clear_owned();
        self.finish(now);
    }
    pub fn restore_nuke_rewards(
        &mut self,
        resources: &crate::game::rewards::killstreaks::NukeResources,
    ) {
        for bot in &mut self.bots {
            if bot.health > 0
                && self.streaks.owned(bot.snapshot.client_index)
                && !self.rules.finished
                && let Some(weapon) = &mut bot.weapon
            {
                weapon.grant_nuke(resources);
            }
        }
    }
}
