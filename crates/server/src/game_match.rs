//! Local 6v6 TDM authority. Native room teams are frozen for each match.
use crate::{
    game_bot::{BotController, BotTarget, BotWeapon},
    game_combat::{self, Actor, ActorShot},
    game_motion::Collision,
};
use anyhow::{Context, Result, ensure};
use codol_protocol::{
    game_enemy::EnemySnapshot,
    game_events::EventQueue,
    game_scoreboard::{ScoreboardRow, ScoreboardSnapshot},
};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MatchConfig {
    #[serde(default = "default_player_count")]
    pub player_count: u8,
    /// Axis and allies starts from this map's captured entity string.
    pub spawns: [Vec<[f32; 3]>; 2],
    pub score_limit: u32,
    pub time_limit_msec: u32,
    #[serde(default = "default_teams")]
    pub teams: [u8; 12],
    #[serde(default)]
    pub roster: Vec<BotLoadout>,
    #[serde(default)]
    pub models: Vec<(u16, String)>,
}
fn default_player_count() -> u8 {
    12
}
pub(crate) fn default_teams() -> [u8; 12] {
    [2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1]
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BotLoadout {
    pub class: String,
    pub weapon_name: String,
    pub animation: crate::game_enemy::EnemyConfig,
    pub attachment_models: [u16; 10],
    pub personality: String,
    #[serde(default)]
    pub outfits: Option<[BotOutfit; 2]>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BotOutfit {
    pub model: u16,
    pub attachment_models: [u16; 10],
    #[serde(default)]
    pub character: Option<codol_protocol::game_character::Character>,
}
impl BotLoadout {
    fn outfit(&self, team: u8) -> BotOutfit {
        self.outfits.as_ref().map_or(
            BotOutfit {
                model: self.animation.model,
                attachment_models: self.attachment_models,
                character: None,
            },
            |a| a[usize::from(team - 1)],
        )
    }
}
impl MatchConfig {
    pub fn validate(&self, collision: &Collision) -> Result<()> {
        ensure!(
            matches!(self.player_count, 4 | 12)
                && self.teams[..usize::from(self.player_count)]
                    .iter()
                    .all(|t| matches!(t, 1 | 2))
                && self.teams[..usize::from(self.player_count)]
                    .iter()
                    .filter(|&&t| t == 1)
                    .count()
                    == usize::from(self.player_count / 2)
                && self.teams[usize::from(self.player_count)..]
                    .iter()
                    .all(|&t| t == 0),
            "TDM requires a balanced active roster with empty unused slots"
        );
        ensure!(
            (100..=100_000).contains(&self.score_limit) && self.score_limit.is_multiple_of(100),
            "invalid TDM score limit"
        );
        ensure!(
            (60_000..=3_600_000).contains(&self.time_limit_msec),
            "invalid TDM duration"
        );
        ensure!(
            self.roster.is_empty() || self.roster.len() == usize::from(self.player_count - 1),
            "TDM roster must match the configured player count"
        );
        for entry in &self.roster {
            entry.animation.validate()?;
            for (faction, outfit) in entry.outfits.iter().flatten().enumerate() {
                if let Some(character) = outfit.character {
                    character.fields()?;
                    ensure!(
                        character.faction == 1 - faction as u8,
                        "Bot character faction mismatch"
                    );
                }
                ensure!(
                    (1..1024).contains(&outfit.model)
                        && outfit.attachment_models.iter().all(|&i| i < 1024),
                    "invalid faction-specific Bot outfit"
                );
                let mut ended = false;
                for model in outfit.attachment_models {
                    ensure!(!ended || model == 0, "sparse Bot outfit attachments");
                    ended |= model == 0;
                }
            }
            ensure!(
                entry.class.starts_with("botclass") && entry.class.len() <= 32,
                "invalid native Bot class"
            );
            ensure!(
                matches!(
                    entry.personality.as_str(),
                    "default" | "camper" | "run_and_gun"
                ),
                "invalid bot personality"
            );
            ensure!(
                entry.weapon_name.ends_with("_mp")
                    && entry
                        .weapon_name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "invalid bot weapon identity"
            );
            ensure!(
                entry.attachment_models.iter().all(|&i| i < 1024),
                "invalid bot part index"
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        for (id, name) in &self.models {
            ensure!(
                (1..1024).contains(id)
                    && ids.insert(*id)
                    && !name.is_empty()
                    && name.len() < 64
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "invalid bot model registration"
            );
        }
        for (team_index, team) in self.spawns.iter().enumerate() {
            ensure!(
                (usize::from(self.player_count / 2)..=128).contains(&team.len()),
                "TDM needs a captured start for every active player"
            );
            for (index, &spawn) in team.iter().enumerate() {
                crate::game_bot::grounded_spawn(spawn, collision).with_context(|| {
                    format!(
                        "invalid TDM spawn: team={} index={index} origin={spawn:?}",
                        team_index + 1
                    )
                })?;
            }
        }
        Ok(())
    }
}

struct Bot {
    view_motion: crate::game_motion::Motion,
    feedback: Option<(u32, codol_protocol::game_hit_feedback::HitKind)>,
    damage_event: u8,
    damage_count: u8,
    snapshot_time: u32,
    weapon: Option<crate::game_player::Player>,
    initial_weapon: Option<crate::game_player::Player>,
    memory: Option<(u8, BotTarget, u32)>,
    perception_time: u32,
    visible_target: Option<BotTarget>,
    random: u64,
    progress_origin: [f32; 3],
    progress_time: u32,
    recent_route: std::collections::VecDeque<[f32; 3]>,
    stuck_level: u8,

    tactical: crate::game_tactical::State,
    poison: crate::game_gas::Poison,
    emp: crate::game_emp::State,
    controller: BotController,
    snapshot: EnemySnapshot,
    health: u16,
    birth: u32,
    death: Option<u32>,
    fire_time: Option<u32>,
    fire_toggle: bool,
    reload_animation: crate::game_enemy::ReloadAnimation,
    route: std::collections::VecDeque<crate::game_navigation::Waypoint>,
    route_target: Option<[f32; 3]>,
    route_time: u32,
    steering: Option<crate::game_navigation::Waypoint>,
    steering_time: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct Kill {
    pub falling: bool,
    pub time: u32,
    pub victim: EnemySnapshot,
    pub attacker: u8,
    pub weapon: u16,
    pub headshot: bool,
    pub melee: bool,
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
    pub finished_at: Option<u32>,
    start: Option<u32>,
    navigation: std::sync::Arc<crate::game_navigation::Navigation>,
    next_diagnostics: u32,
    next_route_client: usize,
}
impl TdmMatch {
    pub(crate) fn set_validation_seed(&mut self, seed: u64) {
        for bot in &mut self.bots {
            bot.random = seed
                .wrapping_add(u64::from(bot.snapshot.client_index) * 0x9e3779b9)
                .max(1);
        }
    }
    #[cfg(test)]
    pub fn new(
        config: MatchConfig,
        template: EnemySnapshot,
        speed: f32,
        collision: &Collision,
    ) -> Result<Self> {
        let navigation = std::sync::Arc::new(crate::game_navigation::Navigation::build(
            config.spawns.iter().flatten().copied(),
            collision,
        ));
        Self::with_navigation(config, template, speed, collision, navigation)
    }
    pub fn with_navigation(
        config: MatchConfig,
        template: EnemySnapshot,
        speed: f32,
        collision: &Collision,
        navigation: std::sync::Arc<crate::game_navigation::Navigation>,
    ) -> Result<Self> {
        config.validate(collision)?;
        let mut bots = Vec::new();
        let mut ordinals = [0usize; 2];
        ordinals[usize::from(config.teams[0] - 1)] = 1;
        for client in 1..config.player_count {
            let team = config.teams[usize::from(client)];
            let ordinal = ordinals[usize::from(team - 1)];
            ordinals[usize::from(team - 1)] += 1;
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
            let mut controller = BotController::new(origin, snapshot.angles, speed)?;
            controller.set_match_engagement();
            bots.push(Bot {
                view_motion: crate::game_motion::Motion::new(origin),
                feedback: None,
                damage_event: 0,
                damage_count: 0,
                snapshot_time: 1000,
                weapon: None,
                initial_weapon: None,
                memory: None,
                perception_time: 0,
                visible_target: None,
                random: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos() as u64
                    ^ (u64::from(client) * 0x9e3779b9),
                progress_origin: origin,
                progress_time: 1000,
                recent_route: Default::default(),
                stuck_level: 0,
                tactical: Default::default(),
                poison: Default::default(),
                emp: Default::default(),
                controller,
                snapshot,
                health: 100,
                birth: 1000,
                death: None,
                fire_time: None,
                fire_toggle: false,
                reload_animation: Default::default(),
                route: Default::default(),
                route_target: None,
                route_time: 0,
                steering: None,
                steering_time: 0,
            });
        }
        let baselines = bots.iter().map(|bot| bot.snapshot).collect();
        let rows = std::array::from_fn(|i| ScoreboardRow {
            client_number: i as u8,
            user_id: if i == 0 {
                0
            } else {
                codol_protocol::room_messages::bot_id(i as u8)
            },
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
            finished_at: None,
            start: None,
            navigation,
            next_diagnostics: 0,
            next_route_client: 0,
        })
    }
    pub fn configure_roster(&mut self, weapons: Vec<crate::game_player::Player>) -> Result<()> {
        ensure!(
            weapons.len() == self.config.roster.len(),
            "bot weapon/roster mismatch"
        );
        for (i, weapon) in weapons.into_iter().enumerate() {
            let bot = &mut self.bots[i];
            let config = &self.config.roster[i];
            let outfit = config.outfit(bot.snapshot.team);
            bot.snapshot.model = outfit.model;
            bot.snapshot.attachment_models = outfit.attachment_models;
            bot.snapshot.weapon = weapon.equipment().weapon;
            bot.controller.set_match_move_scale(weapon.move_scale());
            bot.controller
                .set_match_ads_move_scale(weapon.ads_move_scale());
            bot.initial_weapon = Some(weapon.clone());
            bot.weapon = Some(weapon);
        }
        self.baselines = self.bots.iter().map(|b| b.snapshot).collect();
        Ok(())
    }
    pub fn snapshots(&self) -> Vec<EnemySnapshot> {
        self.bots.iter().map(|bot| bot.snapshot).collect()
    }
    pub(crate) fn jump_count(&self) -> u32 {
        self.bots.iter().map(|b| b.controller.jump_count()).sum()
    }
    pub fn replay_views(&self) -> Vec<crate::game_replay::View> {
        self.bots
            .iter()
            .filter_map(|bot| {
                let weapon = bot.weapon.as_ref()?;
                Some(crate::game_replay::View {
                    shellshock: bot.tactical.snapshot(bot.snapshot_time),
                    client: bot.snapshot.client_index,
                    motion: bot.view_motion.clone(),
                    angles: bot.snapshot.angles,
                    equipment: weapon.equipment(),
                    vitals: codol_protocol::game_snapshot::PlayerVitals {
                        health: bot.health,
                        damage_event: bot.damage_event,
                        damage_count: bot.damage_count,
                        emp_jammed: bot.emp.active(bot.snapshot_time),
                        spawn_count: (self.rows[usize::from(bot.snapshot.client_index)].deaths
                            + u32::from(bot.health > 0)) as u8,
                        ..Default::default()
                    },
                    hit: bot.feedback,
                })
            })
            .collect()
    }
    pub fn teams(&self) -> &[u8; 12] {
        &self.config.teams
    }
    pub fn set_weapon_move_scale(&mut self, scale: f32) {
        for bot in &mut self.bots {
            bot.controller
                .set_match_move_scale(bot.weapon.as_ref().map_or(scale, |w| w.move_scale()));
            bot.controller
                .set_match_ads_move_scale(bot.weapon.as_ref().map_or(1.0, |w| w.ads_move_scale()));
        }
    }
    pub fn baselines(&self) -> &[EnemySnapshot] {
        &self.baselines
    }
    pub fn scoreboard(&self, owner: u64) -> ScoreboardSnapshot {
        let mut rows = self.rows[..usize::from(self.config.player_count)].to_vec();
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
    pub fn winning_team(&self) -> u8 {
        match self.scores[2].cmp(&self.scores[1]) {
            std::cmp::Ordering::Greater => 2,
            std::cmp::Ordering::Less => 1,
            std::cmp::Ordering::Equal => 0,
        }
    }
    pub fn duration_seconds(&self) -> u32 {
        self.finished_at
            .unwrap_or(0)
            .saturating_sub(self.start.unwrap_or(0))
            / 1000
    }
    pub fn advance_clock(&mut self, now: u32) {
        if self.finished {
            return;
        }
        let start = *self.start.get_or_insert(now);
        if now.saturating_sub(start) >= self.config.time_limit_msec {
            self.finish(now);
        }
    }
    fn finish(&mut self, now: u32) {
        if !self.finished {
            self.finished = true;
            self.finished_at = Some(now);
            for bot in &mut self.bots {
                bot.poison.clear();
                bot.tactical.clear();
                bot.emp.clear();
            }
            self.dirty = true;
            tracing::info!(now, winner=self.winning_team(), scores=?self.scores, "TDM match finished");
        }
    }
    pub fn player_spawn(&self, collision: &Collision) -> Result<[f32; 3]> {
        let starts = &self.config.spawns[usize::from(self.config.teams[0] - 1)];
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
        melee: bool,
    ) {
        if self.finished {
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
        let row = &mut self.rows[usize::from(attacker)];
        let suicide = attacker == victim.client_index;
        row.kills += u32::from(!suicide);
        row.score += if suicide { -100 } else { 100 };
        row.headshots += u32::from(headshot);
        let team = usize::from(self.config.teams[usize::from(attacker)]);
        // Reference war.onNormalDeath awards team points only for opposing
        // player kills. Self-grenade penalties affect the personal row only.
        self.scores[team] += if suicide { 0 } else { 100 };
        if self.scores[team] >= self.config.score_limit as i32 {
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
            score = self.scores[team],
            "TDM kill"
        );
    }
    pub fn player_killed(&mut self, now: u32, victim: EnemySnapshot, hit: PlayerHit) {
        self.credit(now, victim, hit.attacker, hit.weapon, hit.headshot, false);
    }
    pub fn actor_fell(&mut self, now: u32, victim: EnemySnapshot) {
        if self.finished {
            return;
        }
        // A world fall has no attacker. Reference handleWorldDeath does not
        // apply suicidepointloss or invoke war.onNormalDeath in that case.
        self.rows[usize::from(victim.client_index)].deaths += 1;
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
        explosion: &crate::game_grenade::Explosion,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> usize {
        if self.finished {
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
            if let Some(hit) = crate::game_tactical::hit(
                explosion,
                crate::game_tactical::Recipient {
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
        explosion: &crate::game_grenade::Explosion,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> (Option<codol_protocol::game_hit_feedback::HitKind>, u16) {
        if self.finished {
            return (None, 0);
        }
        let hits = self
            .bots
            .iter()
            .filter(|bot| bot.health != 0 && bot.snapshot.team != local.team)
            .map(|bot| {
                (
                    bot.snapshot.client_index,
                    crate::game_grenade::radius_damage(
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
            crate::game_grenade::radius_damage(explosion, local.origin, 70.0, collision)
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
        explosion: &crate::game_grenade::Explosion,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> usize {
        if self.finished {
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
                crate::game_emp::Target {
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
    pub fn advance_gas(
        &mut self,
        now: u32,
        gas: &crate::game_gas::Gas,
        collision: &Collision,
    ) -> Option<codol_protocol::game_hit_feedback::HitKind> {
        let mut hits = Vec::new();
        for bot in &mut self.bots {
            let target = crate::game_gas::Recipient {
                client: bot.snapshot.client_index,
                team: bot.snapshot.team,
                origin: bot.snapshot.origin,
                height: bot.view_motion.view_height + 10.0,
                alive: !self.finished && bot.health > 0 && now.wrapping_sub(bot.birth) >= 1000,
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
            if result == Some(codol_protocol::game_hit_feedback::HitKind::Kill)
                || feedback.is_none()
            {
                feedback = result;
            }
        }
        feedback
    }
    fn damage_bots(
        &mut self,
        now: u32,
        attacker: u8,
        weapon: u16,
        hits: &[(u8, u16, bool)],
        melee: bool,
    ) -> Option<codol_protocol::game_hit_feedback::HitKind> {
        use codol_protocol::game_hit_feedback::HitKind;
        let mut feedback = None;
        for &(client, damage, headshot) in hits.iter().filter(|h| h.0 != 0) {
            if self.finished {
                break;
            }
            let bot = &mut self.bots[usize::from(client - 1)];
            let age = now.wrapping_sub(bot.birth);
            if damage == 0 || bot.health == 0 || !(1000..0x8000_0000).contains(&age) {
                continue;
            }
            bot.health = bot.health.saturating_sub(damage);
            bot.damage_event = bot.damage_event.wrapping_add(1);
            bot.damage_count = damage.min(127) as u8;
            feedback.get_or_insert(HitKind::Hit);
            if bot.health == 0 {
                feedback = Some(HitKind::Kill);
                let victim = bot.snapshot;
                bot.death = Some(now);
                bot.snapshot.alive = false;
                bot.controller.kill(now);
                self.credit(now, victim, attacker, weapon, headshot, melee);
            }
        }
        feedback
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
            self.damage_bots(shot.time, 0, shot.weapon, &outcome.damage, false);
        outcome
    }
    pub fn player_melee(
        &mut self,
        strike: crate::game_player::MeleeStrike,
        eye: [f32; 3],
        local: EnemySnapshot,
        collision: &Collision,
    ) -> ActorShot {
        if self.finished || !local.alive {
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
        outcome
    }
    pub fn player_melee_approach(
        &self,
        cmd: &codol_protocol::game_usercmd::UserCommand,
        motion: &crate::game_motion::Motion,
        local: EnemySnapshot,
        collision: &Collision,
    ) -> Option<crate::game_player::MeleeApproach> {
        if self.finished
            || !local.alive
            || cmd.melee_distance == 0
            || cmd.buttons & 4 == 0
            || cmd.buttons & (0x10000000 | 0x200 | 0x100 | 0x400) != 0
            || motion.stance != crate::game_motion::Stance::Stand
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
        let approach = crate::game_player::MeleeApproach {
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
    #[cfg(test)]
    pub fn step(
        &mut self,
        now: u32,
        player: (EnemySnapshot, f32, u16),
        weapon: BotWeapon,
        collision: &Collision,
        animate: impl Fn(EnemySnapshot, &crate::game_bot::BotFrame, bool, bool) -> EnemySnapshot,
    ) -> Result<Tick> {
        self.step_with_smoke(
            now,
            player,
            weapon,
            &crate::game_smoke::Smoke::default(),
            collision,
            animate,
        )
    }
    pub fn step_with_smoke(
        &mut self,
        now: u32,
        player: (EnemySnapshot, f32, u16),
        weapon: BotWeapon,
        smoke: &crate::game_smoke::Smoke,
        collision: &Collision,
        animate: impl Fn(EnemySnapshot, &crate::game_bot::BotFrame, bool, bool) -> EnemySnapshot,
    ) -> Result<Tick> {
        let started = std::time::Instant::now();
        let mut movement = std::time::Duration::ZERO;
        let (mut local, local_height, local_health) = player;
        let mut tick = Tick::default();
        self.advance_clock(now);
        if self.finished {
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
                        let seed = crate::game_navigation::next_random(&mut bot.random);
                        let recent: Vec<_> = bot.recent_route.iter().copied().collect();
                        bot.route = self
                            .navigation
                            .route_varied(origin, goal, collision, seed, &recent)
                            .into();
                        bot.route_target = (!bot.route.is_empty()).then_some(goal);
                        bot.route_time = now;
                        bot.steering = None;
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
            let mut frame =
                bot.controller
                    .step_route(now, target, bot.steering, active_weapon, collision);
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
                let command = codol_protocol::game_usercmd::UserCommand {
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
                frame.shot = shots.first().map(|s| crate::game_bot::BotShot {
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
                        .push(codol_protocol::game_events::FIRE_WEAPON, 0)?;
                    bot.snapshot
                        .events
                        .push(codol_protocol::game_events::EJECT_BRASS, 0)?;
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
                Some(class) => crate::game_state::PreparedGameState::animate_with_config(
                    &class.animation,
                    bot.snapshot,
                    &frame,
                    firing,
                    bot.fire_toggle,
                ),
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
            let mut feedback = self.damage_bots(now, attacker, shot.weapon, &outcome.damage, false);
            for &(victim, damage, headshot) in &outcome.damage {
                if victim == 0 && player_health > 0 {
                    player_health = player_health.saturating_sub(damage);
                    if player_health == 0 {
                        feedback = Some(codol_protocol::game_hit_feedback::HitKind::Kill);
                    } else {
                        feedback.get_or_insert(codol_protocol::game_hit_feedback::HitKind::Hit);
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
fn visible_enemy(
    origin: [f32; 3],
    yaw: u16,
    team: u8,
    actors: &[Actor],
    now: u32,
    smoke: &crate::game_smoke::Smoke,
    collision: &Collision,
) -> Option<(u8, BotTarget)> {
    let eye = [origin[0], origin[1], origin[2] + 60.0];
    let angle = f32::from(yaw) * std::f32::consts::TAU / 65536.0;
    let (sin, cos) = angle.sin_cos();
    actors
        .iter()
        .filter(|a| a.team != team)
        .filter(|a| {
            let dx = a.origin[0] - origin[0];
            let dy = a.origin[1] - origin[1];
            let length = dx.hypot(dy);
            if length > 2000.0 || (length > 160.0 && dx * cos + dy * sin < length * 0.5) {
                return false;
            }
            let trace = collision.visibility_raycast(
                eye,
                [a.origin[0], a.origin[1], a.origin[2] + a.height * 0.8],
            );
            !trace.start_solid
                && !trace.all_solid
                && trace.contents == 0
                && smoke.visible(
                    now,
                    eye,
                    [a.origin[0], a.origin[1], a.origin[2] + a.height * 0.8],
                )
        })
        .min_by(|a, b| distance(origin, a.origin).total_cmp(&distance(origin, b.origin)))
        .map(|a| {
            (
                a.client,
                BotTarget {
                    origin: a.origin,
                    eye: [a.origin[0], a.origin[1], a.origin[2] + a.height * 0.8],
                    alive: true,
                },
            )
        })
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "uses the locally captured Killhouse ledge to verify Bot falling authority"]
    fn captured_bot_fall_uses_real_map_collision_and_preserves_team_scores() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
        let template =
            crate::game_state::PreparedGameState::load(&root.join("tdm-killhouse-hud-state.json"))
                .unwrap();
        let profiles = root.join("../client-work/server-data/profiles/1002");
        let profiles: Vec<_> = std::fs::read_dir(profiles)
            .unwrap()
            .map(|x| x.unwrap().path())
            .filter(|p| p.join("public.bin").is_file() && p.join("inventory.json").is_file())
            .collect();
        assert_eq!(
            profiles.len(),
            1,
            "this local capture fixture requires one account"
        );
        let public = std::fs::read(profiles[0].join("public.bin")).unwrap();
        let inventory: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profiles[0].join("inventory.json")).unwrap())
                .unwrap();
        let owned: Vec<u32> = serde_json::from_value(inventory["items"].clone()).unwrap();
        let state = template
            .for_initial_backpack(&public)
            .unwrap()
            .for_profile(&public, &owned)
            .unwrap();
        let route: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join("landing-diagnostic-route.json")).unwrap(),
        )
        .unwrap();
        let origin: [f32; 3] = serde_json::from_value(route["origin"].clone()).unwrap();
        let end: [f32; 3] = serde_json::from_value(route["end"].clone()).unwrap();
        let collision = state.collision.as_ref().unwrap();
        let mut game = state.tdm_match().unwrap().unwrap();
        for b in &mut game.bots {
            b.health = 0;
            b.snapshot.alive = false;
        }
        let bot = &mut game.bots[0];
        bot.health = 100;
        bot.snapshot.alive = true;
        bot.snapshot.origin = origin;
        bot.controller.respawn(1000, origin, [0; 3]).unwrap();
        let mut local = state.local_player_snapshot(false).unwrap();
        local.alive = false;
        let weapon = state.match_bot_weapon().unwrap();
        for now in (1050..=4000).step_by(50) {
            // This fixture guides the actor over the captured ledge; it does
            // not validate navigation's preferred route or native visuals.
            game.next_route_client = 1;
            game.bots[0].steering = Some(crate::game_navigation::Waypoint::walk(end));
            game.bots[0].steering_time = now;
            game.step(
                now,
                (local, 70.0, 0),
                weapon,
                collision,
                |s, f, fire, toggle| state.animate_enemy(s, f, fire, toggle),
            )
            .unwrap();
        }
        let kills = game.take_kills();
        assert_eq!(kills.len(), 1);
        assert!(kills[0].falling);
        assert_eq!(game.bots[0].health, 0);
        assert_eq!(game.scoreboard(1).team_scores, [0; 4]);
        let proof = serde_json::json!({"client_sha256":codol_protocol::CLIENT_SHA256,"read_only_captured_resources":true,"native_visual_verified":false,"map":state.map(),"ncs_crc":state.ncs_crc(),"source_route":route,"victim":kills[0].victim.client_index,"fall_time":kills[0].time,"death_origin":kills[0].victim.origin,"held_weapon":kills[0].weapon,"deaths":game.scoreboard(1).rows[1].deaths,"personal_score":game.scoreboard(1).rows[1].score,"team_scores":game.scoreboard(1).team_scores});
        std::fs::write(
            root.join("bot-landing-captured-proof.json"),
            serde_json::to_vec_pretty(&proof).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn room_team_assignment_selects_bot_outfits_and_their_pinned_baselines() {
        let outfit:BotLoadout=serde_json::from_value(serde_json::json!({
            "class":"botclass1","weapon_name":"m4_mp","personality":"default",
            "attachment_models":[3,2,5,6,0,0,0,0,0,0],
            "animation":{"model":4,"origin":[0,0,0],"bounds_min":[-15,-15,0],"bounds_max":[15,15,70],
                "idle_animation":395,"walk_animation":243,"fire_animation":616,"death_animation":47,
                "move_speed":190,"angles":[0,0,0],"weapon":352,"source_sha256":"a".repeat(64)},
            "outfits":[{"model":201,"attachment_models":[202,203,204,205,0,0,0,0,0,0]},
                       {"model":301,"attachment_models":[302,303,304,305,0,0,0,0,0,0]}]
        })).unwrap();
        let weapon = crate::game_player::Player::new(
            codol_protocol::game_snapshot::Equipment {
                perks: Default::default(),
                abilities: Default::default(),
                offhands: [None, None],
                offhand_weapon: 0,
                grenade_time: 0,
                move_multiplier: 1.0,
                weapon: 352,
                inventory_weapon: None,
                alternate_weapon: None,
                alternates: [None, None],
                fire_count: 0,
                queued_burst: false,
                barrel_flags: 0,
                spread_scale: 0.0,
                needs_rechamber: false,
                ammo: None,
                viewmodel: None,
                firing: None,
                aim: None,
                secondary: None,
                events: None,
            },
            Some(80),
            None,
            None,
        );
        for swapped in [false, true] {
            let mut c = config();
            c.roster = vec![outfit.clone(); 11];
            if swapped {
                c.teams = c.teams.map(|t| 3 - t);
            }
            let mut game = TdmMatch::new(c, template(), 190.0, &world()).unwrap();
            game.configure_roster(vec![weapon.clone(); 11]).unwrap();
            for (actor, baseline) in game.snapshots().iter().zip(game.baselines()) {
                let expected = if actor.team == 1 { 201 } else { 301 };
                assert_eq!(actor.model, expected);
                assert_eq!(actor.attachment_models[0], expected + 1);
                assert_eq!(baseline.model, actor.model);
                assert_eq!(baseline.attachment_models, actor.attachment_models);
            }
        }
    }
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
            player_count: 12,
            spawns: [
                (0..6).map(|i| [300.0, i as f32 * 100.0, 0.0]).collect(),
                (0..6).map(|i| [-300.0, i as f32 * 100.0, 0.0]).collect(),
            ],
            score_limit: 7500,
            time_limit_msec: 600000,
            teams: default_teams(),
            roster: Vec::new(),
            models: Vec::new(),
        }
    }
    #[test]
    fn four_player_world_keeps_only_its_active_actors_and_preserves_full_roster_requirements() {
        let collision = world();
        let mut small = config();
        small.player_count = 4;
        small.teams = [2, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        small
            .spawns
            .iter_mut()
            .for_each(|starts| starts.truncate(2));
        let game = TdmMatch::new(small.clone(), template(), 190.0, &collision).unwrap();
        assert_eq!(
            game.snapshots()
                .iter()
                .map(|s| s.client_index)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let board = game.scoreboard(42);
        assert_eq!(board.rows.len(), 4);
        assert_eq!(board.rows[0].user_id, 42);
        assert_eq!(game.actors(template(), 70.0).len(), 4);
        let mut bad = small.clone();
        bad.teams[4] = 1;
        assert!(bad.validate(&collision).is_err());
        bad = small.clone();
        bad.teams[2] = 2;
        assert!(bad.validate(&collision).is_err());
        small.player_count = 12;
        small.teams = default_teams();
        assert!(small.validate(&collision).is_err());
    }
    #[test]
    fn fatal_fall_scores_once_as_an_environmental_death_without_awarding_a_kill() {
        let mut game = TdmMatch::new(config(), template(), 190.0, &world()).unwrap();
        game.actor_fell(2000, template());
        let board = game.scoreboard(1);
        assert_eq!(
            (
                board.rows[0].kills,
                board.rows[0].deaths,
                board.rows[0].score
            ),
            (0, 1, 0)
        );
        assert_eq!(board.team_scores[2], 0);
        let kills = game.take_kills();
        assert_eq!(kills.len(), 1);
        assert!(kills[0].falling && !kills[0].headshot && !kills[0].melee);
        assert_eq!(kills[0].weapon, 352);
        assert_eq!(kills[0].attacker, kills[0].victim.client_index);
        game.finished = true;
        game.actor_fell(2100, template());
        assert!(game.take_kills().is_empty());
    }
    #[test]
    fn bot_fall_applies_its_perk_and_runs_the_normal_death_respawn_lifecycle() {
        let c = world();
        for lightweight in [false, true] {
            let mut game = TdmMatch::new(config(), template(), 190.0, &c).unwrap();
            // Isolate one actor's physical fall from gunfire by retaining no
            // other living targets in this simulation fixture.
            for b in &mut game.bots {
                b.health = 0;
                b.snapshot.alive = false;
            }
            let bot = &mut game.bots[0];
            bot.health = 100;
            bot.snapshot.alive = true;
            bot.snapshot.origin = [0.0, 0.0, 350.0];
            bot.controller
                .respawn(1000, bot.snapshot.origin, [0; 3])
                .unwrap();
            let mut held = crate::game_player::tests::scavenger_player(90, 180);
            if lightweight {
                held.set_perks(
                    crate::game_perks::Perks::selected([60041, 0, 0], &[60041]).unwrap(),
                );
            }
            bot.weapon = Some(held.clone());
            bot.initial_weapon = Some(held);
            let mut local = template();
            local.alive = false;
            let fallback = BotWeapon {
                weapon: 352,
                fire_interval_msec: 80,
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
            let animate = |mut s: EnemySnapshot, f: &crate::game_bot::BotFrame, _, _| {
                s.origin = f.motion.origin;
                s.angles = f.angles;
                s
            };
            for now in (1050..=3000).step_by(50) {
                game.step(now, (local, 70.0, 0), fallback, &c, animate)
                    .unwrap();
            }
            let board = game.scoreboard(1);
            if lightweight {
                assert_eq!(game.bots[0].health, 100);
                assert!(game.bots[0].snapshot.alive);
                assert_eq!(board.rows[1].deaths, 0);
                assert!(game.take_kills().is_empty());
            } else {
                assert_eq!(game.bots[0].health, 0);
                assert!(!game.bots[0].snapshot.alive);
                assert_eq!(
                    (
                        board.rows[1].kills,
                        board.rows[1].deaths,
                        board.rows[1].score
                    ),
                    (0, 1, 0)
                );
                assert_eq!(board.team_scores, [0; 4]);
                let kills = game.take_kills();
                assert_eq!(kills.len(), 1);
                assert!(kills[0].falling);
                game.step(6500, (local, 70.0, 0), fallback, &c, animate)
                    .unwrap();
                assert_eq!(game.bots[0].health, 100);
                assert!(game.bots[0].snapshot.alive);
                assert_eq!(game.scoreboard(1).rows[1].deaths, 1);
                assert!(game.take_kills().is_empty());
            }
        }
    }
    #[test]
    fn perception_obeys_team_field_of_view_close_awareness_and_walls() {
        let collision = world();
        let actor = |client, team, x, y| Actor {
            client,
            team,
            origin: [x, y, 0.0],
            height: 70.0,
            yaw: 0.0,
        };
        let allies = actor(1, 2, 100.0, 0.0);
        let behind = actor(2, 1, -500.0, 0.0);
        let front = actor(3, 1, 700.0, 0.0);
        assert_eq!(
            visible_enemy(
                [0.0; 3],
                0,
                2,
                &[allies, behind, front],
                1000,
                &crate::game_smoke::Smoke::default(),
                &collision
            )
            .unwrap()
            .0,
            3
        );
        assert!(
            visible_enemy(
                [0.0; 3],
                0,
                2,
                &[allies, behind],
                1000,
                &crate::game_smoke::Smoke::default(),
                &collision
            )
            .is_none()
        );
        assert_eq!(
            visible_enemy(
                [0.0; 3],
                0,
                2,
                &[actor(4, 1, -100.0, 0.0)],
                1000,
                &crate::game_smoke::Smoke::default(),
                &collision
            )
            .unwrap()
            .0,
            4
        );
        // An enemy under the solid floor cannot be acquired even at close range.
        let underground = Actor {
            origin: [0.0, 0.0, -100.0],
            ..front
        };
        assert!(
            visible_enemy(
                [0.0; 3],
                0,
                2,
                &[underground],
                1000,
                &crate::game_smoke::Smoke::default(),
                &collision
            )
            .is_none()
        );
    }
    #[test]
    fn perception_loses_smoke_occluded_opponents_and_recovers_after_optical_fade() {
        let collision = world();
        let opponent = Actor {
            client: 1,
            team: 1,
            origin: [300.0, 0.0, 0.0],
            height: 70.0,
            yaw: 0.0,
        };
        let mut smoke = crate::game_smoke::Smoke::default();
        smoke.spawn(&crate::game_grenade::Explosion {
            time: 1000,
            origin: [150.0, 0.0, 2.0],
            normal: [0.0, 0.0, 1.0],
            definition: crate::game_grenade::captured("smoke_grenade_mp"),
        });
        assert!(visible_enemy([0.0; 3], 0, 2, &[opponent], 1000, &smoke, &collision).is_some());
        assert!(visible_enemy([0.0; 3], 0, 2, &[opponent], 5000, &smoke, &collision).is_none());
        assert!(visible_enemy([0.0; 3], 0, 2, &[opponent], 14000, &smoke, &collision).is_some());
        assert!(!smoke.snapshots(14000).is_empty());
        let outside = Actor {
            origin: [300.0, 400.0, 0.0],
            ..opponent
        };
        assert!(
            visible_enemy(
                [-500.0, 400.0, 0.0],
                0,
                2,
                &[outside],
                5000,
                &smoke,
                &collision
            )
            .is_some()
        );
    }
    #[test]
    fn tactical_notifications_exclude_team_mates_and_death_resets_the_native_named_state() {
        let collision = world();
        let local = template();
        let mut game = TdmMatch::new(config(), local, 190.0, &collision).unwrap();
        let enemy = game
            .bots
            .iter()
            .position(|b| b.snapshot.team != local.team)
            .unwrap();
        let ally = game
            .bots
            .iter()
            .position(|b| b.snapshot.team == local.team)
            .unwrap();
        for bot in &mut game.bots {
            bot.snapshot.origin = [2000.0, 0.0, 0.0];
        }
        game.bots[enemy].snapshot.origin = [128.0, 0.0, 0.0];
        game.bots[ally].snapshot.origin = [128.0, 0.0, 0.0];
        let explosion = crate::game_grenade::Explosion {
            time: 1000,
            origin: [128.0, 0.0, 60.0],
            normal: [0.0, 0.0, 1.0],
            definition: crate::game_grenade::captured("flash_grenade_mp"),
        };
        assert_eq!(game.player_tactical(1000, &explosion, local, &collision), 1);
        assert!(game.bots[enemy].tactical.blind(1000));
        assert!(!game.bots[ally].tactical.blind(1000));
        let victim = game.bots[enemy].snapshot;
        game.credit(1200, victim, 0, 89, false, false);
        assert_eq!(
            game.bots[enemy].tactical.snapshot(1200),
            Some(codol_protocol::game_shellshock::ShellShockSnapshot {
                index: 5,
                begin_time: 0,
                duration_msec: 0
            })
        );
        assert!(!game.bots[enemy].tactical.blind(1200));
    }
    #[test]
    fn gas_applies_owner_credit_and_mask_immunity_then_clears_on_death_respawn_and_finish() {
        let collision = world();
        let mut game = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
        for bot in &mut game.bots {
            bot.snapshot.origin = [2000.0, 0.0, 0.0];
        }
        let enemy = game
            .bots
            .iter()
            .position(|b| b.snapshot.team != template().team)
            .unwrap();
        let ally = game
            .bots
            .iter()
            .position(|b| b.snapshot.team == template().team)
            .unwrap();
        game.bots[enemy].snapshot.origin = [0.0; 3];
        game.bots[ally].snapshot.origin = [0.0; 3];
        let mut gas = crate::game_gas::Gas::default();
        let mut persistent = crate::game_smoke::Smoke::default();
        assert!(gas.deploy(
            &crate::game_grenade::Explosion {
                time: 2000,
                origin: [0.0, 0.0, 2.0],
                normal: [0.0, 0.0, 1.0],
                definition: crate::game_grenade::captured("tabun_gas_mp")
            },
            0,
            template().team,
            &collision,
            &mut persistent
        ));
        let mut held = crate::game_player::tests::scavenger_player(90, 180);
        held.set_perks(crate::game_perks::Perks::selected([0, 0, 60137], &[60137]).unwrap());
        assert!(held.gas_protected());
        game.bots[enemy].weapon = Some(held);
        assert_eq!(game.advance_gas(2000, &gas, &collision), None);
        assert_eq!(game.bots[enemy].health, 100);
        assert!(game.bots[enemy].tactical.snapshot(2000).is_none());
        game.bots[enemy]
            .weapon
            .as_mut()
            .unwrap()
            .set_perks(Default::default());
        assert_eq!(
            game.advance_gas(2100, &gas, &collision),
            Some(codol_protocol::game_hit_feedback::HitKind::Hit)
        );
        assert_eq!(game.bots[enemy].health, 87);
        assert_eq!(game.bots[ally].health, 100);
        assert_eq!(game.advance_gas(2100, &gas, &collision), None);
        for now in (3100..=9100).step_by(1000) {
            game.advance_gas(now, &gas, &collision);
        }
        assert_eq!(game.bots[enemy].health, 0);
        assert!(!game.bots[enemy].snapshot.alive);
        assert_eq!(
            game.bots[enemy]
                .tactical
                .snapshot(9100)
                .unwrap()
                .duration_msec,
            0
        );
        let kills = game.take_kills();
        assert_eq!(kills.len(), 1);
        assert_eq!((kills[0].attacker, kills[0].weapon), (0, 251));
        assert!(!kills[0].falling);
        assert_eq!(game.scoreboard(1).rows[0].score, 100);
        assert_eq!(game.scoreboard(1).rows[enemy + 1].deaths, 1);
        let weapon = BotWeapon {
            weapon: 352,
            fire_interval_msec: 80,
            ballistics: crate::game_combat::Ballistics {
                damage: 33,
                range: 8192.0,
                near_range: 500.0,
                location_multipliers: None,
                falloff: None,
                pellets: 1,
                spread_degrees: 0.0,
            },
        };
        game.step(
            12100,
            (template(), 70.0, 100),
            weapon,
            &collision,
            |s, _, _, _| s,
        )
        .unwrap();
        assert_eq!(game.bots[enemy].health, 100);
        assert_eq!(
            game.bots[enemy]
                .tactical
                .snapshot(12100)
                .unwrap()
                .duration_msec,
            0
        );
        game.bots[enemy].tactical.apply(
            12200,
            crate::game_tactical::Hit {
                kind: crate::game_tactical::Kind::Gas,
                duration_msec: 1000,
            },
        );
        game.finish(12300);
        assert_eq!(
            game.bots[enemy]
                .tactical
                .snapshot(12300)
                .unwrap()
                .duration_msec,
            0
        );
    }
    #[test]
    fn emp_excludes_allies_checks_hardwire_and_clears_the_archived_actor_on_death_and_finish() {
        let collision = world();
        let local = template();
        let mut game = TdmMatch::new(config(), local, 190.0, &collision).unwrap();
        for bot in &mut game.bots {
            bot.snapshot.origin = [2000.0, 0.0, 0.0];
        }
        let enemy = game
            .bots
            .iter()
            .position(|b| b.snapshot.team != local.team)
            .unwrap();
        let ally = game
            .bots
            .iter()
            .position(|b| b.snapshot.team == local.team)
            .unwrap();
        game.bots[enemy].snapshot.origin = [0.0; 3];
        game.bots[ally].snapshot.origin = [0.0; 3];
        let explosion = crate::game_grenade::Explosion {
            time: 2000,
            origin: [0.0, 0.0, 60.0],
            normal: [0.0, 0.0, 1.0],
            definition: crate::game_grenade::captured("emp_grenade_mp"),
        };
        let mut held = crate::game_player::tests::scavenger_player(90, 180);
        held.set_perks(crate::game_perks::Perks::selected([0, 60213, 0], &[60213]).unwrap());
        game.bots[enemy].weapon = Some(held.clone());
        assert_eq!(game.player_emp(2000, &explosion, local, &collision), 0);
        assert!(!game.bots[enemy].emp.active(2000));
        held.set_perks(Default::default());
        game.bots[enemy].weapon = Some(held);
        assert_eq!(game.player_emp(2000, &explosion, local, &collision), 1);
        assert!(game.bots[enemy].emp.active(2000));
        assert!(!game.bots[ally].emp.active(2000));
        game.bots[enemy].snapshot_time = 2000;
        assert!(
            game.replay_views()
                .iter()
                .find(|v| v.client == game.bots[enemy].snapshot.client_index)
                .unwrap()
                .vitals
                .emp_jammed
        );
        let victim = game.bots[enemy].snapshot;
        game.credit(2100, victim, 0, 74, false, false);
        assert!(!game.bots[enemy].emp.active(2100));
        assert!(
            !game
                .replay_views()
                .iter()
                .find(|v| v.client == victim.client_index)
                .unwrap()
                .vitals
                .emp_jammed
        );
        game.player_emp(2200, &explosion, local, &collision);
        game.finish(2300);
        assert!(!game.bots[enemy].emp.active(2300));
        assert_eq!(
            game.bots[enemy]
                .tactical
                .snapshot(2300)
                .unwrap()
                .duration_msec,
            0
        );
    }
    #[test]
    fn grenade_blast_excludes_allies_and_suicide_never_awards_a_kill() {
        let collision = world();
        let mut game = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
        for bot in &mut game.bots {
            bot.snapshot.origin = [2000.0, 0.0, 0.0];
            bot.birth = 1000;
        }
        let enemy = game
            .bots
            .iter()
            .position(|b| b.snapshot.team != template().team)
            .unwrap();
        let ally = game
            .bots
            .iter()
            .position(|b| b.snapshot.team == template().team)
            .unwrap();
        game.bots[enemy].snapshot.origin = [64.0, 0.0, 0.0];
        game.bots[ally].snapshot.origin = [64.0, 0.0, 0.0];
        let explosion = crate::game_grenade::Explosion {
            time: 1000,
            origin: [64.0, 0.0, 40.0],
            normal: [0.0, 0.0, 1.0],
            definition: crate::game_grenade::captured("frag_grenade_mp"),
        };
        let (feedback, damage) = game.player_explosion(3000, &explosion, template(), &collision);
        assert_eq!(
            feedback,
            Some(codol_protocol::game_hit_feedback::HitKind::Kill)
        );
        assert_eq!(game.bots[enemy].health, 0);
        assert_eq!(game.bots[ally].health, 100);
        assert!(damage >= 100);
        assert_eq!(game.rows[0].kills, 1);
        game.player_killed(
            3000,
            template(),
            PlayerHit {
                damage,
                attacker: 0,
                weapon: 95,
                headshot: false,
            },
        );
        assert_eq!(
            (game.rows[0].kills, game.rows[0].deaths, game.rows[0].score),
            (1, 1, 0)
        );
        assert_eq!(game.scoreboard(1).team_scores[2], 100);
        assert!(
            game.take_kills()
                .iter()
                .all(|k| k.weapon == 95 && !k.headshot && !k.melee)
        );
    }
    #[test]
    fn native_room_teams_control_spawns_scoring_and_winner() {
        let collision = world();
        let mut config = config();
        config.teams = [1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2];
        config.score_limit = 100;
        let mut m = TdmMatch::new(config, template(), 200.0, &collision).unwrap();
        assert_eq!(m.player_spawn(&collision).unwrap()[0], 300.0);
        for bot in &m.bots {
            assert_eq!(
                bot.snapshot.team,
                m.teams()[usize::from(bot.snapshot.client_index)]
            );
            assert_eq!(
                bot.snapshot.origin[0],
                if bot.snapshot.team == 1 {
                    300.0
                } else {
                    -300.0
                }
            );
        }
        m.damage_bots(2100, 0, 352, &[(1, 100, false)], false);
        assert_eq!(m.scoreboard(42).team_scores, [0, 100, 0, 0]);
        assert_eq!(m.winning_team(), 1);
        assert_eq!(m.scoreboard(42).rows[1].deaths, 1);
    }
    #[test]
    fn player_shots_follow_either_room_team_and_teammates_block_without_damage() {
        let collision = world();
        for player_team in [1, 2] {
            let mut config = config();
            config.teams = std::array::from_fn(|i| {
                if i % 2 == 0 {
                    player_team
                } else {
                    3 - player_team
                }
            });
            let mut local = template();
            local.team = player_team;
            let mut m = TdmMatch::new(config, local, 200.0, &collision).unwrap();
            for bot in &mut m.bots {
                bot.snapshot.origin = [500.0, 500.0, 0.0];
            }
            // Bot 2 is friendly and shields the enemy Bot 1 directly behind.
            m.bots[0].snapshot.origin = [200.0, 0.0, 0.0];
            m.bots[1].snapshot.origin = [100.0, 0.0, 0.0];
            let mut shot = crate::game_player::Shot {
                time: 2100,
                angles: [0; 3],
                weapon: 352,
                ballistics: game_combat::Ballistics {
                    damage: 50,
                    range: 8192.0,
                    near_range: 500.0,
                    location_multipliers: None,
                    falloff: None,
                    pellets: 1,
                    spread_degrees: 0.0,
                },
            };
            let blocked = m.player_fire(shot, [0.0, 0.0, 60.0], local, &collision, None);
            assert!(blocked.damage.is_empty());
            assert!(blocked.committed_feedback.is_none());
            assert!(m.bots.iter().all(|b| b.health == 100));
            assert_eq!(m.scoreboard(42).team_scores, [0; 4]);

            m.bots[1].snapshot.origin[1] = 100.0;
            shot.time = 1500;
            let protected = m.player_fire(shot, [0.0, 0.0, 60.0], local, &collision, None);
            assert!(!protected.damage.is_empty());
            assert!(protected.committed_feedback.is_none());
            assert_eq!(m.bots[0].health, 100);
            for time in [2200, 2300] {
                shot.time = time;
                let hit = m.player_fire(shot, [0.0, 0.0, 60.0], local, &collision, None);
                assert_eq!(hit.damage, [(1, 50, false)]);
                assert_eq!(
                    hit.committed_feedback,
                    Some(if time == 2200 {
                        codol_protocol::game_hit_feedback::HitKind::Hit
                    } else {
                        codol_protocol::game_hit_feedback::HitKind::Kill
                    })
                );
            }
            assert_eq!(m.bots[0].health, 0);
            assert_eq!(m.bots[1].health, 100);
            let board = m.scoreboard(42);
            assert_eq!(board.rows[0].kills, 1);
            assert_eq!(board.rows[1].deaths, 1);
            assert_eq!(board.rows[2].deaths, 0);
            assert_eq!(board.team_scores[usize::from(player_team)], 100);
            assert_eq!(board.team_scores[usize::from(3 - player_team)], 0);
            let kills = m.take_kills();
            assert_eq!(kills.len(), 1);
            assert_eq!(kills[0].victim.client_index, 1);
        }
    }
    #[test]
    fn melee_respects_teams_range_lifetimes_and_records_one_knife_kill() {
        use codol_protocol::game_hit_feedback::HitKind;
        let collision = world();
        for team in [1, 2] {
            let mut c = config();
            c.teams = std::array::from_fn(|i| if i % 2 == 0 { team } else { 3 - team });
            let local = template();
            let mut m = TdmMatch::new(c, local, 190.0, &collision).unwrap();
            for b in &mut m.bots {
                b.snapshot.origin = [500.0, 500.0, 0.0];
            }
            let mut strike = crate::game_player::MeleeStrike {
                time: 2100,
                angles: [0; 3],
                weapon: 352,
                damage: 135,
            };
            m.bots[0].snapshot.origin = [60.0, 0.0, 0.0];
            m.bots[1].snapshot.origin = [25.0, 0.0, 0.0];
            assert!(
                m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                    .damage
                    .is_empty()
            );
            m.bots[1].snapshot.origin[1] = 100.0;
            m.bots[0].snapshot.origin[0] = 80.0; // nearest face lies beyond64.
            assert!(
                m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                    .damage
                    .is_empty()
            );
            m.bots[0].snapshot.origin[0] = 60.0;
            strike.time = 900; // stale command from before this life.
            assert!(
                m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                    .committed_feedback
                    .is_none()
            );
            assert_eq!(m.bots[0].health, 100);
            strike.time = 1500;
            assert!(
                m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                    .committed_feedback
                    .is_none()
            );
            assert_eq!(m.bots[0].health, 100);
            strike.time = 2200;
            assert!(
                m.player_melee(
                    strike,
                    [0.0, 0.0, 60.0],
                    EnemySnapshot {
                        alive: false,
                        ..local
                    },
                    &collision
                )
                .damage
                .is_empty()
            );
            let hit = m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision);
            assert_eq!(hit.committed_feedback, Some(HitKind::Kill));
            assert!(hit.impacts.is_empty());
            assert_eq!(m.bots[0].health, 0);
            assert!(
                m.player_melee(strike, [0.0, 0.0, 60.0], local, &collision)
                    .committed_feedback
                    .is_none()
            );
            let kills = m.take_kills();
            assert_eq!(kills.len(), 1);
            assert!(kills[0].melee && !kills[0].headshot);
            assert_eq!(kills[0].weapon, 352);
            let board = m.scoreboard(0);
            assert_eq!(board.rows[0].kills, 1);
            assert_eq!(board.rows[0].headshots, 0);
            assert_eq!(board.team_scores[usize::from(team)], 100);
        }
    }
    #[test]
    fn melee_approach_accepts_native_side_hints_and_rejects_range_or_visibility_forgery() {
        let collision = world();
        let local = template();
        let mut game = TdmMatch::new(config(), local, 190.0, &collision).unwrap();
        for bot in &mut game.bots {
            bot.snapshot.origin = [500.0, 500.0, 0.0];
        }
        game.bots[5].snapshot.origin = [75.0, -24.0, 0.03125];
        let mut motion = crate::game_motion::Motion::new([0.0, 0.0, 0.03125]);
        motion.apply(
            &codol_protocol::game_usercmd::UserCommand {
                time: 1000,
                ..Default::default()
            },
            1.0,
            false,
            &collision,
        );
        let input = codol_protocol::game_usercmd::UserCommand {
            time: 2100,
            buttons: 4,
            melee_yaw: 62306,
            melee_distance: 78,
            melee_entity: 2047,
            ..Default::default()
        };
        assert!(
            game.player_melee_approach(&input, &motion, local, &collision)
                .is_some()
        );
        for bad in [
            codol_protocol::game_usercmd::UserCommand {
                melee_yaw: 0,
                ..input.clone()
            },
            codol_protocol::game_usercmd::UserCommand {
                melee_distance: 110,
                ..input.clone()
            },
            codol_protocol::game_usercmd::UserCommand {
                melee_entity: 7,
                ..input.clone()
            },
            codol_protocol::game_usercmd::UserCommand {
                angles: [0, 32768, 0],
                ..input.clone()
            },
        ] {
            assert!(
                game.player_melee_approach(&bad, &motion, local, &collision)
                    .is_none()
            );
        }
        game.bots[0].snapshot.origin = [35.0, -11.2, 0.03125];
        assert!(
            game.player_melee_approach(&input, &motion, local, &collision)
                .is_none()
        );
        game.bots[0].snapshot.origin = [500.0, 500.0, 0.0];
        game.bots[5].snapshot.origin = [130.0, 0.0, 0.03125];
        let far = codol_protocol::game_usercmd::UserCommand {
            melee_yaw: 0,
            melee_distance: 130,
            ..input
        };
        assert!(
            game.player_melee_approach(&far, &motion, local, &collision)
                .is_none()
        );
    }
    #[test]
    fn melee_approach_validates_the_hint_against_current_enemy_and_view() {
        let collision = world();
        let local = template();
        let mut game = TdmMatch::new(config(), local, 190.0, &collision).unwrap();
        for bot in &mut game.bots {
            bot.snapshot.origin = [500.0, 500.0, 0.0];
        }
        game.bots[5].snapshot.origin = [75.0, 0.0, 0.03125];
        let mut motion = crate::game_motion::Motion::new([0.0, 0.0, 0.03125]);
        motion.apply(
            &codol_protocol::game_usercmd::UserCommand {
                time: 1000,
                ..Default::default()
            },
            1.0,
            false,
            &collision,
        );
        let input = codol_protocol::game_usercmd::UserCommand {
            time: 1100,
            buttons: 4,
            melee_yaw: 0,
            melee_distance: 75,
            melee_entity: 2047,
            ..Default::default()
        };
        assert!(
            game.player_melee_approach(&input, &motion, local, &collision)
                .is_some()
        );
        for bad in [
            codol_protocol::game_usercmd::UserCommand {
                melee_yaw: 32768,
                ..input.clone()
            },
            codol_protocol::game_usercmd::UserCommand {
                melee_distance: 200,
                ..input.clone()
            },
            codol_protocol::game_usercmd::UserCommand {
                melee_entity: 1,
                ..input.clone()
            },
            codol_protocol::game_usercmd::UserCommand {
                angles: [0, 32768, 0],
                ..input.clone()
            },
        ] {
            assert!(
                game.player_melee_approach(&bad, &motion, local, &collision)
                    .is_none()
            );
        }
        game.bots[0].snapshot.origin = [40.0, 0.0, 0.03125];
        assert!(
            game.player_melee_approach(&input, &motion, local, &collision)
                .is_none()
        );
        game.bots[0].snapshot.origin = [500.0, 500.0, 0.0];
        game.bots[5].snapshot.alive = false;
        assert!(
            game.player_melee_approach(&input, &motion, local, &collision)
                .is_none()
        );
    }
    #[test]
    fn committed_feedback_ignores_protection_and_keeps_a_multi_target_kill() {
        use codol_protocol::game_hit_feedback::HitKind;
        let collision = world();
        let mut m = TdmMatch::new(config(), template(), 200.0, &collision).unwrap();
        assert_eq!(m.damage_bots(1500, 0, 352, &[(6, 100, false)], false), None);
        assert_eq!(m.bots[5].health, 100);
        assert_eq!(m.damage_bots(2100, 0, 352, &[(6, 0, false)], false), None);
        assert_eq!(
            m.damage_bots(2200, 0, 352, &[(6, 100, false), (7, 30, false)], false),
            Some(HitKind::Kill)
        );
        assert_eq!(m.bots[5].health, 0);
        assert_eq!(m.bots[6].health, 70);
        assert_eq!(m.damage_bots(2300, 0, 352, &[(6, 100, false)], false), None);
        assert_eq!(
            m.damage_bots(2400, 0, 352, &[(7, 30, false)], false),
            Some(HitKind::Hit)
        );
        m.finished = true;
        assert_eq!(m.damage_bots(2500, 0, 352, &[(7, 100, false)], false), None);
    }
    #[test]
    fn six_per_team_have_unique_clients_lives_scores_and_respawns() {
        let collision = world();
        let mut m = TdmMatch::new(config(), template(), 190.0, &collision).unwrap();
        assert_eq!(m.bots.len(), 11);
        let snapshots = m.snapshots();
        let death_waypoints = [codol_protocol::game_hit_feedback::DeathWaypointSnapshot {
            material: 169,
            origin: [1234.0, -4321.0, 154.0],
        }; 5];
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
                held_weapon_tag: Some(1),
                death_waypoints: Some(&death_waypoints),
                hit_feedback: Some(codol_protocol::game_hit_feedback::HitFeedbackSnapshot {
                    material: 144,
                    event_time: 1000,
                    visible: true,
                    kind: codol_protocol::game_hit_feedback::HitKind::Kill,
                }),
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
        m.damage_bots(2100, 0, 352, &[(6, 100, true)], false);
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
        m.damage_bots(2200, 0, 352, &[(6, 100, true)], false);
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
        m.damage_bots(2100, 0, 352, &[(6, 100, false)], false);
        assert!(m.finished);
        assert_eq!(m.result(), "Allies win");
        assert_eq!(m.scoreboard(1).score_limit, 100);
        let final_board = m.scoreboard(1);
        m.damage_bots(2200, 0, 352, &[(7, 100, false)], false);
        m.player_killed(
            2300,
            template(),
            PlayerHit {
                damage: 100,
                attacker: 6,
                weapon: 352,
                headshot: false,
            },
        );
        m.advance_clock(700000);
        assert_eq!(m.scoreboard(1), final_board);
        assert_eq!(m.finished_at, Some(2100));
        assert_eq!(m.winning_team(), 2);
    }
    #[test]
    fn deadline_freezes_a_loss_and_a_draw_without_respawning_or_rescoring() {
        let collision = world();
        for (attacker, winner) in [(Some(6), 1), (None, 0)] {
            let mut c = config();
            c.time_limit_msec = 60_000;
            let mut m = TdmMatch::new(c, template(), 190.0, &collision).unwrap();
            m.advance_clock(1000);
            if let Some(attacker) = attacker {
                m.damage_bots(2100, attacker, 352, &[(1, 100, false)], false);
            }
            m.advance_clock(60999);
            assert!(!m.finished);
            m.advance_clock(61000);
            assert!(m.finished);
            assert_eq!(m.winning_team(), winner);
            assert_eq!(m.duration_seconds(), 60);
            let final_board = m.scoreboard(1);
            let snapshots = m.snapshots();
            m.damage_bots(62000, 0, 352, &[(6, 100, false)], false);
            m.advance_clock(120000);
            assert_eq!(m.scoreboard(1), final_board);
            assert_eq!(m.finished_at, Some(61000));
            assert_eq!(m.snapshots(), snapshots);
        }
    }
    #[test]
    #[ignore = "requires local captured Afghan preparation"]
    fn captured_map_starts_build_a_real_twelve_player_match() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../research-output/tdm-state.json");
        let state = crate::game_state::PreparedGameState::load(&path).unwrap();
        assert!(state.is_match());
    }
    #[test]
    #[ignore = "requires local three-map faction outfits and CODOL_APPEARANCE_PROFILE"]
    fn captured_room_factions_keep_outfits_and_resource_packets_consistent() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
        let profile =
            std::path::PathBuf::from(std::env::var_os("CODOL_APPEARANCE_PROFILE").unwrap());
        let public = std::fs::read(profile.join("public.bin")).unwrap();
        let inventory: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profile.join("inventory.json")).unwrap())
                .unwrap();
        let owned: Vec<u32> = serde_json::from_value(inventory["items"].clone()).unwrap();
        let suffix =
            std::env::var("CODOL_BOT_OUTFIT_SUFFIX").unwrap_or_else(|_| "faction-outfits".into());
        for name in [
            "tdm-afghan-hud-state",
            "tdm-killhouse-hud-state",
            "tdm-crash-tropical-state",
        ] {
            let state = crate::game_state::PreparedGameState::load(
                &root.join(format!("{name}-{suffix}.json")),
            )
            .unwrap();
            for teams in [default_teams(), default_teams().map(|t| 3 - t)] {
                let resolved = state
                    .clone()
                    .with_room_teams(teams)
                    .unwrap()
                    .for_profile(&public, &owned)
                    .unwrap();
                let game = resolved.tdm_match().unwrap().unwrap();
                let actors = game.snapshots();
                for (i, actor) in actors.iter().enumerate() {
                    let outfit =
                        game.config.roster[i].outfits.unwrap()[usize::from(teams[i + 1] - 1)];
                    assert_eq!(actor.model, outfit.model);
                    assert_eq!(actor.attachment_models, outfit.attachment_models);
                    assert_eq!(game.baselines()[i].model, actor.model);
                }
                let gamestate = resolved.body(1).unwrap();
                let wire =
                    codol_protocol::game_wire::compact_server_packet(3, 0, 0, b"", &gamestate)
                        .unwrap();
                let snapshot = resolved
                    .snapshot(
                        1000,
                        1000,
                        true,
                        [0; 3],
                        resolved.player().map(|p| p.equipment()),
                        crate::game_state::SnapshotWorld {
                            motion: resolved.motion().as_ref(),
                            local_player: resolved.local_player_snapshot(true),
                            players: &actors,
                            vitals: Some(Default::default()),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                assert!(snapshot.len() < codol_protocol::game_wire::FRAGMENT_SIZE - 4);
                eprintln!(
                    "{name} local_team={} models={} wire={} snapshot={}",
                    teams[0],
                    game.config.models.len(),
                    wire.len(),
                    snapshot.len()
                );
            }
        }
    }

    #[test]
    #[ignore = "requires local three-map captures and CODOL_GROUND_PROFILE directory"]
    fn captured_bot_ground_contacts() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../research-output");
        let profile = std::path::PathBuf::from(
            std::env::var_os("CODOL_GROUND_PROFILE").expect("set local profile directory"),
        );
        let public = std::fs::read(profile.join("public.bin")).unwrap();
        let inventory: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profile.join("inventory.json")).unwrap())
                .unwrap();
        let owned: Vec<u32> = serde_json::from_value(inventory["items"].clone()).unwrap();
        let suffix = std::env::var("CODOL_GROUND_CAPTURE").unwrap_or_else(|_| "current".into());
        assert!(
            suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        );
        for name in [
            "tdm-afghan-hud-state",
            "tdm-killhouse-hud-state",
            "tdm-crash-tropical-state",
        ] {
            let state =
                crate::game_state::PreparedGameState::load(&root.join(format!("{name}.json")))
                    .unwrap()
                    .for_profile(&public, &owned)
                    .unwrap();
            let collision = state.collision.as_ref().unwrap();
            if let Some(path) = std::env::var_os("CODOL_GROUND_POSES") {
                let capture: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                assert_eq!(capture["client_sha256"], codol_protocol::CLIENT_SHA256);
                assert_eq!(capture["read_only"], true);
                if capture["ncs_crc"] != state.ncs_crc() {
                    continue;
                }
                let mut contacts = Vec::new();
                let clients: Vec<_> = if let Some(frames) = capture["frames"].as_array() {
                    frames
                        .iter()
                        .flat_map(|f| f["clients"].as_array().unwrap())
                        .collect()
                } else {
                    capture["clients"].as_array().unwrap().iter().collect()
                };
                for client in clients.into_iter().filter(|c| c["client"] != 0) {
                    let pose = &client["ground_pose"];
                    if pose["stable_read"] != true || pose["plausible"] != true {
                        continue;
                    }
                    let root_origin: [f32; 3] =
                        serde_json::from_value(pose["root_origin"].clone()).unwrap();
                    // Include a small upward margin for integer network Z and
                    // interpolation, without moving the authoritative actor.
                    let support = collision.trace(
                        [root_origin[0], root_origin[1], root_origin[2] + 2.0],
                        [root_origin[0], root_origin[1], root_origin[2] - 32.0],
                        70.0,
                    );
                    for foot in pose["feet"].as_array().unwrap() {
                        let p: [f32; 3] = serde_json::from_value(foot["origin"].clone()).unwrap();
                        let floor =
                            collision.raycast([p[0], p[1], p[2] + 16.0], [p[0], p[1], p[2] - 64.0]);
                        let movement_floor = collision
                            .movement_surface([p[0], p[1], p[2] + 16.0], [p[0], p[1], p[2] - 64.0]);
                        contacts.push(serde_json::json!({"client":client["client"],"legs":client["legs_animation"],
                            "time":pose["time"],"bone":foot["name"],"origin":p,
                            "root_origin":root_origin,"root_floor_gap":(support.fraction<1.0 && !support.start_solid).then_some(support.fraction*34.0-2.0),
                            "root_normal":support.normal,
                            "movement_floor_gap":(movement_floor.fraction<1.0 && !movement_floor.start_solid).then_some(movement_floor.fraction*80.0-16.0),
                            "floor_gap":(floor.fraction<1.0 && !floor.start_solid).then_some(floor.fraction*80.0-16.0),
                            "floor_normal":floor.normal}));
                    }
                }
                assert!(!contacts.is_empty(), "no current rendered feet in capture");
                let output = root.join(format!("bot-feet-{name}-{suffix}.json"));
                std::fs::write(&output, serde_json::to_vec(&contacts).unwrap()).unwrap();
                eprintln!(
                    "rendered feet diagnostic: {} -> {}",
                    contacts.len(),
                    output.display()
                );
                continue;
            }
            let mut game = state.tdm_match().unwrap().unwrap();
            for bot in &mut game.bots {
                bot.random = u64::from(bot.snapshot.client_index) * 0x9e3779b9;
            }
            let mut local = game.bots[0].snapshot;
            local.entity_number = 0;
            local.client_index = 0;
            local.alive = false;
            let mut frames = Vec::new();
            for now in (1000..=61_000).step_by(50) {
                game.step(
                    now,
                    (local, 70.0, 0),
                    state.match_bot_weapon().unwrap(),
                    collision,
                    |enemy, frame, firing, toggle| {
                        state.animate_enemy(enemy, frame, firing, toggle)
                    },
                )
                .unwrap();
                for bot in game.bots.iter().filter(|b| b.health > 0) {
                    let motion = &bot.view_motion;
                    let origin = motion.origin;
                    let down =
                        collision.trace(origin, [origin[0], origin[1], origin[2] - 16.0], 70.0);
                    let point = collision.raycast(origin, [origin[0], origin[1], origin[2] - 32.0]);
                    frames.push(
                        serde_json::json!({"time":now,"client":bot.snapshot.client_index,
                        "origin":origin,"velocity":motion.velocity,"grounded":motion.grounded(),
                        "drop":down.fraction * 16.0,"normal":down.normal,"solid":down.start_solid,
                        "point_drop":point.fraction * 32.0,"legs":bot.snapshot.legs_animation}),
                    );
                }
            }
            let output = root.join(format!("bot-ground-{name}-{suffix}.json"));
            std::fs::write(&output, serde_json::to_vec(&frames).unwrap()).unwrap();
            eprintln!(
                "ground contact diagnostic: {} samples -> {}",
                frames.len(),
                output.display()
            );
        }
    }
}
