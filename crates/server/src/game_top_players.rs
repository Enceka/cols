//! Native top-three cards, between the final replay and the result screen.
use crate::game_motion::{Collision, Motion};
use anyhow::{Result, ensure};
use codol_protocol::game_enemy::EnemySnapshot;
use codol_protocol::game_scoreboard::ScoreboardRow;
use codol_protocol::game_wire::ReliableCommands;
use serde::Deserialize;

#[derive(Clone)]
pub(crate) struct Scene {
    pub camera: Motion,
    pub angles: [u16; 3],
    pub actors: Vec<EnemySnapshot>,
    pub poses: Vec<codol_protocol::game_corpse::CorpseSnapshot>,
}
impl Scene {
    pub fn build(
        rows: &[ScoreboardRow],
        actors: &[EnemySnapshot],
        seeds: &[[f32; 3]],
        collision: &Collision,
    ) -> Result<Self> {
        let leaders = leaders(rows)?;
        ensure!(
            actors.len() == 12
                && (0..12).all(|i| actors.iter().filter(|a| a.client_index == i).count() == 1),
            "incomplete podium actor roster"
        );
        for &seed in seeds {
            let Ok(feet) = crate::game_bot::grounded_spawn(seed, collision) else {
                continue;
            };
            for turn in 0..16 {
                let yaw = turn as f32 * std::f32::consts::TAU / 16.0;
                let (sin, cos) = yaw.sin_cos();
                let eye = [feet[0], feet[1], feet[2] + 58.0];
                let mut positions = Vec::new();
                for (depth, right) in [(180.0, 0.0), (210.0, -95.0), (210.0, 95.0)] {
                    let trial = [
                        feet[0] + depth * cos + right * sin,
                        feet[1] + depth * sin - right * cos,
                        feet[2] + 8.0,
                    ];
                    let Ok(origin) = crate::game_bot::grounded_spawn(trial, collision) else {
                        break;
                    };
                    if (origin[2] - feet[2]).abs() > 12.0 {
                        break;
                    }
                    if [8.0, 36.0, 68.0].iter().any(|height| {
                        let trace =
                            collision.raycast(eye, [origin[0], origin[1], origin[2] + height]);
                        trace.start_solid || trace.all_solid || trace.fraction < 1.0
                    }) {
                        break;
                    }
                    positions.push(origin);
                }
                if positions.len() != 3 {
                    continue;
                }
                let mut shown = actors.to_vec();
                // Keep only the real roster's CI identities. Native corpse
                // entities can render a donor's complete standing appearance
                // without introducing extra participants into result caches.
                for actor in &mut shown {
                    actor.alive = false;
                    actor.events = Default::default();
                }
                let mut poses = Vec::new();
                for (rank, client) in leaders.into_iter().enumerate() {
                    let mut actor = *actors.iter().find(|a| a.client_index == client).unwrap();
                    actor.origin = positions[rank];
                    actor.angles = [
                        0,
                        (((eye[1] - actor.origin[1]).atan2(eye[0] - actor.origin[0])
                            / std::f32::consts::TAU
                            * 65536.0) as i32) as u16,
                        0,
                    ];
                    poses.push(codol_protocol::game_corpse::CorpseSnapshot {
                        entity_number: 19 + rank as u16,
                        donor_client: client,
                        origin: actor.origin,
                        angles: actor.angles,
                        legs_animation: actor.legs_animation,
                        torso_animation: 0,
                        weapon: actor.weapon,
                        present: true,
                        phase: codol_protocol::game_corpse::CorpsePhase::Posed,
                        death_sequence: 1,
                    });
                }
                shown.sort_by_key(|a| a.client_index);
                let mut camera = Motion::new(eye);
                camera.view_height = 0.0;
                camera.view_height_target = 0;
                camera.view_height_lerp_time = 0;
                camera.view_height_lerp_target = 0;
                tracing::info!(
                    ?leaders,
                    ?eye,
                    ?positions,
                    turn,
                    "native top players scene prepared"
                );
                return Ok(Self {
                    camera,
                    angles: [0, turn * 4096, 0],
                    actors: shown,
                    poses,
                });
            }
        }
        anyhow::bail!("no visible grounded top players scene in this map")
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resources {
    pub ncs_crc: u32,
    pub menu_index: u8,
}
impl Resources {
    pub fn validate(&self, crc: u32) -> Result<()> {
        ensure!(
            self.ncs_crc == crc,
            "top players menu belongs to another map"
        );
        ensure!(self.menu_index < 64, "invalid native menu index");
        Ok(())
    }
}

fn leaders(rows: &[ScoreboardRow]) -> Result<[u8; 3]> {
    let mut seen = std::collections::BTreeSet::new();
    ensure!(
        rows.iter()
            .all(|r| r.client_number < 12 && seen.insert(r.client_number)),
        "invalid top players roster"
    );
    let mut ranked: Vec<_> = rows.iter().filter(|r| r.ping >= 0).collect();
    ranked.sort_by_key(|r| {
        (
            std::cmp::Reverse(r.score),
            std::cmp::Reverse(r.kills),
            r.deaths,
            r.client_number,
        )
    });
    ensure!(
        ranked.len() >= 3,
        "top players needs three connected players"
    );
    Ok(std::array::from_fn(|i| ranked[i].client_number))
}

#[derive(Default)]
enum Phase {
    #[default]
    Waiting,
    Opening(u32),
    Showing(u32),
    Closing(u32),
    Done,
}
#[derive(Default)]
pub(crate) struct Presentation {
    phase: Phase,
    pub scene: Option<Scene>,
}
impl Presentation {
    /// The caller waits for an acknowledged live snapshot after replay exit.
    /// Durations start at the menu command's ACK, not at queue insertion.
    pub fn advance(
        &mut self,
        now: u32,
        menu: &Resources,
        rows: &[ScoreboardRow],
        commands: &mut ReliableCommands,
    ) -> Result<bool> {
        match self.phase {
            Phase::Waiting => {
                let clients = leaders(rows)?;
                // Following a fixed camera also enables the ordinary spectator
                // card. Hide that widget while the separate top-three menu runs.
                commands.push(b"s ui_hide_playercards \"1\"")?;
                // 0x6b0130 GetTop3Players reads card slots1..3, populated by
                // L <client> <slot>. q/u consume category8 menu indices,
                // base4291 (0x57ae70/0x57afb0), rather than literal names.
                for (rank, client) in clients.into_iter().enumerate() {
                    commands.push(format!("L {client} {}", rank + 1).as_bytes())?;
                }
                let sequence = commands.push(format!("q {}", menu.menu_index).as_bytes())?;
                self.phase = Phase::Opening(sequence);
                tracing::info!(?clients, sequence, "native top players queued");
            }
            Phase::Opening(sequence) if commands.acknowledged() >= sequence => {
                self.phase = Phase::Showing(now);
                tracing::info!("native top players acknowledged");
            }
            Phase::Showing(start) if now.wrapping_sub(start) >= 6000 => {
                let sequence = commands.push(format!("u {}", menu.menu_index).as_bytes())?;
                self.phase = Phase::Closing(sequence);
            }
            Phase::Closing(sequence) if commands.acknowledged() >= sequence => {
                self.phase = Phase::Done;
                tracing::info!("native top players closed");
            }
            _ => {}
        }
        Ok(matches!(self.phase, Phase::Done))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_grounds_real_appearances_and_preserves_live_actors() {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(),serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"read_only":true,"client_sha256":codol_protocol::CLIENT_SHA256,
            "map":"maps/mp/test.d3dbsp","ncs_crc":0,
            "brushes":[{"center":[0,0,-10],"half":[1000,1000,10],"contents":1,"planes":[]}],"triangles":[]
        })).unwrap()).unwrap();
        let collision = Collision::load(f.path()).unwrap();
        let actors: Vec<_> = (0..12)
            .map(|i| EnemySnapshot {
                entity_number: u16::from(i),
                client_index: i,
                team: 1,
                model: u16::from(i) + 1,
                attachment_models: [0; 10],
                attachment_tags: [0; 10],
                origin: [0.0; 3],
                angles: [0; 3],
                legs_animation: 395,
                torso_animation: 0,
                weapon: 352 + u16::from(i),
                events: Default::default(),
                alive: i != 11,
            })
            .collect();
        let original = actors.clone();
        let scene = Scene::build(&rows(), &actors, &[[0.0; 3]], &collision).unwrap();
        assert_eq!(actors, original);
        assert_eq!(scene.actors.len(), 12);
        assert_eq!(scene.poses.len(), 3);
        assert!(scene.actors.iter().all(|a| !a.alive));
        for (rank, actor) in scene.poses.iter().enumerate() {
            let donor = &original[11 - rank];
            assert_eq!(actor.weapon, donor.weapon);
            assert_eq!(actor.donor_client, donor.client_index);
            assert_eq!(actor.entity_number, 19 + rank as u16);
            assert_eq!(actor.legs_animation, donor.legs_animation);
            assert_eq!(actor.phase, codol_protocol::game_corpse::CorpsePhase::Posed);
            assert!(actor.present);
            assert!(
                !collision
                    .trace(actor.origin, actor.origin, 70.0)
                    .start_solid
            );
        }
        assert_eq!(scene.camera.view_height, 0.0);
        assert!(Scene::build(&rows(), &actors, &[[5000.0, 0.0, 0.0]], &collision).is_err());
    }
    fn rows() -> Vec<ScoreboardRow> {
        (0..12)
            .map(|client_number| ScoreboardRow {
                client_number,
                score: i32::from(client_number) * 100,
                ..Default::default()
            })
            .collect()
    }
    #[test]
    fn real_scores_have_stable_ties_and_exclude_connecting_players() {
        let mut rows = rows();
        rows[11].ping = -1;
        rows[9].score = 1000;
        rows[9].kills = 5;
        rows[8].score = 1000;
        rows[8].kills = 5;
        rows[8].deaths = 1;
        assert_eq!(leaders(&rows).unwrap(), [9, 8, 10]);
        rows[8].deaths = 0;
        assert_eq!(leaders(&rows).unwrap(), [8, 9, 10]);
        rows.reverse();
        assert_eq!(leaders(&rows).unwrap(), [8, 9, 10]);
        rows[0].client_number = rows[1].client_number;
        assert!(leaders(&rows).is_err());
        assert!(leaders(&[]).is_err());
    }
    #[test]
    fn result_waits_for_open_duration_and_close_ack_without_duplicate_commands() {
        let menu = Resources {
            ncs_crc: 123,
            menu_index: 16,
        };
        menu.validate(123).unwrap();
        assert!(menu.validate(124).is_err());
        assert!(
            Resources {
                menu_index: 64,
                ..menu.clone()
            }
            .validate(123)
            .is_err()
        );
        let mut p = Presentation::default();
        let mut commands = ReliableCommands::default();
        assert!(!p.advance(100, &menu, &rows(), &mut commands).unwrap());
        let opened = commands.sequence();
        assert_eq!(
            commands.command_for(opened - 4).unwrap(),
            b"s ui_hide_playercards \"1\""
        );
        assert_eq!(commands.command_for(opened - 3).unwrap(), b"L 11 1");
        assert_eq!(commands.command_for(opened - 2).unwrap(), b"L 10 2");
        assert_eq!(commands.command_for(opened - 1).unwrap(), b"L 9 3");
        assert_eq!(commands.command_for(opened).unwrap(), b"q 16");
        assert!(!p.advance(9000, &menu, &rows(), &mut commands).unwrap());
        assert_eq!(commands.sequence(), opened);
        commands.mark_sent(opened).unwrap();
        commands.acknowledge(opened).unwrap();
        assert!(!p.advance(9000, &menu, &rows(), &mut commands).unwrap());
        assert!(!p.advance(14999, &menu, &rows(), &mut commands).unwrap());
        assert_eq!(commands.sequence(), opened);
        assert!(!p.advance(15000, &menu, &rows(), &mut commands).unwrap());
        let closed = commands.sequence();
        assert_eq!(commands.command_for(closed).unwrap(), b"u 16");
        assert!(!p.advance(20000, &menu, &rows(), &mut commands).unwrap());
        commands.mark_sent(closed).unwrap();
        commands.acknowledge(closed).unwrap();
        assert!(p.advance(20000, &menu, &rows(), &mut commands).unwrap());
        assert!(p.advance(30000, &menu, &rows(), &mut commands).unwrap());
        assert_eq!(commands.sequence(), closed);
    }
}
