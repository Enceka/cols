use super::*;
impl SharedMatch {
    pub(super) fn advance_grenades(&mut self, now: u32) -> Result<()> {
        for index in 0..self.humans.len() {
            let human = &mut self.humans[index];
            let collision = human.state.collision.clone().unwrap();
            let eye = [
                human.motion.origin[0],
                human.motion.origin[1],
                human.motion.origin[2] + human.motion.view_height,
            ];
            for request in human.player.take_throws() {
                human
                    .grenades
                    .launch(request, eye, human.motion.velocity, &collision);
            }
            let team = human.member.team;
            for explosion in human.grenades.advance(now, &collision) {
                if explosion.definition.smoke.is_some() {
                    self.smoke.spawn(&explosion);
                }
                if explosion.definition.gas {
                    self.gas
                        .deploy(&explosion, index as u8, team, &collision, &mut self.smoke);
                }
                self.impacts.explosion(
                    now,
                    explosion.definition.index,
                    explosion.origin,
                    explosion.normal,
                );
                for target in 0..self.humans.len() {
                    let human = &mut self.humans[target];
                    if human.vitals.health == 0 || (target != index && human.member.team == team) {
                        continue;
                    }
                    let eye = [
                        human.motion.origin[0],
                        human.motion.origin[1],
                        human.motion.origin[2] + human.motion.view_height,
                    ];
                    if let Some(kind) = explosion.definition.tactical
                        && let Some(hit) = crate::game::status::tactical::hit(
                            &explosion,
                            crate::game::status::tactical::Recipient {
                                feet: human.motion.origin,
                                eye,
                                angles: human.angles,
                            },
                            human.player.tactical_protection(kind),
                            &collision,
                        )
                    {
                        human.tactical.apply(now, hit);
                    }
                    if explosion.definition.emp {
                        human.emp.apply(
                            now,
                            &explosion,
                            crate::game::status::emp::Target {
                                origin: human.motion.origin,
                                height: human.motion.view_height + 10.0,
                                protection: human.player.emp_protection(),
                            },
                            &collision,
                            &mut human.tactical,
                        );
                    }
                    let damage = human.player.explosive_damage(
                        crate::game::weapons::grenade::radius_damage(
                            &explosion,
                            human.motion.origin,
                            human.motion.view_height + 10.0,
                            &collision,
                        ),
                    );
                    self.damage(
                        Hit {
                            attacker: index,
                            victim: target,
                            damage,
                            weapon: explosion.definition.index,
                            headshot: false,
                            melee: false,
                            falling: false,
                        },
                        now,
                    );
                }
            }
        }
        for index in 0..self.humans.len() {
            let human = &mut self.humans[index];
            let collision = human.state.collision.clone().unwrap();
            let hit = human.poison.advance(
                now,
                &self.gas,
                crate::game::status::gas::Recipient {
                    client: index as u8,
                    team: human.member.team,
                    origin: human.motion.origin,
                    height: human.motion.view_height + 10.0,
                    alive: human.vitals.health > 0,
                    protected: human.player.gas_protected(),
                },
                &collision,
                &mut human.tactical,
            );
            if let Some(hit) = hit {
                self.damage(
                    Hit {
                        attacker: usize::from(hit.attacker),
                        victim: index,
                        damage: hit.damage,
                        weapon: hit.weapon,
                        headshot: false,
                        melee: false,
                        falling: false,
                    },
                    now,
                );
            }
        }
        Ok(())
    }
}
