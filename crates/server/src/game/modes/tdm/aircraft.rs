use super::*;
use crate::game::rewards::aircraft::TargetActor;

impl TdmMatch {
    pub(crate) fn step_aircraft(
        &mut self,
        now: u32,
        local: (EnemySnapshot, f32, u32, [bool; 2]),
        c: &Collision,
    ) -> Tick {
        let mut tick = Tick::default();
        if !self.live_at(now) {
            return tick;
        }
        let (player, height, born, hidden) = local;
        let actors = self.actors(player, height);
        let targets = actors
            .iter()
            .map(|&actor| {
                let client = actor.client;
                let (born, hidden) = if client == 0 {
                    (born, hidden)
                } else {
                    let b = &self.bots[usize::from(client - 1)];
                    (
                        b.birth,
                        b.weapon.as_ref().map_or([false; 2], |w| {
                            [w.helicopter_undetectable(), w.airplane_undetectable()]
                        }),
                    )
                };
                TargetActor {
                    actor,
                    born: born.max(self.rules.start.unwrap_or(born)),
                    hidden: hidden[0],
                    airplane_hidden: hidden[1],
                    life: self.streaks.life(client),
                    score: self.rows[usize::from(client)].score,
                }
            })
            .collect::<Vec<_>>();
        let emp = (1..=2)
            .filter(|&team| self.streaks.emp_active(team, now))
            .fold(0, |mask, team| mask | (1 << team));
        let mut shots = self.aircraft.shots(now, &targets, emp, c);
        shots.extend(self.remote.drain_gunner_shots());
        for shot in shots {
            if self.rules.finished {
                break;
            }
            let outcome = crate::game::rewards::deployables::resolve_shot(&shot, now, &actors, c);
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
            for (entity, damage) in outcome.props_damage {
                self.damage_world_model(
                    now,
                    entity,
                    damage,
                    CreditSource::reward(shot.owner, shot.life),
                    false,
                    c,
                );
            }
            for (victim, damage, headshot) in outcome.damage {
                if victim == 0 {
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
        let dog_targets = targets
            .iter()
            .map(|t| crate::game::rewards::dogs::Target {
                actor: t.actor,
                life: t.life,
                born: t.born,
            })
            .collect::<Vec<_>>();
        for bite in self
            .dogs
            .advance(now, self.rules.finished, &dog_targets, &self.navigation, c)
        {
            if self.streaks.life(bite.target) != bite.target_life {
                continue;
            }
            self.damage_bots(
                now,
                CreditSource::reward(bite.owner, bite.life),
                bite.weapon,
                &[(bite.target, bite.damage, false)],
                true,
            );
            if bite.target == 0 && player.alive {
                tick.hits.push(PlayerHit {
                    reward_life: Some(bite.life),
                    damage: bite.damage,
                    attacker: bite.owner,
                    weapon: bite.weapon,
                    headshot: false,
                });
            }
        }
        tick
    }
}
