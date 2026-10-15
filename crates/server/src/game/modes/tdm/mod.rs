//! Local team deathmatch authority. Native war/tdm identifies tactical play;
//! its shared respawns must not be confused with fixed-base assault play.
use crate::game::ai::controller::BotController;
use crate::game::ai::controller::BotTarget;
use crate::game::ai::controller::BotWeapon;
use crate::game::combat as game_combat;
use crate::game::combat::Actor;
use crate::game::combat::ActorShot;
use crate::game::world::collision::Collision;
use anyhow::{Result, ensure};
use codol_protocol::game::snapshot::enemy::EnemySnapshot;
use codol_protocol::game::snapshot::events::EventQueue;
use codol_protocol::game::snapshot::scoreboard::ScoreboardRow;
use codol_protocol::game::snapshot::scoreboard::ScoreboardSnapshot;
mod config;
mod spawns;
#[cfg(test)]
use config::default_teams;
pub(crate) use config::{MatchConfig, SpawnPoint};

struct Bot {
    view_motion: crate::game::player::movement::Motion,
    feedback: Option<(u32, codol_protocol::game::snapshot::hit_feedback::HitKind)>,
    damage_event: u8,
    damage_count: u8,
    snapshot_time: u32,
    weapon: Option<crate::game::weapons::held::HeldWeapons>,
    initial_weapon: Option<crate::game::weapons::held::HeldWeapons>,
    memory: Option<(u8, BotTarget, u32)>,
    perception_time: u32,
    visible_target: Option<BotTarget>,
    random: u64,
    progress_origin: [f32; 3],
    progress_time: u32,
    recent_route: std::collections::VecDeque<[f32; 3]>,
    stuck_level: u8,

