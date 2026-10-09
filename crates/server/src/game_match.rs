//! Local 6v6 TDM authority. The human occupies allies client0; five allies
//! and six axis bots have independent lives, input controllers and statistics.
use crate::{
    game_bot::{BotController, BotTarget, BotWeapon},
    game_combat::{self, Actor, ActorShot},
    game_motion::Collision,
};
use anyhow::{Result, ensure};
use codol_protocol::{
    game_enemy::EnemySnapshot,
    game_events::EventQueue,
    game_scoreboard::{ScoreboardRow, ScoreboardSnapshot},
};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatchConfig {
    /// Axis and allies starts from this map's captured entity string.
    pub spawns: [Vec<[f32; 3]>; 2],
    pub score_limit: u32,
    pub time_limit_msec: u32,
}
impl MatchConfig {
    pub fn validate(&self, collision: &Collision) -> Result<()> {
        ensure!(
            (100..=100_000).contains(&self.score_limit) && self.score_limit.is_multiple_of(100),
            "invalid TDM score limit"
        );
        ensure!(
            (60_000..=3_600_000).contains(&self.time_limit_msec),
            "invalid TDM duration"
        );
        for team in &self.spawns {
            ensure!(
                (6..=128).contains(&team.len()),
                "TDM needs at least six map starts per team"
            );
            for &spawn in team {
                crate::game_bot::grounded_spawn(spawn, collision)?;
            }
        }
        Ok(())
    }
}

