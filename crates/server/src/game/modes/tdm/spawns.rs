//! war.gsc uses team starts for entry and shared mp_tdm_spawn for later lives.
//! _spawnlogic::getSpawnpoint_NearTeam weights horizontal enemy distance minus
//! twice allied distance, then penalizes enemy proximity, sight and occupancy.
use super::*;

pub(super) struct Spawns {
    points: Vec<SpawnPoint>,
    last: [Option<usize>; 12],
}
impl Spawns {
    pub(super) fn new(config: &MatchConfig, collision: &Collision) -> Result<Self> {
        // Older preparations only captured starts. Both formations provide a
        // bounded shared fallback; freshly prepared maps carry the native pool.
        let points = if config.respawn_points.is_empty() {
            config
                .spawns
                .iter()
                .enumerate()
                .flat_map(|(team, points)| {
                    points
                        .iter()
                        .enumerate()
                        .map(move |(index, &origin)| SpawnPoint {
                            origin,
                            angles: config
                                .spawn_angles
                                .as_ref()
                                .map_or([0; 3], |a| a[team][index]),
                        })
                })
                .collect::<Vec<_>>()
        } else {
            config.respawn_points.clone()
        };
        let points = points
            .into_iter()
            .map(|point| {
                Ok(SpawnPoint {
                    origin: crate::game::ai::controller::grounded_spawn(point.origin, collision)?,
                    ..point
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            points,
            last: [None; 12],
        })
    }
    fn choose(
        &mut self,
        client: u8,
        team: u8,
        actors: &[EnemySnapshot],
        collision: &Collision,
    ) -> Result<SpawnPoint> {
        let live = actors
            .iter()
            .filter(|a| a.alive && a.client_index != client)
            .collect::<Vec<_>>();
        let mut best: Option<(usize, SpawnPoint, f32)> = None;
        for (index, captured) in self.points.iter().enumerate() {
            let Ok(origin) =
                crate::game::ai::controller::grounded_spawn(captured.origin, collision)
            else {
                continue;
            };
            let point = SpawnPoint {
                origin,
                ..*captured
            };
            let mut weight = 0.0;
            let mut nearest_enemy = f32::INFINITY;
            let eye = [point.origin[0], point.origin[1], point.origin[2] + 50.0];
            for actor in &live {
                let distance = ((actor.origin[0] - point.origin[0]).powi(2)
                    + (actor.origin[1] - point.origin[1]).powi(2))
                .sqrt();
                weight += if actor.team == team {
                    -2.0 * distance
                } else {
                    distance
                };
            }
            if !live.is_empty() {
                weight /= live.len() as f32;
            }
            for actor in &live {
                let distance = ((actor.origin[0] - point.origin[0]).powi(2)
                    + (actor.origin[1] - point.origin[1]).powi(2))
                .sqrt();
                if distance < 30.0 && (actor.origin[2] - point.origin[2]).abs() < 80.0 {
                    weight -= 100_000.0;
                }
                if actor.team != team {
                    nearest_enemy = nearest_enemy.min(distance);
                    let enemy_eye = [actor.origin[0], actor.origin[1], actor.origin[2] + 50.0];
                    if collision.shot_raycast(eye, enemy_eye, 1.0).fraction >= 1.0 {
                        weight -= 100_000.0;
                    }
                }
            }
            if nearest_enemy < 1000.0 {
                weight -= 2000.0 * (1.0 - nearest_enemy / 1000.0);
            }
            if self.last[usize::from(client)] == Some(index) {
                weight -= 1000.0;
            }
            if best.is_none_or(|(_, _, score)| weight > score) {
                best = Some((index, point, weight));
            }
        }
        let (index, point, _) =
            best.ok_or_else(|| anyhow::anyhow!("no clear shared tactical respawn"))?;
        self.last[usize::from(client)] = Some(index);
        Ok(point)
    }
}
impl TdmMatch {
    /// Select origin and heading together against the current live world.
    /// Sequential Bot respawns include bodies spawned earlier in this tick.
    pub(crate) fn respawn_point(
        &mut self,
        now: u32,
        client: u8,
        local: Option<EnemySnapshot>,
        collision: &Collision,
    ) -> Result<SpawnPoint> {
        ensure!(
            client < self.config.player_count,
            "invalid respawning client"
        );
        if self.config.mode == crate::content::maps::team_modes::TeamMode::Assault
            && !self.in_grace_period(now)
            && let Some(bases) = &self.config.base_respawns
        {
            let team = usize::from(self.config.teams[usize::from(client)] - 1);
            let base = &bases[team];
            let point = base[(usize::from(client)
                + self.rows[usize::from(client)].deaths as usize)
                % base.len()];
            return Ok(SpawnPoint {
                origin: crate::game::ai::controller::grounded_spawn(point.origin, collision)?,
                ..point
            });
        }
        if self.config.mode == crate::content::maps::team_modes::TeamMode::Assault
            || self.in_grace_period(now)
        {
            let team = usize::from(self.config.teams[usize::from(client)] - 1);
            let points = &self.config.spawns[team];
            let index = (usize::from(client) + self.rows[usize::from(client)].deaths as usize)
                % points.len();
            return Ok(SpawnPoint {
                origin: crate::game::ai::controller::grounded_spawn(points[index], collision)?,
                angles: self
                    .config
                    .spawn_angles
                    .as_ref()
                    .map_or([0; 3], |a| a[team][index]),
            });
        }
        let actors = local
            .into_iter()
            .chain(self.bots.iter().map(|b| b.snapshot))
            .collect::<Vec<_>>();
        let point = self.spawns.choose(
            client,
            self.config.teams[usize::from(client)],
            &actors,
            collision,
        )?;
        tracing::info!(client, team=self.config.teams[usize::from(client)], origin=?point.origin, angles=?point.angles, "selected tactical respawn");
        Ok(point)
    }
}