    tactical: crate::game::status::tactical::State,
    poison: crate::game::status::gas::Poison,
    emp: crate::game::status::emp::State,
    controller: BotController,
    snapshot: EnemySnapshot,
    health: u16,
    birth: u32,
    death: Option<u32>,
    fire_time: Option<u32>,
    fire_toggle: bool,
    reload_animation: crate::game::ai::enemy::ReloadAnimation,
    route: std::collections::VecDeque<crate::game::ai::navigation::Waypoint>,
    route_target: Option<[f32; 3]>,
    route_time: u32,
    steering: Option<crate::game::ai::navigation::Waypoint>,
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
    rules: rules::Rules,
    config: MatchConfig,
    bots: Vec<Bot>,
    baselines: Vec<EnemySnapshot>,
    rows: [ScoreboardRow; 12],
    pub(crate) streaks: crate::game::rewards::killstreaks::Streaks,
    pub(crate) props: crate::game::world::props::Props,
    pub(crate) deployables: crate::game::rewards::deployables::Deployables,
    pub(crate) airspace: crate::game::rewards::airspace::Airspace,
    pub(crate) sam_missiles: crate::game::rewards::sam::Missiles,
    pub(crate) acceptance_stationary: bool,
    pub(crate) acceptance_fire_after_msec: Option<u32>,
    kills: Vec<Kill>,
    pub dirty: bool,
    pub result_sent: bool,
    navigation: std::sync::Arc<crate::game::ai::navigation::Navigation>,
    next_diagnostics: u32,
    acceptance_support_index: usize,
    next_route_client: usize,
    spawns: spawns::Spawns,
}
impl TdmMatch {
    pub(crate) fn finished(&self) -> bool {
        self.rules.finished
    }
    pub(crate) fn preparing(&self) -> bool {
        !self.rules.finished && self.rules.start.is_none()
    }
    pub(crate) fn live_at(&self, time: u32) -> bool {
        !self.rules.finished && self.rules.start.is_some_and(|start| time >= start)
    }
    pub(crate) fn in_grace_period(&self, time: u32) -> bool {
        !self.rules.finished
            && self
                .rules
                .start
                .is_none_or(|start| time.saturating_sub(start) < self.config.grace_period_msec)
    }
    pub(crate) fn mark_combat(&mut self, client: u8) {
        if let Some(combat) = self.rules.combat.get_mut(usize::from(client)) {
            *combat = true;
        }
    }
    pub(crate) fn spawn_client(&mut self, client: u8) {
        self.rules.combat[usize::from(client)] = false;
        self.streaks.spawn(client);
    }
    pub(crate) fn can_change_loadout(&self, time: u32) -> bool {
        (self.preparing() || self.in_grace_period(time)) && !self.rules.combat[0]
    }
    pub(crate) fn preparation_hud(
        &self,
        now: u32,
    ) -> Option<codol_protocol::game::snapshot::hit_feedback::PreparationHudSnapshot> {
        self.preparing()
            .then(|| {
                self.rules.preparation_start.map(|start| {
                    codol_protocol::game::snapshot::hit_feedback::PreparationHudSnapshot {
                        remaining_seconds: (start + self.config.preparation_msec)
                            .saturating_sub(now)
                            .div_ceil(1000)
                            .clamp(1, 30) as u8,
                        text: self.config.preparation_text,
                    }
                })
            })
            .flatten()
    }
    pub(crate) fn result(&self) -> &'static str {
        self.rules.result()
    }
    pub(crate) fn winning_team(&self) -> u8 {
        self.rules.winning_team()
    }
    pub(crate) fn duration_seconds(&self) -> u32 {
        self.rules.duration_seconds()
    }
    pub(crate) fn native_mode_number(&self) -> u32 {
        self.config.mode.native_number()
    }

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
        let navigation = std::sync::Arc::new(crate::game::ai::navigation::Navigation::build(
            config
                .spawns
                .iter()
                .flatten()
                .copied()
                .chain(config.respawn_points.iter().map(|p| p.origin))
                .chain(
                    config
                        .base_respawns
                        .iter()
                        .flatten()
                        .flatten()
                        .map(|p| p.origin),
                ),
            collision,
        ));
        Self::with_navigation(config, template, speed, collision, navigation)
    }
    pub fn with_navigation(
        config: MatchConfig,
        template: EnemySnapshot,
        speed: f32,
        collision: &Collision,
        navigation: std::sync::Arc<crate::game::ai::navigation::Navigation>,
    ) -> Result<Self> {
        config.validate(collision)?;
        let spawns = spawns::Spawns::new(&config, collision)?;
        let rules = if config.preparation_msec == 0 {
            rules::Rules {
                start: Some(1000),
                preparation_start: Some(1000),
                ..Default::default()
            }
        } else {
            rules::Rules::default()
        };
        let mut bots = Vec::new();
        let mut ordinals = [0usize; 2];
        ordinals[usize::from(config.teams[0] - 1)] = 1;
        for client in 1..config.player_count {
            let team = config.teams[usize::from(client)];
            let ordinal = ordinals[usize::from(team - 1)];
            ordinals[usize::from(team - 1)] += 1;
            let origin = crate::game::ai::controller::grounded_spawn(
                config.spawns[usize::from(team - 1)][ordinal],
                collision,
            )?;
            let mut snapshot = template;
            snapshot.entity_number = u16::from(client);
            snapshot.client_index = client;
            snapshot.team = team;
            snapshot.origin = origin;
            if let Some(angles) = &config.spawn_angles {
                snapshot.angles = angles[usize::from(team - 1)][ordinal];
            }
            snapshot.events = EventQueue::default();
            let mut controller = BotController::new(origin, snapshot.angles, speed)?;
            controller.set_match_engagement();
            bots.push(Bot {
                view_motion: crate::game::player::movement::Motion::new(origin),
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
                codol_protocol::room::messages::bot_id(i as u8)
            },
            ..Default::default()
        });
        Ok(Self {
            rules,
            config,
            bots,
            baselines,
            rows,
            streaks: Default::default(),
            props: Default::default(),
            deployables: Default::default(),
            airspace: Default::default(),
            sam_missiles: Default::default(),
            acceptance_stationary: false,
            acceptance_fire_after_msec: None,
            kills: Vec::new(),
            dirty: true,
            result_sent: false,
            navigation,
            next_diagnostics: 0,
            acceptance_support_index: 0,
            next_route_client: 0,
            spawns,
        })
    }
    pub fn configure_roster(
        &mut self,
        weapons: Vec<crate::game::weapons::held::HeldWeapons>,
    ) -> Result<()> {
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
    pub fn replay_views(&self, now: u32) -> Vec<crate::game::presentation::replay::View> {
        self.bots
            .iter()
            .filter_map(|bot| {
                let weapon = bot.weapon.as_ref()?;
                Some(crate::game::presentation::replay::View {
                    streaks: self.streaks.snapshot(
                        bot.snapshot.client_index,
                        weapon.reward_inventory()[0].map_or(0, |w| w.weapon),
                    ),
                    radar_flags: Some(
                        self.streaks.radar_flags(bot.snapshot.team, now)
                            & if weapon.counter_uav_protected() {
                                !2
                            } else {
                                u8::MAX
                            },
                    ),
                    reward_inventory: weapon.reward_inventory(),
                    shellshock: bot.tactical.snapshot(now),
                    client: bot.snapshot.client_index,
                    motion: bot.view_motion.clone(),
                    angles: bot.snapshot.angles,
                    equipment: weapon.equipment(),
                    vitals: codol_protocol::game::snapshot::PlayerVitals {
                        health: bot.health,
                        damage_event: bot.damage_event,
                        damage_count: bot.damage_count,
                        emp_jammed: bot.emp.active(now)
                            || (self.streaks.emp_active(bot.snapshot.team, now)
                                && weapon.emp_protection() > 0.0),
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
            team_scores: self.rules.scores,
            score_limit: self.config.score_limit,
        }
    }
    pub fn advance_clock(&mut self, now: u32) {
        if self.rules.finished {
            return;
        }
        let preparation = *self.rules.preparation_start.get_or_insert(now);
        if self.rules.start.is_none() {
            let start = preparation + self.config.preparation_msec;
            if now < start {
                return;
            }
            self.rules.start = Some(start);
            for bot in &mut self.bots {
                bot.progress_time = start;
                bot.snapshot_time = start;
                if let Some(weapon) = &mut bot.weapon {
                    weapon.freeze_at(start);
                }
            }
            tracing::info!(start, "team deathmatch preparation complete");
        }
        if self
            .rules
            .time_expired(now, self.config.time_limit_msec, self.streaks.incoming())
        {
            self.finish(now);
        }
    }
    fn finish(&mut self, now: u32) {
        if !self.rules.finished {
            self.rules.finished = true;
            self.rules.finished_at = Some(now);
            for bot in &mut self.bots {
                bot.poison.clear();
                bot.tactical.clear();
                bot.emp.clear();
            }
            self.dirty = true;
            tracing::info!(now, winner=self.winning_team(), scores=?self.rules.scores, "TDM match finished");
        }
    }
    #[cfg(test)]
    pub fn player_spawn(&self, collision: &Collision) -> Result<[f32; 3]> {
        let starts = &self.config.spawns[usize::from(self.config.teams[0] - 1)];
        crate::game::ai::controller::grounded_spawn(starts[0], collision)
    }
    #[cfg(test)]
    pub fn player_spawn_angles(&self) -> [u16; 3] {
        let team = usize::from(self.config.teams[0] - 1);
        self.config
            .spawn_angles
            .as_ref()
            .map_or([0; 3], |angles| angles[team][0])
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
    #[cfg(test)]
    fn deploy_sentry(
        &mut self,
        now: u32,
        local: EnemySnapshot,
        height: f32,
        r: &crate::game::rewards::killstreaks::RadarResources,
        c: &Collision,
    ) -> bool {
        self.deploy_device(now, local, height, 16, r, c)
    }
    fn damage_world_model(
        &mut self,
        now: u32,
        entity: u16,
        damage: u16,
        owner: u8,
        splash: bool,
        collision: &Collision,
    ) -> bool {
        if (crate::game::rewards::deployables::FIRST_ENTITY
            ..crate::game::rewards::deployables::END_ENTITY)
            .contains(&entity)
        {
            self.deployables.damage(
                now,
                entity,
                damage,
                self.config.teams[usize::from(owner)],
                collision,
            )
        } else if (crate::game::rewards::airspace::FIRST_ENTITY
            ..crate::game::rewards::airspace::END_ENTITY)
            .contains(&entity)
        {
            if let Some(source) = self.airspace.damage(
                now,
                entity,
                damage,
                self.config.teams[usize::from(owner)],
                collision,
            ) {
                self.streaks.stop_flight(source);
            }
            true
        } else {
            self.props
                .damage(now, entity, damage, owner, splash, collision)
        }
    }
    fn damage_bots(
        &mut self,
        now: u32,
        attacker: u8,
        weapon: u16,
        hits: &[(u8, u16, bool)],
        melee: bool,
    ) -> Option<codol_protocol::game::snapshot::hit_feedback::HitKind> {
        use codol_protocol::game::snapshot::hit_feedback::HitKind;
        let mut feedback = None;
        for &(client, damage, headshot) in hits.iter().filter(|h| h.0 != 0) {
            if !self.live_at(now) {
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
}
fn visible_enemy(
    origin: [f32; 3],
    yaw: u16,
    team: u8,
    actors: &[Actor],
    now: u32,
    smoke: &crate::game::weapons::smoke::Smoke,
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
mod tests;

mod rewards;

mod combat;

mod bots;

mod rules;
