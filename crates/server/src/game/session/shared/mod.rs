//! One authoritative room world. Clients own only their input/ACK streams.
//! Native view slots are a per-recipient permutation: self is slot zero.
use super::prepared::PreparedGameState;
use crate::contracts::match_session::Participant;
use crate::game::combat::{self, Actor, ActorScene};
use crate::game::player::movement::Motion;
use crate::game::weapons::held::HeldWeapons;
use anyhow::{Result, ensure};
use codol_protocol::game::channel::usercmd::UserCommand;
use codol_protocol::game::snapshot::enemy::EnemySnapshot;
use codol_protocol::game::snapshot::scoreboard::{ScoreboardRow, ScoreboardSnapshot};
use codol_protocol::game::snapshot::{Equipment, PlayerVitals};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Instant,
};

struct Human {
    ai: Option<bots::Ai>,
    member: Participant,
    name: String,
    state: Arc<PreparedGameState>,
    player: HeldWeapons,
    motion: Motion,
    angles: [u16; 3],
    vitals: PlayerVitals,
    inputs: BTreeMap<u32, UserCommand>,
    last_input: u32,
    born: u32,
    died: Option<u32>,
    connected: bool,
    ready: bool,
    row: ScoreboardRow,
    feedback: crate::game::presentation::feedback::Feedback,
    recovery: crate::game::player::health::HealthRecovery,
    grenades: crate::game::weapons::grenade::Grenades,
    tactical: crate::game::status::tactical::State,
    poison: crate::game::status::gas::Poison,
    emp: crate::game::status::emp::State,
}
struct Kill {
    time: u32,
    victim: u8,
    attacker: u8,
    weapon: u16,
    headshot: bool,
    melee: bool,
    falling: bool,
    entity: u16,
}
struct Hit {
    attacker: usize,
    victim: usize,
    damage: u16,
    weapon: u16,
    headshot: bool,
    melee: bool,
    falling: bool,
}

pub(crate) struct SharedMatch {
    humans: Vec<Human>,
    epoch: Option<Instant>,
    time: u32,
    pub(crate) revision: u64,
    pub(crate) scores: [i32; 4],
    score_limit: u32,
    time_limit: u32,
    pub(crate) ended: Option<u32>,
    impacts: crate::game::presentation::impact::ImpactEvents,
    kills: VecDeque<Kill>,
    next_obituary: u16,
    corpses: VecDeque<(
        u32,
        u8,
        codol_protocol::game::snapshot::corpse::CorpseSnapshot,
    )>,
    next_corpse: u16,
    smoke: crate::game::weapons::smoke::Smoke,
    gas: crate::game::status::gas::Gas,
}

#[derive(Clone)]
pub(crate) struct View {
    pub time: u32,
    pub command_time: u32,
    pub motion: Motion,
    pub angles: [u16; 3],
    pub equipment: Equipment,
    pub vitals: PlayerVitals,
    pub local: EnemySnapshot,
    pub players: Vec<EnemySnapshot>,
    pub characters: Vec<Option<codol_protocol::game::snapshot::character::Character>>,
    pub impacts: Vec<codol_protocol::game::snapshot::impact::ImpactSnapshot>,
    pub obituaries: Vec<codol_protocol::game::snapshot::obituary::ObituarySnapshot>,
    pub corpses: Vec<codol_protocol::game::snapshot::corpse::CorpseSnapshot>,
    pub feedback: Option<codol_protocol::game::snapshot::hit_feedback::HitFeedbackSnapshot>,
    pub missiles: Vec<codol_protocol::game::snapshot::missile::MissileSnapshot>,
    pub smokes: Vec<codol_protocol::game::snapshot::smoke::SmokeSnapshot>,
    pub shellshock: Option<codol_protocol::game::snapshot::shellshock::ShellShockSnapshot>,
    pub scoreboard: ScoreboardSnapshot,
    pub revision: u64,
    pub ended: Option<u32>,
    pub winner: u8,
}

