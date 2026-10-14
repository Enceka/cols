//! Per-viewer friendly death markers, independent of the reusable corpse ring.
use codol_protocol::game::snapshot::enemy::EnemySnapshot;
use codol_protocol::game::snapshot::hit_feedback::DeathWaypointSnapshot;

const LIFETIME_MSEC: u32 = 10_000;
#[derive(Clone, Copy)]
struct Death {
    time: u32,
    team: u8,
    origin: [f32; 3],
}
#[derive(Default)]
pub(crate) struct DeathWaypoints {
    deaths: [Option<Death>; 12],
}
impl DeathWaypoints {
    pub fn record(&mut self, now: u32, viewer_team: u8, victim: EnemySnapshot) {
        if victim.client_index == 0
            || victim.client_index >= 12
            || victim.team != viewer_team
            || !matches!(viewer_team, 1 | 2)
        {
            return;
        }
        let origin = [victim.origin[0], victim.origin[1], victim.origin[2] + 54.0];
        if !(DeathWaypointSnapshot {
            material: 1,
            origin,
        })
        .valid()
        {
            return;
        }
        self.deaths[usize::from(victim.client_index)] = Some(Death {
            time: now,
            team: viewer_team,
            origin,
        });
    }
    pub fn snapshot(
        &mut self,
        now: u32,
        viewer_team: u8,
        players: &[EnemySnapshot],
        material: u16,
        show: bool,
        finished: bool,
    ) -> Vec<DeathWaypointSnapshot> {
        let mut result = Vec::new();
        for (client, slot) in self.deaths.iter_mut().enumerate() {
            let Some(death) = *slot else {
                continue;
            };
            if finished
                || death.team != viewer_team
                || now.wrapping_sub(death.time) >= LIFETIME_MSEC
                || !players.iter().any(|p| {
                    usize::from(p.client_index) == client && !p.alive && p.team == viewer_team
                })
            {
                *slot = None;
                continue;
            }
            if show && result.len() < 5 {
                result.push(DeathWaypointSnapshot {
                    material,
                    origin: death.origin,
                });
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn player(client: u8, team: u8, alive: bool) -> EnemySnapshot {
        EnemySnapshot {
            entity_number: u16::from(client),
            client_index: client,
            team,
            model: 4,
            attachment_models: [0; 10],
            attachment_tags: [0; 10],
            origin: [20.0, -30.0, 40.0],
            angles: [0; 3],
            legs_animation: 395,
            torso_animation: 0,
            weapon: 352,
            events: Default::default(),
            alive,
        }
    }
    #[test]
    fn only_dead_teammates_have_frozen_positions_until_respawn_or_timeout() {
        for team in [1, 2] {
            let mut markers = DeathWaypoints::default();
            for p in [
                player(0, team, true),
                player(1, team, true),
                player(2, 3 - team, true),
            ] {
                markers.record(1000, team, p);
            }
            let mut players = [player(1, team, false), player(2, 3 - team, false)];
            players[0].origin = [1000.0; 3]; // Moving/reusing a corpse cannot move the mark.
            let mark = markers.snapshot(1000, team, &players, 169, true, false);
            assert_eq!(
                mark,
                [DeathWaypointSnapshot {
                    material: 169,
                    origin: [20.0, -30.0, 94.0]
                }]
            );
            assert!(
                markers
                    .snapshot(2000, team, &players, 169, false, false)
                    .is_empty()
            );
            assert_eq!(
                markers.snapshot(10999, team, &players, 169, true, false),
                mark
            );
            assert!(
                markers
                    .snapshot(11000, team, &players, 169, true, false)
                    .is_empty()
            );
            markers.record(12000, team, player(1, team, true));
            players[0].alive = true;
            assert!(
                markers
                    .snapshot(12100, team, &players, 169, true, false)
                    .is_empty()
            );
            players[0].alive = false;
            assert!(
                markers
                    .snapshot(12200, team, &players, 169, true, false)
                    .is_empty()
            );
        }
    }
    #[test]
    fn new_deaths_replace_old_ones_and_match_end_team_change_or_disconnect_clear_them() {
        let mut markers = DeathWaypoints::default();
        let mut victim = player(1, 1, true);
        markers.record(u32::MAX - 100, 1, victim);
        let dead = player(1, 1, false);
        assert_eq!(markers.snapshot(0, 1, &[dead], 169, true, false).len(), 1);
        victim.origin[0] = 55.0;
        markers.record(200, 1, victim);
        victim.alive = false;
        assert_eq!(
            markers.snapshot(300, 1, &[victim], 169, true, false)[0].origin[0],
            55.0
        );
        assert!(
            markers
                .snapshot(400, 1, &[victim], 169, true, true)
                .is_empty()
        );
        assert!(
            markers
                .snapshot(500, 1, &[victim], 169, true, false)
                .is_empty()
        );
        markers.record(600, 1, victim);
        assert!(
            markers
                .snapshot(700, 2, &[victim], 169, true, false)
                .is_empty()
        );
        markers.record(800, 1, victim);
        assert!(markers.snapshot(900, 1, &[], 169, true, false).is_empty());
    }
}
