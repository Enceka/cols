//! Synthetic participants use the same collision, held weapons and damage authority.
use super::*;
use crate::game::ai::{
    controller::{BotController, BotTarget},
    navigation::Waypoint,
};
pub(super) struct Ai {
    pub controller: BotController,
    pub route: VecDeque<Waypoint>,
    planned: u32,
}
impl Ai {
    pub fn new(state: &PreparedGameState) -> Result<Self> {
        Ok(Self {
            controller: state.shared_bot_controller()?,
            route: VecDeque::new(),
            planned: 0,
        })
    }
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}
impl SharedMatch {
    pub(super) fn step_bots(&mut self, now: u32) -> Result<Vec<Hit>> {
        let mut hits = Vec::new();
        if self.ended.is_some() {
            return Ok(hits);
        }
        for index in 0..self.humans.len() {
            let human = &self.humans[index];
            if human.ai.is_none() || human.vitals.health == 0 {
                continue;
            }
            let target = self
                .humans
                .iter()
                .filter(|h| {
                    h.connected && h.vitals.health > 0 && h.member.team != human.member.team
                })
                .min_by(|a, b| {
                    distance(a.motion.origin, human.motion.origin)
                        .total_cmp(&distance(b.motion.origin, human.motion.origin))
                })
                .map(|h| BotTarget {
                    origin: h.motion.origin,
                    eye: [
                        h.motion.origin[0],
                        h.motion.origin[1],
                        h.motion.origin[2] + h.motion.view_height,
                    ],
                    alive: true,
                });
            let human = &mut self.humans[index];
            let Some(weapon) = human.player.bot_weapon() else {
                continue;
            };
            let ai = human.ai.as_mut().unwrap();
            let collision = human.state.collision.as_ref().unwrap();
            let navigation = human.state.shared_navigation();
            let waypoint = if let Some(nav) = navigation {
                if now.saturating_sub(ai.planned) >= 1500 || ai.planned == 0 {
                    if let Some(target) = target {
                        ai.route = nav
                            .route_varied(
                                human.motion.origin,
                                target.origin,
                                collision,
                                human.member.user_id ^ u64::from(now),
                                &[],
                            )
                            .into();
                    }
                    ai.planned = now;
                }
                nav.steering(human.motion.origin, &mut ai.route, collision)
            } else {
                None
            };
            ai.controller.set_tactical_control(
                human.tactical.move_scale(now, human.player.asym_robot()),
                human.tactical.turn_limit(now),
            );
            let target = target.filter(|t| {
                !human.tactical.blind(now)
                    && self.smoke.visible(
                        now,
                        [
                            human.motion.origin[0],
                            human.motion.origin[1],
                            human.motion.origin[2] + human.motion.view_height,
                        ],
                        t.eye,
                    )
            });
            let frame = ai
                .controller
                .step_route(now, target, waypoint, weapon, collision);
            human.motion = frame.motion;
            human.angles = frame.angles;
            human.last_input = now;
            for landing in frame.landings {
                if let Some(event) = landing.event {
                    human.player.movement_event(event.id, event.parameter);
                }
                let damage = human.player.fall_damage(landing.damage);
                if damage > 0 {
                    hits.push(Hit {
                        attacker: index,
                        victim: index,
                        damage,
                        weapon: weapon.weapon,
                        headshot: false,
                        melee: false,
                        falling: true,
                    });
                }
            }
            human.player.spread_environment(&human.motion);
            let reload = human
                .player
                .equipment()
                .ammo
                .is_some_and(|a| a.clip == 0 && a.reserve > 0);
            human.player.apply(&UserCommand {
                time: now,
                angles: frame.angles,
                buttons: if reload {
                    16
                } else if frame.shot.is_some() {
                    1 | 0x800
                } else {
                    0
                },
                weapons: human.player.equipment().input_weapons(),
                ..Default::default()
            });
        }
        Ok(hits)
    }
}