impl SharedMatch {
    pub(crate) fn new(players: Vec<(Participant, String, Arc<PreparedGameState>)>) -> Result<Self> {
        ensure!(
            (2..=12).contains(&players.len()),
            "invalid shared match size"
        );
        let (score_limit, time_limit) = players[0].2.human_limits();
        let mut humans = Vec::new();
        for (member, name, state) in players {
            ensure!(
                humans
                    .iter()
                    .all(|h: &Human| h.member.user_id != member.user_id),
                "duplicate shared identity"
            );
            let mut motion = state
                .motion()
                .ok_or_else(|| anyhow::anyhow!("missing human movement"))?;
            motion.command_time = 1000;
            let player = state
                .player()
                .ok_or_else(|| anyhow::anyhow!("missing human equipment"))?;
            let ai = member.bot.map(|_| bots::Ai::new(&state)).transpose()?;
            let synthetic = ai.is_some();
            humans.push(Human {
                ai,
                row: ScoreboardRow {
                    user_id: member.user_id,
                    ..Default::default()
                },
                member,
                name,
                angles: state.spawn_angles(),
                state,
                player,
                motion,
                vitals: Default::default(),
                inputs: BTreeMap::new(),
                last_input: 999,
                born: 1000,
                died: None,
                connected: synthetic,
                ready: synthetic,
                feedback: Default::default(),
                recovery: Default::default(),
                grenades: Default::default(),
                tactical: Default::default(),
                poison: Default::default(),
                emp: Default::default(),
            });
        }
        Ok(Self {
            humans,
            epoch: None,
            time: 1000,
            revision: 1,
            scores: [0; 4],
            score_limit,
            time_limit,
            ended: None,
            impacts: Default::default(),
            kills: VecDeque::new(),
            corpses: VecDeque::new(),
            next_corpse: 19,
            next_obituary: 1800,
            smoke: Default::default(),
            gas: Default::default(),
        })
    }
    fn index(&self, user: u64) -> Result<usize> {
        self.humans
            .iter()
            .position(|h| h.member.user_id == user)
            .ok_or_else(|| anyhow::anyhow!("player is not admitted to this match"))
    }
    pub(crate) fn progress(&self, user: u64) -> Result<(Motion, [u16; 3])> {
        let human = &self.humans[self.index(user)?];
        Ok((human.motion.clone(), human.angles))
    }
    pub(crate) fn state(&self, user: u64) -> Result<Arc<PreparedGameState>> {
        Ok(self.humans[self.index(user)?].state.clone())
    }
    pub(crate) fn take_feedback_sounds(&mut self, user: u64) -> Result<u32> {
        let index = self.index(user)?;
        Ok(self.humans[index].feedback.take_sounds())
    }
    pub(crate) fn attach(&mut self, user: u64) -> Result<()> {
        let index = self.index(user)?;
        ensure!(
            self.humans[index].member.bot.is_none() && !self.humans[index].connected,
            "human already has a game peer"
        );
        self.humans[index].connected = true;
        Ok(())
    }
    pub(crate) fn ready(&mut self, user: u64) -> Result<()> {
        let index = self.index(user)?;
        self.humans[index].ready = true;
        Ok(())
    }
    pub(crate) fn detach(&mut self, user: u64) {
        if let Ok(index) = self.index(user) {
            self.humans[index].connected = false;
            self.humans[index].vitals.health = 0;
            self.humans[index].inputs.clear();
            self.ended.get_or_insert(self.time);
            self.revision += 1;
        }
    }
    pub(crate) fn names(&self, user: u64) -> Result<Vec<String>> {
        let local = self.index(user)?;
        Ok(std::iter::once(&self.humans[local])
            .chain(
                self.humans
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != local)
                    .map(|(_, h)| h),
            )
            .map(|h| h.name.clone())
            .collect())
    }
    pub(crate) fn submit(&mut self, user: u64, mut input: UserCommand) -> Result<()> {
        let index = self.index(user)?;
        let human = &mut self.humans[index];
        if !human.connected
            || input.time < human.born
            || input.time <= human.last_input
            || input.time > self.time + 250
            || human.inputs.len() >= 256
        {
            return Ok(());
        }
        let offset = human.state.spawn_angles();
        input.angles = std::array::from_fn(|i| input.angles[i].wrapping_add(offset[i]));
        human.inputs.entry(input.time).or_insert(input);
        Ok(())
    }
    pub(crate) fn advance(&mut self, wall: Instant) -> Result<()> {
        if self.epoch.is_none() && self.humans.iter().all(|h| h.ready && h.connected) {
            self.epoch = Some(wall);
        }
        let Some(epoch) = self.epoch else {
            return Ok(());
        };
        let now = 1000
            + wall
                .duration_since(epoch)
                .as_millis()
                .min(u128::from(i32::MAX as u32 - 1000)) as u32;
        if now <= self.time {
            return Ok(());
        }
        self.time = now;
        self.smoke.expire(now);
        self.gas.expire(now);
        if self.ended.is_none() && now - 1000 >= self.time_limit {
            self.ended = Some(now);
            self.revision += 1;
        }
        let mut landings = Vec::new();
        for index in 0..self.humans.len() {
            let human = &mut self.humans[index];
            if human.died.is_some_and(|death| now - death >= 3000)
                && self.ended.is_none()
                && human.connected
            {
                human.motion = human.state.motion().unwrap();
                human.motion.command_time = now;
                human.player = human.state.player().unwrap();
                human.vitals = PlayerVitals {
                    spawn_count: human.vitals.spawn_count.wrapping_add(1),
                    ..Default::default()
                };
                human.born = now;
                human.died = None;
                human.feedback.clear();
                human.recovery = Default::default();
                human.tactical.clear();
                human.poison.clear();
                human.emp.clear();
                if let Some(ai) = &mut human.ai {
                    ai.controller
                        .respawn(now, human.motion.origin, human.state.spawn_angles())?;
                    ai.route.clear();
                }
            }
            human.vitals.health = human.recovery.recover(now, human.vitals.health);
            while human
                .inputs
                .first_key_value()
                .is_some_and(|(&time, _)| time <= now)
            {
                let (_, mut input) = human.inputs.pop_first().unwrap();
                if input.time <= human.last_input {
                    continue;
                }
                human.last_input = input.time;
                if human.vitals.health == 0 || input.time < human.born || self.ended.is_some() {
                    input.buttons = 0;
                    input.direction = 0;
                }
                let collision = human.state.collision.as_ref().unwrap();
                let ads = human
                    .player
                    .equipment()
                    .aim
                    .is_some_and(|a| a.movement_slow);
                let scale = (if ads {
                    human.player.ads_move_scale()
                } else {
                    human.player.move_scale()
                }) * human.tactical.move_scale(now, human.player.asym_robot());
                human.motion.flags = (human.motion.flags & !0x8000)
                    | if human.tactical.stunned(now) {
                        0x8000
                    } else {
                        0
                    };
                human.motion.set_sprint_rules(human.player.sprint_rules());
                human.motion.apply(&input, scale, ads, collision);
                human.angles = input.angles;
                human.player.apply_with_melee_approach(&input, None);
                for landing in human.motion.take_landings() {
                    if let Some(event) = landing.event {
                        human.player.movement_event(event.id, event.parameter);
                    }
                    let damage = human.player.fall_damage(landing.damage);
                    if damage > 0 {
                        landings.push(Hit {
                            attacker: index,
                            victim: index,
                            damage,
                            weapon: human.player.equipment().weapon,
                            headshot: false,
                            melee: false,
                            falling: true,
                        });
                    }
                }
            }
        }
        landings.extend(self.step_bots(now)?);
        for hit in landings {
            self.damage(hit, now);
        }
        if self.ended.is_none() {
            for index in 0..self.humans.len() {
                let shots = self.humans[index].player.take_shots();
                let melees = self.humans[index].player.take_melee_strikes();
                if self.humans[index].vitals.health == 0 {
                    continue;
                }
                let human = &self.humans[index];
                let eye = [
                    human.motion.origin[0],
                    human.motion.origin[1],
                    human.motion.origin[2] + human.motion.view_height,
                ];
                let team = human.member.team;
                let collision = human.state.collision.clone().unwrap();
                let actors = self
                    .humans
                    .iter()
                    .enumerate()
                    .filter(|(_, h)| h.vitals.health > 0 && h.connected)
                    .map(|(i, h)| Actor {
                        client: i as u8,
                        team: h.member.team,
                        origin: h.motion.origin,
                        height: h.motion.view_height + 10.0,
                        yaw: f32::from(h.angles[1]) * std::f32::consts::TAU / 65536.0,
                    })
                    .collect::<Vec<_>>();
                for shot in shots {
                    let outcome = combat::fire_at_actors(
                        eye,
                        shot.angles,
                        shot.time,
                        shot.ballistics,
                        (index as u8, team),
                        ActorScene {
                            actors: &actors,
                            regions: None,
                            collision: &collision,
                        },
                    );
                    for impact in outcome.impacts {
                        self.impacts.push(now, shot.weapon, impact);
                    }
                    for (victim, damage, headshot) in outcome.damage {
                        self.damage(
                            Hit {
                                attacker: index,
                                victim: usize::from(victim),
                                damage,
                                weapon: shot.weapon,
                                headshot,
                                melee: false,
                                falling: false,
                            },
                            now,
                        );
                    }
                }
                for strike in melees {
                    let outcome = combat::melee_at_actors(
                        eye,
                        strike.angles,
                        strike.damage,
                        (index as u8, team),
                        ActorScene {
                            actors: &actors,
                            regions: None,
                            collision: &collision,
                        },
                    );
                    for (victim, damage, _) in outcome.damage {
                        self.damage(
                            Hit {
                                attacker: index,
                                victim: usize::from(victim),
                                damage,
                                weapon: strike.weapon,
                                headshot: false,
                                melee: true,
                                falling: false,
                            },
                            now,
                        );
                    }
                }
            }
            self.advance_grenades(now)?;
        }
        self.kills.retain(|kill| now - kill.time < 200);
        self.corpses.retain(|(time, ..)| now - time < 6000);
        Ok(())
    }
    fn damage(&mut self, hit: Hit, now: u32) {
        let Hit {
            attacker,
            victim,
            damage,
            weapon,
            headshot,
            melee,
            falling,
        } = hit;
        if self.ended.is_some() || self.humans[victim].vitals.health == 0 || damage == 0 {
            return;
        }
        let target = &mut self.humans[victim];
        target.vitals.health = target.vitals.health.saturating_sub(damage);
        target.vitals.damage_event = target.vitals.damage_event.wrapping_add(1);
        target.vitals.damage_count = damage.min(127) as u8;
        target.recovery.damaged(now, target.vitals.health);
        let killed = target.vitals.health == 0;
        if killed {
            target.died = Some(now);
            if let Some(ai) = &mut target.ai {
                ai.controller.kill(now);
            }
            target.row.deaths += 1;
            let snapshot = Self::actor(target, victim as u8);
            let mut corpse = target.state.corpse(snapshot, target.row.deaths);
            corpse.entity_number = self.next_corpse;
            self.next_corpse = 19 + (self.next_corpse - 19 + 1) % 8;
            self.corpses
                .retain(|(_, _, c)| c.entity_number != corpse.entity_number);
            self.corpses.push_back((now, victim as u8, corpse));
            let suicide = attacker == victim;
            self.humans[attacker].row.kills += u32::from(!suicide);
            self.humans[attacker].row.score += if falling {
                0
            } else if suicide {
                -100
            } else {
                100
            };
            self.humans[attacker].row.headshots += u32::from(headshot);
            let team = usize::from(self.humans[attacker].member.team);
            self.scores[team] += if suicide { 0 } else { 100 };
            self.kills.push_back(Kill {
                time: now,
                victim: victim as u8,
                attacker: attacker as u8,
                weapon,
                headshot,
                melee,
                falling,
                entity: self.next_obituary,
            });
            self.next_obituary = if self.next_obituary == 2045 {
                1800
            } else {
                self.next_obituary + 1
            };
            if self.scores[team] >= self.score_limit as i32 {
                self.ended = Some(now);
            }
            tracing::info!(now, attacker, victim, scores=?self.scores, "shared human kill");
        }
        if attacker != victim {
            self.humans[attacker].feedback.record(
                now,
                if killed {
                    codol_protocol::game::snapshot::hit_feedback::HitKind::Kill
                } else {
                    codol_protocol::game::snapshot::hit_feedback::HitKind::Hit
                },
            );
        }
        self.revision += 1;
    }
    fn actor(human: &Human, slot: u8) -> EnemySnapshot {
        let mut actor = human
            .state
            .local_player_snapshot(human.vitals.health > 0)
            .unwrap();
        actor.entity_number = u16::from(slot);
        actor.client_index = slot;
        actor.team = human.member.team;
        actor.origin = human.motion.origin;
        actor.angles = human.angles;
        actor.weapon = human.player.equipment().weapon;
        let frame = crate::game::ai::controller::BotFrame {
            landings: Vec::new(),
            motion: human.motion.clone(),
            angles: human.angles,
            phase: crate::game::ai::controller::BotPhase::Idle,
            shot: None,
            locomotion: if human.motion.flags & 0x4000 != 0 {
                crate::game::ai::controller::Locomotion::Sprint
            } else {
                crate::game::ai::controller::Locomotion::Run
            },
        };
        let equipment = human.player.equipment();
        actor = human.state.animate_enemy(
            actor,
            &frame,
            equipment.firing.is_some_and(|f| f.state == 16),
            human.last_input.is_multiple_of(2),
        );
        actor
    }
    pub(crate) fn view(&mut self, user: u64) -> Result<View> {
        let local = self.index(user)?;
        let order = std::iter::once(local)
            .chain((0..self.humans.len()).filter(|&i| i != local))
            .collect::<Vec<_>>();
        let slot = |index: usize| order.iter().position(|&i| i == index).unwrap() as u8;
        let human = &self.humans[local];
        let scoreboard = ScoreboardSnapshot {
            rows: order
                .iter()
                .enumerate()
                .map(|(i, &n)| ScoreboardRow {
                    client_number: i as u8,
                    ..self.humans[n].row
                })
                .collect(),
            team_scores: self.scores,
            score_limit: self.score_limit,
        };
        Ok(View {
            time: self.time,
            command_time: human.motion.command_time.min(self.time),
            motion: human.motion.clone(),
            angles: human.angles,
            equipment: human.player.equipment(),
            vitals: PlayerVitals {
                intermission: self.ended.is_some(),
                emp_jammed: human.emp.active(self.time),
                ..human.vitals
            },
            local: Self::actor(human, 0),
            players: order
                .iter()
                .enumerate()
                .skip(1)
                .map(|(i, &n)| Self::actor(&self.humans[n], i as u8))
                .collect(),
            characters: order
                .iter()
                .map(|&n| self.humans[n].state.human_character())
                .collect(),
            impacts: self.impacts.snapshot(self.time),
            obituaries: self
                .kills
                .iter()
                .map(
                    |kill| codol_protocol::game::snapshot::obituary::ObituarySnapshot {
                        falling: kill.falling,
                        melee: kill.melee,
                        entity_number: kill.entity,
                        weapon: kill.weapon,
                        headshot: kill.headshot,
                        victim: slot(usize::from(kill.victim)),
                        attacker: slot(usize::from(kill.attacker)),
                    },
                )
                .collect(),
            corpses: self
                .corpses
                .iter()
                .map(|(time, donor, c)| {
                    let mut c = *c;
                    c.donor_client = slot(usize::from(*donor));
                    if self.time - time >= 1200 {
                        c.phase = codol_protocol::game::snapshot::corpse::CorpsePhase::Frozen;
                    }
                    c
                })
                .collect(),
            feedback: human
                .state
                .feedback_resources()
                .map(|r| human.feedback.snapshot(self.time, r)),
            missiles: self
                .humans
                .iter()
                .enumerate()
                .flat_map(|(index, h)| {
                    h.grenades.snapshots().into_iter().map(move |mut m| {
                        m.entity_number = 1400 + index as u16 * 8 + (m.entity_number - 1700);
                        m.owner = slot(index);
                        m
                    })
                })
                .collect(),
            smokes: self.smoke.snapshots(self.time),
            shellshock: human.tactical.snapshot(self.time),
            scoreboard,
            revision: self.revision,
            ended: self.ended,
            winner: match self.scores[1].cmp(&self.scores[2]) {
                std::cmp::Ordering::Greater => 1,
                std::cmp::Ordering::Less => 2,
                _ => 0,
            },
        })
    }
}

mod bots;
mod grenades;
#[cfg(test)]
mod tests;