struct Bot {
    controller: BotController,
    snapshot: EnemySnapshot,
    health: u16,
    birth: u32,
    death: Option<u32>,
    fire_time: Option<u32>,
    fire_toggle: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Kill {
    pub victim: EnemySnapshot,
    pub attacker: u8,
    pub weapon: u16,
    pub headshot: bool,
    pub sequence: u32,
}
#[derive(Clone, Copy)]
pub(crate) struct PlayerHit {
    pub damage: u16,
    pub attacker: u8,
    pub weapon: u16,
    pub headshot: bool,
}
#[derive(Default)]
pub(crate) struct Tick {
    pub hits: Vec<PlayerHit>,
    pub impacts: Vec<(u16, game_combat::WorldImpact)>,
}

pub(crate) struct TdmMatch {
    config: MatchConfig,
    bots: Vec<Bot>,
    baselines: Vec<EnemySnapshot>,
    rows: [ScoreboardRow; 12],
    scores: [i32; 4],
    kills: Vec<Kill>,
    pub dirty: bool,
    pub finished: bool,
    pub result_sent: bool,
    start: Option<u32>,
}
impl TdmMatch {
    pub fn new(
        config: MatchConfig,
        template: EnemySnapshot,
        speed: f32,
        collision: &Collision,
    ) -> Result<Self> {
        config.validate(collision)?;
        let mut bots = Vec::new();
        for client in 1..=11u8 {
            let team = if client <= 5 { 2 } else { 1 };
            let ordinal = if team == 2 {
                usize::from(client)
            } else {
                usize::from(client - 6)
            };
            let origin = crate::game_bot::grounded_spawn(
                config.spawns[usize::from(team - 1)][ordinal],
                collision,
            )?;
            let mut snapshot = template;
            snapshot.entity_number = u16::from(client);
            snapshot.client_index = client;
            snapshot.team = team;
            snapshot.origin = origin;
            snapshot.events = EventQueue::default();
            bots.push(Bot {
                controller: BotController::new(origin, snapshot.angles, speed)?,
                snapshot,
                health: 100,
                birth: 1000,
                death: None,
                fire_time: None,
                fire_toggle: false,
            });
        }
        let baselines = bots.iter().map(|bot| bot.snapshot).collect();
        let rows = std::array::from_fn(|i| ScoreboardRow {
            client_number: i as u8,
            user_id: i as u64,
            ..Default::default()
        });
        Ok(Self {
            config,
            bots,
            baselines,
            rows,
            scores: [0; 4],
            kills: Vec::new(),
            dirty: true,
            finished: false,
            result_sent: false,
            start: None,
        })
    }
    pub fn snapshots(&self) -> Vec<EnemySnapshot> {
        self.bots.iter().map(|bot| bot.snapshot).collect()
    }
    pub fn baselines(&self) -> &[EnemySnapshot] {
        &self.baselines
    }
    pub fn scoreboard(&self, owner: u64) -> ScoreboardSnapshot {
        let mut rows = self.rows.to_vec();
        rows[0].user_id = owner;
        ScoreboardSnapshot {
            rows,
            team_scores: self.scores,
            score_limit: self.config.score_limit,
        }
    }
    pub fn result(&self) -> &'static str {
        match self.scores[2].cmp(&self.scores[1]) {
            std::cmp::Ordering::Greater => "Allies win",
            std::cmp::Ordering::Less => "Axis win",
            std::cmp::Ordering::Equal => "Draw",
        }
    }
    pub fn player_spawn(&self, collision: &Collision) -> Result<[f32; 3]> {
        let starts = &self.config.spawns[1];
        let index = self.rows[0].deaths as usize % starts.len();
        crate::game_bot::grounded_spawn(starts[index], collision)
    }
    fn actors(&self, local: EnemySnapshot, local_height: f32) -> Vec<Actor> {
        std::iter::once(local)
            .chain(self.bots.iter().map(|b| b.snapshot))
            .filter(|s| s.alive)
            .map(|s| Actor {
                client: s.client_index,
                team: s.team,
                origin: s.origin,
                height: if s.client_index == 0 {
                    local_height
                } else {
                    80.0
                },
                yaw: f32::from(s.angles[1]) * 360.0 / 65536.0,
            })
            .collect()
    }
    fn credit(
        &mut self,
        now: u32,
        victim: EnemySnapshot,
        attacker: u8,
        weapon: u16,
        headshot: bool,
    ) {
        self.rows[usize::from(victim.client_index)].deaths += 1;
        let row = &mut self.rows[usize::from(attacker)];
        row.kills += 1;
        row.score += 100;
        row.headshots += u32::from(headshot);
        let team = if attacker <= 5 { 2 } else { 1 };
        self.scores[team] += 100;
        if self.scores[team] >= self.config.score_limit as i32 {
            self.finished = true;
        }
        self.kills.push(Kill {
            victim,
            attacker,
            weapon,
            headshot,
            sequence: self.rows[usize::from(victim.client_index)].deaths,
        });
        self.dirty = true;
        tracing::info!(
            now,
            victim = victim.client_index,
            attacker,
            team,
            score = self.scores[team],
            "TDM kill"
        );
    }
    pub fn player_killed(&mut self, now: u32, victim: EnemySnapshot, hit: PlayerHit) {
        self.credit(now, victim, hit.attacker, hit.weapon, hit.headshot);
    }
    fn damage_bots(&mut self, now: u32, attacker: u8, weapon: u16, hits: &[(u8, u16, bool)]) {
        for &(client, damage, headshot) in hits.iter().filter(|h| h.0 != 0) {
            if self.finished {
                break;
            }
            let bot = &mut self.bots[usize::from(client - 1)];
            if bot.health == 0 || now.wrapping_sub(bot.birth) < 1000 {
                continue;
            }
            bot.health = bot.health.saturating_sub(damage);
            if bot.health == 0 {
                let victim = bot.snapshot;
                bot.death = Some(now);
                bot.snapshot.alive = false;
                bot.controller.kill(now);
                self.credit(now, victim, attacker, weapon, headshot);
            }
        }
    }
    pub fn player_fire(
        &mut self,
        shot: crate::game_player::Shot,
        eye: [f32; 3],
        local: EnemySnapshot,
        collision: &Collision,
        regions: Option<&crate::game_hit_location::HitRegions>,
    ) -> ActorShot {
        if self.finished {
            return ActorShot::default();
        }
        let outcome = game_combat::fire_at_actors(
            eye,
            shot.angles,
            shot.time,
            shot.ballistics,
            (0, 2),
            game_combat::ActorScene {
                actors: &self.actors(local, 70.0),
                regions,
                collision,
            },
        );
        self.damage_bots(shot.time, 0, shot.weapon, &outcome.damage);
        outcome
    }
    pub fn take_kills(&mut self) -> Vec<Kill> {
        std::mem::take(&mut self.kills)
    }
    pub fn step(
        &mut self,
        now: u32,
        player: (EnemySnapshot, f32, u16),
        weapon: BotWeapon,
        collision: &Collision,
        animate: impl Fn(EnemySnapshot, &crate::game_bot::BotFrame, bool, bool) -> EnemySnapshot,
    ) -> Result<Tick> {
        let (mut local, local_height, local_health) = player;
        let mut tick = Tick::default();
        if self.finished {
            return Ok(tick);
        }
        let start = *self.start.get_or_insert(now);
        if now.wrapping_sub(start) >= self.config.time_limit_msec {
            self.finished = true;
            self.dirty = true;
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
                let origin = crate::game_bot::grounded_spawn(starts[index], collision)?;
                let bot = &mut self.bots[i];
                bot.controller.respawn(now, origin, bot.snapshot.angles)?;
                bot.snapshot.origin = origin;
                bot.snapshot.alive = true;
                bot.snapshot.events = EventQueue::default();
                bot.health = 100;
                bot.birth = now;
                bot.death = None;
                bot.fire_time = None;
            }
        }
        let mut shots = Vec::new();
        for i in 0..self.bots.len() {
            if self.bots[i].health == 0 {
                continue;
            }
            let origin = self.bots[i].snapshot.origin;
            let team = self.bots[i].snapshot.team;
            let target = self
                .actors(local, local_height)
                .into_iter()
                .filter(|a| a.team != team)
                .min_by(|a, b| distance(origin, a.origin).total_cmp(&distance(origin, b.origin)))
                .map(|a| BotTarget {
                    origin: a.origin,
                    eye: [a.origin[0], a.origin[1], a.origin[2] + a.height * 0.8],
                    alive: true,
                });
            let bot = &mut self.bots[i];
            let frame = bot.controller.step(now, target, weapon, collision);
            if let Some(shot) = frame.shot {
                bot.fire_time = Some(now);
                bot.fire_toggle = !bot.fire_toggle;
                bot.snapshot
                    .events
                    .push(codol_protocol::game_events::FIRE_WEAPON, 0)?;
                bot.snapshot
                    .events
                    .push(codol_protocol::game_events::EJECT_BRASS, 0)?;
                shots.push((bot.snapshot.client_index, team, shot));
            }
            let events = bot.snapshot.events;
            bot.snapshot = animate(
                bot.snapshot,
                &frame,
                bot.fire_time.is_some_and(|t| now.wrapping_sub(t) < 150),
                bot.fire_toggle,
            );
            bot.snapshot.events = events;
            bot.snapshot.weapon = weapon.weapon;
        }
        let mut player_health = local_health;
        for (attacker, team, shot) in shots {
            if self.finished {
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
            self.damage_bots(now, attacker, shot.weapon, &outcome.damage);
            for &(victim, damage, headshot) in &outcome.damage {
                if victim == 0 && player_health > 0 {
                    player_health = player_health.saturating_sub(damage);
                    tick.hits.push(PlayerHit {
                        damage,
                        attacker,
                        weapon: shot.weapon,
                        headshot,
                    });
                    local.alive = player_health > 0;
                }
            }
            tick.impacts.extend(
                outcome
                    .impacts
                    .into_iter()
                    .map(|impact| (shot.weapon, impact)),
            );
        }
        Ok(tick)
    }
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> Collision {
        let export = serde_json::json!({"schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,"brushes":[{"center":[0,0,-10],"half":[4000,4000,10],"contents":1,"planes":[]}],"triangles":[]});
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), serde_json::to_vec(&export).unwrap()).unwrap();
        Collision::load(file.path()).unwrap()
    }
    fn template() -> EnemySnapshot {
        EnemySnapshot {
            entity_number: 0,
            client_index: 0,
            team: 2,
            model: 4,
            attachment_models: [0; 10],
            attachment_tags: [0; 10],
            origin: [0.0; 3],
            angles: [0; 3],
            legs_animation: 395,
            torso_animation: 395,
            weapon: 352,
            events: EventQueue::default(),
            alive: true,
        }
    }
    fn config() -> MatchConfig {
        MatchConfig {
            spawns: [
                (0..6).map(|i| [300.0, i as f32 * 100.0, 0.0]).collect(),
                (0..6).map(|i| [-300.0, i as f32 * 100.0, 0.0]).collect(),
            ],
            score_limit: 7500,
            time_limit_msec: 600000,
        }
    }
    #[test]
    fn six_per_team_have_unique_clients_lives_scores_and_respawns() {
        let collision = world();
        let mut m = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
        assert_eq!(m.bots.len(), 11);
        let snapshots = m.snapshots();
        let baseline = codol_protocol::game_snapshot::player_frame_with_world(
            1000,
            1000,
            Some([0.0; 3]),
            true,
            None,
            [0; 3],
            codol_protocol::game_snapshot::WorldSnapshot {
                local_player: Some(template()),
                players: &snapshots,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            baseline.len() + 5 < codol_protocol::game_wire::FRAGMENT_SIZE,
            "twelve-player baseline {} bytes",
            baseline.len()
        );
        assert_eq!(m.bots.iter().filter(|b| b.snapshot.team == 2).count(), 5);
        assert_eq!(m.bots.iter().filter(|b| b.snapshot.team == 1).count(), 6);
        m.damage_bots(2100, 0, 352, &[(6, 100, true)]);
        assert!(!m.snapshots()[5].alive);
        let board = m.scoreboard(123);
        assert_eq!(board.rows[0].user_id, 123);
        assert_eq!(
            (
                board.rows[0].kills,
                board.rows[0].headshots,
                board.rows[6].deaths
            ),
            (1, 1, 1)
        );
        assert_eq!(board.team_scores, [0, 0, 100, 0]);
        let kills = m.take_kills();
        assert_eq!(kills.len(), 1);
        m.damage_bots(2200, 0, 352, &[(6, 100, true)]);
        assert!(m.take_kills().is_empty());
        let weapon = BotWeapon {
            weapon: 352,
            fire_interval_msec: 79,
            ballistics: game_combat::Ballistics {
                damage: 33,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
        };
        let mut local = template();
        local.alive = false;
        m.step(
            5100,
            (local, 70.0, 0),
            weapon,
            &collision,
            |mut snapshot, frame, _, _| {
                snapshot.origin = frame.motion.origin;
                snapshot.angles = frame.angles;
                snapshot
            },
        )
        .unwrap();
        assert!(m.snapshots()[5].alive);
        assert_eq!(m.bots[5].health, 100);
        assert_eq!(m.scoreboard(123).rows[6].deaths, 1);
    }
    #[test]
    fn score_and_time_limits_stop_combat_without_additional_score() {
        let collision = world();
        let mut c = config();
        c.score_limit = 100;
        let mut m = TdmMatch::new(c, template(), 190.0, &collision).unwrap();
        m.damage_bots(2100, 0, 352, &[(6, 100, false)]);
        assert!(m.finished);
        assert_eq!(m.result(), "Allies win");
        assert_eq!(m.scoreboard(1).score_limit, 100);
    }
    #[test]
    #[ignore = "requires local captured Afghan preparation"]
    fn captured_map_starts_build_a_real_twelve_player_match() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../research-output/tdm-state.json");
        let state = crate::game_state::PreparedGameState::load(&path).unwrap();
        assert!(state.is_match());
    }
}
